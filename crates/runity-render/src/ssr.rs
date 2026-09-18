//! Screen-space reflections.
//!
//! The environment map knows what the sky looks like in every direction, but
//! nothing about the objects in the scene — so a polished floor reflects the
//! sky and not the box standing on it. SSR fills that in by marching the
//! reflected ray through the depth buffer: if it passes behind something the
//! camera can see, that something is what the surface reflects.
//!
//! The method's limits are worth stating plainly, because they are visible:
//! it can only reflect what is *on screen*. Geometry outside the frame, or
//! hidden behind what is in front of it, has no depth to hit, so the ray misses
//! and the reflection falls back to the environment. That fallback is the whole
//! reason this pass replaces the environment term rather than adding to it.

use crate::color::Color;
use crate::framebuffer::Framebuffer;
use crate::gbuffer::GBuffer;
use crate::pbr::{environment_brdf, f0_for, fresnel_schlick_roughness};
use crate::pipeline::reflect;
use crate::sky::Sky;
use crate::view::CameraView;
use runity_math::Vec3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SsrSettings {
    pub enabled: bool,
    /// How far the ray is allowed to travel, in world units.
    pub max_distance: f32,
    /// Coarse steps along the ray. More steps, fewer missed thin objects.
    pub steps: usize,
    /// Binary-search iterations used to pin down the crossing point.
    pub refine_steps: usize,
    /// How thick a depth sample is assumed to be. Too small and rays tunnel
    /// through surfaces; too large and they hit things they passed long ago.
    pub thickness: f32,
    /// Surfaces rougher than this keep the environment reflection — a blurred
    /// screen-space trace is not worth its cost.
    pub max_roughness: f32,
    /// Fraction of the screen over which reflections fade out at the border,
    /// hiding the hard edge where the trace runs out of picture.
    pub edge_fade: f32,
    pub intensity: f32,
}

impl Default for SsrSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_distance: 24.0,
            steps: 48,
            refine_steps: 6,
            thickness: 0.35,
            max_roughness: 0.55,
            edge_fade: 0.12,
            intensity: 1.0,
        }
    }
}

/// Deterministic per-pixel offset in `[0, 1)`, so neighbouring rays do not all
/// step at the same distances and produce banding.
#[inline]
fn dither(x: usize, y: usize) -> f32 {
    // A 4x4 ordered Bayer matrix: fixed, cheap, and repeatable.
    const BAYER: [[f32; 4]; 4] = [
        [0.0, 8.0, 2.0, 10.0],
        [12.0, 4.0, 14.0, 6.0],
        [3.0, 11.0, 1.0, 9.0],
        [15.0, 7.0, 13.0, 5.0],
    ];
    BAYER[y % 4][x % 4] / 16.0
}

/// What the trace found.
struct Hit {
    /// Screen position of the hit.
    pixel: (usize, usize),
    /// How much to trust it: fades at the screen border and as the ray runs out.
    confidence: f32,
}

/// March one reflected ray through the depth buffer.
fn trace(
    origin: Vec3,
    direction: Vec3,
    start_offset: f32,
    gbuffer: &GBuffer,
    camera: &CameraView,
    settings: &SsrSettings,
) -> Option<Hit> {
    let (width, height) = (gbuffer.width(), gbuffer.height());
    let view_projection = camera.view_projection();
    let step_length = settings.max_distance / settings.steps as f32;

    let mut previous_t = 0.0;
    let mut hit_t = None;
    for step in 0..settings.steps {
        let t = (step as f32 + start_offset) * step_length;
        let position = origin + direction * t;
        let Some((sx, sy, _)) = camera.project(position, width, height, &view_projection) else {
            return None; // behind the camera: nothing to reflect
        };
        if sx < 0.0 || sy < 0.0 || sx >= width as f32 || sy >= height as f32 {
            return None; // walked off screen
        }
        let surface = gbuffer.get(sx as usize, sy as usize);
        if !surface.is_geometry() {
            previous_t = t;
            continue;
        }
        let ray_depth = camera.view_depth(position);
        let scene_depth = camera.view_depth(surface.position);
        let difference = ray_depth - scene_depth;
        if difference > 0.0 && difference < settings.thickness {
            hit_t = Some((previous_t, t));
            break;
        }
        previous_t = t;
    }

    let (mut near, mut far) = hit_t?;
    // Binary search between the last miss and the first hit, which is what
    // turns a stair-stepped intersection into a stable one.
    for _ in 0..settings.refine_steps {
        let middle = (near + far) * 0.5;
        let position = origin + direction * middle;
        let Some((sx, sy, _)) = camera.project(position, width, height, &view_projection) else {
            break;
        };
        if sx < 0.0 || sy < 0.0 || sx >= width as f32 || sy >= height as f32 {
            break;
        }
        let surface = gbuffer.get(sx as usize, sy as usize);
        if surface.is_geometry()
            && camera.view_depth(position) > camera.view_depth(surface.position)
        {
            far = middle;
        } else {
            near = middle;
        }
    }

    let position = origin + direction * far;
    let (sx, sy, _) = camera.project(position, width, height, &view_projection)?;
    if sx < 0.0 || sy < 0.0 || sx >= width as f32 || sy >= height as f32 {
        return None;
    }

    // Fade at the screen border, and with how far the ray had to travel.
    let u = sx / width as f32;
    let v = sy / height as f32;
    let fade = settings.edge_fade.max(1e-4);
    let border = (u / fade)
        .min((1.0 - u) / fade)
        .min(v / fade)
        .min((1.0 - v) / fade);
    let distance_fade = 1.0 - (far / settings.max_distance).clamp(0.0, 1.0).powi(2);
    let confidence = border.clamp(0.0, 1.0) * distance_fade;
    if confidence <= 0.0 {
        return None;
    }

    Some(Hit {
        pixel: (sx as usize, sy as usize),
        confidence,
    })
}

