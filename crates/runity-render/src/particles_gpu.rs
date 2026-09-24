//! Particles on the GPU: `particles: (..., gpu: true)` — tens of thousands
//! at once.
//!
//! The emitter's timing — its rate, bursts, plays — stays where it is
//! ([`crate::particles::Emitting`]), which only counts how many it has
//! given off. Each frame the renderer gives the new ones life in a ring
//! of slots on the GPU, from random numbers by their number, and steps
//! every living one under gravity, in one compute pass; then draws them in
//! the colour pass after what is see-through, each a soft disc turned to
//! the camera, stretched along its way when asked, fading and shrinking as
//! it ages. Depth-tested, not written; blended premultiplied. Lit by
//! nothing: a spark, a grain of grit, a snowflake is its colour.
//!
//! Where they go is each machine's own, as with the CPU's: they are for the
//! eye (DNA, postulate 4).

use glam::{Mat4, Vec3};

use crate::gpu::Gpu;
use crate::scene::Emitter;

/// The most one emitter keeps alive at once.
pub const MOST: u32 = 65_536;

/// What the GPU needs of an emitter for a frame.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuEmitter {
    /// Which emitter, the same every frame: its pool of slots.
    pub key: u64,
    /// Where it is.
    pub placed: Mat4,
    pub emitter: Emitter,
    /// How many it has given off, ever.
    pub born: u64,
    /// Seconds it has run.
    pub lived: f32,
}

const SHADER: &str = r#"
struct Params {
    model: mat4x4<f32>,
    view_projection: mat4x4<f32>,
    // eye, and the seconds it has run
    eye: vec4<f32>,
    // speed, spread (radians), gravity, life
    motion: vec4<f32>,
    // size new, size at the end, stretch, seconds this step
    shape: vec4<f32>,
    color: vec4<f32>,
    end_color: vec4<f32>,
    // which way they leave, in its own axes; 1 when they stay in its space
    direction: vec4<f32>,
    // slots, where the new ones start, how many, and the first's number
    counts: vec4<u32>,
};

struct Particle {
    // where, and its age
    at: vec4<f32>,
    // how fast, and its life (0: a free slot)
    velocity: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;
// The same slots, read by the drawing: a vertex stage may not write.
@group(0) @binding(2) var<storage, read> living: array<Particle>;

fn hash(n: u32) -> f32 {
    var x = n * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    x = (x >> 22u) ^ x;
    return f32(x) / 4294967295.0;
}

fn up_of(d: vec3<f32>) -> vec3<f32> {
    let o = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(d.y) > 0.9);
    return normalize(cross(o, d));
}

