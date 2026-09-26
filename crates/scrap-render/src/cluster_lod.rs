//! Clusters at every level of detail, as Nanite has them: a dense mesh's
//! clusters grouped by fours, each group simplified to half its triangles
//! with its edge held where it is, and cut into clusters again — level on
//! level, up to a handful at the top. Each cluster knows how far its
//! surface may stand from the mesh's (its error, over a sphere) and how far
//! its parents' may: the coarser clusters its group was simplified into.
//!
//! A frame draws, of every instance, the clusters whose own error is under
//! a pixel on the screen and whose parents' is not ([`crate::cluster`]'s
//! cull pass). The errors only grow upward and each sphere holds its
//! children's, so exactly one level of every part of the surface passes,
//! and a group's edge, held through its simplification, is the same line
//! in the clusters on both sides of it: no cracks, whatever the mix of
//! levels. Under a pixel of error, a far mountain range of half a million
//! triangles is drawn with the few thousand that make up what shows.
//!
//! The simplifier is quadric error (Garland and Heckbert) over half-edge
//! collapses, by where vertices are, not which: a vertex that is the same
//! place as another (a seam of the texture, a hard edge's split normals)
//! is held, as is every place a group shares with another. A mesh cut into
//! facets all over (a low-poly rock, every corner its own normal) cannot
//! be simplified that way and keeps its one level — as it should: its
//! look is its facets.
//!
//! Built when the mesh is uploaded, on every core, pure Rust — the browser
//! builds it too. `SCRAP_LOD_WHY=1` prints each level as it is built.

use glam::Vec3;

use crate::asset::Vertex;
use crate::cluster::ClusterRaw;

/// Clusters a group is made of, at most.
const GROUP: usize = 4;
/// A group simplified by less than this share is left as it is: its
/// clusters are the top of their part of the mesh.
const LEAST_CUT: f32 = 0.15;
/// Levels at most.
const LEVELS: usize = 24;
/// An error that is never under the threshold: the parent of a cluster at
/// the top.
pub(crate) const NEVER: f32 = 1.0e30;

/// Every vertex's place: vertices at the same position share one; and
/// whether a place holds vertices that differ (a seam), which is held.
struct Places {
    of: Vec<u32>,
    /// The vertex a place's corners are drawn with, where it is not a seam.
    vertex: Vec<u32>,
    seam: Vec<bool>,
}

impl Places {
    fn new(vertices: &[Vertex]) -> Self {
        let mut ids: std::collections::HashMap<[u32; 3], u32> = std::collections::HashMap::with_capacity(vertices.len());
        let mut of = Vec::with_capacity(vertices.len());
        let mut vertex: Vec<u32> = Vec::new();
        let mut seam: Vec<bool> = Vec::new();
        for (i, v) in vertices.iter().enumerate() {
            let key = v.position.map(f32::to_bits);
            let next = vertex.len() as u32;
            let place = *ids.entry(key).or_insert(next);
            if place == next {
                vertex.push(i as u32);
                seam.push(false);
            } else {
                let first = &vertices[vertex[place as usize] as usize];
                let same = first.uv == v.uv
                    && Vec3::from(first.normal).dot(Vec3::from(v.normal)) > 0.999;
                if !same {
                    seam[place as usize] = true;
                }
            }
            of.push(place);
        }
        Self { of, vertex, seam }
    }
}

