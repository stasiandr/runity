//! Smoke, fire and gas: a line's `smoke`. `smoke: (fire: true)` makes the
//! entity a fire — a campfire, a burning barrel, a chimney, steam off a
//! vent — whose smoke and heat rise into a box of air over it (`size`),
//! curl, drift in the scene's wind, go round what stands in them, and thin
//! away (docs/simulation.md, item 7).
//!
//! Stable fluids (Stam 1999) on a grid: each step the source adds smoke
//! and heat, heat lifts and smoke weighs, vorticity confinement puts back
//! the small curls the grid smooths away (Fedkiw et al. 2001), the air's
//! speed is carried along itself and made divergence-free by a pressure
//! solve, and the smoke and heat are carried along it — semi-Lagrangian,
//! unconditionally stable. The grid is sparse: it is cut into blocks of
//! 8³, and only blocks with smoke in them or next to them are stepped, as
//! OpenVDB keeps only the tiles that hold anything.
//!
//! Drawn by the render's fog: the smoke's density and heat go into a
//! volume texture the fog's cells sample as they march the light through
//! the air — lit by the sun and sky, dark where thick, and glowing where
//! it burns (`runity_render::volume::Smoke`).

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use runity_core::wind::Wind;
use runity_core::world::WorldTransform;
use runity_soft::obstacle::{Obstacle, Obstacles};

/// A fire or a smoke, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Smoke {
    /// The box of air, metres, standing on the entity.
    pub size: Vec3,
    /// Grid cells a metre.
    pub resolution: f32,
    /// How wide the source is, metres.
    pub source: f32,
    /// Smoke given off a second.
    pub rate: f32,
    /// How hot what it gives off is: how hard it rises.
    pub heat: f32,
    /// How much the smoke's weight pulls it down.
    pub weight: f32,
    /// How curly: vorticity confinement.
    pub curl: f32,
    /// How fast smoke and heat fade, a share a second.
    pub fade: f32,
    /// It burns: the hot part glows.
    pub fire: bool,
    /// The smoke's colour.
    pub color: [f32; 3],
}

impl Default for Smoke {
    fn default() -> Self {
        Self {
            size: Vec3::new(2.0, 4.0, 2.0),
            resolution: 12.0,
            source: 0.35,
            rate: 4.0,
            heat: 6.0,
            weight: 0.3,
            curl: 2.0,
            fade: 0.25,
            fire: false,
            color: [0.35, 0.34, 0.33],
        }
    }
}

runity_core::impl_parts! {
    Smoke => "smoke";
}

/// The smoke of a line, read off it.
pub trait SmokeLine {
    fn smoke(&self) -> Option<Smoke>;
}

impl SmokeLine for runity_core::EntityDesc {
    fn smoke(&self) -> Option<Smoke> {
        self.part()
    }
}

impl SmokeLine for runity_core::scene::Override {
    fn smoke(&self) -> Option<Smoke> {
        self.part()
    }
}

pub const STEP: f32 = 1.0 / 30.0;
/// Cells a side of a block.
pub const BLOCK: usize = 8;

/// A smoke as it moves: the component [`run_smokes`] steps.
#[derive(Debug, Clone)]
pub struct SmokeState {
    pub smoke: Smoke,
    /// The scene's wind: whoever spawns the scene sets it.
    pub wind: Wind,
    /// Cells along each way, and their size.
    pub n: [usize; 3],
    pub dx: f32,
    /// The box's low corner, in the world.
    pub origin: Vec3,
    velocity: Vec<Vec3>,
    pub density: Vec<f32>,
    pub heat: Vec<f32>,
    pressure: Vec<f32>,
    solid: Vec<bool>,
    solid_from: Option<Vec<Obstacle>>,
    /// Which blocks are awake.
    awake: Vec<bool>,
    placed: bool,
    owed: f32,
    time: f32,
}

impl SmokeState {
    pub fn new(smoke: Smoke) -> Self {
        let dx = 1.0 / smoke.resolution.max(2.0);
        let cells = (smoke.size / dx).ceil().max(Vec3::splat(8.0));
        // Whole blocks.
        let round = |v: f32| (v as usize).div_ceil(BLOCK) * BLOCK;
        let n = [round(cells.x), round(cells.y), round(cells.z)];
        let count = n[0] * n[1] * n[2];
        Self {
            smoke,
            wind: Wind::default(),
            n,
            dx,
            origin: Vec3::ZERO,
            velocity: vec![Vec3::ZERO; count],
            density: vec![0.0; count],
            heat: vec![0.0; count],
            pressure: vec![0.0; count],
            solid: vec![false; count],
            solid_from: None,
            awake: vec![false; (n[0] / BLOCK) * (n[1] / BLOCK) * (n[2] / BLOCK)],
            placed: false,
            owed: 0.0,
            time: 0.0,
        }
    }

