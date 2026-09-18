use core::ops::{Add, AddAssign, Div, Index, Mul, MulAssign, Neg, Sub, SubAssign};

macro_rules! impl_binop {
    ($ty:ident, $trait:ident, $method:ident, $op:tt, $($field:ident),+) => {
        impl $trait for $ty {
            type Output = $ty;
            #[inline]
            fn $method(self, rhs: $ty) -> $ty {
                $ty { $($field: self.$field $op rhs.$field),+ }
            }
        }
        impl $trait<f32> for $ty {
            type Output = $ty;
            #[inline]
            fn $method(self, rhs: f32) -> $ty {
                $ty { $($field: self.$field $op rhs),+ }
            }
        }
        impl $trait<$ty> for f32 {
            type Output = $ty;
            #[inline]
            fn $method(self, rhs: $ty) -> $ty {
                $ty { $($field: self $op rhs.$field),+ }
            }
        }
    };
}

/// Two-component vector.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

/// Three-component vector.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Four-component vector. Also used for homogeneous (clip-space) positions.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[inline]
pub const fn vec2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

#[inline]
pub const fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

#[inline]
pub const fn vec4(x: f32, y: f32, z: f32, w: f32) -> Vec4 {
    Vec4 { x, y, z, w }
}

impl_binop!(Vec2, Add, add, +, x, y);
impl_binop!(Vec2, Sub, sub, -, x, y);
impl_binop!(Vec2, Mul, mul, *, x, y);
impl_binop!(Vec2, Div, div, /, x, y);
impl_binop!(Vec3, Add, add, +, x, y, z);
impl_binop!(Vec3, Sub, sub, -, x, y, z);
impl_binop!(Vec3, Mul, mul, *, x, y, z);
impl_binop!(Vec3, Div, div, /, x, y, z);
impl_binop!(Vec4, Add, add, +, x, y, z, w);
impl_binop!(Vec4, Sub, sub, -, x, y, z, w);
impl_binop!(Vec4, Mul, mul, *, x, y, z, w);
impl_binop!(Vec4, Div, div, /, x, y, z, w);

impl Vec2 {
    pub const ZERO: Vec2 = vec2(0.0, 0.0);
    pub const ONE: Vec2 = vec2(1.0, 1.0);

    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v }
    }
    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y
    }
    #[inline]
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    #[inline]
    pub fn normalized(self) -> Self {
        let len = self.length();
        if len > 0.0 {
            self * (1.0 / len)
        } else {
            Self::ZERO
        }
    }
    /// 2D cross product (signed area of the parallelogram).
    #[inline]
    pub fn cross(self, o: Self) -> f32 {
        self.x * o.y - self.y * o.x
    }
}

impl Vec3 {
    pub const ZERO: Vec3 = vec3(0.0, 0.0, 0.0);
    pub const ONE: Vec3 = vec3(1.0, 1.0, 1.0);
    pub const X: Vec3 = vec3(1.0, 0.0, 0.0);
    pub const Y: Vec3 = vec3(0.0, 1.0, 0.0);
    pub const Z: Vec3 = vec3(0.0, 0.0, 1.0);

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v, z: v }
    }
    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    #[inline]
    pub fn cross(self, o: Self) -> Self {
        vec3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }
    #[inline]
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    #[inline]
    pub fn normalized(self) -> Self {
        let len = self.length();
        if len > 0.0 {
            self * (1.0 / len)
        } else {
            Self::ZERO
        }
    }
    #[inline]
    pub fn min(self, o: Self) -> Self {
        vec3(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }
    #[inline]
    pub fn max(self, o: Self) -> Self {
        vec3(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }
    #[inline]
    pub fn lerp(self, o: Self, t: f32) -> Self {
        self + (o - self) * t
    }
    #[inline]
    pub fn extend(self, w: f32) -> Vec4 {
        vec4(self.x, self.y, self.z, w)
    }
    #[inline]
    pub fn xy(self) -> Vec2 {
        vec2(self.x, self.y)
    }
}

impl Vec4 {
    pub const ZERO: Vec4 = vec4(0.0, 0.0, 0.0, 0.0);
    pub const ONE: Vec4 = vec4(1.0, 1.0, 1.0, 1.0);

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }
    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self {
            x: v,
            y: v,
            z: v,
            w: v,
        }
    }
    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }
    #[inline]
    pub fn xyz(self) -> Vec3 {
        vec3(self.x, self.y, self.z)
    }
    /// Perspective divide. Returns NDC coordinates; `w` must be non-zero.
    #[inline]
    pub fn perspective_divide(self) -> Vec3 {
        let inv = 1.0 / self.w;
        vec3(self.x * inv, self.y * inv, self.z * inv)
    }
    #[inline]
    pub fn lerp(self, o: Self, t: f32) -> Self {
        self + (o - self) * t
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    #[inline]
    fn neg(self) -> Vec2 {
        vec2(-self.x, -self.y)
    }
}
impl Neg for Vec3 {
    type Output = Vec3;
    #[inline]
    fn neg(self) -> Vec3 {
        vec3(-self.x, -self.y, -self.z)
    }
}
impl Neg for Vec4 {
    type Output = Vec4;
    #[inline]
    fn neg(self) -> Vec4 {
        vec4(-self.x, -self.y, -self.z, -self.w)
    }
}

impl AddAssign for Vec3 {
    #[inline]
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}
impl SubAssign for Vec3 {
    #[inline]
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}
impl MulAssign<f32> for Vec3 {
    #[inline]
    fn mul_assign(&mut self, s: f32) {
        *self = *self * s;
    }
}

impl Index<usize> for Vec4 {
    type Output = f32;
    #[inline]
    fn index(&self, i: usize) -> &f32 {
        match i {
            0 => &self.x,
            1 => &self.y,
            2 => &self.z,
            3 => &self.w,
            _ => panic!("Vec4 index out of range: {i}"),
        }
    }
}

impl From<Vec3> for Vec4 {
    #[inline]
    fn from(v: Vec3) -> Vec4 {
        v.extend(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_is_right_handed() {
        assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
        assert_eq!(Vec3::Y.cross(Vec3::Z), Vec3::X);
    }

    #[test]
    fn normalize_zero_is_zero() {
        assert_eq!(Vec3::ZERO.normalized(), Vec3::ZERO);
    }

    #[test]
    fn scalar_ops_commute() {
        let v = vec3(1.0, 2.0, 3.0);
        assert_eq!(v * 2.0, 2.0 * v);
        assert_eq!((v * 2.0).length(), v.length() * 2.0);
    }
}
