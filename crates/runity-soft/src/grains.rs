//! Loose grains: a line's `grains`. `grains: (size: (0.5, 1.0, 0.5))`
//! fills a box round the entity with gravel, pebbles, grain or coins, let
//! go to pour and heap on what is under them (docs/simulation.md, item 8).
//!
//! The discrete element method as position-based dynamics (Macklin et al.
//! 2014, the granular part of FleX): each grain a ball; two balls that
//! overlap are pushed apart, and what they slid past each other is taken
//! back as friction allows — below the cone it holds (static), above it it
//! slides (kinetic). That is all a heap needs to stand at its angle of
//! repose, steeper the rougher its grains.

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use runity_core::world::WorldTransform;

use crate::obstacle::{Obstacle, Obstacles};
use crate::particles::Particles;

/// A pile of grains, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grains {
    /// The box they start in, metres, round the entity.
    pub size: Vec3,
    /// How big a grain is across, metres.
    pub grain: f32,
    /// How rough they are: 0 ball bearings, 1 crushed stone.
    pub friction: f32,
    /// How it goes over the network (docs/netsim.md): `Local` unless
    /// the line says.
    #[serde(default, skip_serializing_if = "runity_core::netsim::NetMode::is_local")]
    pub net: runity_core::netsim::NetMode,
}

impl Default for Grains {
    fn default() -> Self {
        Self { size: Vec3::new(0.4, 0.8, 0.4), grain: 0.06, friction: 0.7, net: runity_core::netsim::NetMode::Local }
    }
}

runity_core::impl_parts! {
    Grains => "grains";
}

/// The grains of a line, read off it.
pub trait GrainsLine {
    fn grains(&self) -> Option<Grains>;
}

impl GrainsLine for runity_core::EntityDesc {
    fn grains(&self) -> Option<Grains> {
        self.part()
    }
}

impl GrainsLine for runity_core::scene::Override {
    fn grains(&self) -> Option<Grains> {
        self.part()
    }
}

pub const STEP: f32 = 1.0 / 60.0;
pub const SUBSTEPS: usize = 6;
/// The most grains a pile has.
pub const MOST: usize = 30_000;

/// A pile as it moves: the component [`run_grains`] steps.
#[derive(Debug, Clone)]
pub struct GrainsState {
    pub grains: Grains,
    particles: Particles,
    grid: std::collections::HashMap<(i32, i32, i32), Vec<u32>>,
    placed: bool,
    owed: f32,
}

impl GrainsState {
    pub(crate) fn particles_mut(&mut self) -> &mut Particles {
        &mut self.particles
    }

    pub fn new(grains: Grains) -> Self {
        Self { grains, particles: Particles::default(), grid: Default::default(), placed: false, owed: 0.0 }
    }

    pub fn points(&self) -> &[Vec3] {
        &self.particles.x
    }

