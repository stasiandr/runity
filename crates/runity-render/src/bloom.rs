//! Bloom: the glow around things brighter than the display can show.
//!
//! A tone curve has to map an unbounded range onto `[0, 1]`, so a light bulb
//! and a sheet of paper both end up near white. Bloom is what keeps them
//! distinguishable: really bright pixels bleed into their neighbours, the way
//! they do in a lens, so the eye reads them as *brighter* rather than just
//! white. It only means anything in an HDR pipeline — there is nothing above
//! white to extract otherwise.
//!
//! The filter is the usual progressive pyramid: halve the image a few times,
//! then add it back up with a tent filter on the way. A single wide blur would
//! cost far more and look worse.

use crate::color::Color;
use crate::framebuffer::Framebuffer;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomSettings {
    pub enabled: bool,
    /// Brightness where the glow starts, in linear light. 1.0 is display white.
    pub threshold: f32,
    /// Width of the soft ramp below the threshold, so the effect fades in
    /// instead of switching on at a hard edge.
    pub soft_knee: f32,
    /// How much of the blurred result is added back.
    pub intensity: f32,
    /// Pyramid levels. Each one doubles the radius of the glow.
    pub levels: usize,
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold: 1.1,
            soft_knee: 0.6,
            intensity: 0.06,
            levels: 6,
        }
    }
}

/// One level of the pyramid.
#[derive(Debug, Clone)]
struct Level {
    width: usize,
    height: usize,
    texels: Vec<Color>,
}

impl Level {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            texels: vec![Color::BLACK; width * height],
        }
    }

    #[inline]
    fn get(&self, x: isize, y: isize) -> Color {
        let x = x.clamp(0, self.width as isize - 1) as usize;
        let y = y.clamp(0, self.height as isize - 1) as usize;
        self.texels[y * self.width + x]
    }
}

/// Everything the pass needs between frames, so a steady frame rate does not
/// allocate a pyramid sixty times a second.
#[derive(Debug, Default, Clone)]
pub struct Bloom {
    levels: Vec<Level>,
}

#[inline]
fn add(a: Color, b: Color) -> Color {
    Color::rgba(a.r + b.r, a.g + b.g, a.b + b.b, a.a)
}

impl Bloom {
    pub fn new() -> Self {
        Self::default()
    }

    /// Extract the bright parts of `target`, blur them, and add them back.
    pub fn apply(&mut self, target: &mut Framebuffer, settings: &BloomSettings) {
        if !settings.enabled || settings.intensity <= 0.0 || settings.levels == 0 {
            return;
        }
        let (width, height) = (target.width(), target.height());
        if width < 4 || height < 4 {
            return;
        }

        self.build_pyramid(width, height, settings.levels);
        self.bright_pass(target, settings);
        self.downsample();
        self.upsample();

        let bloom = &self.levels[0];
        for y in 0..height {
            for x in 0..width {
                let glow = bloom.texels[y * width + x];
                let color = target.get_pixel(x, y);
                target.set_pixel(x, y, add(color, glow.scale_rgb(settings.intensity)));
            }
        }
    }

