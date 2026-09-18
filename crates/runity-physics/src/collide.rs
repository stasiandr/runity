//! Narrow phase: given two shapes that might be touching, work out where and
//! how deeply.
//!
//! Every pair is handled analytically rather than by a general convex
//! algorithm. That is more code, but it is code whose failure modes are
//! understandable: a sphere-sphere test cannot produce a wrong normal, and a
//! box-box test that does can be debugged by looking at which separating axis
//! won.
//!
//! **Conventions.** The normal points from A towards B, and pushing B along it
//! separates them. Penetration is positive when the shapes overlap.

use crate::shape::{Isometry, Shape};
use runity_math::{Mat3, Vec3};

/// Contacts are matched between frames by this id, so accumulated impulses
/// survive — which is what keeps a stack of boxes from sinking.
pub type FeatureId = u32;

/// The most contact points one manifold can hold. Four is enough for a face
/// against a face, which is the worst case for a convex pair.
pub const MAX_CONTACTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactPoint {
    /// World-space position, halfway between the two surfaces.
    pub position: Vec3,
    /// How far the shapes overlap along the manifold normal.
    pub penetration: f32,
    pub feature: FeatureId,
}

/// Where and how two shapes touch.
#[derive(Debug, Clone, Copy)]
pub struct Manifold {
    /// Unit vector from A to B.
    pub normal: Vec3,
    pub points: [ContactPoint; MAX_CONTACTS],
    pub count: usize,
}

impl Manifold {
    fn new(normal: Vec3) -> Self {
        Self {
            normal,
            points: [ContactPoint {
                position: Vec3::ZERO,
                penetration: 0.0,
                feature: 0,
            }; MAX_CONTACTS],
            count: 0,
        }
    }

    fn push(&mut self, position: Vec3, penetration: f32, feature: FeatureId) {
        if self.count < MAX_CONTACTS {
            self.points[self.count] = ContactPoint {
                position,
                penetration,
                feature,
            };
            self.count += 1;
        }
    }

    pub fn contacts(&self) -> &[ContactPoint] {
        &self.points[..self.count]
    }

