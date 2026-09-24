//! Soft bodies and flesh: a line's `soft_body`. `soft_body: ()` on a line
//! with a `model` makes the thing itself soft — a jelly cube, a rubber
//! ball, a sack of grain — falling, squashing on what it meets and coming
//! back to shape. With `core`, its middle is held to the entity and only
//! what is round it gives: flesh on a bone, a belly that wobbles as the
//! creature moves (Ziva's idea, as a cage rather than muscles).
//!
//! The body is a lattice of particles through the entity's box, each cell
//! cut into six tetrahedra. Two ways to hold it together:
//!
//! * `Tetrahedra` (the default): XPBD on each tetrahedron's edges and
//!   volume (Macklin, Müller 2016; Müller's "ten minutes" soft bodies) —
//!   it bends and squashes locally, and keeps its volume as `volume` says.
//! * `ShapeMatching`: the whole lattice pulled toward its rest shape,
//!   turned as the particles are turned (Müller et al. 2005) — cheaper,
//!   stiffer, never inverts: a gummy bear.
//!
//! The model is drawn deformed: each of its vertices sits in a tetrahedron
//! of the lattice, and goes where that tetrahedron goes (embedded, as FEM
//! games draw a detailed mesh on a coarse cage). A model the module cannot
//! read — one from a library — is drawn as the lattice's own surface.

use glam::{Mat3, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use runity_core::world::WorldTransform;
use runity_geometry::mesh_asset::Vertex;

use crate::obstacle::{Obstacle, Obstacles};
use crate::particles::Particles;

/// A soft body, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoftBody {
    /// Its shape: the entity's unit box, or the ball in it.
    pub shape: SoftShape,
    /// Cells of the lattice along its longest side.
    pub cells: u32,
    /// How hard it holds its shape, 0 (slime) to 1 (hard rubber): the share
    /// of a bend each substep undoes.
    pub stiffness: f32,
    /// How much it keeps its volume, 0 to 1.
    pub volume: f32,
    /// How it is held together.
    pub method: Method,
    /// Kilograms, all of it.
    pub weight: f32,
    /// Its middle, held to the entity as flesh to a bone: the share of its
    /// size that is bone, 0 (none: it falls free) to 1.
    pub core: f32,
    pub friction: f32,
}

/// A soft body's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SoftShape {
    /// The entity's unit box: `builtin:cube`.
    #[default]
    Box,
    /// The ball in it: `builtin:sphere`.
    Ball,
}

/// How a soft body is held together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Method {
    #[default]
    Tetrahedra,
    ShapeMatching,
}

impl Default for SoftBody {
    fn default() -> Self {
        Self {
            shape: SoftShape::Box,
            cells: 5,
            stiffness: 0.5,
            volume: 1.0,
            method: Method::Tetrahedra,
            weight: 5.0,
            core: 0.0,
            friction: 0.6,
        }
    }
}

runity_core::impl_parts! {
    SoftBody => "soft_body";
}

/// The soft body of a line, read off it.
pub trait SoftBodyLine {
    fn soft_body(&self) -> Option<SoftBody>;
}

impl SoftBodyLine for runity_core::EntityDesc {
    fn soft_body(&self) -> Option<SoftBody> {
        self.part()
    }
}

impl SoftBodyLine for runity_core::scene::Override {
    fn soft_body(&self) -> Option<SoftBody> {
        self.part()
    }
}

pub const STEP: f32 = 1.0 / 60.0;
pub const SUBSTEPS: usize = 10;

