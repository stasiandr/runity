//! The camera, as the deferred passes need it.
//!
//! Screen-space effects live or die on being able to go both ways: from a
//! pixel to a ray through the world, and from a world position back to the
//! pixel it lands on. The mapping has to agree with the rasterizer's to the
//! half-pixel, or SSR reflects things one row off.

use runity_math::{Mat4, Vec3, Vec4};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraView {
    pub view: Mat4,
    pub projection: Mat4,
    pub position: Vec3,
    pub near: f32,
    pub far: f32,
}

impl CameraView {
    pub fn new(view: Mat4, projection: Mat4, position: Vec3, near: f32, far: f32) -> Self {
        Self {
            view,
            projection,
            position,
            near,
            far,
        }
    }

    #[inline]
    pub fn view_projection(&self) -> Mat4 {
        self.projection * self.view
    }

    pub fn inverse_view_projection(&self) -> Mat4 {
        self.view_projection().inverse().unwrap_or(Mat4::IDENTITY)
    }

    /// World-space direction through the center of pixel `(x, y)`.
    pub fn ray_direction(
        &self,
        inverse_view_projection: &Mat4,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> Vec3 {
        let ndc_x = (x as f32 + 0.5) / width as f32 * 2.0 - 1.0;
        let ndc_y = 1.0 - (y as f32 + 0.5) / height as f32 * 2.0;
        // The far plane, unprojected: any point along the ray will do.
        let far = *inverse_view_projection * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
        (far.perspective_divide() - self.position).normalized()
    }

    /// Where a world position lands on screen: pixel coordinates plus its NDC
    /// depth. `None` when it is at or behind the near plane.
    ///
    /// This mirrors the rasterizer's viewport mapping exactly, including the
    /// flipped Y.
    pub fn project(
        &self,
        world: Vec3,
        width: usize,
        height: usize,
        view_projection: &Mat4,
    ) -> Option<(f32, f32, f32)> {
        let clip = view_projection.transform_point(world);
        if clip.w <= 1e-6 || clip.z < 0.0 {
            return None;
        }
        let ndc = clip.perspective_divide();
        Some((
            (ndc.x * 0.5 + 0.5) * width as f32,
            (0.5 - ndc.y * 0.5) * height as f32,
            ndc.z,
        ))
    }

    /// Distance from the eye along the view direction — what SSAO compares.
    #[inline]
    pub fn view_depth(&self, world: Vec3) -> f32 {
        -self.view.transform_point(world).z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> CameraView {
        CameraView::new(
            Mat4::look_at(Vec3::new(0.0, 1.0, 4.0), Vec3::ZERO, Vec3::Y),
            Mat4::perspective(60f32.to_radians(), 16.0 / 9.0, 0.1, 100.0),
            Vec3::new(0.0, 1.0, 4.0),
            0.1,
            100.0,
        )
    }

    #[test]
    fn a_pixel_ray_and_the_projection_are_inverses() {
        let camera = camera();
        let inverse = camera.inverse_view_projection();
        let view_projection = camera.view_projection();
        let (width, height) = (160, 90);

        for (x, y) in [(0usize, 0usize), (80, 45), (159, 89), (23, 71)] {
            let direction = camera.ray_direction(&inverse, x, y, width, height);
            // Walk a few units along the ray and project the point back.
            let world = camera.position + direction * 3.5;
            let (px, py, depth) = camera
                .project(world, width, height, &view_projection)
                .expect("in front");
            assert!((px - (x as f32 + 0.5)).abs() < 0.01, "x: {px} vs {x}");
            assert!((py - (y as f32 + 0.5)).abs() < 0.01, "y: {py} vs {y}");
            assert!((0.0..=1.0).contains(&depth));
        }
    }

    #[test]
    fn points_behind_the_camera_do_not_project() {
        let camera = camera();
        let view_projection = camera.view_projection();
        assert!(camera
            .project(Vec3::new(0.0, 1.0, 20.0), 64, 64, &view_projection)
            .is_none());
    }

    #[test]
    fn view_depth_grows_with_distance_from_the_eye() {
        // Straight down -Z, so the arithmetic is exact.
        let position = Vec3::new(0.0, 0.0, 4.0);
        let camera = CameraView::new(
            Mat4::look_at(position, Vec3::ZERO, Vec3::Y),
            Mat4::perspective(60f32.to_radians(), 1.0, 0.1, 100.0),
            position,
            0.1,
            100.0,
        );
        let near = camera.view_depth(Vec3::new(0.0, 0.0, 2.0));
        let far = camera.view_depth(Vec3::new(0.0, 0.0, -6.0));
        assert!((near - 2.0).abs() < 1e-4, "{near}");
        assert!((far - 10.0).abs() < 1e-4, "{far}");
    }
}
