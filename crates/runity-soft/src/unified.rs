//! One solver's worth of contact between soft things of different kinds
//! (docs/simulation.md, item 12, "unified solver" as NVIDIA FleX): a rope
//! lies on a cloth, gravel heaps on a jelly, water pools in a hammock.
//! Each kind steps on its own, as before; then every particle of every
//! soft thing goes into one grid, and particles of different things that
//! overlap are pushed apart, the lighter moving more, the pinned not at
//! all. What they slid past each other is taken back as friction allows.
//!
//! Particles of one thing are left to its own solver: a cloth's to its
//! self-collision, grains' to theirs.

use glam::Vec3;

use crate::obstacle::{collide, Obstacle, Obstacles};
use crate::particles::Particles;

/// A soft thing's particles as the contact pass sees them: the
/// particles, how big each is as a ball, and how many kilograms each
/// weighs against what it meets.
///
/// Each kind keeps its masses in its own units — a grain weighs "1" to
/// its pile — so each says its weight here in kilograms. A particle of
/// something held together — a cloth, a rope, a jelly — meets a stone with
/// the whole thing's weight behind it: its constraints spread the push
/// over all of it, and one particle's own few grams would let every stone
/// through (it would step aside, the cloth's solver pull it back behind
/// the stone, and so on).
pub type Body<'a> = (&'a mut Particles, f32, f32);

pub trait Contacts {
    /// `None` before it has any particles.
    fn contact_body(&mut self) -> Option<Body<'_>>;

    /// Its particles' triangles, for a sheet — a cloth: what falls on it
    /// meets the sheet between its particles too, not only the particles.
    fn sheet(&self) -> Option<Vec<[u32; 3]>> {
        None
    }
}

/// The whole of a held-together thing's weight, kilograms: what each of
/// its particles meets the others with.
fn whole(p: &Particles) -> f32 {
    p.w.iter().filter(|w| **w > 0.0).map(|w| 1.0 / w).sum::<f32>().max(1e-3)
}

impl Contacts for crate::RopeState {
    fn contact_body(&mut self) -> Option<Body<'_>> {
        self.rod_mut().map(|r| {
            let radius = r.radius;
            let weight = whole(&r.particles);
            (&mut r.particles, radius, weight)
        })
    }
}

impl Contacts for crate::ClothState {
    fn sheet(&self) -> Option<Vec<[u32; 3]>> {
        let (across, down) = self.grid();
        let at = |i: usize, j: usize| (j * across + i) as u32;
        let mut out = Vec::new();
        for j in 0..down.saturating_sub(1) {
            for i in 0..across.saturating_sub(1) {
                out.push([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
                out.push([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
            }
        }
        Some(out)
    }

    fn contact_body(&mut self) -> Option<Body<'_>> {
        let radius = sheet_thickness(self);
        let p = self.particles_mut();
        let weight = whole(p);
        (!p.is_empty()).then_some((p, radius, weight))
    }
}

impl Contacts for crate::SoftBodyState {
    fn contact_body(&mut self) -> Option<Body<'_>> {
        let cells = self.body.cells.max(1) as f32;
        let p = self.particles_mut();
        if p.is_empty() {
            return None;
        }
        let (low, high) = p.x.iter().fold((Vec3::MAX, Vec3::MIN), |(l, h), x| (l.min(*x), h.max(*x)));
        let radius = ((high - low).max_element() / cells * 0.5).max(0.01);
        let weight = whole(p);
        Some((p, radius, weight))
    }
}

impl Contacts for crate::GrainsState {
    fn contact_body(&mut self) -> Option<Body<'_>> {
        let radius = self.grains.grain * 0.5;
        // Stone: 2600 kg a cubic metre.
        let weight = 2600.0 * std::f32::consts::FRAC_PI_6 * self.grains.grain.powi(3);
        let p = self.particles_mut();
        (!p.is_empty()).then_some((p, radius, weight))
    }
}

impl Contacts for crate::FluidState {
    fn contact_body(&mut self) -> Option<Body<'_>> {
        let radius = self.fluid.spacing * 0.5;
        // Water: a cube of its spacing.
        let weight = 1000.0 * self.fluid.spacing.powi(3);
        let p = self.particles_mut();
        (!p.is_empty()).then_some((p, radius, weight))
    }
}