    fn at(&self, i: usize, j: usize, k: usize) -> usize {
        (k * self.n[1] + j) * self.n[0] + i
    }

    /// The box it fills, in the world.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        (self.origin, self.origin + Vec3::new(self.n[0] as f32, self.n[1] as f32, self.n[2] as f32) * self.dx)
    }

    /// How many blocks are awake, of all.
    pub fn awake(&self) -> (usize, usize) {
        (self.awake.iter().filter(|a| **a).count(), self.awake.len())
    }

    /// A value of a field at a point of the grid (in cells), trilinear.
    fn sample<T: Copy + std::ops::Mul<f32, Output = T> + std::ops::Add<Output = T>>(&self, field: &[T], p: Vec3) -> T {
        let [nx, ny, nz] = self.n;
        let p = p - Vec3::splat(0.5);
        let p = p.clamp(Vec3::ZERO, Vec3::new(nx as f32 - 1.001, ny as f32 - 1.001, nz as f32 - 1.001));
        let (i, j, k) = (p.x as usize, p.y as usize, p.z as usize);
        let f = p - Vec3::new(i as f32, j as f32, k as f32);
        let v = |a: usize, b: usize, c: usize| field[self.at(a, b, c)];
        let lerp = |a: T, b: T, t: f32| a * (1.0 - t) + b * t;
        let x00 = lerp(v(i, j, k), v(i + 1, j, k), f.x);
        let x10 = lerp(v(i, j + 1, k), v(i + 1, j + 1, k), f.x);
        let x01 = lerp(v(i, j, k + 1), v(i + 1, j, k + 1), f.x);
        let x11 = lerp(v(i, j + 1, k + 1), v(i + 1, j + 1, k + 1), f.x);
        lerp(lerp(x00, x10, f.y), lerp(x01, x11, f.y), f.z)
    }

    /// Wake the blocks with smoke or heat in them, and their neighbours.
    fn wake(&mut self) {
        let [bx, by, bz] = [self.n[0] / BLOCK, self.n[1] / BLOCK, self.n[2] / BLOCK];
        let mut full = vec![false; bx * by * bz];
        for (b, flag) in full.iter_mut().enumerate() {
            let (i0, j0, k0) = (b % bx * BLOCK, b / bx % by * BLOCK, b / (bx * by) * BLOCK);
            'block: for k in k0..k0 + BLOCK {
                for j in j0..j0 + BLOCK {
                    for i in i0..i0 + BLOCK {
                        let at = self.at(i, j, k);
                        if self.density[at] > 1e-3 || self.heat[at] > 1e-3 || self.velocity[at].length_squared() > 1e-4 {
                            *flag = true;
                            break 'block;
                        }
                    }
                }
            }
        }
        for b in 0..full.len() {
            let (i, j, k) = (b % bx, b / bx % by, b / (bx * by));
            let mut any = false;
            for dk in -1i64..=1 {
                for dj in -1i64..=1 {
                    for di in -1i64..=1 {
                        let (a, bb, c) = (i as i64 + di, j as i64 + dj, k as i64 + dk);
                        if a >= 0 && bb >= 0 && c >= 0 && (a as usize) < bx && (bb as usize) < by && (c as usize) < bz {
                            any |= full[(c as usize * by + bb as usize) * bx + a as usize];
                        }
                    }
                }
            }
            self.awake[b] = any;
        }
    }

    /// Every cell of an awake block.
    fn cells(&self) -> Vec<(usize, usize, usize)> {
        let [bx, by, _] = [self.n[0] / BLOCK, self.n[1] / BLOCK, self.n[2] / BLOCK];
        let mut out = Vec::new();
        for (b, awake) in self.awake.iter().enumerate() {
            if !awake {
                continue;
            }
            let (i0, j0, k0) = (b % bx * BLOCK, b / bx % by * BLOCK, b / (bx * by) * BLOCK);
            for k in k0..k0 + BLOCK {
                for j in j0..j0 + BLOCK {
                    for i in i0..i0 + BLOCK {
                        out.push((i, j, k));
                    }
                }
            }
        }
        out
    }

    fn find_solid(&mut self, obstacles: &[Obstacle]) {
        if self.solid_from.as_deref() == Some(obstacles) {
            return;
        }
        let [nx, ny, nz] = self.n;
        for k in 0..nz {
            for j in 0..ny {
                for i in 0..nx {
                    let p = self.origin + (Vec3::new(i as f32, j as f32, k as f32) + 0.5) * self.dx;
                    let at = self.at(i, j, k);
                    self.solid[at] = obstacles.iter().any(|o| o.contact(p, 0.0).is_some());
                }
            }
        }
        self.solid_from = Some(obstacles.to_vec());
    }

    /// Along by `seconds`, standing where the entity is, round `obstacles`.
    pub fn advance(&mut self, placed: Mat4, obstacles: &[Obstacle], seconds: f32) {
        let feet = placed.w_axis.truncate();
        let size = Vec3::new(self.n[0] as f32, self.n[1] as f32, self.n[2] as f32) * self.dx;
        let origin = feet - Vec3::new(size.x * 0.5, 0.0, size.z * 0.5);
        if !self.placed || origin.distance(self.origin) > 1e-4 {
            self.origin = origin;
            self.solid_from = None;
            self.placed = true;
        }
        self.find_solid(obstacles);
        self.owed = (self.owed + seconds.max(0.0)).min(0.2);
        while self.owed >= STEP {
            self.owed -= STEP;
            self.step(STEP);
        }
    }

    fn step(&mut self, dt: f32) {
        self.time += dt;
        let [nx, ny, nz] = self.n;
        let s = self.smoke;
        // The source: a disc of cells at the bottom middle, flickering.
        let (ci, ck) = (nx as f32 * 0.5, nz as f32 * 0.5);
        let r = (s.source / self.dx).max(1.0);
        let flicker = 0.8 + 0.2 * (self.time * 13.0).sin() * (self.time * 7.3).cos();
        for k in 0..nz {
            for j in 0..(r as usize).clamp(1, 3) {
                for i in 0..nx {
                    let d = ((i as f32 + 0.5 - ci).powi(2) + (k as f32 + 0.5 - ck).powi(2)).sqrt();
                    if d < r {
                        let at = self.at(i, j + 1, k);
                        let fall = 1.0 - d / r;
                        self.density[at] = (self.density[at] + s.rate * dt * fall * flicker).min(3.0);
                        self.heat[at] = (self.heat[at] + s.heat * dt * fall * flicker).min(s.heat.max(0.1));
                    }
                }
            }
        }
        self.wake();
        let cells = self.cells();
        // Forces: heat lifts, smoke weighs, the wind blows.
        let wind = Vec3::new(self.wind.direction.x, 0.0, self.wind.direction.z).normalize_or_zero() * self.wind.strength * 1.5;
        for &(i, j, k) in &cells {
            let at = self.at(i, j, k);
            let lift = self.heat[at] * 3.0 - self.density[at] * s.weight;
            self.velocity[at].y += lift * dt;
            let toward = wind - self.velocity[at];
            // The wind takes the air, more the higher it is off the ground.
            self.velocity[at] += toward * (1.5 * dt * (0.3 + j as f32 / ny as f32)).min(1.0);
        }
        self.confine_vorticity(&cells, dt);
        // Carry the speed along itself.
        let was = self.velocity.clone();
        let inv = 1.0 / self.dx;
        for &(i, j, k) in &cells {
            let at = self.at(i, j, k);
            let p = Vec3::new(i as f32, j as f32, k as f32) + 0.5 - was[at] * dt * inv;
            self.velocity[at] = self.sample(&was, p);
        }
        self.solid_walls(&cells);
        self.project(&cells);
        // Carry smoke and heat along the speed, fading.
        let (d0, h0) = (self.density.clone(), self.heat.clone());
        let keep = (1.0 - s.fade.max(0.0) * dt).max(0.0);
        for &(i, j, k) in &cells {
            let at = self.at(i, j, k);
            if self.solid[at] {
                self.density[at] = 0.0;
                self.heat[at] = 0.0;
                continue;
            }
            let p = Vec3::new(i as f32, j as f32, k as f32) + 0.5 - self.velocity[at] * dt * inv;
            self.density[at] = self.sample(&d0, p) * keep;
            self.heat[at] = self.sample(&h0, p) * keep;
        }
    }

    /// Put back the curls: a push round each swirl's centre (Fedkiw 2001).
    fn confine_vorticity(&mut self, cells: &[(usize, usize, usize)], dt: f32) {
        let strength = self.smoke.curl.max(0.0);
        if strength <= 0.0 {
            return;
        }
        let [nx, ny, nz] = self.n;
        let v = &self.velocity;
        let get = |i: usize, j: usize, k: usize| v[(k * ny + j) * nx + i];
        let mut curl = vec![Vec3::ZERO; v.len()];
        for &(i, j, k) in cells {
            if i == 0 || j == 0 || k == 0 || i + 1 >= nx || j + 1 >= ny || k + 1 >= nz {
                continue;
            }
            let dvz_dy = get(i, j + 1, k).z - get(i, j - 1, k).z;
            let dvy_dz = get(i, j, k + 1).y - get(i, j, k - 1).y;
            let dvx_dz = get(i, j, k + 1).x - get(i, j, k - 1).x;
            let dvz_dx = get(i + 1, j, k).z - get(i - 1, j, k).z;
            let dvy_dx = get(i + 1, j, k).y - get(i - 1, j, k).y;
            let dvx_dy = get(i, j + 1, k).x - get(i, j - 1, k).x;
            curl[(k * ny + j) * nx + i] = Vec3::new(dvz_dy - dvy_dz, dvx_dz - dvz_dx, dvy_dx - dvx_dy) * (0.5 / self.dx);
        }
        let len = |i: usize, j: usize, k: usize| curl[(k * ny + j) * nx + i].length();
        for &(i, j, k) in cells {
            if i == 0 || j == 0 || k == 0 || i + 1 >= nx || j + 1 >= ny || k + 1 >= nz {
                continue;
            }
            let grad = Vec3::new(len(i + 1, j, k) - len(i - 1, j, k), len(i, j + 1, k) - len(i, j - 1, k), len(i, j, k + 1) - len(i, j, k - 1));
            let n = grad.normalize_or_zero();
            let at = (k * ny + j) * nx + i;
            self.velocity[at] += n.cross(curl[at]) * (strength * self.dx * dt);
        }
    }

    /// No air into what is solid, nor out of the box's floor.
    fn solid_walls(&mut self, cells: &[(usize, usize, usize)]) {
        for &(i, j, k) in cells {
            let at = self.at(i, j, k);
            if self.solid[at] || j == 0 {
                self.velocity[at] = Vec3::ZERO;
            }
        }
    }

    /// The air made divergence-free: a pressure solved by Jacobi over the
    /// awake cells, its gradient taken off the speed.
    fn project(&mut self, cells: &[(usize, usize, usize)]) {
        let [nx, ny, nz] = self.n;
        let v = &self.velocity;
        let get = |i: usize, j: usize, k: usize| v[(k * ny + j) * nx + i];
        let mut divergence = vec![0.0f32; v.len()];
        for &(i, j, k) in cells {
            if i == 0 || j == 0 || k == 0 || i + 1 >= nx || j + 1 >= ny || k + 1 >= nz {
                continue;
            }
            divergence[(k * ny + j) * nx + i] = (get(i + 1, j, k).x - get(i - 1, j, k).x + get(i, j + 1, k).y - get(i, j - 1, k).y + get(i, j, k + 1).z - get(i, j, k - 1).z) * 0.5;
        }
        self.pressure.fill(0.0);
        let mut next = self.pressure.clone();
        for _ in 0..24 {
            for &(i, j, k) in cells {
                if i == 0 || j == 0 || k == 0 || i + 1 >= nx || j + 1 >= ny || k + 1 >= nz {
                    continue;
                }
                let at = (k * ny + j) * nx + i;
                if self.solid[at] {
                    continue;
                }
                let p = |a: usize| if self.solid[a] { self.pressure[at] } else { self.pressure[a] };
                next[at] = (p(at - 1) + p(at + 1) + p(at - nx) + p(at + nx) + p(at - nx * ny) + p(at + nx * ny) - divergence[at]) / 6.0;
            }
            std::mem::swap(&mut self.pressure, &mut next);
        }
        for &(i, j, k) in cells {
            if i == 0 || j == 0 || k == 0 || i + 1 >= nx || j + 1 >= ny || k + 1 >= nz {
                continue;
            }
            let at = (k * ny + j) * nx + i;
            let p = &self.pressure;
            self.velocity[at] -= Vec3::new(p[at + 1] - p[at - 1], p[at + nx] - p[at - nx], p[at + nx * ny] - p[at - nx * ny]) * 0.5;
        }
    }

    /// All the smoke in it.
    pub fn total(&self) -> f32 {
        self.density.iter().sum::<f32>() * self.dx.powi(3)
    }

    /// Where the smoke is, on the mean.
    pub fn middle(&self) -> Vec3 {
        let mut sum = Vec3::ZERO;
        let mut weight = 0.0;
        for k in 0..self.n[2] {
            for j in 0..self.n[1] {
                for i in 0..self.n[0] {
                    let d = self.density[self.at(i, j, k)];
                    sum += (Vec3::new(i as f32, j as f32, k as f32) + 0.5) * self.dx * d;
                    weight += d;
                }
            }
        }
        self.origin + sum / weight.max(1e-6)
    }
}