    fn build_pyramid(&mut self, width: usize, height: usize, levels: usize) {
        let needs_rebuild =
            self.levels.first().map(|l| (l.width, l.height)) != Some((width, height));
        if !needs_rebuild && !self.levels.is_empty() {
            for level in &mut self.levels {
                level.texels.fill(Color::BLACK);
            }
            return;
        }
        self.levels.clear();
        let (mut w, mut h) = (width, height);
        for _ in 0..levels.max(1) {
            self.levels.push(Level::new(w, h));
            if w <= 4 || h <= 4 {
                break;
            }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
    }

    /// Keep what is brighter than the threshold, with a quadratic ramp below it.
    fn bright_pass(&mut self, target: &Framebuffer, settings: &BloomSettings) {
        let level = &mut self.levels[0];
        let knee = settings.soft_knee.max(1e-4);
        for (destination, color) in level.texels.iter_mut().zip(target.colors()) {
            let brightness = color.max_component();
            // Karis's soft threshold curve.
            let soft = (brightness - settings.threshold + knee).clamp(0.0, 2.0 * knee);
            let soft = soft * soft / (4.0 * knee);
            let contribution = (brightness - settings.threshold).max(soft) / brightness.max(1e-4);
            *destination = color.scale_rgb(contribution.clamp(0.0, 1.0));
        }
    }

    /// Halve the image repeatedly with a 2x2 box filter.
    fn downsample(&mut self) {
        for index in 1..self.levels.len() {
            let (previous, current) = self.levels.split_at_mut(index);
            let source = previous.last().expect("there is always a lower level");
            let destination = &mut current[0];
            for y in 0..destination.height {
                for x in 0..destination.width {
                    let (sx, sy) = (x as isize * 2, y as isize * 2);
                    let mut sum = source.get(sx, sy);
                    sum = add(sum, source.get(sx + 1, sy));
                    sum = add(sum, source.get(sx, sy + 1));
                    sum = add(sum, source.get(sx + 1, sy + 1));
                    destination.texels[y * destination.width + x] = sum.scale_rgb(0.25);
                }
            }
        }
    }

    /// Walk back up, adding each level into the one below with a 3x3 tent.
    fn upsample(&mut self) {
        for index in (1..self.levels.len()).rev() {
            let (lower, upper) = self.levels.split_at_mut(index);
            let source = &upper[0];
            let destination = lower.last_mut().expect("there is always a lower level");
            for y in 0..destination.height {
                for x in 0..destination.width {
                    let fx = x as f32 * 0.5;
                    let fy = y as f32 * 0.5;
                    let (sx, sy) = (fx.floor() as isize, fy.floor() as isize);
                    // Tent weights: 1/4 center, 1/8 edges, 1/16 corners.
                    let mut sum = source.get(sx, sy).scale_rgb(0.25);
                    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        sum = add(sum, source.get(sx + dx, sy + dy).scale_rgb(0.125));
                    }
                    for (dx, dy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                        sum = add(sum, source.get(sx + dx, sy + dy).scale_rgb(0.0625));
                    }
                    let target = &mut destination.texels[y * destination.width + x];
                    *target = add(*target, sum);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: usize, height: usize, fill: Color) -> Framebuffer {
        let mut fb = Framebuffer::new(width, height);
        fb.clear(fill);
        fb
    }

    #[test]
    fn a_dim_frame_is_left_alone() {
        let mut target = frame(32, 32, Color::rgb(0.3, 0.3, 0.3));
        let before = target.colors().to_vec();
        Bloom::new().apply(&mut target, &BloomSettings::default());
        for (after, before) in target.colors().iter().zip(before) {
            assert!(
                (after.r - before.r).abs() < 1e-5,
                "nothing below the threshold should glow"
            );
        }
    }

    #[test]
    fn a_bright_spot_bleeds_into_its_neighbours() {
        let mut target = frame(64, 64, Color::BLACK);
        // One very bright pixel, far above display white.
        target.set_pixel(32, 32, Color::rgb(40.0, 40.0, 40.0));
        let settings = BloomSettings {
            intensity: 1.0,
            ..BloomSettings::default()
        };
        Bloom::new().apply(&mut target, &settings);

        let near = target.get_pixel(35, 32).luminance();
        let far = target.get_pixel(60, 32).luminance();
        assert!(near > 0.0, "the glow should reach nearby pixels: {near}");
        assert!(near > far, "and fall off with distance: {near} vs {far}");
        assert!(
            target.get_pixel(32, 32).r > 40.0,
            "the source stays at least as bright"
        );
    }

    #[test]
    fn the_threshold_decides_what_glows() {
        let settings = BloomSettings {
            threshold: 2.0,
            intensity: 1.0,
            ..BloomSettings::default()
        };
        let mut below = frame(32, 32, Color::BLACK);
        below.set_pixel(16, 16, Color::rgb(1.0, 1.0, 1.0));
        Bloom::new().apply(&mut below, &settings);
        assert!(
            below.get_pixel(20, 16).luminance() < 1e-6,
            "1.0 is under a 2.0 threshold"
        );

        let mut above = frame(32, 32, Color::BLACK);
        above.set_pixel(16, 16, Color::rgb(9.0, 9.0, 9.0));
        Bloom::new().apply(&mut above, &settings);
        assert!(above.get_pixel(20, 16).luminance() > 0.0);
    }

    #[test]
    fn colored_light_keeps_its_color() {
        let mut target = frame(48, 48, Color::BLACK);
        target.set_pixel(24, 24, Color::rgb(30.0, 2.0, 2.0));
        Bloom::new().apply(
            &mut target,
            &BloomSettings {
                intensity: 1.0,
                ..Default::default()
            },
        );
        let glow = target.get_pixel(28, 24);
        assert!(glow.r > glow.g * 3.0, "a red light glows red: {glow:?}");
    }

    #[test]
    fn disabling_it_changes_nothing() {
        let mut target = frame(16, 16, Color::rgb(50.0, 50.0, 50.0));
        let before = target.colors().to_vec();
        let settings = BloomSettings {
            enabled: false,
            ..BloomSettings::default()
        };
        Bloom::new().apply(&mut target, &settings);
        assert_eq!(target.colors(), before);
    }

    #[test]
    fn the_pyramid_is_reused_across_frames() {
        let mut bloom = Bloom::new();
        let mut target = frame(64, 64, Color::BLACK);
        target.set_pixel(10, 10, Color::rgb(20.0, 20.0, 20.0));
        bloom.apply(&mut target, &BloomSettings::default());
        let levels = bloom.levels.len();
        assert!(levels > 1);

        // A second frame at the same size must not rebuild the pyramid, and
        // must not carry the first frame's glow into it either.
        let mut second = frame(64, 64, Color::BLACK);
        bloom.apply(&mut second, &BloomSettings::default());
        assert_eq!(bloom.levels.len(), levels);
        assert!(
            second.colors().iter().all(|c| c.luminance() < 1e-6),
            "no leftovers"
        );
    }
}