/// How thick a cloth is to what meets it: its own thickness, but no
/// thinner than a sixth of the gap between its particles, so that what is
/// pushed off one triangle is not already through the next.
fn sheet_thickness(cloth: &crate::ClothState) -> f32 {
    let (across, down) = cloth.grid();
    let gap = (cloth.cloth.size[0] / (across.max(2) - 1) as f32).min(cloth.cloth.size[1] / (down.max(2) - 1) as f32);
    cloth.cloth.thickness.max(gap / 6.0)
}

/// Every cloth's triangles as they are now, as obstacles: what else is
/// soft meets them within its own substeps — gravel lies in a hammock
/// rather than sinking into it between one contact pass and the next. One
/// way only: the cloth feels them back in [`run_contacts`], a little thinner
/// than the pass's reach, so that what lies on a cloth still weighs on it.
pub fn sheet_obstacles(world: &hecs::World) -> Vec<Obstacle> {
    let mut out = Vec::new();
    for cloth in world.query::<&crate::ClothState>().iter() {
        let (Some(tris), p) = (cloth.sheet(), cloth.points()) else { continue };
        if p.is_empty() {
            continue;
        }
        // A little thinner than the contact pass takes it: what lies on
        // the cloth is still pressed into the pass's reach, and weighs.
        let thick = sheet_thickness(&cloth) * 0.7;
        let triangles = tris.iter().map(|t| t.map(|k| p[k as usize])).collect();
        out.push(Obstacle::Sheet(std::sync::Arc::new(crate::obstacle::Sheet::new(triangles, thick))));
    }
    out
}

/// One particle in the shared pass: which thing it is of, and which of
/// its particles.
#[derive(Clone, Copy)]
struct Entry {
    body: u32,
    index: u32,
}

/// Push apart the particles of different `bodies` that overlap; each body
/// its particles, their radius and their weight ([`Body`]). `friction` as [`crate::obstacle::collide`]'s.
/// The pushes it made, for a test to count.
/// `seconds` is how long since they were last apart: two that have passed
/// through each other in that time, by their velocities, are pushed back to
/// the side they came from, not on through.
pub fn contact(bodies: &mut [Body], friction: f32, seconds: f32) -> usize {
    let befores: Vec<Vec<Vec3>> = bodies.iter().map(|b| b.0.x.iter().zip(&b.0.v).map(|(x, v)| *x - *v * seconds).collect()).collect();
    contact_with_sheets(bodies, &[], friction, &befores)
}

/// [`contact`], with the triangles of those bodies that are sheets
/// ([`Contacts::sheet`]), by body (a body past the end of `sheets` is
/// none), and where each body's particles were when they were last apart,
/// by body.
pub fn contact_with_sheets(bodies: &mut [Body], sheets: &[Option<Vec<[u32; 3]>>], friction: f32, befores: &[Vec<Vec3>]) -> usize {
    // Things apart from everything else — a flag on every tenth crate —
    // cost nothing: only those whose boxes meet another's go in.
    let boxes = bounds(bodies, befores);
    let meets = |a: usize| (0..bodies.len()).any(|b| b != a && overlap(boxes[a], boxes[b]));
    if !(0..bodies.len()).any(meets) {
        return 0;
    }
    let touching: Vec<bool> = (0..bodies.len()).map(meets).collect();
    // A push can press a particle into a third: a few passes settle it.
    (0..PASSES).map(|_| contact_pass(bodies, sheets, friction, befores, &touching)).sum()
}

/// Each body's box, where it is and where it was, grown by its radius.
fn bounds(bodies: &[Body], befores: &[Vec<Vec3>]) -> Vec<(Vec3, Vec3)> {
    bodies
        .iter()
        .zip(befores)
        .map(|(b, before)| {
            let (low, high) = b.0.x.iter().chain(before).fold((Vec3::MAX, Vec3::MIN), |(l, h), x| (l.min(*x), h.max(*x)));
            (low - Vec3::splat(b.1), high + Vec3::splat(b.1))
        })
        .collect()
}

