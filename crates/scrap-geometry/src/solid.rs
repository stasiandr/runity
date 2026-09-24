//! Convex solids and brush CSG: a box, a wedge, an N-sided prism, cut by
//! planes — what destruction breaks things into, and what a blockout's
//! brushes are made of.
//!
//! Brush CSG as Hammer, TrenchBroom and Godot's CSG nodes do it, in order:
//! [`add`] puts a brush where nothing is yet, [`subtract`] cuts one out
//! of everything added before it. The solid is kept as convex pieces that
//! do not overlap — subtracting a convex brush from a convex piece leaves
//! at most one piece per plane of the brush — and [`boundary`] drops the
//! faces two pieces press together, so what is drawn and collided with
//! is the outside only.

use glam::{Mat4, Vec3};

use crate::mesh_asset::Vertex;

/// A corner this close to a plane is on it: cuts along a face that is
/// already there leave no sliver.
const ON_PLANE: f32 = 1e-5;
/// A piece smaller than this (cubic metres) is nothing.
const NO_VOLUME: f32 = 1e-7;
/// A face smaller than this (square metres) is nothing.
const NO_AREA: f32 = 1e-7;

/// A convex solid: its faces, each a loop of corners turning
/// anticlockwise seen from outside; a face made by a cut is `inner`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Solid {
    pub faces: Vec<Face>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    pub corners: Vec<Vec3>,
    pub inner: bool,
}

impl Face {
    fn outer(corners: Vec<Vec3>) -> Self {
        Face {
            corners,
            inner: false,
        }
    }

    /// Which way it faces, out of the solid (Newell's: a sum over every
    /// edge, so a corner nearly on a line does not tip it).
    pub fn normal(&self) -> Vec3 {
        newell(&self.corners).normalize_or_zero()
    }

    pub fn area(&self) -> f32 {
        newell(&self.corners).length() * 0.5
    }
}

fn newell(corners: &[Vec3]) -> Vec3 {
    let n = corners.len();
    (0..n).fold(Vec3::ZERO, |sum, i| {
        let (a, b) = (corners[i], corners[(i + 1) % n]);
        sum + Vec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y),
        )
    })
}

impl Solid {
    /// The box from `low` to `high`.
    pub fn cuboid(low: Vec3, high: Vec3) -> Self {
        let c = |x: usize, y: usize, z: usize| {
            Vec3::new([low.x, high.x][x], [low.y, high.y][y], [low.z, high.z][z])
        };
        let quad = |a, b, c2, d| Face::outer(vec![a, b, c2, d]);
        Solid {
            faces: vec![
                quad(c(0, 0, 0), c(0, 0, 1), c(0, 1, 1), c(0, 1, 0)),
                quad(c(1, 0, 0), c(1, 1, 0), c(1, 1, 1), c(1, 0, 1)),
                quad(c(0, 0, 0), c(1, 0, 0), c(1, 0, 1), c(0, 0, 1)),
                quad(c(0, 1, 0), c(0, 1, 1), c(1, 1, 1), c(1, 1, 0)),
                quad(c(0, 0, 0), c(0, 1, 0), c(1, 1, 0), c(1, 0, 0)),
                quad(c(0, 0, 1), c(1, 0, 1), c(1, 1, 1), c(0, 1, 1)),
            ],
        }
    }

    /// A wedge in the box from `low` to `high`, high at the back (−z) and
    /// down to nothing at the front — `builtin:ramp`'s shape.
    pub fn wedge(low: Vec3, high: Vec3) -> Self {
        let (b0, b1) = (low, Vec3::new(high.x, low.y, low.z));
        let (b2, b3) = (
            Vec3::new(high.x, low.y, high.z),
            Vec3::new(low.x, low.y, high.z),
        );
        let (t0, t1) = (
            Vec3::new(low.x, high.y, low.z),
            Vec3::new(high.x, high.y, low.z),
        );
        Solid {
            faces: vec![
                Face::outer(vec![b0, b1, b2, b3]),
                Face::outer(vec![b0, t0, t1, b1]),
                Face::outer(vec![b3, b2, t1, t0]),
                Face::outer(vec![b1, t1, b2]),
                Face::outer(vec![b0, b3, t0]),
            ],
        }
    }

