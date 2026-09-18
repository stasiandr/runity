use crate::color::Color;

/// What to do with texture coordinates outside `[0, 1]`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    #[default]
    Repeat,
    Clamp,
    Mirror,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    #[default]
    Bilinear,
}

/// One resolution of a texture.
#[derive(Debug, Clone)]
struct Level {
    width: usize,
    height: usize,
    texels: Vec<Color>,
}

/// A CPU texture sampled by fragment shaders.
///
/// Textures carry a mip chain: successively halved copies, used when a surface
/// is far enough away that one pixel covers many texels. Without it, a
/// checkerboard floor turns into crawling noise in the distance — the sampler
/// is point-sampling a signal it can no longer represent. With it, the sampler
/// picks the level whose texels are about the size of a pixel, which is what
/// "correct" means for minification.
#[derive(Debug, Clone)]
pub struct Texture {
    levels: Vec<Level>,
    pub wrap: Wrap,
    pub filter: Filter,
}

impl Texture {
    pub fn new(width: usize, height: usize, texels: Vec<Color>) -> Self {
        assert_eq!(
            texels.len(),
            width * height,
            "texel count must match the texture size"
        );
        assert!(width > 0 && height > 0, "a texture needs a non-zero size");
        let mut texture = Self {
            levels: vec![Level {
                width,
                height,
                texels,
            }],
            wrap: Wrap::default(),
            filter: Filter::default(),
        };
        texture.build_mip_chain();
        texture
    }

