//! The geometry pass: the shader that fills the G-buffer.
//!
//! It does no lighting at all. Its whole job is to answer, per pixel, "what
//! surface is here" — albedo, normal, position, roughness, metallic — so the
//! deferred passes can light it once, no matter how many triangles piled up on
//! that pixel.

use crate::color::Color;
use crate::gbuffer::Surface;
use crate::pbr::Material;
use crate::shader::{Shader, Varying, Vertex, VertexOutput};
use runity_math::{Mat4, Vec2, Vec3, Vec4};

/// What the geometry pass interpolates across a triangle.
#[derive(Debug, Clone, Copy)]
pub struct GeometryVarying {
    pub world_position: Vec3,
    pub normal: Vec3,
    pub tangent: Vec3,
    pub bitangent: Vec3,
    pub uv: Vec2,
    pub color: Color,
}

impl Varying for GeometryVarying {
    #[inline]
    fn scale(self, s: f32) -> Self {
        Self {
            world_position: self.world_position * s,
            normal: self.normal * s,
            tangent: self.tangent * s,
            bitangent: self.bitangent * s,
            uv: self.uv * s,
            color: self.color.scale(s),
        }
    }

    #[inline]
    fn add(self, o: Self) -> Self {
        Self {
            world_position: self.world_position + o.world_position,
            normal: self.normal + o.normal,
            tangent: self.tangent + o.tangent,
            bitangent: self.bitangent + o.bitangent,
            uv: self.uv + o.uv,
            color: self.color.add(o.color),
        }
    }
}

/// Writes a [`Material`] into the G-buffer.
pub struct GeometryShader<'a> {
    pub model: Mat4,
    pub normal_matrix: Mat4,
    pub view_projection: Mat4,
    pub material: Material<'a>,
}

impl<'a> GeometryShader<'a> {
    pub fn new(model: Mat4, view_projection: Mat4, material: Material<'a>) -> Self {
        Self {
            model,
            normal_matrix: model.normal_matrix(),
            view_projection,
            material,
        }
    }

    /// Albedo and alpha at these coordinates, before lighting.
    fn base_color(&self, varying: &GeometryVarying) -> Color {
        let uv = varying.uv * self.material.uv_scale;
        let sampled = match self.material.base_color_texture {
            Some(texture) => texture.sample(uv.x, uv.y),
            None => Color::WHITE,
        };
        sampled
            .modulate(self.material.base_color)
            .modulate(varying.color)
    }

    /// Roughness and metallic, from the glTF-style packed texture if present.
    fn surface_parameters(&self, varying: &GeometryVarying) -> (f32, f32) {
        let uv = varying.uv * self.material.uv_scale;
        match self.material.metallic_roughness_texture {
            // glTF packs roughness in green and metallic in blue.
            Some(texture) => {
                let sample = texture.sample(uv.x, uv.y);
                (
                    (self.material.roughness * sample.g).clamp(0.0, 1.0),
                    (self.material.metallic * sample.b).clamp(0.0, 1.0),
                )
            }
            None => (
                self.material.roughness.clamp(0.0, 1.0),
                self.material.metallic.clamp(0.0, 1.0),
            ),
        }
    }

    /// The shading normal: the interpolated one, or the normal map rotated into
    /// world space by the tangent frame.
    fn shading_normal(&self, varying: &GeometryVarying) -> Vec3 {
        let normal = varying.normal.normalized();
        let Some(map) = self.material.normal_texture else {
            return normal;
        };
        let uv = varying.uv * self.material.uv_scale;
        let sample = map.sample(uv.x, uv.y);
        // Normal maps store a direction in [0,1]; unpack it to [-1,1].
        let tangent_space = Vec3::new(
            sample.r * 2.0 - 1.0,
            sample.g * 2.0 - 1.0,
            sample.b * 2.0 - 1.0,
        );
        let tangent = varying.tangent;
        let bitangent = varying.bitangent;
        if tangent.length_squared() <= 1e-12 {
            return normal;
        }
        // Re-orthogonalize: interpolation across a triangle does not preserve
        // perpendicularity.
        let t = (tangent - normal * normal.dot(tangent)).normalized();
        let b = bitangent.normalized();
        (t * tangent_space.x + b * tangent_space.y + normal * tangent_space.z).normalized()
    }
}

