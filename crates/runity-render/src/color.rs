/// Linear RGBA color with components normally in `[0, 1]`.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);
    pub const RED: Color = Color::rgb(1.0, 0.0, 0.0);
    pub const GREEN: Color = Color::rgb(0.0, 1.0, 0.0);
    pub const BLUE: Color = Color::rgb(0.0, 0.0, 1.0);

    #[inline]
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    #[inline]
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Decode a packed `0xAARRGGBB` pixel.
    #[inline]
    pub fn from_argb8(p: u32) -> Self {
        const INV: f32 = 1.0 / 255.0;
        Self {
            r: ((p >> 16) & 0xff) as f32 * INV,
            g: ((p >> 8) & 0xff) as f32 * INV,
            b: (p & 0xff) as f32 * INV,
            a: ((p >> 24) & 0xff) as f32 * INV,
        }
    }

    /// Encode as a packed `0xAARRGGBB` pixel, clamping out-of-range components.
    ///
    /// This is the layout both X11 (ZPixmap, 24/32bpp, little-endian) and the
    /// Win32 `BI_RGB` 32bpp DIB expect, so a framebuffer can be handed to the
    /// OS without a conversion pass.
    #[inline]
    pub fn to_argb8(self) -> u32 {
        #[inline]
        fn q(v: f32) -> u32 {
            // NaN falls through the comparisons and lands on 0, which beats
            // the wrapping garbage an unchecked `as u32` would produce.
            if v > 1.0 {
                255
            } else if v > 0.0 {
                (v * 255.0 + 0.5) as u32
            } else {
                0
            }
        }
        (q(self.a) << 24) | (q(self.r) << 16) | (q(self.g) << 8) | q(self.b)
    }

    #[inline]
    pub fn lerp(self, o: Self, t: f32) -> Self {
        Self {
            r: self.r + (o.r - self.r) * t,
            g: self.g + (o.g - self.g) * t,
            b: self.b + (o.b - self.b) * t,
            a: self.a + (o.a - self.a) * t,
        }
    }

    /// Alpha-blend `src` over `self`: `self * (1 - t) + opaque(src) * t`.
    ///
    /// `src`'s own alpha is ignored — `t` is the coverage the caller already
    /// folded it into — so a full-strength blend replaces color but leaves
    /// `self`'s alpha sliding towards 1 rather than jumping there, the same
    /// [`raster::Blend::Alpha`](crate::raster::Blend::Alpha) formula.
    #[inline]
    pub fn blend_over(self, src: Self, t: f32) -> Self {
        self.lerp(Color::rgba(src.r, src.g, src.b, 1.0), t.clamp(0.0, 1.0))
    }

    /// Multiply components (a "modulate" blend).
    #[inline]
    pub fn modulate(self, o: Self) -> Self {
        Self {
            r: self.r * o.r,
            g: self.g * o.g,
            b: self.b * o.b,
            a: self.a * o.a,
        }
    }

    #[inline]
    pub fn scale_rgb(self, s: f32) -> Self {
        Self {
            r: self.r * s,
            g: self.g * s,
            b: self.b * s,
            a: self.a,
        }
    }

    /// Approximate linear -> sRGB transfer (gamma 2.2), for display output.
    pub fn to_srgb(self) -> Self {
        #[inline]
        fn enc(v: f32) -> f32 {
            if v <= 0.0 {
                0.0
            } else {
                v.powf(1.0 / 2.2)
            }
        }
        Self {
            r: enc(self.r),
            g: enc(self.g),
            b: enc(self.b),
            a: self.a,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb_round_trips() {
        let c = Color::rgba(0.2, 0.4, 0.6, 1.0);
        let back = Color::from_argb8(c.to_argb8());
        assert!((c.r - back.r).abs() < 0.01);
        assert!((c.g - back.g).abs() < 0.01);
        assert!((c.b - back.b).abs() < 0.01);
        assert_eq!(back.a, 1.0);
    }

    #[test]
    fn out_of_range_components_clamp() {
        assert_eq!(Color::rgba(2.0, -1.0, 0.0, 1.0).to_argb8(), 0xff_ff_00_00);
        assert_eq!(
            Color::rgba(f32::NAN, 0.0, 0.0, 1.0).to_argb8(),
            0xff_00_00_00
        );
    }

    #[test]
    fn blend_over_matches_the_alpha_blend_formula() {
        let dst = Color::rgb(0.2, 0.2, 0.2);
        let src = Color::rgba(1.0, 0.0, 0.0, 0.3); // src.a is ignored; t drives it
        assert_eq!(
            dst.blend_over(src, 0.0),
            dst,
            "no coverage leaves dst alone"
        );
        let full = dst.blend_over(src, 1.0);
        assert_eq!((full.r, full.g, full.b, full.a), (1.0, 0.0, 0.0, 1.0));
        let half = dst.blend_over(src, 0.5);
        assert!((half.r - 0.6).abs() < 1e-6);
        assert!((half.a - 1.0).abs() < 1e-6, "dst had a=1, so it stays 1");
        // Out-of-range t is clamped rather than extrapolated.
        assert_eq!(dst.blend_over(src, 2.0), dst.blend_over(src, 1.0));
        assert_eq!(dst.blend_over(src, -1.0), dst.blend_over(src, 0.0));
    }

    #[test]
    fn channel_order_is_argb() {
        assert_eq!(Color::RED.to_argb8(), 0xff_ff_00_00);
        assert_eq!(Color::GREEN.to_argb8(), 0xff_00_ff_00);
        assert_eq!(Color::BLUE.to_argb8(), 0xff_00_00_ff);
    }
}
