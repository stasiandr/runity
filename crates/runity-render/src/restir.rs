//! ReSTIR DI (Bitterli et al., 2020): the lamps' light by one shadow ray a
//! pixel, whatever the number of lamps.
//!
//! `ray_tracing: (light_shadows: true, restir: true)`. Traced lamp shadows
//! are a ray a lamp a pixel: a courtyard of fifteen lanterns traces
//! fifteen. With ReSTIR each pixel keeps a *reservoir*: one lamp, chosen
//! from candidates in proportion to how much unshadowed light each gives
//! it, and the weight that makes the choice fair. Two compute passes after
//! the prepass fill them:
//!
//! 1. **Candidates and time.** Eight lamps drawn at random from the
//!    pixel's light cell, each kept or passed over by its share; then the
//!    reservoir this pixel had last frame, where it was then (through the
//!    last camera), if the surface there is the same (its depth within a
//!    tenth, its normal within 25°), counted at most twenty times this
//!    frame's.
//! 2. **Neighbours and the ray.** Three reservoirs of pixels round it, on
//!    the same surface, weighed at this pixel; then one ray to the lamp
//!    chosen — blocked, it gives nothing.
//!
//! The lit shader then lights the pixel by that one lamp, times the
//! weight: on average all the lamps, with their shadows. What is left is
//! noise, which TAA settles. The sun is not a candidate: it has its own
//! rays. Unbiased only as far as ReSTIR's simplest form is: the neighbours
//! are weighed without their visibility being traced again.

use crate::gpu::Gpu;

/// A pixel's reservoir: its lamp, the weights' sum, how many it stands
/// for, and its weight — four floats (the lamp's index as bits).
const RESERVOIR: u64 = 16;

pub(crate) struct Restir {
    size: (u32, u32),
    temporary: wgpu::Buffer,
    finals: [wgpu::Buffer; 2],
    geometry: [wgpu::Buffer; 2],
    /// What the lit shader reads: this frame's final reservoirs, copied.
    pub(crate) shade: wgpu::Buffer,
    now: usize,
    frames: u32,
    layout: wgpu::BindGroupLayout,
    pub(crate) pipeline_layout: wgpu::PipelineLayout,
    pub(crate) initial: Option<wgpu::ComputePipeline>,
    pub(crate) spatial: Option<wgpu::ComputePipeline>,
}

fn buffer(gpu: &Gpu, label: &str, pixels: u64, extra: wgpu::BufferUsages) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (pixels.max(1) * RESERVOIR).max(16),
        usage: wgpu::BufferUsages::STORAGE | extra,
        mapped_at_creation: false,
    })
}

impl Restir {
    pub(crate) fn new(gpu: &Gpu, frame_layout: &wgpu::BindGroupLayout) -> Self {
        let compute = wgpu::ShaderStages::COMPUTE;
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: compute,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("restir"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: compute,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: compute,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage(12, false),
                storage(13, true),
                storage(14, true),
                storage(15, false),
                storage(16, false),
            ],
        });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("runity::restir"),
            bind_group_layouts: &[Some(frame_layout), None, None, Some(&layout)],
            immediate_size: 0,
        });
        let empty = wgpu::BufferUsages::empty();
        Self {
            size: (0, 0),
            temporary: buffer(gpu, "restir candidates", 1, empty),
            finals: [buffer(gpu, "restir", 1, wgpu::BufferUsages::COPY_SRC), buffer(gpu, "restir", 1, wgpu::BufferUsages::COPY_SRC)],
            geometry: [buffer(gpu, "restir surface", 1, empty), buffer(gpu, "restir surface", 1, empty)],
            shade: buffer(gpu, "restir shade", 1, wgpu::BufferUsages::COPY_DST),
            now: 0,
            frames: 0,
            layout,
            pipeline_layout,
            initial: None,
            spatial: None,
        }
    }

    /// The pipelines, from the renderer's module: only where it traces.
    pub(crate) fn make_pipelines(
        &self,
        gpu: &Gpu,
        shader: &wgpu::ShaderModule,
        traced: bool,
    ) -> (Option<wgpu::ComputePipeline>, Option<wgpu::ComputePipeline>) {
        if !traced {
            return (None, None);
        }
        let make = |entry| {
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("runity::restir"),
                layout: Some(&self.pipeline_layout),
                module: shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        (Some(make("cs_restir_initial")), Some(make("cs_restir_spatial")))
    }

    /// Sized to the frame: `true` when the buffer the lit shader reads was
    /// made again (the frame's group has to follow). A new size starts
    /// the reservoirs' history again.
    pub(crate) fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) -> bool {
        if self.size == size {
            return false;
        }
        self.size = size;
        let pixels = size.0 as u64 * size.1 as u64;
        let empty = wgpu::BufferUsages::empty();
        self.temporary = buffer(gpu, "restir candidates", pixels, empty);
        self.finals = [
            buffer(gpu, "restir", pixels, wgpu::BufferUsages::COPY_SRC),
            buffer(gpu, "restir", pixels, wgpu::BufferUsages::COPY_SRC),
        ];
        self.geometry = [buffer(gpu, "restir surface", pixels, empty), buffer(gpu, "restir surface", pixels, empty)];
        self.shade = buffer(gpu, "restir shade", pixels, wgpu::BufferUsages::COPY_DST);
        self.frames = 0;
        true
    }

    /// The frame uniform's part: on, the size, the frame's number.
    pub(crate) fn uniform(&self, on: bool) -> [f32; 4] {
        if !on {
            return [0.0; 4];
        }
        [1.0, self.size.0 as f32, self.size.1 as f32, self.frames as f32]
    }

    /// Both passes, and this frame's reservoirs where the lit shader reads
    /// them.
    pub(crate) fn run(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        frame_group: &wgpu::BindGroup,
        depth: &wgpu::TextureView,
        normals: &wgpu::TextureView,
    ) {
        let (Some(initial), Some(spatial)) = (&self.initial, &self.spatial) else {
            return;
        };
        let (now, before) = (self.now, 1 - self.now);
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("restir"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::TextureView(normals),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: self.temporary.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: self.finals[before].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: self.geometry[before].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 15,
                    resource: self.finals[now].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 16,
                    resource: self.geometry[now].as_entire_binding(),
                },
            ],
        });
        let groups = (self.size.0.div_ceil(8), self.size.1.div_ceil(8));
        for (pipeline, label) in [(initial, "restir"), (spatial, "restir spatial")] {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: crate::gpu_timer::compute(if label == "restir" { "restir" } else { "restir spatial" }),
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, frame_group, &[]);
            pass.set_bind_group(3, &group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        let bytes = self.size.0 as u64 * self.size.1 as u64 * RESERVOIR;
        encoder.copy_buffer_to_buffer(&self.finals[now], 0, &self.shade, 0, bytes);
        self.now = before;
        self.frames = self.frames.wrapping_add(1);
    }
}
