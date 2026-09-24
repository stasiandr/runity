//! Smoke simulated on the GPU: `runity_fluid::smoke`'s stable fluids, a
//! thread a cell ([`crate::volume::GpuSmoke`] says what and how far).
//!
//! The world keeps the clock and what is solid round each smoke; the
//! renderer keeps the air — speed, smoke, heat, pressure — in buffers of
//! its own and steps it where the frame is drawn, straight into the fog's
//! picture, with nothing read back. A fire burns every frame and never
//! sleeps: on the CPU three of them took 7 ms a step at their best, here
//! a small part of one.
//!
//! The same passes as the CPU's, in the same order, on every cell of the
//! grid rather than only its awake blocks: what is empty stays empty. Not
//! bit for bit the CPU's — nothing reads it but the eye.

use std::collections::HashMap;

use crate::gpu::Gpu;
use crate::volume::GpuSmoke;

const SHADER: &str = include_str!("smoke_gpu.wgsl");
/// Pressure sweeps a step, from the last step's (the CPU's number), in
/// pairs: there and back.
const SWEEPS: usize = 10;
const _: () = assert!(SWEEPS % 2 == 0);
/// Steps a frame takes at the most: a game's frame owes a few (its clock
/// gives at most a fifth of a second). More — a tool settling a scene for
/// its picture owes seconds — wait for the next frames: one command
/// buffer of thousands of passes is more than the GPU lets run.
const MOST_STEPS: u32 = 8;
/// A step's uniform, at the device's dynamic-offset alignment.
const SLOT: u64 = 256;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    n: [u32; 4],
    step: [f32; 4],
    source: [f32; 4],
    look: [f32; 4],
    wind: [f32; 4],
    pack: [u32; 4],
    stride: [f32; 4],
}

/// One smoke's air on the GPU.
struct Sim {
    n: [u32; 3],
    cells: u32,
    params: wgpu::Buffer,
    buffers: [wgpu::Buffer; 11],
    solid_version: Option<u64>,
    group: Option<(wgpu::BindGroup, u32)>,
    seen: u64,
    /// How far its air has been stepped, seconds.
    clock: f32,
}

/// A smoke's work this frame: its group, the grid's workgroups, the steps,
/// the picture's size, and whether it curls.
struct Job {
    group: wgpu::BindGroup,
    groups: [u32; 3],
    steps: u32,
    size: [u32; 3],
    curly: bool,
}

pub(crate) struct SmokeSim {
    layout: wgpu::BindGroupLayout,
    pipelines: HashMap<&'static str, wgpu::ComputePipeline>,
    sims: HashMap<u64, Sim>,
    frame: u64,
}

const ENTRIES: [&str; 12] = [
    "cs_forces",
    "cs_curl",
    "cs_confine",
    "cs_advect_vel",
    "cs_keep_vel",
    "cs_divergence",
    "cs_jacobi",
    "cs_jacobi_back",
    "cs_gradient",
    "cs_advect_scalars",
    "cs_keep_scalars",
    "cs_pack",
];

