//! The six planes a camera can see between.
//!
//! Drawing something the camera cannot see costs exactly as much as drawing
//! something it can: the triangles are still transformed, clipped and thrown
//! away one at a time. Testing the whole object against six planes first costs
//! a few dozen floating-point operations and removes all of it.
//!
//! The planes come out of the view-projection matrix directly, which is worth
//! knowing because it means they are always the planes actually being used —
//! a frustum built separately from the camera's own numbers is a frustum that
//! drifts out of step with it.

use crate::{Mat4, Vec3, Vec4};

/// A plane, as `dot(normal, point) + distance = 0`, with the normal pointing
/// into the volume that counts as inside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    /// Unit normal.
    pub normal: Vec3,
    /// Signed distance from the origin along the normal.
    pub distance: f32,
}

impl Plane {
    /// A plane from the coefficients of `ax + by + cz + d = 0`, normalized.
    pub fn new(a: f32, b: f32, c: f32, d: f32) -> Self {
        let normal = Vec3::new(a, b, c);
        let length = normal.length();
        if length < 1e-9 {
            return Plane {
                normal: Vec3::Y,
                distance: d,
            };
        }
        Plane {
            normal: normal / length,
            distance: d / length,
        }
    }

    /// How far a point is from the plane: positive inside, negative outside.
    pub fn signed_distance(&self, point: Vec3) -> f32 {
        self.normal.dot(point) + self.distance
    }
}

/// The volume a camera can see.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frustum {
    /// Left, right, bottom, top, near, far.
    pub planes: [Plane; 6],
}

impl Frustum {
    /// Extract the planes from a view-projection matrix.
    ///
    /// The rows of the matrix combine into the clip-space inequalities that
    /// define the volume — the standard Gribb–Hartmann extraction. It works
    /// for any projection the matrix describes, perspective or orthographic,
    /// which is why it is preferable to building the planes from a field of
    /// view and a range that then have to be kept in step.
    pub fn from_view_projection(matrix: Mat4) -> Self {
        // Rows of the matrix, from its columns.
        let row = |index: usize| {
            Vec4::new(
                matrix.cols[0][index],
                matrix.cols[1][index],
                matrix.cols[2][index],
                matrix.cols[3][index],
            )
        };
        let (x, y, z, w) = (row(0), row(1), row(2), row(3));

        // The near plane is `z > 0` rather than `z > -w`, because this
        // engine's projection maps depth to 0..1 as Vulkan and Direct3D do,
        // not to -1..1 as OpenGL does. Getting this wrong clips everything
        // in front of the camera, which looks like the scene is missing.
        let plane = |v: Vec4| Plane::new(v.x, v.y, v.z, v.w);
        Frustum {
            planes: [
                plane(w + x), // left
                plane(w - x), // right
                plane(w + y), // bottom
                plane(w - y), // top
                plane(z),     // near
                plane(w - z), // far
            ],
        }
    }

    /// Whether a sphere is at least partly inside.
    ///
    /// Conservative: something just outside a corner may still be reported as
    /// inside, which costs a draw call. The other mistake — reporting
    /// something visible as outside — costs a hole in the picture, so the
    /// test errs in the only direction it can afford to.
    pub fn contains_sphere(&self, centre: Vec3, radius: f32) -> bool {
        self.planes
            .iter()
            .all(|plane| plane.signed_distance(centre) >= -radius)
    }

    /// Whether a point is inside.
    pub fn contains_point(&self, point: Vec3) -> bool {
        self.contains_sphere(point, 0.0)
    }

