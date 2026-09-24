//! Volumetric fog: light seen in the air. A lantern's glow hanging in the
//! mist round it, the sun coming through the trees in shafts, the dust in
//! a barn lit where the lamp's cone crosses it — HDRP's and Frostbite's
//! froxel fog, which URP does not have.
//!
//! The view is cut into a grid of little frusta — [`WIDTH`] × [`HEIGHT`]
//! across it, [`DEPTH`] deep, thinner near the eye — and a compute pass
//! works out, for each, how much light the air there scatters towards the
//! camera: the sun through its cascades (so a pillar's shadow is a dark
//! shaft through the fog), each lamp in the cell's cluster through its own
//! shadow map, and the sky from all round. A second pass walks each column
//! front to back, adding what each cell scatters and dimming by what it
//! absorbs, so every cell holds what the air between it and the eye does
//! to light. The lit shader then reads the one cell its fragment is in:
//! what is behind is dimmed, and the air's own glow added.
//!
//! The air is denser low down: `density` at `base_height`, thinning by
//! `height_falloff` per metre above it — mist in a valley, not a wall.
//! `anisotropy` is how much the air throws light forward (Henyey–
//! Greenstein): looking towards the sun or a lamp, the fog glows. Off by
//! default: how misty a place is is the scene's call.

use bytemuck::Zeroable;
use serde::{Deserialize, Serialize};

use crate::gpu::Gpu;

/// Smoke or fire in a box of the world, from a simulation on a grid —
/// the fluid module's `smoke` — sampled by the fog's cells as the air's
/// own is: lit by the sun and sky where it is thin, dark where it is
/// thick, glowing where it is hot. Each cell of `cells` is its density
/// (0–255 for 0–3) and its heat (0–255 for none to white-hot); the grid is
/// `size` cells from `low` to `high`, x fastest.
#[derive(Debug, Clone, PartialEq)]
pub struct Smoke {
    pub low: glam::Vec3,
    pub high: glam::Vec3,
    pub size: [u32; 3],
    pub cells: std::sync::Arc<Vec<[u8; 4]>>,
    /// Linear colour it scatters.
    pub color: [f32; 3],
    /// Extinction per metre at a density of 1.
    pub density: f32,
    /// How brightly the hot part glows; 0 for smoke that is not fire.
    pub glow: f32,
    /// Simulated on the GPU ([`crate::smoke_gpu`]): the air is the
    /// renderer's, and `cells` is empty — the step writes the picture.
    pub gpu: Option<GpuSmoke>,
}

/// A smoke the renderer steps: its grid, its source and look, and how far
/// its clock has gone since the last frame. What is solid in it is the
/// world's, one flag a cell, `solid_version` changing when it does.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuSmoke {
    /// Which smoke, between frames.
    pub key: u64,
    pub n: [u32; 3],
    /// Cell size, metres.
    pub dx: f32,
    /// How wide the source is (m), smoke and heat given off a second.
    pub source: f32,
    pub rate: f32,
    pub heat: f32,
    pub weight: f32,
    pub curl: f32,
    pub fade: f32,
    /// The wind the air is drawn toward, metres a second.
    pub wind: glam::Vec3,
    /// How far its clock has gone, seconds, in whole fixed steps of
    /// [`SMOKE_STEP`]: the renderer steps its air up to it. A frame drawn
    /// twice owes nothing the second time.
    pub clock: f32,
    pub solid: std::sync::Arc<Vec<u32>>,
    pub solid_version: u64,
}

/// A smoke's fixed step, seconds (`runity_fluid::smoke::STEP`).
pub const SMOKE_STEP: f32 = 1.0 / 30.0;

/// Every so many cells of a grid of `n` along each way, to fit the fog's
/// picture ([`SMOKE_MOST`]).
pub fn smoke_stride(n: [u32; 3]) -> [u32; 3] {
    [
        n[0].div_ceil(SMOKE_MOST[0]).max(1),
        n[1].div_ceil(SMOKE_MOST[1]).max(1),
        n[2].div_ceil(SMOKE_MOST[2]).max(1),
    ]
}

/// The largest smoke grid the render takes along each way; a larger one
/// is thinned to fit by whoever makes it.
pub const SMOKE_MOST: [u32; 3] = [64, 96, 64];
/// The most smokes a frame draws: the nearest the eye.
pub const MOST_SMOKES: usize = 4;

/// The smokes' boxes as the shader reads them, and how many.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SmokeUniforms {
    boxes: [SmokeUniform; MOST_SMOKES],
    count: [u32; 4],
}

/// One smoke's box as the shader reads it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SmokeUniform {
    /// Its low corner, and 1 when there is smoke.
    low: [f32; 4],
    /// Its high corner, and its extinction per unit of density.
    high: [f32; 4],
    /// Its colour, and how brightly it glows.
    color: [f32; 4],
    /// How much of the texture it fills along each way.
    fill: [f32; 4],
}

