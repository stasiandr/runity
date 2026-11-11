//! Z-fighting: two surfaces at (nearly) the same depth, each winning some
//! of the pixels they share, the pattern crawling as the camera moves — a
//! decal flush with its floor, two walls back to back, a helmet's inside
//! on the head it sits on. Two ways to find it, for any game, headless:
//!
//! * **From the geometry** ([`overlaps`]): every draw's triangles in the
//!   world (the meshes kept on the CPU by [`crate::Renderer::keep_geometry`]),
//!   bucketed by facing, plane and place; two faces of different draws
//!   facing the same way, closer than [`Limits::coplanar`] (or within
//!   [`Limits::near`], the depth buffer's danger zone at a game's camera
//!   distances) and overlapping in their plane by more than
//!   [`Limits::min_area`] are one finding. No picture is drawn, so a
//!   whole level, every asset alone and every body in every armour are
//!   checked in seconds.
//! * **From pictures** ([`id_frame`], [`decode_ids`], [`Flicker`]): the
//!   frame drawn so each pixel says which draw it shows (every draw unlit
//!   in a colour of its own, nothing after), several times — the camera
//!   moved a hair, the draws' order turned round — and the pixels whose
//!   winner changes where both contenders' depths along the pixel's ray
//!   are within a few millimetres ([`ray_depth`]); an edge moving under the
//!   jitter has depths far apart. What the geometry cannot see — a
//!   shader's offset, a depth bias, precision at a distance — shows here.
//!   [`heatmap`] paints the result over the picture with the worst places
//!   boxed and named.
//!
//! What a draw is called (an asset's name, an effect's) is the game's: the
//! findings are by draw index.

use glam::{Mat4, Vec2, Vec3, Vec4Swizzles};
use std::collections::HashMap;

use crate::material::{Material, RenderFace, Shading, SurfaceType};
use crate::render::{Camera, Draw, Frame, MeshHandle, TextureHandle};

/// A mesh's triangles as the CPU keeps them (in the mesh's own space).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshGeometry {
    pub positions: Vec<Vec3>,
    pub indices: Vec<u32>,
}

impl MeshGeometry {
    pub fn new(positions: Vec<Vec3>, indices: Vec<u32>) -> Self {
        Self { positions, indices }
    }

    /// Its triangles placed by `m`.
    pub fn triangles(&self, m: Mat4) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.indices.chunks_exact(3).filter_map(move |t| {
            let p = |i: u32| self.positions.get(i as usize).map(|v| m.transform_point3(*v));
            Some([p(t[0])?, p(t[1])?, p(t[2])?])
        })
    }
}

/// How close is a fight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// Planes closer than this (metres) fight from any distance.
    pub coplanar: f32,
    /// Planes closer than this fight at a distance (the depth buffer's
    /// steps grow with the square of it): reported as [`Kind::Near`].
    pub near: f32,
    /// Two faces face the same way when their normals' dot is above this.
    pub parallel: f32,
    /// Square metres of shared face below which a touch is not a finding.
    pub min_area: f32,
    /// Two see-through draws (neither writes depth) do not fight: whichever
    /// is drawn last is on top, every frame.
    pub skip_both_transparent: bool,
    /// Faces nobody can see are left out: shared face whose front, a few
    /// millimetres out, is inside a closed mesh (the end of a wall inside
    /// the post at its corner, a neck inside its collar).
    pub only_seen: bool,
    /// Report faces back to back (one drawn from both sides). Closed meshes
    /// meeting face to face hide both: only open sheets (a quad drawn from
    /// both sides) show it, so it is off by default.
    pub back_to_back: bool,
    /// Faces turned this way (a unit direction, and the dot above which a
    /// face counts as turned so) are never seen by the game's cameras —
    /// the undersides, for a game whose cameras all look down — and are
    /// left out.
    pub unseen_facing: Option<(Vec3, f32)>,
}

impl Default for Limits {
    fn default() -> Self {
        Self { coplanar: 0.002, near: 0.006, parallel: 0.999, min_area: 1e-4, skip_both_transparent: true, only_seen: true, back_to_back: false, unseen_facing: None }
    }
}

/// What kind of fight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// The same mesh at the same place drawn twice, looking different (the
    /// same look twice is only wasted work).
    Duplicate,
    /// Faces facing the same way in (nearly) one plane.
    Coplanar,
    /// Faces facing the same way closer than the depth buffer can tell
    /// apart at a distance.
    Near,
    /// Faces back to back in one plane, one of them drawn from both sides.
    BackToBack,
}

/// Two draws fighting: `a < b`, indices into what was checked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overlap {
    pub a: usize,
    pub b: usize,
    pub kind: Kind,
    /// Square metres of face they share.
    pub area: f32,
    /// The smallest distance between their planes where they overlap.
    pub gap: f32,
    /// The middle of the largest shared piece, and its facing.
    pub at: Vec3,
    pub normal: Vec3,
}

