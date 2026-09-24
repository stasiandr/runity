//! Cloth: a banner, a flag, a sheet on a line, an awning, a tablecloth —
//! a line's `cloth`. `cloth: (size: (2.0, 3.0), pinned: Top)` hangs a sheet
//! from the entity, in its material; `pinned: Free` lays it level to fall
//! onto what is under it.
//!
//! A grid of particles held to its neighbours by XPBD distance constraints
//! — across and down (it does not stretch), the diagonals (it hardly
//! shears), and two apart (it bends as stiffly as it is told, and does not
//! crease like paper). The wind pushes each triangle as much as it faces
//! it, with gusts rolling through it and turbulence across it. It lies on
//! the colliders as a rope does, and with `self_collide` it does not pass
//! through itself: particles near each other, not neighbours in the grid,
//! are held apart.
//!
//! Only a look: every machine flaps its own (DNA, postulate 4). Its steps
//! are fixed, so the same wind flaps it the same way.

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use scrap_core::wind::Wind;
use scrap_core::world::WorldTransform;
use scrap_geometry::mesh_asset::Vertex;

use crate::obstacle::{Obstacle, Obstacles};
use crate::particles::{distance, Particles};

/// A sheet of cloth, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cloth {
    /// Metres: across (the entity's x) and down (its −y) — or, lying
    /// level (`Corners`, `Free`), across and deep (its −z).
    pub size: [f32; 2],
    /// Particles across and down.
    pub cells: [u32; 2],
    /// What holds it up.
    pub pinned: Pinned,
    /// How much the wind takes it: 1 a flag, less a heavy canvas.
    pub catch: f32,
    /// How much it holds its shape against bending, 0 (silk) to 1 (stiff
    /// canvas).
    pub stiffness: f32,
    /// Kilograms a square metre.
    pub weight: f32,
    /// How thick, metres: how far it keeps from what it lies on, and from
    /// itself.
    pub thickness: f32,
    /// It does not pass through itself — dearer; a flag has no need.
    pub self_collide: bool,
    /// How it goes over the network (docs/netsim.md): `Local` unless
    /// the line says.
    #[serde(default, skip_serializing_if = "scrap_core::netsim::NetMode::is_local")]
    pub net: scrap_core::netsim::NetMode,
}

/// What holds a cloth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Pinned {
    /// Its whole top edge: a banner, a curtain.
    #[default]
    Top,
    /// Its two top corners: a sheet on a line, sagging between.
    TopCorners,
    /// Its left edge: a flag on a pole.
    Left,
    /// Its four corners, lying level: an awning, bellying in the wind.
    Corners,
    /// Nothing: laid level where the entity is, it falls onto what is
    /// under it — a tablecloth, a tarpaulin thrown over a crate.
    Free,
}

impl Default for Cloth {
    fn default() -> Self {
        Self {
            size: [2.0, 2.0],
            cells: [16, 16],
            pinned: Pinned::Top,
            catch: 1.0,
            stiffness: 0.2,
            weight: 0.3,
            thickness: 0.01,
            self_collide: false,
            net: scrap_core::netsim::NetMode::Local,
        }
    }
}

scrap_core::impl_parts! {
    Cloth => "cloth", fractions ["stiffness"];
}

/// The cloth of a line, read off it.
pub trait ClothLine {
    fn cloth(&self) -> Option<Cloth>;
}

impl ClothLine for scrap_core::EntityDesc {
    fn cloth(&self) -> Option<Cloth> {
        self.part()
    }
}

impl ClothLine for scrap_core::scene::Override {
    fn cloth(&self) -> Option<Cloth> {
        self.part()
    }
}

/// Seconds a step, and substeps a step.
pub const STEP: f32 = 1.0 / 60.0;
pub const SUBSTEPS: usize = 8;
/// Metres a second of wind at `strength: 1`, as the physics' wind.
const WIND_SPEED: f32 = 4.0;