fn overlap(a: (Vec3, Vec3), b: (Vec3, Vec3)) -> bool {
    a.0.cmple(b.1).all() && b.0.cmple(a.1).all()
}

/// Passes over the contacts each step.
pub const PASSES: usize = 3;

/// Particles (or a sheet's triangles) a job finds the contacts of: a fixed
/// run, so what is summed and in what order does not hang on the cores.
const RUN: usize = 512;
/// A sheet's triangles a job: each looks through the grid round it, so
/// fewer than particles.
const SHEET_RUN: usize = 128;

fn contact_pass(bodies: &mut [Body], sheets: &[Option<Vec<[u32; 3]>>], friction: f32, befores: &[Vec<Vec3>], touching: &[bool]) -> usize {
    if bodies.len() < 2 {
        return 0;
    }
    let reach = bodies.iter().map(|b| b.1).fold(0.0f32, f32::max) * 2.0;
    if reach <= 0.0 {
        return 0;
    }
    let cell = |p: Vec3| {
        let c = (p / reach).floor();
        (c.x as i32, c.y as i32, c.z as i32)
    };
    let mut grid: runity_core::hash::FastMap<(i32, i32, i32), Vec<Entry>> = Default::default();
    for (b, (particles, _, _)) in bodies.iter().enumerate() {
        if !touching[b] {
            continue;
        }
        for (i, x) in particles.x.iter().enumerate() {
            grid.entry(cell(*x)).or_default().push(Entry { body: b as u32, index: i as u32 });
        }
    }
    // Every contact's push is found from where things are at the start of
    // the pass, and each particle takes the average of its pushes (Jacobi,
    // as FleX): a stone on six triangles of a cloth, among twenty of its
    // particles, moves it once, not twenty-six times.
    let mut pushes = Pushes::new(bodies);
    // Each particle's pushes, found on every core a fixed run of particles
    // at a time, then summed in the order one core would have: the same
    // sums, bit for bit, on any number of them (DNA, postulate 6).
    let particles: Vec<(usize, usize)> = (0..bodies.len())
        .filter(|&b| touching[b])
        .flat_map(|b| (0..bodies[b].0.len()).map(move |i| (b, i)))
        .collect();
    let runs: Vec<&[(usize, usize)]> = particles.chunks(RUN).collect();
    let bodies_seen: &[Body] = bodies;
    let found = runity_core::jobs::map(&runs, 1, |run| {
        let bodies = bodies_seen;
        let mut out: Vec<(u32, u32, Vec3)> = Vec::new();
        for &(b, i) in run.iter() {
            let x = bodies[b].0.x[i];
            let (cx, cy, cz) = cell(x);
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let Some(here) = grid.get(&(cx + dx, cy + dy, cz + dz)) else {
                            continue;
                        };
                        for e in here {
                            // Each pair once: the later body pushes.
                            if (e.body as usize) <= b {
                                continue;
                            }
                            let (o, j) = (e.body as usize, e.index as usize);
                            let (r1, r2) = (bodies[b].1, bodies[o].1);
                            let (xi, xj) = (bodies[b].0.x[i], bodies[o].0.x[j]);
                            let d = xi - xj;
                            let far = d.length();
                            if far >= r1 + r2 {
                                continue;
                            }
                            // Pinned stays pinned; else by weight.
                            let (wi, wj) = (
                                if bodies[b].0.w[i] > 0.0 { 1.0 / bodies[b].2 } else { 0.0 },
                                if bodies[o].0.w[j] > 0.0 { 1.0 / bodies[o].2 } else { 0.0 },
                            );
                            if wi + wj <= 0.0 {
                                continue;
                            }
                            // Two on one spot: apart upward.
                            let mut n = if far > 1e-7 { d / far } else { Vec3::Y };
                            let mut depth = r1 + r2 - far;
                            // Where they were a step ago: on the other side,
                            // they went through, and go back.
                            let before = befores[b][i] - befores[o][j];
                            let closed = d - before;
                            if before.dot(n) < 0.0 && closed.length() > far {
                                n = -n;
                                depth = r1 + r2 + far;
                            }
                            let (si, sj) = (wi / (wi + wj), wj / (wi + wj));
                            let mut di = n * depth * si;
                            let mut dj = -n * depth * sj;
                            // Friction: of how they slid past each other since
                            // the substep began, as much taken back as the push
                            // allows.
                            let moved = (xi - bodies[b].0.was[i]) - (xj - bodies[o].0.was[j]);
                            let along = moved - n * moved.dot(n);
                            let slide = along.length();
                            if slide > 1e-9 {
                                let hold = (friction * depth / slide).min(1.0);
                                di -= along * hold * si;
                                dj += along * hold * sj;
                            }
                            out.push((b as u32, i as u32, di));
                            out.push((o as u32, j as u32, dj));
                        }
                    }
                }
            }
        }
        out
    });
    for (body, index, push) in found.into_iter().flatten() {
        pushes.add(body as usize, index as usize, push);
    }
    // Every sheet's triangles, in fixed runs, all sheets in one go: an
    // awning is a few hundred triangles, and a market has a dozen.
    let most = bodies.iter().map(|b| b.1).fold(0.0f32, f32::max);
    let sheet_runs: Vec<(usize, &[[u32; 3]])> = sheets
        .iter()
        .enumerate()
        .filter(|(s, _)| touching.get(*s).copied().unwrap_or(false))
        .filter_map(|(s, tris)| tris.as_deref().map(|t| (s, t)))
        .flat_map(|(s, tris)| tris.chunks(SHEET_RUN).map(move |run| (s, run)))
        .collect();
    let bodies_seen: &[Body] = bodies;
    let found = runity_core::jobs::map(&sheet_runs, 1, |&(s, run)| {
        let mut out: Vec<(u32, u32, Vec3)> = Vec::new();
        sheet_run(bodies_seen, s, run, &grid, reach, befores, bodies_seen[s].1, most, &mut out);
        out
    });
    for (body, index, push) in found.into_iter().flatten() {
        pushes.add(body as usize, index as usize, push);
    }
    pushes.apply(bodies)
}

