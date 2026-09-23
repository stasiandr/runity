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

use serde::{Deserialize, Serialize};

use crate::gpu::Gpu;

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
                entries: &[storage(0)],
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
        let integrated_view = view(&integrated);
        let inject_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog inject"),
            layout: &inject_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&scattering_view),
            }],
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
        }
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
            timestamp_writes: None,
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