/// Which kind of hold a link is: how much it gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    Stretch,
    Shear,
    Bend,
}

/// A cloth as it moves: the component [`run_cloth`] steps.
#[derive(Debug, Clone)]
pub struct ClothState {
    pub cloth: Cloth,
    /// Its summary over the network, when it is `Rough` (docs/netsim.md).
    pub rough: crate::net::Rough,
    /// The scene's wind: whoever spawns the scene sets it.
    pub wind: Wind,
    across: usize,
    down: usize,
    particles: Particles,
    /// Each particle's place on the entity at rest.
    rest: Vec<Vec3>,
    pinned: Vec<bool>,
    links: Vec<(u32, u32, f32, Hold)>,
    held: Vec<f32>,
    /// Pairs near enough to touch this step that are not neighbours.
    close: Vec<(u32, u32)>,
    triangles: Vec<[u32; 3]>,
    placed: Option<Mat4>,
    near: Vec<Obstacle>,
    pull: Vec<Vec3>,
    time: f32,
    owed: f32,
}

impl ClothState {
    pub(crate) fn particles_mut(&mut self) -> &mut Particles {
        &mut self.particles
    }

    pub fn new(cloth: Cloth) -> Self {
        let across = cloth.cells[0].clamp(2, 128) as usize;
        let down = cloth.cells[1].clamp(2, 128) as usize;
        let (w, h) = (cloth.size[0].max(0.01), cloth.size[1].max(0.01));
        let level = matches!(cloth.pinned, Pinned::Corners | Pinned::Free);
        let mut rest = Vec::with_capacity(across * down);
        let mut pinned = Vec::with_capacity(across * down);
        for j in 0..down {
            for i in 0..across {
                let u = i as f32 / (across - 1) as f32;
                let v = j as f32 / (down - 1) as f32;
                let x = (u - 0.5) * w;
                rest.push(if level {
                    Vec3::new(x, 0.0, (0.5 - v) * h)
                } else {
                    Vec3::new(x, -v * h, 0.0)
                });
                let (left, right, top, bottom) = (i == 0, i == across - 1, j == 0, j == down - 1);
                pinned.push(match cloth.pinned {
                    Pinned::Top => top,
                    Pinned::TopCorners => top && (left || right),
                    Pinned::Left => left,
                    Pinned::Corners => (top || bottom) && (left || right),
                    Pinned::Free => false,
                });
            }
        }
        let at = |i: usize, j: usize| (j * across + i) as u32;
        let mut links = Vec::new();
        let mut link = |a: u32, b: u32, hold: Hold| {
            links.push((a, b, rest[a as usize].distance(rest[b as usize]), hold));
        };
        for j in 0..down {
            for i in 0..across {
                if i + 1 < across {
                    link(at(i, j), at(i + 1, j), Hold::Stretch);
                }
                if j + 1 < down {
                    link(at(i, j), at(i, j + 1), Hold::Stretch);
                }
                if i + 1 < across && j + 1 < down {
                    link(at(i, j), at(i + 1, j + 1), Hold::Shear);
                    link(at(i + 1, j), at(i, j + 1), Hold::Shear);
                }
                if i + 2 < across {
                    link(at(i, j), at(i + 2, j), Hold::Bend);
                }
                if j + 2 < down {
                    link(at(i, j), at(i, j + 2), Hold::Bend);
                }
            }
        }
        let mut triangles = Vec::with_capacity((across - 1) * (down - 1) * 2);
        for j in 0..down - 1 {
            for i in 0..across - 1 {
                let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
                triangles.push([a, c, b]);
                triangles.push([b, c, d]);
            }
        }
        let area = w * h / ((across - 1) * (down - 1)) as f32;
        let mut particles = Particles::new(rest.clone(), (cloth.weight.max(0.01) * area).max(1e-4));
        for (k, held) in pinned.iter().enumerate() {
            if *held {
                particles.w[k] = 0.0;
            }
        }
        Self { rough: Default::default(),
            cloth,
            wind: Wind::default(),
            across,
            down,
            held: vec![0.0; links.len()],
            particles,
            rest,
            pinned,
            links,
            close: Vec::new(),
            triangles,
            placed: None,
            near: Vec::new(),
            pull: Vec::new(),
            time: 0.0,
            owed: 0.0,
        }
    }

