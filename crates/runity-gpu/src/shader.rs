//! The Rust half of the shader pairs, and the trait that ties the two halves
//! together.
//!
//! A shader that runs on the GPU is written twice: once as a
//! [`runity_render::Shader`], which is what the rasterizer executes and what
//! the golden images are made of, and once as Metal Shading Language, which is
//! what the hardware executes. [`GpuShader`] is the seam. It is deliberately
//! not derivable and not generated: the two halves are two programs, and the
//! only thing that proves they agree is a differential test.
//!
//! The MSL sits next to this file, in `shader.metal`, and is pulled in with
//! `include_str!` — one source, one library, compiled once per process.

use crate::wire::{Float4, Matrix};
use runity_math::Mat4;
use runity_render::{BasicShader, Shader, Texture, UnlitShader};

/// The Metal half of every shader the engine ships.
pub const ENGINE_SOURCE: &str = include_str!("shader.metal");

/// A [`Shader`] that also exists in Metal Shading Language.
///
/// Implementing it is a promise: `SOURCE` contains functions named [`VERTEX`]
/// and [`FRAGMENT`], and running them produces the same picture the `Shader`
/// half produces. Write a differential test — [`crate::diff::assert_agrees`] —
/// or the promise is just a comment.
///
/// [`VERTEX`]: GpuShader::VERTEX
/// [`FRAGMENT`]: GpuShader::FRAGMENT
pub trait GpuShader: Shader {
    /// MSL source carrying both stages. Libraries are cached per source, so
    /// shaders sharing one file are compiled once between them.
    const SOURCE: &'static str;
    /// Name of the `[[vertex]]` function inside [`SOURCE`](GpuShader::SOURCE).
    const VERTEX: &'static str;
    /// Name of the `[[fragment]]` function inside [`SOURCE`](GpuShader::SOURCE).
    const FRAGMENT: &'static str;

    /// The uniform block, laid out exactly as the MSL `constant` struct is.
    ///
    /// Matrices first, then `float4`s, padded by hand; nothing narrower is
    /// allowed, which is what keeps `size_of` a multiple of 16 and the two
    /// layouts from drifting. Build it out of [`Matrix`] and [`Float4`] — the
    /// `repr(C)` spellings in [`crate::wire`] — because `Mat4` and `Vec4`
    /// themselves are `repr(Rust)` and promise nothing about where their
    /// members sit. [`assert_uniform_layout`] checks the size, and the echo
    /// shader checks every field.
    ///
    /// [`Matrix`]: crate::wire::Matrix
    /// [`Float4`]: crate::wire::Float4
    type Uniforms: Copy + 'static;

    /// This frame's values for the block.
    fn uniforms(&self) -> Self::Uniforms;

    /// The texture bound at `texture(0)`, if the shader samples one.
    fn texture(&self) -> Option<&Texture> {
        None
    }
}

/// Panics unless `U` can be a uniform block: Metal rounds a `constant` struct
/// up to a 16-byte boundary, so a Rust struct that is not already a multiple
/// of 16 describes a different block than the MSL one does.
///
/// Call it from a test, or from a constructor — it is cheap either way.
#[track_caller]
pub fn assert_uniform_layout<U: Copy + 'static>() {
    let size = core::mem::size_of::<U>();
    assert!(
        size % 16 == 0,
        "a uniform block must be a multiple of 16 bytes; {} is {size}. \
         Pad it by hand — Metal will.",
        core::any::type_name::<U>()
    );
}

// ---------------------------------------------------------------------------
// BasicShader
// ---------------------------------------------------------------------------

/// The uniform block `basic_vertex` and `basic_fragment` read.
///
/// The packing is not arbitrary: three matrices, then six `float4`s, each
/// carrying a scalar in the component the CPU shader has no other use for.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicUniforms {
    pub model: Matrix,
    pub view_projection: Matrix,
    pub normal_matrix: Matrix,
    pub base_color: Float4,
    /// `xyz`: the direction the light travels. `w`: its intensity.
    pub light: Float4,
    /// `rgb`: the light's colour. `w`: specular strength.
    pub light_color: Float4,
    /// `rgb`: the ambient term.
    pub ambient: Float4,
    /// `xyz`: the eye. `w`: shininess.
    pub camera: Float4,
    /// `x`: 1 when a texture is bound, 0 otherwise.
    pub flags: Float4,
}

impl GpuShader for BasicShader<'_> {
    const SOURCE: &'static str = ENGINE_SOURCE;
    const VERTEX: &'static str = "basic_vertex";
    const FRAGMENT: &'static str = "basic_fragment";
    type Uniforms = BasicUniforms;

    fn uniforms(&self) -> BasicUniforms {
        let d = self.light.direction;
        let c = self.light.color;
        let a = self.ambient;
        let b = self.base_color;
        let e = self.camera_position;
        BasicUniforms {
            model: self.model.into(),
            view_projection: self.view_projection.into(),
            normal_matrix: self.normal_matrix.into(),
            base_color: Float4::new(b.r, b.g, b.b, b.a),
            light: Float4::new(d.x, d.y, d.z, self.light.intensity),
            light_color: Float4::new(c.r, c.g, c.b, self.specular_strength),
            ambient: Float4::new(a.r, a.g, a.b, a.a),
            camera: Float4::new(e.x, e.y, e.z, self.shininess),
            flags: Float4::new(self.texture.is_some() as u32 as f32, 0.0, 0.0, 0.0),
        }
    }

    fn texture(&self) -> Option<&Texture> {
        self.texture
    }
}