/// Each particle's pushes this pass, summed, and how many.
struct Pushes {
    sum: Vec<Vec<Vec3>>,
    count: Vec<Vec<u32>>,
}

impl Pushes {
    fn new(bodies: &[Body]) -> Self {
        Self {
            sum: bodies.iter().map(|b| vec![Vec3::ZERO; b.0.len()]).collect(),
            count: bodies.iter().map(|b| vec![0; b.0.len()]).collect(),
        }
    }

    fn add(&mut self, body: usize, i: usize, push: Vec3) {
        if push != Vec3::ZERO {
            self.sum[body][i] += push;
            self.count[body][i] += 1;
        }
    }

    /// Each particle moved by the average of its pushes; how many moved.
    fn apply(self, bodies: &mut [Body]) -> usize {
        let mut moved = 0;
        for (b, (sum, count)) in self.sum.into_iter().zip(self.count).enumerate() {
            for (i, (push, n)) in sum.into_iter().zip(count).enumerate() {
                if n > 0 {
                    bodies[b].0.x[i] += push / n as f32;
                    moved += 1;
                }
            }
        }
        moved
    }
}

/// Every particle of the other bodies against sheet `s`'s triangles: one
/// near a triangle, or gone through it since a step ago, is put back on
/// the side it came from, a sheet's thickness off; the triangle's corners
/// take their share of the push by how near each is.
#[allow(clippy::too_many_arguments)]
fn sheet_run(
    bodies: &[Body],
    s: usize,
    tris: &[[u32; 3]],
    grid: &runity_core::hash::FastMap<(i32, i32, i32), Vec<Entry>>,
    reach: f32,
    befores: &[Vec<Vec3>],
    thick: f32,
    most: f32,
    out: &mut Vec<(u32, u32, Vec3)>,
) {
    for t in tris {
        let [a, b, c] = t.map(|k| k as usize);
        if a.max(b).max(c) >= bodies[s].0.len() {
            continue;
        }
        let corners = [a, b, c].map(|k| bodies[s].0.x[k]);
        let before = [a, b, c].map(|k| befores[s][k]);
        let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
        let area = normal.length();
        if area < 1e-9 {
            continue;
        }
        let n = normal / area;
        let n_before = (before[1] - before[0]).cross(before[2] - before[0]).normalize_or(n);
        let pad = Vec3::splat(most + thick);
        let low = corners[0].min(corners[1]).min(corners[2]) - pad;
        let high = corners[0].max(corners[1]).max(corners[2]) + pad;
        let (from, to) = ((low / reach).floor(), (high / reach).floor());
        // A torn or thrown triangle, larger than any cloth's: left alone.
        if !(to - from).is_finite() || (to - from).max_element() > 64.0 {
            continue;
        }
        let wk = [a, b, c].map(|k| if bodies[s].0.w[k] > 0.0 { 1.0 / bodies[s].2 } else { 0.0 });
        for z in from.z as i32..=to.z as i32 {
            for y in from.y as i32..=to.y as i32 {
                for x in from.x as i32..=to.x as i32 {
                    let Some(here) = grid.get(&(x, y, z)) else { continue };
                    for e in here {
                        let (o, j) = (e.body as usize, e.index as usize);
                        if o == s {
                            continue;
                        }
                        let r = bodies[o].1;
                        let p = bodies[o].0.x[j];
                        let p_before = befores[o][j];
                        // Over the triangle: where it lies in it.
                        let Some(bary) = barycentric(p - n * (p - corners[0]).dot(n), corners) else { continue };
                        let now = (p - corners[0]).dot(n);
                        let was = (p_before - before[0]).dot(n_before);
                        let band = r + thick;
                        // Through it only if near enough to have got there
                        // this step: the plane runs on past the triangle's
                        // prism, and what is far off it merely crossed it.
                        let moved = (now - was).abs();
                        if now.abs() > band + moved || was.abs() > band + moved {
                            continue;
                        }
                        let side = if now * was < 0.0 {
                            // Went through: back to where it came from.
                            was.signum()
                        } else if now.abs() < band {
                            now.signum()
                        } else {
                            continue;
                        };
                        let push = side * band - now;
                        let wp = if bodies[o].0.w[j] > 0.0 { 1.0 / bodies[o].2 } else { 0.0 };
                        let wt = bary[0] * bary[0] * wk[0] + bary[1] * bary[1] * wk[1] + bary[2] * bary[2] * wk[2];
                        if wp + wt <= 0.0 {
                            continue;
                        }
                        let lambda = push / (wp + wt);
                        out.push((o as u32, j as u32, n * (lambda * wp)));
                        for (m, k) in [a, b, c].into_iter().enumerate() {
                            out.push((s as u32, k as u32, -n * (lambda * bary[m] * wk[m])));
                        }
                    }
                }
            }
        }
    }
}

