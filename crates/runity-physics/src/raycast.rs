//! Ray queries: the thing gameplay code asks for most.
//!
//! Standing on the ground, shooting at something, picking an object with the
//! mouse, keeping a camera out of walls — all of it is a ray against the same
//! shapes the simulation uses.

use crate::shape::{Isometry, Shape};
use runity_math::Vec3;

/// A ray with a finite length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    /// Unit length; [`Ray::new`] normalizes it.
    pub direction: Vec3,
    pub max_distance: f32,
}

impl Ray {
    pub fn new(origin: Vec3, direction: Vec3, max_distance: f32) -> Self {
        Self {
            origin,
            direction: direction.normalized(),
            max_distance,
        }
    }

    /// The point `distance` along the ray.
    #[inline]
    pub fn at(&self, distance: f32) -> Vec3 {
        self.origin + self.direction * distance
    }
}

/// Where a ray met a shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    pub distance: f32,
    pub point: Vec3,
    /// Surface normal at the hit, pointing back towards the ray.
    pub normal: Vec3,
}

/// Cast a ray at one placed shape.
///
/// A ray that starts inside a shape reports distance 0 and the surface it is
/// heading for, which is what a "am I stuck in geometry" check wants.
pub fn ray_shape(ray: &Ray, shape: &Shape, at: &Isometry) -> Option<RayHit> {
    match *shape {
        Shape::Sphere { radius } => ray_sphere(ray, at.position, radius),
        Shape::HalfSpace { normal } => ray_halfspace(ray, at, normal),
        Shape::Cuboid { half_extents } => ray_cuboid(ray, at, half_extents),
        Shape::Capsule {
            half_height,
            radius,
        } => ray_capsule(ray, at, half_height, radius),
    }
    .filter(|hit| hit.distance <= ray.max_distance)
}

fn ray_sphere(ray: &Ray, center: Vec3, radius: f32) -> Option<RayHit> {
    let to_center = ray.origin - center;
    // |o + td - c|² = r², solved for t.
    let b = to_center.dot(ray.direction);
    let c = to_center.length_squared() - radius * radius;
    if c > 0.0 && b > 0.0 {
        return None; // outside and pointing away
    }
    let discriminant = b * b - c;
    if discriminant < 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    let distance = (-b - root).max(0.0);
    let point = ray.at(distance);
    let normal = (point - center).normalized();
    Some(RayHit {
        distance,
        point,
        normal,
    })
}

fn ray_halfspace(ray: &Ray, at: &Isometry, plane_normal: Vec3) -> Option<RayHit> {
    let normal = at.transform_direction(plane_normal).normalized();
    let denominator = ray.direction.dot(normal);
    let height = (ray.origin - at.position).dot(normal);
    if height <= 0.0 {
        // Already inside the solid side.
        return Some(RayHit {
            distance: 0.0,
            point: ray.origin,
            normal,
        });
    }
    if denominator >= -1e-6 {
        return None; // parallel, or moving away from the surface
    }
    let distance = -height / denominator;
    Some(RayHit {
        distance,
        point: ray.at(distance),
        normal,
    })
}

fn ray_cuboid(ray: &Ray, at: &Isometry, half: Vec3) -> Option<RayHit> {
    // Work in the box's frame, where the test is three slabs.
    let origin = at.inverse_transform_point(ray.origin);
    let direction = at.inverse_transform_direction(ray.direction);

    let mut near = f32::NEG_INFINITY;
    let mut far = f32::INFINITY;
    let mut axis = 0usize;
    let mut sign = 1.0f32;

    for i in 0..3 {
        let (o, d, h) = (origin[i], direction[i], half[i]);
        if d.abs() < 1e-8 {
            if o < -h || o > h {
                return None; // parallel to this slab and outside it
            }
            continue;
        }
        let inverse = 1.0 / d;
        let mut t_low = (-h - o) * inverse;
        let mut t_high = (h - o) * inverse;
        let mut entry_sign = -1.0;
        if t_low > t_high {
            core::mem::swap(&mut t_low, &mut t_high);
            entry_sign = 1.0;
        }
        if t_low > near {
            near = t_low;
            axis = i;
            sign = entry_sign;
        }
        far = far.min(t_high);
        if near > far {
            return None;
        }
    }

    if far < 0.0 {
        return None; // the box is entirely behind the ray
    }
    let distance = near.max(0.0);
    let mut local_normal = Vec3::ZERO;
    local_normal[axis] = sign;
    Some(RayHit {
        distance,
        point: ray.at(distance),
        normal: at.transform_direction(local_normal),
    })
}

