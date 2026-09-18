//! A grid of heights, and how to read between its samples.

use runity_math::{Fbm, Noise, Vec2, Vec3};
use runity_serialize::{Deserialize, Error, Reader, Result, Serialize, Writer};

/// Heights on a regular grid, in grid coordinates.
///
/// The grid knows nothing about world units or where it sits — that is
/// [`Terrain`](crate::Terrain)'s job. Keeping the two apart means a heightmap
/// can be generated, eroded, saved and tested without a world around it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Heightmap {
    width: usize,
    depth: usize,
    heights: Vec<f32>,
}

impl Heightmap {
    /// A flat map of `width` by `depth` **samples** — one more than the number
    /// of cells in each direction, the way fence posts outnumber panels.
    pub fn new(width: usize, depth: usize) -> Self {
        Self {
            width,
            depth,
            heights: vec![0.0; width * depth],
        }
    }

    /// Wrap existing samples, row by row along +X.
    ///
    /// # Panics
    ///
    /// If `heights` is not exactly `width * depth` long.
    pub fn from_heights(width: usize, depth: usize, heights: Vec<f32>) -> Self {
        assert_eq!(
            heights.len(),
            width * depth,
            "a {width}x{depth} heightmap needs {} samples",
            width * depth
        );
        Self {
            width,
            depth,
            heights,
        }
    }

    /// Fill from a function of grid coordinates.
    pub fn from_fn(
        width: usize,
        depth: usize,
        mut height: impl FnMut(usize, usize) -> f32,
    ) -> Self {
        let mut heights = Vec::with_capacity(width * depth);
        for z in 0..depth {
            for x in 0..width {
                heights.push(height(x, z));
            }
        }
        Self::from_heights(width, depth, heights)
    }

    /// Fill from fractal noise.
    ///
    /// `frequency` is in cycles per sample, so halving it doubles the size of
    /// the hills. The result depends only on the noise's seed and the
    /// arguments — no accumulator, no iteration order — which is what lets two
    /// machines generate the same island without sending it.
    pub fn from_noise(
        width: usize,
        depth: usize,
        noise: &Noise,
        fbm: Fbm,
        frequency: f32,
        amplitude: f32,
    ) -> Self {
        Self::from_fn(width, depth, |x, z| {
            noise.fbm_2d(Vec2::new(x as f32 * frequency, z as f32 * frequency), fbm) * amplitude
        })
    }

    /// Samples across, along +X.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Samples deep, along +Z.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Cells across — one fewer than the samples, and zero for a map too small
    /// to have any.
    pub fn cells_x(&self) -> usize {
        self.width.saturating_sub(1)
    }

