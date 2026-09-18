use crate::vec::{vec4, Vec3, Vec4};
use core::ops::Mul;

/// Column-major 4x4 matrix, the same memory layout GLSL uses.
///
/// `cols[i]` is the i-th column, so `M * v == sum(cols[i] * v[i])`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat4 {
    pub cols: [Vec4; 4],
}

impl Default for Mat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat4 {
    pub const IDENTITY: Mat4 = Mat4 {
        cols: [
            vec4(1.0, 0.0, 0.0, 0.0),
            vec4(0.0, 1.0, 0.0, 0.0),
            vec4(0.0, 0.0, 1.0, 0.0),
            vec4(0.0, 0.0, 0.0, 1.0),
        ],
    };

    pub const ZERO: Mat4 = Mat4 {
        cols: [Vec4::ZERO; 4],
    };

    #[inline]
    pub const fn from_cols(c0: Vec4, c1: Vec4, c2: Vec4, c3: Vec4) -> Self {
        Self {
            cols: [c0, c1, c2, c3],
        }
    }

    #[inline]
    pub fn from_translation(t: Vec3) -> Self {
        let mut m = Self::IDENTITY;
        m.cols[3] = vec4(t.x, t.y, t.z, 1.0);
        m
    }

    #[inline]
    pub fn from_scale(s: Vec3) -> Self {
        Self::from_cols(
            vec4(s.x, 0.0, 0.0, 0.0),
            vec4(0.0, s.y, 0.0, 0.0),
            vec4(0.0, 0.0, s.z, 0.0),
            vec4(0.0, 0.0, 0.0, 1.0),
        )
    }

    pub fn from_rotation_x(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        Self::from_cols(
            vec4(1.0, 0.0, 0.0, 0.0),
            vec4(0.0, c, s, 0.0),
            vec4(0.0, -s, c, 0.0),
            vec4(0.0, 0.0, 0.0, 1.0),
        )
    }

    pub fn from_rotation_y(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        Self::from_cols(
            vec4(c, 0.0, -s, 0.0),
            vec4(0.0, 1.0, 0.0, 0.0),
            vec4(s, 0.0, c, 0.0),
            vec4(0.0, 0.0, 0.0, 1.0),
        )
    }

    pub fn from_rotation_z(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        Self::from_cols(
            vec4(c, s, 0.0, 0.0),
            vec4(-s, c, 0.0, 0.0),
            vec4(0.0, 0.0, 1.0, 0.0),
            vec4(0.0, 0.0, 0.0, 1.0),
        )
    }

    /// Right-handed perspective projection mapping depth to `[0, 1]`
    /// (the Vulkan/D3D convention — it keeps the near-plane clip test at `z > 0`).
    ///
    /// `fov_y` is in radians.
    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Self {
        debug_assert!(
            near > 0.0 && far > near,
            "near/far must be positive and ordered"
        );
        let f = 1.0 / (fov_y * 0.5).tan();
        let nf = 1.0 / (near - far);
        Self::from_cols(
            vec4(f / aspect, 0.0, 0.0, 0.0),
            vec4(0.0, f, 0.0, 0.0),
            vec4(0.0, 0.0, far * nf, -1.0),
            vec4(0.0, 0.0, far * near * nf, 0.0),
        )
    }

    /// Right-handed orthographic projection mapping depth to `[0, 1]`.
    pub fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Self {
        let rl = 1.0 / (right - left);
        let tb = 1.0 / (top - bottom);
        let nf = 1.0 / (near - far);
        Self::from_cols(
            vec4(2.0 * rl, 0.0, 0.0, 0.0),
            vec4(0.0, 2.0 * tb, 0.0, 0.0),
            vec4(0.0, 0.0, nf, 0.0),
            vec4(-(right + left) * rl, -(top + bottom) * tb, near * nf, 1.0),
        )
    }

