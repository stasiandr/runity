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

use crate::bloom::{Bloom, BloomSettings};
use crate::color::Color;
use crate::framebuffer::Framebuffer;
use crate::gbuffer::Surface;
use crate::geometry::GeometryShader;
use crate::mesh::Mesh;
use crate::pbr::{
    direct_light, environment_brdf, f0_for, fresnel_schlick_roughness, Light, Material,
};
use crate::post::{self, PostSettings};
use crate::raster::{DrawStats, Rasterizer};
use crate::shadow::{ShadowMap, ShadowSettings};
use crate::sky::Sky;
use crate::ssao::{self, OcclusionBuffer, SsaoSettings};
use crate::ssr::{self, SsrSettings};
use crate::view::CameraView;
use runity_math::{Mat4, Vec3};

/// What the renderer does beyond plain shading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderSettings {
    /// Fill pixels with no geometry with the sky rather than the clear color.
    pub draw_sky: bool,
    /// Multiplier on image-based lighting.
    pub ambient_intensity: f32,
    pub shadows: ShadowSettings,
    pub ssao: SsaoSettings,
    pub ssr: SsrSettings,
    pub bloom: BloomSettings,
    pub post: PostSettings,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            draw_sky: true,
            ambient_intensity: 1.0,
            shadows: ShadowSettings::default(),
            ssao: SsaoSettings::default(),
            ssr: SsrSettings::default(),
            bloom: BloomSettings::default(),
            post: PostSettings::default(),
        }
    }
}

/// Deferred renderer: owns the lights, the environment and the passes.
pub struct Renderer {
    pub rasterizer: Rasterizer,
    pub sky: Sky,
    pub lights: Vec<Light>,
    pub settings: RenderSettings,
    /// One per light, index-aligned; inactive for lights that cannot cast.
    shadow_maps: Vec<ShadowMap>,
    occlusion: OcclusionBuffer,
    bloom: Bloom,
    frame: u64,
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
            shadow_maps: Vec::new(),
            occlusion: OcclusionBuffer::default(),
            bloom: Bloom::new(),
            frame: 0,
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
        self.frame = self.frame.wrapping_add(1);
        if target.gbuffer().is_none() {
            target.enable_gbuffer(true);
        }
        target.clear(clear_color);