/// Cells across the view.
pub const WIDTH: u32 = 160;
/// Cells down it.
pub const HEIGHT: u32 = 90;
/// Cells deep.
pub const DEPTH: u32 = 64;

/// A ball of dust in the air — kicked up by a foot
/// ([`crate::footprints`]) — added to the fog's grid: lit by the sun as
/// the fog is, and soft at its edge. A frame holds up to [`MOST_PUFFS`];
/// with any, the grid runs even when the scene has no fog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Puff {
    pub position: glam::Vec3,
    pub radius: f32,
    /// Extinction per metre at its middle.
    pub density: f32,
    /// Linear colour.
    pub color: [f32; 3],
}

/// The most puffs a frame carries; past it, the nearest the camera win.
pub const MOST_PUFFS: usize = 16;

/// A dust devil: a whirling column of sand standing on the ground at
/// `position`, `radius` wide at its foot and flaring above, `height` tall,
/// leaning downwind ([`crate::weather::Weather::devils`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Devil {
    pub position: glam::Vec3,
    pub radius: f32,
    pub height: f32,
    /// 0 to 1: how much sand it has lifted now.
    pub strength: f32,
    /// Which way it turns: 1 or −1.
    pub spin: f32,
}

/// The most dust devils a frame carries.
pub const MOST_DEVILS: usize = 6;

/// Sand blown off a dune's crest: a sheet streaming downwind from a
/// stretch of crest `2 * half_length` long, centred at `position` (on the
/// crest) and lying along `along` ([`crate::terrain`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plume {
    pub position: glam::Vec3,
    pub along: glam::Vec3,
    pub half_length: f32,
    /// 0 to 1: how much is blowing.
    pub strength: f32,
}

/// The most crest plumes a frame carries; past it, the nearest win.
pub const MOST_PLUMES: usize = 64;

/// The fog in the air, as a scene says it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VolumetricFog {
    pub enabled: bool,
    /// How much of the light the air takes per metre at `base_height`.
    pub density: f32,
    /// The air's colour: what share of each channel it scatters.
    pub color: [f32; 3],
    /// −1 to 1: how much the air throws light onward rather than back. The
    /// real thing is well forward (0.6–0.8), which is what makes shafts.
    pub anisotropy: f32,
    /// Where the fog is thickest, metres.
    pub base_height: f32,
    /// How fast it thins above that, per metre; 0 is even everywhere.
    pub height_falloff: f32,
    /// How far from the eye the cells reach, metres.
    pub distance: f32,
    /// How much of the sky's light the fog takes from all round.
    pub ambient: f32,
    /// How much of the lamps' light it takes.
    pub lamps: f32,
}

impl Default for VolumetricFog {
    fn default() -> Self {
        Self::OFF
    }
}

impl VolumetricFog {
    pub const OFF: VolumetricFog = VolumetricFog {
        enabled: false,
        density: 0.03,
        color: [1.0, 1.0, 1.0],
        anisotropy: 0.6,
        base_height: 0.0,
        height_falloff: 0.15,
        distance: 64.0,
        ambient: 1.0,
        lamps: 4.0,
    };
}

pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

fn volume(gpu: &Gpu, label: &str, size: (u32, u32, u32), storage: bool) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: size.2,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: FORMAT,
        usage: if storage {
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING
        } else {
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST
        },
        view_formats: &[],
    })
}

/// The grid on the GPU: what each cell scatters, and the sum along the way.
pub(crate) struct Volumes {
    /// What the lit shader reads: light added and let through, eye to cell.
    pub(crate) integrated: wgpu::TextureView,
    /// One cell of clear air, bound while the grid is being made.
    pub(crate) blank: wgpu::TextureView,
    pub(crate) sampler: wgpu::Sampler,
    pub(crate) inject_layout: wgpu::BindGroupLayout,
    pub(crate) integrate_layout: wgpu::BindGroupLayout,
    inject_group: wgpu::BindGroup,
    integrate_group: wgpu::BindGroup,
    smoke_texture: wgpu::Texture,
    /// The same, as the GPU smokes write it.
    pub(crate) smoke_storage: wgpu::TextureView,
    smoke_uniform: wgpu::Buffer,
}

