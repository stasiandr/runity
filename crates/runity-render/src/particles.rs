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
    /// The emitter's material, found when it was spawned; `None` draws the
    /// plain colour.
    pub material: Option<Material>,
    particles: Vec<Particle>,
    /// Where the emitter was at the last step: what local particles are
    /// drawn from, and what a burst from code leaves from.
    placed: Mat4,
    /// The part of a particle owed from the last step.
    owed: f32,
    seed: u64,
    /// Seconds into the play; `None` when not playing (it waits, or a
    /// once-only play is over).
    clock: Option<f32>,
    /// The bursts of this round already given off.
    fired: usize,
    /// For one on the GPU: how many it has given off, ever, and seconds it
    /// has run — what the GPU spawns and steps by.
    gpu_born: u64,
    lived: f32,
}

impl Emitting {
    pub fn new(emitter: Emitter, mesh: MeshHandle) -> Self {
        let emitter_waits = emitter.waits;
        Self {
            emitter,
            mesh,
            material: None,
            particles: Vec::new(),
            placed: Mat4::IDENTITY,
            owed: 0.0,
            gpu_born: 0,
            lived: 0.0,
            seed: 0x9e37_79b9_7f4a_7c15,
            clock: (!emitter_waits).then_some(0.0),
            fired: 0,
        }
    }

    /// Start a play from its beginning: its bursts go off again. What
    /// is already in the air stays. Unity's `ParticleSystem.Play`.
    pub fn play(&mut self) {
        self.clock = Some(0.0);
        self.owed = 0.0;
        self.fired = 0;
    }

    /// Stop giving off; what is in the air lives out its life.
    pub fn stop(&mut self) {
        self.clock = None;
    }

    /// Whether it is giving off, or would be at its rate.
    pub fn is_playing(&self) -> bool {
        self.clock.is_some()
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
        self.placed = placed;
        self.lived += dt.max(0.0);
        let gravity = self.emitter.gravity;
        let local = self.emitter.local;
        // Gravity pulls down the world, whichever way a local emitter is
        // turned.
        let (_, turn, _) = placed.to_scale_rotation_translation();
        let down = if local {
            turn.inverse() * Vec3::Y
        } else {
            Vec3::Y
        };
        for p in &mut self.particles {
            p.velocity += down * gravity * dt;
            p.at += p.velocity * dt;
            p.age += dt;
        }
        let life = self.emitter.life.max(1e-3);
        self.particles.retain(|p| p.age < life);
        let Some(clock) = self.clock else { return };
        let duration = self.emitter.duration.max(1e-3);
        let now = clock + dt;
        self.owed += self.emitter.rate.max(0.0) * (now.min(duration) - clock).max(0.0);
        let due = self.owed.floor() as usize;
        self.owed -= due as f32;
        self.give_off(due, dt);
        // Bursts in order of their time, each once a round.
        let mut bursts = self.emitter.bursts.clone();
        bursts.sort_by(|a, b| a.0.total_cmp(&b.0));
        while let Some(&(_, count)) = bursts.get(self.fired).filter(|(at, _)| *at <= now) {
            self.fired += 1;
            self.give_off(count as usize, 0.0);
        }
        self.clock = if now < duration {
            Some(now)
        } else if self.emitter.once {
            None
        } else {
            // Round again: what is left of the step counts into it.
            for &(at, count) in &bursts[self.fired.min(bursts.len())..] {
                if at <= duration {
                    self.give_off(count as usize, 0.0);
                }
            }
            self.fired = 0;
            Some(now - duration)
        };
    }

    /// Give off `count` now, from where the emitter was at its last step:
    /// a burst from code — the puff when a spade goes in. Unity's
    /// `ParticleSystem.Emit`.
    pub fn emit(&mut self, count: usize) {
        self.give_off(count, 0.0);
    }

    fn give_off(&mut self, count: usize, dt: f32) {
        // On the GPU they are only counted: the GPU gives them off.
        if self.emitter.gpu {
            self.gpu_born += count as u64;
            return;
        }
        let e = self.emitter.clone();
        let (_, turn, origin) = self.placed.to_scale_rotation_translation();
        for _ in 0..count {
            if self.particles.len() >= MOST {
                break;
            }
            // Within the cone about up: a random turn off the axis, a
            // random way round it.
            let off = self.random() * e.spread_deg.to_radians();
            let round = self.random() * std::f32::consts::TAU;
            let axis = e.direction.map_or(Quat::IDENTITY, |d| {
                Quat::from_rotation_arc(Vec3::Y, d.normalize_or(Vec3::Y))
            });
            let along = axis * Quat::from_rotation_y(round) * Quat::from_rotation_x(off) * Vec3::Y;
            let (from, direction) = if e.local {
                (Vec3::ZERO, along)
            } else {
                (origin, turn * along)
            };
            // Given off some time within the step: as far on as it would
            // have got by now, gravity too — so what falls in one step is a
            // stream, not a bead.
            let born = self.random() * dt;
            let down = if e.local {
                turn.inverse() * Vec3::Y
            } else {
                Vec3::Y
            };
            let pull = down * e.gravity;
            self.particles.push(Particle {
                at: from + direction * e.speed * born + pull * (0.5 * born * born),
                velocity: direction * e.speed + pull * born,
                age: born,
            });
        }
    }