struct Tri {
    draw: u32,
    p: [Vec3; 3],
    n: Vec3,
    d: f32,
    min: Vec3,
    max: Vec3,
}

/// Every pair of draws whose faces fight (one [`Overlap`] a pair: its
/// shared area summed), worst first. `geometry` gives a draw's mesh's
/// triangles (none: left out).
pub fn overlaps<'g>(draws: &[Draw], geometry: impl Fn(MeshHandle) -> Option<&'g MeshGeometry>, limits: &Limits) -> Vec<Overlap> {
    let mut out: HashMap<(u32, u32), Overlap> = HashMap::new();
    // The same mesh at the same place: a duplicate, whatever its faces.
    let mut placed: HashMap<(MeshHandle, [i32; 12]), u32> = HashMap::new();
    let mut tris: Vec<Tri> = Vec::new();
    // Each draw's triangles (a range of `tris`) and box.
    let mut solids: Vec<(std::ops::Range<usize>, Vec3, Vec3)> = Vec::new();
    for (i, draw) in draws.iter().enumerate() {
        let Some(g) = geometry(draw.mesh) else { continue };
        let cols = draw.transform.to_cols_array();
        let key: [i32; 12] = std::array::from_fn(|k| {
            let v = cols[if k < 9 { k + k / 3 } else { 12 + k - 9 }];
            (v * 1000.0).round() as i32
        });
        if let Some(&first) = placed.get(&(draw.mesh, key)) {
            let earlier = &draws[first as usize];
            if earlier.material == draw.material && earlier.texture == draw.texture {
                continue;
            }
            let area: f32 = g.triangles(draw.transform).map(|t| (t[1] - t[0]).cross(t[2] - t[0]).length() * 0.5).sum();
            let at = draw.transform.w_axis.xyz();
            out.insert((first, i as u32), Overlap { a: first as usize, b: i, kind: Kind::Duplicate, area, gap: 0.0, at, normal: Vec3::Y });
            continue;
        }
        placed.insert((draw.mesh, key), i as u32);
        let first = tris.len();
        for p in g.triangles(draw.transform) {
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            let len = n.length();
            if len < 1e-9 {
                continue;
            }
            let n = n / len;
            tris.push(Tri { draw: i as u32, p, n, d: n.dot(p[0]), min: p[0].min(p[1]).min(p[2]), max: p[0].max(p[1]).max(p[2]) });
        }
        if tris.len() > first {
            let (lo, hi) = tris[first..].iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), t| (a.min(t.min), b.max(t.max)));
            solids.push((first..tris.len(), lo, hi));
        }
    }
    // Where each draw's box is, by two-metre squares: what a point may be
    // inside of.
    let mut boxes: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (k, (_, lo, hi)) in solids.iter().enumerate() {
        for x in (lo.x / 2.0).floor() as i32..=(hi.x / 2.0).floor() as i32 {
            for z in (lo.z / 2.0).floor() as i32..=(hi.z / 2.0).floor() as i32 {
                boxes.entry((x, z)).or_default().push(k as u32);
            }
        }
    }
    let hidden = |at: Vec3| -> bool {
        let Some(list) = boxes.get(&((at.x / 2.0).floor() as i32, (at.z / 2.0).floor() as i32)) else { return false };
        // Off every axis and edge: a ray that meets edges counts twice.
        let dir = Vec3::new(0.3127, 0.8413, 0.4402).normalize();
        list.iter().any(|k| {
            let (range, lo, hi) = &solids[*k as usize];
            if at.cmplt(*lo).any() || at.cmpgt(*hi).any() {
                return false;
            }
            let crossings = tris[range.clone()].iter().filter(|t| ray_hits(at, dir, &t.p)).count();
            crossings % 2 == 1
        })
    };
    // Buckets: the facing's main axis (a face near a diagonal in both of
    // its), the plane's distance in steps of `near`, and a metre's square
    // of the plane's other two axes.
    let step = limits.near.max(limits.coplanar).max(1e-4) * 1.01;
    let mut grid: HashMap<(u8, i32, i32, i32), Vec<u32>> = HashMap::new();
    let axes_of = |n: Vec3| -> Vec<u8> {
        let a = n.abs();
        let order = if a.x >= a.y && a.x >= a.z { [0, 1, 2] } else if a.y >= a.z { [1, 0, 2] } else { [2, 0, 1] };
        let big = a[order[0]];
        let mut v = vec![order[0] as u8 * 2 + u8::from(n[order[0]] < 0.0)];
        for k in &order[1..] {
            if a[*k] > big * 0.6 {
                v.push(*k as u8 * 2 + u8::from(n[*k] < 0.0));
            }
        }
        v
    };
    let others = |axis: u8| match axis / 2 {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    for (t, tri) in tris.iter().enumerate() {
        // The plane's distance measured along the bucket's axis (the
        // normal turned to its positive side), so faces turned round each
        // other share it (back to back).
        for axis in axes_of(tri.n) {
            let sign = if tri.n[(axis / 2) as usize] < 0.0 { -1.0 } else { 1.0 };
            let dk = (sign * tri.d / step).floor() as i32;
            let (u, v) = others(axis);
            for gu in tri.min[u].floor() as i32..=tri.max[u].floor() as i32 {
                for gv in tri.min[v].floor() as i32..=tri.max[v].floor() as i32 {
                    grid.entry((axis / 2, dk, gu, gv)).or_default().push(t as u32);
                }
            }
        }
    }
    let mut seen: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
    let mut biggest: HashMap<(u32, u32), f32> = HashMap::new();
    let mut keys: Vec<&(u8, i32, i32, i32)> = grid.keys().collect();
    keys.sort();
    for key in keys {
        let here = &grid[key];
        // Each pair of neighbouring plane distances once: the lower asks.
        for dd in [0, 1] {
            let Some(there) = grid.get(&(key.0, key.1 + dd, key.2, key.3)) else { continue };
            for &ti in here {
                for &ui in there {
                    if dd == 0 && ui <= ti {
                        continue;
                    }
                    let (t, u) = (&tris[ti as usize], &tris[ui as usize]);
                    if t.draw == u.draw {
                        continue;
                    }
                    let gap0 = 2.0 * step;
                    if (t.min - Vec3::splat(gap0)).cmpgt(u.max).any() || (u.min - Vec3::splat(gap0)).cmpgt(t.max).any() {
                        continue;
                    }
                    if limits.unseen_facing.is_some_and(|(away, k)| t.n.dot(away) > k) {
                        continue;
                    }
                    let dot = t.n.dot(u.n);
                    let same = dot > limits.parallel;
                    let (a, b) = (&draws[t.draw as usize], &draws[u.draw as usize]);
                    let both_faces = a.material.render_face == RenderFace::Both || b.material.render_face == RenderFace::Both;
                    if !same && !(limits.back_to_back && dot < -limits.parallel && both_faces) {
                        continue;
                    }
                    if limits.skip_both_transparent && a.material.surface == SurfaceType::Transparent && b.material.surface == SurfaceType::Transparent {
                        continue;
                    }
                    let pair = (ti.min(ui), ti.max(ui));
                    if !seen.insert(pair) {
                        continue;
                    }
                    let Some((area, centre)) = shared(t, u) else { continue };
                    if area < 1e-7 {
                        continue;
                    }
                    let gap = (u.n.dot(centre) - u.d).abs();
                    if limits.only_seen && hidden(centre + t.n * 0.004) {
                        continue;
                    }
                    let kind = if !same {
                        if gap > limits.coplanar {
                            continue;
                        }
                        Kind::BackToBack
                    } else if gap <= limits.coplanar {
                        Kind::Coplanar
                    } else if gap <= limits.near {
                        Kind::Near
                    } else {
                        continue;
                    };
                    let (i, j) = (t.draw.min(u.draw), t.draw.max(u.draw));
                    let e = out.entry((i, j)).or_insert(Overlap { a: i as usize, b: j as usize, kind, area: 0.0, gap, at: centre, normal: t.n });
                    let big = biggest.entry((i, j)).or_insert(0.0);
                    if area > *big {
                        *big = area;
                        e.at = centre;
                        e.normal = t.n;
                    }
                    e.area += area;
                    e.gap = e.gap.min(gap);
                    e.kind = e.kind.min(kind);
                }
            }
        }
    }
    let mut list: Vec<Overlap> = out.into_values().filter(|o| o.kind == Kind::Duplicate || o.area >= limits.min_area).collect();
    list.sort_by(|x, y| y.area.total_cmp(&x.area).then(x.a.cmp(&y.a)).then(x.b.cmp(&y.b)));
    list
}