    /// Cells deep.
    pub fn cells_z(&self) -> usize {
        self.depth.saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    /// Every sample, row by row. The order is the storage order.
    pub fn heights(&self) -> &[f32] {
        &self.heights
    }

    /// One sample. Coordinates outside the grid clamp to the edge, so a query
    /// near the border never has to be special-cased by the caller.
    pub fn height(&self, x: usize, z: usize) -> f32 {
        if self.heights.is_empty() {
            return 0.0;
        }
        let x = x.min(self.width - 1);
        let z = z.min(self.depth - 1);
        self.heights[z * self.width + x]
    }

    /// Move one sample.
    pub fn set(&mut self, x: usize, z: usize, height: f32) {
        if x < self.width && z < self.depth {
            self.heights[z * self.width + x] = height;
        }
    }

    /// Raise or lower every sample.
    pub fn offset(&mut self, by: f32) {
        for height in &mut self.heights {
            *height += by;
        }
    }

    /// Clamp every sample into a range — a sea floor and a ceiling.
    pub fn clamp(&mut self, low: f32, high: f32) {
        for height in &mut self.heights {
            *height = height.clamp(low, high);
        }
    }

    /// The lowest and highest sample; `(0, 0)` for an empty map.
    pub fn range(&self) -> (f32, f32) {
        match self.heights.split_first() {
            None => (0.0, 0.0),
            Some((first, rest)) => rest.iter().fold((*first, *first), |(low, high), height| {
                (low.min(*height), high.max(*height))
            }),
        }
    }

    /// The height between samples, interpolated bilinearly.
    ///
    /// Bilinear and not bicubic on purpose: the mesh the renderer draws is
    /// made of flat triangles between the same samples, and a smoother query
    /// would put the ground somewhere the player can see it is not. What is
    /// sampled here and what is drawn must be the same surface — see
    /// [`Terrain::height_at`](crate::Terrain::height_at), which corrects the
    /// remaining difference by interpolating over the triangle rather than the
    /// square.
    pub fn sample(&self, x: f32, z: f32) -> f32 {
        if self.heights.is_empty() {
            return 0.0;
        }
        let (x0, z0, tx, tz) = self.cell_of(x, z);
        let x1 = (x0 + 1).min(self.width - 1);
        let z1 = (z0 + 1).min(self.depth - 1);
        let top = lerp(self.height(x0, z0), self.height(x1, z0), tx);
        let bottom = lerp(self.height(x0, z1), self.height(x1, z1), tx);
        lerp(top, bottom, tz)
    }

    /// The height on the triangulated surface, which is what a mesh built by
    /// [`Terrain`](crate::Terrain) actually has.
    ///
    /// A square of four samples is drawn as two triangles, and a bilinear
    /// patch is not either of them: on a saddle the difference is the whole
    /// diagonal, which is a player standing visibly inside a ridge.
    pub fn sample_surface(&self, x: f32, z: f32) -> f32 {
        if self.heights.is_empty() {
            return 0.0;
        }
        let (x0, z0, tx, tz) = self.cell_of(x, z);
        let x1 = (x0 + 1).min(self.width - 1);
        let z1 = (z0 + 1).min(self.depth - 1);
        let (h00, h10, h01, h11) = (
            self.height(x0, z0),
            self.height(x1, z0),
            self.height(x0, z1),
            self.height(x1, z1),
        );
        // Which way the square is split — the same rule the mesher uses.
        if diagonal_is_forward(x0, z0) {
            // Split from (0,0) to (1,1).
            if tx >= tz {
                h00 + (h10 - h00) * tx + (h11 - h10) * tz
            } else {
                h00 + (h11 - h01) * tx + (h01 - h00) * tz
            }
        } else {
            // Split from (1,0) to (0,1).
            if tx + tz <= 1.0 {
                h00 + (h10 - h00) * tx + (h01 - h00) * tz
            } else {
                h11 + (h01 - h11) * (1.0 - tx) + (h10 - h11) * (1.0 - tz)
            }
        }
    }

    /// The surface normal at a grid position, for a cell `cell_size` wide.
    ///
    /// Central differences on the heightmap, not the face normal of whichever
    /// triangle the point landed in. That is the whole reason chunks do not
    /// show seams: two chunks meeting at an edge derive the same normal from
    /// the same samples, whereas face normals differ across the join.
    pub fn normal(&self, x: f32, z: f32, cell_size: f32) -> Vec3 {
        if self.heights.is_empty() || cell_size <= 0.0 {
            return Vec3::Y;
        }
        let dx = self.sample(x + 1.0, z) - self.sample(x - 1.0, z);
        let dz = self.sample(x, z + 1.0) - self.sample(x, z - 1.0);
        // The gradient spans two cells, hence the 2.
        Vec3::new(-dx, 2.0 * cell_size, -dz).normalized()
    }

    /// Steepness at a grid position, in radians from flat.
    pub fn slope(&self, x: f32, z: f32, cell_size: f32) -> f32 {
        self.normal(x, z, cell_size).y.clamp(-1.0, 1.0).acos()
    }

    /// Average each sample with its four neighbours, `passes` times.
    ///
    /// Terrain straight out of fbm is too noisy to walk on: every cell is a
    /// step the character controller has to climb. Smoothing is cheaper and
    /// more predictable than lowering the octave count, which changes the
    /// shape of the hills as well as their roughness.
    pub fn smooth(&mut self, passes: usize) {
        if self.heights.len() < 2 {
            return;
        }
        for _ in 0..passes {
            let source = self.heights.clone();
            for z in 0..self.depth {
                for x in 0..self.width {
                    let at = |sx: usize, sz: usize| source[sz * self.width + sx];
                    let left = at(x.saturating_sub(1), z);
                    let right = at((x + 1).min(self.width - 1), z);
                    let up = at(x, z.saturating_sub(1));
                    let down = at(x, (z + 1).min(self.depth - 1));
                    let here = at(x, z);
                    self.heights[z * self.width + x] = (here + left + right + up + down) / 5.0;
                }
            }
        }
    }

    /// Which cell a grid position is in, and how far across it.
    fn cell_of(&self, x: f32, z: f32) -> (usize, usize, f32, f32) {
        let clamped_x = x.clamp(0.0, (self.width - 1) as f32);
        let clamped_z = z.clamp(0.0, (self.depth - 1) as f32);
        let x0 = (clamped_x.floor() as usize).min(self.width.saturating_sub(1));
        let z0 = (clamped_z.floor() as usize).min(self.depth.saturating_sub(1));
        (x0, z0, clamped_x - x0 as f32, clamped_z - z0 as f32)
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// How a square of four samples is cut into two triangles.
///
/// Alternating by parity rather than always cutting the same way: a fixed
/// diagonal leans every square in one direction, and on a long ridge running
/// across that direction it shows up as a visible herringbone. The rule lives
/// here because the mesher, the height query and the ray cast must all agree —
/// a surface that is drawn one way and collided another is a player standing
/// in the ground.
pub(crate) fn diagonal_is_forward(x: usize, z: usize) -> bool {
    (x + z) % 2 == 0
}

impl Serialize for Heightmap {
    fn serialize(&self, writer: &mut Writer) {
        writer.varint(self.width as u64);
        writer.varint(self.depth as u64);
        for height in &self.heights {
            writer.f32(*height);
        }
    }
}

impl Deserialize for Heightmap {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let position = reader.position();
        let width = reader.varint()? as usize;
        let depth = reader.varint()? as usize;
        let count = width.checked_mul(depth).ok_or(Error::LengthOutOfRange {
            position,
            length: u64::MAX,
            available: reader.remaining(),
        })?;
        // Four bytes each: a length the rest of the message cannot back is a
        // corrupt file, not a reason to allocate a gigabyte.
        if count * 4 > reader.remaining() {
            return Err(Error::LengthOutOfRange {
                position,
                length: count as u64,
                available: reader.remaining(),
            });
        }
        let mut heights = Vec::with_capacity(count);
        for _ in 0..count {
            heights.push(reader.f32()?);
        }
        Ok(Self {
            width,
            depth,
            heights,
        })
    }
}
