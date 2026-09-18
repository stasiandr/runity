//! Linear algebra for runity.
//!
//! Zero dependencies, `#![no_std]`-friendly in spirit: the only things we take
//! from `std` are `f32` intrinsics (`sqrt`, `sin`, `cos`, `tan`), which live in
//! `std` rather than `core`.

#![forbid(unsafe_code)]

mod mat;
mod quat;
mod vec;

pub use mat::Mat4;
pub use quat::Quat;
pub use vec::{Vec2, Vec3, Vec4};

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
