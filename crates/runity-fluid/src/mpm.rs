//! The material point method: a line's `mpm`. `mpm: (material: Snow,
//! size: (1.0, 1.0, 1.0))` fills a box round the entity with particles of
//! snow, sand, water or jelly that fall into the room round it (`domain`)
//! and over the colliders in it — a snowball smashing, a heap of sand
//! slumping, water splashing, a jelly wobbling (docs/simulation.md, items
//! 6 and 8).
//!
//! MLS-MPM (Hu et al. 2018): each step the particles give their mass and
//! momentum to a grid (quadratic B-splines), the grid moves under gravity
//! and stops at what is solid, and the particles take their speed back.
//! Each particle carries its deformation, and its material decides the
//! stress of it:
//!
//! * `Water`: pressure from how much it is squeezed, nothing else.
//! * `Jelly`: fixed-corotated elasticity — it springs back.
//! * `Snow`: the same, but past a little squeeze or stretch the snow
//!   gives for good and hardens as it packs (Stomakhin et al. 2013).
//! * `Sand`: no pull at all and friction against shearing — Drucker–Prager
//!   plasticity in log strain (Klár et al. 2016): it pours and heaps at its
//!   angle of repose.
//!
//! The speeds come back to the particles by `transfer`: `Apic` keeps each
//! particle's own swirl (affine, as MLS-MPM does); `Flip` blends the
//! grid's change into the particle's own speed (FLIP/PIC), livelier and
//! noisier.

use glam::{Mat3, Mat4, Vec3};
use serde::{Deserialize, Serialize};

use runity_core::world::WorldTransform;
use runity_soft::obstacle::{Obstacle, Obstacles};

/// A block of material, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mpm {
    pub material: MpmMaterial,
    /// The box it starts as, metres, round the entity.
    pub size: Vec3,
    /// The room it may move in, metres, round the entity's feet: what
    /// leaves it stops at its walls.
    pub domain: Vec3,
    /// Grid cells a metre.
    pub resolution: f32,
    pub transfer: Transfer,
    /// How hard it is, Young's modulus in pascals divided by 1000.
    pub stiffness: f32,
    /// How high above the room's floor the block starts, metres: the
    /// entity is the floor's middle.
    pub height: f32,
    /// How it goes over the network (docs/netsim.md): `Local` unless
    /// the line says.
    #[serde(default, skip_serializing_if = "runity_core::netsim::NetMode::is_local")]
    pub net: runity_core::netsim::NetMode,
    /// The most substeps a step may take. A material stiffer than that
    /// many keep stable is stepped as the stiffest they do: a little
    /// softer, and in a frame's budget. A real-time step has a few
    /// milliseconds; each substep is a pass over every particle and node.
    pub max_substeps: u32,
}

/// What an MPM block is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MpmMaterial {
    Water,
    Jelly,
    #[default]
    Snow,
    Sand,
}

/// How speeds come back from the grid.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum Transfer {
    #[default]
    Apic,
    /// FLIP with this much of it (0 is PIC, 0.95 lively).
    Flip(f32),
}

impl Default for Mpm {
    fn default() -> Self {
        Self {
            material: MpmMaterial::Snow,
            size: Vec3::splat(0.6),
            domain: Vec3::new(3.0, 2.5, 3.0),
            resolution: 16.0,
            transfer: Transfer::Apic,
            stiffness: 140.0,
            height: 0.5,
            net: runity_core::netsim::NetMode::Local,
            max_substeps: 8,
        }
    }
}

runity_core::impl_parts! {
    Mpm => "mpm";
}

/// The MPM block of a line, read off it.
pub trait MpmLine {
    fn mpm(&self) -> Option<Mpm>;
}

impl MpmLine for runity_core::EntityDesc {
    fn mpm(&self) -> Option<Mpm> {
        self.part()
    }
}

