//! A heightmap placed in the world.

use crate::heightmap::{diagonal_is_forward, Heightmap};
use runity_math::{Vec2, Vec3};
use runity_serialize::{Deserialize, Reader, Result, Serialize, Writer};

/// Ground, in world coordinates.
///
/// Everything that needs to know where the surface is asks this: the mesher,
/// the character controller, the pathfinder, and the game deciding whether a
/// house fits. One source, so they cannot disagree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Terrain {
    heightmap: Heightmap,
    /// World position of sample `(0, 0)`; `y` shifts the whole surface.
    origin: Vec3,
    /// World units between samples.
    cell_size: f32,
}

/// Where a ray met the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainHit {
    pub point: Vec3,
    pub normal: Vec3,
    /// Distance along the ray.
    pub distance: f32,
}

impl Terrain {
    pub fn new(heightmap: Heightmap, origin: Vec3, cell_size: f32) -> Self {
        Self {
            heightmap,
            origin,
            cell_size: cell_size.max(f32::MIN_POSITIVE),
        }
    }

    /// Flat ground of a given size in cells, centred on the origin.
    pub fn flat(cells_x: usize, cells_z: usize, cell_size: f32) -> Self {
        let heightmap = Heightmap::new(cells_x + 1, cells_z + 1);
        let half = Vec3::new(
            cells_x as f32 * cell_size * 0.5,
            0.0,
            cells_z as f32 * cell_size * 0.5,
        );
        Self::new(heightmap, -half, cell_size)
    }

    /// Put an existing heightmap at the world origin, centred.
    pub fn centred(heightmap: Heightmap, cell_size: f32) -> Self {
        let half = Vec3::new(
            heightmap.cells_x() as f32 * cell_size * 0.5,
            0.0,
            heightmap.cells_z() as f32 * cell_size * 0.5,
        );
        Self::new(heightmap, -half, cell_size)
    }

    pub fn heightmap(&self) -> &Heightmap {
        &self.heightmap
    }

    /// Edit the heights. Nothing is cached, so the next query sees the change.
    pub fn heightmap_mut(&mut self) -> &mut Heightmap {
        &mut self.heightmap
    }