/// The area two triangles share in `t`'s plane, and its middle.
fn shared(t: &Tri, u: &Tri) -> Option<(f32, Vec3)> {
    let n = t.n;
    let e1 = n.any_orthonormal_vector();
    let e2 = n.cross(e1);
    let flat = |p: Vec3| Vec2::new(p.dot(e1), p.dot(e2));
    let mut poly: Vec<Vec2> = t.p.iter().map(|p| flat(*p)).collect();
    let mut clip: Vec<Vec2> = u.p.iter().map(|p| flat(*p)).collect();
    if signed_area(&poly) < 0.0 {
        poly.reverse();
    }
    if signed_area(&clip) < 0.0 {
        clip.reverse();
    }
    for k in 0..clip.len() {
        let (a, b) = (clip[k], clip[(k + 1) % clip.len()]);
        let inside = |p: Vec2| (b - a).perp_dot(p - a) >= -1e-7;
        let mut next = Vec::with_capacity(poly.len() + 2);
        for i in 0..poly.len() {
            let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
            let (pin, qin) = (inside(p), inside(q));
            if pin {
                next.push(p);
            }
            if pin != qin {
                let d = q - p;
                let den = (b - a).perp_dot(d);
                if den.abs() > 1e-12 {
                    let s = (b - a).perp_dot(a - p) / den;
                    next.push(p + d * s.clamp(0.0, 1.0));
                }
            }
        }
        poly = next;
        if poly.len() < 3 {
            return None;
        }
    }
    let area = signed_area(&poly);
    if area <= 0.0 {
        return None;
    }
    let mid = poly.iter().copied().sum::<Vec2>() / poly.len() as f32;
    // Back into the world, on t's plane.
    let centre = e1 * mid.x + e2 * mid.y + n * t.d;
    Some((area, centre))
}

