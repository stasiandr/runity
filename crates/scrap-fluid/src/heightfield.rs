//! Water as a height over a grid (docs/simulation.md, items 6 and 10):
//! what a game's rivers, floods, ponds and puddles are when it does not
//! need water going over itself.
//!
//! * `shallow_water` — the shallow water equations on virtual pipes (Mei,
//!   Decaudin, Hu 2007): each cell's water flows through four pipes to its
//!   neighbours by the difference in their surfaces. It runs downhill, piles
//!   against walls, floods round what stands in it. Its bed is what is
//!   solid under each cell — the ground, a ramp, a rock — found again as
//!   things move, so a body pushed through it pushes the water: a wake.
//!   `dam` holds the upstream third higher at the start: let go, it floods.
//! * `ripples` — the wave equation on a height field: a pond's still
//!   surface rings where things go through it, the rings spreading,
//!   crossing and dying away; rain on it if `rain` says.
//!
//! Both are drawn as a live mesh of their surface in the entity's
//! material — `water: true` for the water's look — and both answer what
//! height the water is at a point ([`water_height`]), which is what floats
//! things.

use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use scrap_core::world::WorldTransform;
use scrap_geometry::mesh_asset::Vertex;
use scrap_soft::obstacle::{Obstacle, Obstacles};

/// Shallow water, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShallowWater {
    /// Metres across, along x and z, round the entity.
    pub size: Vec2,
    /// Cells along its longer side.
    pub cells: u32,
    /// How deep the water stands at the start, metres over its bed.
    pub depth: f32,
    /// How much higher the upstream third (−x) is held at the start: let
    /// go, a flood.
    pub dam: f32,
    /// How fast what moves in it loses speed: 0 slick, 1 sluggish.
    pub friction: f32,
    /// How it goes over the network (docs/netsim.md): `Local` unless
    /// the line says.
    #[serde(default, skip_serializing_if = "scrap_core::netsim::NetMode::is_local")]
    pub net: scrap_core::netsim::NetMode,
}

impl Default for ShallowWater {
    fn default() -> Self {
        Self { size: Vec2::splat(8.0), cells: 64, depth: 0.2, dam: 0.0, friction: 0.05, net: scrap_core::netsim::NetMode::Local }
    }
}

/// Ripples on still water, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ripples {
    pub size: Vec2,
    pub cells: u32,
    /// Metres a second the rings spread at.
    pub speed: f32,
    /// How fast they die, a share a second.
    pub damping: f32,
    /// Drops a second falling on it, anywhere: rain.
    pub rain: f32,
}

impl Default for Ripples {
    fn default() -> Self {
        Self { size: Vec2::splat(6.0), cells: 96, speed: 1.2, damping: 0.6, rain: 0.0 }
    }
}

/// A cover of snow, mud or sand that keeps what is pressed into it: a
/// deformable ground, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnowCover {
    pub size: Vec2,
    pub cells: u32,
    /// How deep it lies, metres.
    pub depth: f32,
    /// How much of what is pressed out of a track heaps beside it.
    pub berm: f32,
    /// Seconds for fresh snow to fill a track again; 0 never.
    pub refill: f32,
    /// How it goes over the network (docs/netsim.md): `Local` unless
    /// the line says.
    #[serde(default, skip_serializing_if = "scrap_core::netsim::NetMode::is_local")]
    pub net: scrap_core::netsim::NetMode,
}

impl Default for SnowCover {
    fn default() -> Self {
        Self { size: Vec2::splat(8.0), cells: 128, depth: 0.25, berm: 0.5, refill: 0.0, net: scrap_core::netsim::NetMode::Local }
    }
}

scrap_core::impl_parts! {
    ShallowWater => "shallow_water", fractions ["friction"];
    Ripples => "ripples";
    SnowCover => "snow_cover", fractions ["berm"];
}

/// The height-field water of a line, read off it.
pub trait HeightfieldLine {
    fn shallow_water(&self) -> Option<ShallowWater>;
    fn ripples(&self) -> Option<Ripples>;
    fn snow_cover(&self) -> Option<SnowCover>;
}

impl HeightfieldLine for scrap_core::EntityDesc {
    fn shallow_water(&self) -> Option<ShallowWater> {
        self.part()
    }
    fn ripples(&self) -> Option<Ripples> {
        self.part()
    }
    fn snow_cover(&self) -> Option<SnowCover> {
        self.part()
    }
}

