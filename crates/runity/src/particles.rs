//! Particles: the running side of a line's `particles`.
//!
//! Each emitting entity keeps its own few hundred particles as plain data
//! beside it ([`Emitting`]); [`run_particles`] moves them on and gives off
//! new ones, and [`crate::build_frame`] draws each as a small unlit cube
//! that shrinks as it ages. No GPU simulation and no textures: enough for a
//! greybox to have sparks, dust and spray, as Unity's Particle System
//! starts with. The randomness is the emitter's own, seeded from where it
//! is, so a replay gives the same sparks.

use glam::{Mat4, Quat, Vec3};

use crate::render::{Draw, MeshHandle, TextureHandle};
use crate::scene::Emitter;
use crate::world::WorldTransform;
use crate::Material;

/// The most one emitter keeps alive.
pub const MOST: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Particle {
    at: Vec3,
    velocity: Vec3,
    age: f32,
}

/// An emitter on an entity, and what it has given off.
#[derive(Debug, Clone)]
pub struct Emitting {
    pub emitter: Emitter,
    /// What each particle is drawn as.
    pub mesh: MeshHandle,
    particles: Vec<Particle>,
    /// The part of a particle owed from the last step.
    owed: f32,
    seed: u64,
}

impl Emitting {
    pub fn new(emitter: Emitter, mesh: MeshHandle) -> Self {
        Self {
            emitter,
            mesh,
            particles: Vec::new(),
            owed: 0.0,
            seed: 0x9e37_79b9_7f4a_7c15,
        }
    }

    /// How many are alive.
    pub fn count(&self) -> usize {
        self.particles.len()
    }

    fn random(&mut self) -> f32 {
        // xorshift: small, fast, and the same every run.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Move the particles on by `dt` and give off what is due, from
    /// `placed` (the entity's world matrix).
    pub fn advance(&mut self, placed: Mat4, dt: f32) {
        let e = self.emitter;
        for p in &mut self.particles {
            p.velocity.y += e.gravity * dt;
            p.at += p.velocity * dt;
            p.age += dt;
        }
        let life = e.life.max(1e-3);
        self.particles.retain(|p| p.age < life);

        let (_, turn, origin) = placed.to_scale_rotation_translation();
        self.owed += e.rate.max(0.0) * dt;
        while self.owed >= 1.0 {
            self.owed -= 1.0;
            if self.particles.len() >= MOST {
                continue;
            }
            // Within the cone about up: a random turn off the axis, a
            // random way round it.
            let off = self.random() * e.spread_deg.to_radians();
            let round = self.random() * std::f32::consts::TAU;
            let direction =
                turn * (Quat::from_rotation_y(round) * Quat::from_rotation_x(off) * Vec3::Y);
            let born = self.random() * dt;
            self.particles.push(Particle {
                at: origin + direction * e.speed * born,
                velocity: direction * e.speed,
                age: born,
            });
        }
    }

    /// Each particle as a draw: a cube `size` across when new, nothing
    /// when it dies.
    pub fn draws(&self) -> impl Iterator<Item = Draw> + '_ {
        let e = self.emitter;
        let linear = |c: f32| crate::material::srgb_to_linear(c.clamp(0.0, 1.0));
        let material =
            Material::new(linear(e.color.0), linear(e.color.1), linear(e.color.2)).unlit();
        let life = e.life.max(1e-3);
        self.particles.iter().map(move |p| Draw {
            mesh: self.mesh,
            transform: Mat4::from_scale_rotation_translation(
                Vec3::splat(e.size.max(0.0) * (1.0 - p.age / life)),
                Quat::IDENTITY,
                p.at,
            ),
            texture: TextureHandle::WHITE,
            material,
            pose: None,
        })
    }
}

/// Move every emitter's particles on: call it once a frame with the
/// frame's delta — they are for the eye, not for the simulation.
pub fn run_particles(world: &mut hecs::World, dt: f32) {
    for (placed, emitting) in world.query_mut::<(&WorldTransform, &mut Emitting)>() {
        emitting.advance(placed.0, dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sparks() -> Emitter {
        ron::from_str("(rate: 100.0, life: 0.5, speed: 2.0, spread_deg: 10.0, gravity: -4.0)")
            .unwrap()
    }

    #[test]
    fn an_emitter_gives_off_as_many_as_its_rate_and_they_die_at_their_life() {
        let mut emitting = Emitting::new(sparks(), MeshHandle::TEST);
        let at = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0));
        for _ in 0..30 {
            emitting.advance(at, 1.0 / 60.0);
        }
        // Half a second at 100 a second, and none older than half a second.
        assert!(
            (45..=51).contains(&emitting.count()),
            "{}",
            emitting.count()
        );
        for _ in 0..60 {
            emitting.advance(at, 1.0 / 60.0);
        }
        assert!(
            (45..=51).contains(&emitting.count()),
            "steady: {}",
            emitting.count()
        );
        // They went up, within the cone, and gravity bends them back.
        let draws: Vec<Draw> = emitting.draws().collect();
        assert_eq!(draws.len(), emitting.count());
        assert!(draws.iter().all(|d| d.transform.w_axis.y >= 0.99 - 0.3));
        assert!(draws.iter().any(|d| d.transform.w_axis.y > 1.2));
        let spread = draws
            .iter()
            .map(|d| Vec3::new(d.transform.w_axis.x, 0.0, d.transform.w_axis.z).length())
            .fold(0.0f32, f32::max);
        assert!(spread < 0.25, "a narrow cone: {spread}");

        // Turned over, it gives off downwards.
        let mut down = Emitting::new(sparks(), MeshHandle::TEST);
        let flipped = Mat4::from_rotation_translation(
            Quat::from_rotation_x(std::f32::consts::PI),
            Vec3::Y * 5.0,
        );
        for _ in 0..10 {
            down.advance(flipped, 1.0 / 60.0);
        }
        assert!(down.draws().all(|d| d.transform.w_axis.y <= 5.0));
    }
}
