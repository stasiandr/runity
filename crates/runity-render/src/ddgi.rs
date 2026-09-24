//! Global illumination by probes the rays keep lit: DDGI (Majercik et al.,
//! 2019), the way RTXGI does it.
//!
//! An entity's `irradiance_volume: (size: (40.0, 8.0, 40.0), spacing:
//! 2.0)` fills its box with a grid of probes, `spacing` metres apart.
//! Every frame each probe sends 64 rays round itself, turned a different
//! way each frame. What a ray meets is lit as a traced reflection's hit is
//! — its material's colour, the sun and the lamps with a shadow ray each —
//! and by the probes themselves round it, so light that has bounced once
//! bounces again next frame, and over a few frames every bounce is in.
//! What a ray does not meet is the sky.
//!
//! Each probe keeps two small octahedral pictures, 8×8 texels: the light
//! coming to it from every way, as a face turned that way would take it
//! (cosine-weighted), and how far it is to what is there, with its square
//! — its mean and variance. A frame's rays are blended into them, three
//! parts of a hundred at a time, so a lamp switched on fills a room over a
//! second and the rays' noise does not show.
//!
//! A surface in the box takes its diffuse light from the eight probes
//! round it, each weighted by how near it is, whether it is on the side
//! the surface faces, and — by the distance picture, a Chebyshev test —
//! whether it can see the surface at all: a probe on the other side of a
//! wall sees the wall nearer than the surface and counts for nothing, so
//! light does not leak through. A probe whose rays mostly meet the backs
//! of faces is inside something, and is left out. Near the box's edge
//! the probes give way to the sky and ground's hemisphere (and the
//! reflection probes, where there are).
//!
//! It needs rays (a device that traces, [`crate::ray`]); without them the
//! box is ignored and the scene is lit as before.

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::gpu::Gpu;

/// Rays a probe sends each frame.
pub const RAYS: u32 = 64;
/// Texels across a probe's pictures.
pub const TEXELS: u32 = 8;
/// A probe's `vec4`s in the buffer: its light, then its distances.
const PER_PROBE: u64 = (TEXELS * TEXELS * 2) as u64;
/// The most probes a volume has: past this the spacing is widened.
pub const MOST_PROBES: u32 = 16384;

/// A box filled with probes, at an entity.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IrradianceVolume {
    /// The box, metres, centred on the entity.
    pub size: Vec3,
    /// Metres between probes.
    pub spacing: f32,
    /// How much of each probe's picture a frame keeps: 0.97 blends a
    /// frame's rays in at three parts of a hundred.
    pub hysteresis: f32,
}

impl Default for IrradianceVolume {
    fn default() -> Self {
        Self {
            size: Vec3::new(32.0, 8.0, 32.0),
            spacing: 2.0,
            hysteresis: 0.97,
        }
    }
}

/// A volume placed in the world, for the renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedVolume {
    pub centre: Vec3,
    pub volume: IrradianceVolume,
}

impl PlacedVolume {
    /// Probes along each axis and how far apart, the spacing widened if
    /// there would be too many.
    pub fn grid(&self) -> ([u32; 3], f32) {
        let size = self.volume.size.abs().max(Vec3::splat(0.1));
        let mut spacing = self.volume.spacing.max(0.1);
        loop {
            let counts = (size / spacing).ceil().as_uvec3() + 1;
            let counts = [counts.x.max(2), counts.y.max(2), counts.z.max(2)];
            if counts[0] * counts[1] * counts[2] <= MOST_PROBES {
                return (counts, spacing);
            }
            spacing *= 1.25;
        }
    }

