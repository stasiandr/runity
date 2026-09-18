//! Final image treatment: the things a lens and a sensor do that a perfect
//! pinhole camera does not.
//!
//! All of it runs in linear light, before tone mapping, because that is where
//! these effects behave predictably — darkening a corner by 20% should mean 20%
//! of the light, not 20% of an already-compressed display value.

use crate::color::Color;
use crate::framebuffer::Framebuffer;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PostSettings {
    /// How much the corners are darkened. 0 is off.
    pub vignette: f32,
    /// Where the darkening starts, as a fraction of the half-diagonal.
    pub vignette_radius: f32,
    /// Amplitude of the film grain, relative to each pixel's own brightness.
    pub grain: f32,
    /// Lateral chromatic aberration: how far apart the red and blue channels
    /// are pulled at the edge of the frame, in pixels.
    pub chromatic_aberration: f32,
    /// 1.0 leaves color alone; 0 is greyscale.
    pub saturation: f32,
    /// 1.0 leaves contrast alone. Pivots around middle grey.
    pub contrast: f32,
}

impl Default for PostSettings {
    fn default() -> Self {
        Self {
            vignette: 0.0,
            vignette_radius: 0.7,
            grain: 0.0,
            chromatic_aberration: 0.0,
            saturation: 1.0,
            contrast: 1.0,
        }
    }
}

impl PostSettings {
    /// A restrained "photographic" preset: a little corner falloff, a little
    /// grain, a touch of extra contrast.
    pub fn cinematic() -> Self {
        Self {
            vignette: 0.35,
            vignette_radius: 0.65,
            grain: 0.015,
            chromatic_aberration: 0.8,
            saturation: 1.06,
            contrast: 1.08,
        }
    }
}

/// Middle grey in linear light — the pivot contrast rotates around.
const MIDDLE_GREY: f32 = 0.18;

/// Deterministic value noise in `[-1, 1]`, a pure function of pixel and frame.
#[inline]
fn noise(x: usize, y: usize, frame: u64) -> f32 {
    // An integer hash (Wang-style), so the grain does not repeat in a visible
    // grid and does not depend on any random state.
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d)
        ^ (y as u32).wrapping_mul(0x1656_67b1)
        ^ (frame as u32).wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h as f32 / u32::MAX as f32) * 2.0 - 1.0
}

pub fn apply(target: &mut Framebuffer, settings: &PostSettings, frame: u64) {
    let (width, height) = (target.width(), target.height());
    if width == 0 || height == 0 {
        return;
    }

    if settings.chromatic_aberration > 0.0 {
        chromatic_aberration(target, settings.chromatic_aberration);
    }

    let center_x = width as f32 * 0.5;
    let center_y = height as f32 * 0.5;
    let half_diagonal = (center_x * center_x + center_y * center_y).sqrt().max(1e-4);

    for y in 0..height {
        for x in 0..width {
            let mut color = target.get_pixel(x, y);

            if settings.saturation != 1.0 {
                let luminance = color.luminance();
                color = Color::rgba(
                    luminance + (color.r - luminance) * settings.saturation,
                    luminance + (color.g - luminance) * settings.saturation,
                    luminance + (color.b - luminance) * settings.saturation,
                    color.a,
                );
            }

            if settings.contrast != 1.0 {
                let f = |v: f32| ((v - MIDDLE_GREY) * settings.contrast + MIDDLE_GREY).max(0.0);
                color = Color::rgba(f(color.r), f(color.g), f(color.b), color.a);
            }

            if settings.vignette > 0.0 {
                let dx = (x as f32 + 0.5) - center_x;
                let dy = (y as f32 + 0.5) - center_y;
                let distance = (dx * dx + dy * dy).sqrt() / half_diagonal;
                let start = settings.vignette_radius.clamp(0.0, 0.999);
                let t = ((distance - start) / (1.0 - start)).clamp(0.0, 1.0);
                // Squared falloff: gentle at first, then quicker.
                color = color.scale_rgb(1.0 - settings.vignette * t * t);
            }

            if settings.grain > 0.0 {
                // Scaled by brightness: sensor noise is multiplicative enough
                // that adding a constant to black looks wrong.
                let amount = noise(x, y, frame) * settings.grain;
                color = color.scale_rgb((1.0 + amount).max(0.0));
            }

            target.set_pixel(x, y, color);
        }
    }
}