    /// A prism of `sides` sides standing on y, from `-half_height` to
    /// `half_height`, its corners `radius` from the axis — a cylinder as
    /// a brush can be one, `builtin:cylinder` at 24 sides.
    pub fn prism(sides: u32, radius: f32, half_height: f32) -> Self {
        let sides = sides.max(3);
        let ring = |y: f32| -> Vec<Vec3> {
            (0..sides)
                .map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / sides as f32;
                    Vec3::new(a.cos() * radius, y, a.sin() * radius)
                })
                .collect()
        };
        let (low, high) = (ring(-half_height), ring(half_height));
        let n = sides as usize;
        let mut faces = vec![
            Face::outer(low.clone()),
            Face::outer(high.iter().rev().copied().collect()),
        ];
        for i in 0..n {
            let j = (i + 1) % n;
            faces.push(Face::outer(vec![low[i], high[i], high[j], low[j]]));
        }
        Solid { faces }
    }

    pub fn is_empty(&self) -> bool {
        self.faces.len() < 4
    }

    /// Its corners, each once or more.
    pub fn corners(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.faces.iter().flat_map(|f| f.corners.iter().copied())
    }

    /// The box it lies in.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.corners().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(a, b), p| (a.min(p), b.max(p)),
        )
    }

    /// Its volume and the middle of it.
    pub fn volume_and_centre(&self) -> (f32, Vec3) {
        let mut volume = 0.0;
        let mut moment = Vec3::ZERO;
        let origin = self
            .faces
            .first()
            .and_then(|f| f.corners.first())
            .copied()
            .unwrap_or(Vec3::ZERO);
        for face in &self.faces {
            for k in 1..face.corners.len().saturating_sub(1) {
                let (a, b, c) = (
                    face.corners[0] - origin,
                    face.corners[k] - origin,
                    face.corners[k + 1] - origin,
                );
                let v = a.dot(b.cross(c)) / 6.0;
                volume += v;
                moment += (a + b + c) / 4.0 * v;
            }
        }
        let centre = if volume.abs() > 1e-12 {
            origin + moment / volume
        } else {
            origin
        };
        (volume, centre)
    }

    /// The same solid moved, turned and stretched by `m`. A mirror turns
    /// every face inside out, so its corners are walked the other way.
    pub fn transformed(&self, m: Mat4) -> Solid {
        let mirror = m.determinant() < 0.0;
        Solid {
            faces: self
                .faces
                .iter()
                .map(|f| {
                    let mut corners: Vec<Vec3> =
                        f.corners.iter().map(|p| m.transform_point3(*p)).collect();
                    if mirror {
                        corners.reverse();
                    }
                    Face {
                        corners,
                        inner: f.inner,
                    }
                })
                .collect(),
        }
    }

    /// The planes of its faces: `(outward normal, offset)`, inside where
    /// `normal · p <= offset`.
    pub fn planes(&self) -> Vec<(Vec3, f32)> {
        self.faces
            .iter()
            .filter_map(|f| {
                let normal = f.normal();
                (normal != Vec3::ZERO).then(|| (normal, normal.dot(middle(&f.corners))))
            })
            .collect()
    }

    /// What is on the side of the plane `normal · p <= offset`, with the
    /// cut closed by an inner face. A plane that cuts nothing away leaves
    /// it as it was; one that keeps nothing leaves nothing.
    pub fn clip(&self, normal: Vec3, offset: f32) -> Solid {
        let side = |p: Vec3| {
            let d = normal.dot(p) - offset;
            if d.abs() < ON_PLANE {
                0.0
            } else {
                d
            }
        };
        if self.corners().all(|p| side(p) <= 0.0) {
            return self.clone();
        }
        if self.corners().all(|p| side(p) >= 0.0) {
            return Solid::default();
        }
        let mut faces = Vec::with_capacity(self.faces.len() + 1);
        let mut cut: Vec<Vec3> = Vec::new();
        for face in &self.faces {
            let n = face.corners.len();
            let mut kept = Vec::with_capacity(n + 2);
            for i in 0..n {
                let (a, b) = (face.corners[i], face.corners[(i + 1) % n]);
                let (da, db) = (side(a), side(b));
                if da <= 0.0 {
                    kept.push(a);
                }
                if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
                    let p = a + (b - a) * (da / (da - db));
                    kept.push(p);
                    cut.push(p);
                } else if da == 0.0 {
                    cut.push(a);
                }
            }
            if kept.len() >= 3 {
                faces.push(Face {
                    corners: kept,
                    inner: face.inner,
                });
            }
        }
        // Close the cut: its corners round their middle, anticlockwise seen
        // from the side cut away.
        let mut unique: Vec<Vec3> = Vec::new();
        for p in cut {
            if !unique.iter().any(|q| q.distance(p) < 1e-6) {
                unique.push(p);
            }
        }
        if unique.len() >= 3 {
            let middle = middle(&unique);
            let u = normal.any_orthonormal_vector();
            let w = normal.cross(u);
            unique.sort_by(|a, b| {
                let (pa, pb) = (*a - middle, *b - middle);
                pa.dot(w)
                    .atan2(pa.dot(u))
                    .total_cmp(&pb.dot(w).atan2(pb.dot(u)))
            });
            faces.push(Face {
                corners: unique,
                inner: true,
            });
        }
        Solid { faces }
    }

    /// What is left of it with `cutter` taken out: no piece, itself (they
    /// do not meet), or up to one convex piece per plane of the cutter.
    pub fn subtract(&self, cutter: &Solid) -> Vec<Solid> {
        if !overlaps(self.bounds(), cutter.bounds()) {
            return vec![self.clone()];
        }
        let planes = cutter.planes();
        let mut common = self.clone();
        for &(normal, offset) in &planes {
            common = common.clip(normal, offset);
            if common.is_empty() {
                break;
            }
        }
        if common.is_empty() || common.volume_and_centre().0 < NO_VOLUME {
            return vec![self.clone()];
        }
        let mut pieces = Vec::new();
        let mut rest = self.clone();
        for (normal, offset) in planes {
            let outside = rest.clip(-normal, -offset);
            if !outside.is_empty() && outside.volume_and_centre().0 >= NO_VOLUME {
                pieces.push(outside);
            }
            rest = rest.clip(normal, offset);
            if rest.is_empty() {
                break;
            }
        }
        pieces
    }

    /// Its triangles, flat-shaded, with pictures laid on by the way each
    /// face looks most (in metres, `scale` being what the solid's space is
    /// stretched by in the world), every corner less `centre`.
    pub fn mesh(&self, centre: Vec3, scale: Vec3) -> (Vec<Vertex>, Vec<u32>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for face in &self.faces {
            let n = face.corners.len();
            if n < 3 {
                continue;
            }
            let normal = (face.corners[1] - face.corners[0])
                .cross(face.corners[2] - face.corners[0])
                .normalize_or(Vec3::Y);
            push_face(
                &mut vertices,
                &mut indices,
                &face.corners,
                normal,
                centre,
                scale,
            );
        }
        (vertices, indices)
    }
}