impl Shader for GeometryShader<'_> {
    type Varying = GeometryVarying;

    fn vertex(&self, vertex: &Vertex) -> VertexOutput<Self::Varying> {
        let world = self.model.transform_point(vertex.position).xyz();
        let normal = self.normal_matrix.transform_vector(vertex.normal);
        let tangent = self.model.transform_vector(Vec3::new(
            vertex.tangent.x,
            vertex.tangent.y,
            vertex.tangent.z,
        ));
        // The bitangent is derived, not stored: one float instead of three.
        let bitangent = normal.cross(tangent) * vertex.tangent.w;

        VertexOutput {
            clip_position: self.view_projection * Vec4::new(world.x, world.y, world.z, 1.0),
            varying: GeometryVarying {
                world_position: world,
                normal,
                tangent,
                bitangent,
                uv: vertex.uv,
                color: vertex.color,
            },
        }
    }

    fn fragment(&self, varying: &Self::Varying) -> Option<Color> {
        let color = self.base_color(varying);
        if let Some(cutoff) = self.material.alpha_cutoff {
            if color.a < cutoff {
                return None; // alpha test, before anything is written
            }
        }
        Some(color)
    }

    fn surface(&self, varying: &Self::Varying) -> Option<Surface> {
        let albedo = self.base_color(varying);
        if let Some(cutoff) = self.material.alpha_cutoff {
            if albedo.a < cutoff {
                return None;
            }
        }
        let (roughness, metallic) = self.surface_parameters(varying);
        let uv = varying.uv * self.material.uv_scale;
        let emissive = match self.material.emissive_texture {
            Some(texture) => texture.sample(uv.x, uv.y).modulate(self.material.emissive),
            None => self.material.emissive,
        };

        Some(Surface {
            albedo,
            normal: self.shading_normal(varying),
            position: varying.world_position,
            roughness,
            metallic,
            occlusion: self.material.occlusion,
            emissive,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture::Texture;

    fn varying() -> GeometryVarying {
        GeometryVarying {
            world_position: Vec3::ZERO,
            normal: Vec3::Y,
            tangent: Vec3::X,
            bitangent: Vec3::Z,
            uv: Vec2::new(0.5, 0.5),
            color: Color::WHITE,
        }
    }

    #[test]
    fn the_surface_carries_the_material_through() {
        let material = Material {
            roughness: 0.3,
            metallic: 1.0,
            ..Material::default()
        }
        .with_emissive(Color::rgb(2.0, 0.0, 0.0));
        let shader = GeometryShader::new(Mat4::IDENTITY, Mat4::IDENTITY, material);
        let surface = shader.surface(&varying()).expect("not discarded");
        assert_eq!(surface.roughness, 0.3);
        assert_eq!(surface.metallic, 1.0);
        assert_eq!(surface.emissive.r, 2.0);
        assert!(surface.is_geometry());
    }

    #[test]
    fn alpha_below_the_cutoff_discards_the_fragment() {
        let material = Material {
            base_color: Color::rgba(1.0, 1.0, 1.0, 0.2),
            alpha_cutoff: Some(0.5),
            ..Material::default()
        };
        let shader = GeometryShader::new(Mat4::IDENTITY, Mat4::IDENTITY, material);
        assert!(shader.fragment(&varying()).is_none());
        assert!(
            shader.surface(&varying()).is_none(),
            "and writes nothing to the G-buffer"
        );
    }

    #[test]
    fn a_normal_map_tilts_the_shading_normal() {
        // A map pointing along +U in tangent space, which here is world +X.
        let tilted = Texture::solid(Color::rgb(1.0, 0.5, 0.5));
        let material = Material::default().with_normal_map(&tilted);
        let shader = GeometryShader::new(Mat4::IDENTITY, Mat4::IDENTITY, material);
        let normal = shader.shading_normal(&varying());
        assert!(normal.x > 0.7, "should lean along the tangent: {normal:?}");
        assert!((normal.length() - 1.0).abs() < 1e-5);

        // Without a map, the interpolated normal comes back untouched.
        let plain = GeometryShader::new(Mat4::IDENTITY, Mat4::IDENTITY, Material::default());
        assert_eq!(plain.shading_normal(&varying()), Vec3::Y);
    }

    #[test]
    fn a_flat_normal_map_leaves_the_normal_alone() {
        let flat = Texture::solid(Color::rgb(0.5, 0.5, 1.0));
        let material = Material::default().with_normal_map(&flat);
        let shader = GeometryShader::new(Mat4::IDENTITY, Mat4::IDENTITY, material);
        let normal = shader.shading_normal(&varying());
        assert!((normal - Vec3::Y).length() < 1e-4, "{normal:?}");
    }

    #[test]
    fn the_vertex_stage_puts_geometry_in_world_space() {
        let model = Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0));
        let shader = GeometryShader::new(model, Mat4::IDENTITY, Material::default());
        let out = shader.vertex(&Vertex::new(Vec3::ZERO, Vec3::Y, Vec2::ZERO));
        assert_eq!(out.varying.world_position, Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(out.varying.normal, Vec3::Y);
    }
}