    /// Each particle as a draw: `size` across when new, `end_size` (or
    /// nothing) when it dies, its colour going from `color` to `end_color`,
    /// drawn longer along its way the faster it goes when it stretches.
    pub fn draws(&self) -> impl Iterator<Item = Draw> + '_ {
        self.draws_facing(None)
    }

    /// [`Self::draws`] for a camera at `eye`: a `facing` emitter's squares
    /// turn to it (stretched ones along their way, as seen from it).
    pub fn draws_facing(&self, eye: Option<Vec3>) -> impl Iterator<Item = Draw> + '_ {
        let e = &self.emitter;
        let linear = |c: (f32, f32, f32)| {
            let l = |v: f32| crate::material::srgb_to_linear(v.clamp(0.0, 1.0));
            Vec3::new(l(c.0), l(c.1), l(c.2))
        };
        let (start, end) = (linear(e.color), linear(e.end_color.unwrap_or(e.color)));
        let life = e.life.max(1e-3);
        let placed = self.placed;
        let (_, turn, _) = placed.to_scale_rotation_translation();
        self.particles.iter().map(move |p| {
            let t = (p.age / life).clamp(0.0, 1.0);
            let size = match e.end_size {
                Some(end) => e.size + (end - e.size) * t,
                None => e.size * (1.0 - t),
            }
            .max(0.0);
            let (at, velocity) = if e.local {
                (placed.transform_point3(p.at), turn * p.velocity)
            } else {
                (p.at, p.velocity)
            };
            let to_eye = eye.map(|eye| eye - at).filter(|d| d.length() > 1e-4);
            let (scale, rotation) = if let Some(n) = to_eye.filter(|_| e.facing) {
                // The square's normal (its y) to the eye; its z along the
                // way it goes, as the eye sees it.
                let n = n.normalize();
                let seen = velocity - n * velocity.dot(n);
                let (z, long) = if e.stretch > 0.0 && seen.length() > 1e-4 {
                    (seen.normalize(), 1.0 + velocity.length() * e.stretch)
                } else {
                    (n.any_orthonormal_vector(), 1.0)
                };
                let x = n.cross(z);
                (
                    Vec3::new(size, size, size * long),
                    Quat::from_mat3(&glam::Mat3::from_cols(x, n, z)),
                )
            } else if e.stretch > 0.0 && velocity.length() > 1e-4 {
                (
                    Vec3::new(size, size * (1.0 + velocity.length() * e.stretch), size),
                    Quat::from_rotation_arc(Vec3::Y, velocity.normalize()),
                )
            } else {
                (Vec3::splat(size), Quat::IDENTITY)
            };
            let tint = start + (end - start) * t;
            let alpha = e.alpha + (e.end_alpha.unwrap_or(e.alpha) - e.alpha) * t;
            let fading = e.alpha < 1.0 || e.end_alpha.is_some_and(|a| a < 1.0);
            let mut material = match self.material {
                Some(mut m) => {
                    m.base_color = [
                        m.base_color[0] * tint.x,
                        m.base_color[1] * tint.y,
                        m.base_color[2] * tint.z,
                    ];
                    m
                }
                None => Material::new(tint.x, tint.y, tint.z).unlit(),
            };
            if fading {
                material.surface = crate::material::SurfaceType::Transparent;
                material.alpha *= alpha.clamp(0.0, 1.0);
            }
            Draw {
                mesh: self.mesh,
                transform: Mat4::from_scale_rotation_translation(scale, rotation, at),
                texture: TextureHandle::WHITE,
                material,
                pose: None,
            }
        })
    }
}

impl Emitting {
    /// What the GPU needs of an emitter on it this frame: none for one on
    /// the CPU.
    pub fn gpu(&self, key: u64) -> Option<crate::particles_gpu::GpuEmitter> {
        self.emitter.gpu.then(|| crate::particles_gpu::GpuEmitter {
            key,
            placed: self.placed,
            emitter: self.emitter.clone(),
            born: self.gpu_born,
            lived: self.lived,
        })
    }
}