/// Every smoke on by `seconds`, round `obstacles`.
pub fn run_smokes(world: &mut hecs::World, seconds: f32, obstacles: &Obstacles) {
    let mut near = Vec::new();
    for (state, placed) in world.query_mut::<(&mut SmokeState, &WorldTransform)>() {
        let feet = placed.0.w_axis.truncate();
        let size = state.smoke.size;
        obstacles.near(feet - Vec3::new(size.x, 0.1, size.z) * 0.6, feet + Vec3::new(size.x * 0.6, size.y, size.z * 0.6), &mut near);
        near.retain(|o| !matches!(o, Obstacle::Plane { .. }));
        state.advance(placed.0, &near, seconds);
    }
}

/// Every smoke blown by this wind.
pub fn set_wind(world: &mut hecs::World, wind: Wind) {
    for state in world.query_mut::<&mut SmokeState>() {
        state.wind = wind;
    }
}

/// The fluid module's dresser for smoke.
pub struct SmokeDress;

impl runity_core::world::Dress for SmokeDress {
    fn parts(&self) -> &[&'static str] {
        &["smoke"]
    }

    fn dress(
        &mut self,
        line: &runity_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: runity_core::world::Changed,
        _: &mut Vec<runity_core::world::Unresolved>,
    ) {
        match line.smoke() {
            Some(s) => {
                let _ = world.insert_one(entity, SmokeState::new(s));
            }
            None => {
                let _ = world.remove_one::<SmokeState>(entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(smoke: Smoke, wind: Wind, obstacles: &[Obstacle], seconds: f32) -> SmokeState {
        let mut state = SmokeState::new(smoke);
        state.wind = wind;
        for _ in 0..(seconds * 30.0) as usize {
            state.advance(Mat4::IDENTITY, obstacles, 1.0 / 30.0);
        }
        state
    }

    fn still() -> Wind {
        Wind { direction: Vec3::X, strength: 0.0 }
    }

    #[test]
    fn hot_smoke_rises_and_only_the_blocks_it_is_in_are_awake() {
        let state = run(Smoke { size: Vec3::new(6.0, 4.0, 6.0), resolution: 8.0, ..Smoke::default() }, still(), &[], 3.0);
        assert!(state.total() > 0.05, "smoke given off: {}", state.total());
        let middle = state.middle();
        // The plume's top: the highest cell with smoke worth seeing.
        let top = (0..state.n[1]).rev().find(|j| (0..state.n[2]).any(|k| (0..state.n[0]).any(|i| state.density[state.at(i, *j, k)] > 0.02))).unwrap();
        let top = (top as f32 + 0.5) * state.dx;
        assert!(top > 2.0, "risen to {top}");
        assert!(middle.x.abs() < 0.3 && middle.z.abs() < 0.3, "straight up in still air: {middle}");
        let (awake, all) = state.awake();
        assert!(awake < all, "sparse: {awake} of {all} blocks awake");
    }

    #[test]
    fn the_wind_bends_the_plume_and_a_roof_stops_it() {
        let blown = run(Smoke { resolution: 8.0, size: Vec3::new(4.0, 4.0, 2.0), ..Smoke::default() }, Wind { direction: Vec3::X, strength: 2.0 }, &[], 3.0);
        assert!(blown.middle().x > 0.3, "downwind: {}", blown.middle());
        let roof = Obstacle::Box { center: Vec3::new(0.0, 1.5, 0.0), rotation: glam::Quat::IDENTITY, half: Vec3::new(2.0, 0.1, 2.0) };
        let under = run(Smoke { resolution: 8.0, ..Smoke::default() }, still(), &[roof], 3.0);
        let above: f32 = (0..under.n[2])
            .flat_map(|k| (0..under.n[1]).flat_map(move |j| (0..under.n[0]).map(move |i| (i, j, k))))
            .filter(|(_, j, _)| (*j as f32 + 0.5) * under.dx > 1.7)
            .map(|(i, j, k)| under.density[under.at(i, j, k)])
            .sum();
        assert!(above < 0.05 * under.density.iter().sum::<f32>(), "the roof holds it: {above}");
    }
}
