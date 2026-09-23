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

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
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
}

/// The colour of sand in the air.
const SAND: [f32; 3] = [1.0, 0.56, 0.22];

impl Weather {
    pub(crate) fn uniform(&self) -> [[f32; 4]; 2] {
        let c = |x: f32| x.clamp(0.0, 1.0);
        [
            [c(self.wetness), c(self.puddles), c(self.snow), c(self.rain)],
            [c(self.snowfall), c(self.sandstorm), 0.0, 0.0],
        ]
    }

    /// Whether anything falls: the pass over the frame is drawn only then.
    pub fn falling(&self) -> bool {
        self.rain > 0.0 || self.snowfall > 0.0 || self.sandstorm > 0.0
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
    fn weather_reads_from_a_scene_line_and_is_clear_by_default() {
        let w: Weather = ron::from_str("(rain: 1.0, puddles: 0.5)").unwrap();
        assert_eq!((w.rain, w.puddles, w.snow), (1.0, 0.5, 0.0));
        assert!(w.falling());
        assert!(!Weather::default().falling());
    }
}
