use crate::mat::Mat4;
use crate::vec::{vec4, Vec3};
use core::ops::Mul;

/// Unit quaternion rotation, `w` last.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Quat = Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// Rotation of `angle` radians around `axis` (which is normalized first).
    pub fn from_axis_angle(axis: Vec3, angle: f32) -> Self {
        let axis = axis.normalized();
        let (s, c) = (angle * 0.5).sin_cos();
        Self::new(axis.x * s, axis.y * s, axis.z * s, c)
    }

    /// Intrinsic Y-X-Z ("yaw, pitch, roll") Euler rotation.
    pub fn from_euler(yaw: f32, pitch: f32, roll: f32) -> Self {
        Self::from_axis_angle(Vec3::Y, yaw)
            * Self::from_axis_angle(Vec3::X, pitch)
            * Self::from_axis_angle(Vec3::Z, roll)
    }

    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }

    #[inline]
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    pub fn normalized(self) -> Self {
        let len = self.length();
        if len > 0.0 {
            let inv = 1.0 / len;
            Self::new(self.x * inv, self.y * inv, self.z * inv, self.w * inv)
        } else {
            Self::IDENTITY
        }
    }

    #[inline]
    pub fn conjugate(self) -> Self {
        Self::new(-self.x, -self.y, -self.z, self.w)
    }

    /// Rotate a vector by this quaternion (assumed unit length).
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let u = Vec3::new(self.x, self.y, self.z);
        let t = u.cross(v) * 2.0;
        v + t * self.w + u.cross(t)
    }

    /// Shortest-arc spherical interpolation.
    pub fn slerp(self, mut other: Self, t: f32) -> Self {
        let mut cos_theta = self.dot(other);
        if cos_theta < 0.0 {
            other = Self::new(-other.x, -other.y, -other.z, -other.w);
            cos_theta = -cos_theta;
        }
        if cos_theta > 0.9995 {
            // Nearly parallel: lerp and renormalize to avoid a division by ~0.
            return Self::new(
                self.x + (other.x - self.x) * t,
                self.y + (other.y - self.y) * t,
                self.z + (other.z - self.z) * t,
                self.w + (other.w - self.w) * t,
            )
            .normalized();
        }
        let theta = cos_theta.acos();
        let sin_theta = theta.sin();
        let a = ((1.0 - t) * theta).sin() / sin_theta;
        let b = (t * theta).sin() / sin_theta;
        Self::new(
            self.x * a + other.x * b,
            self.y * a + other.y * b,
            self.z * a + other.z * b,
            self.w * a + other.w * b,
        )
    }

    pub fn to_mat4(self) -> Mat4 {
        let Quat { x, y, z, w } = self;
        let (x2, y2, z2) = (x + x, y + y, z + z);
        let (xx, xy, xz) = (x * x2, x * y2, x * z2);
        let (yy, yz, zz) = (y * y2, y * z2, z * z2);
        let (wx, wy, wz) = (w * x2, w * y2, w * z2);
        Mat4::from_cols(
            vec4(1.0 - (yy + zz), xy + wz, xz - wy, 0.0),
            vec4(xy - wz, 1.0 - (xx + zz), yz + wx, 0.0),
            vec4(xz + wy, yz - wx, 1.0 - (xx + yy), 0.0),
            vec4(0.0, 0.0, 0.0, 1.0),
        )
    }
}

impl Mul for Quat {
    type Output = Quat;
    #[inline]
    fn mul(self, o: Quat) -> Quat {
        Quat::new(
            self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
            self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn quarter_turn_around_y_maps_x_to_minus_z() {
        let q = Quat::from_axis_angle(Vec3::Y, FRAC_PI_2);
        assert!(
            close(q.rotate(Vec3::X), -Vec3::Z),
            "{:?}",
            q.rotate(Vec3::X)
        );
    }

    #[test]
    fn matrix_and_direct_rotation_agree() {
        let q = Quat::from_euler(0.3, -0.8, 1.2);
        let v = Vec3::new(1.0, -2.0, 0.5);
        assert!(close(q.rotate(v), q.to_mat4().transform_vector(v)));
    }

    #[test]
    fn slerp_endpoints_are_exact() {
        let a = Quat::IDENTITY;
        let b = Quat::from_axis_angle(Vec3::Z, 1.0);
        assert!(close(a.slerp(b, 0.0).rotate(Vec3::X), a.rotate(Vec3::X)));
        assert!(close(a.slerp(b, 1.0).rotate(Vec3::X), b.rotate(Vec3::X)));
    }

    #[test]
    fn conjugate_undoes_rotation() {
        let q = Quat::from_euler(0.4, 0.2, -0.9);
        let v = Vec3::new(0.3, 1.0, -0.2);
        assert!(close(q.conjugate().rotate(q.rotate(v)), v));
    }
}