    /// Whether an axis-aligned box is at least partly inside.
    ///
    /// Tests the corner of the box furthest along each plane's normal: if
    /// even that one is outside, every corner is.
    pub fn intersects_box(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|plane| {
            let furthest = Vec3::new(
                if plane.normal.x >= 0.0 { max.x } else { min.x },
                if plane.normal.y >= 0.0 { max.y } else { min.y },
                if plane.normal.z >= 0.0 { max.z } else { min.z },
            );
            plane.signed_distance(furthest) >= 0.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vec3;

    /// A camera at the origin looking down -Z, as everything else here does.
    fn looking_forward() -> Frustum {
        let projection = Mat4::perspective(60f32.to_radians(), 16.0 / 9.0, 0.1, 100.0);
        let view = Mat4::look_at(Vec3::ZERO, vec3(0.0, 0.0, -1.0), Vec3::Y);
        Frustum::from_view_projection(projection * view)
    }

    #[test]
    fn what_is_in_front_is_inside_and_what_is_behind_is_not() {
        let frustum = looking_forward();
        assert!(frustum.contains_point(vec3(0.0, 0.0, -10.0)));
        assert!(
            !frustum.contains_point(vec3(0.0, 0.0, 10.0)),
            "behind the camera"
        );
        assert!(
            !frustum.contains_point(vec3(0.0, 0.0, -1_000.0)),
            "past the far plane"
        );
        assert!(
            !frustum.contains_point(vec3(0.0, 0.0, -0.01)),
            "in front of the near plane"
        );
    }

    #[test]
    fn the_sides_open_out_with_distance() {
        let frustum = looking_forward();
        // A point ten metres off to the side is outside near the camera and
        // inside far away, which is what a perspective frustum means.
        assert!(!frustum.contains_point(vec3(10.0, 0.0, -2.0)));
        assert!(frustum.contains_point(vec3(10.0, 0.0, -50.0)));
    }

    #[test]
    fn a_sphere_that_pokes_in_counts_as_visible() {
        let frustum = looking_forward();
        let just_outside = vec3(6.0, 0.0, -5.0);
        assert!(!frustum.contains_point(just_outside));
        assert!(
            frustum.contains_sphere(just_outside, 5.0),
            "its near edge is in view"
        );
        assert!(!frustum.contains_sphere(just_outside, 0.1));
    }

    #[test]
    fn a_box_is_tested_by_its_nearest_corner() {
        let frustum = looking_forward();
        // A long wall running away from the camera: mostly outside, but the
        // end of it is visible.
        assert!(frustum.intersects_box(vec3(-40.0, -1.0, -30.0), vec3(-8.0, 1.0, -10.0)));
        // And one entirely off to the side, close in.
        assert!(!frustum.intersects_box(vec3(20.0, -1.0, -2.0), vec3(30.0, 1.0, -1.0)));
    }

    #[test]
    fn a_box_around_the_camera_is_visible() {
        // The case a naive "are all corners inside" test gets wrong: a room
        // the camera is standing in has no corner in view at all.
        let frustum = looking_forward();
        assert!(frustum.intersects_box(vec3(-50.0, -50.0, -50.0), vec3(50.0, 50.0, 50.0)));
    }

    #[test]
    fn turning_the_camera_turns_the_frustum() {
        let projection = Mat4::perspective(60f32.to_radians(), 1.0, 0.1, 100.0);
        let view = Mat4::look_at(Vec3::ZERO, vec3(1.0, 0.0, 0.0), Vec3::Y);
        let frustum = Frustum::from_view_projection(projection * view);

        assert!(
            frustum.contains_point(vec3(10.0, 0.0, 0.0)),
            "along the new forward"
        );
        assert!(
            !frustum.contains_point(vec3(0.0, 0.0, -10.0)),
            "where it used to look"
        );
    }

    #[test]
    fn moving_the_camera_moves_the_frustum() {
        let projection = Mat4::perspective(60f32.to_radians(), 1.0, 0.1, 100.0);
        let view = Mat4::look_at(vec3(0.0, 0.0, 50.0), vec3(0.0, 0.0, 0.0), Vec3::Y);
        let frustum = Frustum::from_view_projection(projection * view);

        assert!(frustum.contains_point(Vec3::ZERO));
        assert!(
            !frustum.contains_point(vec3(0.0, 0.0, -100.0)),
            "now past the far plane"
        );
    }

    #[test]
    fn an_orthographic_camera_gives_a_box() {
        let projection = Mat4::orthographic(-10.0, 10.0, -10.0, 10.0, 0.1, 100.0);
        let view = Mat4::look_at(Vec3::ZERO, vec3(0.0, 0.0, -1.0), Vec3::Y);
        let frustum = Frustum::from_view_projection(projection * view);

        // Unlike perspective, the sides do not open out: what is outside near
        // the camera is outside far away too.
        assert!(frustum.contains_point(vec3(9.0, 0.0, -5.0)));
        assert!(frustum.contains_point(vec3(9.0, 0.0, -90.0)));
        assert!(!frustum.contains_point(vec3(11.0, 0.0, -50.0)));
    }

    #[test]
    fn the_planes_agree_with_projecting_the_points() {
        // The independent check: a point is inside the frustum exactly when
        // its projected coordinates land in the clip volume.
        let projection = Mat4::perspective(70f32.to_radians(), 16.0 / 9.0, 0.5, 60.0);
        let view = Mat4::look_at(vec3(3.0, 2.0, 8.0), vec3(0.0, 0.0, -4.0), Vec3::Y);
        let matrix = projection * view;
        let frustum = Frustum::from_view_projection(matrix);

        let mut rng = crate::Rng::named(1, "frustum");
        let mut agreed = 0;
        for _ in 0..4_000 {
            let point = vec3(
                rng.range(-40.0, 40.0),
                rng.range(-40.0, 40.0),
                rng.range(-60.0, 40.0),
            );
            let clip = matrix * Vec4::new(point.x, point.y, point.z, 1.0);
            let projected = clip.w > 0.0
                && clip.x.abs() <= clip.w
                && clip.y.abs() <= clip.w
                && (0.0..=clip.w).contains(&clip.z);
            assert_eq!(
                frustum.contains_point(point),
                projected,
                "disagreed about {point:?}"
            );
            agreed += 1;
        }
        assert_eq!(agreed, 4_000);
    }

    #[test]
    fn a_degenerate_plane_does_not_produce_nonsense() {
        let plane = Plane::new(0.0, 0.0, 0.0, 5.0);
        assert!(plane.signed_distance(Vec3::ZERO).is_finite());
    }
}