/// Move every emitter's particles on: call it once a frame with the
/// frame's delta — they are for the eye, not for the simulation. A
/// switched-off emitter waits: switched on, it starts from nothing.
pub fn run_particles(world: &mut hecs::World, dt: f32) {
    for (placed, emitting) in world
        .query_mut::<(&WorldTransform, &mut Emitting)>()
        .without::<&crate::world::Inactive>()
    {
        emitting.advance(placed.0, dt);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_on_the_gpu_counts_what_it_gives_off_and_draws_nothing_itself() {
        let mut emitting = Emitting::new(
            Emitter {
                rate: 1000.0,
                gpu: true,
                ..Emitter::default()
            },
            MeshHandle::TEST,
        );
        for _ in 0..30 {
            emitting.advance(Mat4::IDENTITY, 1.0 / 30.0);
        }
        let gpu = emitting.gpu(3).expect("on the gpu");
        assert!((980..=1000).contains(&gpu.born), "a second of them: {}", gpu.born);
        assert!((gpu.lived - 1.0).abs() < 1e-3);
        assert_eq!(emitting.draws().count(), 0, "none drawn on the CPU");
        // And one on the CPU says nothing to the GPU.
        let cpu = Emitting::new(Emitter::default(), MeshHandle::TEST);
        assert!(cpu.gpu(3).is_none());
    }

    use super::*;

    #[test]
    fn a_once_only_effect_bursts_when_played_and_then_stops() {
        let mut e = sparks();
        e.rate = 0.0;
        e.once = true;
        e.waits = true;
        e.duration = 0.5;
        e.bursts = vec![(0.0, 30), (0.2, 10)];
        e.life = 5.0;
        let mut emitting = Emitting::new(e, MeshHandle::TEST);
        let step = |em: &mut Emitting, n: usize| {
            for _ in 0..n {
                em.advance(Mat4::IDENTITY, 0.05);
            }
        };
        step(&mut emitting, 10);
        assert_eq!(emitting.count(), 0, "it waits to be played");
        emitting.play();
        step(&mut emitting, 1);
        assert_eq!(emitting.count(), 30);
        step(&mut emitting, 20);
        assert_eq!(emitting.count(), 40, "the second burst, and no more");
        assert!(!emitting.is_playing());
        emitting.play();
        step(&mut emitting, 1);
        assert_eq!(emitting.count(), 70, "played again, it bursts again");
    }

    #[test]
    fn a_looping_emitter_bursts_every_round() {
        let mut e = sparks();
        e.rate = 0.0;
        e.duration = 1.0;
        e.bursts = vec![(0.0, 5)];
        e.life = 100.0;
        let mut emitting = Emitting::new(e, MeshHandle::TEST);
        for _ in 0..100 {
            emitting.advance(Mat4::IDENTITY, 0.025);
        }
        // 2.5 s: rounds starting at 0, 1 and 2.
        assert_eq!(emitting.count(), 15);
    }

    #[test]
    fn particles_thin_to_nothing_with_an_end_alpha() {
        let mut e = sparks();
        e.end_alpha = Some(0.0);
        e.life = 1.0;
        let mut emitting = Emitting::new(e, MeshHandle::TEST);
        emitting.emit(1);
        emitting.advance(Mat4::IDENTITY, 0.5);
        let d = emitting.draws().next().unwrap();
        assert!(d.material.is_transparent());
        assert!(
            (d.material.alpha - 0.5).abs() < 0.05,
            "{}",
            d.material.alpha
        );
    }

    #[test]
    fn a_facing_particle_turns_its_square_to_the_eye() {
        let mut e = sparks();
        e.facing = true;
        e.gravity = 0.0;
        let mut emitting = Emitting::new(e, MeshHandle::TEST);
        emitting.advance(Mat4::IDENTITY, 0.1);
        let eye = Vec3::new(0.0, 0.0, 10.0);
        for d in emitting.draws_facing(Some(eye)) {
            let normal = d.transform.transform_vector3(Vec3::Y).normalize();
            let to_eye = (eye - d.transform.w_axis.truncate()).normalize();
            assert!(normal.dot(to_eye) > 0.999, "{normal} {to_eye}");
        }
    }

    #[test]
    fn a_local_emitter_carries_its_particles_and_a_burst_comes_from_code() {
        let mut e = sparks();
        e.local = true;
        e.rate = 0.0;
        e.gravity = 0.0;
        e.end_size = Some(0.5);
        e.stretch = 1.0;
        let mut emitting = Emitting::new(e, MeshHandle::TEST);
        emitting.advance(Mat4::IDENTITY, 1.0 / 60.0);
        assert_eq!(emitting.count(), 0, "no rate, none");
        emitting.emit(20);
        assert_eq!(emitting.count(), 20);
        emitting.advance(Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)), 0.1);
        // Moved with the emitter: all near x = 10, not left at 0.
        assert!(emitting
            .draws()
            .all(|d| (d.transform.w_axis.x - 10.0).abs() < 1.0));
        // Stretched along their way: taller than wide.
        let d = emitting.draws().next().unwrap();
        let (scale, _, _) = d.transform.to_scale_rotation_translation();
        assert!(scale.y > scale.x * 1.5, "{scale}");
    }

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
