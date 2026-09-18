use crate::quat::Quat;
use crate::vec::{vec3, Vec3};
use core::ops::Mul;

/// Column-major 3x3 matrix.
///
/// Rotations live in [`Quat`]; this type exists mostly for inertia tensors,
/// which are symmetric 3x3 matrices that have to be rotated into world space
/// every time a body turns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3 {
    pub cols: [Vec3; 3],
}

impl Default for Mat3 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3 {
        cols: [
            vec3(1.0, 0.0, 0.0),
            vec3(0.0, 1.0, 0.0),
            vec3(0.0, 0.0, 1.0),
        ],
    };

    pub const ZERO: Mat3 = Mat3 {
        cols: [Vec3::ZERO; 3],
    };

    #[inline]
    pub const fn from_cols(c0: Vec3, c1: Vec3, c2: Vec3) -> Self {
        Self { cols: [c0, c1, c2] }
    }

    /// Diagonal matrix — the shape every inertia tensor has in its own
    /// principal axes.
    #[inline]
    pub const fn from_diagonal(d: Vec3) -> Self {
        Self::from_cols(
            vec3(d.x, 0.0, 0.0),
            vec3(0.0, d.y, 0.0),
            vec3(0.0, 0.0, d.z),
        )
    }

    pub fn from_quat(q: Quat) -> Self {
        let Quat { x, y, z, w } = q;
        let (x2, y2, z2) = (x + x, y + y, z + z);
        let (xx, xy, xz) = (x * x2, x * y2, x * z2);
        let (yy, yz, zz) = (y * y2, y * z2, z * z2);
        let (wx, wy, wz) = (w * x2, w * y2, w * z2);
        Self::from_cols(
            vec3(1.0 - (yy + zz), xy + wz, xz - wy),
            vec3(xy - wz, 1.0 - (xx + zz), yz + wx),
            vec3(xz + wy, yz - wx, 1.0 - (xx + yy)),
        )
    }

    pub fn transpose(&self) -> Self {
        let c = &self.cols;
        Self::from_cols(
            vec3(c[0].x, c[1].x, c[2].x),
            vec3(c[0].y, c[1].y, c[2].y),
            vec3(c[0].z, c[1].z, c[2].z),
        )
    }

    pub fn determinant(&self) -> f32 {
        self.cols[0].dot(self.cols[1].cross(self.cols[2]))
    }

    /// Inverse, or `None` when the matrix is singular.
    pub fn inverse(&self) -> Option<Self> {
        let c = &self.cols;
        // Cofactors: the cross products of pairs of columns.
        let a = c[1].cross(c[2]);
        let b = c[2].cross(c[0]);
        let d = c[0].cross(c[1]);
        let determinant = c[0].dot(a);
        if determinant.abs() < 1e-20 {
            return None;
        }
        let inv = 1.0 / determinant;
        // The cofactors are rows of the inverse, so this is a transpose too.
        Some(Self::from_cols(
            vec3(a.x, b.x, d.x) * inv,
            vec3(a.y, b.y, d.y) * inv,
            vec3(a.z, b.z, d.z) * inv,
        ))
    }

    /// `self * scale * selfᵀ` — how a tensor expressed in local axes looks from
    /// the outside once the body has rotated.
    pub fn transform_tensor(rotation: Mat3, local: Mat3) -> Mat3 {
        rotation * local * rotation.transpose()
    }

    /// The matrix `M` with `M * b == a × b` for every `b`.
    ///
    /// Angular terms in a contact solver are all cross products; writing them
    /// as a matrix is what lets an impulse be turned into a change in velocity.
    pub fn skew_symmetric(a: Vec3) -> Mat3 {
        Self::from_cols(
            vec3(0.0, a.z, -a.y),
            vec3(-a.z, 0.0, a.x),
            vec3(a.y, -a.x, 0.0),
        )
    }
}

impl Mul<Vec3> for Mat3 {
    type Output = Vec3;
    #[inline]
    fn mul(self, v: Vec3) -> Vec3 {
        self.cols[0] * v.x + self.cols[1] * v.y + self.cols[2] * v.z
    }
}

impl Mul<Mat3> for Mat3 {
    type Output = Mat3;
    #[inline]
    fn mul(self, rhs: Mat3) -> Mat3 {
        Mat3::from_cols(self * rhs.cols[0], self * rhs.cols[1], self * rhs.cols[2])
    }
}

impl Mul<f32> for Mat3 {
    type Output = Mat3;
    #[inline]
    fn mul(self, s: f32) -> Mat3 {
        Mat3::from_cols(self.cols[0] * s, self.cols[1] * s, self.cols[2] * s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::FRAC_PI_2;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn a_quaternion_and_its_matrix_rotate_the_same_way() {
        let q = Quat::from_euler(0.4, -0.8, 1.1);
        let v = vec3(0.3, -1.2, 0.7);
        assert!(close(Mat3::from_quat(q) * v, q.rotate(v)));
    }

    #[test]
    fn inverse_round_trips_and_rejects_singular_matrices() {
        let m = Mat3::from_quat(Quat::from_axis_angle(Vec3::Y, 0.7))
            * Mat3::from_diagonal(vec3(2.0, 3.0, 0.5));
        let inverse = m.inverse().expect("invertible");
        let identity = inverse * m;
        for i in 0..3 {
            assert!(
                close(identity.cols[i], Mat3::IDENTITY.cols[i]),
                "{identity:?}"
            );
        }
        assert!(Mat3::ZERO.inverse().is_none());
        // A matrix that flattens space onto a plane has no inverse either.
        assert!(Mat3::from_diagonal(vec3(1.0, 1.0, 0.0)).inverse().is_none());
    }

    #[test]
    fn skew_symmetric_is_a_cross_product() {
        let a = vec3(0.4, -1.0, 2.0);
        let b = vec3(-0.3, 0.6, 1.4);
        assert!(close(Mat3::skew_symmetric(a) * b, a.cross(b)));
        // ...and skew-symmetric means M = -Mᵀ.
        let m = Mat3::skew_symmetric(a);
        for i in 0..3 {
            assert!(close(m.transpose().cols[i], m.cols[i] * -1.0));
        }
    }

    #[test]
    fn rotating_an_inertia_tensor_moves_its_axes_with_the_body() {
        // A long thin rod along X: hard to spin around Y and Z, easy around X.
        let local = Mat3::from_diagonal(vec3(0.1, 4.0, 4.0));
        // Turn it a quarter turn about Z, so the rod now lies along Y.
        let rotation = Mat3::from_quat(Quat::from_axis_angle(Vec3::Z, FRAC_PI_2));
        let world = Mat3::transform_tensor(rotation, local);

        // The small moment of inertia has moved from X to Y.
        assert!((world.cols[1].y - 0.1).abs() < 1e-4, "{world:?}");
        assert!((world.cols[0].x - 4.0).abs() < 1e-4, "{world:?}");
    }

    #[test]
    fn determinant_measures_volume_scaling() {
        assert_eq!(Mat3::from_diagonal(vec3(2.0, 3.0, 4.0)).determinant(), 24.0);
        // Rotations preserve volume.
        let r = Mat3::from_quat(Quat::from_euler(0.3, 0.2, -0.5));
        assert!((r.determinant() - 1.0).abs() < 1e-5);
    }
}
