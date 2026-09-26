//! A physical sky: sunlight scattered by the air, as HDRP's Physically
//! Based Sky and Unreal's Sky Atmosphere do it (after Hillaire, 2020).
//!
//! `sky: (mode: Physical)` in a scene. The sky is no longer a gradient
//! someone picked: air molecules scatter blue more than red (Rayleigh),
//! haze scatters everything, mostly forward, round the sun (Mie), ozone
//! takes a little orange — over a round planet, 6360 km across its
//! radius, under 100 km of air thinning with height. So noon is blue
//! overhead and pale at the horizon, and a low sun is orange because its
//! light has come through a long way of air, and the sky round it glows.
//!
//! What it drives:
//!
//! * **The sky**, and what reflects it: a compute pass fills a small
//!   picture of the whole sky each frame (a sky-view table, 192 × 108,
//!   denser near the horizon where the colour changes fastest), which the
//!   sky pass and the reflections read.
//! * **The sun's colour** and **the light from all round** — the frame's
//!   `sun_color`, `sky_color` and `ground_color` — worked out here on the
//!   CPU by the same model, so a sunset lights the scene orange and dims it,
//!   with no hand-picked colour to disagree with the sky.
//! * **Aerial perspective**: what is far is seen through air, bluer and
//!   paler with distance. A second pass fills a 32 × 32 × 32 grid over the
//!   view with what the air adds and lets through; the lit shader reads its
//!   pixel's cell. Real air needs kilometres to show; a game's valley is a
//!   few hundred metres, so `aerial_scale` stretches every distance (16 by
//!   default) — HDRP has the same knob.
//!
//! Units: metres, and a sun whose light, straight overhead at the top of
//! the air, is 1; `brightness` scales the sky against that.

use glam::Vec3;
use serde::{Deserialize, Serialize};

pub const PLANET_RADIUS: f32 = 6_360_000.0;
pub const ATMOSPHERE_TOP: f32 = 6_460_000.0;
const RAYLEIGH_SCATTERING: Vec3 = Vec3::new(5.802e-6, 13.558e-6, 33.1e-6);
const RAYLEIGH_HEIGHT: f32 = 8_000.0;
const MIE_SCATTERING: f32 = 3.996e-6;
const MIE_ABSORPTION: f32 = 4.4e-6;
const MIE_HEIGHT: f32 = 1_200.0;
const OZONE_ABSORPTION: Vec3 = Vec3::new(0.65e-6, 1.881e-6, 0.085e-6);

/// The air, as a scene tunes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Atmosphere {
    /// How much air: more is a deeper blue and redder sunsets.
    pub rayleigh: f32,
    /// How much haze: more is a paler sky and a bigger glow round the sun.
    pub mie: f32,
    pub ozone: f32,
    /// How forward the haze throws light, 0 to 0.99.
    pub mie_anisotropy: f32,
    /// How much light the ground sends back up, 0 to 1.
    pub ground_albedo: f32,
    /// The sky's light against the sun's.
    pub brightness: f32,
    /// Every distance in the view counted this many times over for the
    /// air in front of it: a valley of hundreds of metres gets the blue
    /// distance of real kilometres. 0 turns aerial perspective off.
    pub aerial_scale: f32,
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self {
            rayleigh: 1.0,
            mie: 1.0,
            ozone: 1.0,
            mie_anisotropy: 0.8,
            ground_albedo: 0.3,
            brightness: 12.0,
            aerial_scale: 16.0,
        }
    }
}

impl Atmosphere {
    fn densities(&self, height: f32) -> (f32, f32, f32) {
        let h = height.max(0.0);
        let rayleigh = (-h / RAYLEIGH_HEIGHT).exp() * self.rayleigh;
        let mie = (-h / MIE_HEIGHT).exp() * self.mie;
        let ozone = (1.0 - (h - 25_000.0).abs() / 15_000.0).max(0.0) * self.ozone;
        (rayleigh, mie, ozone)
    }

    fn extinction(&self, height: f32) -> Vec3 {
        let (r, m, o) = self.densities(height);
        RAYLEIGH_SCATTERING * r
            + Vec3::splat((MIE_SCATTERING + MIE_ABSORPTION) * m)
            + OZONE_ABSORPTION * o
    }

    /// How much of the sun's light gets through the air from the top down
    /// to `altitude`, coming from `to_sun` (up is +y).
    pub fn transmittance(&self, altitude: f32, to_sun: Vec3) -> Vec3 {
        let origin = Vec3::new(0.0, PLANET_RADIUS + altitude.max(0.0), 0.0);
        let Some(length) = ray_to_top(origin, to_sun) else {
            return Vec3::ZERO;
        };
        if hits_ground(origin, to_sun) {
            return Vec3::ZERO;
        }
        let steps = 40;
        let step = length / steps as f32;
        let mut depth = Vec3::ZERO;
        for i in 0..steps {
            let p = origin + to_sun * (step * (i as f32 + 0.5));
            depth += self.extinction(p.length() - PLANET_RADIUS) * step;
        }
        (-depth).exp()
    }