fn middle(points: &[Vec3]) -> Vec3 {
    points.iter().sum::<Vec3>() / points.len().max(1) as f32
}

fn overlaps((a0, a1): (Vec3, Vec3), (b0, b1): (Vec3, Vec3)) -> bool {
    (a0 - Vec3::splat(ON_PLANE)).cmple(b1).all() && (b0 - Vec3::splat(ON_PLANE)).cmple(a1).all()
}

/// One flat face as a fan of triangles, its pictures laid on by the axis
/// it looks along most.
fn push_face(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    corners: &[Vec3],
    normal: Vec3,
    centre: Vec3,
    scale: Vec3,
) {
    let first = vertices.len() as u32;
    let most = normal.abs();
    for p in corners {
        let m = *p * scale;
        let uv = if most.x >= most.y && most.x >= most.z {
            [m.z, m.y]
        } else if most.y >= most.z {
            [m.x, m.z]
        } else {
            [m.x, m.y]
        };
        vertices.push(Vertex {
            position: (*p - centre).to_array(),
            normal: normal.to_array(),
            uv,
        });
    }
    for k in 1..corners.len() as u32 - 1 {
        indices.extend_from_slice(&[first, first + k, first + k + 1]);
    }
}

/// `brush` added to `pieces`: what of it is not already there joins them,
/// so the pieces still do not overlap.
pub fn add(mut pieces: Vec<Solid>, brush: Solid) -> Vec<Solid> {
    let mut new = vec![brush];
    for piece in &pieces {
        new = new.iter().flat_map(|n| n.subtract(piece)).collect();
        if new.is_empty() {
            break;
        }
    }
    pieces.extend(new);
    pieces
}