/// A soft body as it moves: the component [`run_soft_bodies`] steps.
#[derive(Debug, Clone)]
pub struct SoftBodyState {
    pub body: SoftBody,
    particles: Particles,
    /// Each particle at rest, in the entity's unit space.
    rest: Vec<Vec3>,
    /// Held to the entity: the core.
    held: Vec<bool>,
    edges: Vec<(u32, u32, f32)>,
    tets: Vec<([u32; 4], f32)>,
    /// What is drawn: the model's vertices, each in a tetrahedron with its
    /// barycentric weights, and its triangles.
    embedded: Vec<(u32, [f32; 4])>,
    uvs: Vec<[f32; 2]>,
    triangles: Vec<u32>,
    /// The particles' rest centre and the offsets from it, for shape
    /// matching.
    rest_centre: Vec3,
    turn: Quat,
    placed: Option<Mat4>,
    /// Where it was put, for its first step and for shape matching.
    hung: Mat4,
    near: Vec<Obstacle>,
    owed: f32,
}

/// A lattice cell's corner by its three bits.
fn corner(c: u32) -> [u32; 3] {
    [c & 1, (c >> 1) & 1, (c >> 2) & 1]
}

impl SoftBodyState {
    /// A soft body, drawn as `mesh` (the model's vertices and triangles, in
    /// the entity's space) or, with none, as its lattice.
    pub fn new(body: SoftBody, mesh: Option<(&[Vertex], &[u32])>) -> Self {
        let n = body.cells.clamp(1, 16) as usize;
        let side = n + 1;
        let at = |i: usize, j: usize, k: usize| (k * side + j) * side + i;
        let point = |i: usize, j: usize, k: usize| Vec3::new(i as f32, j as f32, k as f32) / n as f32 - Vec3::splat(0.5);
        // Which cells are in it: all of a box, those touching the ball.
        let inside = |i: usize, j: usize, k: usize| match body.shape {
            SoftShape::Box => true,
            SoftShape::Ball => (0..8).any(|c| {
                let [a, b, d] = corner(c);
                point(i + a as usize, j + b as usize, k + d as usize).length() <= 0.5 + 1e-4
            }),
        };
        let mut used = vec![u32::MAX; side * side * side];
        let mut rest = Vec::new();
        let mut cells = Vec::new();
        for k in 0..n {
            for j in 0..n {
                for i in 0..n {
                    if !inside(i, j, k) {
                        continue;
                    }
                    let mut ids = [0u32; 8];
                    for (c, id) in ids.iter_mut().enumerate() {
                        let [a, b, d] = corner(c as u32);
                        let g = at(i + a as usize, j + b as usize, k + d as usize);
                        if used[g] == u32::MAX {
                            used[g] = rest.len() as u32;
                            rest.push(point(i + a as usize, j + b as usize, k + d as usize));
                        }
                        *id = used[g];
                    }
                    cells.push(((i, j, k), ids));
                }
            }
        }
        // Six tetrahedra a cell, along each way from its corner 0 to 7.
        let mut tets = Vec::new();
        let mut cell_tets: std::collections::HashMap<(usize, usize, usize), [usize; 6]> = Default::default();
        const WAYS: [[u32; 3]; 6] = [[1, 2, 4], [1, 4, 2], [2, 1, 4], [2, 4, 1], [4, 1, 2], [4, 2, 1]];
        for (cell, ids) in &cells {
            let mut these = [0usize; 6];
            for (w, way) in WAYS.iter().enumerate() {
                let (a, b) = (way[0], way[0] | way[1]);
                let t = [ids[0], ids[a as usize], ids[b as usize], ids[7]];
                let v = volume(&rest, t);
                these[w] = tets.len();
                tets.push((t, v));
            }
            cell_tets.insert(*cell, these);
        }
        let mut edges: Vec<(u32, u32, f32)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (t, _) in &tets {
            for (a, b) in [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)] {
                let (p, q) = (t[a].min(t[b]), t[a].max(t[b]));
                if seen.insert((p, q)) {
                    edges.push((p, q, rest[p as usize].distance(rest[q as usize])));
                }
            }
        }
        let core = body.core.clamp(0.0, 1.0) * 0.5;
        let held: Vec<bool> = rest
            .iter()
            .map(|p| core > 0.0 && p.abs().max_element() <= core + 1e-4)
            .collect();
        // What is drawn: the model in the lattice, or the lattice's skin.
        let (embedded, uvs, triangles) = match mesh {
            Some((vertices, indices)) => {
                let embedded = vertices
                    .iter()
                    .map(|v| embed(Vec3::from_array(v.position), n, &cell_tets, &tets, &rest))
                    .collect();
                (embedded, vertices.iter().map(|v| v.uv).collect(), indices.to_vec())
            }
            None => skin(&tets, &rest),
        };
        let mass = (body.weight.max(0.01) / rest.len() as f32).max(1e-4);
        let mut particles = Particles::new(rest.clone(), mass);
        for (k, h) in held.iter().enumerate() {
            if *h {
                particles.w[k] = 0.0;
            }
        }
        let rest_centre = rest.iter().sum::<Vec3>() / rest.len() as f32;
        Self {
            body,
            particles,
            rest,
            held,
            edges,
            tets,
            embedded,
            uvs,
            triangles,
            rest_centre,
            turn: Quat::IDENTITY,
            placed: None,
            hung: Mat4::IDENTITY,
            near: Vec::new(),
            owed: 0.0,
        }
    }

    /// Where its particles are, in the world.
    pub fn points(&self) -> &[Vec3] {
        &self.particles.x
    }

    /// Its volume now, cubic metres.
    pub fn volume_now(&self) -> f32 {
        // Half the tetrahedra of a cell are wound the other way: each by
        // the sign it has at rest.
        self.tets.iter().map(|(t, rest)| volume(&self.particles.x, *t) * rest.signum()).sum()
    }

    fn place(&mut self, placed: Mat4) {
        for (k, r) in self.rest.iter().enumerate() {
            self.particles.place(k, placed.transform_point3(*r));
        }
        // Rest lengths and volumes in the world, as the entity is scaled.
        for e in &mut self.edges {
            e.2 = self.particles.x[e.0 as usize].distance(self.particles.x[e.1 as usize]);
        }
        for t in &mut self.tets {
            t.1 = volume(&self.particles.x, t.0);
        }
        self.hung = placed;
        self.placed = Some(placed);
    }

    /// Along by `seconds`, its core where the entity is, lying on
    /// `obstacles`.
    pub fn advance(&mut self, placed: Mat4, obstacles: &[Obstacle], seconds: f32) {
        if self.placed.is_none() {
            self.place(placed);
        }
        self.owed = (self.owed + seconds.max(0.0)).min(0.25);
        while self.owed >= STEP {
            self.owed -= STEP;
            self.step(placed, obstacles);
        }
    }

    fn step(&mut self, placed: Mat4, obstacles: &[Obstacle]) {
        let was = self.placed.unwrap_or(placed);
        self.placed = Some(placed);
        let h = STEP / SUBSTEPS as f32;
        let s = self.body.stiffness.clamp(0.01, 1.0);
        // Stiffness to compliance, against the body's weight and lattice so
        // that the same stiffness sags a heavy body and a light one alike:
        // at 0.5 a cube sags a few per cent under itself, at 0.1 a fifth.
        let give = 0.15 * (1.0 / s - 1.0) / (self.body.weight.max(0.01) * self.body.cells.max(1) as f32);
        let soft = give / (h * h);
        let keep_volume = self.body.volume.clamp(0.0, 1.0);
        let radius = 0.01;
        for sub in 0..SUBSTEPS {
            let t = (sub + 1) as f32 / SUBSTEPS as f32;
            for (k, r) in self.rest.iter().enumerate() {
                if self.held[k] {
                    self.particles.x[k] = was.transform_point3(*r).lerp(placed.transform_point3(*r), t);
                }
            }
            self.particles.predict(h, |_| Vec3::new(0.0, -9.81, 0.0));
            match self.body.method {
                Method::Tetrahedra => {
                    for &(a, b, rest) in &self.edges {
                        let (a, b) = (a as usize, b as usize);
                        let p = &mut self.particles;
                        let sum = p.w[a] + p.w[b];
                        if sum <= 0.0 {
                            continue;
                        }
                        let d = p.x[b] - p.x[a];
                        let l = d.length();
                        if l < 1e-9 {
                            continue;
                        }
                        let lambda = -(l - rest) / (sum + soft);
                        let n = d / l;
                        p.x[a] -= n * lambda * p.w[a];
                        p.x[b] += n * lambda * p.w[b];
                    }
                    if keep_volume > 0.0 {
                        for i in 0..self.tets.len() {
                            let (t, rest) = self.tets[i];
                            self.hold_volume(t, rest, keep_volume);
                        }
                    }
                }
                    Method::ShapeMatching => self.match_shape(s * 0.2),
            }
            self.particles.collide(radius, self.body.friction, obstacles);
            self.particles.finish(h, 0.5);
        }
    }

    /// One tetrahedron held at its rest volume, by `share` of what is off.
    fn hold_volume(&mut self, t: [u32; 4], rest: f32, share: f32) {
        let p = &mut self.particles;
        let [a, b, c, d] = t.map(|i| i as usize);
        let (x0, x1, x2, x3) = (p.x[a], p.x[b], p.x[c], p.x[d]);
        let g1 = (x2 - x0).cross(x3 - x0) / 6.0;
        let g2 = (x3 - x0).cross(x1 - x0) / 6.0;
        let g3 = (x1 - x0).cross(x2 - x0) / 6.0;
        let g0 = -(g1 + g2 + g3);
        let sum = p.w[a] * g0.length_squared() + p.w[b] * g1.length_squared() + p.w[c] * g2.length_squared() + p.w[d] * g3.length_squared();
        if sum < 1e-12 {
            return;
        }
        let v = (x1 - x0).dot((x2 - x0).cross(x3 - x0)) / 6.0;
        let lambda = -(v - rest) / sum * share;
        p.x[a] += g0 * lambda * p.w[a];
        p.x[b] += g1 * lambda * p.w[b];
        p.x[c] += g2 * lambda * p.w[c];
        p.x[d] += g3 * lambda * p.w[d];
    }

    /// Every particle pulled toward where its rest shape, fitted to where
    /// the particles are, puts it.
    fn match_shape(&mut self, share: f32) {
        let p = &mut self.particles;
        let n = p.len() as f32;
        let centre = p.x.iter().sum::<Vec3>() / n;
        let hung = self.hung;
        let rest_centre = hung.transform_point3(self.rest_centre);
        // A = Σ (x − c)(x₀ − c₀)ᵀ, and its turn.
        let mut a = Mat3::ZERO;
        for (x, r) in p.x.iter().zip(&self.rest) {
            let q = hung.transform_point3(*r) - rest_centre;
            let d = *x - centre;
            a += Mat3::from_cols(d * q.x, d * q.y, d * q.z);
        }
        self.turn = rotation_of(a, self.turn);
        for (k, r) in self.rest.iter().enumerate() {
            if p.w[k] <= 0.0 {
                continue;
            }
            let goal = centre + self.turn * (hung.transform_point3(*r) - rest_centre);
            let off = goal - p.x[k];
            p.x[k] += off * share;
        }
    }

    /// What is drawn, in the space of `placed`.
    pub fn mesh(&self, placed: Mat4) -> (Vec<Vertex>, Vec<u32>) {
        if self.placed.is_none() {
            return (Vec::new(), Vec::new());
        }
        let back = placed.inverse();
        let x = &self.particles.x;
        let positions: Vec<Vec3> = self
            .embedded
            .iter()
            .map(|(t, w)| {
                let ids = self.tets[*t as usize].0;
                let p = x[ids[0] as usize] * w[0] + x[ids[1] as usize] * w[1] + x[ids[2] as usize] * w[2] + x[ids[3] as usize] * w[3];
                back.transform_point3(p)
            })
            .collect();
        let mut normals = vec![Vec3::ZERO; positions.len()];
        for tri in self.triangles.chunks_exact(3) {
            let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            let n = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
            normals[a] += n;
            normals[b] += n;
            normals[c] += n;
        }
        let vertices = positions
            .iter()
            .zip(normals)
            .zip(&self.uvs)
            .map(|((p, n), uv)| Vertex {
                position: p.to_array(),
                normal: n.normalize_or(Vec3::Y).to_array(),
                uv: *uv,
            })
            .collect();
        (vertices, self.triangles.clone())
    }
}

