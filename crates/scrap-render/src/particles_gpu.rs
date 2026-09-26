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
//!
//! Where each is born, how it moves and how it looks is an effect graph's
//! three functions ([`scrap_shadergraph::effect`]): the emitter's `graph`,
//! or the empty graph — the emitter's numbers as they are — when it names
//! none. Each graph is its own pipelines, over the same slots.

use glam::{Mat4, Vec3};

use crate::gpu::Gpu;
use crate::scene::Emitter;

/// The most one emitter keeps alive at once.
pub const MOST: u32 = 65_536;

/// Bytes a particle's slot takes: three vec4s.
const SLOT: u64 = 48;

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
    // to the world from the screen
    inverse_view_projection: mat4x4<f32>,
    // 1 when they bounce off the scene's depth, the bounce, how thick a
    // surface is taken to be, 0
    collide: vec4<f32>,
};

struct Particle {
    // where, and its age
    at: vec4<f32>,
    // how fast, and its life (0: a free slot)
    velocity: vec4<f32>,
    // its own random number from 0 to 1, and room
    extra: vec4<f32>,
};

// What the effect's functions read of a particle and its emitter.
struct Effect {
    time: f32,
    seed: f32,
    life: f32,
    t: f32,
    age: f32,
    dt: f32,
    origin: vec3<f32>,
    cone: vec3<f32>,
    position: vec3<f32>,
    velocity: vec3<f32>,
};

struct Born {
    position: vec3<f32>,
    velocity: vec3<f32>,
    life: f32,
};

struct Moved {
    velocity: vec3<f32>,
    position: vec3<f32>,
};

struct Looks {
    color: vec3<f32>,
    alpha: f32,
    size: f32,
};

// What an emitter with no graph does.
fn born_default(e: Effect) -> Born {
    return Born(e.origin, e.cone * params.motion.x, params.motion.w);
}

fn moved_default(e: Effect) -> Moved {
    return Moved(e.velocity + vec3<f32>(0.0, params.motion.z, 0.0) * e.dt, e.position);
}

fn looks_default(e: Effect) -> Looks {
    let c = mix(params.color, params.end_color, e.t);
    return Looks(c.rgb, c.a, mix(params.shape.x, params.shape.y, e.t));
}

