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
}

impl Weather {
    pub(crate) fn uniform(&self) -> [[f32; 4]; 2] {
        let c = |x: f32| x.clamp(0.0, 1.0);
        [
            [c(self.wetness), c(self.puddles), c(self.snow), c(self.rain)],
            [c(self.snowfall), 0.0, 0.0, 0.0],
        ]
    }

    /// Whether anything falls: the pass over the frame is drawn only then.
    pub fn falling(&self) -> bool {
        self.rain > 0.0 || self.snowfall > 0.0
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