impl HeightfieldLine for scrap_core::scene::Override {
    fn shallow_water(&self) -> Option<ShallowWater> {
        self.part()
    }
    fn ripples(&self) -> Option<Ripples> {
        self.part()
    }
    fn snow_cover(&self) -> Option<SnowCover> {
        self.part()
    }
}

pub const STEP: f32 = 1.0 / 60.0;
const G: f32 = 9.81;

/// A grid over the entity's ground: cells, their size, where it starts.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Grid {
    nx: usize,
    nz: usize,
    cell: f32,
    /// The low corner, in the world (the entity's own turn is left out: a
    /// water surface lies level).
    origin: Vec3,
}

impl Grid {
    fn new(size: Vec2, cells: u32, placed: Mat4) -> Self {
        let cells = cells.clamp(4, 512) as f32;
        let cell = size.x.max(size.y) / cells;
        let nx = (size.x / cell).round().max(2.0) as usize;
        let nz = (size.y / cell).round().max(2.0) as usize;
        let centre = placed.w_axis.truncate();
        Self { nx, nz, cell, origin: centre - Vec3::new(nx as f32 * cell * 0.5, 0.0, nz as f32 * cell * 0.5) }
    }

    fn at(&self, i: usize, k: usize) -> usize {
        k * self.nx + i
    }

    /// A cell's middle, at the grid's own height.
    fn middle(&self, i: usize, k: usize) -> Vec3 {
        self.origin + Vec3::new((i as f32 + 0.5) * self.cell, 0.0, (k as f32 + 0.5) * self.cell)
    }

    /// Which cell a point is over, and how far into it.
    fn locate(&self, p: Vec3) -> Option<(usize, usize, f32, f32)> {
        let x = (p.x - self.origin.x) / self.cell - 0.5;
        let z = (p.z - self.origin.z) / self.cell - 0.5;
        if x < 0.0 || z < 0.0 || x >= (self.nx - 1) as f32 || z >= (self.nz - 1) as f32 {
            return None;
        }
        Some((x as usize, z as usize, x.fract(), z.fract()))
    }

    /// A value of the grid at a point, bilinear.
    fn sample(&self, values: &[f32], p: Vec3) -> Option<f32> {
        let (i, k, fx, fz) = self.locate(p)?;
        let v = |a: usize, b: usize| values[self.at(a, b)];
        let low = v(i, k) + (v(i + 1, k) - v(i, k)) * fx;
        let high = v(i, k + 1) + (v(i + 1, k + 1) - v(i, k + 1)) * fx;
        Some(low + (high - low) * fz)
    }