/// `cutter` taken out of every piece.
pub fn subtract(pieces: Vec<Solid>, cutter: &Solid) -> Vec<Solid> {
    pieces.iter().flat_map(|p| p.subtract(cutter)).collect()
}

/// The outside of pieces that do not overlap: each face less where another
/// piece's face presses against it, facing the other way. Each part left
/// is convex; T-joins where a big face meets smaller ones stay.
pub fn boundary(pieces: &[Solid]) -> Vec<Face> {
    let bounds: Vec<(Vec3, Vec3)> = pieces.iter().map(Solid::bounds).collect();
    let mut out = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        for face in &piece.faces {
            let normal = face.normal();
            if normal == Vec3::ZERO {
                continue;
            }
            let offset = normal.dot(middle(&face.corners));
            let mut parts = vec![face.corners.clone()];
            for (j, other) in pieces.iter().enumerate() {
                if i == j || !overlaps(bounds[i], bounds[j]) {
                    continue;
                }
                for against in &other.faces {
                    let n = against.normal();
                    if n.dot(normal) > -1.0 + 1e-4
                        || (n.dot(middle(&against.corners)) + offset).abs() > ON_PLANE * 10.0
                    {
                        continue;
                    }
                    parts = parts
                        .iter()
                        .flat_map(|p| polygon_minus(p, &against.corners, n))
                        .collect();
                }
                if parts.is_empty() {
                    break;
                }
            }
            out.extend(parts.into_iter().map(|corners| Face {
                corners,
                inner: face.inner,
            }));
        }
    }
    out
}

/// The part of a flat polygon on the side `normal · p <= offset`.
fn clip_polygon(corners: &[Vec3], normal: Vec3, offset: f32) -> Vec<Vec3> {
    let side = |p: Vec3| {
        let d = normal.dot(p) - offset;
        if d.abs() < ON_PLANE {
            0.0
        } else {
            d
        }
    };
    let n = corners.len();
    let mut kept = Vec::with_capacity(n + 1);
    for i in 0..n {
        let (a, b) = (corners[i], corners[(i + 1) % n]);
        let (da, db) = (side(a), side(b));
        if da <= 0.0 {
            kept.push(a);
        }
        if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
            kept.push(a + (b - a) * (da / (da - db)));
        }
    }
    kept
}

fn polygon_area(corners: &[Vec3]) -> f32 {
    newell(corners).length() * 0.5
}

/// A flat convex polygon less the convex polygon `hole` lying in its plane
/// (`hole` facing `hole_normal`): up to one convex part per edge of the hole.
fn polygon_minus(polygon: &[Vec3], hole: &[Vec3], hole_normal: Vec3) -> Vec<Vec<Vec3>> {
    let n = hole.len();
    // Each edge's plane, across the face: outward, away from the hole.
    let edges: Vec<(Vec3, f32)> = (0..n)
        .filter_map(|i| {
            let (a, b) = (hole[i], hole[(i + 1) % n]);
            let out = (b - a).cross(hole_normal).normalize_or_zero();
            (out != Vec3::ZERO).then(|| (out, out.dot(a)))
        })
        .collect();
    let mut common = polygon.to_vec();
    for &(normal, offset) in &edges {
        common = clip_polygon(&common, normal, offset);
        if common.len() < 3 {
            break;
        }
    }
    if common.len() < 3 || polygon_area(&common) < NO_AREA {
        return vec![polygon.to_vec()];
    }
    let mut parts = Vec::new();
    let mut rest = polygon.to_vec();
    for (normal, offset) in edges {
        let outside = clip_polygon(&rest, -normal, -offset);
        if outside.len() >= 3 && polygon_area(&outside) >= NO_AREA {
            parts.push(outside);
        }
        rest = clip_polygon(&rest, normal, offset);
        if rest.len() < 3 {
            break;
        }
    }
    parts
}