/// Replace the environment's specular contribution with what the scene itself
/// reflects, wherever a ray finds something.
pub fn apply(
    target: &mut Framebuffer,
    gbuffer: &GBuffer,
    camera: &CameraView,
    sky: &Sky,
    settings: &SsrSettings,
) {
    if !settings.enabled || settings.steps == 0 {
        return;
    }
    let (width, height) = (target.width(), target.height());

    // Reflections read the lit frame. Snapshot it so a pixel already touched by
    // this pass cannot feed itself back in.
    let lit: Vec<Color> = target.colors().to_vec();

    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let surface = gbuffer.at(index);
            if !surface.is_geometry() || surface.roughness > settings.max_roughness {
                continue;
            }

            let to_eye = (camera.position - surface.position).normalized();
            let normal = surface.normal;
            let n_dot_v = normal.dot(to_eye);
            if n_dot_v <= 0.0 {
                continue;
            }
            let direction = reflect(-to_eye, normal);

            // Start a little off the surface so the ray does not hit its own pixel.
            let origin = surface.position + normal * 0.02;
            let Some(hit) = trace(origin, direction, dither(x, y), gbuffer, camera, settings)
            else {
                continue;
            };

            let reflected = lit[hit.pixel.1 * width + hit.pixel.0];
            let environment = sky.sample(direction, surface.roughness);

            // How much of this surface's look is environment specular: exactly
            // what the lighting pass added, so subtracting it is not a guess.
            let f0 = f0_for(surface.albedo, surface.metallic);
            let fresnel = fresnel_schlick_roughness(n_dot_v, f0, surface.roughness);
            let (scale, bias) = environment_brdf(n_dot_v, surface.roughness);
            let weight = hit.confidence
                * settings.intensity
                * (1.0 - (surface.roughness / settings.max_roughness).clamp(0.0, 1.0)).max(0.0);

            let current = target.get_pixel(x, y);
            let channel = |reflected: f32, environment: f32, fresnel: f32, current: f32| {
                let reflectance = fresnel * scale + bias;
                current + (reflected - environment) * reflectance * weight
            };
            target.set_pixel(
                x,
                y,
                Color::rgba(
                    channel(reflected.r, environment.r, fresnel.r, current.r).max(0.0),
                    channel(reflected.g, environment.g, fresnel.g, current.g).max(0.0),
                    channel(reflected.b, environment.b, fresnel.b, current.b).max(0.0),
                    current.a,
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gbuffer::Surface;
    use runity_math::Mat4;

    fn camera(width: usize, height: usize) -> CameraView {
        let position = Vec3::new(0.0, 1.5, 4.0);
        CameraView::new(
            Mat4::look_at(position, Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
            Mat4::perspective(60f32.to_radians(), width as f32 / height as f32, 0.1, 100.0),
            position,
            0.1,
            100.0,
        )
    }

    #[test]
    fn the_dither_pattern_is_fixed_and_varies_across_a_tile() {
        assert_eq!(dither(5, 5), dither(9, 9), "it repeats every four pixels");
        assert_ne!(dither(0, 0), dither(1, 0));
        for y in 0..4 {
            for x in 0..4 {
                assert!((0.0..1.0).contains(&dither(x, y)));
            }
        }
    }

    #[test]
    fn a_ray_with_nothing_in_front_of_it_misses() {
        let (width, height) = (32, 32);
        let camera = camera(width, height);
        let gbuffer = GBuffer::new(width, height); // empty: no geometry anywhere
        let hit = trace(
            Vec3::ZERO,
            Vec3::Y,
            0.0,
            &gbuffer,
            &camera,
            &SsrSettings::default(),
        );
        assert!(hit.is_none());
    }

    #[test]
    fn rough_surfaces_are_left_to_the_environment() {
        let (width, height) = (16, 16);
        let camera = camera(width, height);
        let sky = Sky::default();
        let mut gbuffer = GBuffer::new(width, height);
        for index in 0..width * height {
            gbuffer.set(
                index,
                Surface {
                    albedo: Color::WHITE,
                    normal: Vec3::Y,
                    position: Vec3::ZERO,
                    roughness: 0.9, // well above max_roughness
                    ..Surface::default()
                },
            );
        }

        let mut target = Framebuffer::new(width, height);
        target.clear(Color::rgb(0.25, 0.25, 0.25));
        let before = target.colors().to_vec();
        apply(
            &mut target,
            &gbuffer,
            &camera,
            &sky,
            &SsrSettings::default(),
        );
        assert_eq!(target.colors(), before, "nothing should have changed");
    }

    #[test]
    fn disabled_settings_change_nothing() {
        let (width, height) = (8, 8);
        let camera = camera(width, height);
        let mut target = Framebuffer::new(width, height);
        target.clear(Color::rgb(0.5, 0.1, 0.1));
        let before = target.colors().to_vec();
        let settings = SsrSettings {
            enabled: false,
            ..SsrSettings::default()
        };
        apply(
            &mut target,
            &GBuffer::new(width, height),
            &camera,
            &Sky::default(),
            &settings,
        );
        assert_eq!(target.colors(), before);
    }
}