@compute @workgroup_size(64)
fn cs_spawn(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= params.counts.z {
        return;
    }
    let slot = (params.counts.y + i) % params.counts.x;
    let n = params.counts.w + i;
    let spin = hash(n * 3u) * 6.2831853;
    let off = sqrt(hash(n * 3u + 1u)) * params.motion.y;
    let axis = normalize(params.direction.xyz);
    let side = up_of(axis);
    let other = cross(axis, side);
    let local_dir = axis * cos(off) + (side * cos(spin) + other * sin(spin)) * sin(off);
    let stays = params.direction.w > 0.5;
    var origin = vec3<f32>(0.0);
    var dir = local_dir;
    if !stays {
        origin = (params.model * vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;
        dir = normalize((params.model * vec4<f32>(local_dir, 0.0)).xyz);
    }
    // Given off some time within the step, as far on as it would be.
    let born = hash(n * 3u + 2u) * params.shape.w;
    let pull = vec3<f32>(0.0, params.motion.z, 0.0);
    var p: Particle;
    p.at = vec4<f32>(origin + dir * params.motion.x * born + pull * (0.5 * born * born), born);
    p.velocity = vec4<f32>(dir * params.motion.x + pull * born, params.motion.w);
    particles[slot] = p;
}

@compute @workgroup_size(64)
fn cs_step(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= params.counts.x {
        return;
    }
    var p = particles[i];
    if p.velocity.w <= 0.0 {
        return;
    }
    let dt = params.shape.w;
    p.velocity = vec4<f32>(p.velocity.xyz + vec3<f32>(0.0, params.motion.z, 0.0) * dt, p.velocity.w);
    p.at = vec4<f32>(p.at.xyz + p.velocity.xyz * dt, p.at.w + dt);
    if p.at.w >= p.velocity.w {
        p.velocity.w = 0.0;
    }
    particles[i] = p;
}

struct Varyings {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_particle(@builtin(vertex_index) v: u32, @builtin(instance_index) k: u32) -> Varyings {
    var out: Varyings;
    let p = living[k];
    if p.velocity.w <= 0.0 {
        out.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
        return out;
    }
    let t = clamp(p.at.w / p.velocity.w, 0.0, 1.0);
    var size = mix(params.shape.x, params.shape.y, t);
    var at = p.at.xyz;
    var velocity = p.velocity.xyz;
    if params.direction.w > 0.5 {
        at = (params.model * vec4<f32>(at, 1.0)).xyz;
        velocity = (params.model * vec4<f32>(velocity, 0.0)).xyz;
    }
    let to_eye = normalize(params.eye.xyz - at);
    // Along its way as the eye sees it, when it stretches; else any way.
    let seen = velocity - to_eye * dot(velocity, to_eye);
    var along = up_of(to_eye);
    var long = 1.0;
    if params.shape.z > 0.0 && length(seen) > 1e-4 {
        along = normalize(seen);
        long = 1.0 + length(velocity) * params.shape.z / max(size, 1e-4);
    }
    let across = cross(to_eye, along);
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[v];
    let world = at + (across * c.x + along * c.y * long) * size * 0.5;
    out.clip = params.view_projection * vec4<f32>(world, 1.0);
    out.uv = c;
    out.color = mix(params.color, params.end_color, t);
    return out;
}

@fragment
fn fs_particle(in: Varyings) -> @location(0) vec4<f32> {
    // A soft disc.
    let d = length(in.uv);
    let a = (1.0 - smoothstep(0.55, 1.0, d)) * in.color.a;
    if a <= 0.002 {
        discard;
    }
    return vec4<f32>(in.color.rgb * a, a);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    model: [[f32; 4]; 4],
    view_projection: [[f32; 4]; 4],
    eye: [f32; 4],
    motion: [f32; 4],
    shape: [f32; 4],
    color: [f32; 4],
    end_color: [f32; 4],
    direction: [f32; 4],
    counts: [u32; 4],
}

struct Pool {
    /// Held for the bind group.
    _buffer: wgpu::Buffer,
    params: wgpu::Buffer,
    group: wgpu::BindGroup,
    draw_group: wgpu::BindGroup,
    capacity: u32,
    head: u32,
    born: u64,
    lived: f32,
    /// The frame it was last in: a pool left out of a frame goes.
    seen: u64,
}

/// The renderer's particles on the GPU: its pipelines, and a pool of slots
/// for each emitter.
pub(crate) struct GpuParticles {
    layout: wgpu::BindGroupLayout,
    draw_layout: wgpu::BindGroupLayout,
    spawn: wgpu::ComputePipeline,
    step: wgpu::ComputePipeline,
    draw: wgpu::RenderPipeline,
    pools: std::collections::HashMap<u64, Pool>,
    frame: u64,
    /// The pools to draw this frame, in order.
    drawn: Vec<u64>,
}

fn linear(c: (f32, f32, f32)) -> [f32; 3] {
    let l = |v: f32| crate::material::srgb_to_linear(v.clamp(0.0, 1.0));
    [l(c.0), l(c.1), l(c.2)]
}

impl GpuParticles {
    pub(crate) fn new(gpu: &Gpu, format: wgpu::TextureFormat, depth: wgpu::TextureFormat, samples: u32) -> Self {
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("runity::gpu particles"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniform = |stages| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: stages,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let storage = |binding, stages, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: stages,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        // Stepped with the slots writable; drawn with them read — a buffer
        // is not both at once.
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu particles step"),
            entries: &[
                uniform(wgpu::ShaderStages::COMPUTE),
                storage(1, wgpu::ShaderStages::COMPUTE, false),
            ],
        });
        let draw_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu particles draw"),
            entries: &[
                uniform(wgpu::ShaderStages::VERTEX),
                storage(2, wgpu::ShaderStages::VERTEX, true),
            ],
        });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu particles step"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let draw_pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu particles draw"),
            bind_group_layouts: &[Some(&draw_layout)],
            immediate_size: 0,
        });
        let compute = |entry: &str| {
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("runity::gpu particles"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let draw = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("runity::gpu particles"),
            layout: Some(&draw_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_particle"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_particle"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        Self {
            spawn: compute("cs_spawn"),
            step: compute("cs_step"),
            layout,
            draw_layout,
            draw,
            pools: std::collections::HashMap::new(),
            frame: 0,
            drawn: Vec::new(),
        }
    }

    /// Drawn into a scene of `samples` a pixel now: the draw pipeline made
    /// again, the pools kept.
    pub(crate) fn resample(&mut self, gpu: &Gpu, format: wgpu::TextureFormat, depth: wgpu::TextureFormat, samples: u32) {
        self.draw = Self::new(gpu, format, depth, samples).draw;
    }

    /// Give off and step this frame's particles, before the colour pass.
    pub(crate) fn run(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        emitters: &[GpuEmitter],
        view_projection: Mat4,
        eye: Vec3,
    ) {
        self.frame += 1;
        self.drawn.clear();
        for e in emitters {
            let em = &e.emitter;
            let want = ((em.rate.max(0.0) * em.life.max(0.01) * 1.3).ceil() as u32
                + em.bursts.iter().map(|(_, n)| *n).sum::<u32>())
            .clamp(64, MOST)
            .next_power_of_two()
            .min(MOST);
            let fresh = self.pools.get(&e.key).is_none_or(|p| p.capacity < want);
            if fresh {
                let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpu particles"),
                    size: want as u64 * 32,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                // All slots free.
                gpu.queue.write_buffer(&buffer, 0, &vec![0u8; want as usize * 32]);
                let params = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpu particles"),
                    size: std::mem::size_of::<Params>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gpu particles step"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: buffer.as_entire_binding(),
                        },
                    ],
                });
                let draw_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gpu particles draw"),
                    layout: &self.draw_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: buffer.as_entire_binding(),
                        },
                    ],
                });
                let (born, lived) = self
                    .pools
                    .get(&e.key)
                    .map_or((e.born, e.lived), |p| (p.born, p.lived));
                self.pools.insert(
                    e.key,
                    Pool {
                        _buffer: buffer,
                        params,
                        group,
                        draw_group,
                        capacity: want,
                        head: 0,
                        born,
                        lived,
                        seen: 0,
                    },
                );
            }
            let pool = self.pools.get_mut(&e.key).expect("made above");
            pool.seen = self.frame;
            // Given off since last frame (a restart counts from nothing).
            if e.born < pool.born || e.lived < pool.lived {
                pool.born = e.born;
                pool.lived = e.lived;
            }
            let spawn = (e.born - pool.born).min(pool.capacity as u64) as u32;
            let dt = (e.lived - pool.lived).clamp(0.0, 0.25);
            let first = pool.born as u32;
            pool.born = e.born;
            pool.lived = e.lived;
            let direction = em.direction.unwrap_or(Vec3::Y).normalize_or(Vec3::Y);
            let color = linear(em.color);
            let end = linear(em.end_color.unwrap_or(em.color));
            let end_alpha = em.end_alpha.unwrap_or(em.alpha);
            let params = Params {
                model: e.placed.to_cols_array_2d(),
                view_projection: view_projection.to_cols_array_2d(),
                eye: [eye.x, eye.y, eye.z, e.lived],
                motion: [em.speed, em.spread_deg.to_radians(), em.gravity, em.life.max(0.01)],
                shape: [em.size, em.end_size.unwrap_or(0.0), em.stretch.max(0.0), dt],
                color: [color[0], color[1], color[2], em.alpha.clamp(0.0, 1.0)],
                end_color: [end[0], end[1], end[2], end_alpha.clamp(0.0, 1.0)],
                direction: [direction.x, direction.y, direction.z, if em.local { 1.0 } else { 0.0 }],
                counts: [pool.capacity, pool.head, spawn, first],
            };
            gpu.queue.write_buffer(&pool.params, 0, bytemuck::bytes_of(&params));
            pool.head = (pool.head + spawn) % pool.capacity;
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("runity::gpu particles"),
                timestamp_writes: crate::gpu_timer::compute("gpu particles"),
            });
            pass.set_bind_group(0, &pool.group, &[]);
            if spawn > 0 {
                pass.set_pipeline(&self.spawn);
                pass.dispatch_workgroups(spawn.div_ceil(64), 1, 1);
            }
            pass.set_pipeline(&self.step);
            pass.dispatch_workgroups(pool.capacity.div_ceil(64), 1, 1);
            drop(pass);
            self.drawn.push(e.key);
        }
        let frame = self.frame;
        self.pools.retain(|_, p| p.seen == frame);
    }

    /// Draw this frame's particles, in the colour pass after what is
    /// see-through.
    pub(crate) fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        if self.drawn.is_empty() {
            return;
        }
        pass.set_pipeline(&self.draw);
        for key in &self.drawn {
            if let Some(pool) = self.pools.get(key) {
                pass.set_bind_group(0, &pool.draw_group, &[]);
                pass.draw(0..6, 0..pool.capacity);
            }
        }
    }

    /// How many slots the pools hold.
    pub(crate) fn slots(&self) -> u32 {
        self.pools.values().map(|p| p.capacity).sum()
    }
}