/// Faces as triangles, flat-shaded, with pictures in metres laid on by the
/// axis each face looks along most — a grid lines up across every face.
pub fn mesh_of(faces: &[Face]) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for face in faces {
        let normal = face.normal();
        if face.corners.len() < 3 || normal == Vec3::ZERO {
            continue;
        }
        push_face(
            &mut vertices,
            &mut indices,
            &face.corners,
            normal,
            Vec3::ZERO,
            Vec3::ONE,
        );
    }
    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(pieces: &[Solid]) -> f32 {
        pieces.iter().map(|p| p.volume_and_centre().0).sum()
    }

    /// Volume and area from the triangles alone: they close a solid only
    /// if the volume comes out as the pieces say.
    fn mesh_volume_and_area(vertices: &[Vertex], indices: &[u32]) -> (f32, f32) {
        let mut v = 0.0;
        let mut a = 0.0;
        for t in indices.chunks(3) {
            let [p, q, r] = [t[0], t[1], t[2]].map(|i| Vec3::from(vertices[i as usize].position));
            v += p.dot(q.cross(r)) / 6.0;
            a += (q - p).cross(r - p).length() * 0.5;
        }
        (v, a)
    }

    #[test]
    fn the_shapes_are_closed_and_face_out() {
        for (solid, want) in [
            (Solid::cuboid(Vec3::splat(-0.5), Vec3::splat(0.5)), 1.0),
            (Solid::wedge(Vec3::splat(-0.5), Vec3::splat(0.5)), 0.5),
            (
                Solid::prism(64, 0.5, 0.5),
                // A 64-gon is nearly the circle.
                0.5 * 64.0 * 0.25 * (std::f32::consts::TAU / 64.0).sin(),
            ),
        ] {
            let (v, centre) = solid.volume_and_centre();
            assert!((v - want).abs() < 1e-4, "{v} ≠ {want}");
            for (normal, offset) in solid.planes() {
                assert!(normal.dot(centre) < offset, "a face turned in");
            }
        }
    }

    #[test]
    fn a_box_through_a_box_is_a_tunnel() {
        let wall = Solid::cuboid(Vec3::splat(-2.0), Vec3::splat(2.0));
        let hole = Solid::cuboid(Vec3::new(-1.0, -1.0, -3.0), Vec3::new(1.0, 1.0, 3.0));
        let pieces = subtract(vec![wall], &hole);
        assert!(pieces.len() <= 6, "{} pieces", pieces.len());
        assert!((volume(&pieces) - (64.0 - 16.0)).abs() < 1e-3);
        // Nothing of it where the tunnel runs.
        for p in &pieces {
            let (low, high) = p.bounds();
            let inside = low.x < 1.0 - 1e-4
                && high.x > -1.0 + 1e-4
                && low.y < 1.0 - 1e-4
                && high.y > -1.0 + 1e-4;
            assert!(!inside, "a piece in the tunnel: {low} {high}");
        }
        let faces = boundary(&pieces);
        let (vertices, indices) = mesh_of(&faces);
        let (v, a) = mesh_volume_and_area(&vertices, &indices);
        // The outside only: six sides of 16 less two doorways of 4, and the
        // tunnel's four walls, 2 by 4.
        assert!((a - (96.0 - 8.0 + 32.0)).abs() < 1e-2, "area {a}");
        // Closed, facing out: the triangles hold the volume.
        assert!((v - 48.0).abs() < 1e-2, "volume {v}");
    }

    #[test]
    fn a_doorway_cut_from_the_bottom_edge_and_order_matters() {
        // A wall 4 wide, 3 high, 0.25 thick; a door 1 by 2 from the floor.
        let wall = Solid::cuboid(Vec3::new(-2.0, 0.0, -0.125), Vec3::new(2.0, 3.0, 0.125));
        let door = Solid::cuboid(Vec3::new(-0.5, -0.5, -1.0), Vec3::new(0.5, 2.0, 1.0));
        let cut = subtract(vec![wall.clone()], &door);
        assert!((volume(&cut) - (12.0 - 2.0) * 0.25).abs() < 1e-4);
        let (vertices, indices) = mesh_of(&boundary(&cut));
        assert!((mesh_volume_and_area(&vertices, &indices).0 - 2.5).abs() < 1e-3);
        // Adding after the cut fills it again; before, the cut wins.
        let filled = add(
            cut,
            Solid::cuboid(Vec3::new(-2.0, 0.0, -0.125), Vec3::new(2.0, 1.0, 0.125)),
        );
        assert!(
            (volume(&filled) - (2.5 + 0.25)).abs() < 1e-4,
            "{}",
            volume(&filled)
        );
    }

    #[test]
    fn overlapping_adds_count_once_and_keep_no_inner_faces() {
        let a = Solid::cuboid(Vec3::ZERO, Vec3::splat(2.0));
        let b = Solid::cuboid(Vec3::ONE, Vec3::splat(3.0));
        let pieces = add(add(Vec::new(), a), b);
        assert!((volume(&pieces) - (8.0 + 8.0 - 1.0)).abs() < 1e-4);
        let (vertices, indices) = mesh_of(&boundary(&pieces));
        let (v, area) = mesh_volume_and_area(&vertices, &indices);
        assert!((v - 15.0).abs() < 1e-3, "{v}");
        // Two boxes' 24 each, less the three faces of 1 each hides in the other.
        assert!((area - (48.0 - 6.0)).abs() < 1e-3, "{area}");
    }

    #[test]
    fn a_cutter_that_misses_changes_nothing_and_one_that_covers_leaves_nothing() {
        let a = Solid::cuboid(Vec3::ZERO, Vec3::ONE);
        let far = Solid::cuboid(Vec3::splat(5.0), Vec3::splat(6.0));
        assert_eq!(a.subtract(&far), vec![a.clone()]);
        // Touching along a face is not meeting.
        let beside = Solid::cuboid(Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 1.0));
        assert_eq!(a.subtract(&beside), vec![a.clone()]);
        let over = Solid::cuboid(Vec3::splat(-1.0), Vec3::splat(2.0));
        assert!(a.subtract(&over).is_empty());
    }

    #[test]
    fn a_turned_prism_cuts_a_round_window() {
        let wall = Solid::cuboid(Vec3::new(-2.0, 0.0, -0.25), Vec3::new(2.0, 3.0, 0.25));
        // A cylinder lying along z, through the wall.
        let window = Solid::prism(16, 0.5, 1.0).transformed(
            Mat4::from_translation(Vec3::new(0.0, 1.5, 0.0))
                * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2),
        );
        let pieces = subtract(vec![wall], &window);
        let hole = 0.5 * 16.0 * 0.25 * (std::f32::consts::TAU / 16.0).sin() * 0.5;
        assert!((volume(&pieces) - (6.0 - hole)).abs() < 1e-3);
        let (vertices, indices) = mesh_of(&boundary(&pieces));
        assert!((mesh_volume_and_area(&vertices, &indices).0 - (6.0 - hole)).abs() < 1e-2);
    }

    #[test]
    fn a_mirror_keeps_faces_out() {
        let wedge = Solid::wedge(Vec3::splat(-0.5), Vec3::splat(0.5))
            .transformed(Mat4::from_scale(Vec3::new(-2.0, 1.0, 1.0)));
        let (v, _) = wedge.volume_and_centre();
        assert!((v - 1.0).abs() < 1e-5, "{v}");
    }
}
