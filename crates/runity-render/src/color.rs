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

    /// Linear -> sRGB transfer, for display output.
    pub fn to_srgb(self) -> Self {
        Self {
            r: linear_to_srgb(self.r),
            g: linear_to_srgb(self.g),
            b: linear_to_srgb(self.b),
            a: self.a,
        }
    }

    /// sRGB -> linear. Everything authored for a screen — texture files, color
    /// pickers, hex codes — is sRGB, and has to come through here before it can
    /// be added to or multiplied by anything.
    pub fn to_linear(self) -> Self {
        Self {
            r: srgb_to_linear(self.r),
            g: srgb_to_linear(self.g),
            b: srgb_to_linear(self.b),
            a: self.a,
        }
    }

    /// Decode a packed sRGB `0xAARRGGBB` pixel into linear light.
    #[inline]
    pub fn from_srgb8(p: u32) -> Self {
        Self::from_argb8(p).to_linear()
    }

    /// Encode as a packed `0xAARRGGBB` pixel through the sRGB transfer.
    #[inline]
    pub fn to_srgb8(self) -> u32 {
        self.to_srgb().to_argb8()
    }

    /// Relative luminance (Rec. 709), in linear light.
    #[inline]
    pub fn luminance(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }

    /// Component-wise maximum, for bright-pass style thresholds.
    #[inline]
    pub fn max_component(self) -> f32 {
        self.r.max(self.g).max(self.b)
    }
}

/// The exact sRGB transfer function, not the gamma-2.2 approximation: the
/// linear toe near black is what keeps dark gradients from banding.
#[inline]
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

#[inline]
pub fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.0 {
        0.0
    } else if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
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
    fn srgb_round_trips_through_linear() {
        for value in [0.0f32, 0.002, 0.05, 0.25, 0.5, 0.9, 1.0] {
            let back = linear_to_srgb(srgb_to_linear(value));
            assert!((back - value).abs() < 1e-5, "{value} -> {back}");
        }
    }

    #[test]
    fn mid_grey_is_not_half_the_light() {
        // The classic gotcha: sRGB 0.5 is about 21% of the light, which is why
        // lighting has to happen in linear space.
        assert!((srgb_to_linear(0.5) - 0.2140).abs() < 1e-3);
    }

    #[test]
    fn channel_order_is_argb() {
        assert_eq!(Color::RED.to_argb8(), 0xff_ff_00_00);
        assert_eq!(Color::GREEN.to_argb8(), 0xff_00_ff_00);
        assert_eq!(Color::BLUE.to_argb8(), 0xff_00_00_ff);
    }
}