// ---------------------------------------------------------------------------
// UnlitShader
// ---------------------------------------------------------------------------

/// The uniform block `unlit_vertex` and `unlit_fragment` read.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnlitUniforms {
    pub mvp: Matrix,
    pub tint: Float4,
    /// `x`: 1 when a texture is bound.
    pub flags: Float4,
}

impl GpuShader for UnlitShader<'_> {
    const SOURCE: &'static str = ENGINE_SOURCE;
    const VERTEX: &'static str = "unlit_vertex";
    const FRAGMENT: &'static str = "unlit_fragment";
    type Uniforms = UnlitUniforms;

    fn uniforms(&self) -> UnlitUniforms {
        let t = self.tint;
        UnlitUniforms {
            mvp: self.mvp.into(),
            tint: Float4::new(t.r, t.g, t.b, t.a),
            flags: Float4::new(self.texture.is_some() as u32 as f32, 0.0, 0.0, 0.0),
        }
    }

    fn texture(&self) -> Option<&Texture> {
        self.texture
    }
}

// ---------------------------------------------------------------------------
// The pulse
// ---------------------------------------------------------------------------

/// The uniform block `pulse_vertex` and `pulse_fragment` read.
///
/// Two shaders in this repository are this shader — `PulseShader` in the
/// showcase and `Gradient` in `hello_triangle` — so the block and the MSL are
/// shared and only the Rust halves are written twice.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PulseUniforms {
    pub mvp: Matrix,
    /// `x`: the brightness the fragment stage multiplies in.
    pub params: Float4,
}

impl PulseUniforms {
    pub fn new(mvp: Mat4, pulse: f32) -> Self {
        Self {
            mvp: mvp.into(),
            params: Float4::new(pulse, 0.0, 0.0, 0.0),
        }
    }
}

/// The names an implementor of the pulse pairs with [`PulseUniforms`].
pub const PULSE_VERTEX: &str = "pulse_vertex";
/// See [`PULSE_VERTEX`].
pub const PULSE_FRAGMENT: &str = "pulse_fragment";

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::Vec3;
    use runity_render::Color;

    #[test]
    fn every_uniform_block_is_a_multiple_of_sixteen_bytes() {
        assert_uniform_layout::<BasicUniforms>();
        assert_uniform_layout::<UnlitUniforms>();
        assert_uniform_layout::<PulseUniforms>();
        // And the sizes MSL will compute for the same declarations.
        assert_eq!(core::mem::size_of::<BasicUniforms>(), 3 * 64 + 6 * 16);
        assert_eq!(core::mem::size_of::<UnlitUniforms>(), 64 + 2 * 16);
        assert_eq!(core::mem::size_of::<PulseUniforms>(), 64 + 16);
    }

    #[test]
    fn the_basic_uniforms_carry_what_the_cpu_shader_reads() {
        let shader = BasicShader::new(Mat4::IDENTITY, Mat4::IDENTITY)
            .with_base_color(Color::rgba(0.1, 0.2, 0.3, 0.4))
            .with_camera_position(Vec3::new(1.0, 2.0, 3.0));
        let u = shader.uniforms();
        assert_eq!(u.base_color, Float4::new(0.1, 0.2, 0.3, 0.4));
        assert_eq!(u.camera, Float4::new(1.0, 2.0, 3.0, shader.shininess));
        assert_eq!(u.light.0[3], shader.light.intensity);
        assert_eq!(u.light_color.0[3], shader.specular_strength);
        assert_eq!(u.flags.x(), 0.0, "no texture bound");

        let texture = Texture::solid(Color::WHITE);
        let shader = shader.with_texture(&texture);
        assert_eq!(shader.uniforms().flags.x(), 1.0);
    }

    #[test]
    fn the_engine_source_declares_every_function_the_traits_name() {
        for name in [
            <BasicShader as GpuShader>::VERTEX,
            <BasicShader as GpuShader>::FRAGMENT,
            <UnlitShader as GpuShader>::VERTEX,
            <UnlitShader as GpuShader>::FRAGMENT,
            PULSE_VERTEX,
            PULSE_FRAGMENT,
            "echo_vertex",
            "echo_uniform_fragment",
            "echo_vertex_fragment",
            "line_vertex",
            "line_fragment",
        ] {
            assert!(
                ENGINE_SOURCE.contains(&format!("{name}(")),
                "{name} is named in Rust but missing from shader.metal"
            );
        }
    }
}