    /// Where its particles are, in the world; where it rests on the entity
    /// before it is first stepped.
    pub fn points(&self) -> &[Vec3] {
        &self.particles.x
    }

    /// Particles across and down.
    pub fn grid(&self) -> (usize, usize) {
        (self.across, self.down)
    }

    /// Hung where the entity is, as it rests.
    fn place(&mut self, placed: Mat4) {
        for (k, r) in self.rest.iter().enumerate() {
            self.particles.place(k, placed.transform_point3(*r));
        }
        self.placed = Some(placed);
    }

    /// Along by `seconds`, where the entity now is, in `wind`, lying on
    /// `obstacles`.
    pub fn advance(&mut self, placed: Mat4, wind: &Wind, obstacles: &[Obstacle], seconds: f32) {
        if self.placed.is_none() {
            self.place(placed);
        }
        self.owed = (self.owed + seconds.max(0.0)).min(0.25);
        while self.owed >= STEP {
            self.owed -= STEP;
            self.step(placed, wind, obstacles);
        }
    }

    fn step(&mut self, placed: Mat4, wind: &Wind, obstacles: &[Obstacle]) {
        let was = self.placed.unwrap_or(placed);
        self.placed = Some(placed);
        let radius = self.cloth.thickness.max(0.001);
        // What is near enough to touch this step.
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &self.particles.x {
            low = low.min(*p);
            high = high.max(*p);
        }
        let fastest = self.particles.v.iter().map(|v| v.length()).fold(0.0, f32::max);
        let room = Vec3::splat(radius + fastest * STEP + 0.25);
        self.near.clear();
        self.near.extend(obstacles.iter().filter(|o| o.near(low - room, high + room)).cloned());
        if self.cloth.self_collide {
            self.find_close(radius * 2.0 + fastest * STEP);
        }
        let h = STEP / SUBSTEPS as f32;
        let bend = {
            let s = self.cloth.stiffness.clamp(0.0, 1.0);
            if s >= 1.0 { 0.0 } else { 1e-2 * (1.0 / s.max(1e-4) - 1.0) }
        };
        let level = Vec3::new(wind.direction.x, 0.0, wind.direction.z).normalize_or_zero();
        let speed = WIND_SPEED * wind.strength.max(0.0);
        let catch = self.cloth.catch.max(0.0) * 0.3 / self.cloth.weight.max(0.01);
        for s in 0..SUBSTEPS {
            let t = (s + 1) as f32 / SUBSTEPS as f32;
            self.time += h;
            // The pinned go where the entity is going, a share a substep.
            for (k, r) in self.rest.iter().enumerate() {
                if self.pinned[k] {
                    let (a, b) = (was.transform_point3(*r), placed.transform_point3(*r));
                    self.particles.x[k] = a.lerp(b, t);
                }
            }
            self.air(level, speed, catch);
            let pull = &self.pull;
            self.particles.predict(h, |i| Vec3::new(0.0, -9.81, 0.0) + pull[i]);
            self.held.fill(0.0);
            for (n, &(a, b, rest, hold)) in self.links.iter().enumerate() {
                let give = match hold {
                    Hold::Stretch => 0.0,
                    Hold::Shear => 1e-6,
                    Hold::Bend => bend,
                };
                self.held[n] = distance(&mut self.particles, a as usize, b as usize, rest, give, h, self.held[n]);
            }
            // Apart from itself: two particles closer than twice its
            // thickness are pushed to it.
            for &(a, b) in &self.close {
                let (a, b) = (a as usize, b as usize);
                let d = self.particles.x[b] - self.particles.x[a];
                let apart = d.length();
                let least = radius * 2.0;
                let sum = self.particles.w[a] + self.particles.w[b];
                if apart >= least || apart < 1e-9 || sum <= 0.0 {
                    continue;
                }
                let n = d / apart;
                let depth = least - apart;
                let push = n * (depth / sum);
                self.particles.x[a] -= push * self.particles.w[a];
                self.particles.x[b] += push * self.particles.w[b];
                // Friction between the layers, as against an obstacle: of
                // how far they slid past each other this substep, as much
                // taken back as the push allows.
                let slid = (self.particles.x[b] - self.particles.was[b]) - (self.particles.x[a] - self.particles.was[a]);
                let along = slid - n * slid.dot(n);
                let length = along.length();
                if length > 1e-9 {
                    let hold = along * ((0.5 * depth / length).min(1.0) / sum);
                    self.particles.x[a] += hold * self.particles.w[a];
                    self.particles.x[b] -= hold * self.particles.w[b];
                }
            }
            self.particles.collide(radius, 0.5, &self.near);
            self.particles.finish(h, 0.4);
        }
    }

