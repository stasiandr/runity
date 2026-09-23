//! Footprints: what walks on soft ground leaves a trail in it, and kicks
//! up a little dust at every step.
//!
//! An entity's `footprints: (stride: 0.75)` is all it takes: as the entity
//! moves, [`run_footprints`] drops a print every `stride` metres, left and
//! right of its way by turns, and gives off a puff of dust from each. A
//! print is a decal ([`crate::decals`]) whose shape — a sole and a heel,
//! pressed in with a rim round them — is drawn by the shader, not a
//! picture: nothing to import, and it is lit, shadowed and wet like the
//! ground it is in. A puff is a ball of dust in the volumetric fog's grid
//! ([`crate::volume`]), so the sun lights it and it thins as it spreads.
//!
//! Prints fade out over `lasts` seconds, the oldest going first past a
//! limit, so a long walk costs no more than a short one. Both are for the
//! eye: they are worked out on each machine from where the walker is, and
//! nothing about them goes over the network.

use std::collections::VecDeque;

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::material::Material;
use crate::world::WorldTransform;

/// How an entity leaves prints: its line's `footprints`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Footprints {
    /// Metres from one print to the next — one foot to the other.
    pub stride: f32,
    /// How far each foot is off the line walked, metres.
    pub stance: f32,
    /// A print's length, metres; it is about 0.4 of that wide.
    pub size: f32,
    /// How deep a print is pressed, metres: a few centimetres in sand.
    pub depth: f32,
    /// Seconds a print takes to fade out.
    pub lasts: f32,
    /// How much dust each step kicks up: 0 is none, 1 dry sand.
    pub dust: f32,
    /// How far below the entity's own origin its feet are, metres: 0 for
    /// one whose origin is at its feet, about 0.9 for a capsule centred on
    /// its middle.
    pub feet: f32,
    /// The colour of the ground turned over in a print (sRGB): a little
    /// damper and darker than the top of it. The shape shows mostly by its
    /// light and shade.
    pub color: [f32; 3],
}

impl Default for Footprints {
    fn default() -> Self {
        Self {
            stride: 0.75,
            stance: 0.12,
            size: 0.28,
            depth: 0.03,
            lasts: 60.0,
            dust: 1.0,
            feet: 0.0,
            color: [0.7, 0.54, 0.36],
        }
    }
}