impl Volumes {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        let size = (WIDTH, HEIGHT, DEPTH);
        let scattering = volume(gpu, "fog scattering", size, true);
        let integrated = volume(gpu, "fog integrated", size, true);
        let blank = volume(gpu, "no fog", (1, 1, 1), false);
        // Clear air: nothing added, everything let through.
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &blank,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            // Half floats: 0, 0, 0 and 1.0.
            bytemuck::cast_slice(&[0u16, 0, 0, 0x3C00]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: FORMAT,
                view_dimension: wgpu::TextureViewDimension::D3,
            },
            count: None,
        };
        let inject_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fog inject"),
                entries: &[
                    storage(0),
                    wgpu::BindGroupLayoutEntry {
                        binding: 10,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D3,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 11,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 12,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let integrate_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("fog integrate"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D3,
                                multisampled: false,
                            },
                            count: None,
                        },
                        storage(2),
                    ],
                });
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let scattering_view = view(&scattering);
        // The smoke's grid, as large as it may be, and its box: none yet.
        let smoke_texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("smoke"),
            size: wgpu::Extent3d {
                width: SMOKE_MOST[0],
                height: SMOKE_MOST[1],
                // The smokes one above the other along z.
                depth_or_array_layers: SMOKE_MOST[2] * MOST_SMOKES as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let smoke_storage = view(&smoke_texture);
        let smoke_uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smoke box"),
            size: std::mem::size_of::<SmokeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue.write_buffer(&smoke_uniform, 0, bytemuck::bytes_of(&SmokeUniforms::zeroed()));
        let smoke_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("smoke"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let smoke_view = view(&smoke_texture);
        let integrated_view = view(&integrated);
        let inject_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog inject"),
            layout: &inject_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&scattering_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&smoke_view),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::Sampler(&smoke_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: smoke_uniform.as_entire_binding(),
                },
            ],
        });
        let integrate_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog integrate"),
            layout: &integrate_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&scattering_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&integrated_view),
                },
            ],
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fog"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            integrated: integrated_view,
            blank: view(&blank),
            sampler,
            inject_layout,
            integrate_layout,
            inject_group,
            integrate_group,
            smoke_texture,
            smoke_storage,
            smoke_uniform,
        }
    }

    /// The frame's smokes into the grid the fog samples: the nearest
    /// [`MOST_SMOKES`], each in its own slab of the texture.
    /// Those simulated on the GPU come back with their slab and size, for
    /// [`crate::smoke_gpu::SmokeSim::run`] to write.
    pub(crate) fn set_smoke<'a>(&self, gpu: &Gpu, smokes: &'a [Smoke]) -> Vec<(u32, [u32; 3], &'a GpuSmoke)> {
        let mut uniforms = SmokeUniforms::zeroed();
        let mut count = 0;
        let mut simulated = Vec::new();
        for smoke in smokes.iter().filter(|s| s.size.iter().all(|n| *n > 0)).take(MOST_SMOKES) {
            let size = [
                smoke.size[0].min(SMOKE_MOST[0]),
                smoke.size[1].min(SMOKE_MOST[1]),
                smoke.size[2].min(SMOKE_MOST[2]),
            ];
            if let Some(sim) = &smoke.gpu {
                simulated.push((count as u32, size, sim));
            } else if smoke.cells.len() < (size[0] * size[1] * size[2]) as usize {
                continue;
            } else {
                gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.smoke_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: SMOKE_MOST[2] * count as u32 },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&smoke.cells[..(size[0] * size[1] * size[2]) as usize]),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size[0] * 4),
                    rows_per_image: Some(size[1]),
                },
                wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: size[2],
                },
            );
            }
            uniforms.boxes[count] = SmokeUniform {
                low: [smoke.low.x, smoke.low.y, smoke.low.z, 1.0],
                high: [smoke.high.x, smoke.high.y, smoke.high.z, smoke.density.max(0.0)],
                color: [smoke.color[0], smoke.color[1], smoke.color[2], smoke.glow.max(0.0)],
                fill: [
                    size[0] as f32 / SMOKE_MOST[0] as f32,
                    size[1] as f32 / SMOKE_MOST[1] as f32,
                    size[2] as f32 / SMOKE_MOST[2] as f32,
                    count as f32,
                ],
            };
            count += 1;
        }
        uniforms.count = [count as u32, 0, 0, 0];
        gpu.queue.write_buffer(&self.smoke_uniform, 0, bytemuck::bytes_of(&uniforms));
        simulated
    }

    /// Fill the grid: what each cell scatters, then the sums front to back.
    /// `frame_group` is the frame's bind group with the grid left out.
    pub(crate) fn run(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame_group: &wgpu::BindGroup,
        inject: &wgpu::ComputePipeline,
        integrate: &wgpu::ComputePipeline,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("runity::fog"),
            timestamp_writes: crate::gpu_timer::compute("fog"),
        });
        pass.set_pipeline(inject);
        pass.set_bind_group(0, frame_group, &[]);
        pass.set_bind_group(3, &self.inject_group, &[]);
        pass.dispatch_workgroups(WIDTH.div_ceil(4), HEIGHT.div_ceil(4), DEPTH.div_ceil(4));
        pass.set_pipeline(integrate);
        pass.set_bind_group(0, frame_group, &[]);
        pass.set_bind_group(3, &self.integrate_group, &[]);
        pass.dispatch_workgroups(WIDTH.div_ceil(8), HEIGHT.div_ceil(8), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fog_reads_from_a_scene_line() {
        let fog: VolumetricFog =
            ron::from_str("(enabled: true, density: 0.08, anisotropy: 0.8)").unwrap();
        assert!(fog.enabled);
        assert_eq!(fog.density, 0.08);
        assert_eq!(fog.distance, 64.0);
        assert!(!VolumetricFog::default().enabled);
    }
}