/// The whole of it: `sorted` and `clusters` are level 0 (from
/// [`crate::cluster::build`]), their indices first in the mesh's buffer.
/// Returns the indices of every coarser level, to go after them, and
/// every cluster of every level, level 0's first — each with its errors
/// and spheres filled in.
pub(crate) fn levels(vertices: &[Vertex], sorted: &[u32], level0: &[ClusterRaw]) -> (Vec<u32>, Vec<ClusterRaw>) {
    let places = Places::new(vertices);
    let mut all: Vec<ClusterRaw> = Vec::with_capacity(level0.len() * 2);
    let mut extra: Vec<u32> = Vec::new();
    let base = sorted.len() as u32;
    // Level 0: no error, each its own sphere; its parents to be found.
    let mut level: Vec<(usize, Vec<u32>)> = level0
        .iter()
        .map(|c| {
            let mut raw = *c;
            raw.lod = [c.sphere, [c.sphere[0], c.sphere[1], c.sphere[2], c.sphere[3]]];
            raw.error = [0.0, NEVER];
            all.push(raw);
            let triangles = sorted[c.first as usize..(c.first + c.count * 3) as usize].to_vec();
            (all.len() - 1, triangles)
        })
        .collect();
    for _ in 0..LEVELS {
        if level.len() <= 1 {
            break;
        }
        let t0 = std::time::Instant::now();
        let groups = group(&level, &places, &all);
        let t_group = t0.elapsed();
        // Which group each place is in, to hold what groups share.
        let mut owner: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        let mut shared: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for (g, members) in groups.iter().enumerate() {
            for &m in members {
                for &v in &level[m].1 {
                    let p = places.of[v as usize];
                    match owner.get(&p) {
                        Some(&o) if o != g as u32 => {
                            shared.insert(p);
                        }
                        Some(_) => {}
                        None => {
                            owner.insert(p, g as u32);
                        }
                    }
                }
            }
        }
        let t_shared = t0.elapsed();
        let made = scrap_core::jobs::map(&groups, 1, |members| {
            let triangles: Vec<u32> = members.iter().flat_map(|&m| level[m].1.iter().copied()).collect();
            let target = triangles.len() / 3 / 2;
            let (simplified, error) = simplify(vertices, &places, &shared, &triangles, target);
            if simplified.len() as f32 > triangles.len() as f32 * (1.0 - LEAST_CUT) {
                return None;
            }
            let (split, clusters) = crate::cluster::split(vertices, &simplified, 0);
            Some((simplified.len(), error, split, clusters))
        });
        if std::env::var_os("SCRAP_LOD_WHY").is_some() {
            let ok = made.iter().filter(|m| m.is_some()).count();
            let tris_in: usize = level.iter().map(|l| l.1.len() / 3).sum();
            let tris_out: usize = made.iter().flatten().map(|m| m.0 / 3).sum();
            eprintln!(
                "level: {} clusters, {tris_in} triangles, {} groups, {ok} simplified, {tris_out} triangles out; grouped {:.0} ms, shared {:.0}, simplified {:.0}",
                level.len(),
                groups.len(),
                t_group.as_secs_f32() * 1e3,
                (t_shared - t_group).as_secs_f32() * 1e3,
                (t0.elapsed() - t_shared).as_secs_f32() * 1e3
            );
        }
        let mut next: Vec<(usize, Vec<u32>)> = Vec::new();
        for (members, made) in groups.iter().zip(made) {
            let Some((_, error, split, clusters)) = made else {
                // Held as it is: its clusters are the top here.
                continue;
            };
            // The group's error is its worst child's and what it lost; its
            // sphere holds its children's.
            let child_error = members.iter().map(|&m| all[level[m].0].error[0]).fold(0.0f32, f32::max);
            let group_error = child_error + error;
            let sphere = enclose(members.iter().map(|&m| all[level[m].0].lod[0]));
            for &m in members {
                let c = &mut all[level[m].0];
                c.lod[1] = sphere;
                c.error[1] = group_error;
            }
            let offset = base + extra.len() as u32;
            extra.extend_from_slice(&split);
            for mut c in clusters {
                let triangles = split[c.first as usize..(c.first + c.count * 3) as usize].to_vec();
                c.first += offset;
                c.lod = [sphere, [0.0, 0.0, 0.0, 0.0]];
                c.error = [group_error, NEVER];
                all.push(c);
                next.push((all.len() - 1, triangles));
            }
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    (extra, all)
}

/// A sphere holding all these.
fn enclose(spheres: impl Iterator<Item = [f32; 4]>) -> [f32; 4] {
    let mut out: Option<(Vec3, f32)> = None;
    for s in spheres {
        let (c, r) = (Vec3::new(s[0], s[1], s[2]), s[3]);
        out = Some(match out {
            None => (c, r),
            Some((oc, or)) => {
                let d = c.distance(oc);
                if d + r <= or {
                    (oc, or)
                } else if d + or <= r {
                    (c, r)
                } else {
                    let radius = (d + r + or) * 0.5;
                    let centre = oc + (c - oc) * ((radius - or) / d.max(1e-12));
                    (centre, radius)
                }
            }
        });
    }
    let (c, r) = out.unwrap_or((Vec3::ZERO, 0.0));
    // A hair over, for the rounding of the next one to hold it.
    [c.x, c.y, c.z, r * 1.0001 + 1e-6]
}

/// The level's clusters in groups that touch, of about four clusters'
/// triangles: from each not yet taken, in the order of their centres along
/// a curve, the one that shares the most places with the group so far,
/// until it has enough. Then a group left small — the leftovers of a
/// level's cutting, a few triangles each — joins the neighbour it shares
/// most with: alone it could not be simplified, and what it holds of its
/// neighbours' edges would stop theirs.
fn group(level: &[(usize, Vec<u32>)], places: &Places, all: &[ClusterRaw]) -> Vec<Vec<usize>> {
    let budget = GROUP * crate::cluster::TRIANGLES as usize;
    let mut by_place: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    let mut touched: Vec<Vec<u32>> = Vec::with_capacity(level.len());
    for (i, (_, triangles)) in level.iter().enumerate() {
        let mut mine: Vec<u32> = triangles.iter().map(|&v| places.of[v as usize]).collect();
        mine.sort_unstable();
        mine.dedup();
        for &p in &mine {
            by_place.entry(p).or_default().push(i as u32);
        }
        touched.push(mine);
    }
    let size = |i: usize| level[i].1.len() / 3;
    // Along a curve through the centres, so groups grow from one end.
    let centres: Vec<Vec3> = level.iter().map(|(c, _)| Vec3::new(all[*c].sphere[0], all[*c].sphere[1], all[*c].sphere[2])).collect();
    let (lo, hi) = centres.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), c| (a.min(*c), b.max(*c)));
    let span = (hi - lo).max(Vec3::splat(1e-6));
    let mut order: Vec<(u32, usize)> = centres.iter().enumerate().map(|(i, c)| (crate::cluster::morton((*c - lo) / span), i)).collect();
    order.sort_unstable();
    let neighbours = |members: &[usize], open: &dyn Fn(usize) -> bool| -> std::collections::HashMap<usize, u32> {
        let mut shares: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
        for &m in members {
            for p in &touched[m] {
                for &n in &by_place[p] {
                    if open(n as usize) {
                        *shares.entry(n as usize).or_default() += 1;
                    }
                }
            }
        }
        shares
    };
    let mut in_group = vec![usize::MAX; level.len()];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for &(_, seed) in &order {
        if in_group[seed] != usize::MAX {
            continue;
        }
        let g = groups.len();
        in_group[seed] = g;
        let mut members = vec![seed];
        let mut triangles = size(seed);
        let mut cursor = 0usize;
        while triangles < budget * 3 / 4 {
            let shares = neighbours(&members, &|n| in_group[n] == usize::MAX);
            let best = match shares.iter().max_by_key(|(n, s)| (**s, std::cmp::Reverse(**n))) {
                Some((&best, _)) => best,
                None => {
                    // Nothing touches it (grass: blades apart): the next
                    // along the curve. Its edges are held all the same.
                    while cursor < order.len() && in_group[order[cursor].1] != usize::MAX {
                        cursor += 1;
                    }
                    let Some(&(_, next)) = order.get(cursor) else {
                        break;
                    };
                    next
                }
            };
            in_group[best] = g;
            members.push(best);
            triangles += size(best);
        }
        groups.push(members);
    }
    // Small groups into their best neighbour.
    for g in 0..groups.len() {
        let triangles: usize = groups[g].iter().map(|&m| size(m)).sum();
        if triangles >= budget / 4 || groups[g].is_empty() {
            continue;
        }
        let members = groups[g].clone();
        let shares = neighbours(&members, &|n| in_group[n] != g);
        let mut into: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
        for (n, s) in shares {
            *into.entry(in_group[n]).or_default() += s;
        }
        let Some((&target, _)) = into.iter().max_by_key(|(t, s)| (**s, std::cmp::Reverse(**t))) else {
            continue;
        };
        for &m in &members {
            in_group[m] = target;
        }
        groups[g].clear();
        groups[target].extend(members);
    }
    groups.retain(|g| !g.is_empty());
    groups
}