fn ray_capsule(ray: &Ray, at: &Isometry, half_height: f32, radius: f32) -> Option<RayHit> {
    // In the capsule's frame the axis is Y, so this is an infinite cylinder
    // test clipped to the straight part, plus a sphere at each end.
    let origin = at.inverse_transform_point(ray.origin);
    let direction = at.inverse_transform_direction(ray.direction);

    // Starting inside is a hit at zero distance, as it is for every other shape.
    let closest = Vec3::new(0.0, origin.y.clamp(-half_height, half_height), 0.0);
    let offset = origin - closest;
    if offset.length() <= radius {
        let normal = if offset.length_squared() > 1e-12 {
            offset.normalized()
        } else {
            Vec3::Y
        };
        return Some(RayHit {
            distance: 0.0,
            point: ray.origin,
            normal: at.transform_direction(normal),
        });
    }

    let mut best: Option<(f32, Vec3)> = None;
    let mut consider = |distance: f32, normal: Vec3| {
        if distance >= 0.0 && best.as_ref().map_or(true, |(d, _)| distance < *d) {
            best = Some((distance, normal));
        }
    };

    // Cylinder: drop the Y component and solve the 2D circle problem.
    let a = direction.x * direction.x + direction.z * direction.z;
    if a > 1e-8 {
        let b = origin.x * direction.x + origin.z * direction.z;
        let c = origin.x * origin.x + origin.z * origin.z - radius * radius;
        let discriminant = b * b - a * c;
        if discriminant >= 0.0 {
            let root = discriminant.sqrt();
            // Both roots are candidates: the near one is usually the answer,
            // but it can be behind the ray or past the end of the cylinder.
            for t in [(-b - root) / a, (-b + root) / a] {
                let y = origin.y + direction.y * t;
                if t >= 0.0 && y.abs() <= half_height {
                    let point = origin + direction * t;
                    consider(t, Vec3::new(point.x, 0.0, point.z).normalized());
                }
            }
        }
    }

    // The two caps, each contributing only the half that sticks out past the
    // cylinder.
    for end in [-half_height, half_height] {
        let center = Vec3::new(0.0, end, 0.0);
        let local_ray = Ray {
            origin,
            direction,
            max_distance: f32::INFINITY,
        };
        if let Some(hit) = ray_sphere(&local_ray, center, radius) {
            let y = origin.y + direction.y * hit.distance;
            if (end > 0.0 && y >= half_height) || (end < 0.0 && y <= -half_height) {
                consider(hit.distance, hit.normal);
            }
        }
    }

    let (distance, local_normal) = best?;
    Some(RayHit {
        distance,
        point: ray.at(distance),
        normal: at.transform_direction(local_normal),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::Quat;
    use std::f32::consts::FRAC_PI_2;

    fn ray(origin: Vec3, direction: Vec3) -> Ray {
        Ray::new(origin, direction, 100.0)
    }

    #[test]
    fn a_ray_hits_the_near_side_of_a_sphere() {
        let hit = ray_shape(
            &ray(Vec3::new(-5.0, 0.0, 0.0), Vec3::X),
            &Shape::sphere(1.0),
            &Isometry::IDENTITY,
        )
        .expect("hit");
        assert!((hit.distance - 4.0).abs() < 1e-4);
        assert!((hit.point + Vec3::X).length() < 1e-4, "{:?}", hit.point);
        assert!(
            (hit.normal + Vec3::X).length() < 1e-4,
            "the normal faces the ray"
        );
    }

    #[test]
    fn a_ray_that_misses_or_points_away_reports_nothing() {
        let sphere = Shape::sphere(1.0);
        assert!(ray_shape(
            &ray(Vec3::new(-5.0, 3.0, 0.0), Vec3::X),
            &sphere,
            &Isometry::IDENTITY
        )
        .is_none());
        assert!(ray_shape(
            &ray(Vec3::new(-5.0, 0.0, 0.0), -Vec3::X),
            &sphere,
            &Isometry::IDENTITY
        )
        .is_none());
        // Too short to reach.
        let short = Ray::new(Vec3::new(-5.0, 0.0, 0.0), Vec3::X, 2.0);
        assert!(ray_shape(&short, &sphere, &Isometry::IDENTITY).is_none());
    }

    #[test]
    fn a_ray_starting_inside_hits_at_zero() {
        let hit = ray_shape(
            &ray(Vec3::ZERO, Vec3::X),
            &Shape::sphere(1.0),
            &Isometry::IDENTITY,
        )
        .expect("hit");
        assert_eq!(hit.distance, 0.0);
    }

    #[test]
    fn a_ray_hits_the_ground_plane() {
        let hit = ray_shape(
            &ray(Vec3::new(0.0, 5.0, 0.0), -Vec3::Y),
            &Shape::ground(),
            &Isometry::IDENTITY,
        )
        .expect("hit");
        assert!((hit.distance - 5.0).abs() < 1e-4);
        assert!((hit.normal - Vec3::Y).length() < 1e-5);

        // Pointing up from above: nothing.
        assert!(ray_shape(
            &ray(Vec3::new(0.0, 5.0, 0.0), Vec3::Y),
            &Shape::ground(),
            &Isometry::IDENTITY
        )
        .is_none());
    }

    #[test]
    fn a_ray_hits_a_box_on_the_face_it_enters() {
        let cuboid = Shape::cuboid(Vec3::new(1.0, 2.0, 3.0));
        let hit = ray_shape(
            &ray(Vec3::new(-5.0, 0.0, 0.0), Vec3::X),
            &cuboid,
            &Isometry::IDENTITY,
        )
        .expect("hit");
        assert!((hit.distance - 4.0).abs() < 1e-4);
        assert!((hit.normal + Vec3::X).length() < 1e-4, "{:?}", hit.normal);

        // From above, it meets the top face.
        let top = ray_shape(
            &ray(Vec3::new(0.0, 9.0, 0.0), -Vec3::Y),
            &cuboid,
            &Isometry::IDENTITY,
        )
        .expect("hit");
        assert!((top.distance - 7.0).abs() < 1e-4);
        assert!((top.normal - Vec3::Y).length() < 1e-4);

        // Past the corner: a miss.
        assert!(ray_shape(
            &ray(Vec3::new(-5.0, 2.5, 0.0), Vec3::X),
            &cuboid,
            &Isometry::IDENTITY
        )
        .is_none());
    }

    #[test]
    fn a_rotated_box_is_hit_in_its_own_frame() {
        let cuboid = Shape::cuboid(Vec3::new(2.0, 0.5, 0.5));
        // Turned upright: the long axis now runs along Y.
        let at = Isometry::new(Vec3::ZERO, Quat::from_axis_angle(Vec3::Z, FRAC_PI_2));
        let hit = ray_shape(&ray(Vec3::new(0.0, 5.0, 0.0), -Vec3::Y), &cuboid, &at).expect("hit");
        assert!((hit.distance - 3.0).abs() < 1e-3, "{}", hit.distance);
        assert!((hit.normal - Vec3::Y).length() < 1e-3, "{:?}", hit.normal);
    }

    #[test]
    fn a_ray_hits_a_capsule_on_its_side_and_on_its_cap() {
        let capsule = Shape::capsule(1.0, 0.5);

        let side = ray_shape(
            &ray(Vec3::new(-5.0, 0.0, 0.0), Vec3::X),
            &capsule,
            &Isometry::IDENTITY,
        )
        .expect("hit the cylinder");
        assert!((side.distance - 4.5).abs() < 1e-4, "{}", side.distance);
        assert!((side.normal + Vec3::X).length() < 1e-4);

        let cap = ray_shape(
            &ray(Vec3::new(0.0, 5.0, 0.0), -Vec3::Y),
            &capsule,
            &Isometry::IDENTITY,
        )
        .expect("hit the cap");
        assert!((cap.distance - 3.5).abs() < 1e-4, "{}", cap.distance);
        assert!((cap.normal - Vec3::Y).length() < 1e-4);

        // Level with the cylinder but past its radius.
        assert!(ray_shape(
            &ray(Vec3::new(-5.0, 0.0, 1.0), Vec3::X),
            &capsule,
            &Isometry::IDENTITY
        )
        .is_none());
    }

    #[test]
    fn a_ray_inside_a_capsule_hits_at_zero() {
        let hit = ray_shape(
            &ray(Vec3::ZERO, Vec3::X),
            &Shape::capsule(1.0, 0.5),
            &Isometry::IDENTITY,
        )
        .expect("inside");
        assert_eq!(hit.distance, 0.0);
    }

    #[test]
    fn a_ray_along_an_offset_capsules_axis_still_hits_it() {
        let at = Isometry::from_position(Vec3::new(2.0, 0.0, 0.0));
        let hit = ray_shape(
            &ray(Vec3::new(2.0, 5.0, 0.0), -Vec3::Y),
            &Shape::capsule(1.0, 0.5),
            &at,
        )
        .expect("hit");
        assert!((hit.distance - 3.5).abs() < 1e-4, "{}", hit.distance);
    }
}