    pub fn origin(&self) -> Vec3 {
        self.origin
    }

    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    /// The world-space box the ground occupies.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let (low, high) = self.heightmap.range();
        let span = Vec3::new(
            self.heightmap.cells_x() as f32 * self.cell_size,
            0.0,
            self.heightmap.cells_z() as f32 * self.cell_size,
        );
        (
            self.origin + Vec3::new(0.0, low, 0.0),
            self.origin + span + Vec3::new(0.0, high, 0.0),
        )
    }

    /// Whether a world position is over the ground rather than past its edge.
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let (gx, gz) = self.to_grid(x, z);
        gx >= 0.0
            && gz >= 0.0
            && gx <= self.heightmap.cells_x() as f32
            && gz <= self.heightmap.cells_z() as f32
    }

    /// World XZ to grid coordinates.
    pub fn to_grid(&self, x: f32, z: f32) -> (f32, f32) {
        (
            (x - self.origin.x) / self.cell_size,
            (z - self.origin.z) / self.cell_size,
        )
    }

    /// Grid coordinates to a world position on the surface.
    pub fn to_world(&self, gx: f32, gz: f32) -> Vec3 {
        Vec3::new(
            self.origin.x + gx * self.cell_size,
            self.origin.y + self.heightmap.sample_surface(gx, gz),
            self.origin.z + gz * self.cell_size,
        )
    }

    /// The ground height under a world position.
    ///
    /// This is the triangulated surface, not a bilinear patch: it is the same
    /// surface the renderer draws and the ray cast hits, so a character placed
    /// here is standing exactly on what the player can see. Outside the
    /// terrain the edge height is returned rather than nothing — falling off
    /// the world is the game's decision to make, and [`contains`] answers it.
    ///
    /// [`contains`]: Terrain::contains
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let (gx, gz) = self.to_grid(x, z);
        self.origin.y + self.heightmap.sample_surface(gx, gz)
    }

    /// The surface normal under a world position.
    pub fn normal_at(&self, x: f32, z: f32) -> Vec3 {
        let (gx, gz) = self.to_grid(x, z);
        self.heightmap.normal(gx, gz, self.cell_size)
    }

    /// Steepness under a world position, in radians from flat.
    pub fn slope_at(&self, x: f32, z: f32) -> f32 {
        let (gx, gz) = self.to_grid(x, z);
        self.heightmap.slope(gx, gz, self.cell_size)
    }

    /// The world position at the centre of one cell.
    pub fn cell_centre(&self, x: usize, z: usize) -> Vec3 {
        let gx = x as f32 + 0.5;
        let gz = z as f32 + 0.5;
        self.to_world(gx, gz)
    }

    /// Where a ray first meets the ground, if it does within `max_distance`.
    ///
    /// Walks the cells the ray crosses rather than testing every triangle: a
    /// DDA over the grid touches a few dozen cells for a long ray across a
    /// thousand-cell map. Within a cell the two triangles are tested exactly,
    /// using the same diagonal the mesher used — the alternative is a cursor
    /// that lands slightly beside where the player sees the ground.
    pub fn cast_ray(&self, from: Vec3, direction: Vec3, max_distance: f32) -> Option<TerrainHit> {
        if self.heightmap.is_empty() || max_distance <= 0.0 {
            return None;
        }
        let direction = direction.normalized();
        if !direction.x.is_finite() || !direction.y.is_finite() || !direction.z.is_finite() {
            return None;
        }

        let cells_x = self.heightmap.cells_x() as i64;
        let cells_z = self.heightmap.cells_z() as i64;
        if cells_x == 0 || cells_z == 0 {
            return None;
        }

        // Start at the ray's entry into the terrain's footprint, so a camera
        // far outside the map does not step through empty cells to reach it.
        let mut travelled = self.entry_distance(from, direction)?;
        if travelled > max_distance {
            return None;
        }
        let entry = from + direction * travelled;
        let (mut gx, mut gz) = self.to_grid(entry.x, entry.z);
        gx = gx.clamp(0.0, cells_x as f32);
        gz = gz.clamp(0.0, cells_z as f32);

        let mut cell_x = (gx.floor() as i64).clamp(0, cells_x - 1);
        let mut cell_z = (gz.floor() as i64).clamp(0, cells_z - 1);

        let step_x: i64 = if direction.x > 0.0 { 1 } else { -1 };
        let step_z: i64 = if direction.z > 0.0 { 1 } else { -1 };
        // Distance along the ray to cross one whole cell, per axis.
        let span_x = if direction.x.abs() > 1e-9 {
            (self.cell_size / direction.x).abs()
        } else {
            f32::INFINITY
        };
        let span_z = if direction.z.abs() > 1e-9 {
            (self.cell_size / direction.z).abs()
        } else {
            f32::INFINITY
        };
        let mut next_x = travelled + boundary_distance(gx, cell_x, direction.x, self.cell_size);
        let mut next_z = travelled + boundary_distance(gz, cell_z, direction.z, self.cell_size);

        // A bound on the steps: every step leaves one cell for good.
        let limit = (cells_x + cells_z) as usize * 2 + 4;
        for _ in 0..limit {
            let leave = next_x.min(next_z).min(max_distance);
            if let Some(hit) = self.hit_in_cell(cell_x as usize, cell_z as usize, from, direction) {
                // Only accept a hit inside the piece of the ray that is
                // actually in this cell, or a ray grazing a hill would hit the
                // far side of it through the near side.
                if hit.distance >= travelled - 1e-4 && hit.distance <= leave + 1e-4 {
                    return Some(hit);
                }
            }
            if leave >= max_distance {
                return None;
            }
            travelled = leave;
            if next_x < next_z {
                cell_x += step_x;
                next_x += span_x;
            } else {
                cell_z += step_z;
                next_z += span_z;
            }
            if cell_x < 0 || cell_z < 0 || cell_x >= cells_x || cell_z >= cells_z {
                return None;
            }
        }
        None
    }

    /// The straight-down height query as a ray, for callers that want a normal
    /// with it.
    pub fn drop_onto(&self, x: f32, z: f32) -> Vec3 {
        Vec3::new(x, self.height_at(x, z), z)
    }

    /// Distance along the ray at which it enters the terrain's XZ footprint,
    /// or `None` if it never does. Zero when it starts inside.
    fn entry_distance(&self, from: Vec3, direction: Vec3) -> Option<f32> {
        let (min, max) = self.bounds();
        let mut enter = 0.0f32;
        let mut exit = f32::INFINITY;
        for (start, step, low, high) in [
            (from.x, direction.x, min.x, max.x),
            (from.z, direction.z, min.z, max.z),
        ] {
            if step.abs() < 1e-9 {
                if start < low || start > high {
                    return None;
                }
                continue;
            }
            let first = (low - start) / step;
            let second = (high - start) / step;
            enter = enter.max(first.min(second));
            exit = exit.min(first.max(second));
        }
        if enter > exit {
            None
        } else {
            Some(enter.max(0.0))
        }
    }

    /// Test the two triangles of one cell.
    fn hit_in_cell(&self, x: usize, z: usize, from: Vec3, direction: Vec3) -> Option<TerrainHit> {
        let corner = |cx: usize, cz: usize| {
            Vec3::new(
                self.origin.x + cx as f32 * self.cell_size,
                self.origin.y + self.heightmap.height(cx, cz),
                self.origin.z + cz as f32 * self.cell_size,
            )
        };
        let a = corner(x, z);
        let b = corner(x + 1, z);
        let c = corner(x, z + 1);
        let d = corner(x + 1, z + 1);
        let triangles = if diagonal_is_forward(x, z) {
            [[a, c, d], [a, d, b]]
        } else {
            [[a, c, b], [b, c, d]]
        };
        let mut best: Option<TerrainHit> = None;
        for triangle in triangles {
            if let Some(distance) = ray_triangle(from, direction, triangle) {
                let closer = match best {
                    Some(hit) => distance < hit.distance,
                    None => true,
                };
                if closer {
                    let point = from + direction * distance;
                    best = Some(TerrainHit {
                        point,
                        normal: self.normal_at(point.x, point.z),
                        distance,
                    });
                }
            }
        }
        best
    }
}

