//! Weather: rain and snow, on what they fall on and in the air.
//!
//! `weather: (rain: 1.0, wetness: 1.0, puddles: 0.6)` in a scene, or
//! `(snow: 0.8, snowfall: 0.5)`. All 0 by default.
//!
//! * **Wetness** darkens what can soak (a wet stone is darker) and makes it
//!   shine — up-facing surfaces most, as the rain lands on them.
//! * **Puddles** gather on what is level, in patches that grow with the
//!   amount: there the surface is still water — a mirror, darker below,
//!   rings spreading where drops land while it rains.
//! * **Snow** settles on what faces up, patchy at first and whole at 1,
//!   white and matte, hiding the surface's own bumps.
//! * **Rain** and **snowfall** are what falls: streaks and flakes drawn
//!   over the view, slanted by the scene's wind, anchored to where the
//!   camera looks so turning does not drag them.
//!
//! All of it is in the lit shader and one pass over the frame; nothing is
//! simulated, so it costs the same in a storm as in a drizzle.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Weather {
    /// Rain falling, 0 to 1.
    pub rain: f32,
    /// How wet everything is, 0 to 1.
    pub wetness: f32,
    /// How much of what is level is under water, 0 to 1.
    pub puddles: f32,
    /// Snow lying on what faces up, 0 to 1.
    pub snow: f32,
    /// Snow falling, 0 to 1.
    pub snowfall: f32,
    /// A sandstorm, 0 to 1: the air thick with sand rolling on the wind,
    /// the distance gone, the sun a dim disc, grains flying past.
    pub sandstorm: f32,
    /// A wall of dust coming over the land — a haboob: a front of billowing
    /// sand hundreds of metres high, bright where the sun catches its tops,
    /// dark at its foot, advancing on the wind. 0 to 1; clear air in front
    /// of it. Marched with the clouds, stopped by what stands before it.
    pub dust_wall: f32,
    /// How far upwind of the scene's origin its front starts, metres.
    pub dust_wall_distance: f32,
    /// How high it towers, metres.
    pub dust_wall_height: f32,
    /// Dust devils, 0 to 1: whirling columns of sand wandering over the
    /// ground on the wind, a few at a time, each for half a minute. Set
    /// where they are in the world, not round the camera.
    pub dust_devils: f32,
    /// Sand the wind has laid on things, 0 to 1: on what faces up, in
    /// drifts against what stands in its way — as snow lies, but sand.
    pub drifted: f32,
    /// Lightning, 0 to 1: how often a stroke comes down out of the storm.
    /// Each lights the whole scene for a moment, throwing hard shadows from
    /// where it struck, and stands in the sky as a jagged line. Set in the
    /// world by the clock, not round the camera: the same strokes at the
    /// same times on every machine.
    pub lightning: f32,
}

/// A stroke of lightning at a moment: its path from the cloud to the
/// ground, and how bright it is now.
#[derive(Debug, Clone, PartialEq)]
pub struct Bolt {
    /// Points along it, the main channel then a branch; each `w` is how
    /// bright the channel is from the point before to it, 0 where a branch
    /// begins again.
    pub points: Vec<glam::Vec4>,
    /// How bright its flash is, 0 to about 1.5 — a few strokes down the
    /// same channel, each dying away in a few hundredths of a second.
    pub flash: f32,
    /// The middle of its channel: where its light comes from.
    pub from: glam::Vec3,
}

/// The most points a bolt has: its channel and one branch.
pub const BOLT_POINTS: usize = 32;

impl Default for Weather {
    fn default() -> Self {
        Self {
            rain: 0.0,
            wetness: 0.0,
            puddles: 0.0,
            snow: 0.0,
            snowfall: 0.0,
            sandstorm: 0.0,
            dust_wall: 0.0,
            dust_wall_distance: 900.0,
            dust_wall_height: 350.0,
            dust_devils: 0.0,
            drifted: 0.0,
            lightning: 0.0,
        }
    }
}

/// The colour of sand in the air.
const SAND: [f32; 3] = [1.0, 0.56, 0.22];

impl Weather {
    pub(crate) fn uniform(&self) -> [[f32; 4]; 3] {
        let c = |x: f32| x.clamp(0.0, 1.0);
        [
            [c(self.wetness), c(self.puddles), c(self.snow), c(self.rain)],
            [c(self.snowfall), c(self.sandstorm), c(self.dust_wall), 0.0],
            [c(self.drifted), 0.0, 0.0, 0.0],
        ]
    }