    /// Right-handed "camera at `eye` looking at `target`" view matrix.
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let f = (target - eye).normalized();
        let s = f.cross(up).normalized();
        let u = s.cross(f);
        Self::from_cols(
            vec4(s.x, u.x, -f.x, 0.0),
            vec4(s.y, u.y, -f.y, 0.0),
            vec4(s.z, u.z, -f.z, 0.0),
            vec4(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
        )
    }

    /// Transform a point (implicit `w = 1`), returning homogeneous coordinates.
    #[inline]
    pub fn transform_point(&self, p: Vec3) -> Vec4 {
        *self * p.extend(1.0)
    }

    /// Transform a direction (implicit `w = 0`, translation ignored).
    #[inline]
    pub fn transform_vector(&self, v: Vec3) -> Vec3 {
        (*self * v.extend(0.0)).xyz()
    }

    pub fn transpose(&self) -> Self {
        let c = &self.cols;
        Self::from_cols(
            vec4(c[0].x, c[1].x, c[2].x, c[3].x),
            vec4(c[0].y, c[1].y, c[2].y, c[3].y),
            vec4(c[0].z, c[1].z, c[2].z, c[3].z),
            vec4(c[0].w, c[1].w, c[2].w, c[3].w),
        )
    }

    pub fn to_array(&self) -> [f32; 16] {
        let c = &self.cols;
        [
            c[0].x, c[0].y, c[0].z, c[0].w, c[1].x, c[1].y, c[1].z, c[1].w, c[2].x, c[2].y, c[2].z,
            c[2].w, c[3].x, c[3].y, c[3].z, c[3].w,
        ]
    }

    pub fn from_array(m: [f32; 16]) -> Self {
        Self::from_cols(
            vec4(m[0], m[1], m[2], m[3]),
            vec4(m[4], m[5], m[6], m[7]),
            vec4(m[8], m[9], m[10], m[11]),
            vec4(m[12], m[13], m[14], m[15]),
        )
    }

    /// General cofactor inverse. Returns `None` for singular matrices.
    ///
    /// The index math below is layout-agnostic: feeding it column-major data
    /// yields the inverse in column-major, because `inv(Aᵀ) == inv(A)ᵀ`.
    pub fn inverse(&self) -> Option<Self> {
        let m = self.to_array();
        let mut inv = [0.0f32; 16];

        inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15]
            + m[9] * m[7] * m[14]
            + m[13] * m[6] * m[11]
            - m[13] * m[7] * m[10];
        inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15]
            - m[8] * m[7] * m[14]
            - m[12] * m[6] * m[11]
            + m[12] * m[7] * m[10];
        inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15]
            + m[8] * m[7] * m[13]
            + m[12] * m[5] * m[11]
            - m[12] * m[7] * m[9];
        inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14]
            - m[8] * m[6] * m[13]
            - m[12] * m[5] * m[10]
            + m[12] * m[6] * m[9];
        inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15]
            - m[9] * m[3] * m[14]
            - m[13] * m[2] * m[11]
            + m[13] * m[3] * m[10];
        inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15]
            + m[8] * m[3] * m[14]
            + m[12] * m[2] * m[11]
            - m[12] * m[3] * m[10];
        inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15]
            - m[8] * m[3] * m[13]
            - m[12] * m[1] * m[11]
            + m[12] * m[3] * m[9];
        inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14]
            + m[8] * m[2] * m[13]
            + m[12] * m[1] * m[10]
            - m[12] * m[2] * m[9];
        inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15]
            + m[5] * m[3] * m[14]
            + m[13] * m[2] * m[7]
            - m[13] * m[3] * m[6];
        inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15]
            - m[4] * m[3] * m[14]
            - m[12] * m[2] * m[7]
            + m[12] * m[3] * m[6];
        inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15]
            + m[4] * m[3] * m[13]
            + m[12] * m[1] * m[7]
            - m[12] * m[3] * m[5];
        inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14]
            - m[4] * m[2] * m[13]
            - m[12] * m[1] * m[6]
            + m[12] * m[2] * m[5];
        inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11]
            - m[5] * m[3] * m[10]
            - m[9] * m[2] * m[7]
            + m[9] * m[3] * m[6];
        inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11]
            + m[4] * m[3] * m[10]
            + m[8] * m[2] * m[7]
            - m[8] * m[3] * m[6];
        inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11]
            - m[4] * m[3] * m[9]
            - m[8] * m[1] * m[7]
            + m[8] * m[3] * m[5];
        inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10]
            + m[4] * m[2] * m[9]
            + m[8] * m[1] * m[6]
            - m[8] * m[2] * m[5];

        let det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
        if det.abs() < 1e-20 {
            return None;
        }
        let inv_det = 1.0 / det;
        for v in inv.iter_mut() {
            *v *= inv_det;
        }
        Some(Self::from_array(inv))
    }

    /// Matrix used to transform normals: `inverse(model)` transposed.
    pub fn normal_matrix(&self) -> Self {
        self.inverse()
            .map(|i| i.transpose())
            .unwrap_or(Self::IDENTITY)
    }
}