    /// Swap the roles of A and B: the normal flips, the points stay.
    fn flipped(mut self) -> Self {
        self.normal = -self.normal;
        self
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// Test two placed shapes. `None` when they are apart.
pub fn collide(
    shape_a: &Shape,
    at_a: &Isometry,
    shape_b: &Shape,
    at_b: &Isometry,
) -> Option<Manifold> {
    use Shape::*;
    let manifold = match (shape_a, shape_b) {
        (Sphere { radius: ra }, Sphere { radius: rb }) => sphere_sphere(at_a, *ra, at_b, *rb),
        (Sphere { radius }, HalfSpace { normal }) => sphere_halfspace(at_a, *radius, at_b, *normal),
        (HalfSpace { normal }, Sphere { radius }) => {
            sphere_halfspace(at_b, *radius, at_a, *normal).map(Manifold::flipped)
        }
        (Sphere { radius }, Cuboid { half_extents }) => {
            sphere_cuboid(at_a, *radius, at_b, *half_extents)
        }
        (Cuboid { half_extents }, Sphere { radius }) => {
            sphere_cuboid(at_b, *radius, at_a, *half_extents).map(Manifold::flipped)
        }
        (
            Sphere { radius },
            Capsule {
                half_height,
                radius: cr,
            },
        ) => sphere_capsule(at_a, *radius, at_b, *half_height, *cr),
        (
            Capsule {
                half_height,
                radius: cr,
            },
            Sphere { radius },
        ) => sphere_capsule(at_b, *radius, at_a, *half_height, *cr).map(Manifold::flipped),
        (
            Capsule {
                half_height,
                radius,
            },
            HalfSpace { normal },
        ) => capsule_halfspace(at_a, *half_height, *radius, at_b, *normal),
        (
            HalfSpace { normal },
            Capsule {
                half_height,
                radius,
            },
        ) => capsule_halfspace(at_b, *half_height, *radius, at_a, *normal).map(Manifold::flipped),
        (
            Capsule {
                half_height: ha,
                radius: ra,
            },
            Capsule {
                half_height: hb,
                radius: rb,
            },
        ) => capsule_capsule(at_a, *ha, *ra, at_b, *hb, *rb),
        (
            Capsule {
                half_height,
                radius,
            },
            Cuboid { half_extents },
        ) => capsule_cuboid(at_a, *half_height, *radius, at_b, *half_extents),
        (
            Cuboid { half_extents },
            Capsule {
                half_height,
                radius,
            },
        ) => {
            capsule_cuboid(at_b, *half_height, *radius, at_a, *half_extents).map(Manifold::flipped)
        }
        (Cuboid { half_extents }, HalfSpace { normal }) => {
            cuboid_halfspace(at_a, *half_extents, at_b, *normal)
        }
        (HalfSpace { normal }, Cuboid { half_extents }) => {
            cuboid_halfspace(at_b, *half_extents, at_a, *normal).map(Manifold::flipped)
        }
        (Cuboid { half_extents: ha }, Cuboid { half_extents: hb }) => {
            cuboid_cuboid(at_a, *ha, at_b, *hb)
        }
        // Two infinite planes never usefully collide.
        (HalfSpace { .. }, HalfSpace { .. }) => None,
    };
    manifold.filter(|m| !m.is_empty())
}

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

/// The point on segment `a`-`b` closest to `p`.
pub fn closest_point_on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let ab = b - a;
    let length_squared = ab.length_squared();
    if length_squared <= 1e-12 {
        return a;
    }
    let t = ((p - a).dot(ab) / length_squared).clamp(0.0, 1.0);
    a + ab * t
}

/// Closest pair of points between two segments.
///
/// The standard clamped-parameter solution (Ericson, *Real-Time Collision
/// Detection*): solve the unconstrained system, then clamp each parameter into
/// `[0, 1]` and re-solve the other.
pub fn closest_points_between_segments(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.length_squared();
    let e = d2.length_squared();
    let f = d2.dot(r);

    // Degenerate cases: one or both segments are points.
    if a <= 1e-12 && e <= 1e-12 {
        return (p1, p2);
    }
    if a <= 1e-12 {
        return (p1, closest_point_on_segment(p1, p2, q2));
    }
    if e <= 1e-12 {
        return (closest_point_on_segment(p2, p1, q1), p2);
    }

    let c = d1.dot(r);
    let b = d1.dot(d2);
    let denominator = a * e - b * b;
    // Parallel segments leave s free; picking 0 is as good as anything.
    let mut s = if denominator > 1e-12 {
        ((b * f - c * e) / denominator).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut t = (b * s + f) / e;

    if t < 0.0 {
        t = 0.0;
        s = (-c / a).clamp(0.0, 1.0);
    } else if t > 1.0 {
        t = 1.0;
        s = ((b - c) / a).clamp(0.0, 1.0);
    }
    (p1 + d1 * s, p2 + d2 * t)
}

/// Endpoints of a capsule's inner segment, in world space.
fn capsule_segment(at: &Isometry, half_height: f32) -> (Vec3, Vec3) {
    let axis = at.transform_direction(Vec3::Y) * half_height;
    (at.position - axis, at.position + axis)
}

/// The point of a box closest to a world-space point, and whether that point is
/// inside the box.
fn closest_point_on_cuboid(point: Vec3, at: &Isometry, half: Vec3) -> (Vec3, bool) {
    let local = at.inverse_transform_point(point);
    let clamped = Vec3::new(
        local.x.clamp(-half.x, half.x),
        local.y.clamp(-half.y, half.y),
        local.z.clamp(-half.z, half.z),
    );
    (at.transform_point(clamped), clamped == local)
}

/// For a point already inside a box: the face it is closest to, as a local-space
/// normal and the distance to it.
fn deepest_face(local: Vec3, half: Vec3) -> (Vec3, f32) {
    let distances = [
        (
            half.x - local.x.abs(),
            Vec3::new(local.x.signum(), 0.0, 0.0),
        ),
        (
            half.y - local.y.abs(),
            Vec3::new(0.0, local.y.signum(), 0.0),
        ),
        (
            half.z - local.z.abs(),
            Vec3::new(0.0, 0.0, local.z.signum()),
        ),
    ];
    let mut best = distances[0];
    for candidate in &distances[1..] {
        if candidate.0 < best.0 {
            best = *candidate;
        }
    }
    (best.1, best.0)
}

// ---------------------------------------------------------------------------
// Pairs
// ---------------------------------------------------------------------------

fn sphere_sphere(at_a: &Isometry, ra: f32, at_b: &Isometry, rb: f32) -> Option<Manifold> {
    let offset = at_b.position - at_a.position;
    let distance = offset.length();
    let radii = ra + rb;
    if distance > radii {
        return None;
    }
    // Concentric spheres have no meaningful normal; pick one rather than divide
    // by zero.
    let normal = if distance > 1e-6 {
        offset * (1.0 / distance)
    } else {
        Vec3::Y
    };
    let mut manifold = Manifold::new(normal);
    let on_a = at_a.position + normal * ra;
    let on_b = at_b.position - normal * rb;
    manifold.push((on_a + on_b) * 0.5, radii - distance, 0);
    Some(manifold)
}

fn sphere_halfspace(
    at_sphere: &Isometry,
    radius: f32,
    at_plane: &Isometry,
    plane_normal: Vec3,
) -> Option<Manifold> {
    let normal = at_plane.transform_direction(plane_normal).normalized();
    let distance = (at_sphere.position - at_plane.position).dot(normal) - radius;
    if distance > 0.0 {
        return None;
    }
    // The plane is B, so the normal has to point from the sphere towards it.
    let mut manifold = Manifold::new(-normal);
    manifold.push(
        at_sphere.position - normal * (radius + distance * 0.5),
        -distance,
        0,
    );
    Some(manifold)
}

fn sphere_cuboid(
    at_sphere: &Isometry,
    radius: f32,
    at_box: &Isometry,
    half: Vec3,
) -> Option<Manifold> {
    let (closest, inside) = closest_point_on_cuboid(at_sphere.position, at_box, half);

    if inside {
        // The center is in the box: push out through the nearest face.
        let local = at_box.inverse_transform_point(at_sphere.position);
        let (local_normal, distance) = deepest_face(local, half);
        let normal = at_box.transform_direction(local_normal);
        let mut manifold = Manifold::new(normal);
        manifold.push(at_sphere.position, distance + radius, 0);
        return Some(manifold);
    }

    let offset = closest - at_sphere.position;
    let distance = offset.length();
    if distance > radius {
        return None;
    }
    let normal = if distance > 1e-6 {
        offset * (1.0 / distance)
    } else {
        Vec3::Y
    };
    let mut manifold = Manifold::new(normal);
    manifold.push(closest, radius - distance, 0);
    Some(manifold)
}

fn sphere_capsule(
    at_sphere: &Isometry,
    radius: f32,
    at_capsule: &Isometry,
    half_height: f32,
    capsule_radius: f32,
) -> Option<Manifold> {
    let (a, b) = capsule_segment(at_capsule, half_height);
    let closest = closest_point_on_segment(at_sphere.position, a, b);
    // A capsule is a sphere swept along a segment, so this reduces exactly.
    sphere_sphere(
        at_sphere,
        radius,
        &Isometry::from_position(closest),
        capsule_radius,
    )
}

fn capsule_halfspace(
    at_capsule: &Isometry,
    half_height: f32,
    radius: f32,
    at_plane: &Isometry,
    plane_normal: Vec3,
) -> Option<Manifold> {
    let normal = at_plane.transform_direction(plane_normal).normalized();
    let (a, b) = capsule_segment(at_capsule, half_height);
    let mut manifold = Manifold::new(-normal);

    // Both ends can touch, and a capsule lying flat needs both of them to rest
    // still instead of rocking.
    for (index, end) in [a, b].into_iter().enumerate() {
        let distance = (end - at_plane.position).dot(normal) - radius;
        if distance <= 0.0 {
            manifold.push(
                end - normal * (radius + distance * 0.5),
                -distance,
                index as u32,
            );
        }
    }
    (!manifold.is_empty()).then_some(manifold)
}

fn capsule_capsule(
    at_a: &Isometry,
    half_a: f32,
    radius_a: f32,
    at_b: &Isometry,
    half_b: f32,
    radius_b: f32,
) -> Option<Manifold> {
    let (a1, a2) = capsule_segment(at_a, half_a);
    let (b1, b2) = capsule_segment(at_b, half_b);
    let (p, q) = closest_points_between_segments(a1, a2, b1, b2);
    sphere_sphere(
        &Isometry::from_position(p),
        radius_a,
        &Isometry::from_position(q),
        radius_b,
    )
}

fn capsule_cuboid(
    at_capsule: &Isometry,
    half_height: f32,
    radius: f32,
    at_box: &Isometry,
    half: Vec3,
) -> Option<Manifold> {
    let (a, b) = capsule_segment(at_capsule, half_height);

    // Alternate between "closest point on the box to the segment" and "closest
    // point on the segment to the box" — a couple of rounds is enough for the
    // shapes this crate has, and it degrades to the exact answer when the
    // segment is parallel to a face.
    let mut on_segment = (a + b) * 0.5;
    for _ in 0..4 {
        let on_box = closest_point_on_cuboid(on_segment, at_box, half).0;
        on_segment = closest_point_on_segment(on_box, a, b);
    }

    let first = sphere_cuboid(&Isometry::from_position(on_segment), radius, at_box, half)?;

    // A capsule lying along a face needs a second point, or it will rock.
    let axis = (b - a).normalized();
    let mut manifold = first;
    if axis.dot(manifold.normal).abs() < 0.2 {
        let ends = [(a, 1u32), (b, 2u32)];
        for (end, feature) in ends {
            if (end - on_segment).length() < 1e-4 {
                continue;
            }
            if let Some(extra) = sphere_cuboid(&Isometry::from_position(end), radius, at_box, half)
            {
                // Keep the manifold's own normal: mixing normals in one
                // manifold makes the solver fight itself.
                let point = extra.points[0];
                manifold.push(point.position, point.penetration, feature);
            }
        }
    }
    Some(manifold)
}

fn cuboid_halfspace(
    at_box: &Isometry,
    half: Vec3,
    at_plane: &Isometry,
    plane_normal: Vec3,
) -> Option<Manifold> {
    let normal = at_plane.transform_direction(plane_normal).normalized();
    let mut manifold = Manifold::new(-normal);

    // Every corner below the plane is a contact; the four deepest are enough.
    let mut corners: [(f32, Vec3, u32); 8] = [(0.0, Vec3::ZERO, 0); 8];
    for (index, slot) in corners.iter_mut().enumerate() {
        // The three bits of the index pick a sign per axis.
        let corner = at_box.transform_point(Vec3::new(
            if index & 1 == 0 { -half.x } else { half.x },
            if index & 2 == 0 { -half.y } else { half.y },
            if index & 4 == 0 { -half.z } else { half.z },
        ));
        let distance = (corner - at_plane.position).dot(normal);
        *slot = (distance, corner, index as u32);
    }
    corners.sort_by(|a, b| a.0.total_cmp(&b.0));

    for (distance, corner, feature) in corners {
        if distance > 0.0 || manifold.count == MAX_CONTACTS {
            break;
        }
        manifold.push(corner - normal * (distance * 0.5), -distance, feature);
    }
    (!manifold.is_empty()).then_some(manifold)
}

/// One candidate separating axis and how much the boxes overlap along it.
struct Separation {
    axis: Vec3,
    overlap: f32,
    /// Which of the 15 axes this is, for feature ids.
    index: usize,
}

fn project_cuboid(half: Vec3, rotation: &Mat3, axis: Vec3) -> f32 {
    half.x * rotation.cols[0].dot(axis).abs()
        + half.y * rotation.cols[1].dot(axis).abs()
        + half.z * rotation.cols[2].dot(axis).abs()
}

fn cuboid_cuboid(at_a: &Isometry, half_a: Vec3, at_b: &Isometry, half_b: Vec3) -> Option<Manifold> {
    let rotation_a = Mat3::from_quat(at_a.rotation);
    let rotation_b = Mat3::from_quat(at_b.rotation);
    let offset = at_b.position - at_a.position;

    // The separating axis theorem: two convex shapes are apart if and only if
    // some axis separates their projections. For boxes, fifteen candidates are
    // enough — the six face normals and the nine edge-edge cross products.
    let mut best: Option<Separation> = None;
    let mut consider = |axis: Vec3, index: usize| -> bool {
        let length = axis.length();
        if length < 1e-6 {
            return true; // parallel edges: this axis says nothing
        }
        let axis = axis * (1.0 / length);
        let overlap = project_cuboid(half_a, &rotation_a, axis)
            + project_cuboid(half_b, &rotation_b, axis)
            - offset.dot(axis).abs();
        if overlap < 0.0 {
            return false; // found a separating axis: the boxes are apart
        }
        if best.as_ref().map_or(true, |b| overlap < b.overlap) {
            // Keep the normal pointing from A to B.
            let signed = if offset.dot(axis) < 0.0 { -axis } else { axis };
            best = Some(Separation {
                axis: signed,
                overlap,
                index,
            });
        }
        true
    };

    for i in 0..3 {
        if !consider(rotation_a.cols[i], i) {
            return None;
        }
    }
    for i in 0..3 {
        if !consider(rotation_b.cols[i], 3 + i) {
            return None;
        }
    }
    for i in 0..3 {
        for j in 0..3 {
            if !consider(rotation_a.cols[i].cross(rotation_b.cols[j]), 6 + i * 3 + j) {
                return None;
            }
        }
    }

    let separation = best?;
    let normal = separation.axis;

    // Face contact: clip the incident face against the reference face. Edge
    // contacts (axes 6..15) get a single point, which is all an edge has.
    if separation.index < 6 {
        let (reference_at, reference_half, incident_at, incident_half, flip) =
            if separation.index < 3 {
                (at_a, half_a, at_b, half_b, false)
            } else {
                (at_b, half_b, at_a, half_a, true)
            };
        let reference_normal = if flip { -normal } else { normal };
        if let Some(manifold) = clip_faces(
            reference_at,
            reference_half,
            reference_normal,
            incident_at,
            incident_half,
            normal,
            separation.overlap,
        ) {
            return Some(manifold);
        }
    }

    // Fallback, and the right answer for an edge-edge crossing: one point
    // midway between the closest features.
    let point_on_a = at_a.transform_point(support_local(half_a, &rotation_a, normal));
    let point_on_b = at_b.transform_point(support_local(half_b, &rotation_b, -normal));
    let mut manifold = Manifold::new(normal);
    manifold.push(
        (point_on_a + point_on_b) * 0.5,
        separation.overlap,
        separation.index as u32,
    );
    Some(manifold)
}

/// The corner of a box furthest along a world-space direction, in local space.
fn support_local(half: Vec3, rotation: &Mat3, direction: Vec3) -> Vec3 {
    Vec3::new(
        half.x.copysign(rotation.cols[0].dot(direction)),
        half.y.copysign(rotation.cols[1].dot(direction)),
        half.z.copysign(rotation.cols[2].dot(direction)),
    )
}

/// Clip the incident box's most-facing face against the reference face's side
/// planes, keeping whatever ends up below the reference plane.
#[allow(clippy::too_many_arguments)]
fn clip_faces(
    reference_at: &Isometry,
    reference_half: Vec3,
    reference_normal: Vec3,
    incident_at: &Isometry,
    incident_half: Vec3,
    manifold_normal: Vec3,
    overlap: f32,
) -> Option<Manifold> {
    // Work in the reference box's local frame, where its face is axis aligned.
    let local_normal = reference_at.inverse_transform_direction(reference_normal);
    let axis = dominant_axis(local_normal)?;
    let sign = local_normal[axis].signum();

    // The incident face is the one whose normal is most opposed.
    let incident_rotation = Mat3::from_quat(incident_at.rotation);
    let mut incident_axis = 0;
    let mut incident_sign = 1.0;
    let mut best = f32::INFINITY;
    for i in 0..3 {
        let dot = incident_rotation.cols[i].dot(reference_normal);
        if dot < best {
            best = dot;
            incident_axis = i;
            incident_sign = 1.0;
        }
        if -dot < best {
            best = -dot;
            incident_axis = i;
            incident_sign = -1.0;
        }
    }

    // Corners of the incident face, in the reference box's local frame.
    let (u, v) = other_axes(incident_axis);
    let mut polygon: Vec<Vec3> = Vec::with_capacity(4);
    for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let mut corner = Vec3::ZERO;
        set_axis(
            &mut corner,
            incident_axis,
            incident_half[incident_axis] * incident_sign,
        );
        set_axis(&mut corner, u, incident_half[u] * su);
        set_axis(&mut corner, v, incident_half[v] * sv);
        let world = incident_at.transform_point(corner);
        polygon.push(reference_at.inverse_transform_point(world));
    }

    // Clip against the four side planes of the reference face.
    let (ru, rv) = other_axes(axis);
    for (clip_axis, extent) in [(ru, reference_half[ru]), (rv, reference_half[rv])] {
        polygon = clip_to_slab(&polygon, clip_axis, extent);
        if polygon.is_empty() {
            return None;
        }
    }

    let face_offset = reference_half[axis] * sign;
    let mut manifold = Manifold::new(manifold_normal);
    for (index, point) in polygon.iter().enumerate() {
        // Distance below the reference face, along its own normal.
        let depth = (face_offset - point[axis]) * sign;
        if depth < -1e-4 {
            continue;
        }
        if manifold.count == MAX_CONTACTS {
            break;
        }
        let world = reference_at.transform_point(*point);
        manifold.push(world, depth.max(0.0).min(overlap + 1e-3), index as u32);
    }
    (!manifold.is_empty()).then_some(manifold)
}

fn dominant_axis(v: Vec3) -> Option<usize> {
    let (x, y, z) = (v.x.abs(), v.y.abs(), v.z.abs());
    if x >= y && x >= z {
        Some(0)
    } else if y >= z {
        Some(1)
    } else {
        Some(2)
    }
}

fn other_axes(axis: usize) -> (usize, usize) {
    match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

#[inline]
fn set_axis(v: &mut Vec3, axis: usize, value: f32) {
    match axis {
        0 => v.x = value,
        1 => v.y = value,
        _ => v.z = value,
    }
}

/// Sutherland-Hodgman clipping of a polygon to the slab `|p[axis]| <= extent`.
fn clip_to_slab(polygon: &[Vec3], axis: usize, extent: f32) -> Vec<Vec3> {
    let above = clip_to_plane(polygon, axis, extent, -1.0);
    clip_to_plane(&above, axis, -extent, 1.0)
}

/// Keep the part of the polygon on one side of an axis-aligned plane.
///
/// `side` is +1 to keep `p[axis] >= bound` and -1 to keep `p[axis] <= bound`;
/// edges that cross the plane get a new vertex on it.
fn clip_to_plane(polygon: &[Vec3], axis: usize, bound: f32, side: f32) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(polygon.len() + 2);
    if polygon.is_empty() {
        return out;
    }
    let distance = |p: &Vec3| (p[axis] - bound) * side;
    for i in 0..polygon.len() {
        let current = polygon[i];
        let next = polygon[(i + 1) % polygon.len()];
        let d_current = distance(&current);
        let d_next = distance(&next);
        if d_current >= 0.0 {
            out.push(current);
        }
        if (d_current >= 0.0) != (d_next >= 0.0) {
            let t = d_current / (d_current - d_next);
            out.push(current + (next - current) * t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::Quat;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    fn at(x: f32, y: f32, z: f32) -> Isometry {
        Isometry::from_position(Vec3::new(x, y, z))
    }

    fn turned(position: Vec3, axis: Vec3, angle: f32) -> Isometry {
        Isometry::new(position, Quat::from_axis_angle(axis, angle))
    }

    #[test]
    fn two_spheres_touch_along_the_line_between_them() {
        let a = Shape::sphere(1.0);
        let b = Shape::sphere(0.5);

        assert!(collide(&a, &at(0.0, 0.0, 0.0), &b, &at(3.0, 0.0, 0.0)).is_none());

        let manifold = collide(&a, &at(0.0, 0.0, 0.0), &b, &at(1.2, 0.0, 0.0)).expect("touching");
        assert!(
            (manifold.normal - Vec3::X).length() < 1e-5,
            "{:?}",
            manifold.normal
        );
        assert_eq!(manifold.count, 1);
        assert!((manifold.points[0].penetration - 0.3).abs() < 1e-5);
        // The contact sits halfway between the two surfaces: A's at x = 1.0,
        // B's at x = 0.7.
        assert!((manifold.points[0].position.x - 0.85).abs() < 1e-4);
    }

    #[test]
    fn concentric_spheres_still_produce_a_usable_normal() {
        let a = Shape::sphere(1.0);
        let manifold = collide(&a, &at(0.0, 0.0, 0.0), &a, &at(0.0, 0.0, 0.0)).expect("overlap");
        assert!(
            (manifold.normal.length() - 1.0).abs() < 1e-5,
            "not NaN: {:?}",
            manifold.normal
        );
        assert!(manifold.points[0].penetration > 1.9);
    }

    #[test]
    fn a_sphere_rests_on_the_ground() {
        let sphere = Shape::sphere(0.5);
        let ground = Shape::ground();

        assert!(collide(&sphere, &at(0.0, 0.6, 0.0), &ground, &Isometry::IDENTITY).is_none());

        let manifold =
            collide(&sphere, &at(0.0, 0.3, 0.0), &ground, &Isometry::IDENTITY).expect("touching");
        // A is the sphere, B is the ground below it.
        assert!(
            (manifold.normal - (-Vec3::Y)).length() < 1e-5,
            "{:?}",
            manifold.normal
        );
        assert!((manifold.points[0].penetration - 0.2).abs() < 1e-5);

        // The reversed pair reports the opposite normal and the same depth.
        let flipped =
            collide(&ground, &Isometry::IDENTITY, &sphere, &at(0.0, 0.3, 0.0)).expect("touching");
        assert!((flipped.normal - Vec3::Y).length() < 1e-5);
        assert!((flipped.points[0].penetration - 0.2).abs() < 1e-5);
    }

    #[test]
    fn a_sphere_against_a_box_face_edge_and_corner() {
        let sphere = Shape::sphere(0.5);
        let cuboid = Shape::cuboid(Vec3::ONE);

        // Face: the normal is the face normal, pointing from the sphere (A)
        // towards the box (B) — so back along -X.
        let face = collide(&sphere, &at(1.4, 0.0, 0.0), &cuboid, &Isometry::IDENTITY)
            .expect("touching the face");
        assert!((face.normal + Vec3::X).length() < 1e-4, "{:?}", face.normal);
        assert!((face.points[0].penetration - 0.1).abs() < 1e-4);

        // Corner: the normal points back along the diagonal.
        let offset = 1.0 + 0.2;
        let corner = collide(
            &sphere,
            &at(offset, offset, offset),
            &cuboid,
            &Isometry::IDENTITY,
        )
        .expect("touching the corner");
        let expected = -Vec3::splat(1.0).normalized();
        assert!(
            (corner.normal - expected).length() < 1e-4,
            "{:?}",
            corner.normal
        );

        // Well away: nothing.
        assert!(collide(&sphere, &at(2.0, 2.0, 2.0), &cuboid, &Isometry::IDENTITY).is_none());
    }

    #[test]
    fn a_sphere_inside_a_box_is_pushed_out_the_nearest_face() {
        let sphere = Shape::sphere(0.2);
        let cuboid = Shape::cuboid(Vec3::new(1.0, 1.0, 1.0));
        let manifold =
            collide(&sphere, &at(0.0, 0.8, 0.0), &cuboid, &Isometry::IDENTITY).expect("inside");
        assert!(
            (manifold.normal - Vec3::Y).length() < 1e-4,
            "{:?}",
            manifold.normal
        );
        assert!(manifold.points[0].penetration > 0.2);
    }

    #[test]
    fn a_capsule_lying_on_the_ground_gets_two_contacts() {
        let capsule = Shape::capsule(1.0, 0.25);
        let lying = turned(Vec3::new(0.0, 0.2, 0.0), Vec3::Z, FRAC_PI_2);
        let manifold =
            collide(&capsule, &lying, &Shape::ground(), &Isometry::IDENTITY).expect("touching");
        assert_eq!(manifold.count, 2, "both ends rest on the floor");
        assert!((manifold.normal - (-Vec3::Y)).length() < 1e-5);
        for point in manifold.contacts() {
            assert!((point.penetration - 0.05).abs() < 1e-5, "{point:?}");
        }
        assert_ne!(manifold.points[0].feature, manifold.points[1].feature);
    }

    #[test]
    fn an_upright_capsule_touches_the_ground_with_one_end() {
        let capsule = Shape::capsule(1.0, 0.25);
        let manifold = collide(
            &capsule,
            &at(0.0, 1.2, 0.0),
            &Shape::ground(),
            &Isometry::IDENTITY,
        )
        .expect("touching");
        assert_eq!(manifold.count, 1);
        assert!((manifold.points[0].penetration - 0.05).abs() < 1e-5);
    }

    #[test]
    fn crossed_capsules_meet_where_they_cross() {
        let a = Shape::capsule(1.0, 0.2);
        let b = Shape::capsule(1.0, 0.2);
        // One upright, one lying across just above it: the closest points are
        // A's top end and the middle of B, 0.35 apart, so they overlap by 0.05.
        let across = turned(Vec3::new(0.0, 1.35, 0.0), Vec3::Z, FRAC_PI_2);
        let manifold = collide(&a, &Isometry::IDENTITY, &b, &across).expect("crossing");
        assert!(
            (manifold.normal - Vec3::Y).length() < 1e-4,
            "{:?}",
            manifold.normal
        );
        assert!(
            (manifold.points[0].penetration - 0.05).abs() < 1e-4,
            "{}",
            manifold.points[0].penetration
        );

        // Parallel and apart: nothing.
        let far = Isometry::from_position(Vec3::new(3.0, 0.0, 0.0));
        assert!(collide(&a, &Isometry::IDENTITY, &b, &far).is_none());
    }

    #[test]
    fn a_box_resting_flat_on_the_ground_has_four_contacts() {
        let cuboid = Shape::cuboid(Vec3::new(0.5, 0.5, 0.5));
        let manifold = collide(
            &cuboid,
            &at(0.0, 0.45, 0.0),
            &Shape::ground(),
            &Isometry::IDENTITY,
        )
        .expect("resting");
        assert_eq!(manifold.count, 4, "a face against a plane is four corners");
        assert!((manifold.normal - (-Vec3::Y)).length() < 1e-5);
        for point in manifold.contacts() {
            assert!((point.penetration - 0.05).abs() < 1e-4, "{point:?}");
        }
    }

    #[test]
    fn a_box_balanced_on_a_corner_has_one_contact() {
        let cuboid = Shape::cuboid(Vec3::splat(0.5));
        // Tipped so the body diagonal points straight down, which puts one
        // corner — and only one — at the bottom.
        let diagonal = Vec3::splat(1.0).normalized();
        let axis = diagonal.cross(-Vec3::Y).normalized();
        let angle = diagonal.dot(-Vec3::Y).clamp(-1.0, 1.0).acos();
        let tipped = Isometry::new(
            Vec3::new(0.0, 0.85, 0.0),
            Quat::from_axis_angle(axis, angle),
        );
        let manifold =
            collide(&cuboid, &tipped, &Shape::ground(), &Isometry::IDENTITY).expect("touching");
        assert_eq!(manifold.count, 1, "{manifold:?}");
    }

    #[test]
    fn two_boxes_face_to_face_produce_a_four_point_manifold() {
        let a = Shape::cuboid(Vec3::splat(0.5));
        let b = Shape::cuboid(Vec3::splat(0.5));
        let manifold = collide(&a, &at(0.0, 0.0, 0.0), &b, &at(0.95, 0.0, 0.0)).expect("touching");
        assert!(
            (manifold.normal - Vec3::X).length() < 1e-4,
            "{:?}",
            manifold.normal
        );
        assert_eq!(manifold.count, 4, "face against face");
        for point in manifold.contacts() {
            assert!((point.penetration - 0.05).abs() < 1e-3, "{point:?}");
        }
    }

    #[test]
    fn separated_boxes_report_nothing_on_every_axis() {
        let a = Shape::cuboid(Vec3::splat(0.5));
        let b = Shape::cuboid(Vec3::splat(0.5));
        for offset in [
            Vec3::new(1.01, 0.0, 0.0),
            Vec3::new(0.0, 1.01, 0.0),
            Vec3::new(0.0, 0.0, 1.01),
            // Apart on every axis at once, too.
            Vec3::new(1.01, 1.01, 1.01),
        ] {
            assert!(
                collide(
                    &a,
                    &Isometry::IDENTITY,
                    &b,
                    &Isometry::from_position(offset)
                )
                .is_none(),
                "should be apart at {offset:?}"
            );
        }
    }

    #[test]
    fn a_rotated_box_still_finds_the_shallowest_axis() {
        let a = Shape::cuboid(Vec3::splat(0.5));
        let b = Shape::cuboid(Vec3::splat(0.5));
        // Turned 45 degrees about Y and pushed in along X: the reference face
        // is A's, because B presents an edge.
        let spun = turned(Vec3::new(1.1, 0.0, 0.0), Vec3::Y, FRAC_PI_4);
        let manifold = collide(&a, &Isometry::IDENTITY, &b, &spun).expect("touching");
        assert!(
            manifold.normal.x > 0.9,
            "pushes along X: {:?}",
            manifold.normal
        );
        assert!(manifold.points[0].penetration > 0.0);
        assert!(manifold.points[0].penetration < 0.2);
    }

    #[test]
    fn a_capsule_lying_on_a_box_gets_two_contacts() {
        let capsule = Shape::capsule(0.8, 0.2);
        let cuboid = Shape::cuboid(Vec3::new(2.0, 0.5, 2.0));
        let lying = turned(Vec3::new(0.0, 0.65, 0.0), Vec3::Z, FRAC_PI_2);
        let manifold = collide(&capsule, &lying, &cuboid, &Isometry::IDENTITY).expect("resting");
        assert!(
            manifold.normal.y < -0.9,
            "points down into the box: {:?}",
            manifold.normal
        );
        assert!(
            manifold.count >= 2,
            "a lying capsule needs both ends: {manifold:?}"
        );
    }

    #[test]
    fn segment_helpers_agree_with_the_obvious_answers() {
        let a = Vec3::ZERO;
        let b = Vec3::X * 2.0;
        assert_eq!(
            closest_point_on_segment(Vec3::new(1.0, 5.0, 0.0), a, b),
            Vec3::X
        );
        assert_eq!(closest_point_on_segment(Vec3::new(-3.0, 0.0, 0.0), a, b), a);
        assert_eq!(closest_point_on_segment(Vec3::new(9.0, 0.0, 0.0), a, b), b);

        // Two segments crossing at right angles, one unit apart.
        let (p, q) = closest_points_between_segments(
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, -1.0),
            Vec3::new(0.0, 1.0, 1.0),
        );
        assert!((p - Vec3::ZERO).length() < 1e-5, "{p:?}");
        assert!((q - Vec3::Y).length() < 1e-5, "{q:?}");

        // Parallel segments: any closest pair will do, but the distance is right.
        let (p, q) =
            closest_points_between_segments(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.0));
        assert!(((p - q).length() - 1.0).abs() < 1e-5);
    }
}
