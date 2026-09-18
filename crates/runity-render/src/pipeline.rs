//! The deferred renderer: geometry once, lighting once, effects after.
//!
//! ```text
//! begin_frame   clear, attach the G-buffer
//! draw          geometry pass: rasterize surfaces into the G-buffer
//! shade         lighting pass: PBR + image-based lighting, then the sky
//!               behind everything that has no geometry
//! ```
//!
//! Splitting it this way is what makes the screen-space effects possible at
//! all: by the time lighting runs, every pixel knows its normal, position and
//! material, and a pass can walk the buffer instead of the scene.

use crate::color::Color;
use crate::framebuffer::Framebuffer;
use crate::gbuffer::Surface;
use crate::geometry::GeometryShader;
use crate::mesh::Mesh;
use crate::pbr::{
    direct_light, environment_brdf, f0_for, fresnel_schlick_roughness, Light, Material,
};
use crate::raster::{DrawStats, Rasterizer};
use crate::sky::Sky;
use crate::view::CameraView;
use runity_math::{Mat4, Vec3};

/// What the renderer does beyond plain shading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderSettings {
    /// Fill pixels with no geometry with the sky rather than the clear color.
    pub draw_sky: bool,
    /// Multiplier on image-based lighting.
    pub ambient_intensity: f32,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            draw_sky: true,
            ambient_intensity: 1.0,
        }
    }
}

/// Deferred renderer: owns the lights, the environment and the passes.
pub struct Renderer {
    pub rasterizer: Rasterizer,
    pub sky: Sky,
    pub lights: Vec<Light>,
    pub settings: RenderSettings,
    camera: CameraView,
    stats: DrawStats,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new(Sky::default())
    }
}

impl Renderer {
    pub fn new(sky: Sky) -> Self {
        // The sun is a light like any other; the sky just knows where it is.
        let lights = sky.sun_light().into_iter().collect();
        Self {
            rasterizer: Rasterizer::new(),
            sky,
            lights,
            settings: RenderSettings::default(),
            camera: CameraView::new(Mat4::IDENTITY, Mat4::IDENTITY, Vec3::ZERO, 0.1, 100.0),
            stats: DrawStats::default(),
        }
    }

    /// Replace the environment, and the sun light that came with it.
    pub fn set_sky(&mut self, sky: Sky) {
        self.lights
            .retain(|light| !light.casts_shadow || light.position().is_some());
        if let Some(sun) = sky.sun_light() {
            self.lights.insert(0, sun);
        }
        self.sky = sky;
    }

    pub fn camera(&self) -> &CameraView {
        &self.camera
    }

    pub fn stats(&self) -> DrawStats {
        self.stats
    }

    /// Start a frame: attach the G-buffer, clear everything, remember the camera.
    ///
    /// `clear_color` is what stays where nothing is drawn and the sky is off.
    pub fn begin_frame(
        &mut self,
        target: &mut Framebuffer,
        camera: CameraView,
        clear_color: Color,
    ) {
        self.camera = camera;
        self.stats = DrawStats::default();
        if target.gbuffer().is_none() {
            target.enable_gbuffer(true);
        }
        target.clear(clear_color);
    }

    /// Geometry pass for one mesh.
    pub fn draw(
        &mut self,
        target: &mut Framebuffer,
        mesh: &Mesh,
        model: Mat4,
        material: &Material<'_>,
    ) -> DrawStats {
        let shader = GeometryShader::new(model, self.camera.view_projection(), *material);
        let stats = self.rasterizer.draw_mesh(target, mesh, &shader);
        self.stats.triangles_in += stats.triangles_in;
        self.stats.triangles_rasterized += stats.triangles_rasterized;
        self.stats.fragments_shaded += stats.fragments_shaded;
        self.stats.fragments_written += stats.fragments_written;
        stats
    }