impl SmokeSim {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64),
            },
            count: None,
        }];
        for b in 1..=10 {
            entries.push(storage(b, false));
        }
        entries.push(storage(11, true));
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 12,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba8Unorm,
                view_dimension: wgpu::TextureViewDimension::D3,
            },
            count: None,
        });
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("runity::smoke (gpu)"),
            entries: &entries,
        });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("runity::smoke (gpu)"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("runity::smoke (gpu)"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipelines = ENTRIES
            .iter()
            .map(|&entry| {
                (
                    entry,
                    gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: Some(&pipeline_layout),
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    }),
                )
            })
            .collect();
        Self {
            layout,
            pipelines,
            sims: HashMap::new(),
            frame: 0,
        }
    }

    /// Step each smoke as far as its clock says and write it into its slab
    /// of the fog's picture (`picture`, the smoke texture's storage view).
    /// `smokes` are the frame's GPU smokes with their slabs, and the size
    /// of the picture each takes.
    pub(crate) fn run(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        smokes: &[(u32, [u32; 3], &GpuSmoke)],
        picture: &wgpu::TextureView,
    ) {
        self.frame += 1;
        let mut jobs: Vec<Job> = Vec::new();
        for &(slab, size, smoke) in smokes {
            let cells = smoke.n[0] * smoke.n[1] * smoke.n[2];
            if cells == 0 {
                continue;
            }
            let remade = self.sims.get(&smoke.key).is_none_or(|s| s.n != smoke.n);
            if remade {
                self.sims.insert(smoke.key, Sim::new(gpu, smoke.n, cells));
            }
            let sim = self.sims.get_mut(&smoke.key).expect("made above");
            sim.seen = self.frame;
            if sim.solid_version != Some(smoke.solid_version) && smoke.solid.len() == cells as usize {
                gpu.queue.write_buffer(&sim.buffers[10], 0, bytemuck::cast_slice(&smoke.solid));
                sim.solid_version = Some(smoke.solid_version);
            }
            let group = match &sim.group {
                Some((group, s)) if *s == slab => group.clone(),
                _ => {
                    let group = sim.bind(gpu, &self.layout, picture);
                    sim.group = Some((group.clone(), slab));
                    group
                }
            };
            // Up to the world's clock, a few steps a frame; a clock gone
            // back (the world started again) starts the air again.
            if smoke.clock + 1e-4 < sim.clock {
                *sim = Sim::new(gpu, smoke.n, cells);
                sim.seen = self.frame;
            }
            let owed = ((smoke.clock - sim.clock) / crate::volume::SMOKE_STEP + 0.5).max(0.0) as u32;
            let steps = owed.min(MOST_STEPS);
            let from = sim.clock;
            sim.clock += crate::volume::SMOKE_STEP * steps as f32;
            // A step's numbers each, the last slot the picture's.
            let stride = crate::volume::smoke_stride(smoke.n);
            let heat_most = (smoke.heat * 0.2).max(0.05);
            let dt = crate::volume::SMOKE_STEP;
            let wind = smoke.wind;
            for s in 0..=steps {
                let time = from + dt * (s + 1) as f32;
                let flicker = 0.8 + 0.2 * (time * 13.0).sin() * (time * 7.3).cos();
                let params = Params {
                    n: [smoke.n[0], smoke.n[1], smoke.n[2], cells],
                    step: [smoke.dx, dt, time, flicker],
                    source: [(smoke.source / smoke.dx).max(1.0), smoke.rate, smoke.heat, smoke.heat],
                    look: [smoke.weight, smoke.curl.max(0.0), (1.0 - smoke.fade.max(0.0) * dt).max(0.0), 0.0],
                    wind: [wind.x, wind.y, wind.z, 0.0],
                    pack: [size[0], size[1], size[2], slab * crate::volume::SMOKE_MOST[2]],
                    stride: [stride[0] as f32, stride[1] as f32, stride[2] as f32, heat_most],
                };
                gpu.queue.write_buffer(&sim.params, s as u64 * SLOT, bytemuck::bytes_of(&params));
            }
            let groups = [smoke.n[0].div_ceil(4), smoke.n[1].div_ceil(4), smoke.n[2].div_ceil(4)];
            jobs.push(Job {
                group,
                groups,
                steps,
                size,
                curly: smoke.curl > 0.0,
            });
        }
        // Every smoke's steps in one pass: wgpu orders what each dispatch
        // writes before what the next reads, and a pass apiece would cost
        // the CPU more than the GPU spends.
        if !jobs.is_empty() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("runity::smoke (gpu)"),
                timestamp_writes: None,
            });
            for Job { group, groups, steps, size, curly } in &jobs {
                for s in 0..*steps {
                    pass.set_bind_group(0, group, &[(s as u64 * SLOT) as u32]);
                    let mut run = |entry: &str| {
                        pass.set_pipeline(&self.pipelines[entry]);
                        pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
                    };
                    run("cs_forces");
                    if *curly {
                        run("cs_curl");
                        run("cs_confine");
                    }
                    run("cs_advect_vel");
                    run("cs_keep_vel");
                    run("cs_divergence");
                    for _ in 0..SWEEPS / 2 {
                        run("cs_jacobi");
                        run("cs_jacobi_back");
                    }
                    run("cs_gradient");
                    run("cs_advect_scalars");
                    run("cs_keep_scalars");
                }
                // Into the picture, every frame (the slab may be another's
                // last).
                pass.set_bind_group(0, group, &[(*steps as u64 * SLOT) as u32]);
                pass.set_pipeline(&self.pipelines["cs_pack"]);
                pass.dispatch_workgroups(size[0].div_ceil(4), size[1].div_ceil(4), size[2].div_ceil(4));
            }
        }
        // A smoke gone from the frames lets its air go.
        let frame = self.frame;
        self.sims.retain(|_, s| frame - s.seen < 120);
    }
}

impl Sim {
    fn new(gpu: &Gpu, n: [u32; 3], cells: u32) -> Self {
        let buffer = |label: &str, bytes: u64| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.max(16),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                // Zeroed: still air, no smoke.
                mapped_at_creation: false,
            })
        };
        let c = cells as u64;
        let buffers = [
            buffer("smoke speed", c * 16),
            buffer("smoke speed (next)", c * 16),
            buffer("smoke", c * 4),
            buffer("smoke (next)", c * 4),
            buffer("smoke heat", c * 4),
            buffer("smoke heat (next)", c * 4),
            buffer("smoke pressure", c * 4),
            buffer("smoke pressure (next)", c * 4),
            buffer("smoke divergence", c * 4),
            buffer("smoke curl", c * 16),
            buffer("smoke solid", c * 4),
        ];
        let params = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smoke steps"),
            size: SLOT * (MOST_STEPS as u64 + 1),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            n,
            cells,
            params,
            buffers,
            solid_version: None,
            group: None,
            seen: 0,
            clock: 0.0,
        }
    }

    fn bind(&self, gpu: &Gpu, layout: &wgpu::BindGroupLayout, picture: &wgpu::TextureView) -> wgpu::BindGroup {
        let _ = self.cells;
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &self.params,
                offset: 0,
                size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64),
            }),
        }];
        // Bindings 1..=11 in the shader's order: speed, speed next, smoke,
        // smoke next, heat, heat next, pressure, pressure next, divergence,
        // curl, solid.
        let order = [0usize, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        for (b, &i) in order.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: b as u32 + 1,
                resource: self.buffers[i].as_entire_binding(),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: 12,
            resource: wgpu::BindingResource::TextureView(picture),
        });
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("runity::smoke (gpu)"),
            layout,
            entries: &entries,
        })
    }
}