impl MpmLine for runity_core::scene::Override {
    fn mpm(&self) -> Option<Mpm> {
        self.part()
    }
}

pub const STEP: f32 = 1.0 / 60.0;
/// The most particles a block has.
pub const MOST: usize = 60_000;

/// One particle of material.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grain {
    pub x: Vec3,
    pub v: Vec3,
    /// Its affine speed (APIC).
    c: Mat3,
    /// Its deformation.
    f: Mat3,
    /// How much snow has packed, by volume.
    jp: f32,
}

/// An MPM block as it moves: the component [`run_mpm`] steps.
#[derive(Debug, Clone)]
pub struct MpmState {
    pub mpm: Mpm,
    pub grains: Vec<Grain>,
    /// The room's low corner and its cells.
    origin: Vec3,
    dx: f32,
    n: [usize; 3],
    /// Each node's momentum then speed, its speed before the step (FLIP),
    /// and its mass.
    grid_v: Vec<Vec3>,
    grid_was: Vec<Vec3>,
    grid_m: Vec<f32>,
    /// Nodes inside something solid, and the way out of it.
    solid: Vec<Option<Vec3>>,
    solid_from: Option<Vec<Obstacle>>,
    placed: bool,
    owed: f32,
    /// Settled: not stepped until something could move it.
    rest: runity_soft::rest::Rest,
    /// Where the grains are, for [`runity_soft::rest::Rest::asleep`].
    at: Vec<Vec3>,
}

impl MpmState {
    pub fn new(mpm: Mpm) -> Self {
        Self {
            mpm,
            grains: Vec::new(),
            origin: Vec3::ZERO,
            dx: 1.0 / mpm.resolution.max(2.0),
            n: [0; 3],
            grid_v: Vec::new(),
            grid_was: Vec::new(),
            grid_m: Vec::new(),
            solid: Vec::new(),
            solid_from: None,
            placed: false,
            owed: 0.0,
            rest: Default::default(),
            at: Vec::new(),
        }
    }

    /// Where its particles are, in the world.
    pub fn points(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.grains.iter().map(|g| g.x)
    }