    /// Lighting pass: shade every pixel the geometry pass wrote, then fill the
    /// rest with the sky.
    pub fn shade(&mut self, target: &mut Framebuffer) {
        let (width, height) = (target.width(), target.height());
        let inverse_view_projection = self.camera.inverse_view_projection();

        // The G-buffer is read while the color buffer is written; taking it out
        // of the framebuffer keeps the borrow checker out of the way, and it
        // goes back when the pass is done.
        let Some(gbuffer) = target.take_gbuffer() else {
            return;
        };

        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let surface = gbuffer.at(index);
                let color = if surface.is_geometry() {
                    self.shade_surface(surface, 1.0)
                } else if self.settings.draw_sky {
                    let direction =
                        self.camera
                            .ray_direction(&inverse_view_projection, x, y, width, height);
                    self.sky.radiance(direction)
                } else {
                    continue;
                };
                target.set_pixel(x, y, color);
            }
        }

        target.put_gbuffer(gbuffer);
    }

    /// Light one surface: every light, then the environment.
    ///
    /// `occlusion` is the screen-space ambient occlusion factor; it dims
    /// ambient light only — direct light has its own shadows.
    pub fn shade_surface(&self, surface: &Surface, occlusion: f32) -> Color {
        let normal = surface.normal;
        let to_eye = (self.camera.position - surface.position).normalized();

        let mut total = Color::BLACK;
        for light in &self.lights {
            let Some((to_light, radiance)) = light.sample(surface.position) else {
                continue;
            };
            let contribution = direct_light(surface, normal, to_eye, to_light, radiance);
            total = Color::rgb(
                total.r + contribution.r,
                total.g + contribution.g,
                total.b + contribution.b,
            );
        }

        let ambient = self.ambient_light(surface, to_eye, occlusion * surface.occlusion);
        Color::rgba(
            total.r + ambient.r + surface.emissive.r,
            total.g + ambient.g + surface.emissive.g,
            total.b + ambient.b + surface.emissive.b,
            1.0,
        )
    }

    /// Image-based lighting: the diffuse half from the irradiance harmonics,
    /// the specular half from the prefiltered environment, weighted by the
    /// split-sum approximation of the BRDF.
    fn ambient_light(&self, surface: &Surface, to_eye: Vec3, occlusion: f32) -> Color {
        let intensity = self.settings.ambient_intensity;
        if intensity <= 0.0 {
            return Color::BLACK;
        }
        let normal = surface.normal;
        let n_dot_v = normal.dot(to_eye).max(1e-4);
        let roughness = surface.roughness.clamp(0.0, 1.0);

        let f0 = f0_for(surface.albedo, surface.metallic);
        let fresnel = fresnel_schlick_roughness(n_dot_v, f0, roughness);

        // Whatever is not reflected can scatter — and metals scatter nothing.
        let diffuse_weight = 1.0 - surface.metallic;
        let irradiance = self.sky.irradiance(normal);
        let diffuse = Color::rgb(
            irradiance.r * surface.albedo.r * (1.0 - fresnel.r) * diffuse_weight,
            irradiance.g * surface.albedo.g * (1.0 - fresnel.g) * diffuse_weight,
            irradiance.b * surface.albedo.b * (1.0 - fresnel.b) * diffuse_weight,
        );

        let reflection = reflect(-to_eye, normal);
        let prefiltered = self.sky.sample(reflection, roughness);
        let (scale, bias) = environment_brdf(n_dot_v, roughness);
        let specular = Color::rgb(
            prefiltered.r * (fresnel.r * scale + bias),
            prefiltered.g * (fresnel.g * scale + bias),
            prefiltered.b * (fresnel.b * scale + bias),
        );

        Color::rgb(
            (diffuse.r + specular.r) * occlusion * intensity,
            (diffuse.g + specular.g) * occlusion * intensity,
            (diffuse.b + specular.b) * occlusion * intensity,
        )
    }
}

