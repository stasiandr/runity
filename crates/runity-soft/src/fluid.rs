//! Liquid as particles: a line's `fluid`. `fluid: (size: (1.0, 1.0,
//! 1.0))` fills a box round the entity with water, let go at once — a
//! dam breaking, a bucket tipped, a fountain's basin sloshing — to run over
//! the colliders about it.
//!
//! Two ways to solve it, both on the same neighbour grid:
//!
//! * `Pbf` (the default): position based fluids (Macklin, Müller 2013) —
//!   the density at each particle held to the water's own by a constraint
//!   solved as the soft module solves everything, with the kernels of SPH;
//!   an artificial pressure keeps it from clumping, XSPH viscosity makes
//!   it flow as one. Stable at a game's step.
//! * `Sph`: weakly compressible SPH (Becker, Teschner 2007) — pressure
//!   from how packed it is (Tait's equation), forces between neighbours,
//!   stepped explicitly: the textbook, dearer and springier.
//!
//! Drawn as its surface (`look: Surface`): the particles laid into a field
//! on a grid and the field's level turned into triangles (Surface Nets,
//! [`runity_geometry::field`]) — or as its drops (`Drops`), a small ball at
//! each particle, drawn as instances.

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use runity_core::world::WorldTransform;
use runity_geometry::mesh_asset::Vertex;

use crate::obstacle::{Obstacle, Obstacles};
use crate::particles::Particles;

/// A body of liquid, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fluid {
    /// The box of water it starts as, metres, round the entity.
    pub size: Vec3,
    /// Metres between particles: smaller is finer and dearer.
    pub spacing: f32,
    pub method: FluidMethod,
    /// How thick it is: 0.01 water, 0.3 honey.
    pub viscosity: f32,
    pub look: FluidLook,
}

/// How a fluid is solved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FluidMethod {
    #[default]
    Pbf,
    Sph,
}

/// How a fluid is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FluidLook {
    #[default]
    Surface,
    Drops,
}

impl Default for Fluid {
    fn default() -> Self {
        Self {
            size: Vec3::ONE,
            spacing: 0.08,
            method: FluidMethod::Pbf,
            viscosity: 0.02,
            look: FluidLook::Surface,
        }
    }
}

runity_core::impl_parts! {
    Fluid => "fluid";
}

/// The fluid of a line, read off it.
pub trait FluidLine {
    fn fluid(&self) -> Option<Fluid>;
}

impl FluidLine for runity_core::EntityDesc {
    fn fluid(&self) -> Option<Fluid> {
        self.part()
    }
}

impl FluidLine for runity_core::scene::Override {
    fn fluid(&self) -> Option<Fluid> {
        self.part()
    }
}

pub const STEP: f32 = 1.0 / 60.0;
/// The most particles a fluid has.
pub const MOST: usize = 20_000;

/// A fluid as it moves: the component [`run_fluids`] steps.
#[derive(Debug, Clone)]
pub struct FluidState {
    pub fluid: Fluid,
    particles: Particles,
    /// The smoothing radius, and the water's own density.
    h: f32,
    rest: f32,
    grid: runity_core::hash::FastMap<(i32, i32, i32), Vec<u32>>,
    neighbours: Vec<Vec<u32>>,
    lambda: Vec<f32>,
    density: Vec<f32>,
    placed: bool,
    owed: f32,
    /// Still: not stepped until something could move it.
    sleep: crate::rest::Rest,
    /// What was solid round it last step.
    obstacles: Vec<Obstacle>,
}

fn poly6(r2: f32, h: f32) -> f32 {
    let h2 = h * h;
    if r2 >= h2 {
        return 0.0;
    }
    315.0 / (64.0 * std::f32::consts::PI * h.powi(9)) * (h2 - r2).powi(3)
}

fn spiky_gradient(d: Vec3, h: f32) -> Vec3 {
    let r = d.length();
    if r >= h || r < 1e-9 {
        return Vec3::ZERO;
    }
    d / r * (-45.0 / (std::f32::consts::PI * h.powi(6)) * (h - r) * (h - r))
}

