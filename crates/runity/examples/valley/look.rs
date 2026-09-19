//! Valley: sky, sun, and ember colors and light.
//!
//! Game color lives here, in the example tree, not in `runity-render` or
//! `runity-core` — `docs/ARCHITECTURE.md` keeps the rasterizer below the
//! game, and `docs/design/07-look.md` part 0 draws the line at this file.
//! The scene itself (models, atlases, placement) lives in `assets/valley/`,
//! outside the crate entirely; only the things that change with the hour —
//! light, sky, embers, and the four settler shirt tints — are code.
//!
//! Reference: `docs/design/07-look.md` part 2 (the palette table) and the
//! design discussion for card #33, which puts embers on the "code" side of
//! the line because they are a light, not a surface.

use runity::prelude::*;
use std::f32::consts::TAU;

const fn hex(r: u8, g: u8, b: u8) -> Color {
    Color::rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

// Sky color at the four reference hours (`07-look.md` part 2, "код" rows).
pub const SKY_DAWN: Color = hex(0xE8, 0xB9, 0x8A);
pub const SKY_NOON: Color = hex(0x9F, 0xC4, 0xE8);
pub const SKY_DUSK: Color = hex(0xD9, 0x8A, 0x5A);
pub const SKY_NIGHT: Color = hex(0x14, 0x20, 0x3A);

// Sun/moon light color at the same four hours.
pub const SUN_DAWN: Color = hex(0xFF, 0xD9, 0xA0);
pub const SUN_NOON: Color = hex(0xFF, 0xF4, 0xE0);
pub const SUN_DUSK: Color = hex(0xFF, 0x9C, 0x5A);
pub const SUN_MOON: Color = hex(0x9F, 0xB4, 0xD8);

/// Fire embers: a light, not a surface — the atlas carries no orange at all,
/// so this is the one color allowed to read as loud (see
/// [`is_saturated_color`]).
pub const EMBER_COLOR: Color = hex(0xD2, 0x70, 0x3A);

/// Four muted shirt tones, tinted onto settler instances rather than baked
/// into the atlas (`07-look.md` part 2, "тинт инстанса").
pub const SHIRT_TONES: [Color; 4] = [
    hex(0x6E, 0x6A, 0x5E),
    hex(0x7A, 0x5E, 0x4A),
    hex(0x58, 0x6A, 0x62),
    hex(0x8A, 0x7A, 0x62),
];

/// Threshold above which [`is_saturated_color`] calls a color "saturated".
/// It sits between the palette's most vivid muted tone (night sky, HSV
/// saturation ~0.66) and the embers (~0.72) — by design, the embers are the
/// only exported constant that crosses it.
pub const SATURATION_THRESHOLD: f32 = 0.69;

/// HSV saturation, ignoring alpha: `(max - min) / max`, `0.0` for black.
fn hsv_saturation(color: Color) -> f32 {
    let max = color.r.max(color.g).max(color.b);
    let min = color.r.min(color.g).min(color.b);
    if max <= 0.0 {
        0.0
    } else {
        (max - min) / max
    }
}

/// Whether `color` reads as a loud, saturated color rather than one of the
/// valley's muted northern tones. Shared with the atlas test in the
/// content-sources card, which checks that the berry bush is the only
/// saturated surface in the scene.
pub fn is_saturated_color(color: Color) -> bool {
    hsv_saturation(color) >= SATURATION_THRESHOLD
}

/// Normalize an hour of day into `[0, 24)`, wrapping through midnight either
/// direction (so `-1.0` and `23.0` land on the same value).
fn wrap_hour(hour: f32) -> f32 {
    hour.rem_euclid(24.0)
}

/// Which pair of the four reference hours (dawn = 6, noon = 12, dusk = 18,
/// night = 0/24) brackets `hour`, as an index into a `[dawn, noon, dusk,
/// night]` array, plus how far between them (`0.0` at the first anchor,
/// `1.0` at the second). Wraps through midnight: hour `23` sits between dusk
/// (index 2) and night (index 3); hour `1` sits between night (index 3) and
/// the next day's dawn (index 0).
fn hour_segment(hour: f32) -> (usize, f32) {
    let h = wrap_hour(hour);
    if h < 6.0 {
        (3, h / 6.0)
    } else if h < 12.0 {
        (0, (h - 6.0) / 6.0)
    } else if h < 18.0 {
        (1, (h - 12.0) / 6.0)
    } else {
        (2, (h - 18.0) / 6.0)
    }
}

#[derive(Clone, Copy)]
struct SunAnchor {
    direction: Vec3,
    color: Color,
    intensity: f32,
}

fn sun_anchors() -> [SunAnchor; 4] {
    [
        SunAnchor {
            direction: Vec3::new(0.9, -0.25, 0.15).normalized(),
            color: SUN_DAWN,
            intensity: 0.6,
        },
        SunAnchor {
            direction: Vec3::new(-0.1, -0.98, -0.05).normalized(),
            color: SUN_NOON,
            intensity: 1.25,
        },
        SunAnchor {
            direction: Vec3::new(-0.9, -0.25, 0.15).normalized(),
            color: SUN_DUSK,
            intensity: 0.6,
        },
        SunAnchor {
            direction: Vec3::new(0.35, -0.6, 0.45).normalized(),
            color: SUN_MOON,
            intensity: 0.18,
        },
    ]
}

/// The sun/moon light and the sky color at `hour` (`0..24`, wraps).
///
/// Linearly interpolates between four opaque reference points — dawn (6),
/// noon (12), dusk (18), night (0/24) — and returns exactly those reference
/// values at those exact hours.
pub fn sky_at(hour: f32) -> (DirectionalLight, Color) {
    let (i, t) = hour_segment(hour);
    let anchors = sun_anchors();
    let a = anchors[i];
    let b = anchors[(i + 1) % 4];

    let light = DirectionalLight {
        direction: a.direction.lerp(b.direction, t).normalized(),
        color: a.color.lerp(b.color, t),
        intensity: a.intensity + (b.intensity - a.intensity) * t,
    };

    let skies = [SKY_DAWN, SKY_NOON, SKY_DUSK, SKY_NIGHT];
    let sky = skies[i].lerp(skies[(i + 1) % 4], t);

    (light, sky)
}

// Base ember brightness at the same four reference hours: near-full at
// night, almost extinguished at noon.
const EMBER_STRENGTH_DAWN: f32 = 0.35;
const EMBER_STRENGTH_NOON: f32 = 0.08;
const EMBER_STRENGTH_DUSK: f32 = 0.35;
const EMBER_STRENGTH_NIGHT: f32 = 1.0;

/// Floor and ceiling the flickering strength is clamped to: embers never
/// go fully dark and never overshoot full brightness.
const EMBER_STRENGTH_MIN: f32 = 0.06;
const EMBER_STRENGTH_MAX: f32 = 1.0;

const EMBER_FLICKER_PERIOD_SECS: f32 = 1.5;
const EMBER_FLICKER_AMPLITUDE: f32 = 0.10;
const EMBER_NOISE_AMPLITUDE: f32 = 0.05;

/// Distance in meters at which the embers' warm ambient addition fades to
/// nothing.
const EMBER_AMBIENT_RANGE_M: f32 = 6.0;

fn ember_base_strength(hour: f32) -> f32 {
    let (i, t) = hour_segment(hour);
    let anchors = [
        EMBER_STRENGTH_DAWN,
        EMBER_STRENGTH_NOON,
        EMBER_STRENGTH_DUSK,
        EMBER_STRENGTH_NIGHT,
    ];
    let a = anchors[i];
    let b = anchors[(i + 1) % 4];
    a + (b - a) * t
}

/// Cheap deterministic pseudo-noise in `[0, 1)` — the classic sine-hash
/// trick, not a stateful RNG, so the flicker is a pure function of time and
/// reproducible frame to frame.
fn pseudo_noise(t: f32) -> f32 {
    let x = (t * 12.9898).sin() * 43_758.547;
    x - x.floor()
}

fn ember_flicker(t: f32) -> f32 {
    let phase = t * TAU / EMBER_FLICKER_PERIOD_SECS;
    let wave = phase.sin() * EMBER_FLICKER_AMPLITUDE;
    let noise = (pseudo_noise(t) - 0.5) * 2.0 * EMBER_NOISE_AMPLITUDE;
    wave + noise
}

/// The embers' own glow and the warm addition they lend to ambient light.
///
/// `hour` drives the base brightness (full at night, almost out at noon);
/// `t` (seconds) drives a slow flicker — sine plus a little noise, period
/// ~1.5s — layered on top and clamped so it never fully dies or overshoots.
/// The returned [`Color`] is the warm tint to add to `Engine::ambient`; it
/// fades with both `hour` (through the same brightness curve) and
/// `distance_to_fire` (meters), since there is no real point light to carry
/// it — only [`DirectionalLight`] and a global ambient term.
pub fn ember_glow(hour: f32, distance_to_fire: f32, t: f32) -> (Color, f32) {
    let strength = (ember_base_strength(hour) + ember_flicker(t))
        .clamp(EMBER_STRENGTH_MIN, EMBER_STRENGTH_MAX);

    let falloff = (1.0 - distance_to_fire.max(0.0) / EMBER_AMBIENT_RANGE_M).clamp(0.0, 1.0);
    let warmth = EMBER_COLOR.scale_rgb(strength * falloff);

    (warmth, strength)
}

fn main() {
    println!(
        "valley::look holds color and light only — the runnable scene arrives in a later card."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_color_close(a: Color, b: Color, eps: f32) {
        assert!((a.r - b.r).abs() < eps, "r: {} vs {}", a.r, b.r);
        assert!((a.g - b.g).abs() < eps, "g: {} vs {}", a.g, b.g);
        assert!((a.b - b.b).abs() < eps, "b: {} vs {}", a.b, b.b);
    }

    fn assert_vec3_close(a: Vec3, b: Vec3, eps: f32) {
        assert!((a.x - b.x).abs() < eps, "x: {} vs {}", a.x, b.x);
        assert!((a.y - b.y).abs() < eps, "y: {} vs {}", a.y, b.y);
        assert!((a.z - b.z).abs() < eps, "z: {} vs {}", a.z, b.z);
    }

    #[test]
    fn sky_at_matches_anchors_at_the_four_reference_hours() {
        let anchors = sun_anchors();
        let expected = [
            (6.0, anchors[0], SKY_DAWN),
            (12.0, anchors[1], SKY_NOON),
            (18.0, anchors[2], SKY_DUSK),
            (0.0, anchors[3], SKY_NIGHT),
        ];
        for (hour, sun, sky) in expected {
            let (light, sky_color) = sky_at(hour);
            assert_vec3_close(light.direction, sun.direction, 1e-5);
            assert_color_close(light.color, sun.color, 1e-6);
            assert!((light.intensity - sun.intensity).abs() < 1e-6);
            assert_color_close(sky_color, sky, 1e-6);
        }
    }

    #[test]
    fn sky_at_matches_the_night_anchor_at_hour_24_too() {
        let (light_0, sky_0) = sky_at(0.0);
        let (light_24, sky_24) = sky_at(24.0);
        assert_vec3_close(light_0.direction, light_24.direction, 1e-5);
        assert_color_close(sky_0, sky_24, 1e-6);
    }

    #[test]
    fn sky_at_interpolates_linearly_between_anchors() {
        let (_, midday_sky) = sky_at(9.0); // halfway between dawn (6) and noon (12)
        let expected = SKY_DAWN.lerp(SKY_NOON, 0.5);
        assert_color_close(midday_sky, expected, 1e-6);
    }

    #[test]
    fn sky_at_wraps_through_midnight() {
        // Hour 23 sits well into the dusk -> night segment; hour 1 sits
        // early in the night -> dawn segment. Both should read close to
        // the night anchor, and the two sides of midnight should agree.
        let (_, near_end_of_day) = sky_at(23.9);
        let (_, just_after_midnight) = sky_at(0.1);
        assert_color_close(near_end_of_day, just_after_midnight, 0.02);

        let (_, hour_23) = sky_at(23.0);
        let (_, hour_1) = sky_at(1.0);
        let dusk_to_night = SKY_DUSK.lerp(SKY_NIGHT, 5.0 / 6.0);
        let night_to_dawn = SKY_NIGHT.lerp(SKY_DAWN, 1.0 / 6.0);
        assert_color_close(hour_23, dusk_to_night, 1e-6);
        assert_color_close(hour_1, night_to_dawn, 1e-6);
    }

    #[test]
    fn sky_at_boundary_hours() {
        for hour in [0.0, 6.0, 12.0, 18.0, 23.0] {
            let (light, sky) = sky_at(hour);
            assert!(light.intensity > 0.0);
            assert!(sky.a > 0.0);
        }
    }

    #[test]
    fn only_the_ember_color_is_saturated() {
        let mut named: Vec<(String, Color)> = vec![
            ("SKY_DAWN".into(), SKY_DAWN),
            ("SKY_NOON".into(), SKY_NOON),
            ("SKY_DUSK".into(), SKY_DUSK),
            ("SKY_NIGHT".into(), SKY_NIGHT),
            ("SUN_DAWN".into(), SUN_DAWN),
            ("SUN_NOON".into(), SUN_NOON),
            ("SUN_DUSK".into(), SUN_DUSK),
            ("SUN_MOON".into(), SUN_MOON),
            ("EMBER_COLOR".into(), EMBER_COLOR),
        ];
        for (i, tone) in SHIRT_TONES.into_iter().enumerate() {
            named.push((format!("SHIRT_TONES[{i}]"), tone));
        }

        for (name, color) in named {
            let saturated = is_saturated_color(color);
            if name == "EMBER_COLOR" {
                assert!(saturated, "{name} should be the module's saturated color");
            } else {
                assert!(!saturated, "{name} should not read as saturated");
            }
        }
    }

    #[test]
    fn ember_glow_flickers_within_bounds_and_actually_varies() {
        let mut min_seen = f32::MAX;
        let mut max_seen = f32::MIN;
        for i in 0..200 {
            let t = i as f32 * 0.05;
            let (_, strength) = ember_glow(0.0, 0.0, t); // night: base is full
            assert!(strength >= EMBER_STRENGTH_MIN - 1e-6);
            assert!(strength <= EMBER_STRENGTH_MAX + 1e-6);
            min_seen = min_seen.min(strength);
            max_seen = max_seen.max(strength);
        }
        assert!(max_seen - min_seen > 0.01, "flicker should vary over time");
    }

    #[test]
    fn ember_glow_warmth_is_almost_zero_at_noon() {
        let (warmth, strength) = ember_glow(12.0, 0.0, 0.0);
        assert!(strength < 0.2, "noon strength should be near its floor");
        assert!(warmth.r < 0.1 && warmth.g < 0.1 && warmth.b < 0.1);
    }

    #[test]
    fn ember_glow_warmth_fades_with_distance() {
        let (near, _) = ember_glow(0.0, 0.0, 0.0);
        let (mid, _) = ember_glow(0.0, EMBER_AMBIENT_RANGE_M / 2.0, 0.0);
        let (far, _) = ember_glow(0.0, EMBER_AMBIENT_RANGE_M * 2.0, 0.0);
        assert!(near.r > mid.r && mid.r > far.r);
        assert_color_close(far, Color::rgb(0.0, 0.0, 0.0), 1e-6);
    }
}
