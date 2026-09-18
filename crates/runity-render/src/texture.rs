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

/// A CPU texture sampled by fragment shaders.
#[derive(Debug, Clone)]
pub struct Texture {
    width: usize,
    height: usize,
    texels: Vec<Color>,
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
        Self {
            width,
            height,
            texels,
            wrap: Wrap::default(),
            filter: Filter::default(),
        }
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
        self.width
    }

    #[inline]
    pub fn height(&self) -> usize {
        self.height
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
    fn texel(&self, x: i64, y: i64) -> Color {
        let x = self.wrap_coord(x, self.width as i64);
        let y = self.wrap_coord(y, self.height as i64);
        self.texels[y * self.width + x]
    }

    /// Sample with `u` right and `v` down, origin at the top-left texel.
    pub fn sample(&self, u: f32, v: f32) -> Color {
        if !u.is_finite() || !v.is_finite() {
            return Color::TRANSPARENT;
        }
        match self.filter {
            Filter::Nearest => {
                let x = (u * self.width as f32).floor() as i64;
                let y = (v * self.height as f32).floor() as i64;
                self.texel(x, y)
            }
            Filter::Bilinear => {
                // Sample positions sit at texel centers, hence the -0.5.
                let fx = u * self.width as f32 - 0.5;
                let fy = v * self.height as f32 - 0.5;
                let x0 = fx.floor();
                let y0 = fy.floor();
                let tx = fx - x0;
                let ty = fy - y0;
                let (x0, y0) = (x0 as i64, y0 as i64);
                let c00 = self.texel(x0, y0);
                let c10 = self.texel(x0 + 1, y0);
                let c01 = self.texel(x0, y0 + 1);
                let c11 = self.texel(x0 + 1, y0 + 1);
                c00.lerp(c10, tx).lerp(c01.lerp(c11, tx), ty)
            }
        }
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
    fn non_finite_uv_does_not_panic() {
        let t = two_by_one();
        assert_eq!(t.sample(f32::NAN, 0.0), Color::TRANSPARENT);
    }
}