        // Point every shadow-casting light's map at what the camera is looking
        // at. Meshes go into them during the geometry pass.
        let resolution = self.settings.shadows.resolution;
        while self.shadow_maps.len() < self.lights.len() {
            self.shadow_maps.push(ShadowMap::new(resolution));
        }
        self.shadow_maps.truncate(self.lights.len());
        for (map, light) in self.shadow_maps.iter_mut().zip(&self.lights) {
            map.begin(light, &self.camera, &self.settings.shadows);
        }
    }

    /// The depth maps rendered this frame, for debug views.
    pub fn shadow_maps(&self) -> &[ShadowMap] {
        &self.shadow_maps
    }

    /// Ambient visibility computed for this frame, for debug views.
    pub fn occlusion(&self) -> &OcclusionBuffer {
        &self.occlusion
    }

    /// Geometry pass for one mesh.
    pub fn draw(
        &mut self,
        target: &mut Framebuffer,
        mesh: &Mesh,
        model: Mat4,
        material: &Material<'_>,
    ) -> DrawStats {
        // The same geometry goes into every active shadow map. Depth-only, so
        // it costs a fraction of the main pass.
        for map in &mut self.shadow_maps {
            map.draw(mesh, model);
        }

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

        // Ambient occlusion has to run before lighting: it is an input to it,
        // not a filter applied afterwards.
        ssao::compute(
            &gbuffer,
            &self.camera,
            &self.settings.ssao,
            &mut self.occlusion,
        );

        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let surface = gbuffer.at(index);
                let color = if surface.is_geometry() {
                    self.shade_surface(surface, self.occlusion.at(index))
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

        // Screen-space reflections run on the lit frame: they reflect light,
        // not albedo, so they have to come after shading.
        ssr::apply(
            target,
            &gbuffer,
            &self.camera,
            &self.sky,
            &self.settings.ssr,
        );

        // Then the camera's own contribution: glare around the highlights, and
        // the lens and sensor treatment. Both still in linear light — tone
        // mapping is the last thing that happens, at resolve.
        self.bloom.apply(target, &self.settings.bloom);
        post::apply(target, &self.settings.post, self.frame);

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
        for (index, light) in self.lights.iter().enumerate() {
            let Some((to_light, radiance)) = light.sample(surface.position) else {
                continue;
            };
            let visibility = match self.shadow_maps.get(index) {
                Some(map) => map.visibility(surface.position, normal, &self.settings.shadows),
                None => 1.0,
            };
            if visibility <= 0.0 {
                continue;
            }
            let radiance = radiance.scale_rgb(visibility);
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
    fn a_caster_darkens_the_ground_under_it() {
        let mut renderer = renderer();
        renderer.settings.ambient_intensity = 0.0; // isolate the sun
        renderer.settings.shadows.extent = 10.0;
        renderer.settings.shadows.resolution = 512;
        // Straight down, so the shadow lands directly under the cube.
        renderer.lights = vec![Light::directional(-Vec3::Y, Color::WHITE, 3.0)];

        // Off to the side and above, with a clear line of sight to the ground
        // beneath the caster.
        let position = Vec3::new(6.0, 4.0, 6.0);
        let camera = CameraView::new(
            Mat4::look_at(position, Vec3::ZERO, Vec3::Y),
            Mat4::perspective(60f32.to_radians(), 1.0, 0.1, 100.0),
            position,
            0.1,
            100.0,
        );

        let (width, height) = (96, 96);
        let mut target = Framebuffer::new(width, height);
        renderer.begin_frame(&mut target, camera, Color::BLACK);
        let ground = Material::dielectric(Color::rgb(0.8, 0.8, 0.8), 0.9);
        renderer.draw(&mut target, &Mesh::plane(24.0, 1), Mat4::IDENTITY, &ground);
        renderer.draw(
            &mut target,
            &Mesh::cube(2.0),
            Mat4::from_translation(Vec3::new(0.0, 3.0, 0.0)),
            &ground,
        );
        renderer.shade(&mut target);

        // Ask the camera where each ground point landed rather than guessing.
        let view_projection = camera.view_projection();
        let pixel = |world: Vec3| {
            let (x, y, _) = camera
                .project(world, width, height, &view_projection)
                .expect("visible");
            (x as usize, y as usize)
        };
        let under_the_cube = pixel(Vec3::ZERO);
        // Off to the side, where the cube does not block the view of the floor.
        let open_ground = pixel(Vec3::new(3.0, 0.0, -3.0));
        let luminance =
            |target: &Framebuffer, (x, y): (usize, usize)| target.get_pixel(x, y).luminance();

        let shadowed = luminance(&target, under_the_cube);
        let lit = luminance(&target, open_ground);
        assert!(lit > 0.05, "the open ground is lit: {lit}");
        assert!(
            shadowed < lit * 0.2,
            "under the cube is dark: {shadowed} vs {lit}"
        );

        // And with shadows off, the same point is lit again.
        renderer.settings.shadows.enabled = false;
        renderer.begin_frame(&mut target, camera, Color::BLACK);
        renderer.draw(&mut target, &Mesh::plane(24.0, 1), Mat4::IDENTITY, &ground);
        renderer.draw(
            &mut target,
            &Mesh::cube(2.0),
            Mat4::from_translation(Vec3::new(0.0, 3.0, 0.0)),
            &ground,
        );
        renderer.shade(&mut target);
        assert!(
            luminance(&target, under_the_cube) > lit * 0.8,
            "shadows can be switched off"
        );
    }

    #[test]
    fn a_mirror_floor_reflects_what_is_standing_on_it() {
        let mut renderer = renderer();
        renderer.lights.clear();
        renderer.settings.ambient_intensity = 0.15;
        renderer.settings.ssao.enabled = false;

        let position = Vec3::new(0.0, 1.2, 4.0);
        let camera = CameraView::new(
            Mat4::look_at(position, Vec3::new(0.0, 0.4, 0.0), Vec3::Y),
            Mat4::perspective(55f32.to_radians(), 1.0, 0.1, 100.0),
            position,
            0.1,
            100.0,
        );

        // A glowing green box above a polished floor: whatever shows up in the
        // floor's reflection can only have come from the box.
        let render = |renderer: &mut Renderer| {
            let mut target = Framebuffer::new(96, 96);
            renderer.begin_frame(&mut target, camera, Color::BLACK);
            renderer.draw(
                &mut target,
                &Mesh::plane(20.0, 1),
                Mat4::IDENTITY,
                &Material {
                    roughness: 0.05,
                    metallic: 1.0,
                    base_color: Color::WHITE,
                    ..Material::default()
                },
            );
            renderer.draw(
                &mut target,
                &Mesh::cube(1.0),
                Mat4::from_translation(Vec3::new(0.0, 0.9, 0.0)),
                &Material {
                    emissive: Color::rgb(0.0, 6.0, 0.0),
                    ..Material::default()
                },
            );
            renderer.shade(&mut target);
            target
        };

        let with_ssr = render(&mut renderer);
        renderer.settings.ssr.enabled = false;
        let without_ssr = render(&mut renderer);

        // Where the box's mirror image lands: the line from the eye to the
        // box reflected through the floor plane crosses y = 0 here.
        let view_projection = camera.view_projection();
        let (x, y, _) = camera
            .project(Vec3::new(0.0, 0.0, 1.71), 96, 96, &view_projection)
            .expect("visible");
        let (x, y) = (x as usize, y as usize);

        let reflected = with_ssr.get_pixel(x, y);
        let plain = without_ssr.get_pixel(x, y);
        assert!(
            reflected.g > plain.g + 0.05,
            "the floor should pick up the green box: {reflected:?} vs {plain:?}"
        );
        assert!(
            reflected.g > reflected.r * 1.5,
            "and it should be green: {reflected:?}"
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