    /// Halve repeatedly with a box filter until a level is 1x1.
    fn build_mip_chain(&mut self) {
        self.levels.truncate(1);
        loop {
            let previous = self.levels.last().expect("level 0 always exists");
            if previous.width == 1 && previous.height == 1 {
                break;
            }
            let width = (previous.width / 2).max(1);
            let height = (previous.height / 2).max(1);
            let mut texels = Vec::with_capacity(width * height);
            for y in 0..height {
                for x in 0..width {
                    let mut sum = Color::rgba(0.0, 0.0, 0.0, 0.0);
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(previous.width - 1);
                        let sy = (y * 2 + dy).min(previous.height - 1);
                        let c = previous.texels[sy * previous.width + sx];
                        sum = Color::rgba(sum.r + c.r, sum.g + c.g, sum.b + c.b, sum.a + c.a);
                    }
                    texels.push(Color::rgba(
                        sum.r * 0.25,
                        sum.g * 0.25,
                        sum.b * 0.25,
                        sum.a * 0.25,
                    ));
                }
            }
            self.levels.push(Level {
                width,
                height,
                texels,
            });
        }
    }

    /// Number of mip levels, including the full-resolution one.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    pub fn solid(color: Color) -> Self {
        Self::new(1, 1, vec![color])
    }

    /// Build a texture from a function of integer texel coordinates.
    pub fn from_fn(width: usize, height: usize, f: impl Fn(usize, usize) -> Color) -> Self {
        let mut texels = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                texels.push(f(x, y));
            }
        }
        Self::new(width, height, texels)
    }

    /// Classic debug checkerboard.
    pub fn checker(size: usize, cells: usize, a: Color, b: Color) -> Self {
        let cell = (size / cells.max(1)).max(1);
        Self::from_fn(size, size, |x, y| {
            if ((x / cell) + (y / cell)) % 2 == 0 {
                a
            } else {
                b
            }
        })
    }

    #[inline]
    pub fn width(&self) -> usize {
        self.levels[0].width
    }

    #[inline]
    pub fn height(&self) -> usize {
        self.levels[0].height
    }

    #[inline]
    fn wrap_coord(&self, v: i64, n: i64) -> usize {
        let w = match self.wrap {
            Wrap::Repeat => v.rem_euclid(n),
            Wrap::Clamp => v.clamp(0, n - 1),
            Wrap::Mirror => {
                let period = 2 * n;
                let m = v.rem_euclid(period);
                if m < n {
                    m
                } else {
                    period - 1 - m
                }
            }
        };
        w as usize
    }

    #[inline]
    fn texel(&self, level: usize, x: i64, y: i64) -> Color {
        let level = &self.levels[level.min(self.levels.len() - 1)];
        let x = self.wrap_coord(x, level.width as i64);
        let y = self.wrap_coord(y, level.height as i64);
        level.texels[y * level.width + x]
    }

    /// Sample one mip level.
    fn sample_level(&self, level: usize, u: f32, v: f32) -> Color {
        let index = level.min(self.levels.len() - 1);
        let (width, height) = (self.levels[index].width, self.levels[index].height);
        match self.filter {
            Filter::Nearest => {
                let x = (u * width as f32).floor() as i64;
                let y = (v * height as f32).floor() as i64;
                self.texel(index, x, y)
            }
            Filter::Bilinear => {
                // Sample positions sit at texel centers, hence the -0.5.
                let fx = u * width as f32 - 0.5;
                let fy = v * height as f32 - 0.5;
                let x0 = fx.floor();
                let y0 = fy.floor();
                let tx = fx - x0;
                let ty = fy - y0;
                let (x0, y0) = (x0 as i64, y0 as i64);
                let c00 = self.texel(index, x0, y0);
                let c10 = self.texel(index, x0 + 1, y0);
                let c01 = self.texel(index, x0, y0 + 1);
                let c11 = self.texel(index, x0 + 1, y0 + 1);
                c00.lerp(c10, tx).lerp(c01.lerp(c11, tx), ty)
            }
        }
    }

    /// Sample with `u` right and `v` down, origin at the top-left texel, using
    /// the full-resolution level.
    pub fn sample(&self, u: f32, v: f32) -> Color {
        self.sample_lod(u, v, 0.0)
    }

    /// Sample at a level of detail: 0 is full resolution, 1 is half, and so on.
    ///
    /// Fractional levels blend the two neighbouring ones — trilinear filtering,
    /// which is what keeps the transition from showing up as a visible band.
    pub fn sample_lod(&self, u: f32, v: f32, lod: f32) -> Color {
        if !u.is_finite() || !v.is_finite() {
            return Color::TRANSPARENT;
        }
        let max_level = (self.levels.len() - 1) as f32;
        let lod = lod.clamp(0.0, max_level);
        let low = lod.floor();
        let blend = lod - low;
        let low = low as usize;
        let sample = self.sample_level(low, u, v);
        if blend <= 0.0 || low + 1 >= self.levels.len() {
            return sample;
        }
        sample.lerp(self.sample_level(low + 1, u, v), blend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_by_one() -> Texture {
        Texture::new(2, 1, vec![Color::BLACK, Color::WHITE])
    }

    #[test]
    fn nearest_picks_the_covering_texel() {
        let mut t = two_by_one();
        t.filter = Filter::Nearest;
        assert_eq!(t.sample(0.25, 0.5), Color::BLACK);
        assert_eq!(t.sample(0.75, 0.5), Color::WHITE);
    }

    #[test]
    fn bilinear_blends_between_texel_centers() {
        let mut t = two_by_one();
        t.filter = Filter::Bilinear;
        let mid = t.sample(0.5, 0.5);
        assert!((mid.r - 0.5).abs() < 1e-5, "{mid:?}");
    }

    #[test]
    fn repeat_and_clamp_differ_outside_the_unit_square() {
        let mut t = two_by_one();
        t.filter = Filter::Nearest;
        t.wrap = Wrap::Repeat;
        assert_eq!(t.sample(1.25, 0.5), Color::BLACK);
        t.wrap = Wrap::Clamp;
        assert_eq!(t.sample(1.25, 0.5), Color::WHITE);
    }

    #[test]
    fn a_mip_chain_is_built_down_to_one_texel() {
        let texture = Texture::checker(64, 8, Color::BLACK, Color::WHITE);
        // 64, 32, 16, 8, 4, 2, 1
        assert_eq!(texture.level_count(), 7);
        assert_eq!(texture.width(), 64, "level 0 is still the original");

        // The smallest level is the average of the whole checkerboard: grey.
        let smallest = texture.sample_lod(0.5, 0.5, 99.0);
        assert!((smallest.r - 0.5).abs() < 0.02, "{smallest:?}");
    }

    #[test]
    fn a_higher_lod_averages_away_the_detail() {
        let texture = Texture::checker(64, 32, Color::BLACK, Color::WHITE);
        // At the finest level a checker texel is black or white...
        let sharp = texture.sample_lod(0.02, 0.02, 0.0);
        assert!(sharp.r < 0.1 || sharp.r > 0.9, "{sharp:?}");
        // ...and several levels up it has averaged towards grey.
        let blurred = texture.sample_lod(0.02, 0.02, 4.0);
        assert!((blurred.r - 0.5).abs() < 0.25, "{blurred:?}");
    }

    #[test]
    fn fractional_levels_blend_between_two_mips() {
        let texture = Texture::checker(32, 4, Color::BLACK, Color::WHITE);
        let low = texture.sample_lod(0.1, 0.1, 1.0).r;
        let high = texture.sample_lod(0.1, 0.1, 2.0).r;
        let middle = texture.sample_lod(0.1, 0.1, 1.5).r;
        let expected = (low + high) * 0.5;
        assert!((middle - expected).abs() < 1e-5, "{middle} vs {expected}");
    }

    #[test]
    fn non_finite_uv_does_not_panic() {
        let t = two_by_one();
        assert_eq!(t.sample(f32::NAN, 0.0), Color::TRANSPARENT);
    }
}