impl Mul<Vec4> for Mat4 {
    type Output = Vec4;
    #[inline]
    fn mul(self, v: Vec4) -> Vec4 {
        self.cols[0] * v.x + self.cols[1] * v.y + self.cols[2] * v.z + self.cols[3] * v.w
    }
}

impl Mul<Mat4> for Mat4 {
    type Output = Mat4;
    #[inline]
    fn mul(self, rhs: Mat4) -> Mat4 {
        Mat4::from_cols(
            self * rhs.cols[0],
            self * rhs.cols[1],
            self * rhs.cols[2],
            self * rhs.cols[3],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vec::vec3;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn identity_is_neutral() {
        let m = Mat4::from_translation(vec3(1.0, 2.0, 3.0));
        assert_eq!(m * Mat4::IDENTITY, m);
        assert_eq!(Mat4::IDENTITY * m, m);
    }

    #[test]
    fn translation_moves_points_not_directions() {
        let m = Mat4::from_translation(vec3(1.0, 2.0, 3.0));
        assert_eq!(m.transform_point(Vec3::ZERO).xyz(), vec3(1.0, 2.0, 3.0));
        assert_eq!(m.transform_vector(Vec3::X), Vec3::X);
    }

    #[test]
    fn inverse_round_trips() {
        let m = Mat4::from_translation(vec3(3.0, -1.0, 2.0))
            * Mat4::from_rotation_y(0.7)
            * Mat4::from_scale(vec3(2.0, 2.0, 2.0));
        let inv = m.inverse().expect("invertible");
        let back = inv * m;
        for (a, b) in back.to_array().iter().zip(Mat4::IDENTITY.to_array().iter()) {
            assert!(approx(*a, *b), "{back:?}");
        }
    }

    #[test]
    fn singular_matrix_has_no_inverse() {
        assert!(Mat4::ZERO.inverse().is_none());
    }

    #[test]
    fn perspective_maps_near_and_far_to_zero_and_one() {
        let p = Mat4::perspective(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        // Right-handed: the camera looks down -Z, so the near plane sits at z = -0.1.
        let near = p.transform_point(vec3(0.0, 0.0, -0.1)).perspective_divide();
        let far = p
            .transform_point(vec3(0.0, 0.0, -100.0))
            .perspective_divide();
        assert!(approx(near.z, 0.0), "near z = {}", near.z);
        assert!(approx(far.z, 1.0), "far z = {}", far.z);
    }

    #[test]
    fn look_at_puts_target_on_the_view_axis() {
        let v = Mat4::look_at(vec3(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let origin = v.transform_point(Vec3::ZERO);
        assert!(approx(origin.x, 0.0) && approx(origin.y, 0.0));
        assert!(
            approx(origin.z, -5.0),
            "target should be 5 units down -Z, got {origin:?}"
        );
    }
}