    /// Where the first probe is: the grid centred on the box.
    pub fn origin(&self) -> Vec3 {
        let (counts, spacing) = self.grid();
        let span = Vec3::new(counts[0] as f32 - 1.0, counts[1] as f32 - 1.0, counts[2] as f32 - 1.0) * spacing;
        self.centre - span * 0.5
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Step {
    /// The frame's turn of the rays.
    rotation: [[f32; 4]; 4],
    /// Rays a probe, how much of the old is kept, the farthest a distance
    /// counts, probes.
    rays: [f32; 4],
}

pub(crate) struct Ddgi {
    /// The probes' pictures: what the lit shader reads (group 0) and the
    /// update writes (group 3).
    pub(crate) probes: wgpu::Buffer,
    rays: wgpu::Buffer,
    step: wgpu::Buffer,
    placed: Option<PlacedVolume>,
    frames: u32,
    trace_layout: wgpu::BindGroupLayout,
    update_layout: wgpu::BindGroupLayout,
    pub(crate) trace_pipeline_layout: wgpu::PipelineLayout,
    pub(crate) update_pipeline_layout: wgpu::PipelineLayout,
    pub(crate) trace: Option<wgpu::ComputePipeline>,
    pub(crate) update: Option<wgpu::ComputePipeline>,
}

fn probes_buffer(gpu: &Gpu, count: u64) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ddgi probes"),
        size: (count.max(1) * PER_PROBE * 16).max(16),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn rays_buffer(gpu: &Gpu, count: u64) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ddgi rays"),
        size: (count.max(1) * RAYS as u64 * 16).max(16),
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    })
}