    /// A mesh of heights over the grid, in the space of `placed`.
    fn mesh(&self, placed: Mat4, height: impl Fn(usize, usize) -> f32) -> (Vec<Vertex>, Vec<u32>) {
        let back = placed.inverse();
        let turn = back.transform_vector3(Vec3::Y).length().max(1e-6);
        let (nx, nz) = (self.nx, self.nz);
        let heights: Vec<f32> = (0..nz).flat_map(|k| (0..nx).map(move |i| (i, k))).map(|(i, k)| height(i, k)).collect();
        let mut vertices = Vec::with_capacity(nx * nz);
        for k in 0..nz {
            for i in 0..nx {
                let h = |a: usize, b: usize| heights[b.min(nz - 1) * nx + a.min(nx - 1)];
                let dx = h(i + 1, k) - h(i.saturating_sub(1), k);
                let dz = h(i, k + 1) - h(i, k.saturating_sub(1));
                let normal = Vec3::new(-dx, 2.0 * self.cell, -dz).normalize();
                let p = self.middle(i, k) + Vec3::Y * h(i, k);
                vertices.push(Vertex {
                    position: back.transform_point3(p).to_array(),
                    normal: (back.transform_vector3(normal) / turn).normalize_or(Vec3::Y).to_array(),
                    uv: [p.x, p.z],
                });
            }
        }
        let mut indices = Vec::with_capacity((nx - 1) * (nz - 1) * 6);
        for k in 0..nz as u32 - 1 {
            for i in 0..nx as u32 - 1 {
                let a = k * nx as u32 + i;
                let (b, c, d) = (a + 1, a + nx as u32, a + nx as u32 + 1);
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        (vertices, indices)
    }
}

/// The highest solid under a point, from `below` to `above`: the bed.
fn bed_under(p: Vec3, below: f32, above: f32, obstacles: &[Obstacle]) -> f32 {
    let solid = |y: f32| obstacles.iter().any(|o| o.contact(Vec3::new(p.x, y, p.z), 0.0).is_some());
    if !solid(below) {
        return below;
    }
    if solid(above) {
        return above;
    }
    let (mut lo, mut hi) = (below, above);
    for _ in 0..14 {
        let mid = (lo + hi) * 0.5;
        if solid(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Shallow water as it flows: the component [`run_heightfields`] steps.
#[derive(Debug, Clone)]
pub struct ShallowState {
    pub water: ShallowWater,
    grid: Option<Grid>,
    bed: Vec<f32>,
    /// Water over the bed, metres.
    pub depth: Vec<f32>,
    /// What flows out through each cell's four pipes: +x, −x, +z, −z.
    flux: Vec<[f32; 4]>,
    bed_from: Option<Vec<Obstacle>>,
    owed: f32,
}

impl ShallowState {
    pub fn new(water: ShallowWater) -> Self {
        Self { water, grid: None, bed: Vec::new(), depth: Vec::new(), flux: Vec::new(), bed_from: None, owed: 0.0 }
    }

    /// All the water in it, cubic metres.
    pub fn volume(&self) -> f32 {
        let cell = self.grid.map_or(0.0, |g| g.cell);
        self.depth.iter().sum::<f32>() * cell * cell
    }

    /// The water's surface at a point, when it is over the grid and wet.
    pub fn height_at(&self, p: Vec3) -> Option<f32> {
        let grid = self.grid?;
        let depth = grid.sample(&self.depth, p)?;
        (depth > 0.005).then(|| grid.sample(&self.bed, p).unwrap_or(0.0) + depth + grid.origin.y)
    }

    fn start(&mut self, placed: Mat4, obstacles: &[Obstacle]) {
        let grid = Grid::new(self.water.size, self.water.cells, placed);
        let n = grid.nx * grid.nz;
        self.grid = Some(grid);
        self.bed = vec![0.0; n];
        self.find_bed(obstacles);
        self.depth = vec![self.water.depth.max(0.0); n];
        // Held behind a dam: the upstream third higher, to the same level.
        for k in 0..grid.nz {
            for i in 0..grid.nx {
                let at = grid.at(i, k);
                let lift = if i < grid.nx / 3 { self.water.dam.max(0.0) } else { 0.0 };
                self.depth[at] = (self.water.depth + lift - self.bed[at]).max(0.0);
            }
        }
        self.flux = vec![[0.0; 4]; n];
    }

    /// The bed under each cell, when what is solid about it moved.
    fn find_bed(&mut self, obstacles: &[Obstacle]) {
        if self.bed_from.as_deref() == Some(obstacles) {
            return;
        }
        let Some(grid) = self.grid else { return };
        let old = self.bed.clone();
        let top = self.water.depth + self.water.dam + 2.0;
        for k in 0..grid.nz {
            for i in 0..grid.nx {
                let at = grid.at(i, k);
                let bed = bed_under(grid.middle(i, k), grid.origin.y - 1.0, grid.origin.y + top, obstacles) - grid.origin.y;
                self.bed[at] = bed.max(0.0);
            }
        }
        // What rose under the water pushes it up: its volume kept.
        if old.len() == self.bed.len() && self.depth.len() == self.bed.len() {
            for (at, was) in old.iter().enumerate() {
                let rose = self.bed[at] - was;
                if rose > 0.0 && self.depth[at] > 0.0 {
                    // Where the bed came up through the water, the water it
                    // took the place of spills over the neighbours.
                    let spilled = rose.min(self.depth[at]);
                    self.depth[at] -= spilled;
                    let (i, k) = (at % grid.nx, at / grid.nx);
                    let near: Vec<usize> = [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .filter_map(|(di, dk)| {
                            let (a, b) = (i as i64 + di, k as i64 + dk);
                            (a >= 0 && b >= 0 && (a as usize) < grid.nx && (b as usize) < grid.nz).then(|| grid.at(a as usize, b as usize))
                        })
                        .collect();
                    for n in &near {
                        self.depth[*n] += spilled / near.len() as f32;
                    }
                }
            }
        }
        self.bed_from = Some(obstacles.to_vec());
    }

    /// Along by `seconds`, over `obstacles`.
    pub fn advance(&mut self, placed: Mat4, obstacles: &[Obstacle], seconds: f32) {
        if self.grid.is_none() {
            self.start(placed, obstacles);
        }
        self.find_bed(obstacles);
        self.owed = (self.owed + seconds.max(0.0)).min(0.1);
        while self.owed >= STEP {
            self.owed -= STEP;
            // As many substeps as a wave needs to cross a cell no faster
            // than a third of it.
            let deepest = self.depth.iter().cloned().fold(0.0, f32::max);
            let cell = self.grid.map_or(1.0, |g| g.cell);
            let n = ((STEP * (G * deepest.max(0.01)).sqrt() / (cell * 0.3)).ceil() as usize).clamp(1, 64);
            for _ in 0..n {
                self.substep(STEP / n as f32);
            }
        }
    }

    fn substep(&mut self, dt: f32) {
        let Some(grid) = self.grid else { return };
        let (nx, nz, l) = (grid.nx, grid.nz, grid.cell);
        let surface = |s: &Self, at: usize| s.bed[at] + s.depth[at];
        let keep = 1.0 - self.water.friction.clamp(0.0, 1.0) * dt * 4.0;
        for k in 0..nz {
            for i in 0..nx {
                let at = grid.at(i, k);
                let here = surface(self, at);
                let ways = [(i + 1 < nx).then(|| at + 1), (i > 0).then(|| at - 1), (k + 1 < nz).then(|| at + nx), (k > 0).then(|| at - nx)];
                let mut out = [0.0f32; 4];
                for (w, next) in ways.iter().enumerate() {
                    if let Some(next) = next {
                        let fall = here - surface(self, *next);
                        // A pipe as wide as the cell and as deep as the
                        // water going through it: df/dt = g · Δh · A / l.
                        let deep = self.depth[at].max(self.depth[*next]);
                        out[w] = (self.flux[at][w] * keep + dt * G * fall * deep).max(0.0);
                    }
                }
                // No more out than there is.
                let total: f32 = out.iter().sum();
                if total > 0.0 {
                    let most = self.depth[at] * l * l / (total * dt);
                    if most < 1.0 {
                        for o in &mut out {
                            *o *= most;
                        }
                    }
                }
                self.flux[at] = out;
            }
        }
        let flux = &self.flux;
        for k in 0..nz {
            for i in 0..nx {
                let at = grid.at(i, k);
                let mut inflow = 0.0;
                if i > 0 {
                    inflow += flux[at - 1][0];
                }
                if i + 1 < nx {
                    inflow += flux[at + 1][1];
                }
                if k > 0 {
                    inflow += flux[at - nx][2];
                }
                if k + 1 < nz {
                    inflow += flux[at + nx][3];
                }
                let outflow: f32 = flux[at].iter().sum();
                self.depth[at] = (self.depth[at] + dt * (inflow - outflow) / (l * l)).max(0.0);
            }
        }
    }

    /// Its surface, in the space of `placed`: where it is dry, just under
    /// the bed, out of sight.
    pub fn mesh(&self, placed: Mat4) -> (Vec<Vertex>, Vec<u32>) {
        let Some(grid) = self.grid else { return (Vec::new(), Vec::new()) };
        grid.mesh(placed, |i, k| {
            let at = grid.at(i, k);
            if self.depth[at] > 0.003 { self.bed[at] + self.depth[at] } else { self.bed[at] - 0.03 }
        })
    }
}

/// Ripples as they spread: the component [`run_heightfields`] steps.
#[derive(Debug, Clone)]
pub struct RipplesState {
    pub ripples: Ripples,
    grid: Option<Grid>,
    pub height: Vec<f32>,
    was: Vec<f32>,
    /// What was in the water last step, to ring where it moved.
    wet: Vec<bool>,
    dice: u64,
    owed: f32,
}

impl RipplesState {
    pub fn new(ripples: Ripples) -> Self {
        Self { ripples, grid: None, height: Vec::new(), was: Vec::new(), wet: Vec::new(), dice: 0x9e37_79b9, owed: 0.0 }
    }

    /// The surface at a point.
    pub fn height_at(&self, p: Vec3) -> Option<f32> {
        let grid = self.grid?;
        Some(grid.origin.y + grid.sample(&self.height, p)?)
    }

    /// A drop at a point, `depth` metres down.
    pub fn drop_at(&mut self, p: Vec3, depth: f32) {
        let Some(grid) = self.grid else { return };
        if let Some((i, k, _, _)) = grid.locate(p) {
            for dk in 0..2 {
                for di in 0..2 {
                    let at = grid.at(i + di, k + dk);
                    self.height[at] -= depth;
                }
            }
        }
    }

    /// Along by `seconds`; what cuts its surface in `obstacles` rings it.
    pub fn advance(&mut self, placed: Mat4, obstacles: &[Obstacle], seconds: f32) {
        let grid = *self.grid.get_or_insert_with(|| Grid::new(self.ripples.size, self.ripples.cells, placed));
        let n = grid.nx * grid.nz;
        if self.height.len() != n {
            self.height = vec![0.0; n];
            self.was = vec![0.0; n];
            self.wet = vec![false; n];
        }
        self.owed = (self.owed + seconds.max(0.0)).min(0.1);
        while self.owed >= STEP {
            self.owed -= STEP;
            // Where something is in the water now and was not, or left it:
            // the surface is pushed, and rings.
            for k in 0..grid.nz {
                for i in 0..grid.nx {
                    let at = grid.at(i, k);
                    let p = grid.middle(i, k);
                    let wet = obstacles.iter().any(|o| o.contact(p, 0.0).is_some());
                    if wet != self.wet[at] {
                        self.height[at] -= 0.02;
                    }
                    if wet {
                        // Water does not stand where something is.
                        self.height[at] = 0.0;
                        self.was[at] = 0.0;
                    }
                    self.wet[at] = wet;
                }
            }
            // Rain: drops here and there.
            let drops = self.ripples.rain.max(0.0) * STEP;
            let mut whole = drops.floor() as usize;
            let mut next = || {
                self.dice ^= self.dice << 13;
                self.dice ^= self.dice >> 7;
                self.dice ^= self.dice << 17;
                (self.dice >> 40) as f32 / (1u64 << 24) as f32
            };
            if next() < drops.fract() {
                whole += 1;
            }
            for _ in 0..whole {
                let (x, z) = (next(), next());
                let at = grid.at((x * (grid.nx - 1) as f32) as usize, (z * (grid.nz - 1) as f32) as usize);
                self.height[at] -= 0.01;
            }
            // The wave equation, as many substeps as it needs to stay put.
            let c = self.ripples.speed.max(0.01);
            let sub = ((STEP * c / (grid.cell * 0.5)).ceil() as usize).clamp(1, 16);
            let dt = STEP / sub as f32;
            let r = (c * dt / grid.cell).powi(2);
            let keep = 1.0 - self.ripples.damping.clamp(0.0, 10.0) * dt;
            for _ in 0..sub {
                let mut next = vec![0.0; n];
                for k in 1..grid.nz - 1 {
                    for i in 1..grid.nx - 1 {
                        let at = grid.at(i, k);
                        let h = self.height[at];
                        let around = self.height[at - 1] + self.height[at + 1] + self.height[at - grid.nx] + self.height[at + grid.nx] - 4.0 * h;
                        next[at] = h + (h - self.was[at]) * keep + around * r;
                    }
                }
                self.was = std::mem::replace(&mut self.height, next);
            }
        }
    }

    pub fn mesh(&self, placed: Mat4) -> (Vec<Vertex>, Vec<u32>) {
        let Some(grid) = self.grid else { return (Vec::new(), Vec::new()) };
        grid.mesh(placed, |i, k| self.height[grid.at(i, k)])
    }
}

/// Snow as it is trodden: the component [`run_heightfields`] steps.
#[derive(Debug, Clone)]
pub struct SnowState {
    pub cover: SnowCover,
    grid: Option<Grid>,
    /// Snow over the ground, metres.
    pub depth: Vec<f32>,
    /// Pressed into, ever: the berms heap beside tracks, not in them.
    trodden: Vec<bool>,
    owed: f32,
}

impl SnowState {
    pub fn new(cover: SnowCover) -> Self {
        Self { cover, grid: None, depth: Vec::new(), trodden: Vec::new(), owed: 0.0 }
    }

    /// How deep the snow is at a point.
    pub fn depth_at(&self, p: Vec3) -> Option<f32> {
        self.grid?.sample(&self.depth, p)
    }

    /// Along by `seconds`: whatever is in the snow presses it down to its
    /// underside, and what it pressed out heaps round the track.
    pub fn advance(&mut self, placed: Mat4, obstacles: &[Obstacle], seconds: f32) {
        let grid = *self.grid.get_or_insert_with(|| Grid::new(self.cover.size, self.cover.cells, placed));
        let n = grid.nx * grid.nz;
        if self.depth.len() != n {
            self.depth = vec![self.cover.depth.max(0.0); n];
            self.trodden = vec![false; n];
        }
        self.owed = (self.owed + seconds.max(0.0)).min(0.1);
        while self.owed >= STEP {
            self.owed -= STEP;
            let mut pressed = vec![false; n];
            let mut moved = 0.0f32;
            for k in 0..grid.nz {
                for i in 0..grid.nx {
                    let at = grid.at(i, k);
                    let base = grid.middle(i, k);
                    let top = self.depth[at];
                    if top <= 0.0 {
                        continue;
                    }
                    let solid = |y: f32| obstacles.iter().any(|o| o.contact(base + Vec3::Y * y, 0.0).is_some());
                    // The lowest of what is in the snow here: sampled up the
                    // column, then found exactly.
                    let samples = 6;
                    let Some(first) = (0..=samples).map(|s| top * s as f32 / samples as f32).find(|y| solid(*y)) else { continue };
                    let (mut lo, mut hi) = ((first - top / samples as f32).max(0.0), first);
                    if solid(lo) {
                        hi = lo;
                    }
                    for _ in 0..10 {
                        let mid = (lo + hi) * 0.5;
                        if solid(mid) {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                    }
                    let under = hi.max(0.0);
                    if under < top {
                        moved += (top - under) * self.cover.berm.clamp(0.0, 1.0);
                        self.depth[at] = under;
                        pressed[at] = true;
                        self.trodden[at] = true;
                    }
                }
            }
            // What was pressed out heaps on the snow round the tracks.
            if moved > 0.0 {
                let mut rim = Vec::new();
                for k in 0..grid.nz {
                    for i in 0..grid.nx {
                        let at = grid.at(i, k);
                        if self.trodden[at] {
                            continue;
                        }
                        let beside = [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)].iter().any(|(di, dk)| {
                            let (a, b) = (i as i64 + di, k as i64 + dk);
                            a >= 0 && b >= 0 && (a as usize) < grid.nx && (b as usize) < grid.nz && pressed[grid.at(a as usize, b as usize)]
                        });
                        if beside {
                            rim.push(at);
                        }
                    }
                }
                for at in &rim {
                    self.depth[*at] += moved / rim.len().max(1) as f32;
                }
            }
            // Heaped snow slumps to its angle of repose (40°): what a
            // plough pushes ahead of it spills to the sides.
            let most = grid.cell * 0.84;
            for _ in 0..2 {
                for k in 0..grid.nz {
                    for i in 0..grid.nx {
                        let at = grid.at(i, k);
                        for (di, dk) in [(1usize, 0usize), (0, 1)] {
                            let (a, b) = (i + di, k + dk);
                            if a >= grid.nx || b >= grid.nz {
                                continue;
                            }
                            let there = grid.at(a, b);
                            let step = self.depth[at] - self.depth[there];
                            if step.abs() > most {
                                let shift = (step.abs() - most) * 0.5 * step.signum();
                                self.depth[at] -= shift;
                                self.depth[there] += shift;
                            }
                        }
                    }
                }
            }
            // Fresh snow fills the tracks again.
            if self.cover.refill > 0.0 {
                let rate = self.cover.depth / self.cover.refill * STEP;
                for d in &mut self.depth {
                    if *d < self.cover.depth {
                        *d = (*d + rate).min(self.cover.depth);
                    }
                }
            }
        }
    }

    pub fn mesh(&self, placed: Mat4) -> (Vec<Vertex>, Vec<u32>) {
        let Some(grid) = self.grid else { return (Vec::new(), Vec::new()) };
        grid.mesh(placed, |i, k| self.depth[grid.at(i, k)])
    }
}

/// The water's surface at a point, from whatever water there is there:
/// shallow water, ripples, an ocean. What floats things.
pub fn water_height(world: &hecs::World, p: Vec3) -> Option<f32> {
    let shallow = world.query::<&ShallowState>().iter().find_map(|s| s.height_at(p));
    let ripples = || world.query::<&RipplesState>().iter().find_map(|s| s.height_at(p));
    let ocean = || world.query::<&crate::ocean::OceanState>().iter().find_map(|s| s.height_at(p));
    shallow.or_else(ripples).or_else(ocean)
}

/// Step every height-field water by `seconds` over `obstacles`.
pub fn run_heightfields(world: &mut hecs::World, seconds: f32, obstacles: &Obstacles) {
    let mut near = Vec::new();
    for (state, placed) in world.query_mut::<(&mut ShallowState, &WorldTransform)>() {
        let c = placed.0.w_axis.truncate();
        let half = state.water.size * 0.5;
        obstacles.near(c - Vec3::new(half.x, 1.0, half.y), c + Vec3::new(half.x, 4.0, half.y), &mut near);
        state.advance(placed.0, &near, seconds);
    }
    for (state, placed) in world.query_mut::<(&mut SnowState, &WorldTransform)>() {
        let c = placed.0.w_axis.truncate();
        let half = state.cover.size * 0.5;
        obstacles.near(c - Vec3::new(half.x, 0.1, half.y), c + Vec3::new(half.x, state.cover.depth + 0.5, half.y), &mut near);
        // What it lies on is not in it: the ground, a floor.
        near.retain(|o| o.bounds().is_some_and(|(_, top)| top.y > c.y + 0.01));
        state.advance(placed.0, &near, seconds);
    }
    for (state, placed) in world.query_mut::<(&mut RipplesState, &WorldTransform)>() {
        let c = placed.0.w_axis.truncate();
        let half = state.ripples.size * 0.5;
        obstacles.near(c - Vec3::new(half.x, 0.2, half.y), c + Vec3::new(half.x, 0.2, half.y), &mut near);
        // Only what reaches its surface: not the bottom of the pond.
        near.retain(|o| !matches!(o, Obstacle::Plane { .. }));
        state.advance(placed.0, &near, seconds);
    }
}

/// The fluid module's dresser for height-field water.
pub struct HeightfieldDress;

impl scrap_core::world::Dress for HeightfieldDress {
    fn parts(&self) -> &[&'static str] {
        &["shallow_water", "ripples", "snow_cover"]
    }

    fn dress(
        &mut self,
        line: &scrap_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: scrap_core::world::Changed,
        _: &mut Vec<scrap_core::world::Unresolved>,
    ) {
        match line.shallow_water() {
            Some(w) => {
                let _ = world.insert_one(entity, ShallowState::new(w));
            }
            None => {
                let _ = world.remove_one::<ShallowState>(entity);
            }
        }
        match line.ripples() {
            Some(r) => {
                let _ = world.insert_one(entity, RipplesState::new(r));
            }
            None => {
                let _ = world.remove_one::<RipplesState>(entity);
            }
        }
        match line.snow_cover() {
            Some(c) => {
                let _ = world.insert_one(entity, SnowState::new(c));
            }
            None => {
                let _ = world.remove_one::<SnowState>(entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dam_breaks_the_flood_runs_downstream_and_no_water_is_lost() {
        let water = ShallowWater { size: Vec2::new(6.0, 1.0), cells: 60, depth: 0.1, dam: 0.4, friction: 0.05, ..Default::default() };
        let mut state = ShallowState::new(water);
        let floor = [Obstacle::ground(0.0)];
        state.advance(Mat4::IDENTITY, &floor, 0.0);
        let at_start = state.volume();
        let downstream = |s: &ShallowState| s.height_at(Vec3::new(1.0, 0.0, 0.0)).unwrap_or(0.0);
        let before = downstream(&state);
        for _ in 0..90 {
            state.advance(Mat4::IDENTITY, &floor, 1.0 / 60.0);
        }
        assert!(downstream(&state) > before + 0.03, "the flood arrives: {} from {}", downstream(&state), before);
        assert!((state.volume() - at_start).abs() < at_start * 0.01, "{} vs {}", state.volume(), at_start);
        // It settles level.
        for _ in 0..1200 {
            state.advance(Mat4::IDENTITY, &floor, 1.0 / 60.0);
        }
        let (a, b) = (state.height_at(Vec3::new(-2.5, 0.0, 0.0)).unwrap(), state.height_at(Vec3::new(2.5, 0.0, 0.0)).unwrap());
        assert!((a - b).abs() < 0.02, "level: {a} {b}");
    }

    #[test]
    fn water_goes_round_a_rock_and_a_body_pushed_in_raises_it() {
        let water = ShallowWater { size: Vec2::new(4.0, 4.0), cells: 40, depth: 0.3, dam: 0.0, friction: 0.1, ..Default::default() };
        let mut state = ShallowState::new(water);
        let rock = Obstacle::Box { center: Vec3::new(0.0, 0.5, 0.0), rotation: glam::Quat::IDENTITY, half: Vec3::splat(0.5) };
        state.advance(Mat4::IDENTITY, &[Obstacle::ground(0.0), rock.clone()], 1.0 / 60.0);
        // No water inside the rock.
        assert!(state.height_at(Vec3::new(0.0, 0.0, 0.0)).is_none());
        let level = state.height_at(Vec3::new(1.5, 0.0, 1.5)).unwrap();
        assert!((level - 0.3).abs() < 0.01, "{level}");
        // A second rock pushed down into it: the water rises round about.
        let sunk = Obstacle::Box { center: Vec3::new(-1.2, 0.4, 0.0), rotation: glam::Quat::IDENTITY, half: Vec3::new(0.4, 0.4, 0.8) };
        for _ in 0..240 {
            state.advance(Mat4::IDENTITY, &[Obstacle::ground(0.0), rock.clone(), sunk.clone()], 1.0 / 60.0);
        }
        let risen = state.height_at(Vec3::new(1.5, 0.0, 1.5)).unwrap();
        assert!(risen > level + 0.005, "raised: {risen} from {level}");
    }

    #[test]
    fn a_ball_rolled_through_snow_leaves_a_track_with_berms_that_stays() {
        let mut state = SnowState::new(SnowCover { size: Vec2::new(4.0, 2.0), cells: 80, depth: 0.2, berm: 0.5, refill: 0.0, ..Default::default() });
        // A ball of 0.3 m sunk to 0.1 over the ground, rolled along x.
        for step in 0..60 {
            let x = -1.5 + step as f32 * 0.05;
            let ball = Obstacle::Sphere { center: Vec3::new(x, 0.25, 0.0), radius: 0.15 };
            state.advance(Mat4::IDENTITY, &[ball], 1.0 / 60.0);
        }
        // Gone: the track is still there, as deep as the ball went.
        state.advance(Mat4::IDENTITY, &[], 1.0);
        let track = state.depth_at(Vec3::new(0.0, 0.0, 0.0)).unwrap();
        assert!((track - 0.1).abs() < 0.02, "track depth {track}");
        let beside = state.depth_at(Vec3::new(0.0, 0.0, 0.2)).unwrap();
        assert!(beside > 0.2, "a berm beside it: {beside}");
        let untouched = state.depth_at(Vec3::new(0.0, 0.0, 0.8)).unwrap();
        assert!((untouched - 0.2).abs() < 1e-4, "{untouched}");
        // Before it was rolled over: untouched.
        assert!((state.depth_at(Vec3::new(1.8, 0.0, 0.0)).unwrap() - 0.2).abs() < 1e-4);
    }

    #[test]
    fn a_stick_through_the_pond_sends_rings_out_that_die_away() {
        let mut state = RipplesState::new(Ripples { size: Vec2::splat(4.0), cells: 80, speed: 1.0, damping: 0.5, rain: 0.0 });
        let stick = Obstacle::Sphere { center: Vec3::new(0.0, 0.0, 0.0), radius: 0.1 };
        state.advance(Mat4::IDENTITY, &[stick], 1.0 / 60.0);
        for _ in 0..40 {
            state.advance(Mat4::IDENTITY, &[], 1.0 / 60.0);
        }
        // A ring out at about a speed × time from the stick.
        let at = |r: f32| state.height_at(Vec3::new(r, 0.0, 0.0)).unwrap().abs();
        let ring = (0..20).map(|i| at(0.2 + i as f32 * 0.05)).fold(0.0, f32::max);
        assert!(ring > 1e-4, "a ring: {ring}");
        let far = at(1.8);
        assert!(far < ring * 0.2, "not yet far: {far} vs {ring}");
        for _ in 0..600 {
            state.advance(Mat4::IDENTITY, &[], 1.0 / 60.0);
        }
        let calm = state.height.iter().map(|h| h.abs()).fold(0.0, f32::max);
        assert!(calm < ring * 0.1, "and dies away: {calm}");
    }
}