/// A plane's quadric, weighted: `[a² ab ac ad b² bc bd c² cd d²]` and the
/// weight, for the squared distance to it.
#[derive(Clone, Copy, Default)]
struct Quadric {
    q: [f64; 10],
    w: f64,
}

impl Quadric {
    fn plane(n: Vec3, d: f32, w: f64) -> Self {
        let (a, b, c, d) = (n.x as f64, n.y as f64, n.z as f64, d as f64);
        Self {
            q: [a * a * w, a * b * w, a * c * w, a * d * w, b * b * w, b * c * w, b * d * w, c * c * w, c * d * w, d * d * w],
            w,
        }
    }

    fn add(&mut self, o: &Quadric) {
        for i in 0..10 {
            self.q[i] += o.q[i];
        }
        self.w += o.w;
    }

    /// The mean squared distance of `p` to the planes.
    fn error(&self, p: Vec3) -> f64 {
        let (x, y, z) = (p.x as f64, p.y as f64, p.z as f64);
        let q = &self.q;
        let e = q[0] * x * x + 2.0 * q[1] * x * y + 2.0 * q[2] * x * z + 2.0 * q[3] * x
            + q[4] * y * y + 2.0 * q[5] * y * z + 2.0 * q[6] * y
            + q[7] * z * z + 2.0 * q[8] * z
            + q[9];
        e.max(0.0) / self.w.max(1e-12)
    }
}