    /// The sky's light coming from `direction` for a sun towards `to_sun`,
    /// seen from `altitude`: sunlight scattered once on the way, with a
    /// share for the light scattered more than once.
    pub fn sky(&self, altitude: f32, direction: Vec3, to_sun: Vec3) -> Vec3 {
        let origin = Vec3::new(0.0, PLANET_RADIUS + altitude.max(1.0), 0.0);
        let Some(mut length) = ray_to_top(origin, direction) else {
            return Vec3::ZERO;
        };
        if let Some(ground) = ray_to_ground(origin, direction) {
            length = length.min(ground);
        }
        let steps = 24;
        let step = length / steps as f32;
        let cos = direction.dot(to_sun);
        let rayleigh_phase = 3.0 / (16.0 * std::f32::consts::PI) * (1.0 + cos * cos);
        let mie_phase = cornette_shanks(cos, self.mie_anisotropy);
        let mut through = Vec3::ONE;
        let mut light = Vec3::ZERO;
        for i in 0..steps {
            let p = origin + direction * (step * (i as f32 + 0.5));
            let height = p.length() - PLANET_RADIUS;
            let (r, m, _) = self.densities(height);
            let sun = self.transmittance(height, to_sun);
            let scattering = RAYLEIGH_SCATTERING * r * rayleigh_phase
                + Vec3::splat(MIE_SCATTERING * m * mie_phase);
            // Light scattered more than once, spread evenly: without it the
            // sky by the horizon and after sunset is far too dark.
            let many = (RAYLEIGH_SCATTERING * r + Vec3::splat(MIE_SCATTERING * m)) * 0.25
                / (4.0 * std::f32::consts::PI);
            let extinction = self.extinction(height);
            let segment = (-extinction * step).exp();
            let added = (scattering * sun + many * sun.length() * 0.577) * self.brightness;
            light += through * added * (Vec3::ONE - segment) / extinction.max(Vec3::splat(1e-12));
            through *= segment;
        }
        light
    }

    /// The frame's light from a sun towards `to_sun` of strength
    /// `intensity`, at `altitude`: the sun's colour after the air, and the
    /// light from all round on a surface facing up and one facing down.
    pub fn lighting(&self, altitude: f32, to_sun: Vec3, intensity: f32) -> (Vec3, Vec3, Vec3) {
        let (sun, sky) = self.light_of_one(altitude, to_sun);
        let (sun, sky) = (sun * intensity, sky * intensity);
        // What the ground sends back up: the sun and sky on it, times its
        // colour.
        let ground =
            (sun * to_sun.y.max(0.0) + sky) * self.ground_albedo / std::f32::consts::PI * 2.0;
        (sun, sky, ground)
    }
}

impl Atmosphere {
    /// The sun's light through the air and the sky's average over the
    /// upper half, for a sun of strength one: what [`Atmosphere::lighting`]
    /// scales. Some thirty thousand steps through the air — the sky from a
    /// handful of ways, each way's steps lit through their own column — so
    /// the ways are found across the cores and summed in order after, the
    /// same bits as one after another.
    pub fn light_of_one(&self, altitude: f32, to_sun: Vec3) -> (Vec3, Vec3) {
        let sun = self.transmittance(altitude, to_sun);
        let ways: Vec<(f32, u32)> = [8.0f32, 25.0, 50.0, 80.0]
            .into_iter()
            .flat_map(|elevation| (0..8).map(move |azimuth| (elevation, azimuth)))
            .collect();
        let seen = scrap_core::jobs::map(&ways, 4, |&(elevation, azimuth)| {
            let (e, a) = (
                elevation.to_radians(),
                azimuth as f32 * std::f32::consts::FRAC_PI_4,
            );
            let d = Vec3::new(e.cos() * a.cos(), e.sin(), e.cos() * a.sin());
            let w = e.sin();
            (self.sky(altitude, d, to_sun) * w, w)
        });
        let mut sky = Vec3::ZERO;
        let mut weight = 0.0;
        for (light, w) in seen {
            sky += light;
            weight += w;
        }
        (sun, sky / weight)
    }
}

/// Mie's phase, Cornette–Shanks.
fn cornette_shanks(cos: f32, g: f32) -> f32 {
    let g2 = g * g;
    3.0 / (8.0 * std::f32::consts::PI) * ((1.0 - g2) * (1.0 + cos * cos))
        / ((2.0 + g2) * (1.0 + g2 - 2.0 * g * cos).max(1e-4).powf(1.5))
}

/// How far along `d` from `o` (inside the air) the top of the air is.
fn ray_to_top(o: Vec3, d: Vec3) -> Option<f32> {
    let b = o.dot(d);
    let c = o.length_squared() - ATMOSPHERE_TOP * ATMOSPHERE_TOP;
    let disc = b * b - c;
    (disc >= 0.0).then(|| -b + disc.sqrt())
}

/// How far along `d` from `o` the ground is, if the ray meets it.
fn ray_to_ground(o: Vec3, d: Vec3) -> Option<f32> {
    let b = o.dot(d);
    let c = o.length_squared() - PLANET_RADIUS * PLANET_RADIUS;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    (t > 0.0).then_some(t)
}