/// The most prints one walker keeps; past it the oldest go.
pub const MOST_PRINTS: usize = 128;
/// The most puffs one walker keeps in the air.
pub const MOST_PUFFS: usize = 8;
/// Seconds a puff of dust lasts.
const PUFF_LIFE: f32 = 1.8;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Print {
    placed: Mat4,
    age: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Puff {
    at: Vec3,
    drift: Vec3,
    age: f32,
}

/// A walker's prints and dust: the running side of [`Footprints`].
#[derive(Debug, Clone)]
pub struct Trail {
    pub settings: Footprints,
    prints: VecDeque<Print>,
    puffs: Vec<Puff>,
    /// Where its feet were at the last step, on the level.
    last: Option<Vec3>,
    /// Metres walked since the last print.
    walked: f32,
    /// Which foot is next.
    left: bool,
}

impl Trail {
    pub fn new(settings: Footprints) -> Self {
        Self {
            settings,
            prints: VecDeque::new(),
            puffs: Vec::new(),
            last: None,
            walked: 0.0,
            left: false,
        }
    }

    /// Prints lying on the ground now.
    pub fn prints(&self) -> usize {
        self.prints.len()
    }

    /// Puffs of dust in the air now.
    pub fn puffs(&self) -> usize {
        self.puffs.len()
    }

    /// Moved on to `feet` (where its feet are now) over `dt` seconds.
    pub fn advance(&mut self, feet: Vec3, dt: f32) {
        let s = self.settings;
        for print in &mut self.prints {
            print.age += dt;
        }
        let lasts = s.lasts.max(0.1);
        self.prints.retain(|p| p.age < lasts);
        for puff in &mut self.puffs {
            puff.age += dt;
            puff.at += puff.drift * dt;
            // Slowing as the air takes it.
            puff.drift *= (-1.5 * dt).exp();
        }
        self.puffs.retain(|p| p.age < PUFF_LIFE);

        let Some(last) = self.last else {
            self.last = Some(feet);
            return;
        };
        let way = Vec3::new(feet.x - last.x, 0.0, feet.z - last.z);
        let moved = way.length();
        // Further than anyone walks in a frame: put there, not walked there.
        if moved > 5.0 {
            self.last = Some(feet);
            self.walked = 0.0;
            return;
        }
        if moved < 1e-5 {
            self.last = Some(feet);
            return;
        }
        let forward = way / moved;
        let stride = s.stride.max(0.05);
        self.walked += moved;
        while self.walked >= stride {
            self.walked -= stride;
            // Where along this frame's move the step fell.
            let back = self.walked.min(moved);
            let at = feet - forward * back;
            self.step(at, forward);
        }
        self.last = Some(feet);
    }

    fn step(&mut self, at: Vec3, forward: Vec3) {
        let s = self.settings;
        let right = forward.cross(Vec3::Y).normalize_or(Vec3::X);
        let side = if self.left { -1.0 } else { 1.0 };
        self.left = !self.left;
        let foot = at + right * (s.stance * side);
        // The box: its x across the foot (turned over for the left one, so
        // the shape is a mirror of the right), its z along it, toes to −z;
        // tall enough to reach the ground on a slope under the foot.
        let turn = Quat::from_rotation_arc(Vec3::NEG_Z, forward);
        let size = s.size.max(0.02);
        let placed = Mat4::from_scale_rotation_translation(
            Vec3::new(size * 0.42 * side, 0.6, size),
            turn,
            foot,
        );
        self.prints.push_back(Print { placed, age: 0.0 });
        while self.prints.len() > MOST_PRINTS {
            self.prints.pop_front();
        }
        if s.dust > 0.0 {
            // Kicked back from the toe and up.
            self.puffs.push(Puff {
                at: foot + Vec3::Y * 0.08 - forward * 0.05,
                drift: -forward * 0.5 + Vec3::Y * 0.35,
                age: 0.0,
            });
            if self.puffs.len() > MOST_PUFFS {
                self.puffs.remove(0);
            }
        }
    }

    /// Its prints as decals, each fading over the last third of its time.
    pub fn decals(&self) -> impl Iterator<Item = crate::decals::Decal> + '_ {
        let s = self.settings;
        let lasts = s.lasts.max(0.1);
        self.prints.iter().map(move |p| {
            let left = 1.0 - p.age / lasts;
            let mut material = Material::new(s.color[0], s.color[1], s.color[2]);
            material.alpha = (left * 3.0).clamp(0.0, 1.0);
            // How deep, where the shader reads a normal map's strength.
            material.normal_scale = s.depth.max(0.0);
            material.smoothness = 0.05;
            crate::decals::Decal {
                transform: p.placed,
                material,
                shape: crate::decals::DecalShape::Footprint,
            }
        })
    }

    /// Its dust in the air: each puff swelling and thinning as it goes.
    pub fn dust(&self, sand: [f32; 3]) -> impl Iterator<Item = crate::volume::Puff> + '_ {
        let amount = self.settings.dust.max(0.0);
        self.puffs.iter().map(move |p| {
            let t = (p.age / PUFF_LIFE).clamp(0.0, 1.0);
            crate::volume::Puff {
                position: p.at,
                radius: 0.15 + 0.4 * t.sqrt(),
                // Thick at first, gone by the end.
                density: amount * 6.0 * (1.0 - t) * (1.0 - t),
                color: sand,
            }
        })
    }
}

/// Move every walker's trail on: call it once a frame with the frame's
/// delta, after the walkers have moved.
pub fn run_footprints(world: &mut hecs::World, dt: f32) {
    for (trail, placed) in world.query_mut::<(&mut Trail, &WorldTransform)>() {
        let feet = placed.0.w_axis.truncate() - Vec3::Y * trail.settings.feet;
        trail.advance(feet, dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_walk_leaves_a_print_a_stride_left_and_right_by_turns() {
        let mut trail = Trail::new(Footprints::default());
        for i in 0..=100 {
            trail.advance(Vec3::new(0.0, 0.0, -0.075 * i as f32), 1.0 / 60.0);
        }
        // 7.5 metres at 0.75 a print.
        assert_eq!(trail.prints(), 10);
        let sides: Vec<f32> = trail.prints.iter().map(|p| p.placed.w_axis.x).collect();
        for pair in sides.windows(2) {
            assert!(pair[0] * pair[1] < 0.0, "left, right, left: {sides:?}");
        }
        assert!(trail.puffs() > 0, "and dust from the last steps");
    }

    #[test]
    fn standing_still_or_being_put_somewhere_leaves_nothing() {
        let mut trail = Trail::new(Footprints::default());
        for _ in 0..100 {
            trail.advance(Vec3::ZERO, 1.0 / 60.0);
        }
        trail.advance(Vec3::new(40.0, 0.0, 0.0), 1.0 / 60.0);
        assert_eq!(trail.prints(), 0);
    }

    #[test]
    fn prints_fade_and_go_and_a_long_walk_keeps_only_so_many() {
        let mut trail = Trail::new(Footprints {
            lasts: 2.0,
            ..Footprints::default()
        });
        for i in 0..=2000 {
            trail.advance(Vec3::new(0.0, 0.0, -0.2 * i as f32), 1.0 / 1000.0);
        }
        assert_eq!(trail.prints(), MOST_PRINTS);
        trail.advance(Vec3::new(0.0, 0.0, -400.0), 3.0);
        assert_eq!(trail.prints(), 0, "all faded");
        assert_eq!(trail.puffs(), 0, "the dust settled");
    }
}