impl FluidState {
    pub(crate) fn particles_mut(&mut self) -> &mut Particles {
        &mut self.particles
    }

    pub fn new(fluid: Fluid) -> Self {
        let spacing = fluid.spacing.max(0.02);
        let h = spacing * 2.0;
        // The density of a particle in the middle of still water, as the
        // kernel counts it: what every particle is held to.
        let mut rest = 0.0;
        for z in -3..=3 {
            for y in -3..=3 {
                for x in -3..=3 {
                    let d = Vec3::new(x as f32, y as f32, z as f32) * spacing;
                    rest += poly6(d.length_squared(), h);
                }
            }
        }
        Self {
            fluid,
            particles: Particles::default(),
            h,
            rest,
            grid: Default::default(),
            neighbours: Vec::new(),
            lambda: Vec::new(),
            density: Vec::new(),
            placed: false,
            owed: 0.0,
            sleep: Default::default(),
            obstacles: Vec::new(),
        }
    }

    /// Where its particles are, in the world.
    pub fn points(&self) -> &[Vec3] {
        &self.particles.x
    }

    /// Its particles' speeds.
    pub fn speeds(&self) -> &[Vec3] {
        &self.particles.v
    }

    /// Filled where the entity is: a box of particles, a little jittered so
    /// the lattice does not stand.
    fn fill(&mut self, placed: Mat4) {
        let s = self.fluid.spacing.max(0.02);
        let (scale, _, _) = placed.to_scale_rotation_translation();
        let extent = self.fluid.size * scale.abs();
        let n = (extent / s).ceil().max(Vec3::ONE);
        let (nx, ny, nz) = (n.x as usize, n.y as usize, n.z as usize);
        let centre = placed.w_axis.truncate();
        let mut x = Vec::new();
        let mut k = 0u32;
        'fill: for j in 0..ny {
            for kz in 0..nz {
                for i in 0..nx {
                    if x.len() >= MOST {
                        break 'fill;
                    }
                    k = k.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    let jitter = ((k >> 16) as f32 / 65536.0 - 0.5) * s * 0.02;
                    let p = centre - extent * 0.5 + Vec3::new(i as f32 + 0.5, j as f32 + 0.5, kz as f32 + 0.5) * s;
                    x.push(p + Vec3::splat(jitter));
                }
            }
        }
        self.particles = Particles::new(x, 1.0);
        let n = self.particles.len();
        self.lambda = vec![0.0; n];
        self.density = vec![0.0; n];
        self.placed = true;
    }

    /// Along by `seconds`, over `obstacles`.
    pub fn advance(&mut self, placed: Mat4, obstacles: &[Obstacle], seconds: f32) {
        if !self.placed {
            self.fill(placed);
        }
        self.owed = (self.owed + seconds.max(0.0)).min(0.1);
        let mut changed = self.obstacles.as_slice() != obstacles;
        if changed {
            self.obstacles.clear();
            self.obstacles.extend_from_slice(obstacles);
        }
        while self.owed >= STEP {
            self.owed -= STEP;
            // Still and left alone: nothing to step.
            if self.sleep.asleep(&self.particles.x, std::mem::take(&mut changed)) {
                continue;
            }
            match self.fluid.method {
                FluidMethod::Pbf => self.step_pbf(obstacles),
                FluidMethod::Sph => self.step_sph(obstacles),
            }
            self.sleep.stepped(crate::rest::fastest(self.particles.v.iter().copied()));
        }
    }

    /// Still, and not stepped until something could move it.
    pub fn asleep(&self) -> bool {
        self.sleep.sleeping()
    }

    /// Each particle's neighbours within the smoothing radius, from a grid
    /// of cells that size.
    fn find_neighbours(&mut self) {
        let h = self.h;
        let key = |p: Vec3| ((p.x / h).floor() as i32, (p.y / h).floor() as i32, (p.z / h).floor() as i32);
        for list in self.grid.values_mut() {
            list.clear();
        }
        for (i, p) in self.particles.x.iter().enumerate() {
            self.grid.entry(key(*p)).or_default().push(i as u32);
        }
        let n = self.particles.len();
        self.neighbours.resize(n, Vec::new());
        let h2 = h * h;
        // Each particle's own list, from the grid read by all: across the
        // cores.
        let (x, grid) = (&self.particles.x, &self.grid);
        runity_core::jobs::for_each_indexed_mut(&mut self.neighbours, 128, |i, list| {
            let p = x[i];
            let (cx, cy, cz) = key(p);
            list.clear();
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if let Some(cell) = grid.get(&(cx + dx, cy + dy, cz + dz)) {
                            for &j in cell {
                                if x[j as usize].distance_squared(p) < h2 {
                                    list.push(j);
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    fn step_pbf(&mut self, obstacles: &[Obstacle]) {
        const SUBSTEPS: usize = 2;
        const ITERATIONS: usize = 4;
        let dt = STEP / SUBSTEPS as f32;
        let (h, rest) = (self.h, self.rest);
        let radius = self.fluid.spacing * 0.5;
        for _ in 0..SUBSTEPS {
            self.particles.predict(dt, |_| Vec3::new(0.0, -9.81, 0.0));
            self.find_neighbours();
            let n = self.particles.len();
            let dq = poly6((0.2 * h) * (0.2 * h), h);
            for _ in 0..ITERATIONS {
                // Each particle's density and multiplier from its
                // neighbours, then its move from theirs: both a particle to
                // itself, across the cores.
                let (x, neighbours) = (&self.particles.x, &self.neighbours);
                let found: Vec<(f32, f32)> = runity_core::jobs::map_range(n, 128, |i| {
                    let list = &neighbours[i];
                    let p = x[i];
                    let mut density = 0.0;
                    let mut grad_i = Vec3::ZERO;
                    let mut sum_grad = 0.0;
                    for &j in list {
                        let d = p - x[j as usize];
                        density += poly6(d.length_squared(), h);
                        let g = spiky_gradient(d, h) / rest;
                        grad_i += g;
                        if j as usize != i {
                            sum_grad += g.length_squared();
                        }
                    }
                    sum_grad += grad_i.length_squared();
                    // Only pushed apart, never pulled together: the free
                    // surface is not held to the water's density.
                    let c = (density / rest - 1.0).max(0.0);
                    (density, -c / (sum_grad + 100.0))
                });
                for (i, (density, lambda)) in found.into_iter().enumerate() {
                    self.density[i] = density;
                    self.lambda[i] = lambda;
                }
                let lambda = &self.lambda;
                let mut moves = vec![Vec3::ZERO; n];
                runity_core::jobs::for_each_indexed_mut(&mut moves, 128, |i, m| {
                    let p = x[i];
                    for &j in &neighbours[i] {
                        let j = j as usize;
                        if j == i {
                            continue;
                        }
                        let d = p - x[j];
                        // Artificial pressure: a little push apart that
                        // keeps drops from clumping.
                        let corr = -0.001 * (poly6(d.length_squared(), h) / dq).powi(4);
                        *m += (lambda[i] + lambda[j] + corr) * spiky_gradient(d, h);
                    }
                });
                for (i, m) in moves.iter().enumerate() {
                    self.particles.x[i] += *m / rest;
                }
                self.particles.collide(radius, 0.1, obstacles);
            }
            self.particles.finish(dt, 0.0);
            self.viscosity();
        }
    }

    /// XSPH: each particle's speed drawn toward its neighbours'.
    fn viscosity(&mut self) {
        let c = self.fluid.viscosity.clamp(0.0, 1.0);
        if c <= 0.0 {
            return;
        }
        let (h, rest) = (self.h, self.rest);
        let v = self.particles.v.clone();
        let (x, neighbours) = (&self.particles.x, &self.neighbours);
        runity_core::jobs::for_each_indexed_mut(&mut self.particles.v, 128, |i, vi| {
            let p = x[i];
            let mut sum = Vec3::ZERO;
            for &j in &neighbours[i] {
                let j = j as usize;
                let w = poly6(p.distance_squared(x[j]), h) / rest;
                sum += (v[j] - v[i]) * w;
            }
            *vi += sum * c;
        });
    }

    fn step_sph(&mut self, obstacles: &[Obstacle]) {
        const SUBSTEPS: usize = 24;
        let dt = STEP / SUBSTEPS as f32;
        let (h, rest) = (self.h, self.rest);
        let radius = self.fluid.spacing * 0.5;
        // As stiff as the step allows: a few per cent packed, springy.
        let k = 1.0;
        for _ in 0..SUBSTEPS {
            self.find_neighbours();
            let n = self.particles.len();
            // Densities, then pressures' pushes: a particle to itself,
            // across the cores.
            let (x, neighbours) = (&self.particles.x, &self.neighbours);
            runity_core::jobs::for_each_indexed_mut(&mut self.density, 128, |i, density| {
                let p = x[i];
                *density = neighbours[i]
                    .iter()
                    .map(|&j| poly6(p.distance_squared(x[j as usize]), h))
                    .sum::<f32>()
                    .max(rest * 0.5);
            });
            let density = &self.density;
            let pressure: Vec<f32> = density.iter().map(|d| k * ((d / rest).powi(7) - 1.0).max(0.0)).collect();
            runity_core::jobs::for_each_indexed_mut(&mut self.particles.v, 128, |i, v| {
                let p = x[i];
                let mut a = Vec3::new(0.0, -9.81, 0.0);
                for &j in &neighbours[i] {
                    let j = j as usize;
                    if j == i {
                        continue;
                    }
                    let d = p - x[j];
                    let (di, dj) = (density[i] / rest, density[j] / rest);
                    a -= spiky_gradient(d, h) / rest * (pressure[i] / (di * di) + pressure[j] / (dj * dj));
                }
                *v += a * dt;
            });
            for i in 0..n {
                self.particles.was[i] = self.particles.x[i];
                self.particles.x[i] += self.particles.v[i] * dt;
            }
            self.particles.collide(radius, 0.1, obstacles);
            self.particles.finish(dt, 0.0);
            self.viscosity();
        }
    }

    /// Its surface as triangles, in the space of `placed`: the particles
    /// laid into a field and the field's level drawn.
    pub fn surface(&self, placed: Mat4) -> (Vec<Vertex>, Vec<u32>) {
        if self.particles.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let s = self.fluid.spacing;
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &self.particles.x {
            low = low.min(*p);
            high = high.max(*p);
        }
        let cell = s * 0.6;
        let margin = Vec3::splat(s * 3.0);
        let origin = low - margin;
        let size = ((high - low + margin * 2.0) / cell).ceil();
        let dims = [size.x as usize + 1, size.y as usize + 1, size.z as usize + 1];
        if dims.iter().product::<usize>() > 4_000_000 {
            return (Vec::new(), Vec::new());
        }
        let mut field = runity_geometry::field::Field::new(dims, origin, cell);
        for p in &self.particles.x {
            field.splat(*p, s * 1.8, 1.0);
        }
        let (mut vertices, indices) = field.surface(0.35);
        let back = placed.inverse();
        for v in &mut vertices {
            let p = back.transform_point3(Vec3::from_array(v.position));
            let n = back.transform_vector3(Vec3::from_array(v.normal)).normalize_or(Vec3::Y);
            v.position = p.to_array();
            v.normal = n.to_array();
        }
        (vertices, indices)
    }

    /// A ball at each particle, in the world, for `builtin:sphere`.
    pub fn drops(&self) -> Vec<Mat4> {
        let size = self.fluid.spacing * 1.1;
        self.particles.x.iter().map(|p| Mat4::from_scale_rotation_translation(Vec3::splat(size), glam::Quat::IDENTITY, *p)).collect()
    }
}

/// Step every fluid by `seconds` over `obstacles`.
pub fn run_fluids(world: &mut hecs::World, seconds: f32, obstacles: &Obstacles) {
    let mut near = Vec::new();
    for (state, placed) in world.query_mut::<(&mut FluidState, &WorldTransform)>() {
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        if state.placed {
            for p in state.points() {
                low = low.min(*p);
                high = high.max(*p);
            }
        } else {
            let c = placed.0.w_axis.truncate();
            low = c - state.fluid.size * 2.0;
            high = c + state.fluid.size * 2.0;
        }
        obstacles.near(low - Vec3::splat(1.0), high + Vec3::splat(1.0), &mut near);
        state.advance(placed.0, &near, seconds);
    }
}

/// The soft module's dresser for fluids.
pub struct FluidDress;

impl runity_core::world::Dress for FluidDress {
    fn parts(&self) -> &[&'static str] {
        &["fluid"]
    }

    fn dress(
        &mut self,
        line: &runity_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: runity_core::world::Changed,
        _: &mut Vec<runity_core::world::Unresolved>,
    ) {
        match line.fluid() {
            Some(fluid) => {
                let _ = world.insert_one(entity, FluidState::new(fluid));
            }
            None => {
                let _ = world.remove_one::<FluidState>(entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tank two metres long, its walls and floor, with a column of water
    /// against one end, let go: a dam breaking.
    fn dam(method: FluidMethod, seconds: f32) -> FluidState {
        let tank = [
            Obstacle::ground(0.0),
            Obstacle::Plane { point: Vec3::new(-1.0, 0.0, 0.0), normal: Vec3::X },
            Obstacle::Plane { point: Vec3::new(1.0, 0.0, 0.0), normal: -Vec3::X },
            Obstacle::Plane { point: Vec3::new(0.0, 0.0, -0.2), normal: Vec3::Z },
            Obstacle::Plane { point: Vec3::new(0.0, 0.0, 0.2), normal: -Vec3::Z },
        ];
        let fluid = Fluid { size: Vec3::new(0.5, 0.8, 0.4), spacing: 0.08, method, ..Fluid::default() };
        let mut state = FluidState::new(fluid);
        let placed = Mat4::from_translation(Vec3::new(-0.75, 0.4, 0.0));
        for _ in 0..(seconds * 60.0) as usize {
            state.advance(placed, &tank, 1.0 / 60.0);
        }
        state
    }

    #[test]
    fn a_dam_breaks_runs_along_the_tank_and_keeps_its_volume() {
        for method in [FluidMethod::Pbf, FluidMethod::Sph] {
            // Let go, run along, splashed up the far wall and fallen back.
            let after = dam(method, 3.0);
            let p = after.points();
            // 7 × 10 × 5 particles, none lost.
            assert_eq!(p.len(), 350);
            // In the tank, all of it.
            assert!(p.iter().all(|p| p.x > -1.01 && p.x < 1.01 && p.y > -0.01 && p.z.abs() < 0.21), "{method:?}");
            // Run along it: the front far from where it stood.
            let front = p.iter().map(|p| p.x).fold(f32::MIN, f32::max);
            assert!(front > 0.3, "{method:?}: the front reached {front}");
            // And the column fallen: its top well below 0.8.
            let top = p.iter().map(|p| p.y).fold(f32::MIN, f32::max);
            assert!(top < 0.6, "{method:?}: top {top}");
            // Not squashed: it takes about the room it did.
            let mean_height = p.iter().map(|p| p.y).sum::<f32>() / p.len() as f32;
            assert!(mean_height > 0.05, "{method:?}: mean height {mean_height}");
            // And calm: nothing flying off.
            let fastest = after.speeds().iter().map(|v| v.length()).fold(0.0, f32::max);
            assert!(fastest < 8.0, "{method:?}: {fastest}");
        }
    }

    #[test]
    fn its_surface_is_a_closed_skin_round_the_water() {
        let mut still = dam(FluidMethod::Pbf, 0.0);
        still.advance(Mat4::from_translation(Vec3::new(-0.75, 0.4, 0.0)), &[Obstacle::ground(0.0)], 0.0);
        let (vertices, indices) = still.surface(Mat4::IDENTITY);
        assert!(indices.len() > 300);
        let top = vertices.iter().map(|v| v.position[1]).fold(f32::MIN, f32::max);
        assert!((top - 0.8).abs() < 0.1, "the skin's top by the water's: {top}");
        assert_eq!(still.drops().len(), still.points().len());
    }
}