fn hits_ground(o: Vec3, d: Vec3) -> bool {
    ray_to_ground(o, d).is_some()
}

/// What the sky's compute passes read.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct AtmosphereUniform {
    pub inverse_view_projection: [[f32; 4]; 4],
    /// Towards the sun; w the camera's altitude.
    pub to_sun: [f32; 4],
    /// The camera's position; w the aerial grid's far end, metres.
    pub eye: [f32; 4],
    /// Rayleigh, Mie and ozone amounts, the haze's anisotropy.
    pub amounts: [f32; 4],
    /// Brightness, the sun's intensity, the aerial scale, ground albedo.
    pub scale: [f32; 4],
}

pub(crate) const SKY_VIEW: (u32, u32) = (192, 108);
pub(crate) const AERIAL: u32 = 32;

pub const SHADER: &str = include_str!("atmosphere.wgsl");

/// The sky's table and the aerial grid on the GPU, and what fills them.
pub(crate) struct AtmosphereRenderer {
    uniforms: wgpu::Buffer,
    group: wgpu::BindGroup,
    sky_view_pass: wgpu::ComputePipeline,
    aerial_pass: wgpu::ComputePipeline,
    /// What the sky pass and the reflections read.
    pub(crate) sky_view: wgpu::TextureView,
    /// What the lit shader reads: added light, and what gets through.
    pub(crate) aerial: wgpu::TextureView,
}

impl AtmosphereRenderer {
    pub(crate) fn new(gpu: &crate::gpu::Gpu) -> Self {
        let format = wgpu::TextureFormat::Rgba16Float;
        let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        let sky_view = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky view"),
            size: wgpu::Extent3d {
                width: SKY_VIEW.0,
                height: SKY_VIEW.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let aerial = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("aerial perspective"),
            size: wgpu::Extent3d {
                width: AERIAL,
                height: AERIAL,
                depth_or_array_layers: AERIAL,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format,
            usage,
            view_formats: &[],
        });
        let sky_view = sky_view.create_view(&wgpu::TextureViewDescriptor::default());
        let aerial = aerial.create_view(&wgpu::TextureViewDescriptor::default());
        let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atmosphere"),
            size: std::mem::size_of::<AtmosphereUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let storage = |binding, dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension: dimension,
            },
            count: None,
        };
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("atmosphere"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    storage(1, wgpu::TextureViewDimension::D2),
                    storage(2, wgpu::TextureViewDimension::D3),
                ],
            });
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atmosphere"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&sky_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&aerial),
                },
            ],
        });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("scrap::atmosphere"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("scrap::atmosphere"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = |entry: &str| {
            gpu.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        Self {
            sky_view_pass: pipeline("cs_sky_view"),
            aerial_pass: pipeline("cs_aerial"),
            uniforms,
            group,
            sky_view,
            aerial,
        }
    }

    /// Fill the sky's table and the aerial grid for this frame.
    pub(crate) fn run(
        &self,
        gpu: &crate::gpu::Gpu,
        encoder: &mut wgpu::CommandEncoder,
        uniform: &AtmosphereUniform,
    ) {
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniform));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("scrap::atmosphere"),
            timestamp_writes: crate::gpu_timer::compute("atmosphere"),
        });
        pass.set_bind_group(0, &self.group, &[]);
        pass.set_pipeline(&self.sky_view_pass);
        pass.dispatch_workgroups(SKY_VIEW.0.div_ceil(8), SKY_VIEW.1.div_ceil(8), 1);
        pass.set_pipeline(&self.aerial_pass);
        pass.dispatch_workgroups(AERIAL.div_ceil(8), AERIAL.div_ceil(8), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noon_is_blue_overhead_and_a_low_sun_is_orange() {
        let air = Atmosphere::default();
        let overhead = Vec3::Y;
        let zenith = air.sky(2.0, Vec3::Y, overhead);
        assert!(zenith.z > zenith.x * 1.5, "a blue sky: {zenith}");
        let noon = air.transmittance(2.0, overhead);
        let low = air.transmittance(2.0, Vec3::new(1.0, 0.05, 0.0).normalize());
        assert!(
            noon.x > 0.8 && noon.z > 0.6,
            "the noon sun comes through: {noon}"
        );
        assert!(low.x > low.z * 3.0, "a low sun is red: {low}");
        assert!(
            low.length() < noon.length() * 0.6,
            "and dimmer: {low} vs {noon}"
        );
    }

    #[test]
    fn the_scene_is_dimmer_and_warmer_at_sunset() {
        let air = Atmosphere::default();
        let (noon_sun, noon_sky, _) = air.lighting(2.0, Vec3::Y, 1.0);
        let (dusk_sun, dusk_sky, _) = air.lighting(2.0, Vec3::new(1.0, 0.08, 0.0).normalize(), 1.0);
        assert!(dusk_sun.length() < noon_sun.length());
        assert!(dusk_sky.length() < noon_sky.length());
        assert!(dusk_sun.x / dusk_sun.z > noon_sun.x / noon_sun.z);
    }
}