    /// Whether anything falls: the pass over the frame is drawn only then.
    pub fn falling(&self) -> bool {
        self.rain > 0.0 || self.snowfall > 0.0 || self.sandstorm > 0.0
    }

    /// How fast the dust wall comes on, metres a second: with the wind.
    pub fn dust_wall_speed(wind: &crate::foliage::Wind) -> f32 {
        wind.strength.max(0.0) * 6.0
    }

    /// How far upwind of the origin the dust wall's front is at `time`:
    /// it starts at `dust_wall_distance` and comes on with the wind, past
    /// the origin and on.
    pub fn dust_front(&self, wind: &crate::foliage::Wind, time: f32) -> f32 {
        self.dust_wall_distance - Self::dust_wall_speed(wind) * time
    }

    /// The weather where the camera is. A dust wall that has reached the
    /// camera has swallowed it: inside, the storm is all round — the sand
    /// in the air, the grains flying past — as much as the camera is behind
    /// the front, eased over a few dozen metres.
    pub fn at(&self, eye: glam::Vec3, wind: &crate::foliage::Wind, time: f32) -> Weather {
        if self.dust_wall <= 0.0 {
            return *self;
        }
        let level = glam::Vec2::new(wind.direction.x, wind.direction.z).normalize_or(glam::Vec2::X);
        let along = glam::Vec2::new(eye.x, eye.z).dot(level);
        // The front's mean line; its bulges reach a little either side.
        let inside = -self.dust_front(wind, time) - along;
        let swallowed = ((inside + 40.0) / 120.0).clamp(0.0, 1.0);
        let swallowed = swallowed * swallowed * (3.0 - 2.0 * swallowed);
        Weather {
            sandstorm: self.sandstorm.max(swallowed * self.dust_wall),
            ..*self
        }
    }