/// The signed volume of a tetrahedron.
fn volume(x: &[Vec3], t: [u32; 4]) -> f32 {
    let [a, b, c, d] = t.map(|i| x[i as usize]);
    (b - a).dot((c - a).cross(d - a)) / 6.0
}

/// The turn nearest `a` (its rotational part), from `guess` on (Müller et
/// al. 2016, "A robust method to extract the rotational part").
fn rotation_of(a: Mat3, guess: Quat) -> Quat {
    let mut q = guess;
    for _ in 0..16 {
        let r = Mat3::from_quat(q);
        let omega = (r.x_axis.cross(a.x_axis) + r.y_axis.cross(a.y_axis) + r.z_axis.cross(a.z_axis))
            / ((r.x_axis.dot(a.x_axis) + r.y_axis.dot(a.y_axis) + r.z_axis.dot(a.z_axis)).abs() + 1e-9);
        let w = omega.length();
        if w < 1e-9 {
            break;
        }
        q = (Quat::from_axis_angle(omega / w, w) * q).normalize();
    }
    q
}

/// A point of the entity's unit space in the tetrahedron that holds it,
/// with its barycentric weights: the tetrahedron of its cell (or the
/// nearest cell the body has) it is most inside.
fn embed(
    p: Vec3,
    n: usize,
    cell_tets: &std::collections::HashMap<(usize, usize, usize), [usize; 6]>,
    tets: &[([u32; 4], f32)],
    rest: &[Vec3],
) -> (u32, [f32; 4]) {
    let g = ((p + Vec3::splat(0.5)) * n as f32).floor();
    let clamp = |v: f32| (v.max(0.0) as usize).min(n - 1);
    let home = (clamp(g.x), clamp(g.y), clamp(g.z));
    let candidates: Vec<usize> = match cell_tets.get(&home) {
        Some(these) => these.to_vec(),
        None => {
            // Its own cell is not in the body: the nearest that is.
            let near = cell_tets
                .keys()
                .min_by(|a, b| {
                    let d = |c: &(usize, usize, usize)| {
                        (c.0 as i64 - home.0 as i64).pow(2) + (c.1 as i64 - home.1 as i64).pow(2) + (c.2 as i64 - home.2 as i64).pow(2)
                    };
                    d(a).cmp(&d(b))
                })
                .unwrap();
            cell_tets[near].to_vec()
        }
    };
    let mut best = (candidates[0] as u32, [0.25; 4], f32::MIN);
    for t in candidates {
        let ids = tets[t].0;
        let [a, b, c, d] = ids.map(|i| rest[i as usize]);
        let m = Mat3::from_cols(b - a, c - a, d - a);
        if m.determinant().abs() < 1e-12 {
            continue;
        }
        let w = m.inverse() * (p - a);
        let weights = [1.0 - w.x - w.y - w.z, w.x, w.y, w.z];
        let least = weights.iter().cloned().fold(f32::MAX, f32::min);
        if least > best.2 {
            best = (t as u32, weights, least);
        }
    }
    (best.0, best.1)
}

