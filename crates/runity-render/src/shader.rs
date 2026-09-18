use crate::color::Color;
use crate::gbuffer::Surface;
use crate::texture::Texture;
use runity_math::{Mat4, Vec2, Vec3, Vec4};

/// A mesh vertex as it enters the pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    /// Tangent in model space; `w` is the bitangent's handedness (+1 or -1).
    /// Only normal mapping reads it.
    pub tangent: Vec4,
    pub uv: Vec2,
    pub color: Color,
}

impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            normal: Vec3::Y,
            tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
            uv: Vec2::ZERO,
            color: Color::WHITE,
        }
    }
}

impl Vertex {
    pub fn new(position: Vec3, normal: Vec3, uv: Vec2) -> Self {
        Self {
            position,
            normal,
            uv,
            ..Self::default()
        }
    }

    /// Tangent in model space; `w` carries the bitangent's handedness.
    pub fn with_tangent(mut self, tangent: Vec4) -> Self {
        self.tangent = tangent;
        self
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}

/// Data handed from the vertex stage to the fragment stage.
///
/// The rasterizer interpolates it with perspective correction, which only needs
/// these two operations — the same contract a GPU's interpolator provides.
pub trait Varying: Copy {
    fn scale(self, s: f32) -> Self;
    fn add(self, other: Self) -> Self;

    #[inline]
    fn lerp(self, other: Self, t: f32) -> Self {
        self.scale(1.0 - t).add(other.scale(t))
    }
}

impl Varying for f32 {
    #[inline]
    fn scale(self, s: f32) -> Self {
        self * s
    }
    #[inline]
    fn add(self, other: Self) -> Self {
        self + other
    }
}

impl Varying for Vec2 {
    #[inline]
    fn scale(self, s: f32) -> Self {
        self * s
    }
    #[inline]
    fn add(self, other: Self) -> Self {
        self + other
    }
}

impl Varying for Vec3 {
    #[inline]
    fn scale(self, s: f32) -> Self {
        self * s
    }
    #[inline]
    fn add(self, other: Self) -> Self {
        self + other
    }
}

impl Varying for Vec4 {
    #[inline]
    fn scale(self, s: f32) -> Self {
        self * s
    }
    #[inline]
    fn add(self, other: Self) -> Self {
        self + other
    }
}

impl Varying for Color {
    #[inline]
    fn scale(self, s: f32) -> Self {
        Color::rgba(self.r * s, self.g * s, self.b * s, self.a * s)
    }
    #[inline]
    fn add(self, o: Self) -> Self {
        Color::rgba(self.r + o.r, self.g + o.g, self.b + o.b, self.a + o.a)
    }
}

impl<A: Varying, B: Varying> Varying for (A, B) {
    #[inline]
    fn scale(self, s: f32) -> Self {
        (self.0.scale(s), self.1.scale(s))
    }
    #[inline]
    fn add(self, o: Self) -> Self {
        (self.0.add(o.0), self.1.add(o.1))
    }
}

/// What the vertex stage produces: a clip-space position plus interpolants.
#[derive(Debug, Clone, Copy)]
pub struct VertexOutput<V> {
    /// Homogeneous clip-space position. The near plane is `z == 0`.
    pub clip_position: Vec4,
    pub varying: V,
}

/// The programmable part of the pipeline.
pub trait Shader {
    type Varying: Varying;

    fn vertex(&self, vertex: &Vertex) -> VertexOutput<Self::Varying>;

    /// Linear-light color for this fragment, or `None` to discard it (the
    /// equivalent of GLSL's `discard`).
    ///
    /// In a forward pass this is the final color; in a geometry pass it is the
    /// albedo, and the lighting pass overwrites it later.
    fn fragment(&self, varying: &Self::Varying) -> Option<Color>;

    /// Surface parameters for the G-buffer.
    ///
    /// Returning `None` — the default — means this shader is forward-only and
    /// contributes nothing to the deferred passes.
    fn surface(&self, _varying: &Self::Varying) -> Option<Surface> {
        None
    }
}

/// Emits the vertex color, ignoring lights. Handy for gizmos and debugging.
pub struct UnlitShader<'a> {
    pub mvp: Mat4,
    pub tint: Color,
    pub texture: Option<&'a Texture>,
}

impl<'a> UnlitShader<'a> {
    pub fn new(mvp: Mat4) -> Self {
        Self {
            mvp,
            tint: Color::WHITE,
            texture: None,
        }
    }
}

impl Shader for UnlitShader<'_> {
    type Varying = (Color, Vec2);

    fn vertex(&self, v: &Vertex) -> VertexOutput<Self::Varying> {
        VertexOutput {
            clip_position: self.mvp.transform_point(v.position),
            varying: (v.color, v.uv),
        }
    }

    fn fragment(&self, (color, uv): &Self::Varying) -> Option<Color> {
        let base = match self.texture {
            Some(t) => t.sample(uv.x, uv.y),
            None => Color::WHITE,
        };
        Some(base.modulate(*color).modulate(self.tint))
    }
}
