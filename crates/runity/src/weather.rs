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
}

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
        }
    }
}

/// The colour of sand in the air.
const SAND: [f32; 3] = [1.0, 0.56, 0.22];

impl Weather {
    pub(crate) fn uniform(&self) -> [[f32; 4]; 2] {
        let c = |x: f32| x.clamp(0.0, 1.0);
        [
            [c(self.wetness), c(self.puddles), c(self.snow), c(self.rain)],
            [c(self.snowfall), c(self.sandstorm), c(self.dust_wall), 0.0],
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
    use super::*;

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