    /// The dust devils about at `time`: each born somewhere in the few
    /// hundred metres round the world's middle, carried off on the wind and
    /// wandering, swelling up and dying away over half a minute. As many as
    /// `dust_devils` says, up to [`crate::volume::MOST_DEVILS`]; the same
    /// ones for the same time on every machine.
    /// The stroke of lightning coming down at `time`, if one is: in slots
    /// of seven seconds, as many of them struck as `lightning` says, each
    /// somewhere a few hundred metres round the world's middle. The same
    /// stroke for the same time on every machine.
    pub fn bolt(&self, time: f32) -> Option<Bolt> {
        let amount = self.lightning.clamp(0.0, 1.0);
        if amount <= 0.0 {
            return None;
        }
        let hash = |a: i64, b: u32| {
            let mut h = (a as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ (b as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
            h ^= h >> 31;
            h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
            h ^= h >> 29;
            (h & 0xff_ffff) as f32 / 0xff_ffff as f32
        };
        let slot = 7.0;
        let now = (time / slot).floor() as i64;
        for epoch in [now, now - 1] {
            if hash(epoch, 1) > amount * 0.85 + 0.05 {
                continue;
            }
            let start = epoch as f32 * slot + hash(epoch, 2) * (slot - 1.0);
            let t = time - start;
            if !(0.0..1.0).contains(&t) {
                continue;
            }
            // A few return strokes down the one channel, each dying away.
            let strokes = [0.0, 0.07 + 0.06 * hash(epoch, 3), 0.22 + 0.18 * hash(epoch, 4)];
            let flash: f32 = strokes
                .iter()
                .filter(|&&at| t >= at)
                .map(|&at| (-(t - at) / 0.05).exp())
                .sum::<f32>()
                .min(1.5);
            if flash < 0.01 {
                return None;
            }
            let angle = hash(epoch, 5) * std::f32::consts::TAU;
            let distance = 160.0 + 440.0 * hash(epoch, 6);
            let ground = glam::Vec3::new(angle.cos() * distance, 0.0, angle.sin() * distance);
            let top = ground
                + glam::Vec3::new(hash(epoch, 7) * 140.0 - 70.0, 420.0, hash(epoch, 8) * 140.0 - 70.0);
            let mut points = Vec::with_capacity(BOLT_POINTS);
            let main = 22;
            for i in 0..main {
                let f = i as f32 / (main - 1) as f32;
                let jag = if i == 0 || i == main - 1 { 0.0 } else { 1.0 };
                let wobble = glam::Vec3::new(
                    hash(epoch, 20 + i) - 0.5,
                    (hash(epoch, 60 + i) - 0.5) * 0.4,
                    hash(epoch, 100 + i) - 0.5,
                ) * 34.0
                    * jag;
                points.push((top.lerp(ground, f) + wobble).extend(1.0));
            }
            // A branch off a third of the way down, fainter, dying out: it
            // starts again where it leaves the channel (brightness 0 there).
            let fork = 6 + (hash(epoch, 9) * 4.0) as usize;
            let off = glam::Vec3::new(hash(epoch, 10) - 0.5, -0.6, hash(epoch, 11) - 0.5).normalize() * 150.0;
            let from = points[fork].truncate();
            points.push(from.extend(0.0));
            let branch = BOLT_POINTS - points.len();
            for i in 0..branch {
                let f = (i + 1) as f32 / branch as f32;
                let wobble = glam::Vec3::new(
                    hash(epoch, 140 + i as u32) - 0.5,
                    0.0,
                    hash(epoch, 180 + i as u32) - 0.5,
                ) * 22.0;
                points.push((from + off * f + wobble).extend((0.5 * (1.0 - f)).max(0.05)));
            }
            points.truncate(BOLT_POINTS);
            let from = top.lerp(ground, 0.5);
            return Some(Bolt { points, flash, from });
        }
        None
    }

    pub fn devils(&self, wind: &crate::foliage::Wind, time: f32) -> Vec<crate::volume::Devil> {
        let amount = self.dust_devils.clamp(0.0, 1.0);
        if amount <= 0.0 {
            return Vec::new();
        }
        let hash = |a: i64, b: u32| {
            let mut h = (a as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ (b as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
            h ^= h >> 31;
            h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
            h ^= h >> 29;
            (h & 0xff_ffff) as f32 / 0xff_ffff as f32
        };
        let level = glam::Vec2::new(wind.direction.x, wind.direction.z).normalize_or(glam::Vec2::X);
        let speed = 1.0 + 1.2 * wind.strength.max(0.0);
        let slot = 8.0;
        let now = (time / slot).floor() as i64;
        let mut out = Vec::new();
        for epoch in now - 5..=now {
            for j in 0..2u32 {
                let seed = j * 7 + 1;
                if hash(epoch, seed) > amount {
                    continue;
                }
                let born = epoch as f32 * slot + hash(epoch, seed + 1) * slot;
                let life = 25.0 + 15.0 * hash(epoch, seed + 2);
                let age = time - born;
                if !(0.0..life).contains(&age) {
                    continue;
                }
                // Born upwind, so it crosses the middle as it goes.
                let start = glam::Vec2::new(hash(epoch, seed + 3), hash(epoch, seed + 4)) * 240.0
                    - glam::Vec2::splat(120.0)
                    - level * speed * life * 0.5;
                let side = glam::Vec2::new(-level.y, level.x);
                let wander = (age * 0.35 + hash(epoch, seed + 5) * 6.0).sin() * 8.0;
                let at = start + level * speed * age + side * wander;
                let t = age / life;
                let strength = (t * std::f32::consts::PI).sin().powf(0.7);
                out.push(crate::volume::Devil {
                    position: glam::Vec3::new(at.x, 0.0, at.y),
                    radius: 2.0 + 3.0 * hash(epoch, seed + 6),
                    height: 25.0 + 35.0 * hash(epoch, seed + 7),
                    strength,
                    spin: if hash(epoch, seed + 8) > 0.5 {
                        1.0
                    } else {
                        -1.0
                    },
                });
            }
        }
        out.truncate(crate::volume::MOST_DEVILS);
        out
    }

    /// The volumetric fog a sandstorm makes of the scene's: thick sandy
    /// air, low and rolling, as much as the storm is strong.
    pub(crate) fn storm_fog(
        &self,
        fog: &crate::volume::VolumetricFog,
    ) -> crate::volume::VolumetricFog {
        let s = self.sandstorm.clamp(0.0, 1.0);
        if s <= 0.0 {
            return *fog;
        }
        let base = if fog.enabled {
            *fog
        } else {
            crate::volume::VolumetricFog {
                density: 0.0,
                ..crate::volume::VolumetricFog::OFF
            }
        };
        let mix = |a: f32, b: f32| a + (b - a) * s;
        crate::volume::VolumetricFog {
            enabled: true,
            density: mix(base.density, 0.055),
            color: [
                mix(base.color[0], SAND[0]),
                mix(base.color[1], SAND[1]),
                mix(base.color[2], SAND[2]),
            ],
            anisotropy: mix(base.anisotropy, 0.5),
            height_falloff: mix(base.height_falloff, 0.03),
            ambient: mix(base.ambient, 1.5),
            ..base
        }
    }

    /// The distance fog in a sandstorm: sand-coloured, and close.
    pub(crate) fn storm_distance(
        &self,
        fog: &crate::render::FogSettings,
    ) -> crate::render::FogSettings {
        let s = self.sandstorm.clamp(0.0, 1.0);
        if s <= 0.0 {
            return *fog;
        }
        let mix = |a: f32, b: f32| a + (b - a) * s;
        crate::render::FogSettings {
            color: fog.color.lerp(glam::Vec3::from_array(SAND) * 0.8, s),
            start: mix(fog.start, 2.0),
            end: mix(fog.end, 140.0),
            ..*fog
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lightning_strikes_now_and_then_from_the_cloud_to_the_ground_the_same_everywhere() {
        let storm = Weather {
            lightning: 1.0,
            ..Weather::default()
        };
        // A minute of frames: some strokes, each a moment long.
        let moments: Vec<f32> = (0..3600).map(|i| i as f32 / 60.0).collect();
        let lit: Vec<f32> = moments.iter().filter(|&&t| storm.bolt(t).is_some()).copied().collect();
        assert!(lit.len() > 20, "it strikes: {} frames of sixty seconds", lit.len());
        assert!(lit.len() < 600, "only for moments: {} frames", lit.len());
        let t = lit[0];
        let bolt = storm.bolt(t).unwrap();
        // From high in the cloud down to the ground.
        let top = bolt.points.first().unwrap();
        let foot = bolt.points[21];
        assert!(top.y > 380.0 && foot.y.abs() < 1.0, "{top} to {foot}");
        assert!(bolt.points.len() <= BOLT_POINTS);
        // The same stroke for the same moment.
        assert_eq!(storm.bolt(t), Some(bolt.clone()));
        // Its flash dies away within the second.
        assert!(storm.bolt(t + 1.2).is_none_or(|b| b != bolt));
        // And no storm, no lightning.
        assert!(Weather::default().bolt(t).is_none());
    }

    use super::*;

    #[test]
    fn dust_devils_are_the_same_everywhere_and_travel_downwind() {
        let wind = crate::foliage::Wind {
            direction: glam::Vec3::X,
            strength: 2.0,
        };
        let none = Weather::default();
        assert!(none.devils(&wind, 20.0).is_empty());
        let some = Weather {
            dust_devils: 1.0,
            ..Weather::default()
        };
        let now = some.devils(&wind, 20.0);
        assert!(!now.is_empty());
        assert_eq!(now, some.devils(&wind, 20.0), "the same for the same time");
        let later = some.devils(&wind, 21.0);
        // One that is still about has moved on the wind.
        let moved = now
            .iter()
            .zip(&later)
            .any(|(a, b)| b.position.x > a.position.x);
        assert!(moved, "{now:?} then {later:?}");
        let few = Weather {
            dust_devils: 0.3,
            ..Weather::default()
        };
        let counted = |w: &Weather| {
            (0..40)
                .map(|t| w.devils(&wind, t as f32 * 3.0).len())
                .sum::<usize>()
        };
        assert!(
            counted(&few) < counted(&some),
            "fewer when fewer are asked for"
        );
    }

    #[test]
    fn a_dust_wall_comes_on_and_swallows_the_camera() {
        let wind = crate::foliage::Wind {
            direction: glam::Vec3::X,
            strength: 2.0,
        };
        let wall = Weather {
            dust_wall: 1.0,
            dust_wall_distance: 600.0,
            ..Default::default()
        };
        let eye = glam::Vec3::new(0.0, 1.7, 0.0);
        assert_eq!(wall.dust_front(&wind, 0.0), 600.0);
        assert_eq!(
            wall.dust_front(&wind, 10.0),
            480.0,
            "twelve metres a second"
        );
        assert_eq!(
            wall.at(eye, &wind, 0.0).sandstorm,
            0.0,
            "far off: clear air"
        );
        assert_eq!(
            wall.at(eye, &wind, 60.0).sandstorm,
            1.0,
            "past it: inside the storm"
        );
        let arriving = wall.at(eye, &wind, 49.0).sandstorm;
        assert!(arriving > 0.0 && arriving < 1.0, "arriving: {arriving}");
        assert!(wall.at(eye, &wind, 60.0).falling(), "the sand flies");
    }

    #[test]
    fn weather_reads_from_a_scene_line_and_is_clear_by_default() {
        let w: Weather = ron::from_str("(rain: 1.0, puddles: 0.5)").unwrap();
        assert_eq!((w.rain, w.puddles, w.snow), (1.0, 0.5, 0.0));
        assert!(w.falling());
        assert!(!Weather::default().falling());
    }
}