    fn fill(&mut self, placed: Mat4) {
        let dx = self.dx;
        let feet = placed.w_axis.truncate();
        let domain = self.mpm.domain.max(Vec3::splat(4.0 * dx));
        self.origin = feet - Vec3::new(domain.x * 0.5, 0.0, domain.z * 0.5);
        let cells = (domain / dx).ceil();
        self.n = [cells.x as usize + 3, cells.y as usize + 3, cells.z as usize + 3];
        let nodes = self.n[0] * self.n[1] * self.n[2];
        self.grid_v = vec![Vec3::ZERO; nodes];
        self.grid_was = vec![Vec3::ZERO; nodes];
        self.grid_m = vec![0.0; nodes];
        // Two particles a cell along each way, jittered.
        let size = self.mpm.size.min(domain);
        let spacing = dx * 0.5;
        let count = (size / spacing).floor().max(Vec3::ONE);
        let centre = feet + Vec3::new(0.0, self.mpm.height.max(0.0) + size.y * 0.5, 0.0);
        let mut k = 0x2545_f491u32;
        let mut jitter = || {
            k ^= k << 13;
            k ^= k >> 17;
            k ^= k << 5;
            (k as f32 / u32::MAX as f32 - 0.5) * spacing * 0.8
        };
        self.grains.clear();
        for z in 0..count.z as usize {
            for y in 0..count.y as usize {
                for x in 0..count.x as usize {
                    if self.grains.len() >= MOST {
                        break;
                    }
                    let p = centre - size * 0.5 + (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * spacing;
                    self.grains.push(Grain {
                        x: p + Vec3::new(jitter(), jitter(), jitter()),
                        v: Vec3::ZERO,
                        c: Mat3::ZERO,
                        f: Mat3::IDENTITY,
                        jp: 1.0,
                    });
                }
            }
        }
        self.placed = true;
    }

    fn node(&self, i: usize, j: usize, k: usize) -> usize {
        (k * self.n[1] + j) * self.n[0] + i
    }

    /// Which nodes are inside what is solid, and the way out: found again
    /// when the obstacles move.
    fn find_solid(&mut self, obstacles: &[Obstacle]) {
        if self.solid_from.as_deref() == Some(obstacles) {
            return;
        }
        let nodes = self.n[0] * self.n[1] * self.n[2];
        self.solid = vec![None; nodes];
        for k in 0..self.n[2] {
            for j in 0..self.n[1] {
                for i in 0..self.n[0] {
                    let p = self.origin + Vec3::new(i as f32, j as f32, k as f32) * self.dx;
                    let out = obstacles.iter().find_map(|o| o.contact(p, 0.0).map(|(n, _)| n));
                    let at = self.node(i, j, k);
                    self.solid[at] = out;
                }
            }
        }
        self.solid_from = Some(obstacles.to_vec());
    }

    /// Along by `seconds`, over `obstacles`.
    pub fn advance(&mut self, placed: Mat4, obstacles: &[Obstacle], seconds: f32) {
        if !self.placed {
            self.fill(placed);
        }
        let changed = self.solid_from.as_deref() != Some(obstacles);
        self.find_solid(obstacles);
        self.owed = (self.owed + seconds.max(0.0)).min(0.1);
        let mut changed = changed;
        while self.owed >= STEP {
            self.owed -= STEP;
            // Settled and left alone: nothing to step.
            self.at.clear();
            self.at.extend(self.grains.iter().map(|g| g.x));
            if self.rest.asleep(&self.at, std::mem::take(&mut changed)) {
                continue;
            }
            let (substeps, stiffness) = self.stepping();
            let dt = STEP / substeps as f32;
            for _ in 0..substeps {
                self.substep(dt, stiffness);
            }
            self.rest.stepped(runity_soft::rest::fastest(self.grains.iter().map(|g| g.v)));
        }
    }

    /// Settled, and not stepped until something could move it.
    pub fn asleep(&self) -> bool {
        self.rest.sleeping()
    }

    /// How much harder packed snow grows, at the most: stepped for that.
    fn hardening(&self) -> f32 {
        if self.mpm.material == MpmMaterial::Snow {
            10.0
        } else {
            1.0
        }
    }

    /// Substeps a step (a wave crosses no more than a fraction of a cell
    /// each), as many as its stiffness needs up to `max_substeps`, and the
    /// stiffness they keep stable: its own, or less.
    fn stepping(&self) -> (usize, f32) {
        let hard = self.hardening();
        let stiffness = self.mpm.stiffness.max(1.0);
        let needed = |k: f32| (STEP * (k * hard).sqrt() / (self.dx * 0.3)).ceil() as usize;
        let most = (self.mpm.max_substeps.max(1) as usize).min(400);
        let n = needed(stiffness).clamp(4.min(most), most);
        if needed(stiffness) <= n {
            return (n, stiffness);
        }
        // The stiffest `n` substeps hold.
        let k = (n as f32 * self.dx * 0.3 / STEP).powi(2) / hard;
        (n, k.min(stiffness))
    }

    /// One substep: particles to the grid, the grid moved, the grid back to
    /// the particles. What each particle and each slice of the grid does
    /// alone is spread over the cores (`runity_core::jobs`); only the
    /// scatter onto the grid, where particles share nodes, runs in order —
    /// so the result is the same on any number of them.
    fn substep(&mut self, dt: f32, stiffness: f32) {
        use runity_core::jobs;
        let dx = self.dx;
        let inv = 1.0 / dx;
        let vol = (dx * 0.5).powi(3);
        let mass = vol * 1000.0;
        let e = stiffness * 1000.0;
        let nu = 0.2;
        let mu0 = e / (2.0 * (1.0 + nu));
        let lambda0 = e * nu / ((1.0 + nu) * (1.0 - 2.0 * nu));
        self.grid_v.fill(Vec3::ZERO);
        self.grid_m.fill(0.0);
        let material = self.mpm.material;
        let [nx, ny, nz] = self.n;
        // Each particle's stress, and from it what it hands its nodes: the
        // costly part (a decomposition of its deformation), and its own.
        let affine: Vec<Mat3> = jobs::map(&self.grains, 256, |g| {
            stress_of(g, material, mu0, lambda0) * (-dt * vol * 4.0 * inv * inv) + g.c * mass
        });
        // Particles to grid, in order: they share nodes.
        for (g, affine) in self.grains.iter().zip(&affine) {
            let base = (g.x - self.origin) * inv - Vec3::splat(0.5);
            let base = base.floor();
            let fx = (g.x - self.origin) * inv - base;
            let w = weights(fx);
            let (bi, bj, bk) = (base.x as i64, base.y as i64, base.z as i64);
            for a in 0..3 {
                for b in 0..3 {
                    for c in 0..3 {
                        let (i, j, k) = (bi + a as i64, bj + b as i64, bk + c as i64);
                        if i < 0 || j < 0 || k < 0 || i as usize >= nx || j as usize >= ny || k as usize >= nz {
                            continue;
                        }
                        let weight = w[a].x * w[b].y * w[c].z;
                        let dpos = (Vec3::new(a as f32, b as f32, c as f32) - fx) * dx;
                        let at = (k as usize * ny + j as usize) * nx + i as usize;
                        self.grid_v[at] += (g.v * mass + *affine * dpos) * weight;
                        self.grid_m[at] += mass * weight;
                    }
                }
            }
        }
        // The grid: momentum to speed, gravity, and what is solid — a
        // slice of it to a core.
        // Snow and sand grip the floor; water and jelly slide on it.
        let sticky = matches!(material, MpmMaterial::Snow | MpmMaterial::Sand);
        let slice = nx * ny;
        {
            let (grid_m, grid_v) = (&self.grid_m, &self.grid_v);
            jobs::for_each_chunk_mut(&mut self.grid_was, slice, |first, was| {
                for (o, w) in was.iter_mut().enumerate() {
                    let m = grid_m[first + o];
                    if m > 0.0 {
                        *w = grid_v[first + o] / m;
                    }
                }
            });
        }
        {
            let (grid_m, solid) = (&self.grid_m, &self.solid);
            jobs::for_each_chunk_mut(&mut self.grid_v, slice, |first, nodes| {
                let k = first / slice;
                for (o, node) in nodes.iter_mut().enumerate() {
                    let at = first + o;
                    let m = grid_m[at];
                    if m <= 0.0 {
                        continue;
                    }
                    let (i, j) = (o % nx, o / nx);
                    let mut v = *node / m;
                    v.y -= 9.81 * dt;
                    // The room's walls: nothing leaves.
                    if (i < 2 && v.x < 0.0) || (i + 3 > nx && v.x > 0.0) {
                        v.x = 0.0;
                    }
                    if j < 2 && v.y < 0.0 {
                        v.y = 0.0;
                        if sticky {
                            v = Vec3::ZERO;
                        } else {
                            v.x *= 0.9;
                            v.z *= 0.9;
                        }
                    }
                    if j + 3 > ny && v.y > 0.0 {
                        v.y = 0.0;
                    }
                    if (k < 2 && v.z < 0.0) || (k + 3 > nz && v.z > 0.0) {
                        v.z = 0.0;
                    }
                    // What is solid: no going in, and a little grip along.
                    if let Some(out) = solid[at] {
                        let into = v.dot(out);
                        if into < 0.0 {
                            v -= out * into;
                            v *= if sticky { 0.0 } else { 0.9 };
                        }
                    }
                    *node = v;
                }
            });
        }
        // Grid to particles: each reads the nodes round it and moves itself.
        let flip = match self.mpm.transfer {
            Transfer::Flip(share) => Some(share.clamp(0.0, 1.0)),
            Transfer::Apic => None,
        };
        let (grid_v, grid_was, origin) = (&self.grid_v, &self.grid_was, self.origin);
        jobs::for_each_mut(&mut self.grains, 256, |g| {
            let base = ((g.x - origin) * inv - Vec3::splat(0.5)).floor();
            let fx = (g.x - origin) * inv - base;
            let w = weights(fx);
            let mut v = Vec3::ZERO;
            let mut change = Vec3::ZERO;
            let mut c = Mat3::ZERO;
            let (bi, bj, bk) = (base.x as i64, base.y as i64, base.z as i64);
            for a in 0..3 {
                for b in 0..3 {
                    for cc in 0..3 {
                        let (i, j, k) = (bi + a as i64, bj + b as i64, bk + cc as i64);
                        if i < 0 || j < 0 || k < 0 || i as usize >= nx || j as usize >= ny || k as usize >= nz {
                            continue;
                        }
                        let weight = w[a].x * w[b].y * w[cc].z;
                        let at = (k as usize * ny + j as usize) * nx + i as usize;
                        let gv = grid_v[at];
                        let dpos = Vec3::new(a as f32, b as f32, cc as f32) - fx;
                        v += gv * weight;
                        change += (gv - grid_was[at]) * weight;
                        c += outer(gv * weight, dpos) * (4.0 * inv);
                    }
                }
            }
            g.v = match flip {
                Some(share) => (g.v + change) * share + v * (1.0 - share),
                None => v,
            };
            g.c = if flip.is_some() { c * 0.0 } else { c };
            g.x += g.v * dt;
            // Kept inside the room.
            let low = origin + Vec3::splat(dx);
            let high = origin + Vec3::new(nx as f32 - 2.0, ny as f32 - 2.0, nz as f32 - 2.0) * dx;
            g.x = g.x.clamp(low, high);
            let f = (Mat3::IDENTITY + c * dt) * g.f;
            g.f = plastic(f, material, &mut g.jp);
        });
    }

    /// A small box at each particle, in the world: how snow and sand are
    /// drawn.
    pub fn grains_placed(&self) -> Vec<Mat4> {
        let size = self.dx * 0.55;
        self.grains.iter().map(|g| Mat4::from_scale_rotation_translation(Vec3::splat(size), glam::Quat::IDENTITY, g.x)).collect()
    }

    /// Its surface, in the space of `placed`: how water and jelly are drawn.
    pub fn surface(&self, placed: Mat4) -> (Vec<runity_geometry::mesh_asset::Vertex>, Vec<u32>) {
        if self.grains.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let s = self.dx * 0.5;
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for g in &self.grains {
            low = low.min(g.x);
            high = high.max(g.x);
        }
        let cell = s * 1.2;
        let margin = Vec3::splat(s * 4.0);
        let origin = low - margin;
        let size = ((high - low + margin * 2.0) / cell).ceil();
        let dims = [size.x as usize + 1, size.y as usize + 1, size.z as usize + 1];
        if dims.iter().product::<usize>() > 4_000_000 {
            return (Vec::new(), Vec::new());
        }
        let mut field = runity_geometry::field::Field::new(dims, origin, cell);
        for g in &self.grains {
            field.splat(g.x, s * 2.2, 1.0);
        }
        let (mut vertices, indices) = field.surface(0.5);
        let back = placed.inverse();
        for v in &mut vertices {
            v.position = back.transform_point3(Vec3::from_array(v.position)).to_array();
            v.normal = back.transform_vector3(Vec3::from_array(v.normal)).normalize_or(Vec3::Y).to_array();
        }
        (vertices, indices)
    }
}

/// The quadratic B-spline's weights along each axis for the three nodes
/// round a particle `fx` cells past the first.
fn weights(fx: Vec3) -> [Vec3; 3] {
    [
        (Vec3::splat(1.5) - fx) * (Vec3::splat(1.5) - fx) * 0.5,
        Vec3::splat(0.75) - (fx - Vec3::ONE) * (fx - Vec3::ONE),
        (fx - Vec3::splat(0.5)) * (fx - Vec3::splat(0.5)) * 0.5,
    ]
}

fn outer(a: Vec3, b: Vec3) -> Mat3 {
    Mat3::from_cols(a * b.x, a * b.y, a * b.z)
}

/// The Kirchhoff stress of a particle, by its material.
fn stress_of(g: &Grain, material: MpmMaterial, mu0: f32, lambda0: f32) -> Mat3 {
    let j = g.f.determinant();
    match material {
        MpmMaterial::Water => {
            // Pressure from how much it is squeezed: J − 1.
            let k = lambda0 * 0.5;
            Mat3::IDENTITY * (k * (j - 1.0) * j)
        }
        MpmMaterial::Jelly | MpmMaterial::Snow => {
            let (mu, lambda) = if material == MpmMaterial::Snow {
                // Packed snow is harder.
                let hard = (10.0 * (1.0 - g.jp)).exp().min(10.0);
                (mu0 * hard, lambda0 * hard)
            } else {
                (mu0 * 0.3, lambda0 * 0.3)
            };
            let (u, _, v) = svd(g.f);
            let r = u * v.transpose();
            (g.f - r) * g.f.transpose() * (2.0 * mu) + Mat3::IDENTITY * (lambda * (j - 1.0) * j)
        }
        MpmMaterial::Sand => {
            // St. Venant–Kirchhoff in log strain; what plasticity left.
            let (u, sigma, v) = svd(g.f);
            let _ = v;
            let log = Vec3::new(sigma.x.max(1e-4).ln(), sigma.y.max(1e-4).ln(), sigma.z.max(1e-4).ln());
            let trace = log.x + log.y + log.z;
            let tau = log * (2.0 * mu0) + Vec3::splat(lambda0 * trace);
            u * Mat3::from_diagonal(tau) * u.transpose()
        }
    }
}

/// A deformation after what the material could not hold is let go.
fn plastic(f: Mat3, material: MpmMaterial, jp: &mut f32) -> Mat3 {
    match material {
        MpmMaterial::Water => {
            // Only its volume counts: a shear is forgotten.
            let j = f.determinant().clamp(0.2, 2.0);
            Mat3::from_diagonal(Vec3::new(j, 1.0, 1.0))
        }
        MpmMaterial::Jelly => f,
        MpmMaterial::Snow => {
            let (u, sigma, v) = svd(f);
            let kept = sigma.clamp(Vec3::splat(1.0 - 2.5e-2), Vec3::splat(1.0 + 4.5e-3));
            *jp = (*jp * (sigma.x * sigma.y * sigma.z) / (kept.x * kept.y * kept.z)).clamp(0.6, 20.0);
            u * Mat3::from_diagonal(kept) * v.transpose()
        }
        MpmMaterial::Sand => {
            let (u, sigma, v) = svd(f);
            let log = Vec3::new(sigma.x.max(1e-4).ln(), sigma.y.max(1e-4).ln(), sigma.z.max(1e-4).ln());
            let trace = log.x + log.y + log.z;
            let kept = if trace > 0.0 {
                // Pulled apart: sand does not hold together at all.
                Vec3::ZERO
            } else {
                let shear = log - Vec3::splat(trace / 3.0);
                let shear_len = shear.length();
                // Friction: 30° or so; the cone of what shear it holds.
                // The friction angle's 35°: sin φ ≈ 0.57.
                let alpha = (2.0f32 / 3.0).sqrt() * 2.0 * 0.57 / (3.0 - 0.57);
                // (dλ + 2μ) / 2μ is 2 for ν = 0.2.
                let slip = shear_len + 2.0 * alpha * trace;
                if slip <= 0.0 || shear_len < 1e-9 {
                    log
                } else {
                    log - shear / shear_len * slip
                }
            };
            u * Mat3::from_diagonal(Vec3::new(kept.x.exp(), kept.y.exp(), kept.z.exp())) * v.transpose()
        }
    }
}

/// The singular value decomposition of `f`: `u · diag(σ) · vᵀ`, with `u`
/// and `v` turns (no reflection) — through the eigenvectors of `fᵀf`
/// (Jacobi rotations).
fn svd(f: Mat3) -> (Mat3, Vec3, Mat3) {
    let a = f.transpose() * f;
    let mut m = [[a.x_axis.x, a.y_axis.x, a.z_axis.x], [a.x_axis.y, a.y_axis.y, a.z_axis.y], [a.x_axis.z, a.y_axis.z, a.z_axis.z]];
    let mut v = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..8 {
        // Done once what is off the diagonal is nothing beside it: a
        // deformation is mostly a turn, and most need a sweep or two.
        let off = m[0][1] * m[0][1] + m[0][2] * m[0][2] + m[1][2] * m[1][2];
        let diagonal = m[0][0] * m[0][0] + m[1][1] * m[1][1] + m[2][2] * m[2][2];
        if off <= diagonal * 1e-12 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if m[p][q].abs() < 1e-12 {
                continue;
            }
            let theta = (m[q][q] - m[p][p]) / (2.0 * m[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for k in 0..3 {
                let (mkp, mkq) = (m[k][p], m[k][q]);
                m[k][p] = c * mkp - s * mkq;
                m[k][q] = s * mkp + c * mkq;
            }
            for k in 0..3 {
                let (mpk, mqk) = (m[p][k], m[q][k]);
                m[p][k] = c * mpk - s * mqk;
                m[q][k] = s * mpk + c * mqk;
            }
            for row in &mut v {
                let (vp, vq) = (row[p], row[q]);
                row[p] = c * vp - s * vq;
                row[q] = s * vp + c * vq;
            }
        }
    }
    let mut vm = Mat3::from_cols(
        Vec3::new(v[0][0], v[1][0], v[2][0]),
        Vec3::new(v[0][1], v[1][1], v[2][1]),
        Vec3::new(v[0][2], v[1][2], v[2][2]),
    );
    if vm.determinant() < 0.0 {
        vm.z_axis = -vm.z_axis;
    }
    let sigma = Vec3::new(m[0][0].max(0.0).sqrt(), m[1][1].max(0.0).sqrt(), m[2][2].max(0.0).sqrt());
    let fv = f * vm;
    let col = |c: Vec3, s: f32, fallback: Vec3| if s > 1e-6 { c / s } else { fallback };
    let x = col(fv.x_axis, sigma.x, Vec3::X);
    let y = col(fv.y_axis, sigma.y, x.any_orthonormal_vector());
    let z = x.cross(y);
    let mut sigma = sigma;
    if fv.z_axis.dot(z) < 0.0 {
        // f reflects: the last value takes the sign, u stays a turn.
        sigma.z = -sigma.z;
    }
    let u = Mat3::from_cols(x, y, z);
    (u, sigma, vm)
}

/// Step every MPM block by `seconds` over `obstacles`.
pub fn run_mpm(world: &mut hecs::World, seconds: f32, obstacles: &Obstacles) {
    let mut near = Vec::new();
    for (state, placed) in world.query_mut::<(&mut MpmState, &WorldTransform)>() {
        let feet = placed.0.w_axis.truncate();
        let half = state.mpm.domain * 0.5;
        obstacles.near(feet - Vec3::new(half.x, 0.5, half.z), feet + Vec3::new(half.x, state.mpm.domain.y, half.z), &mut near);
        state.advance(placed.0, &near, seconds);
    }
}

/// The fluid module's dresser for MPM.
pub struct MpmDress;

impl runity_core::world::Dress for MpmDress {
    fn parts(&self) -> &[&'static str] {
        &["mpm"]
    }

    fn dress(
        &mut self,
        line: &runity_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: runity_core::world::Changed,
        _: &mut Vec<runity_core::world::Unresolved>,
    ) {
        match line.mpm() {
            Some(mpm) => {
                let _ = world.insert_one(entity, MpmState::new(mpm));
            }
            None => {
                let _ = world.remove_one::<MpmState>(entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drop(material: MpmMaterial, transfer: Transfer, seconds: f32) -> MpmState {
        // A block 0.4 m across, its foot 0.4 m over the floor of a room
        // 1.6 × 1.2 × 0.8 m.
        let mpm = Mpm { material, transfer, size: Vec3::splat(0.4), height: 0.4, domain: Vec3::new(1.6, 1.2, 0.8), resolution: 16.0, ..Mpm::default() };
        let mut state = MpmState::new(mpm);
        let feet = Mat4::IDENTITY;
        state.advance(feet, &[], 0.0);
        for _ in 0..(seconds * 60.0) as usize {
            state.advance(feet, &[], 1.0 / 60.0);
        }
        state
    }

    fn top(state: &MpmState) -> f32 {
        state.points().map(|p| p.y).fold(f32::MIN, f32::max)
    }

    fn spread(state: &MpmState) -> f32 {
        let (lo, hi) = state.points().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
        hi - lo
    }

    #[test]
    fn the_svd_takes_a_deformation_apart_and_puts_it_together() {
        let f = Mat3::from_cols(Vec3::new(1.2, 0.1, -0.2), Vec3::new(0.05, 0.9, 0.3), Vec3::new(-0.1, 0.2, 1.1));
        let (u, s, v) = svd(f);
        let back = u * Mat3::from_diagonal(s) * v.transpose();
        for (a, b) in back.to_cols_array().iter().zip(f.to_cols_array()) {
            assert!((a - b).abs() < 1e-3, "{back} vs {f}");
        }
        assert!((u.determinant() - 1.0).abs() < 1e-3 && (v.determinant() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn water_spreads_flat_sand_heaps_and_jelly_keeps_its_shape() {
        let water = drop(MpmMaterial::Water, Transfer::Apic, 2.0);
        let sand = drop(MpmMaterial::Sand, Transfer::Apic, 2.0);
        let jelly = drop(MpmMaterial::Jelly, Transfer::Apic, 2.0);
        // All on the floor.
        for s in [&water, &sand, &jelly] {
            assert!(s.points().all(|p| p.y > -0.01 && p.y < 1.2));
        }
        // Water runs out to the walls, low; sand slumps but stands as a
        // heap; jelly stands nearly as tall as it was.
        assert!(top(&water) < 0.2, "water top {}", top(&water));
        assert!(spread(&water) > 1.2, "water spreads {}", spread(&water));
        assert!(top(&sand) > top(&water) + 0.05 && top(&sand) < 0.4, "sand {}", top(&sand));
        assert!(spread(&sand) > 0.45, "sand slumps {}", spread(&sand));
        assert!(top(&jelly) > 0.3, "jelly stands {}", top(&jelly));
    }

    #[test]
    fn flip_keeps_the_particles_own_speed_and_pic_smooths_it() {
        let flip = drop(MpmMaterial::Water, Transfer::Flip(0.95), 0.5);
        let pic = drop(MpmMaterial::Water, Transfer::Flip(0.0), 0.5);
        let energy = |s: &MpmState| s.grains.iter().map(|g| g.v.length_squared()).sum::<f32>();
        assert!(energy(&flip) > energy(&pic), "FLIP is livelier: {} vs {}", energy(&flip), energy(&pic));
    }
}
