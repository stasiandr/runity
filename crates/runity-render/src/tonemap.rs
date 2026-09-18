//! Turning unbounded light into displayable pixels.
//!
//! The renderer works in linear light with no upper bound: a sunlit surface and
//! a light bulb are hundreds of times brighter than a shadowed wall, and
//! clamping them at 1.0 is what makes naive software renders look like plastic.
//! Tone mapping is the step that compresses that range, and it belongs at the
//! very end — after every light, reflection and bloom has been added.

use crate::color::Color;

/// How a linear HDR frame becomes 8-bit output.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ToneMap {
    /// Clamp to `[0, 1]` and pack, with no exposure and no transfer function.
    ///
    /// For buffers that already hold display values — debug views, masks,
    /// anything that is data rather than light.
    Raw,
    /// `c / (1 + c)`: cheap, never clips, but washes out saturated highlights.
    Reinhard,
    /// Narkowicz's fit of the ACES filmic curve — contrasty shoulder, the
    /// default because it makes bright highlights roll off instead of clip.
    #[default]
    AcesFilmic,
}

impl ToneMap {
    /// Apply exposure, the curve, and the sRGB transfer.
    pub fn apply(self, color: Color, exposure: f32) -> Color {
        match self {
            ToneMap::Raw => color,
            ToneMap::Reinhard => reinhard(color.scale_rgb(exposure)).to_srgb(),
            ToneMap::AcesFilmic => aces_filmic(color.scale_rgb(exposure)).to_srgb(),
        }
    }
}

fn reinhard(c: Color) -> Color {
    #[inline]
    fn f(v: f32) -> f32 {
        let v = v.max(0.0);
        v / (1.0 + v)
    }
    Color::rgba(f(c.r), f(c.g), f(c.b), c.a)
}

/// ACES filmic approximation (Krzysztof Narkowicz, 2015).
fn aces_filmic(c: Color) -> Color {
    #[inline]
    fn f(v: f32) -> f32 {
        const A: f32 = 2.51;
        const B: f32 = 0.03;
        const C: f32 = 2.43;
        const D: f32 = 0.59;
        const E: f32 = 0.14;
        let v = v.max(0.0);
        ((v * (A * v + B)) / (v * (C * v + D) + E)).clamp(0.0, 1.0)
    }
    Color::rgba(f(c.r), f(c.g), f(c.b), c.a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_passes_values_through_untouched() {
        let c = Color::rgb(0.25, 0.5, 2.0);
        assert_eq!(
            ToneMap::Raw.apply(c, 4.0),
            c,
            "no exposure, no curve, no gamma"
        );
    }

    #[test]
    fn curves_are_monotonic_and_bounded() {
        for map in [ToneMap::Reinhard, ToneMap::AcesFilmic] {
            let mut previous = -1.0;
            for step in 0..200 {
                let input = step as f32 * 0.5; // up to 100x over white
                let out = map.apply(Color::rgb(input, input, input), 1.0);
                assert!(out.r >= previous, "{map:?} is not monotonic at {input}");
                assert!(
                    (0.0..=1.0).contains(&out.r),
                    "{map:?} left the range at {input}"
                );
                previous = out.r;
            }
        }
    }

    #[test]
    fn bright_values_roll_off_instead_of_clipping() {
        // Two very different HDR values must not map to the same output.
        let a = ToneMap::AcesFilmic.apply(Color::rgb(2.0, 2.0, 2.0), 1.0);
        let b = ToneMap::AcesFilmic.apply(Color::rgb(8.0, 8.0, 8.0), 1.0);
        assert!(b.r > a.r, "highlights must stay distinguishable");
        assert!(a.r < 1.0, "and 2.0 must not already be pure white");
    }

    #[test]
    fn exposure_scales_the_input() {
        let dim = ToneMap::Reinhard.apply(Color::rgb(0.5, 0.5, 0.5), 0.5);
        let bright = ToneMap::Reinhard.apply(Color::rgb(0.5, 0.5, 0.5), 2.0);
        assert!(bright.r > dim.r);
    }

    #[test]
    fn negative_light_does_not_produce_negative_pixels() {
        let out = ToneMap::AcesFilmic.apply(Color::rgb(-1.0, 0.0, 0.5), 1.0);
        assert_eq!(out.r, 0.0);
    }
}