/// A ray from `from` along `dir` meets triangle `p` ahead.
fn ray_hits(from: Vec3, dir: Vec3, p: &[Vec3; 3]) -> bool {
    let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
    let h = dir.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-12 {
        return false;
    }
    let f = 1.0 / det;
    let s = from - p[0];
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = s.cross(e1);
    let v = f * dir.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    f * e2.dot(q) > 0.0
}

fn signed_area(p: &[Vec2]) -> f32 {
    let mut s = 0.0;
    for i in 0..p.len() {
        s += p[i].perp_dot(p[(i + 1) % p.len()]);
    }
    s * 0.5
}

// ---------------------------------------------------------------- pictures

/// Draws an id picture tells apart: five bits a channel.
pub const ID_LEVELS: u32 = 32;

/// The colour (linear, for an sRGB target) a picture of ids paints draw
/// number `id` with; 0 is nothing (the background).
pub fn id_color(id: u32) -> [f32; 3] {
    let id = id % (ID_LEVELS * ID_LEVELS * ID_LEVELS);
    let level = |k: u32| {
        let byte = 4.0 + 8.0 * k as f32;
        let c = byte / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    [level(id % ID_LEVELS), level(id / ID_LEVELS % ID_LEVELS), level(id / (ID_LEVELS * ID_LEVELS))]
}

/// A pixel that is no one draw's: an edge where the samples mixed.
pub const MIXED: u32 = u32::MAX;

/// The draw each pixel of an id picture (RGBA8, sRGB) shows: its number
/// as [`id_frame`] numbered them (0 nothing), or [`MIXED`].
pub fn decode_ids(rgba: &[u8]) -> Vec<u32> {
    rgba.chunks_exact(4)
        .map(|p| {
            // The cleared background: black.
            if p[0] <= 1 && p[1] <= 1 && p[2] <= 1 {
                return 0;
            }
            let level = |b: u8| {
                let k = (b as i32 - 4 + 4).div_euclid(8);
                let want = 4 + 8 * k;
                ((b as i32 - want).abs() <= 2 && (0..ID_LEVELS as i32).contains(&k)).then_some(k as u32)
            };
            match (level(p[0]), level(p[1]), level(p[2])) {
                (Some(r), Some(g), Some(b)) => r + g * ID_LEVELS + b * ID_LEVELS * ID_LEVELS,
                _ => MIXED,
            }
        })
        .collect()
}

/// `frame` drawn as ids: every draw (the frame's, then each view's in
/// turn: [`draw_list`]) unlit in [`id_color`]`(index + 1)`, opaque, its
/// textures and both-faced-ness kept as they are culled; no lights,
/// shadows, fog, post, decals or particles. `reverse`: the draws in the
/// opposite order, so two at exactly one depth swap winners.
pub fn id_frame(frame: &Frame, reverse: bool) -> Frame {
    let mut next = 1u32;
    let mut paint = |list: &[Draw]| -> Vec<Draw> {
        let mut out: Vec<Draw> = list
            .iter()
            .map(|d| {
                let c = id_color(next);
                next += 1;
                let material = Material {
                    base_color: c,
                    shading: Shading::Unlit,
                    emission: [0.0; 3],
                    alpha: 1.0,
                    alpha_clip: 0.0,
                    render_face: d.material.render_face,
                    ..Material::default()
                };
                Draw { mesh: d.mesh, transform: d.transform, texture: TextureHandle::WHITE, pose: d.pose, material }
            })
            .collect();
        if reverse {
            out.reverse();
        }
        out
    };
    let mut f = frame.clone();
    f.draws = paint(&frame.draws);
    for v in &mut f.views {
        v.draws = paint(&v.draws);
    }
    f.overlay_draws.clear();
    f.outline_draws.clear();
    f.lights.clear();
    f.flares.clear();
    f.decals.clear();
    f.puffs.clear();
    f.smoke.clear();
    f.plumes.clear();
    f.gpu_particles.clear();
    f.live_meshes.clear();
    f.fullscreen = None;
    f.shadows.enabled = false;
    f.fog = crate::render::FogSettings { start: 1e7, end: 2e7, ..Default::default() };
    f.volumetric_fog.enabled = false;
    f.ambient_occlusion.enabled = false;
    f.post = crate::post::PostProcess::OFF;
    f.post.enabled = false;
    f.clear_color = Vec3::ZERO;
    f.sky.mode = crate::render::SkyMode::Color;
    f
}

/// Every draw of `frame` in [`id_frame`]'s numbering (index + 1 is the
/// id): the frame's own, then each view's.
pub fn draw_list(frame: &Frame) -> Vec<Draw> {
    let mut out = frame.draws.clone();
    for v in &frame.views {
        out.extend(v.draws.iter().cloned());
    }
    out
}

/// The ray through pixel (`x`, `y`) of a picture `size` (its middle) of
/// a view drawn into `rect` = (x, y, width, height) pixels with `camera`:
/// from the eye, unit length.
pub fn pixel_ray(camera: &Camera, rect: (u32, u32, u32, u32), x: u32, y: u32) -> (Vec3, Vec3) {
    let (rx, ry, rw, rh) = rect;
    let inv = camera.view_projection(rw as f32 / rh as f32).inverse();
    let nx = ((x as f32 + 0.5 - rx as f32) / rw as f32) * 2.0 - 1.0;
    let ny = 1.0 - ((y as f32 + 0.5 - ry as f32) / rh as f32) * 2.0;
    let near = inv.project_point3(Vec3::new(nx, ny, 0.0));
    let far = inv.project_point3(Vec3::new(nx, ny, 1.0));
    (near, (far - near).normalize_or_zero())
}

/// How far along the ray (`from`, `dir`) a mesh placed by `transform`
/// is first met (from either side), if it is.
pub fn ray_depth(geometry: &MeshGeometry, transform: Mat4, from: Vec3, dir: Vec3) -> Option<f32> {
    let mut best: Option<f32> = None;
    for [a, b, c] in geometry.triangles(transform) {
        let (e1, e2) = (b - a, c - a);
        let h = dir.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1e-10 {
            continue;
        }
        let f = 1.0 / det;
        let s = from - a;
        let u = f * s.dot(h);
        if !(-1e-4..=1.0 + 1e-4).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = f * dir.dot(q);
        if v < -1e-4 || u + v > 1.0 + 1e-4 {
            continue;
        }
        let t = f * e2.dot(q);
        if t > 0.0 && best.is_none_or(|x| t < x) {
            best = Some(t);
        }
    }
    best
}

/// What the id pictures of one view say pixel by pixel: how often each
/// pixel's winner changed, and between which two draws.
#[derive(Debug, Clone)]
pub struct Flicker {
    pub width: u32,
    pub height: u32,
    /// Per pixel: the two draws (ids) that won it most often, when two did.
    pub pairs: Vec<Option<(u32, u32)>>,
}

impl Flicker {
    /// From pictures of the same size (ids per pixel, [`decode_ids`]); a
    /// [`MIXED`] pixel in a picture says nothing.
    pub fn of(pictures: &[Vec<u32>], width: u32, height: u32) -> Self {
        let n = (width * height) as usize;
        let mut pairs = vec![None; n];
        for (i, slot) in pairs.iter_mut().enumerate() {
            let mut counts: Vec<(u32, u32)> = Vec::new();
            for pic in pictures {
                let id = pic[i];
                if id == MIXED || id == 0 {
                    continue;
                }
                match counts.iter_mut().find(|(k, _)| *k == id) {
                    Some(c) => c.1 += 1,
                    None => counts.push((id, 1)),
                }
            }
            if counts.len() >= 2 {
                counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                let (a, b) = (counts[0].0, counts[1].0);
                *slot = Some((a.min(b), a.max(b)));
            }
        }
        Self { width, height, pairs }
    }
}

/// A place a picture shows two draws fighting.
#[derive(Debug, Clone, PartialEq)]
pub struct Hot {
    /// The two ids.
    pub ids: (u32, u32),
    /// Pixels of it, and the box round them (x0, y0, x1, y1).
    pub pixels: u32,
    pub rect: (u32, u32, u32, u32),
}

/// The fighting pixels' places, by pair, most pixels first: pixels of one
/// pair within `join` pixels of each other are one place.
pub fn hot_places(fighting: &[Option<(u32, u32)>], width: u32, height: u32, join: u32) -> Vec<Hot> {
    let mut out: Vec<Hot> = Vec::new();
    let mut by_pair: HashMap<(u32, u32), Vec<(u32, u32)>> = HashMap::new();
    for (i, f) in fighting.iter().enumerate() {
        if let Some(p) = f {
            by_pair.entry(*p).or_default().push((i as u32 % width, i as u32 / width));
        }
    }
    let _ = height;
    let mut pairs: Vec<_> = by_pair.into_iter().collect();
    pairs.sort_by_key(|(k, _)| *k);
    for (ids, pts) in pairs {
        let mut places: Vec<Hot> = Vec::new();
        for (x, y) in pts {
            let near = places.iter_mut().find(|h| x + join >= h.rect.0 && x <= h.rect.2 + join && y + join >= h.rect.1 && y <= h.rect.3 + join);
            match near {
                Some(h) => {
                    h.pixels += 1;
                    h.rect = (h.rect.0.min(x), h.rect.1.min(y), h.rect.2.max(x), h.rect.3.max(y));
                }
                None => places.push(Hot { ids, pixels: 1, rect: (x, y, x, y) }),
            }
        }
        out.extend(places);
    }
    out.sort_by(|a, b| b.pixels.cmp(&a.pixels).then(a.ids.cmp(&b.ids)));
    out
}

/// A heat map: `base` (RGBA8) greyed and darkened, the fighting pixels
/// red, and the first `boxes` places boxed in yellow with `labels`
/// written over them (capitals, digits, `_ - . : # /` and spaces; others
/// as `?`).
pub fn heatmap(base: &[u8], width: u32, height: u32, fighting: &[bool], places: &[(u32, u32, u32, u32)], labels: &[String]) -> Vec<u8> {
    let mut out = Vec::with_capacity(base.len());
    for (i, p) in base.chunks_exact(4).enumerate() {
        let grey = (p[0] as f32 * 0.3 + p[1] as f32 * 0.55 + p[2] as f32 * 0.15) * 0.45;
        if fighting.get(i).copied().unwrap_or(false) {
            out.extend_from_slice(&[255, 40, 30, 255]);
        } else {
            let g = grey as u8;
            out.extend_from_slice(&[g, g, g, 255]);
        }
    }
    let mut put = |x: i32, y: i32, c: [u8; 3]| {
        if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
            let k = ((y as u32 * width + x as u32) * 4) as usize;
            out[k..k + 3].copy_from_slice(&c);
        }
    };
    for (n, r) in places.iter().enumerate() {
        let (x0, y0, x1, y1) = (r.0 as i32 - 3, r.1 as i32 - 3, r.2 as i32 + 3, r.3 as i32 + 3);
        for x in x0..=x1 {
            for w in 0..2 {
                put(x, y0 - w, [255, 220, 0]);
                put(x, y1 + w, [255, 220, 0]);
            }
        }
        for y in y0..=y1 {
            for w in 0..2 {
                put(x0 - w, y, [255, 220, 0]);
                put(x1 + w, y, [255, 220, 0]);
            }
        }
        if let Some(text) = labels.get(n) {
            let ty = if y0 > 14 { y0 - 14 } else { y1 + 4 };
            let mut x = x0;
            for ch in text.chars() {
                let rows = glyph(ch.to_ascii_uppercase());
                // Shadow, then the letter, at twice the font's size.
                for (ry, row) in rows.iter().enumerate() {
                    for rx in 0..3 {
                        if row & (4 >> rx) != 0 {
                            for (dx, dy, c) in [(1, 1, [0u8, 0, 0]), (0, 0, [255u8, 255, 255])] {
                                for sy in 0..2 {
                                    for sx in 0..2 {
                                        put(x + rx * 2 + sx + dx, ty + ry as i32 * 2 + sy + dy, c);
                                    }
                                }
                            }
                        }
                    }
                }
                x += 8;
            }
        }
    }
    out
}