/// Pull the red and blue channels apart radially, the way a simple lens does.
fn chromatic_aberration(target: &mut Framebuffer, strength: f32) {
    let (width, height) = (target.width(), target.height());
    let source: Vec<Color> = target.colors().to_vec();
    let center_x = width as f32 * 0.5;
    let center_y = height as f32 * 0.5;
    let half_diagonal = (center_x * center_x + center_y * center_y).sqrt().max(1e-4);

    let sample = |x: f32, y: f32| -> Color {
        let x = x.clamp(0.0, width as f32 - 1.0) as usize;
        let y = y.clamp(0.0, height as f32 - 1.0) as usize;
        source[y * width + x]
    };

    for y in 0..height {
        for x in 0..width {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - center_x;
            let dy = py - center_y;
            let distance = (dx * dx + dy * dy).sqrt();
            if distance < 1e-4 {
                continue;
            }
            // Zero at the center, full strength at the corners.
            let offset = strength * (distance / half_diagonal);
            let nx = dx / distance;
            let ny = dy / distance;
            let green = source[y * width + x];
            let red = sample(px + nx * offset, py + ny * offset);
            let blue = sample(px - nx * offset, py - ny * offset);
            target.set_pixel(x, y, Color::rgba(red.r, green.g, blue.b, green.a));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: usize, height: usize, color: Color) -> Framebuffer {
        let mut fb = Framebuffer::new(width, height);
        fb.clear(color);
        fb
    }

    #[test]
    fn the_default_settings_are_a_no_op() {
        let mut target = flat(16, 16, Color::rgb(0.4, 0.5, 0.6));
        let before = target.colors().to_vec();
        apply(&mut target, &PostSettings::default(), 0);
        assert_eq!(target.colors(), before);
    }

    #[test]
    fn a_vignette_darkens_the_corners_and_leaves_the_center() {
        let mut target = flat(64, 64, Color::rgb(1.0, 1.0, 1.0));
        let settings = PostSettings {
            vignette: 0.8,
            ..PostSettings::default()
        };
        apply(&mut target, &settings, 0);
        assert!(
            (target.get_pixel(32, 32).r - 1.0).abs() < 1e-5,
            "the center is untouched"
        );
        assert!(target.get_pixel(0, 0).r < 0.5, "the corner is darkened");
        assert!(
            target.get_pixel(0, 0).r < target.get_pixel(10, 10).r,
            "and it falls off smoothly"
        );
    }

    #[test]
    fn saturation_and_contrast_move_in_the_right_direction() {
        let mut grey = flat(8, 8, Color::rgb(0.6, 0.3, 0.3));
        apply(
            &mut grey,
            &PostSettings {
                saturation: 0.0,
                ..PostSettings::default()
            },
            0,
        );
        let c = grey.get_pixel(4, 4);
        assert!(
            (c.r - c.g).abs() < 1e-5,
            "zero saturation is greyscale: {c:?}"
        );

        let mut dark = flat(8, 8, Color::rgb(0.05, 0.05, 0.05));
        apply(
            &mut dark,
            &PostSettings {
                contrast: 2.0,
                ..PostSettings::default()
            },
            0,
        );
        assert!(
            dark.get_pixel(4, 4).r < 0.05,
            "below middle grey gets darker"
        );

        let mut bright = flat(8, 8, Color::rgb(0.5, 0.5, 0.5));
        apply(
            &mut bright,
            &PostSettings {
                contrast: 2.0,
                ..PostSettings::default()
            },
            0,
        );
        assert!(bright.get_pixel(4, 4).r > 0.5, "above it gets brighter");
    }

    #[test]
    fn grain_is_deterministic_and_proportional_to_brightness() {
        let settings = PostSettings {
            grain: 0.5,
            ..PostSettings::default()
        };
        let mut first = flat(16, 16, Color::rgb(0.5, 0.5, 0.5));
        let mut second = flat(16, 16, Color::rgb(0.5, 0.5, 0.5));
        apply(&mut first, &settings, 7);
        apply(&mut second, &settings, 7);
        assert_eq!(first.colors(), second.colors(), "same frame, same grain");

        let mut later = flat(16, 16, Color::rgb(0.5, 0.5, 0.5));
        apply(&mut later, &settings, 8);
        assert_ne!(
            first.colors(),
            later.colors(),
            "and it moves between frames"
        );

        let mut black = flat(16, 16, Color::BLACK);
        apply(&mut black, &settings, 7);
        assert!(
            black.colors().iter().all(|c| c.r == 0.0),
            "black stays black"
        );
    }

    #[test]
    fn chromatic_aberration_splits_channels_only_away_from_the_center() {
        // A white disc: its edge runs across the radial direction everywhere,
        // which is exactly where a lens splits the channels.
        let mut target = Framebuffer::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let dx = x as f32 + 0.5 - 32.0;
                let dy = y as f32 + 0.5 - 32.0;
                let inside = (dx * dx + dy * dy).sqrt() < 16.0;
                target.set_pixel(x, y, if inside { Color::WHITE } else { Color::BLACK });
            }
        }
        let settings = PostSettings {
            chromatic_aberration: 4.0,
            ..PostSettings::default()
        };
        apply(&mut target, &settings, 0);

        // On the rim, the red and blue channels sample opposite sides.
        let fringe = target.get_pixel(48, 32);
        assert!(
            (fringe.r - fringe.b).abs() > 0.5,
            "expected a color fringe on the rim: {fringe:?}"
        );
        // The exact center has zero offset by construction.
        let middle = target.get_pixel(32, 32);
        assert_eq!(middle.r, middle.b);
    }
}
