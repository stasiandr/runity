use crate::color::Color;
use crate::gbuffer::Surface;
use crate::texture::Texture;
use runity_math::{Mat4, Vec2, Vec3, Vec4};

/// A mesh vertex as it enters the pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
    pub color: Color,
}

impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            normal: Vec3::Y,
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
            color: Color::WHITE,
        }
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

/// Interpolants used by [`BasicShader`].
#[derive(Debug, Clone, Copy)]
pub struct BasicVarying {
    pub world_position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
    pub color: Color,
}

impl Varying for BasicVarying {
    #[inline]
    fn scale(self, s: f32) -> Self {
        Self {
            world_position: self.world_position * s,
            normal: self.normal * s,
            uv: self.uv * s,
            color: self.color.scale(s),
        }
    }
    #[inline]
    fn add(self, o: Self) -> Self {
        Self {
            world_position: self.world_position + o.world_position,
            normal: self.normal + o.normal,
            uv: self.uv + o.uv,
            color: self.color.add(o.color),
        }
    }
}

/// A directional light with an ambient term.
#[derive(Debug, Clone, Copy)]
pub struct DirectionalLight {
    /// Direction the light travels, in world space.
    pub direction: Vec3,
    pub color: Color,
    pub intensity: f32,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            direction: Vec3::new(-0.4, -1.0, -0.5).normalized(),
            color: Color::WHITE,
            intensity: 1.0,
        }
    }
}

/// Lambert diffuse + Blinn-Phong specular over an optional texture.
///
/// This is deliberately a plain `Shader` implementation: anything it does, a
/// user-written shader can do too.
pub struct BasicShader<'a> {
    pub model: Mat4,
    pub view_projection: Mat4,
    pub normal_matrix: Mat4,
    pub base_color: Color,
    pub texture: Option<&'a Texture>,
    pub light: DirectionalLight,
    pub ambient: Color,
    pub camera_position: Vec3,
    pub specular_strength: f32,
    pub shininess: f32,
}

impl<'a> BasicShader<'a> {
    pub fn new(model: Mat4, view_projection: Mat4) -> Self {
        Self {
            model,
            view_projection,
            normal_matrix: model.normal_matrix(),
            base_color: Color::WHITE,
            texture: None,
            light: DirectionalLight::default(),
            ambient: Color::rgb(0.12, 0.13, 0.16),
            camera_position: Vec3::ZERO,
            specular_strength: 0.25,
            shininess: 32.0,
        }
    }

    pub fn with_texture(mut self, texture: &'a Texture) -> Self {
        self.texture = Some(texture);
        self
    }

    pub fn with_base_color(mut self, color: Color) -> Self {
        self.base_color = color;
        self
    }

    pub fn with_light(mut self, light: DirectionalLight) -> Self {
        self.light = light;
        self
    }

    pub fn with_camera_position(mut self, eye: Vec3) -> Self {
        self.camera_position = eye;
        self
    }
}

impl Shader for BasicShader<'_> {
    type Varying = BasicVarying;

    fn vertex(&self, v: &Vertex) -> VertexOutput<Self::Varying> {
        let world = self.model.transform_point(v.position);
        VertexOutput {
            clip_position: self.view_projection * world,
            varying: BasicVarying {
                world_position: world.xyz(),
                normal: self.normal_matrix.transform_vector(v.normal),
                uv: v.uv,
                color: v.color,
            },
        }
    }

    fn fragment(&self, f: &Self::Varying) -> Option<Color> {
        let albedo = match self.texture {
            Some(t) => t.sample(f.uv.x, f.uv.y).modulate(self.base_color),
            None => self.base_color,
        }
        .modulate(f.color);

        let n = f.normal.normalized();
        let to_light = -self.light.direction.normalized();
        let diffuse = n.dot(to_light).max(0.0) * self.light.intensity;

        let specular = if diffuse > 0.0 && self.specular_strength > 0.0 {
            let to_eye = (self.camera_position - f.world_position).normalized();
            let half = (to_light + to_eye).normalized();
            n.dot(half).max(0.0).powf(self.shininess) * self.specular_strength
        } else {
            0.0
        };

        let lit = Color::rgba(
            albedo.r * (self.ambient.r + self.light.color.r * diffuse) + specular,
            albedo.g * (self.ambient.g + self.light.color.g * diffuse) + specular,
            albedo.b * (self.ambient.b + self.light.color.b * diffuse) + specular,
            albedo.a,
        );
        Some(lit)
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