/// Simplify `triangles` (vertex indices) toward `target` triangles by
/// half-edge collapses, cheapest first by quadric error: a place moves onto
/// a neighbour's, never one that is held — a seam, or shared with another
/// group (`shared`), or on the edge of this one. Returns what is left, its
/// corners the places' own vertices, and the error: the distance from the
/// surface the worst collapse left, in the mesh's units.
fn simplify(
    vertices: &[Vertex],
    places: &Places,
    shared: &std::collections::HashSet<u32>,
    triangles: &[u32],
    target: usize,
) -> (Vec<u32>, f32) {
    // The group's places, numbered from 0.
    let mut local: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut place_of_local: Vec<u32> = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::with_capacity(triangles.len() / 3);
    for t in triangles.chunks_exact(3) {
        let mut tri = [0u32; 3];
        for (k, &v) in t.iter().enumerate() {
            let p = places.of[v as usize];
            let next = place_of_local.len() as u32;
            tri[k] = *local.entry(p).or_insert_with(|| {
                place_of_local.push(p);
                next
            });
        }
        tris.push(tri);
    }
    let n = place_of_local.len();
    let pos: Vec<Vec3> = place_of_local.iter().map(|&p| Vec3::from(vertices[places.vertex[p as usize] as usize].position)).collect();
    // Held: seams, what other groups share, and the group's own edge.
    let mut held: Vec<bool> = place_of_local.iter().map(|&p| places.seam[p as usize] || shared.contains(&p)).collect();
    let mut edges: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for t in &tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    for (&(a, b), &uses) in &edges {
        if uses != 2 {
            held[a as usize] = true;
            held[b as usize] = true;
        }
    }
    // Each place's quadric: the planes of its triangles, by their areas.
    let mut quadric = vec![Quadric::default(); n];
    for t in &tris {
        let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
        let cross = (b - a).cross(c - a);
        let area = cross.length() * 0.5;
        if area <= 1e-20 {
            continue;
        }
        let normal = cross / (area * 2.0);
        let plane = Quadric::plane(normal, -normal.dot(a), area as f64);
        for &v in t {
            quadric[v as usize].add(&plane);
        }
    }
    let mut alive = vec![true; tris.len()];
    let mut live = tris.len();
    let mut of_place: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (i, t) in tris.iter().enumerate() {
        for &v in t {
            of_place[v as usize].push(i as u32);
        }
    }
    let mut worst = 0.0f64;
    let mut pairs: Vec<(u32, u32)> = Vec::with_capacity(tris.len() * 3);
    let mut candidates: Vec<(f64, u32, u32)> = Vec::with_capacity(tris.len() * 3);
    let mut touched = vec![false; n];
    let (mut ring_a, mut ring_b) = (Vec::new(), Vec::new());
    for _pass in 0..16 {
        if live <= target {
            break;
        }
        // Every edge still there, each way it may collapse, cheapest first.
        pairs.clear();
        for (i, t) in tris.iter().enumerate() {
            if alive[i] {
                for k in 0..3 {
                    let (a, b) = (t[k], t[(k + 1) % 3]);
                    pairs.push((a.min(b), a.max(b)));
                }
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        candidates.clear();
        for &(a, b) in &pairs {
            for (from, to) in [(a, b), (b, a)] {
                if held[from as usize] {
                    continue;
                }
                let mut q = quadric[from as usize];
                q.add(&quadric[to as usize]);
                candidates.push((q.error(pos[to as usize]), from, to));
            }
        }
        if candidates.is_empty() {
            break;
        }
        candidates.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        touched.iter_mut().for_each(|t| *t = false);
        let mut collapsed = 0;
        for &(cost, from, to) in &candidates {
            if live <= target {
                break;
            }
            let (f, t) = (from as usize, to as usize);
            if touched[f] || touched[t] {
                continue;
            }
            ring(&tris, &alive, &of_place[f], from, &mut ring_a);
            ring(&tris, &alive, &of_place[t], to, &mut ring_b);
            if !collapsible(&tris, &alive, &of_place[f], &pos, from, to, &ring_a, &ring_b) {
                continue;
            }
            // Collapse: `from`'s triangles now use `to`; those with both go.
            let mine = std::mem::take(&mut of_place[f]);
            for &ti in &mine {
                let ti = ti as usize;
                if !alive[ti] {
                    continue;
                }
                if tris[ti].contains(&to) {
                    alive[ti] = false;
                    live -= 1;
                } else {
                    for v in tris[ti].iter_mut() {
                        if *v == from {
                            *v = to;
                        }
                    }
                    of_place[t].push(ti as u32);
                }
            }
            of_place[t].retain(|&ti| alive[ti as usize]);
            let qf = quadric[f];
            quadric[t].add(&qf);
            worst = worst.max(cost);
            // The neighbourhood changed: nothing more around it this pass.
            touched[f] = true;
            touched[t] = true;
            for &v in &ring_a {
                touched[v as usize] = true;
            }
            for &v in &ring_b {
                touched[v as usize] = true;
            }
            collapsed += 1;
        }
        if collapsed == 0 {
            break;
        }
    }
    let out: Vec<u32> = tris
        .iter()
        .zip(&alive)
        .filter(|(_, a)| **a)
        .flat_map(|(t, _)| t.iter().map(|&v| places.vertex[place_of_local[v as usize] as usize]))
        .collect();
    (out, worst.sqrt() as f32)
}

/// The places round `p` (its triangles' other corners), sorted, once each.
fn ring(tris: &[[u32; 3]], alive: &[bool], mine: &[u32], p: u32, out: &mut Vec<u32>) {
    out.clear();
    for &t in mine {
        if alive[t as usize] {
            out.extend(tris[t as usize].iter().copied().filter(|&v| v != p));
        }
    }
    out.sort_unstable();
    out.dedup();
}

/// Whether `from` may move onto `to`: they share at most the two places
/// across their edge (else the surface would pinch), and no triangle of
/// `from`'s that stays turns over or collapses to a sliver.
#[allow(clippy::too_many_arguments)]
fn collapsible(tris: &[[u32; 3]], alive: &[bool], mine: &[u32], pos: &[Vec3], from: u32, to: u32, ring_from: &[u32], ring_to: &[u32]) -> bool {
    // Places round both, by a merge of the sorted rings.
    let (mut i, mut j, mut common) = (0, 0, 0);
    while i < ring_from.len() && j < ring_to.len() {
        match ring_from[i].cmp(&ring_to[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                i += 1;
                j += 1;
            }
        }
    }
    if common > 2 {
        return false;
    }
    let target = pos[to as usize];
    for &ti in mine {
        let t = tris[ti as usize];
        if !alive[ti as usize] || t.contains(&to) {
            continue;
        }
        let p = |v: u32| pos[v as usize];
        let before = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
        let q = |v: u32| if v == from { target } else { p(v) };
        let after = (q(t[1]) - q(t[0])).cross(q(t[2]) - q(t[0]));
        if after.length_squared() <= before.length_squared() * 1e-6 {
            return false;
        }
        if before.normalize_or_zero().dot(after.normalize_or_zero()) < 0.2 {
            return false;
        }
    }
    true
}

/// How many pixels an error covers from `eye`: `error` over `sphere`,
/// placed by `model` (its scale), at `pixels_per_radian` — a frame's
/// height over twice the tangent of half its field of view.
#[cfg(test)]
pub(crate) fn projected(error: f32, sphere: [f32; 4], model: glam::Mat4, eye: Vec3, pixels_per_radian: f32, near: f32) -> f32 {
    let scale = model.x_axis.truncate().length().max(model.y_axis.truncate().length()).max(model.z_axis.truncate().length());
    let centre = model.transform_point3(Vec3::new(sphere[0], sphere[1], sphere[2]));
    let distance = (centre.distance(eye) - sphere[3] * scale).max(near);
    error * scale * pixels_per_radian / distance
}

/// Whether a cluster is drawn from `eye`: its own error under `threshold`
/// pixels, its parents' not.
#[cfg(test)]
pub(crate) fn chosen(c: &ClusterRaw, model: glam::Mat4, eye: Vec3, pixels_per_radian: f32, near: f32, threshold: f32) -> bool {
    let own = projected(c.error[0], c.lod[0], model, eye, pixels_per_radian, near);
    let parent = if c.error[1] >= NEVER { f32::INFINITY } else { projected(c.error[1], c.lod[1], model, eye, pixels_per_radian, near) };
    own <= threshold && parent > threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sphere of `rings` rings, smooth: one vertex a place, but a seam of
    /// its texture down one side.
    fn ball(rings: u32) -> (Vec<Vertex>, Vec<u32>) {
        let mut vertices = Vec::new();
        let segments = rings * 2;
        for r in 0..=rings {
            let theta = std::f32::consts::PI * r as f32 / rings as f32;
            for s in 0..=segments {
                let phi = std::f32::consts::TAU * s as f32 / segments as f32;
                // The poles and the seam: one position, many vertices.
                let (st, ct) = theta.sin_cos();
                let p = if r == 0 || r == rings { [0.0, ct, 0.0] } else { [st * phi.cos(), ct, st * phi.sin()] };
                let p = if s == segments && r != 0 && r != rings { let q = vertices_at(&vertices, r, segments); q } else { p };
                vertices.push(Vertex { position: p, normal: p, uv: [s as f32 / segments as f32, r as f32 / rings as f32] });
            }
        }
        let row = segments + 1;
        let mut indices = Vec::new();
        for r in 0..rings {
            for s in 0..segments {
                let (a, b, c, d) = (r * row + s, r * row + s + 1, (r + 1) * row + s, (r + 1) * row + s + 1);
                if r != 0 {
                    indices.extend_from_slice(&[a, c, b]);
                }
                if r != rings - 1 {
                    indices.extend_from_slice(&[b, c, d]);
                }
            }
        }
        (vertices, indices)
    }

    fn vertices_at(vertices: &[Vertex], r: u32, segments: u32) -> [f32; 3] {
        vertices[(r * (segments + 1)) as usize].position
    }

    /// Every edge of the chosen clusters, by place, is in exactly two of
    /// their triangles: the surface is closed, whatever the mix of levels.
    fn closed(vertices: &[Vertex], indices: &[u32], chosen: &[&ClusterRaw]) -> Result<(), String> {
        let place = |v: u32| vertices[v as usize].position.map(f32::to_bits);
        let mut edges: std::collections::HashMap<([u32; 3], [u32; 3]), i32> = std::collections::HashMap::new();
        for c in chosen {
            for t in indices[c.first as usize..(c.first + c.count * 3) as usize].chunks_exact(3) {
                for k in 0..3 {
                    let (a, b) = (place(t[k]), place(t[(k + 1) % 3]));
                    if a == b {
                        continue;
                    }
                    *edges.entry(if a < b { (a, b) } else { (b, a) }).or_default() += 1;
                }
            }
        }
        let open = edges.values().filter(|&&n| n != 2).count();
        if open > 0 {
            return Err(format!("{open} of {} edges are not shared by two triangles", edges.len()));
        }
        Ok(())
    }

    #[test]
    fn a_ball_gets_coarser_levels_up_to_a_top_and_every_cut_through_them_is_closed() {
        let (vertices, indices) = ball(90);
        let (sorted, level0) = crate::cluster::split(&vertices, &indices, 0);
        let (extra, all) = levels(&vertices, &sorted, &level0);
        let mut buffer = sorted.clone();
        buffer.extend_from_slice(&extra);
        let triangles0: u32 = level0.iter().map(|c| c.count).sum();
        eprintln!("{} clusters at level 0 ({triangles0} triangles), {} in all, {} indices over", level0.len(), all.len(), extra.len());
        assert!(all.len() > level0.len() * 3 / 2, "coarser levels were made");
        // Level 0 alone is the mesh: closed.
        let zero: Vec<&ClusterRaw> = all.iter().filter(|c| c.error[0] == 0.0).collect();
        closed(&vertices, &buffer, &zero).map_err(|e| format!("level 0: {e}")).unwrap();
        // The errors grow upward and the spheres hold their children's.
        for c in &all {
            assert!(c.error[1] >= c.error[0], "a parent's error under its child's: {:?}", c.error);
            if c.error[1] < NEVER {
                let (a, b) = (Vec3::from_slice(&c.lod[0][..3]), Vec3::from_slice(&c.lod[1][..3]));
                assert!(a.distance(b) + c.lod[0][3] <= c.lod[1][3] * 1.001 + 1e-4, "a parent's sphere does not hold its child's");
            }
        }
        // From near to far, the chosen clusters close the ball, and fewer
        // triangles are chosen the farther it is.
        let mut last = u32::MAX;
        for distance in [1.5f32, 4.0, 12.0, 40.0, 150.0, 600.0] {
            let eye = Vec3::new(0.0, 0.3, distance);
            let chosen: Vec<&ClusterRaw> = all.iter().filter(|c| chosen(c, glam::Mat4::IDENTITY, eye, 540.0, 0.1, 1.0)).collect();
            let drawn: u32 = chosen.iter().map(|c| c.count).sum();
            eprintln!("from {distance} m: {} clusters, {drawn} triangles", chosen.len());
            closed(&vertices, &buffer, &chosen).map_err(|e| format!("from {distance} m: {e}")).unwrap();
            assert!(drawn <= last, "farther drew more");
            last = drawn;
        }
        assert!(last * 20 < triangles0, "far away the ball is a few triangles: {last} of {triangles0}");
    }

    #[test]
    fn a_mesh_of_facets_keeps_its_one_level() {
        // Every triangle its own three vertices, its own normal.
        let (smooth, indices) = ball(60);
        let mut vertices = Vec::new();
        let mut split = Vec::new();
        for t in indices.chunks_exact(3) {
            let (a, b, c) = (Vec3::from(smooth[t[0] as usize].position), Vec3::from(smooth[t[1] as usize].position), Vec3::from(smooth[t[2] as usize].position));
            let n = (b - a).cross(c - a).normalize_or_zero().to_array();
            for p in [a, b, c] {
                split.push(vertices.len() as u32);
                vertices.push(Vertex { position: p.to_array(), normal: n, uv: [0.0, 0.0] });
            }
        }
        let (sorted, level0) = crate::cluster::split(&vertices, &split, 0);
        let (extra, all) = levels(&vertices, &sorted, &level0);
        assert!(extra.is_empty() && all.len() == level0.len(), "facets were simplified away");
    }
}
