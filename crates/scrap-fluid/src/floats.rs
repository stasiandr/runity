//! What floats: a line's `floats`. `floats: (share: 0.4)` on a body makes
//! the water about it hold it up — shallow water, ripples or the sea: it
//! bobs, rocks as the waves pass under it, and drifts. `share` is how much
//! of it is under the water when it floats still: a log 0.6, a buoy 0.3.
//! The module says how; the facade pushes the body through the physics
//! ([`Floating::forces`]).

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

/// How a body floats, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Floats {
    /// How much of it is under water when it floats still.
    pub share: f32,
    /// How much the water holds it back, moving through it: what stops
    /// it bobbing for ever.
    pub drag: f32,
}

impl Default for Floats {
    fn default() -> Self {
        Self { share: 0.4, drag: 4.0 }
    }
}

scrap_core::impl_parts! {
    Floats => "floats", fractions ["share"];
}

/// How a line floats, read off it.
pub trait FloatsLine {
    fn floats(&self) -> Option<Floats>;
}

impl FloatsLine for scrap_core::EntityDesc {
    fn floats(&self) -> Option<Floats> {
        self.part()
    }
}

impl FloatsLine for scrap_core::scene::Override {
    fn floats(&self) -> Option<Floats> {
        self.part()
    }
}

/// A floating body: how, and the points of it the water pushes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Floating(pub Floats);

impl Floating {
    /// The points of a unit box the water pushes: four columns through its
    /// middle, at its corners drawn in a little — each lifted by as much of
    /// it as is under, so a wave under one end tips it.
    pub fn points(placed: Mat4) -> [Vec3; 4] {
        std::array::from_fn(|c| {
            let corner = Vec3::new(if c & 1 == 0 { -0.4 } else { 0.4 }, 0.0, if c & 2 == 0 { -0.4 } else { 0.4 });
            placed.transform_point3(corner)
        })
    }

    /// The force at each point, for a body of `mass` kilograms `height`
    /// metres tall, the water at `water(p)` and each point going `speed(p)`:
    /// lifted by as much of it as is under, so that under by `share` it
    /// floats; held back as it moves through.
    pub fn forces(&self, mass: f32, height: f32, points: &[Vec3], water: impl Fn(Vec3) -> Option<f32>, speed: impl Fn(Vec3) -> Vec3) -> Vec<(Vec3, Vec3)> {
        let share = self.0.share.clamp(0.05, 1.0);
        let each = mass * 9.81 / points.len() as f32;
        let height = height.max(1e-3);
        points
            .iter()
            .filter_map(|p| {
                let top = water(*p)?;
                // How much of its column is under, the column as tall as
                // the body and through its middle.
                let under = ((top - p.y) / height + 0.5).clamp(0.0, 1.0);
                if under <= 0.0 {
                    return None;
                }
                let lift = Vec3::Y * (each * under / share);
                let drag = -speed(*p) * (mass * self.0.drag.max(0.0) / points.len() as f32 * under);
                Some((*p, lift + drag))
            })
            .collect()
    }
}

/// The fluid module's dresser for what floats.
pub struct FloatsDress;

impl scrap_core::world::Dress for FloatsDress {
    fn parts(&self) -> &[&'static str] {
        &["floats"]
    }

    fn dress(
        &mut self,
        line: &scrap_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: scrap_core::world::Changed,
        _: &mut Vec<scrap_core::world::Unresolved>,
    ) {
        match line.floats() {
            Some(f) => {
                let _ = world.insert_one(entity, Floating(f));
            }
            None => {
                scrap_core::world::take_off::<Floating>(world, entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_floats_as_deep_as_its_share_says() {
        // A box a metre tall held still at each depth: the lift balances
        // its weight where it is under by its share.
        let f = Floating(Floats { share: 0.4, drag: 0.0 });
        let lift_at = |centre_y: f32| {
            let placed = Mat4::from_translation(Vec3::new(0.0, centre_y, 0.0));
            let points = Floating::points(placed);
            f.forces(10.0, 1.0, &points, |_| Some(0.0), |_| Vec3::ZERO).iter().map(|(_, force)| force.y).sum::<f32>()
        };
        let weight = 10.0 * 9.81;
        // Under by 0.4 of its height: its middle 0.1 above the water.
        assert!((lift_at(0.1) - weight).abs() < 1.0, "{} vs {weight}", lift_at(0.1));
        assert!(lift_at(-0.2) > weight, "pushed down, pushed back up");
        assert!(lift_at(0.4) < weight, "lifted out, falls back");
        assert_eq!(lift_at(2.0), 0.0);
    }
}
