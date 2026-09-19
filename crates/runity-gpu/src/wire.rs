//! The layouts the GPU actually reads.
//!
//! This module exists because of a fact that is easy to assume away:
//! `runity_render::Vertex`, `runity_math::Mat4` and `runity_math::Vec4` are
//! `repr(Rust)`, and Rust does not promise where a `repr(Rust)` struct puts
//! its fields. It is not a theoretical worry — on the compiler this was
//! written with, `Vertex` lays its four members out as
//!
//! ```text
//! color @ 0    uv @ 16    position @ 24    normal @ 36
//! ```
//!
//! which is 48 bytes with no holes, exactly as promised, and in none of the
//! order anybody would write down. A `memcpy` of a `&[Vertex]` into a Metal
//! buffer is therefore not a vertex buffer; it is noise that happens to be the
//! right length.
//!
//! So the wire format lives here, in `#[repr(C)]` types that mirror
//! `shader.metal` one member at a time, and an upload converts rather than
//! copies. The conversion costs a few nanoseconds per vertex and buys a layout
//! the language guarantees. Nothing in `runity-render` changes — this crate
//! takes its data types and describes its own bytes.

use runity_math::{Mat4, Vec4};
use runity_render::Vertex;

/// A `float4x4`, four columns in column order — the layout Metal reads and the
/// layout [`Mat4`] means.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Matrix(pub [f32; 16]);

impl From<Mat4> for Matrix {
    fn from(m: Mat4) -> Self {
        let mut out = [0.0; 16];
        for (index, column) in m.cols.iter().enumerate() {
            out[index * 4] = column.x;
            out[index * 4 + 1] = column.y;
            out[index * 4 + 2] = column.z;
            out[index * 4 + 3] = column.w;
        }
        Self(out)
    }
}

/// A `float4`. Every scalar a uniform block carries rides in one of these,
/// which is what keeps a block a multiple of 16 bytes on both sides.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Float4(pub [f32; 4]);

impl Float4 {
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self([x, y, z, w])
    }

    /// The `x` component, which is where a lone scalar goes.
    pub fn x(self) -> f32 {
        self.0[0]
    }
}

impl From<Vec4> for Float4 {
    fn from(v: Vec4) -> Self {
        Self([v.x, v.y, v.z, v.w])
    }
}

impl From<runity_render::Color> for Float4 {
    fn from(c: runity_render::Color) -> Self {
        Self([c.r, c.g, c.b, c.a])
    }
}

/// Byte-for-byte the `Vertex` struct in `shader.metal`: 12 + 12 + 8 + 16 = 48,
/// in that order, which is what `[[vertex_id]]` indexes.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GpuVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

impl From<&Vertex> for GpuVertex {
    fn from(v: &Vertex) -> Self {
        Self {
            position: [v.position.x, v.position.y, v.position.z],
            normal: [v.normal.x, v.normal.y, v.normal.z],
            uv: [v.uv.x, v.uv.y],
            color: [v.color.r, v.color.g, v.color.b, v.color.a],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::{Vec2, Vec3};
    use runity_render::Color;

    #[test]
    fn the_wire_vertex_is_forty_eight_bytes_in_the_declared_order() {
        assert_eq!(std::mem::size_of::<GpuVertex>(), 48);
        let v = GpuVertex::default();
        let base = &v as *const _ as usize;
        let offset = |p: *const f32| p as usize - base;
        assert_eq!(offset(v.position.as_ptr()), 0);
        assert_eq!(offset(v.normal.as_ptr()), 12);
        assert_eq!(offset(v.uv.as_ptr()), 24);
        assert_eq!(offset(v.color.as_ptr()), 32);
    }

    /// The reason this module exists, written down as a test: the engine's own
    /// vertex is the same size and a different shape, so uploading it needs a
    /// conversion and not a `memcpy`.
    #[test]
    fn the_engine_vertex_is_the_same_size_but_not_the_same_layout() {
        assert_eq!(
            std::mem::size_of::<Vertex>(),
            std::mem::size_of::<GpuVertex>()
        );
        let v = Vertex::new(
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(4.0, 5.0, 6.0),
            Vec2::new(7.0, 8.0),
        )
        .with_color(Color::rgba(9.0, 10.0, 11.0, 12.0));
        let wire = GpuVertex::from(&v);
        assert_eq!(wire.position, [1.0, 2.0, 3.0]);
        assert_eq!(wire.normal, [4.0, 5.0, 6.0]);
        assert_eq!(wire.uv, [7.0, 8.0]);
        assert_eq!(wire.color, [9.0, 10.0, 11.0, 12.0]);

        // Read the engine's vertex as raw words and the first one is not the
        // x of its position. If this ever starts passing as an equality, the
        // conversion is still correct — it is only then no longer necessary.
        // SAFETY: reading 48 bytes of a 48-byte struct as f32 words.
        let raw: [f32; 12] = unsafe { std::mem::transmute(v) };
        assert_ne!(
            raw[0], 1.0,
            "if Vertex ever became repr(C) this test should be deleted, not muted"
        );
    }

    #[test]
    fn a_matrix_goes_out_column_by_column() {
        let m = Mat4::from_translation(Vec3::new(7.0, 8.0, 9.0));
        let wire = Matrix::from(m);
        // A translation lives in the fourth column, which is words 12..16.
        assert_eq!(&wire.0[12..], &[7.0, 8.0, 9.0, 1.0]);
        assert_eq!(&wire.0[..4], &[1.0, 0.0, 0.0, 0.0]);
        assert_eq!(std::mem::size_of::<Matrix>(), 64);
    }

    #[test]
    fn a_colour_and_a_vector_both_reach_a_float4_in_order() {
        assert_eq!(
            Float4::from(Vec4::new(1.0, 2.0, 3.0, 4.0)),
            Float4::new(1.0, 2.0, 3.0, 4.0)
        );
        assert_eq!(
            Float4::from(Color::rgba(0.1, 0.2, 0.3, 0.4)),
            Float4::new(0.1, 0.2, 0.3, 0.4)
        );
        assert_eq!(std::mem::size_of::<Float4>(), 16);
    }
}