/// Mirror `incident` about `normal`. `incident` points *at* the surface.
#[inline]
pub fn reflect(incident: Vec3, normal: Vec3) -> Vec3 {
    incident - normal * (2.0 * incident.dot(normal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::SkyParams;
    use runity_math::Vec3;

    fn camera() -> CameraView {
        let position = Vec3::new(0.0, 0.0, 3.0);
        CameraView::new(
            Mat4::look_at(position, Vec3::ZERO, Vec3::Y),
            Mat4::perspective(60f32.to_radians(), 1.0, 0.1, 100.0),
            position,
            0.1,
            100.0,
        )
    }

    fn renderer() -> Renderer {
        Renderer::new(Sky::new(SkyParams::default()))
    }

    #[test]
    fn reflect_mirrors_about_the_normal() {
        let r = reflect(Vec3::new(1.0, -1.0, 0.0).normalized(), Vec3::Y);
        assert!(
            (r - Vec3::new(1.0, 1.0, 0.0).normalized()).length() < 1e-6,
            "{r:?}"
        );
    }

    #[test]
    fn a_frame_lights_its_geometry_and_fills_the_rest_with_sky() {
        let mut renderer = renderer();
        let mut target = Framebuffer::new(48, 48);
        renderer.begin_frame(&mut target, camera(), Color::BLACK);
        let stats = renderer.draw(
            &mut target,
            &Mesh::sphere(1.0, 24, 16),
            Mat4::IDENTITY,
            &Material::dielectric(Color::rgb(0.8, 0.2, 0.2), 0.4),
        );
        assert!(stats.fragments_written > 100);
        renderer.shade(&mut target);

        // The sphere is at the center; the corner is sky.
        let center = target.get_pixel(24, 24);
        assert!(
            center.r > center.g && center.r > center.b,
            "a red sphere: {center:?}"
        );
        assert!(center.luminance() > 0.01, "and it is lit: {center:?}");

        let corner = target.get_pixel(1, 1);
        assert!(corner.b > corner.r, "the sky is blue: {corner:?}");
        assert!(
            target.depth_at(1, 1).is_infinite(),
            "the sky writes no depth"
        );
    }

    #[test]
    fn a_metal_reflects_the_environment_rather_than_its_own_color() {
        let mut renderer = renderer();
        renderer.lights.clear(); // ambient only, so this is purely IBL

        let dull = Surface {
            albedo: Color::rgb(0.9, 0.9, 0.9),
            normal: Vec3::Y,
            position: Vec3::ZERO,
            roughness: 0.1,
            metallic: 1.0,
            ..Surface::default()
        };
        let up = renderer.shade_surface(&dull, 1.0);
        // Facing up at a blue sky, a mirror is blue.
        assert!(up.b > up.r, "{up:?}");

        // Under a grey sky the same mirror is grey: the color is the
        // environment's, not the material's.
        let grey = Color::rgb(0.5, 0.5, 0.5);
        let mut overcast = Renderer::new(Sky::new(SkyParams {
            sun_intensity: 0.0,
            sun_irradiance: 0.0,
            zenith_color: grey,
            horizon_color: grey,
            ground_color: grey,
            ..SkyParams::default()
        }));
        overcast.lights.clear();
        let neutral = overcast.shade_surface(&dull, 1.0);
        assert!(
            (neutral.b - neutral.r).abs() < 0.02,
            "a grey environment gives a grey reflection: {neutral:?}"
        );

        // A black dielectric, by contrast, has almost nothing to reflect.
        let dark = Surface {
            albedo: Color::BLACK,
            metallic: 0.0,
            roughness: 0.8,
            ..dull
        };
        assert!(renderer.shade_surface(&dark, 1.0).luminance() < up.luminance() * 0.2);
    }

    #[test]
    fn occlusion_dims_ambient_light_but_not_direct_light() {
        let mut renderer = renderer();
        let surface = Surface {
            albedo: Color::rgb(0.8, 0.8, 0.8),
            normal: Vec3::Y,
            position: Vec3::ZERO,
            roughness: 0.6,
            ..Surface::default()
        };

        let open = renderer.shade_surface(&surface, 1.0);
        let occluded = renderer.shade_surface(&surface, 0.0);
        assert!(
            occluded.luminance() < open.luminance(),
            "ambient must be dimmed"
        );
        assert!(occluded.luminance() > 0.0, "but the sun still reaches it");

        // With no lights at all, full occlusion leaves nothing.
        renderer.lights.clear();
        let dark = renderer.shade_surface(&surface, 0.0);
        assert!(dark.luminance() < 1e-6, "{dark:?}");
    }

    #[test]
    fn emissive_surfaces_glow_without_any_light() {
        let mut renderer = renderer();
        renderer.lights.clear();
        renderer.settings.ambient_intensity = 0.0;
        let surface = Surface {
            albedo: Color::BLACK,
            normal: Vec3::Y,
            emissive: Color::rgb(3.0, 1.0, 0.5),
            ..Surface::default()
        };
        let color = renderer.shade_surface(&surface, 1.0);
        assert_eq!(
            color.r, 3.0,
            "emission passes through untouched, above white"
        );
    }

    #[test]
    fn shading_the_same_frame_twice_gives_the_same_pixels() {
        let render_once = || {
            let mut renderer = renderer();
            let mut target = Framebuffer::new(32, 32);
            renderer.begin_frame(&mut target, camera(), Color::BLACK);
            renderer.draw(
                &mut target,
                &Mesh::cube(1.2),
                Mat4::from_rotation_y(0.7),
                &Material::metal(Color::rgb(0.9, 0.8, 0.5), 0.25),
            );
            renderer.shade(&mut target);
            target.colors().to_vec()
        };
        assert_eq!(render_once(), render_once());
    }
}