/// Where `p`, in the plane of triangle `t`, lies in it: its weights on the
/// corners, or `None` outside it.
fn barycentric(p: Vec3, t: [Vec3; 3]) -> Option<[f32; 3]> {
    let (v0, v1, v2) = (t[1] - t[0], t[2] - t[0], p - t[0]);
    let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-12 {
        return None;
    }
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    let u = 1.0 - v - w;
    let edge = -0.02;
    (u >= edge && v >= edge && w >= edge).then_some([u, v, w])
}

/// Where every soft thing's particles are, by entity: taken before a
/// step, for [`run_contacts`] to tell what went through what during it.
pub type Starts = std::collections::HashMap<hecs::Entity, Vec<Vec3>>;

/// [`Starts`] now.
pub fn frame_starts(world: &mut hecs::World) -> Starts {
    let mut out = Starts::new();
    fn take<T: Contacts + hecs::Component>(world: &mut hecs::World, out: &mut Starts) {
        for (e, s) in world.query_mut::<(hecs::Entity, &mut T)>() {
            if let Some(b) = s.contact_body() {
                out.insert(e, b.0.x.clone());
            }
        }
    }
    take::<crate::RopeState>(world, &mut out);
    take::<crate::ClothState>(world, &mut out);
    take::<crate::SoftBodyState>(world, &mut out);
    take::<crate::GrainsState>(world, &mut out);
    take::<crate::FluidState>(world, &mut out);
    out
}