fn effect_of(p: Particle) -> Effect {
    var e: Effect;
    e.time = params.eye.w;
    e.seed = p.extra.x;
    e.life = p.velocity.w;
    e.age = p.at.w;
    e.t = clamp(p.at.w / max(p.velocity.w, 1e-6), 0.0, 1.0);
    e.dt = params.shape.w;
    e.position = p.at.xyz;
    e.velocity = p.velocity.xyz;
    return e;
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;
// The same slots, read by the drawing: a vertex stage may not write.
@group(0) @binding(2) var<storage, read> living: array<Particle>;
// The scene's depth, from the prepass: what they bounce off.
@group(1) @binding(0) var scene_depth: texture_depth_2d;

/// Where the scene is, in the world, at a pixel of its depth.
fn scene_point(pixel: vec2<i32>, size: vec2<f32>) -> vec3<f32> {
    let d = textureLoad(scene_depth, pixel, 0);
    let uv = (vec2<f32>(pixel) + 0.5) / size;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let w = params.inverse_view_projection * ndc;
    return w.xyz / w.w;
}

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
    var e: Effect;
    e.time = params.eye.w;
    e.seed = hash(n * 5u + 4u);
    e.dt = params.shape.w;
    e.origin = origin;
    e.cone = dir;
    let b = effect_spawn(e);
    // Given off some time within the step, as far on as it would be.
    let born = hash(n * 3u + 2u) * params.shape.w;
    let pull = vec3<f32>(0.0, params.motion.z, 0.0);
    var p: Particle;
    p.at = vec4<f32>(b.position + b.velocity * born + pull * (0.5 * born * born), born);
    p.velocity = vec4<f32>(b.velocity + pull * born, max(b.life, 1e-3));
    p.extra = vec4<f32>(e.seed, 0.0, 0.0, 0.0);
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
    let m = effect_update(effect_of(p));
    p.velocity = vec4<f32>(m.velocity, p.velocity.w);
    p.at = vec4<f32>(m.position, p.at.w + dt);
    // Behind what is drawn, and not far behind: back onto the surface, and
    // bounced off it.
    if params.collide.x > 0.5 {
        let clip = params.view_projection * vec4<f32>(p.at.xyz, 1.0);
        if clip.w > 0.0 {
            let ndc = clip.xyz / clip.w;
            if all(abs(ndc.xy) < vec2<f32>(0.999)) {
                let size = vec2<f32>(textureDimensions(scene_depth));
                let pixel = vec2<i32>((vec2<f32>(ndc.x, -ndc.y) * 0.5 + 0.5) * size);
                let surface = scene_point(pixel, size);
                let depth = textureLoad(scene_depth, pixel, 0);
                if ndc.z > depth && distance(surface, p.at.xyz) < params.collide.z {
                    let right = scene_point(pixel + vec2<i32>(1, 0), size) - surface;
                    let down = scene_point(pixel + vec2<i32>(0, 1), size) - surface;
                    var n = normalize(cross(down, right));
                    if dot(n, params.eye.xyz - surface) < 0.0 {
                        n = -n;
                    }
                    let v = p.velocity.xyz;
                    let into = dot(v, n);
                    if into < 0.0 {
                        let along = v - n * into;
                        p.velocity = vec4<f32>(along * 0.8 - n * into * params.collide.y, p.velocity.w);
                    }
                    p.at = vec4<f32>(surface + n * 0.01, p.at.w);
                }
            }
        }
    }
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
    let looks = effect_output(effect_of(p));
    let size = looks.size;
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
    out.color = vec4<f32>(looks.color, looks.alpha);
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

// scrap:effect
"#;

/// The GPU particles' shader with an effect graph's three functions in
/// (from [`scrap_shadergraph::effect::to_wgsl`]).
pub fn with_effect(effect: &str) -> String {
    SHADER.replace("// scrap:effect\n", effect)
}

/// Whether an effect graph's functions build into the particles' shader,
/// without a GPU: parsed and validated as the renderer would. What `scrap
/// check` asks of every effect.
pub fn check_effect(effect: &str) -> Result<(), String> {
    use wgpu::naga;
    let full = with_effect(effect);
    let module = naga::front::wgsl::parse_str(&full).map_err(|e| e.emit_to_string(&full))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .map_err(|e| e.emit_to_string(&full))?;
    Ok(())
}

/// The empty graph: an emitter's numbers as they are.
fn plain_effect() -> String {
    scrap_shadergraph::effect::to_wgsl(&Default::default(), "no graph").expect("the empty graph compiles")
}

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
    inverse_view_projection: [[f32; 4]; 4],
    collide: [f32; 4],
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

/// One effect's pipelines: an emitter's graph, or the plain one ("").
struct Kind {
    module: wgpu::ShaderModule,
    spawn: wgpu::ComputePipeline,
    step: wgpu::ComputePipeline,
    draw: wgpu::RenderPipeline,
}

/// Each effect's draw pipeline for one sampling of the scene: kept by the
/// renderer to swap back in.
pub(crate) struct Draws {
    samples: u32,
    draws: std::collections::HashMap<String, wgpu::RenderPipeline>,
}

/// The renderer's particles on the GPU: its pipelines, and a pool of slots
/// for each emitter.
pub(crate) struct GpuParticles {
    layout: wgpu::BindGroupLayout,
    draw_layout: wgpu::BindGroupLayout,
    depth_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    draw_pipeline_layout: wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    depth: wgpu::TextureFormat,
    samples: u32,
    /// By graph name; "" is an emitter with none.
    kinds: std::collections::HashMap<String, Kind>,
    pools: std::collections::HashMap<u64, Pool>,
    frame: u64,
    /// The pools to draw this frame, in order, with the effect of each.
    drawn: Vec<(u64, String)>,
}

fn linear(c: (f32, f32, f32)) -> [f32; 3] {
    let l = |v: f32| crate::material::srgb_to_linear(v.clamp(0.0, 1.0));
    [l(c.0), l(c.1), l(c.2)]
}

impl GpuParticles {
    pub(crate) fn new(gpu: &Gpu, format: wgpu::TextureFormat, depth: wgpu::TextureFormat, samples: u32) -> Self {
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
        let depth_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu particles depth"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu particles step"),
            bind_group_layouts: &[Some(&layout), Some(&depth_layout)],
            immediate_size: 0,
        });
        let draw_pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu particles draw"),
            bind_group_layouts: &[Some(&draw_layout)],
            immediate_size: 0,
        });
        let mut particles = Self {
            layout,
            draw_layout,
            depth_layout,
            pipeline_layout,
            draw_pipeline_layout,
            format,
            depth,
            samples,
            kinds: std::collections::HashMap::new(),
            pools: std::collections::HashMap::new(),
            frame: 0,
            drawn: Vec::new(),
        };
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scrap::gpu particles"),
            source: wgpu::ShaderSource::Wgsl(with_effect(&plain_effect()).into()),
        });
        let kind = particles.kind(gpu, module);
        particles.kinds.insert(String::new(), kind);
        particles
    }

    /// An effect's pipelines from its module, drawing at the present
    /// sampling.
    fn kind(&self, gpu: &Gpu, module: wgpu::ShaderModule) -> Kind {
        let compute = |entry: &str| {
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("scrap::gpu particles"),
                layout: Some(&self.pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Kind {
            spawn: compute("cs_spawn"),
            step: compute("cs_step"),
            draw: self.draw_pipeline(gpu, &module, self.samples),
            module,
        }
    }

    fn draw_pipeline(&self, gpu: &Gpu, module: &wgpu::ShaderModule, samples: u32) -> wgpu::RenderPipeline {
        gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scrap::gpu particles"),
            layout: Some(&self.draw_pipeline_layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_particle"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_particle"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: self.depth,
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
        })
    }

    /// Put in effect `name` from its graph's functions
    /// ([`scrap_shadergraph::effect::to_wgsl`]): emitters naming it draw
    /// with it from the next frame, their particles kept. Refused in words
    /// when it does not build, and the one before goes on.
    pub(crate) fn set_effect(&mut self, gpu: &Gpu, name: &str, effect: &str) -> Result<(), String> {
        check_effect(effect)?;
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scrap::gpu particles effect"),
            source: wgpu::ShaderSource::Wgsl(with_effect(effect).into()),
        });
        let kind = self.kind(gpu, module);
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(format!("the effect does not fit the particles: {error}"));
        }
        self.kinds.insert(name.to_string(), kind);
        Ok(())
    }

    /// Whether effect `name` is in.
    pub(crate) fn has_effect(&self, name: &str) -> bool {
        self.kinds.contains_key(name)
    }

    /// Every effect's draw pipeline for a scene of `samples` a pixel.
    pub(crate) fn make_draw(&self, gpu: &Gpu, samples: u32) -> Draws {
        Draws {
            samples,
            draws: self
                .kinds
                .iter()
                .map(|(name, kind)| (name.clone(), self.draw_pipeline(gpu, &kind.module, samples)))
                .collect(),
        }
    }

    /// Draw with `draws` from now, the pools kept; the ones they replace
    /// back. An effect put in since they were made is made for them.
    pub(crate) fn swap_draw(&mut self, gpu: &Gpu, mut draws: Draws) -> Draws {
        let mut old = Draws {
            samples: self.samples,
            draws: std::collections::HashMap::new(),
        };
        self.samples = draws.samples;
        let names: Vec<String> = self.kinds.keys().cloned().collect();
        for name in names {
            let draw = match draws.draws.remove(&name) {
                Some(d) => d,
                None => self.draw_pipeline(gpu, &self.kinds[&name].module, self.samples),
            };
            let kind = self.kinds.get_mut(&name).expect("listed above");
            old.draws.insert(name, std::mem::replace(&mut kind.draw, draw));
        }
        old
    }

    /// Give off and step this frame's particles, before the colour pass.
    pub(crate) fn run(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        emitters: &[GpuEmitter],
        view_projection: Mat4,
        eye: Vec3,
        depth: &wgpu::TextureView,
        depth_drawn: bool,
    ) {
        self.frame += 1;
        self.drawn.clear();
        // The scene's depth this frame, for those that bounce off it.
        let depth_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu particles depth"),
            layout: &self.depth_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(depth),
            }],
        });
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
                    size: want as u64 * SLOT,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                // All slots free.
                gpu.queue.write_buffer(&buffer, 0, &vec![0u8; (want as u64 * SLOT) as usize]);
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
                inverse_view_projection: view_projection.inverse().to_cols_array_2d(),
                collide: [if em.collide && depth_drawn { 1.0 } else { 0.0 }, 0.35, 0.4, 0.0],
            };
            gpu.queue.write_buffer(&pool.params, 0, bytemuck::bytes_of(&params));
            pool.head = (pool.head + spawn) % pool.capacity;
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("scrap::gpu particles"),
                timestamp_writes: crate::gpu_timer::compute("gpu particles"),
            });
            // An effect not (yet) in draws as an emitter without one: what
            // `scrap check` says of a name nothing answers to.
            let effect = if self.kinds.contains_key(&em.graph) { em.graph.as_str() } else { "" };
            let kind = &self.kinds[effect];
            pass.set_bind_group(0, &pool.group, &[]);
            pass.set_bind_group(1, &depth_group, &[]);
            if spawn > 0 {
                pass.set_pipeline(&kind.spawn);
                pass.dispatch_workgroups(spawn.div_ceil(64), 1, 1);
            }
            pass.set_pipeline(&kind.step);
            pass.dispatch_workgroups(pool.capacity.div_ceil(64), 1, 1);
            drop(pass);
            self.drawn.push((e.key, effect.to_string()));
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
        for (key, effect) in &self.drawn {
            if let (Some(pool), Some(kind)) = (self.pools.get(key), self.kinds.get(effect)) {
                pass.set_pipeline(&kind.draw);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plain_effect_and_one_of_every_part_build_into_the_particles_shader() {
        check_effect(&plain_effect()).unwrap_or_else(|e| panic!("{e}"));
        let graph = scrap_shadergraph::effect::parse(
            r#"(
                nodes: {
                    "ring": Random(low: (-1.0, 0.0, -1.0), high: (1.0, 0.0, 1.0)),
                    "at": Add(a: "origin", b: "ring"),
                    "long": Random(low: 1.0, high: 3.0),
                    "wind": Turbulence(at: "position", scale: 0.7),
                    "push": Multiply(a: "wind", b: "dt"),
                    "moved": Add(a: "velocity", b: "push"),
                    "hue": Random(low: (1.0, 0.3, 0.05), high: (1.0, 0.8, 0.3)),
                    "fade": OneMinus(of: "t"),
                    "flicker": Noise(at: "position", scale: 4.0),
                    "big": Multiply(a: "size", b: "flicker"),
                },
                spawn: (position: "at", velocity: "cone", life: "long"),
                update: (velocity: "moved"),
                output: (color: "hue", alpha: "fade", size: "big"),
            )"#,
        )
        .unwrap();
        let wgsl = scrap_shadergraph::effect::to_wgsl(&graph, "shaders/every.vfx.ron").unwrap();
        check_effect(&wgsl).unwrap_or_else(|e| panic!("{e}\n{wgsl}"));
        assert!(scrap_shadergraph::effect::problems(&graph).is_empty());
    }
}