    /// Each particle's pull from the wind this substep: each triangle
    /// pushed along its facing by the air through it, and each particle
    /// pulled as the triangles round it are, on the mean.
    fn air(&mut self, level: Vec3, speed: f32, catch: f32) {
        let n = self.particles.len();
        self.pull.clear();
        self.pull.resize(n, Vec3::ZERO);
        let mut round = vec![0u8; n];
        if speed <= 0.0 && catch <= 0.0 {
            return;
        }
        let time = self.time;
        let x = &self.particles.x;
        let v = &self.particles.v;
        let side = level.cross(Vec3::Y);
        for tri in &self.triangles {
            let [a, b, c] = tri.map(|i| i as usize);
            let middle = (x[a] + x[b] + x[c]) / 3.0;
            let moving = (v[a] + v[b] + v[c]) / 3.0;
            // Gusts: slow waves rolling downwind through it, never quite
            // repeating; and turbulence across the wind and up, which sets
            // a flag flapping even when the wind runs along it.
            let along = middle.dot(level);
            let gust = 0.55
                + 0.3 * (time * 1.3 - along * 0.35).sin()
                + 0.25 * (time * 3.7 + middle.y * 0.8 - along * 0.9).sin().max(0.0);
            let swirl = side * (time * 5.1 + along * 2.3 + middle.y * 1.7).sin() * 0.3
                + Vec3::Y * (time * 4.3 - along * 1.9).sin() * 0.15;
            let air = (level * gust + swirl) * speed - moving;
            let facing = (x[b] - x[a]).cross(x[c] - x[a]);
            let area2 = facing.length();
            if area2 < 1e-9 {
                continue;
            }
            let normal = facing / area2;
            // Pressure on its face, and a little drag along it.
            let push = normal * (normal.dot(air) * catch * 3.0) + air * (0.08 * catch);
            for i in [a, b, c] {
                self.pull[i] += push;
                round[i] += 1;
            }
        }
        for (p, count) in self.pull.iter_mut().zip(round) {
            *p /= count.max(1) as f32;
        }
    }

