use runity_math::{Mat4, Quat, Vec3};

/// Position, rotation and scale of an object in the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn from_position(position: Vec3) -> Self {
        Self {
            position,
            ..Self::IDENTITY
        }
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    /// Take a transform apart from a matrix.
    ///
    /// Position is the last column, scale the lengths of the first three, and
    /// rotation what is left once the scale is divided out. A matrix carrying
    /// shear cannot be described this way, and comes back as the nearest
    /// rotation — which is the honest answer, and the reason a scene graph
    /// stores transforms rather than matrices.
    pub fn from_matrix(matrix: Mat4) -> Self {
        let column = |index: usize| {
            let c = matrix.cols[index];
            Vec3::new(c.x, c.y, c.z)
        };
        let (x, y, z) = (column(0), column(1), column(2));
        let mut scale = Vec3::new(x.length(), y.length(), z.length());
        // A negative determinant means the matrix mirrors; put that in one
        // axis rather than trying to express it as a rotation, which cannot.
        if x.cross(y).dot(z) < 0.0 {
            scale.x = -scale.x;
        }
        let safe = |value: f32| if value.abs() < 1e-8 { 1.0 } else { value };
        let rotation = Quat::from_axes(x / safe(scale.x), y / safe(scale.y), z / safe(scale.z));
        Transform {
            position: column(3),
            rotation,
            scale,
        }
    }

    /// Local-to-world matrix: translate ∘ rotate ∘ scale.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_translation(self.position)
            * self.rotation.to_mat4()
            * Mat4::from_scale(self.scale)
    }

    pub fn forward(&self) -> Vec3 {
        // Right-handed convention: -Z is forward.
        self.rotation.rotate(-Vec3::Z)
    }

    pub fn right(&self) -> Vec3 {
        self.rotation.rotate(Vec3::X)
    }

    pub fn up(&self) -> Vec3 {
        self.rotation.rotate(Vec3::Y)
    }

    pub fn translate(&mut self, delta: Vec3) {
        self.position += delta;
    }

    pub fn rotate(&mut self, rotation: Quat) {
        self.rotation = (rotation * self.rotation).normalized();
    }
}

/// A perspective camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    /// Vertical field of view, in radians.
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 1.5, 4.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            fov_y: 60f32.to_radians(),
            near: 0.05,
            far: 500.0,
        }
    }
}

impl Camera {
    pub fn look_at(position: Vec3, target: Vec3) -> Self {
        Self {
            position,
            target,
            ..Self::default()
        }
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at(self.position, self.target, self.up)
    }

    pub fn projection(&self, aspect_ratio: f32) -> Mat4 {
        Mat4::perspective(self.fov_y, aspect_ratio, self.near, self.far)
    }

    pub fn view_projection(&self, aspect_ratio: f32) -> Mat4 {
        self.projection(aspect_ratio) * self.view()
    }

    /// Orbit the camera around its target.
    pub fn orbit(&mut self, yaw: f32, pitch: f32, distance: f32) {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let offset = Vec3::new(cp * sy, sp, cp * cy) * distance;
        self.position = self.target + offset;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_applies_scale_then_rotation_then_translation() {
        let t = Transform::from_position(Vec3::new(0.0, 0.0, 5.0))
            .with_rotation(Quat::from_axis_angle(Vec3::Y, std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(2.0));
        // X scaled to 2, rotated onto -Z, then translated by +5 on Z.
        let p = t.matrix().transform_point(Vec3::X).xyz();
        assert!((p - Vec3::new(0.0, 0.0, 3.0)).length() < 1e-5, "{p:?}");
    }

    #[test]
    fn basis_vectors_follow_the_rotation() {
        let t = Transform::IDENTITY
            .with_rotation(Quat::from_axis_angle(Vec3::Y, std::f32::consts::FRAC_PI_2));
        assert!(
            (t.forward() - (-Vec3::X)).length() < 1e-5,
            "{:?}",
            t.forward()
        );
        assert!((t.up() - Vec3::Y).length() < 1e-5);
    }

    #[test]
    fn orbit_keeps_the_configured_distance() {
        let mut c = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
        c.orbit(1.1, 0.4, 7.0);
        assert!(((c.position - c.target).length() - 7.0).abs() < 1e-4);
    }

    #[test]
    fn the_camera_looks_at_its_target() {
        let c = Camera::look_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO);
        let clip = c.view_projection(1.0).transform_point(Vec3::ZERO);
        let ndc = clip.perspective_divide();
        assert!(
            ndc.x.abs() < 1e-5 && ndc.y.abs() < 1e-5,
            "the target lands in the center"
        );
        assert!((0.0..=1.0).contains(&ndc.z), "and inside the depth range");
    }
}