impl Ddgi {
    pub(crate) fn new(gpu: &Gpu, frame_layout: &wgpu::BindGroupLayout) -> Self {
        let compute = wgpu::ShaderStages::COMPUTE;
        let uniform = wgpu::BindGroupLayoutEntry {
            binding: 7,
            visibility: compute,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: compute,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let trace_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ddgi trace"),
            entries: &[uniform, storage(8)],
        });
        let update_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ddgi update"),
            entries: &[uniform, storage(8), storage(9)],
        });
        let trace_pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("runity::ddgi trace"),
            bind_group_layouts: &[Some(frame_layout), None, None, Some(&trace_layout)],
            immediate_size: 0,
        });
        let update_pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("runity::ddgi update"),
            bind_group_layouts: &[None, None, None, Some(&update_layout)],
            immediate_size: 0,
        });
        Self {
            probes: probes_buffer(gpu, 1),
            rays: rays_buffer(gpu, 1),
            step: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ddgi step"),
                size: std::mem::size_of::<Step>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            placed: None,
            frames: 0,
            trace_layout,
            update_layout,
            trace_pipeline_layout,
            update_pipeline_layout,
            trace: None,
            update: None,
        }
    }

    /// The pipelines, from the renderer's shader module: the trace only
    /// where the module traces.
    pub(crate) fn make_pipelines(
        &self,
        gpu: &Gpu,
        shader: &wgpu::ShaderModule,
        traced: bool,
    ) -> (Option<wgpu::ComputePipeline>, Option<wgpu::ComputePipeline>) {
        if !traced {
            return (None, None);
        }
        let make = |layout, entry| {
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("runity::ddgi"),
                layout: Some(layout),
                module: shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        (
            Some(make(&self.trace_pipeline_layout, "cs_ddgi_trace")),
            Some(make(&self.update_pipeline_layout, "cs_ddgi_update")),
        )
    }

    /// This frame's volume: `true` when the probes' buffer was made again
    /// (the frame's bind group has to follow).
    pub(crate) fn prepare(&mut self, gpu: &Gpu, placed: Option<PlacedVolume>) -> bool {
        let placed = placed.filter(|_| self.trace.is_some());
        if placed == self.placed {
            return false;
        }
        let was = self.placed.map(|p| p.grid().0);
        self.placed = placed;
        self.frames = 0;
        let now = placed.map(|p| p.grid().0);
        if was == now {
            return false;
        }
        let count = now.map_or(1, |c| c[0] as u64 * c[1] as u64 * c[2] as u64);
        self.probes = probes_buffer(gpu, count);
        self.rays = rays_buffer(gpu, count);
        true
    }

    /// The frame uniform's four `vec4`s: the first probe and the spacing;
    /// the counts and 1 when on; the biases.
    pub(crate) fn uniform(&self) -> [[f32; 4]; 4] {
        let Some(placed) = self.placed else {
            return [[0.0; 4]; 4];
        };
        let (counts, spacing) = placed.grid();
        let origin = placed.origin();
        [
            [origin.x, origin.y, origin.z, spacing],
            [counts[0] as f32, counts[1] as f32, counts[2] as f32, 1.0],
            // Pushed off the surface, along its normal and toward the eye,
            // before the probes are asked: a fifth and four fifths of a
            // quarter of the spacing.
            [0.2 * 0.25 * spacing, 0.8 * 0.25 * spacing, spacing * 1.5, 0.0],
            [0.0; 4],
        ]
    }

    /// Trace this frame's rays and blend them in: `frame_group` is the
    /// frame's bind group, with the scene's rays in it.
    pub(crate) fn run(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, frame_group: &wgpu::BindGroup) {
        let (Some(placed), Some(trace), Some(update)) = (self.placed, &self.trace, &self.update) else {
            return;
        };
        let (counts, spacing) = placed.grid();
        let probes = counts[0] * counts[1] * counts[2];
        // A turn a frame, never quite repeating (golden-ratio steps round
        // two axes): every direction round a probe is sampled in time.
        let f = self.frames as f32;
        let rotation = Mat4::from_quat(
            Quat::from_axis_angle(Vec3::new(0.36, 0.8, 0.48).normalize(), f * 2.399_963)
                * Quat::from_axis_angle(Vec3::X, f * 1.618_034),
        );
        // The first frames fill the pictures rather than blend into black.
        let keep = if self.frames < 4 {
            0.0
        } else {
            placed.volume.hysteresis.clamp(0.0, 0.999)
        };
        let step = Step {
            rotation: rotation.to_cols_array_2d(),
            rays: [RAYS as f32, keep, spacing * 1.5, probes as f32],
        };
        gpu.queue.write_buffer(&self.step, 0, bytemuck::bytes_of(&step));
        let trace_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ddgi trace"),
            layout: &self.trace_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: self.step.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: self.rays.as_entire_binding(),
                },
            ],
        });
        let update_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ddgi update"),
            layout: &self.update_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: self.step.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: self.rays.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: self.probes.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("runity::ddgi trace"),
                timestamp_writes: crate::gpu_timer::compute("ddgi trace"),
            });
            pass.set_pipeline(trace);
            pass.set_bind_group(0, frame_group, &[]);
            pass.set_bind_group(3, &trace_group, &[]);
            pass.dispatch_workgroups((probes * RAYS).div_ceil(64), 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("runity::ddgi update"),
                timestamp_writes: crate::gpu_timer::compute("ddgi update"),
            });
            pass.set_pipeline(update);
            pass.set_bind_group(3, &update_group, &[]);
            pass.dispatch_workgroups(probes, 1, 1);
        }
        self.frames = self.frames.saturating_add(1);
    }

    /// Whether a volume is being lit this frame.
    pub(crate) fn on(&self) -> bool {
        self.placed.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_volume_is_a_grid_centred_on_its_box_and_never_too_many_probes() {
        let placed = PlacedVolume {
            centre: Vec3::new(10.0, 2.0, 0.0),
            volume: IrradianceVolume {
                size: Vec3::new(8.0, 4.0, 8.0),
                spacing: 2.0,
                ..Default::default()
            },
        };
        let (counts, spacing) = placed.grid();
        assert_eq!(counts, [5, 3, 5]);
        assert_eq!(spacing, 2.0);
        assert_eq!(placed.origin(), Vec3::new(6.0, 0.0, -4.0));
        let huge = PlacedVolume {
            centre: Vec3::ZERO,
            volume: IrradianceVolume {
                size: Vec3::splat(1000.0),
                spacing: 1.0,
                ..Default::default()
            },
        };
        let (counts, spacing) = huge.grid();
        assert!(counts[0] * counts[1] * counts[2] <= MOST_PROBES);
        assert!(spacing > 1.0);
    }
}