/// A 3×5 letter: five rows of three bits, the left one high.
fn glyph(c: char) -> [u8; 5] {
    match c {
        'A' => [2, 5, 7, 5, 5],
        'B' => [6, 5, 6, 5, 6],
        'C' => [3, 4, 4, 4, 3],
        'D' => [6, 5, 5, 5, 6],
        'E' => [7, 4, 6, 4, 7],
        'F' => [7, 4, 6, 4, 4],
        'G' => [3, 4, 5, 5, 3],
        'H' => [5, 5, 7, 5, 5],
        'I' => [7, 2, 2, 2, 7],
        'J' => [1, 1, 1, 5, 2],
        'K' => [5, 5, 6, 5, 5],
        'L' => [4, 4, 4, 4, 7],
        'M' => [5, 7, 7, 5, 5],
        'N' => [6, 5, 5, 5, 5],
        'O' => [2, 5, 5, 5, 2],
        'P' => [6, 5, 6, 4, 4],
        'Q' => [2, 5, 5, 6, 3],
        'R' => [6, 5, 6, 5, 5],
        'S' => [3, 4, 2, 1, 6],
        'T' => [7, 2, 2, 2, 2],
        'U' => [5, 5, 5, 5, 7],
        'V' => [5, 5, 5, 5, 2],
        'W' => [5, 5, 7, 7, 5],
        'X' => [5, 5, 2, 5, 5],
        'Y' => [5, 5, 2, 2, 2],
        'Z' => [7, 1, 2, 4, 7],
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [6, 1, 2, 4, 7],
        '3' => [6, 1, 2, 1, 6],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 6, 1, 6],
        '6' => [3, 4, 7, 5, 7],
        '7' => [7, 1, 2, 2, 2],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 6],
        '_' => [0, 0, 0, 0, 7],
        '-' => [0, 0, 7, 0, 0],
        '.' => [0, 0, 0, 0, 2],
        ':' => [0, 2, 0, 2, 0],
        '#' => [5, 7, 5, 7, 5],
        '/' => [1, 1, 2, 4, 4],
        '+' => [0, 2, 7, 2, 0],
        ' ' => [0; 5],
        _ => [7, 1, 2, 0, 2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::MeshHandle;

    fn quad(size: f32) -> MeshGeometry {
        let h = size * 0.5;
        // A floor quad facing up.
        MeshGeometry::new(vec![Vec3::new(-h, 0.0, -h), Vec3::new(h, 0.0, -h), Vec3::new(h, 0.0, h), Vec3::new(-h, 0.0, h)], vec![0, 2, 1, 0, 3, 2])
    }

    /// A unit cube about its middle, faces outward.
    fn cube() -> MeshGeometry {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for axis in 0..3 {
            for sign in [-1.0f32, 1.0] {
                let n = Vec3::from_array(std::array::from_fn(|k| if k == axis { sign } else { 0.0 }));
                let u = Vec3::from_array(std::array::from_fn(|k| if k == (axis + 1) % 3 { 1.0 } else { 0.0 }));
                let v = n.cross(u);
                let base = positions.len() as u32;
                for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    positions.push((n + u * a + v * b) * 0.5);
                }
                indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
        MeshGeometry::new(positions, indices)
    }

    fn draw(at: Vec3) -> Draw {
        Draw { mesh: MeshHandle::TEST, transform: Mat4::from_translation(at), texture: TextureHandle::WHITE, pose: None, material: Material::default() }
    }

    #[test]
    fn a_decal_flush_with_its_floor_fights_and_one_lifted_does_not() {
        let floor = quad(2.0);
        let geometry = |_| Some(&floor);
        let draws = vec![draw(Vec3::ZERO), draw(Vec3::new(0.5, 0.0005, 0.5))];
        let found = overlaps(&draws, geometry, &Limits::default());
        assert_eq!(found.len(), 1, "{found:?}");
        let o = found[0];
        assert_eq!((o.a, o.b, o.kind), (0, 1, Kind::Coplanar));
        // Overlap: x 0..1 × z 0..1 of the lifted one inside the floor's.
        assert!((o.area - 2.25).abs() < 1e-3, "area {}", o.area);
        assert!(o.normal.y > 0.99);
        // Four millimetres up: near (the danger zone); a centimetre: clean.
        let draws = vec![draw(Vec3::ZERO), draw(Vec3::new(0.5, 0.004, 0.5))];
        assert_eq!(overlaps(&draws, geometry, &Limits::default())[0].kind, Kind::Near);
        let draws = vec![draw(Vec3::ZERO), draw(Vec3::new(0.5, 0.01, 0.5))];
        assert!(overlaps(&draws, geometry, &Limits::default()).is_empty());
        // Side by side, touching along an edge: no shared area.
        let draws = vec![draw(Vec3::ZERO), draw(Vec3::new(2.0, 0.0, 0.0))];
        assert!(overlaps(&draws, geometry, &Limits::default()).is_empty());
    }

    #[test]
    fn faces_turned_round_fight_only_when_drawn_from_both_sides() {
        let floor = quad(2.0);
        let geometry = |_| Some(&floor);
        let mut flipped = draw(Vec3::ZERO);
        flipped.transform = Mat4::from_rotation_x(std::f32::consts::PI);
        let draws = vec![draw(Vec3::ZERO), flipped];
        assert!(overlaps(&draws, geometry, &Limits::default()).is_empty());
        let mut both = flipped;
        both.material.render_face = RenderFace::Both;
        assert!(overlaps(&[draw(Vec3::ZERO), both], geometry, &Limits::default()).is_empty(), "off by default");
        let found = overlaps(&[draw(Vec3::ZERO), both], geometry, &Limits { back_to_back: true, ..Limits::default() });
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, Kind::BackToBack);
    }

    #[test]
    fn the_same_mesh_twice_in_one_place_is_a_duplicate() {
        let floor = quad(2.0);
        // The same look twice: wasted, not a fight.
        assert!(overlaps(&[draw(Vec3::ONE), draw(Vec3::ONE)], |_| Some(&floor), &Limits::default()).is_empty());
        let mut red = draw(Vec3::ONE);
        red.material.base_color = [1.0, 0.0, 0.0];
        let found = overlaps(&[draw(Vec3::ONE), red], |_| Some(&floor), &Limits::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, Kind::Duplicate);
    }

    #[test]
    fn faces_inside_another_solid_are_not_seen() {
        // Two slabs' tops in one plane, the shared strip under a lid box
        // sitting on them: nobody sees it.
        let cube = cube();
        let slab = |z: f32| Draw { transform: Mat4::from_scale_rotation_translation(Vec3::new(2.0, 1.0, 0.4), Default::default(), Vec3::new(0.0, 0.5, z)), ..draw(Vec3::ZERO) };
        let lid = Draw { transform: Mat4::from_scale_rotation_translation(Vec3::new(3.0, 0.5, 1.0), Default::default(), Vec3::new(0.0, 1.2, 0.0)), ..draw(Vec3::ZERO) };
        let open = overlaps(&[slab(0.0), slab(0.2)], |_| Some(&cube), &Limits::default());
        assert!(!open.is_empty());
        // The lid covers the tops (and the slabs' ends stay out in the open).
        let covered = overlaps(&[slab(0.0), slab(0.2), lid], |_| Some(&cube), &Limits::default());
        let tops: f32 = covered.iter().filter(|o| o.normal.y > 0.9).map(|o| o.area).sum();
        assert_eq!(tops, 0.0, "{covered:?}");
    }

    #[test]
    fn walls_turned_any_way_are_found_as_floors_are() {
        // Two thin slabs back to back, turned 30° about y: their tops share
        // a strip.
        let cube = cube();
        let turn = Mat4::from_rotation_y(0.52);
        let a = Draw { transform: turn * Mat4::from_scale_rotation_translation(Vec3::new(2.0, 3.0, 0.2), Default::default(), Vec3::new(0.0, 1.5, 0.0)), ..draw(Vec3::ZERO) };
        let b = Draw { transform: turn * Mat4::from_scale_rotation_translation(Vec3::new(2.0, 3.0, 0.2), Default::default(), Vec3::new(0.0, 1.5, 0.1)), ..draw(Vec3::ZERO) };
        let found = overlaps(&[a, b], |_| Some(&cube), &Limits::default());
        let area: f32 = found.iter().map(|o| o.area).sum();
        // Top, bottom (2 × 0.1) and the two ends (3 × 0.1).
        assert!((area - 1.0).abs() < 0.02, "{found:?}");
    }

    #[test]
    fn ids_round_trip_through_their_colours() {
        for id in [0u32, 1, 2, 31, 32, 1000, 32767] {
            let c = id_color(id);
            let enc = |l: f32| {
                let s = if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
                (s * 255.0).round() as u8
            };
            let px = [enc(c[0]), enc(c[1]), enc(c[2]), 255];
            assert_eq!(decode_ids(&px), vec![id]);
        }
        // Half of one and half of another is nobody's.
        assert_eq!(decode_ids(&[8, 4, 4, 255]), vec![MIXED]);
    }

    #[test]
    fn a_ray_meets_a_quad_where_it_is() {
        let floor = quad(2.0);
        let t = ray_depth(&floor, Mat4::from_translation(Vec3::Y * 0.25), Vec3::new(0.2, 3.0, 0.1), Vec3::NEG_Y);
        assert!((t.unwrap() - 2.75).abs() < 1e-5);
        assert!(ray_depth(&floor, Mat4::IDENTITY, Vec3::new(5.0, 3.0, 0.0), Vec3::NEG_Y).is_none());
    }

    #[test]
    fn places_gather_their_pixels() {
        let mut f = vec![None; 100 * 50];
        for x in 10..20 {
            f[5 * 100 + x] = Some((1, 2));
        }
        f[40 * 100 + 90] = Some((1, 2));
        f[30 * 100 + 50] = Some((3, 4));
        let hot = hot_places(&f, 100, 50, 4);
        assert_eq!(hot.len(), 3);
        assert_eq!((hot[0].ids, hot[0].pixels, hot[0].rect), ((1, 2), 10, (10, 5, 19, 5)));
        let img = heatmap(&vec![128; 100 * 50 * 4], 100, 50, &f.iter().map(Option::is_some).collect::<Vec<_>>(), &[hot[0].rect], &["AB 1".into()]);
        assert_eq!(&img[(5 * 100 + 12) * 4..(5 * 100 + 12) * 4 + 3], &[255, 40, 30]);
    }
}
