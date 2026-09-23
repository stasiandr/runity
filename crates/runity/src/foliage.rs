//! Foliage that lives: trees and grass sway in the wind, leaves glow with
//! the sun behind them, and grass bends round what walks through it.
//!
//! * **Wind** is the scene's: `wind: (direction: (1.0, 0.0, 0.3), strength:
//!   1.0)` — a gentle breeze when the scene does not say. What sways, and
//!   how much, is the material's `wind` (0 for a rock, ~1 for a tree, 5 or
//!   so for grass): the vertex shader bends a thing away from the wind by its
//!   height above its own origin — curved up to a metre, so a blade bows,
//!   straight above it, so a crown sways a hand's width — rocking slowly in
//!   gusts that roll across the ground, with
//!   a quick flutter on top. The shadow passes bend it the same way, so
//!   shadows sway with what casts them.
//! * **Translucency** (the material's `translucency`, 0 to 1): a leaf or a
//!   blade of grass with the sun behind it is lit through, brightest looking
//!   straight at the sun — what makes a backlit meadow glow.
//! * **Benders**: an entity with `bends_grass: 0.8` pushes whatever sways
//!   out of the way within that many metres of it, and down — grass parts
//!   round a player. Up to [`MAX_BENDERS`] a frame, the nearest the camera.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// The most benders a frame bends by.
pub const MAX_BENDERS: usize = 8;

/// The scene's wind.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wind {
    /// Which way it blows; only its level part counts.
    pub direction: Vec3,
    /// 0 is still air; 1 a breeze; 3 a gale.
    pub strength: f32,
}

impl Default for Wind {
    /// A breeze from the west: foliage is never quite still.
    fn default() -> Self {
        Self {
            direction: Vec3::new(1.0, 0.0, 0.3),
            strength: 1.0,
        }
    }
}

/// Something that bends grass round it: where, and how far it reaches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bender {
    pub position: Vec3,
    pub radius: f32,
}

/// What the vertex shaders — the lit one and the shadow ones — need to bend
/// foliage, in the frame's uniform and each shadow pass's.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct FoliageUniform {
    /// Wind: level direction x and z, strength, time in seconds.
    pub wind: [f32; 4],
    /// Benders: position and radius, nearest first; unused slots have
    /// radius zero and bend nothing.
    pub benders: [[f32; 4]; MAX_BENDERS],
}

impl FoliageUniform {
    pub(crate) fn new(wind: &Wind, benders: &[Bender], eye: Vec3, time: f32) -> Self {
        let level = Vec3::new(wind.direction.x, 0.0, wind.direction.z).normalize_or(Vec3::X);
        let mut near: Vec<&Bender> = benders.iter().filter(|b| b.radius > 0.0).collect();
        near.sort_by(|a, b| {
            (a.position - eye)
                .length_squared()
                .total_cmp(&(b.position - eye).length_squared())
        });
        let mut out = [[0.0; 4]; MAX_BENDERS];
        for (slot, bender) in out.iter_mut().zip(near) {
            *slot = [
                bender.position.x,
                bender.position.y,
                bender.position.z,
                bender.radius,
            ];
        }
        Self {
            wind: [level.x, level.z, wind.strength.max(0.0), time],
            benders: out,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_benders_are_kept_and_the_wind_is_levelled() {
        let benders: Vec<Bender> = (0..12)
            .map(|i| Bender {
                position: Vec3::new(i as f32, 0.0, 0.0),
                radius: 1.0,
            })
            .collect();
        let u = FoliageUniform::new(
            &Wind {
                direction: Vec3::new(0.0, 5.0, 2.0),
                strength: 2.0,
            },
            &benders,
            Vec3::new(11.0, 0.0, 0.0),
            3.5,
        );
        assert_eq!(u.wind, [0.0, 1.0, 2.0, 3.5]);
        assert_eq!(u.benders[0][0], 11.0, "nearest first");
        assert_eq!(u.benders[MAX_BENDERS - 1][0], 4.0);
    }

    #[test]
    fn wind_reads_from_a_scene_line() {
        let w: Wind = ron::from_str("(strength: 2.5)").unwrap();
        assert_eq!(w.strength, 2.5);
        assert_eq!(w.direction, Wind::default().direction);
    }
}