/// The lattice's outside: the faces of tetrahedra no other shares, each a
/// vertex of its own (embedded at a corner), wound outward.
fn skin(tets: &[([u32; 4], f32)], rest: &[Vec3]) -> (Vec<(u32, [f32; 4])>, Vec<[f32; 2]>, Vec<u32>) {
    let mut count: std::collections::HashMap<[u32; 3], (usize, [usize; 3], usize)> = Default::default();
    for (t, (ids, _)) in tets.iter().enumerate() {
        for (face, other) in [([0, 1, 2], 3), ([0, 1, 3], 2), ([0, 2, 3], 1), ([1, 2, 3], 0)] {
            let mut key = face.map(|i| ids[i]);
            key.sort_unstable();
            let entry = count.entry(key).or_insert((0, face, t));
            entry.0 += 1;
            let _ = other;
        }
    }
    let mut embedded = Vec::new();
    let mut triangles = Vec::new();
    let mut faces: Vec<_> = count.into_iter().filter(|(_, (n, _, _))| *n == 1).collect();
    faces.sort_by_key(|(k, _)| *k);
    for (_, (_, face, t)) in faces {
        let ids = tets[t].0;
        let inner = [0, 1, 2, 3].into_iter().find(|i| !face.contains(i)).unwrap();
        let [a, b, c] = face.map(|i| rest[ids[i] as usize]);
        let outward = (b - a).cross(c - a).dot(a - rest[ids[inner] as usize]) > 0.0;
        let order = if outward { face } else { [face[0], face[2], face[1]] };
        for i in order {
            let mut w = [0.0; 4];
            w[i] = 1.0;
            triangles.push(embedded.len() as u32);
            embedded.push((t as u32, w));
        }
    }
    let uvs = vec![[0.0, 0.0]; embedded.len()];
    (embedded, uvs, triangles)
}

