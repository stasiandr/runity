//! Linear algebra for runity.
//!
//! Zero dependencies, `#![no_std]`-friendly in spirit: the only things we take
//! from `std` are `f32` intrinsics (`sqrt`, `sin`, `cos`, `tan`), which live in
//! `std` rather than `core`.

#![forbid(unsafe_code)]

mod frustum;
mod mat;
mod mat3;
mod noise;
mod quat;
mod rng;
mod vec;

pub use frustum::{Frustum, Plane};
pub use mat::Mat4;
pub use mat3::Mat3;
pub use noise::{Cell, Fbm, Noise};
pub use quat::Quat;
pub use rng::Rng;
pub use vec::{vec2, vec3, vec4, Vec2, Vec3, Vec4};

/// Linear interpolation.
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Clamp `v` into `[lo, hi]`.
#[inline]
pub fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}
