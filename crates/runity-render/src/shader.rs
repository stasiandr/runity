use crate::color::Color;
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

    /// Return `None` to discard the fragment (an alpha-test / `discard`).
    fn fragment(&self, varying: &Self::Varying) -> Option<Color>;
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

/// Linear distance fog: blends a fragment's color toward [`Fog::color`] as its
/// distance from the camera goes from `start` to `end`. Alpha is left alone —
/// fog thickens what you see, it does not make it transparent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fog {
    pub color: Color,
    /// Distance at which the fog has no effect yet.
    pub start: f32,
    /// Distance beyond which a fragment is entirely the fog color.
    pub end: f32,
}

impl Fog {
    /// Blend `color` toward [`Fog::color`] for a fragment `distance` away from
    /// the camera. Distances outside `[start, end]` saturate rather than
    /// extrapolate.
    fn apply(&self, color: Color, distance: f32) -> Color {
        let t = if self.end > self.start {
            ((distance - self.start) / (self.end - self.start)).clamp(0.0, 1.0)
        } else if distance >= self.start {
            1.0
        } else {
            0.0
        };
        let blended = color.lerp(self.color, t);
        Color::rgba(blended.r, blended.g, blended.b, color.a)
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
    /// Discard a fragment whose final alpha is below this threshold instead of
    /// drawing it. `None` (the default) disables the test.
    pub alpha_cutoff: Option<f32>,
    /// Distance fog mixed into the final color. `None` (the default) disables it.
    pub fog: Option<Fog>,
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
            alpha_cutoff: None,
            fog: None,
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

    pub fn with_alpha_cutoff(mut self, threshold: f32) -> Self {
        self.alpha_cutoff = Some(threshold);
        self
    }

    pub fn with_fog(mut self, fog: Fog) -> Self {
        self.fog = Some(fog);
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

        if self.alpha_cutoff.is_some_and(|threshold| lit.a < threshold) {
            return None;
        }

        Some(match self.fog {
            Some(fog) => {
                let distance = (self.camera_position - f.world_position).length();
                fog.apply(lit, distance)
            }
            None => lit,
        })
    }
}

/// Interpolants used by [`UnlitShader`].
#[derive(Debug, Clone, Copy)]
pub struct UnlitVarying {
    pub color: Color,
    pub uv: Vec2,
    /// Distance from the camera along its view axis, for [`Fog`].
    ///
    /// `UnlitShader` only ever sees a combined `mvp`, so it has no world
    /// position to measure a true Euclidean distance from. This is instead the
    /// clip-space `w` a perspective projection already produces (`-z` in view
    /// space) recovered per-fragment by storing it as an ordinary
    /// perspective-interpolated attribute: `w_i * (w_clip_i / w_clip_i) = w_i`
    /// sums to `1`, which perspective-correct interpolation then divides back
    /// out to the exact per-fragment `w`.
    pub view_distance: f32,
}

impl Varying for UnlitVarying {
    #[inline]
    fn scale(self, s: f32) -> Self {
        Self {
            color: self.color.scale(s),
            uv: self.uv * s,
            view_distance: self.view_distance * s,
        }
    }
    #[inline]
    fn add(self, o: Self) -> Self {
        Self {
            color: self.color.add(o.color),
            uv: self.uv + o.uv,
            view_distance: self.view_distance + o.view_distance,
        }
    }
}

/// Emits the vertex color, ignoring lights. Handy for gizmos and debugging.
pub struct UnlitShader<'a> {
    pub mvp: Mat4,
    pub tint: Color,
    pub texture: Option<&'a Texture>,
    /// Discard a fragment whose final alpha is below this threshold instead of
    /// drawing it. `None` (the default) disables the test.
    pub alpha_cutoff: Option<f32>,
    /// Distance fog mixed into the final color. `None` (the default) disables it.
    pub fog: Option<Fog>,
}

impl<'a> UnlitShader<'a> {
    pub fn new(mvp: Mat4) -> Self {
        Self {
            mvp,
            tint: Color::WHITE,
            texture: None,
            alpha_cutoff: None,
            fog: None,
        }
    }

    pub fn with_alpha_cutoff(mut self, threshold: f32) -> Self {
        self.alpha_cutoff = Some(threshold);
        self
    }

    pub fn with_fog(mut self, fog: Fog) -> Self {
        self.fog = Some(fog);
        self
    }
}