    /// The pairs of particles within `reach` of each other that are not
    /// neighbours at rest, from a grid of the particles.
    fn find_close(&mut self, reach: f32) {
        self.close.clear();
        let cell = reach.max(1e-3);
        let key = |p: Vec3| {
            let c = (p / cell).floor();
            (c.x as i32, c.y as i32, c.z as i32)
        };
        let mut grid: scrap_core::hash::FastMap<(i32, i32, i32), Vec<u32>> = Default::default();
        for (k, p) in self.particles.x.iter().enumerate() {
            grid.entry(key(*p)).or_default().push(k as u32);
        }
        let x = &self.particles.x;
        let across = self.across as i64;
        for (k, p) in x.iter().enumerate() {
            let (cx, cy, cz) = key(*p);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(here) = grid.get(&(cx + dx, cy + dy, cz + dz)) else { continue };
                        for &other in here {
                            let o = other as usize;
                            if o <= k || x[o].distance(*p) >= reach {
                                continue;
                            }
                            // Neighbours in the grid are held by their links.
                            let (ik, jk) = (k as i64 % across, k as i64 / across);
                            let (io, jo) = (o as i64 % across, o as i64 / across);
                            if (ik - io).abs() <= 2 && (jk - jo).abs() <= 2 {
                                continue;
                            }
                            self.close.push((k as u32, other));
                        }
                    }
                }
            }
        }
    }

    /// Each particle's facing, from its neighbours across and down.
    fn normals(&self) -> Vec<Vec3> {
        let (w, h) = (self.across, self.down);
        let at = |i: usize, j: usize| self.particles.x[j * w + i];
        let mut out = Vec::with_capacity(w * h);
        for j in 0..h {
            for i in 0..w {
                let across = at((i + 1).min(w - 1), j) - at(i.saturating_sub(1), j);
                let down = at(i, (j + 1).min(h - 1)) - at(i, j.saturating_sub(1));
                out.push(down.cross(across).normalize_or(Vec3::Z));
            }
        }
        out
    }

    /// Its triangles, in the space of `placed` (what it is drawn at), both
    /// faces.
    pub fn mesh(&self, placed: Mat4) -> (Vec<Vertex>, Vec<u32>) {
        let back = placed.inverse();
        let normals = self.normals();
        let (w, h) = (self.across, self.down);
        let count = w * h;
        let mut vertices = Vec::with_capacity(count * 2);
        for side in [1.0f32, -1.0] {
            for k in 0..count {
                let p = back.transform_point3(self.particles.x[k]);
                let n = back.transform_vector3(normals[k]).normalize_or(Vec3::Z) * side;
                let (i, j) = (k % w, k / w);
                vertices.push(Vertex {
                    position: p.to_array(),
                    normal: n.to_array(),
                    uv: [i as f32 / (w - 1) as f32, j as f32 / (h - 1) as f32],
                });
            }
        }
        let mut indices = Vec::with_capacity(self.triangles.len() * 6);
        let o = count as u32;
        for &[a, b, c] in &self.triangles {
            // The front is wound the way its normals face; the back, its
            // normals turned over, the other way.
            indices.extend_from_slice(&[a, b, c]);
            indices.extend_from_slice(&[a + o, c + o, b + o]);
        }
        (vertices, indices)
    }
}

/// Every cloth flapped by this wind: the scene's, as it is spawned or
/// changes.
pub fn set_wind(world: &mut hecs::World, wind: Wind) {
    for state in world.query_mut::<&mut ClothState>() {
        state.wind = wind;
    }
}

/// Step every cloth by `seconds` in its wind, lying on `obstacles`.
pub fn run_cloth(world: &mut hecs::World, seconds: f32, obstacles: &Obstacles) {
    let late = (scrap_core::netsim::link_delay(world) / crate::net::NET_HZ as f64) as f32;
    let clock = scrap_core::netsim::session_time(world);
    let mut near = Vec::new();
    for (state, placed, replica) in world.query_mut::<(&mut ClothState, &WorldTransform, Option<&scrap_core::world::Replica>)>() {
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let points: Vec<Vec3> = if state.placed.is_some() {
            state.points().to_vec()
        } else {
            state.rest.iter().map(|r| placed.0.transform_point3(*r)).collect()
        };
        for p in &points {
            low = low.min(*p);
            high = high.max(*p);
        }
        // A cloth not yet hung may fall its own size before it lands.
        let fall = if state.placed.is_some() { 1.0 + seconds * 20.0 } else { state.cloth.size[0].max(state.cloth.size[1]) + 1.0 };
        obstacles.near(low - Vec3::splat(fall), high + Vec3::splat(fall), &mut near);
        let wind = state.wind;
        crate::net::keep_time(&mut state.time, clock);
        state.advance(placed.0, &wind, &near, seconds);
        // Rough: the owner's summary recorded, or pulled toward.
        if state.cloth.net == scrap_core::netsim::NetMode::Rough {
            let space = placed.0;
            if replica.is_some() {
                let rough = std::mem::take(&mut state.rough);
                let slack = crate::net::ROUGH_SLACK;
        rough.pull(&mut state.particles_mut().x, space, slack, late);
                state.rough = rough;
            } else {
                let points = state.particles_mut().x.clone();
                state.rough.record(&points, space, seconds);
            }
        }
    }
}