    fn fill(&mut self, placed: Mat4) {
        let d = self.grains.grain.max(0.005);
        let (scale, _, _) = placed.to_scale_rotation_translation();
        let size = self.grains.size * scale.abs();
        let n = (size / d).floor().max(Vec3::ONE);
        let centre = placed.w_axis.truncate();
        let mut x = Vec::new();
        let mut k = 7u32;
        'fill: for j in 0..n.y as usize {
            for kz in 0..n.z as usize {
                for i in 0..n.x as usize {
                    if x.len() >= MOST {
                        break 'fill;
                    }
                    k = k.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let jitter = ((k >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * d * 0.1;
                    x.push(centre - size * 0.5 + (Vec3::new(i as f32, j as f32, kz as f32) + 0.5) * d + Vec3::new(jitter, 0.0, -jitter));
                }
            }
        }
        self.particles = Particles::new(x, 1.0);
        self.placed = true;
    }

    pub fn advance(&mut self, placed: Mat4, obstacles: &[Obstacle], seconds: f32) {
        if !self.placed {
            self.fill(placed);
        }
        self.owed = (self.owed + seconds.max(0.0)).min(0.1);
        while self.owed >= STEP {
            self.owed -= STEP;
            self.step(obstacles);
        }
    }

    fn step(&mut self, obstacles: &[Obstacle]) {
        let d = self.grains.grain.max(0.005);
        let r = d * 0.5;
        let mu = self.grains.friction.clamp(0.0, 2.0);
        let h = STEP / SUBSTEPS as f32;
        // Pre-stabilisation (Macklin et al. 2014, 5.1): what overlaps before
        // the step — pushed together by something else, a cloth, a hand —
        // is put apart first, where and where it was alike, so that the
        // step does not take the push as speed and throw the grains.
        self.sort();
        for _ in 0..2 {
            self.push_apart(0.0);
        }
        self.particles.collide(r, 0.0, obstacles);
        for i in 0..self.particles.len() {
            self.particles.was[i] = self.particles.x[i];
        }
        for _ in 0..SUBSTEPS {
            self.particles.predict(h, |_| Vec3::new(0.0, -9.81, 0.0));
            self.sort();
            for _ in 0..2 {
                self.push_apart(mu);
                self.particles.collide(r, mu, obstacles);
            }
            self.particles.finish(h, 0.1);
        }
    }

    fn key(&self, p: Vec3) -> (i32, i32, i32) {
        let d = self.grains.grain.max(0.005);
        ((p.x / d).floor() as i32, (p.y / d).floor() as i32, (p.z / d).floor() as i32)
    }

    /// Each grain into the grid of cells a grain across.
    fn sort(&mut self) {
        for list in self.grid.values_mut() {
            list.clear();
        }
        for i in 0..self.particles.len() {
            let k = self.key(self.particles.x[i]);
            self.grid.entry(k).or_default().push(i as u32);
        }
    }

    /// One pass pushing apart grains that overlap, with friction `mu`.
    fn push_apart(&mut self, mu: f32) {
        let d = self.grains.grain.max(0.005);
        let n = self.particles.len();
        for i in 0..n {
            let (cx, cy, cz) = self.key(self.particles.x[i]);
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let Some(cell) = self.grid.get(&(cx + dx, cy + dy, cz + dz)) else { continue };
                        for &j in cell {
                            let j = j as usize;
                            if j <= i {
                                continue;
                            }
                            let x = &mut self.particles.x;
                            let delta = x[j] - x[i];
                            let apart = delta.length();
                            if apart >= d || apart < 1e-9 {
                                continue;
                            }
                            let normal = delta / apart;
                            let depth = d - apart;
                            x[i] -= normal * depth * 0.5;
                            x[j] += normal * depth * 0.5;
                            // Friction: of how far they slid past each other
                            // this substep, as much taken back as the push
                            // allows.
                            let was = &self.particles.was;
                            let slid = (x[j] - was[j]) - (x[i] - was[i]);
                            let along = slid - normal * slid.dot(normal);
                            let length = along.length();
                            if length > 1e-9 {
                                let hold = along * ((mu * depth / length).min(1.0) * 0.5);
                                x[i] += hold;
                                x[j] -= hold;
                            }
                        }
                    }
                }
            }
        }
    }

    /// A small ball at each grain, in the world, for `builtin:sphere`.
    pub fn placed_grains(&self) -> Vec<Mat4> {
        let d = self.grains.grain;
        self.particles.x.iter().map(|p| Mat4::from_scale_rotation_translation(Vec3::splat(d), glam::Quat::IDENTITY, *p)).collect()
    }
}

/// Step every pile by `seconds`, over `obstacles`.
pub fn run_grains(world: &mut hecs::World, seconds: f32, obstacles: &Obstacles) {
    let mut near = Vec::new();
    for (state, placed) in world.query_mut::<(&mut GrainsState, &WorldTransform)>() {
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        if state.placed {
            for p in state.points() {
                low = low.min(*p);
                high = high.max(*p);
            }
        } else {
            let c = placed.0.w_axis.truncate();
            low = c - state.grains.size * 2.0;
            high = c + state.grains.size * 2.0;
        }
        obstacles.near(low - Vec3::splat(1.0), high + Vec3::splat(1.0), &mut near);
        state.advance(placed.0, &near, seconds);
    }
}

/// The soft module's dresser for grains.
pub struct GrainsDress;

impl runity_core::world::Dress for GrainsDress {
    fn parts(&self) -> &[&'static str] {
        &["grains"]
    }

    fn dress(
        &mut self,
        line: &runity_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: runity_core::world::Changed,
        _: &mut Vec<runity_core::world::Unresolved>,
    ) {
        match line.grains() {
            Some(g) => {
                let _ = world.insert_one(entity, GrainsState::new(g));
            }
            None => {
                let _ = world.remove_one::<GrainsState>(entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A column of grains let go on the ground: how high and how wide the
    /// heap it slumps into is.
    fn heap(friction: f32) -> (f32, f32) {
        let mut state = GrainsState::new(Grains { size: Vec3::new(0.3, 0.9, 0.3), grain: 0.05, friction, ..Default::default() });
        let placed = Mat4::from_translation(Vec3::new(0.0, 0.46, 0.0));
        for _ in 0..150 {
            state.advance(placed, &[Obstacle::ground(0.0)], 1.0 / 60.0);
        }
        let p = state.points();
        let top = p.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let wide = p.iter().map(|p| Vec3::new(p.x, 0.0, p.z).length()).fold(0.0, f32::max);
        (top, wide)
    }

    #[test]
    fn rough_grains_heap_steeper_than_smooth_and_none_sink() {
        let (rough_top, rough_wide) = heap(0.9);
        let (smooth_top, smooth_wide) = heap(0.05);
        assert!(rough_top > smooth_top + 0.05, "rough stands higher: {rough_top} vs {smooth_top}");
        assert!(smooth_wide > rough_wide, "smooth runs wider: {smooth_wide} vs {rough_wide}");
        assert!(rough_top < 0.85, "but it slumped: {rough_top}");
    }
}