impl Shader for UnlitShader<'_> {
    type Varying = UnlitVarying;

    fn vertex(&self, v: &Vertex) -> VertexOutput<Self::Varying> {
        let clip_position = self.mvp.transform_point(v.position);
        VertexOutput {
            clip_position,
            varying: UnlitVarying {
                color: v.color,
                uv: v.uv,
                view_distance: clip_position.w,
            },
        }
    }

    fn fragment(&self, f: &Self::Varying) -> Option<Color> {
        let base = match self.texture {
            Some(t) => t.sample(f.uv.x, f.uv.y),
            None => Color::WHITE,
        };
        let color = base.modulate(f.color).modulate(self.tint);

        if self
            .alpha_cutoff
            .is_some_and(|threshold| color.a < threshold)
        {
            return None;
        }

        Some(match self.fog {
            Some(fog) => fog.apply(color, f.view_distance),
            None => color,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic_varying_at(world_position: Vec3, color: Color) -> BasicVarying {
        BasicVarying {
            world_position,
            normal: Vec3::Y,
            uv: Vec2::ZERO,
            color,
        }
    }

    #[test]
    fn basic_shader_keeps_fragments_by_default() {
        let shader = BasicShader::new(Mat4::IDENTITY, Mat4::IDENTITY);
        let v = basic_varying_at(Vec3::ZERO, Color::rgba(1.0, 1.0, 1.0, 0.0));
        assert!(shader.fragment(&v).is_some());
    }

    #[test]
    fn basic_shader_alpha_cutoff_discards_below_and_keeps_above() {
        let shader = BasicShader::new(Mat4::IDENTITY, Mat4::IDENTITY).with_alpha_cutoff(0.5);
        let below = basic_varying_at(Vec3::ZERO, Color::rgba(1.0, 1.0, 1.0, 0.4));
        let above = basic_varying_at(Vec3::ZERO, Color::rgba(1.0, 1.0, 1.0, 0.6));
        assert!(shader.fragment(&below).is_none());
        assert!(shader.fragment(&above).is_some());
    }

    #[test]
    fn basic_shader_fog_leaves_near_fragments_alone_and_dominates_far_ones() {
        let fog = Fog {
            color: Color::rgb(0.7, 0.75, 0.85),
            start: 10.0,
            end: 20.0,
        };
        let unfogged = BasicShader::new(Mat4::IDENTITY, Mat4::IDENTITY)
            .with_base_color(Color::rgb(1.0, 0.2, 0.2))
            .with_camera_position(Vec3::ZERO);
        let fogged = BasicShader {
            fog: Some(fog),
            ..BasicShader::new(Mat4::IDENTITY, Mat4::IDENTITY)
                .with_base_color(Color::rgb(1.0, 0.2, 0.2))
                .with_camera_position(Vec3::ZERO)
        };

        let near = basic_varying_at(Vec3::new(0.0, 0.0, -1.0), Color::WHITE);
        let far = basic_varying_at(Vec3::new(0.0, 0.0, -1000.0), Color::WHITE);

        assert_eq!(fogged.fragment(&near), unfogged.fragment(&near));

        let far_unfogged = unfogged.fragment(&far).unwrap();
        assert_eq!(
            fogged.fragment(&far),
            Some(Color::rgba(
                fog.color.r,
                fog.color.g,
                fog.color.b,
                far_unfogged.a
            ))
        );
    }

    #[test]
    fn unlit_shader_keeps_fragments_by_default() {
        let shader = UnlitShader::new(Mat4::IDENTITY);
        let v = UnlitVarying {
            color: Color::rgba(1.0, 1.0, 1.0, 0.0),
            uv: Vec2::ZERO,
            view_distance: 1.0,
        };
        assert!(shader.fragment(&v).is_some());
    }

    #[test]
    fn unlit_shader_alpha_cutoff_discards_below_and_keeps_above() {
        let shader = UnlitShader::new(Mat4::IDENTITY).with_alpha_cutoff(0.5);
        let below = UnlitVarying {
            color: Color::rgba(1.0, 1.0, 1.0, 0.4),
            uv: Vec2::ZERO,
            view_distance: 1.0,
        };
        let above = UnlitVarying {
            color: Color::rgba(1.0, 1.0, 1.0, 0.6),
            uv: Vec2::ZERO,
            view_distance: 1.0,
        };
        assert!(shader.fragment(&below).is_none());
        assert!(shader.fragment(&above).is_some());
    }

    #[test]
    fn unlit_shader_fog_leaves_near_fragments_alone_and_dominates_far_ones() {
        let fog = Fog {
            color: Color::rgb(0.7, 0.75, 0.85),
            start: 10.0,
            end: 20.0,
        };
        let shader = UnlitShader {
            fog: Some(fog),
            ..UnlitShader::new(Mat4::IDENTITY)
        };

        let near = UnlitVarying {
            color: Color::WHITE,
            uv: Vec2::ZERO,
            view_distance: 1.0,
        };
        let far = UnlitVarying {
            color: Color::WHITE,
            uv: Vec2::ZERO,
            view_distance: 1000.0,
        };

        assert_eq!(shader.fragment(&near), Some(Color::WHITE));
        assert_eq!(
            shader.fragment(&far),
            Some(Color::rgba(fog.color.r, fog.color.g, fog.color.b, 1.0))
        );
    }
}