/// Every soft thing in the world against every other: after each kind
/// has stepped, from where they were at the step's start (`starts`, see
/// [`frame_starts`]). Their velocities take the pushes, so what was pushed
/// out does not fall straight back in; and what was pushed is put back out
/// of `obstacles`, so a push never drives water through a floor.
pub fn run_contacts(world: &mut hecs::World, starts: &Starts, obstacles: &Obstacles) {
    let mut ropes = world.query::<(hecs::Entity, &mut crate::RopeState)>();
    let mut cloths = world.query::<(hecs::Entity, &mut crate::ClothState)>();
    let mut soft = world.query::<(hecs::Entity, &mut crate::SoftBodyState)>();
    let mut grains = world.query::<(hecs::Entity, &mut crate::GrainsState)>();
    let mut fluids = world.query::<(hecs::Entity, &mut crate::FluidState)>();
    let mut bodies: Vec<Body> = Vec::new();
    let mut befores: Vec<Vec<Vec3>> = Vec::new();
    let mut sheets: Vec<Option<Vec<[u32; 3]>>> = Vec::new();
    #[allow(clippy::too_many_arguments)]
    fn add<'a>(
        e: hecs::Entity,
        sheet: Option<Vec<[u32; 3]>>,
        body: Option<Body<'a>>,
        bodies: &mut Vec<Body<'a>>,
        befores: &mut Vec<Vec<Vec3>>,
        sheets: &mut Vec<Option<Vec<[u32; 3]>>>,
        starts: &Starts,
    ) {
        let Some(body) = body else { return };
        // Where it was; new since, where it is.
        let before = starts.get(&e).filter(|s| s.len() == body.0.len()).cloned().unwrap_or_else(|| body.0.x.clone());
        befores.push(before);
        sheets.push(sheet);
        bodies.push(body);
    }
    for (e, s) in ropes.iter() {
        add(e, None, s.contact_body(), &mut bodies, &mut befores, &mut sheets, starts);
    }
    for (e, s) in cloths.iter() {
        let sheet = s.sheet();
        add(e, sheet, s.contact_body(), &mut bodies, &mut befores, &mut sheets, starts);
    }
    for (e, s) in soft.iter() {
        add(e, None, s.contact_body(), &mut bodies, &mut befores, &mut sheets, starts);
    }
    for (e, s) in grains.iter() {
        add(e, None, s.contact_body(), &mut bodies, &mut befores, &mut sheets, starts);
    }
    for (e, s) in fluids.iter() {
        add(e, None, s.contact_body(), &mut bodies, &mut befores, &mut sheets, starts);
    }
    if bodies.len() < 2 {
        return;
    }
    let before: Vec<Vec<Vec3>> = bodies.iter().map(|b| b.0.x.clone()).collect();
    if contact_with_sheets(&mut bodies, &sheets, 0.4, &befores) == 0 {
        return;
    }
    let mut near = Vec::new();
    for ((particles, radius, _), before) in bodies.iter_mut().zip(before) {
        let moved: Vec<usize> = (0..before.len()).filter(|i| particles.x[*i] != before[*i]).collect();
        if moved.is_empty() {
            continue;
        }
        let (low, high) = moved.iter().fold((Vec3::MAX, Vec3::MIN), |(l, h), i| (l.min(particles.x[*i]), h.max(particles.x[*i])));
        obstacles.near(low - Vec3::splat(*radius), high + Vec3::splat(*radius), &mut near);
        for i in moved {
            let was = before[i];
            collide(&mut particles.x[i], was, *radius, 0.4, &near);
            let pushed = particles.x[i] - was;
            // Only the push's inward-going speed is taken: it was
            // stopped, not thrown.
            let n = pushed.normalize_or_zero();
            let into = particles.v[i].dot(n);
            if into < 0.0 {
                particles.v[i] -= n * into;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heavy_and_a_light_body_overlapping_are_pushed_apart_the_light_one_more() {
        let mut heavy = Particles::new(vec![Vec3::ZERO], 10.0);
        let mut light = Particles::new(vec![Vec3::new(0.06, 0.0, 0.0)], 1.0);
        assert!(contact(&mut [(&mut heavy, 0.05, 10.0), (&mut light, 0.05, 1.0)], 0.0, 0.0) >= 1);
        assert!((heavy.x[0].distance(light.x[0]) - 0.1).abs() < 1e-4);
        assert!(light.x[0].x - 0.06 > 5.0 * -heavy.x[0].x, "{} {}", heavy.x[0], light.x[0]);
        // A pinned one is not moved at all.
        let mut pinned = Particles::new(vec![Vec3::new(0.0, 0.05, 0.0)], 1.0);
        pinned.w[0] = 0.0;
        contact(&mut [(&mut pinned, 0.05, 1.0), (&mut light, 0.05, 1.0)], 0.0, 0.0);
        let mut drop = Particles::new(vec![Vec3::new(0.0, 0.1, 0.0)], 1.0);
        contact(&mut [(&mut pinned, 0.05, 1.0), (&mut drop, 0.05, 1.0)], 0.0, 0.0);
        assert_eq!(pinned.x[0], Vec3::new(0.0, 0.05, 0.0));
        assert!((drop.x[0].y - 0.15).abs() < 1e-4, "{}", drop.x[0]);
        // A fast drop that went through a pinned one in the last step goes
        // back above it.
        let mut fast = Particles::new(vec![Vec3::new(0.0, 0.0, 0.0)], 1.0);
        fast.v[0] = Vec3::new(0.0, -6.0, 0.0);
        contact(&mut [(&mut pinned, 0.05, 1.0), (&mut fast, 0.05, 1.0)], 0.0, 1.0 / 60.0);
        assert!((fast.x[0].y - 0.15).abs() < 1e-4, "{}", fast.x[0]);
    }

    #[test]
    fn gravel_poured_into_a_hammock_lies_in_it() {
        use glam::Mat4;
        use runity_core::world::WorldTransform;
        let mut world = hecs::World::new();
        let cloth = crate::Cloth { size: [1.8, 1.2], cells: [28, 18], pinned: crate::Pinned::Corners, catch: 0.2, ..Default::default() };
        world.spawn((crate::ClothState::new(cloth), WorldTransform(Mat4::from_translation(Vec3::new(0.0, 1.3, 0.0)))));
        let gravel = crate::Grains { size: Vec3::new(0.35, 0.4, 0.25), grain: 0.08, friction: 0.7 };
        world.spawn((crate::GrainsState::new(gravel), WorldTransform(Mat4::from_translation(Vec3::new(0.0, 2.2, 0.0)))));
        let obstacles = Obstacles::new(vec![crate::Obstacle::ground(0.0)]);
        for _ in 0..120 {
            let starts = frame_starts(&mut world);
            crate::run_cloth(&mut world, 1.0 / 60.0, &obstacles);
            let mut with_sheets = vec![crate::Obstacle::ground(0.0)];
            with_sheets.extend(sheet_obstacles(&world));
            crate::run_grains(&mut world, 1.0 / 60.0, &Obstacles::new(with_sheets));
            run_contacts(&mut world, &starts, &obstacles);
        }
        let grains: Vec<Vec3> = world.query::<&crate::GrainsState>().iter().flat_map(|s| s.points().to_vec()).collect();
        let through = grains.iter().filter(|p| p.y < 0.9 && p.x.abs() < 0.8 && p.z.abs() < 0.5).count();
        assert!(through * 10 <= grains.len(), "{through} of {} fell through the hammock", grains.len());
        // Nothing thrown: every grain is near where it was poured.
        assert!(grains.iter().all(|p| p.x.abs() < 1.5 && p.z.abs() < 1.5 && p.y < 2.5), "thrown");
        // And the hammock sags under them.
        let lowest = world.query::<&crate::ClothState>().iter().map(|s| s.points().iter().map(|p| p.y).fold(9.0, f32::min)).next().unwrap();
        assert!(lowest < 1.25, "sags: {lowest}");
    }
}