/// The soft module's dresser for cloth: a line's `cloth`, hung afresh
/// when it changes.
pub struct ClothDress;

impl scrap_core::world::Dress for ClothDress {
    fn parts(&self) -> &[&'static str] {
        &["cloth"]
    }

    fn dress(
        &mut self,
        line: &scrap_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: scrap_core::world::Changed,
        _: &mut Vec<scrap_core::world::Unresolved>,
    ) {
        match line.cloth() {
            Some(cloth) => {
                let _ = world.insert_one(entity, ClothState::new(cloth));
            }
            None => {
                let _ = world.remove_one::<ClothState>(entity);
            }
        }
    }
}


/// Its state for the network (`Components::register_state`): the
/// summary, from the owner, when it is `Rough`.
pub fn gather_net(world: &hecs::World, entity: hecs::Entity) -> Option<Vec<u8>> {
    let state = world.get::<&ClothState>(entity).ok()?;
    if state.cloth.net != scrap_core::netsim::NetMode::Rough || world.get::<&scrap_core::world::Replica>(entity).is_ok() {
        return None;
    }
    state.rough.bytes.clone()
}

/// The owner's summary onto everyone else's.
pub fn take_net(world: &mut hecs::World, entity: hecs::Entity, _sender: u32, _tick: u64, bytes: &[u8]) {
    if let Ok(mut state) = world.get::<&mut ClothState>(entity) {
        state.rough.take(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wind(strength: f32) -> Wind {
        // Blowing into the banner's face, along +z.
        Wind {
            direction: Vec3::Z,
            strength,
        }
    }

    fn settle(state: &mut ClothState, placed: Mat4, wind: Wind, obstacles: &[Obstacle], seconds: f32) {
        for _ in 0..(seconds * 30.0) as usize {
            state.advance(placed, &wind, obstacles, 1.0 / 30.0);
        }
    }

    #[test]
    fn a_banner_hangs_in_still_air_and_streams_out_downwind_in_a_gale() {
        let banner = Cloth {
            size: [1.0, 2.0],
            cells: [8, 12],
            pinned: Pinned::Top,
            ..Cloth::default()
        };
        let pole = Mat4::from_translation(Vec3::new(0.0, 5.0, 0.0));
        let hem = |state: &ClothState| state.points()[state.points().len() - state.across / 2];
        let mut still = ClothState::new(banner);
        settle(&mut still, pole, wind(0.0), &[], 4.0);
        let hung = hem(&still);
        // Straight down, its full length and no more.
        assert!(hung.z.abs() < 0.05, "{hung}");
        assert!((hung.y - 3.0).abs() < 0.05, "hangs its length: {hung}");

        let mut blown = ClothState::new(banner);
        settle(&mut blown, pole, wind(3.0), &[], 4.0);
        let streamed = hem(&blown);
        assert!(streamed.z > 0.6, "streams downwind: {streamed}");
        assert!(streamed.y > hung.y + 0.3, "lifted: {streamed}");
        // Its top still where it is pinned, and it has not stretched.
        assert!(blown.points()[0].distance(Vec3::new(-0.5, 5.0, 0.0)) < 1e-4);
        let (vertices, indices) = blown.mesh(pole);
        assert_eq!(vertices.len(), 8 * 12 * 2);
        assert_eq!(indices.len(), 7 * 11 * 12);
        assert!(vertices.iter().all(|v| v.position.iter().all(|c| c.is_finite())));
        // Each face is wound the way its normals face, or it is culled from
        // the side it should be seen from and lit from the other.
        for t in indices.chunks_exact(3) {
            let at = |i: u32| Vec3::from_array(vertices[i as usize].position);
            let wound = (at(t[1]) - at(t[0])).cross(at(t[2]) - at(t[0]));
            let normal = Vec3::from_array(vertices[t[0] as usize].normal);
            assert!(wound.dot(normal) >= 0.0, "a face wound against its normal");
        }
    }

    #[test]
    fn a_tablecloth_falls_onto_the_table_and_hangs_over_its_edges() {
        let cloth = Cloth {
            size: [2.0, 2.0],
            cells: [20, 20],
            pinned: Pinned::Free,
            thickness: 0.01,
            ..Cloth::default()
        };
        // A table a metre square, its top at 1 m; the cloth laid level
        // a little above it.
        let table = Obstacle::Box {
            center: Vec3::new(0.0, 0.5, 0.0),
            rotation: glam::Quat::IDENTITY,
            half: Vec3::splat(0.5),
        };
        let mut state = ClothState::new(cloth);
        settle(&mut state, Mat4::from_translation(Vec3::new(0.0, 1.2, 0.0)), wind(0.0), &[table.clone(), Obstacle::ground(0.0)], 3.0);
        let p = state.points();
        let (w, h) = state.grid();
        let middle = p[(h / 2) * w + w / 2];
        assert!((middle.y - 1.01).abs() < 0.02, "lies on the table: {middle}");
        // Its corners hang down past the table's edge, clear of the table.
        for corner in [p[0], p[w - 1], p[(h - 1) * w], p[h * w - 1]] {
            assert!(corner.y < 0.8, "hangs over the edge: {corner}");
            assert!(table.contact(corner, 0.0).is_none(), "not inside: {corner}");
        }
    }

    #[test]
    fn a_strip_dropped_in_a_heap_does_not_pass_through_itself() {
        // A long limp strip let fall on end folds up on the ground. Held
        // apart from itself, the folds stack; with nothing to hold them,
        // they lie through each other as one layer.
        let heap = |self_collide: bool| {
            let cloth = Cloth {
                size: [0.3, 3.0],
                cells: [4, 60],
                pinned: Pinned::Free,
                thickness: 0.01,
                stiffness: 0.0,
                self_collide,
                ..Cloth::default()
            };
            let mut state = ClothState::new(cloth);
            // On end, leaning a little, its foot just off the ground.
            let upright = Mat4::from_rotation_translation(glam::Quat::from_rotation_x(1.45), Vec3::new(0.0, 1.55, 0.0));
            settle(&mut state, upright, wind(0.0), &[Obstacle::ground(0.0)], 4.0);
            state.points().iter().map(|p| p.y).fold(0.0, f32::max)
        };
        let (stacked, through) = (heap(true), heap(false));
        assert!(stacked > 0.05, "the folds stack: {stacked}");
        assert!(through < stacked * 0.6, "without, they lie through each other: {through} vs {stacked}");
    }

    #[test]
    fn the_same_wind_flaps_it_the_same_way() {
        let flag = Cloth {
            pinned: Pinned::Left,
            ..Cloth::default()
        };
        let run = || {
            let mut s = ClothState::new(flag);
            settle(&mut s, Mat4::IDENTITY, wind(2.0), &[], 3.0);
            s.points().to_vec()
        };
        assert_eq!(run(), run());
    }
}