/// Distance along the ray to the next cell boundary on one axis.
fn boundary_distance(grid: f32, cell: i64, direction: f32, cell_size: f32) -> f32 {
    if direction.abs() < 1e-9 {
        return f32::INFINITY;
    }
    let within = grid - cell as f32;
    let remaining = if direction > 0.0 {
        1.0 - within
    } else {
        within
    };
    (remaining.max(0.0) * cell_size / direction).abs()
}

/// Möller–Trumbore, front and back faces alike: the ground is watertight and a
/// ray from below is a legitimate query (a mole, a thrown stone, a camera that
/// clipped through).
fn ray_triangle(from: Vec3, direction: Vec3, triangle: [Vec3; 3]) -> Option<f32> {
    let edge1 = triangle[1] - triangle[0];
    let edge2 = triangle[2] - triangle[0];
    let pvec = direction.cross(edge2);
    let determinant = edge1.dot(pvec);
    if determinant.abs() < 1e-9 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let tvec = from - triangle[0];
    let u = tvec.dot(pvec) * inverse;
    if !(-1e-5..=1.000_01).contains(&u) {
        return None;
    }
    let qvec = tvec.cross(edge1);
    let v = direction.dot(qvec) * inverse;
    if v < -1e-5 || u + v > 1.000_01 {
        return None;
    }
    let distance = edge2.dot(qvec) * inverse;
    if distance < 0.0 {
        None
    } else {
        Some(distance)
    }
}

impl Serialize for Terrain {
    fn serialize(&self, writer: &mut Writer) {
        writer.write(&self.heightmap);
        writer.f32(self.origin.x);
        writer.f32(self.origin.y);
        writer.f32(self.origin.z);
        writer.f32(self.cell_size);
    }
}

impl Deserialize for Terrain {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let heightmap = reader.read()?;
        let origin = Vec3::new(reader.f32()?, reader.f32()?, reader.f32()?);
        let cell_size = reader.f32()?;
        Ok(Self::new(heightmap, origin, cell_size))
    }
}

/// Convenience for callers that think in 2D, like the pathfinder.
impl Terrain {
    /// The XZ footprint's minimum corner — where a [`NavGrid`] should start if
    /// it is to line up with the ground.
    ///
    /// [`NavGrid`]: https://docs.rs/runity-ai
    pub fn footprint_origin(&self) -> Vec2 {
        Vec2::new(self.origin.x, self.origin.z)
    }
}