/// Step every soft body by `seconds`, lying on `obstacles`.
pub fn run_soft_bodies(world: &mut hecs::World, seconds: f32, obstacles: &Obstacles) {
    let mut near = Vec::new();
    for (state, placed) in world.query_mut::<(&mut SoftBodyState, &WorldTransform)>() {
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let fresh: Vec<Vec3>;
        let points = if state.placed.is_some() {
            state.points()
        } else {
            fresh = state.rest.iter().map(|r| placed.0.transform_point3(*r)).collect();
            &fresh
        };
        for p in points {
            low = low.min(*p);
            high = high.max(*p);
        }
        let room = Vec3::splat(1.0 + seconds * 20.0);
        obstacles.near(low - room, high + room, &mut near);
        state.near.clear();
        state.advance(placed.0, &near, seconds);
    }
}

/// The soft module's dresser for soft bodies: the line's `soft_body`, its
/// model drawn in it when it is one of the builtins.
pub struct SoftBodyDress;

impl runity_core::world::Dress for SoftBodyDress {
    fn parts(&self) -> &[&'static str] {
        &["soft_body", "model"]
    }

    fn dress(
        &mut self,
        line: &runity_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: runity_core::world::Changed,
        _: &mut Vec<runity_core::world::Unresolved>,
    ) {
        use runity_geometry::line::GeometryLine;
        match line.soft_body() {
            Some(body) => {
                let model = runity_geometry::builtin::by_name(line.model().as_str());
                let mesh = model.as_ref().map(|m| (&m.vertices[..], &m.indices[..]));
                let _ = world.insert_one(entity, SoftBodyState::new(body, mesh));
            }
            None => {
                let _ = world.remove_one::<SoftBodyState>(entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drop_onto_ground(body: SoftBody, seconds: f32) -> SoftBodyState {
        let cube = runity_geometry::builtin::cube(1.0);
        let mut state = SoftBodyState::new(body, Some((&cube.vertices, &cube.indices)));
        let placed = Mat4::from_translation(Vec3::new(0.0, 1.5, 0.0));
        for _ in 0..(seconds * 30.0) as usize {
            state.advance(placed, &[Obstacle::ground(0.0)], 1.0 / 30.0);
        }
        state
    }

    #[test]
    fn a_jelly_cube_lands_squashes_and_keeps_its_volume() {
        let state = drop_onto_ground(SoftBody { stiffness: 0.3, ..SoftBody::default() }, 3.0);
        let lowest = state.points().iter().map(|p| p.y).fold(f32::MAX, f32::min);
        let highest = state.points().iter().map(|p| p.y).fold(f32::MIN, f32::max);
        assert!(lowest > -0.01 && lowest < 0.03, "on the ground: {lowest}");
        // Soft: sagged under its own weight, a little lower than a metre.
        assert!(highest < 0.99 && highest > 0.6, "sags: {highest}");
        // Its volume kept near a cubic metre.
        let v = state.volume_now();
        assert!((v - 1.0).abs() < 0.08, "volume {v}");
        // The cube drawn in it: its 24 vertices, deformed with it.
        let (vertices, indices) = state.mesh(Mat4::from_translation(Vec3::new(0.0, 1.5, 0.0)));
        assert_eq!(vertices.len(), 24);
        assert_eq!(indices.len(), 36);
        let top = vertices.iter().map(|v| v.position[1] + 1.5).fold(f32::MIN, f32::max);
        assert!((top - highest).abs() < 0.05, "the model's top where the body's is: {top} {highest}");
    }

    #[test]
    fn stiffer_sags_less_and_shape_matching_holds_its_shape() {
        let height = |body: SoftBody| {
            let state = drop_onto_ground(body, 3.0);
            state.points().iter().map(|p| p.y).fold(f32::MIN, f32::max)
        };
        let soft = height(SoftBody { stiffness: 0.1, ..SoftBody::default() });
        let hard = height(SoftBody { stiffness: 0.9, ..SoftBody::default() });
        let matched = height(SoftBody { stiffness: 0.5, method: Method::ShapeMatching, ..SoftBody::default() });
        assert!(hard > soft + 0.03, "hard {hard} soft {soft}");
        assert!(matched > 0.95, "shape matching stands: {matched}");
    }

    #[test]
    fn flesh_on_a_bone_goes_with_it_and_lags_as_it_swings() {
        let body = SoftBody { core: 0.4, stiffness: 0.3, shape: SoftShape::Ball, cells: 6, ..SoftBody::default() };
        let sphere = runity_geometry::builtin::sphere(0.5, 16, 8);
        let mut state = SoftBodyState::new(body, Some((&sphere.vertices, &sphere.indices)));
        let at = |x: f32| Mat4::from_translation(Vec3::new(x, 2.0, 0.0));
        for _ in 0..30 {
            state.advance(at(0.0), &[], 1.0 / 30.0);
        }
        // Held by its core, it does not fall.
        let centre = state.points().iter().sum::<Vec3>() / state.points().len() as f32;
        assert!((centre.y - 2.0).abs() < 0.1, "held up: {centre}");
        // The bone jerks sideways: the flesh's middle goes at once, its
        // outside trails behind.
        state.advance(at(0.5), &[], 1.0 / 30.0);
        let trailing = state
            .points()
            .iter()
            .zip(&state.held)
            .filter(|(_, h)| !**h)
            .map(|(p, _)| p.x)
            .sum::<f32>()
            / state.held.iter().filter(|h| !**h).count() as f32;
        assert!(trailing < 0.45, "the outside lags: {trailing}");
        for _ in 0..60 {
            state.advance(at(0.5), &[], 1.0 / 30.0);
        }
        let settled = state.points().iter().sum::<Vec3>() / state.points().len() as f32;
        assert!((settled.x - 0.5).abs() < 0.05, "and catches up: {settled}");
    }

    #[test]
    fn a_body_the_module_cannot_draw_is_drawn_as_its_lattice() {
        let state = drop_onto_ground(SoftBody { cells: 2, ..SoftBody::default() }, 0.1);
        let fresh = SoftBodyState::new(SoftBody { cells: 2, ..SoftBody::default() }, None);
        let (_, indices) = state.mesh(Mat4::IDENTITY);
        assert_eq!(indices.len(), 36);
        // A 2×2×2 lattice's skin: 6 sides of 4 squares of 2 triangles.
        assert_eq!(fresh.triangles.len(), 6 * 4 * 2 * 3);
    }
}
