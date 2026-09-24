//! Clusters: a dense mesh cut into pieces of 124 triangles, each piece
//! culled on its own on the GPU — Nanite's and every meshlet renderer's
//! first half, done in a compute pass so it runs on any device.
//!
//! When a mesh of a few thousand triangles or more is uploaded, its
//! triangles are put in order along a Morton curve through its box, so
//! neighbours come together, and cut into runs of 124. Each run keeps a
//! sphere round it and the cone its faces point within.
//!
//! Each frame, before the prepass, a compute pass takes every instance of
//! such a mesh with every one of its clusters and keeps the pair when:
//! the sphere is inside the view; not every face in it is turned away from
//! the eye (the cone, as meshoptimizer tests it); and, when the occlusion
//! culling has last frame's depth to go by, it is not behind that. What is
//! kept goes onto a list, and the list is drawn in one indirect draw a
//! batch: an instance per cluster, 372 vertices each, the vertex shader
//! reading the mesh's own vertices and indices and the instance's numbers
//! out of storage buffers (vertex pulling). A cluster of fewer triangles
//! leaves the rest of its vertices degenerate.
//!
//! What it saves: the vertices of what is out of view, turned away or
//! hidden, a cluster at a time — a whole building behind a wall, the far
//! side of a rock. Shadows, skinned meshes, terrain, what is see-through
//! and materials with their own shaders are drawn as before.

use glam::Vec3;

use crate::asset::Vertex;
use crate::gpu::Gpu;
use crate::material::RenderFace;

/// Triangles in a cluster: what a mesh shader's group is usually given.
pub const TRIANGLES: u32 = 124;

/// Triangles a mesh needs before it is cut into clusters: below this the
/// whole mesh is one cluster's worth of work to draw anyway.
pub const FROM_TRIANGLES: usize = 2048;

/// A cluster as the GPU reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ClusterRaw {
    /// Centre and radius, in the mesh's own space.
    pub sphere: [f32; 4],
    /// The axis its faces point round and the cone's cutoff (1 when it has
    /// none: faces every way).
    pub cone: [f32; 4],
    /// Where its indices start, and how many triangles.
    pub first: u32,
    pub count: u32,
    pub pad: [u32; 2],
}

fn morton(p: Vec3) -> u32 {
    fn spread(v: u32) -> u32 {
        let mut x = v & 0x3ff;
        x = (x | (x << 16)) & 0x0300_00ff;
        x = (x | (x << 8)) & 0x0300_f00f;
        x = (x | (x << 4)) & 0x030c_30c3;
        x = (x | (x << 2)) & 0x0924_9249;
        x
    }
    let q = (p.clamp(Vec3::ZERO, Vec3::ONE) * 1023.0).as_uvec3();
    spread(q.x) | (spread(q.y) << 1) | (spread(q.z) << 2)
}

/// The mesh's indices in cluster order, and its clusters: `None` when it
/// is too small to be worth it.
///
/// Grown, as meshoptimizer's are: from the first triangle not yet taken in
/// Morton order through the mesh's box, a cluster takes in, of the
/// triangles touching it, the one nearest its middle, until it has 124 or
/// nothing touches it. Triangles touch where they share a corner's place,
/// so a seam of split vertices does not cut a cluster.
pub(crate) fn build(vertices: &[Vertex], indices: &[u32]) -> Option<(Vec<u32>, Vec<ClusterRaw>)> {
    let triangles = indices.len() / 3;
    if triangles < FROM_TRIANGLES || vertices.is_empty() {
        return None;
    }
    let at = |i: u32| Vec3::from_array(vertices[i as usize].position);
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for v in vertices {
        lo = lo.min(Vec3::from_array(v.position));
        hi = hi.max(Vec3::from_array(v.position));
    }
    let span = (hi - lo).max(Vec3::splat(1e-6));
    // Corners by where they are, to a millionth of the box.
    let mut places: std::collections::HashMap<(i32, i32, i32), u32> = std::collections::HashMap::new();
    let place: Vec<u32> = vertices
        .iter()
        .map(|v| {
            let q = ((Vec3::from_array(v.position) - lo) / span * 1.0e6).round().as_ivec3();
            let next = places.len() as u32;
            *places.entry((q.x, q.y, q.z)).or_insert(next)
        })
        .collect();
    let mut touching: Vec<Vec<u32>> = vec![Vec::new(); places.len()];
    let mut centres = Vec::with_capacity(triangles);
    for (t, tri) in indices.chunks_exact(3).enumerate() {
        for &i in tri {
            touching[place[i as usize] as usize].push(t as u32);
        }
        centres.push((at(tri[0]) + at(tri[1]) + at(tri[2])) / 3.0);
    }
    let mut seeds: Vec<(u32, u32)> = centres
        .iter()
        .enumerate()
        .map(|(t, c)| (morton((*c - lo) / span), t as u32))
        .collect();
    seeds.sort_unstable();
    let mut taken = vec![false; triangles];
    let mut sorted = Vec::with_capacity(indices.len());
    let mut clusters = Vec::new();
    let mut members: Vec<u32> = Vec::with_capacity(TRIANGLES as usize);
    let mut near: Vec<u32> = Vec::new();
    for &(_, seed) in &seeds {
        if taken[seed as usize] {
            continue;
        }
        members.clear();
        near.clear();
        let mut sum = Vec3::ZERO;
        let take = |t: u32, taken: &mut [bool], members: &mut Vec<u32>, near: &mut Vec<u32>, sum: &mut Vec3| {
            taken[t as usize] = true;
            members.push(t);
            *sum += centres[t as usize];
            for &i in &indices[t as usize * 3..t as usize * 3 + 3] {
                near.extend_from_slice(&touching[place[i as usize] as usize]);
            }
        };
        take(seed, &mut taken, &mut members, &mut near, &mut sum);
        while members.len() < TRIANGLES as usize {
            let middle = sum / members.len() as f32;
            near.retain(|&t| !taken[t as usize]);
            let Some(&best) = near.iter().min_by(|&&a, &&b| {
                centres[a as usize]
                    .distance_squared(middle)
                    .total_cmp(&centres[b as usize].distance_squared(middle))
            }) else {
                break;
            };
            take(best, &mut taken, &mut members, &mut near, &mut sum);
        }
        let first = sorted.len() as u32;
        for &t in &members {
            sorted.extend_from_slice(&indices[t as usize * 3..t as usize * 3 + 3]);
        }
        clusters.push(describe(&sorted[first as usize..], first, &at));
    }
    Some((sorted, clusters))
}

/// A cluster's sphere and cone, from its run of indices.
fn describe(run: &[u32], first: u32, at: &impl Fn(u32) -> Vec3) -> ClusterRaw {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut normals = Vec::with_capacity(run.len() / 3);
    for tri in run.chunks_exact(3) {
        let (a, b, d) = (at(tri[0]), at(tri[1]), at(tri[2]));
        for p in [a, b, d] {
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let n = (b - a).cross(d - a);
        if n.length_squared() > 1e-20 {
            normals.push(n.normalize());
        }
    }
    let centre = (lo + hi) * 0.5;
    let radius = run.iter().map(|&i| at(i).distance(centre)).fold(0.0f32, f32::max);
    let axis = normals.iter().copied().sum::<Vec3>().normalize_or_zero();
    let least = normals.iter().map(|n| n.dot(axis)).fold(1.0f32, f32::min);
    // Every face within 90° of the axis: a cone to test by. The cutoff is
    // the sine of how far they spread.
    let cutoff = if axis != Vec3::ZERO && least > 0.05 {
        (1.0 - least * least).max(0.0).sqrt()
    } else {
        1.0
    };
    ClusterRaw {
        sphere: [centre.x, centre.y, centre.z, radius],
        cone: [axis.x, axis.y, axis.z, cutoff],
        first,
        count: (run.len() / 3) as u32,
        pad: [0; 2],
    }
}

const CULL_SHADER: &str = r#"
struct Job {
    view_projection: mat4x4<f32>,
    // last frame's, for its depth
    hiz_view_projection: mat4x4<f32>,
    // the eye; 1 in w when last frame's depth is there to test against
    eye: vec4<f32>,
    // the pyramid's size and levels
    hiz_size: vec4<f32>,
    // first instance, instances, clusters, where the kept go
    counts: vec4<u32>,
    // which slot of arguments; 0 front faces, 1 back, 2 both
    slot: vec4<u32>,
};

struct Cluster {
    sphere: vec4<f32>,
    cone: vec4<f32>,
    first: u32,
    count: u32,
    pad_a: u32,
    pad_b: u32,
};

struct Drawn {
    instance: u32,
    first: u32,
    count: u32,
    pad: u32,
};

@group(0) @binding(0) var<uniform> job: Job;
@group(0) @binding(1) var<storage, read> clusters: array<Cluster>;
@group(0) @binding(2) var<storage, read> instances: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> args: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> drawn: array<Drawn>;
@group(0) @binding(5) var hiz: texture_2d<f32>;

fn in_view(c: vec3<f32>, r: f32) -> bool {
    let m = transpose(job.view_projection);
    let planes = array<vec4<f32>, 6>(m[3] + m[0], m[3] - m[0], m[3] + m[1], m[3] - m[1], m[2], m[3] - m[2]);
    for (var i = 0; i < 6; i = i + 1) {
        let p = planes[i];
        if dot(p.xyz, c) + p.w < -r * length(p.xyz) {
            return false;
        }
    }
    return true;
}

fn not_hidden(c: vec3<f32>, r: f32) -> bool {
    if job.eye.w < 0.5 {
        return true;
    }
    var lo = vec3<f32>(1e9);
    var hi = vec3<f32>(-1e9);
    for (var i = 0u; i < 8u; i = i + 1u) {
        let corner = c + r * vec3<f32>(
            select(-1.0, 1.0, (i & 1u) != 0u),
            select(-1.0, 1.0, (i & 2u) != 0u),
            select(-1.0, 1.0, (i & 4u) != 0u),
        );
        let clip = job.hiz_view_projection * vec4<f32>(corner, 1.0);
        if clip.w <= 1e-4 {
            return true;
        }
        let ndc = clip.xyz / clip.w;
        lo = min(lo, ndc);
        hi = max(hi, ndc);
    }
    if hi.x < -1.0 || lo.x > 1.0 || hi.y < -1.0 || lo.y > 1.0 {
        return true;
    }
    let size = job.hiz_size.xy;
    let uv_lo = clamp(vec2<f32>(lo.x * 0.5 + 0.5, 0.5 - hi.y * 0.5), vec2<f32>(0.0), vec2<f32>(1.0));
    let uv_hi = clamp(vec2<f32>(hi.x * 0.5 + 0.5, 0.5 - lo.y * 0.5), vec2<f32>(0.0), vec2<f32>(1.0));
    let span = (uv_hi - uv_lo) * size;
    let level = clamp(ceil(log2(max(max(span.x, span.y), 1.0) * 0.5)), 0.0, job.hiz_size.z - 1.0);
    let dims = vec2<f32>(textureDimensions(hiz, i32(level)));
    let a = vec2<i32>(clamp(uv_lo * dims, vec2<f32>(0.0), dims - 1.0));
    let b = vec2<i32>(clamp(uv_hi * dims, vec2<f32>(0.0), dims - 1.0));
    var far = 0.0;
    for (var y = a.y; y <= b.y; y = y + 1) {
        for (var x = a.x; x <= b.x; x = x + 1) {
            far = max(far, textureLoad(hiz, vec2<i32>(x, y), i32(level)).r);
        }
    }
    return lo.z <= far + 1e-5;
}

@compute @workgroup_size(64)
fn cs_clusters(@builtin(global_invocation_id) id: vec3<u32>) {
    let per = job.counts.z;
    let i = id.x;
    if i >= job.counts.y * per {
        return;
    }
    let instance = job.counts.x + i / per;
    let cluster = clusters[i % per];
    let s = instance * 12u;
    let model = mat4x4<f32>(instances[s], instances[s + 1u], instances[s + 2u], instances[s + 3u]);
    let scale = max(length(model[0].xyz), max(length(model[1].xyz), length(model[2].xyz)));
    let c = (model * vec4<f32>(cluster.sphere.xyz, 1.0)).xyz;
    let r = cluster.sphere.w * scale;
    if !in_view(c, r) {
        return;
    }
    // Every face turned away: the cone, conservatively for the sphere.
    let face = job.slot.y;
    if face != 2u && cluster.cone.w < 1.0 {
        var axis = normalize((model * vec4<f32>(cluster.cone.xyz, 0.0)).xyz);
        if face == 1u {
            axis = -axis;
        }
        let to = c - job.eye.xyz;
        if dot(to, axis) >= cluster.cone.w * length(to) + r {
            return;
        }
    }
    if !not_hidden(c, r) {
        return;
    }
    let at = atomicAdd(&args[job.slot.x * 4u + 1u], 1u);
    drawn[job.counts.w + at] = Drawn(instance, cluster.first, cluster.count, 0u);
}
"#;

/// A clustered mesh's clusters on the GPU.
pub(crate) struct MeshClusters {
    pub(crate) buffer: wgpu::Buffer,
    pub(crate) count: u32,
}

impl MeshClusters {
    pub(crate) fn new(gpu: &Gpu, clusters: &[ClusterRaw]) -> Self {
        use wgpu::util::DeviceExt;
        Self {
            buffer: gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("clusters"),
                contents: bytemuck::cast_slice(clusters),
                usage: wgpu::BufferUsages::STORAGE,
            }),
            count: clusters.len() as u32,
        }
    }
}

/// A batch whose clusters are culled this frame.
pub(crate) struct Job<'a> {
    /// Which of the frame's batches.
    pub(crate) batch: usize,
    pub(crate) first_instance: u32,
    pub(crate) instances: u32,
    pub(crate) face: RenderFace,
    pub(crate) clusters: &'a MeshClusters,
    pub(crate) vertices: &'a wgpu::Buffer,
    pub(crate) indices: &'a wgpu::Buffer,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct JobUniform {
    view_projection: [[f32; 4]; 4],
    hiz_view_projection: [[f32; 4]; 4],
    eye: [f32; 4],
    hiz_size: [f32; 4],
    counts: [u32; 4],
    slot: [u32; 4],
    // To a uniform buffer offset's alignment.
    pad: [u32; 16],
}

/// Last frame's depth, for the occlusion test: the pyramid, the view it
/// was drawn with, its size and levels.
pub(crate) struct Hiz<'a> {
    pub(crate) view: &'a wgpu::TextureView,
    pub(crate) view_projection: glam::Mat4,
    pub(crate) size: (u32, u32),
    pub(crate) levels: u32,
}

/// The pipelines a cluster is drawn with: the scene's (by face, water or
/// not) and the prepass's (by face).
pub(crate) struct ClusterPipelines {
    pub(crate) scene: std::collections::HashMap<(RenderFace, bool), wgpu::RenderPipeline>,
    pub(crate) prepass: std::collections::HashMap<RenderFace, wgpu::RenderPipeline>,
}

pub(crate) struct Clusters {
    /// The device can: indirect draws past their first instance, storage
    /// buffers in the vertex stage.
    pub(crate) can: bool,
    pub(crate) enabled: bool,
    cull_layout: wgpu::BindGroupLayout,
    cull: wgpu::ComputePipeline,
    /// Group 3 of the drawing pipelines: vertices, indices, instances, the
    /// clusters kept.
    pub(crate) draw_layout: wgpu::BindGroupLayout,
    pub(crate) pipeline_layout: wgpu::PipelineLayout,
    pub(crate) pipelines: Option<ClusterPipelines>,
    jobs: wgpu::Buffer,
    job_capacity: u64,
    args: wgpu::Buffer,
    args_capacity: u64,
    drawn: wgpu::Buffer,
    drawn_capacity: u64,
    blank: wgpu::TextureView,
    /// This frame's: for each batch drawn by clusters, its slot of
    /// arguments and its group 3.
    pub(crate) this_frame: std::collections::HashMap<usize, (u32, wgpu::BindGroup)>,
    /// Slots written this frame, for reading back.
    slots: u32,
}

const JOB_STRIDE: u64 = std::mem::size_of::<JobUniform>() as u64;

fn storage(gpu: &Gpu, label: &str, size: u64, extra: wgpu::BufferUsages) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(16),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | extra,
        mapped_at_creation: false,
    })
}

impl Clusters {
    pub(crate) fn new(gpu: &Gpu, frame_layout: &wgpu::BindGroupLayout, maps_layout: &wgpu::BindGroupLayout) -> Self {
        let buffer = |binding, read_only, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let compute = wgpu::ShaderStages::COMPUTE;
        let cull_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("clusters cull"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: compute,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(JOB_STRIDE),
                    },
                    count: None,
                },
                buffer(1, true, compute),
                buffer(2, true, compute),
                buffer(3, false, compute),
                buffer(4, false, compute),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: compute,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("runity::clusters"),
            source: wgpu::ShaderSource::Wgsl(CULL_SHADER.into()),
        });
        let cull = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("runity::clusters"),
            layout: Some(&gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("runity::clusters"),
                bind_group_layouts: &[Some(&cull_layout)],
                immediate_size: 0,
            })),
            module: &module,
            entry_point: Some("cs_clusters"),
            compilation_options: Default::default(),
            cache: None,
        });
        let vertex = wgpu::ShaderStages::VERTEX;
        let draw_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("clusters draw"),
            entries: &[
                buffer(3, true, vertex),
                buffer(4, true, vertex),
                buffer(5, true, vertex),
                buffer(6, true, vertex),
            ],
        });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("runity::clusters draw"),
            bind_group_layouts: &[Some(frame_layout), Some(maps_layout), None, Some(&draw_layout)],
            immediate_size: 0,
        });
        let blank = gpu
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("clusters no depth"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let can = gpu.device.features().contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
            && gpu
                .adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::VERTEX_STORAGE)
            && gpu.device.limits().max_storage_buffers_per_shader_stage >= 8;
        Self {
            can,
            enabled: can && std::env::var_os("RUNITY_NO_CLUSTERS").is_none(),
            cull_layout,
            cull,
            draw_layout,
            pipeline_layout,
            pipelines: None,
            jobs: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cluster jobs"),
                size: JOB_STRIDE * 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            job_capacity: 16,
            args: storage(gpu, "cluster args", 16 * 16, wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC),
            args_capacity: 16,
            drawn: storage(gpu, "clusters kept", 1024 * 16, wgpu::BufferUsages::empty()),
            drawn_capacity: 1024,
            blank,
            this_frame: std::collections::HashMap::new(),
            slots: 0,
        }
    }

    /// The drawing pipelines, from the renderer's shader module: at start
    /// and when it is reloaded.
    pub(crate) fn make_pipelines(&self, gpu: &Gpu, shader: &wgpu::ShaderModule, samples: u32) -> Option<ClusterPipelines> {
        if !self.can {
            return None;
        }
        let pipeline = |face: RenderFace, fragment: &str, format: wgpu::TextureFormat, depth: wgpu::TextureFormat, samples: u32| {
            gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("runity::clusters"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_cluster"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: match face {
                        RenderFace::Front => Some(wgpu::Face::Back),
                        RenderFace::Back => Some(wgpu::Face::Front),
                        RenderFace::Both => None,
                    },
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };
        let faces = [RenderFace::Front, RenderFace::Back, RenderFace::Both];
        let mut scene = std::collections::HashMap::new();
        let mut prepass = std::collections::HashMap::new();
        for face in faces {
            for water in [false, true] {
                let fragment = if water { "fs_water" } else { "fs" };
                scene.insert(
                    (face, water),
                    pipeline(face, fragment, crate::post::HDR_FORMAT, crate::render::DEPTH_FORMAT, samples),
                );
            }
            prepass.insert(
                face,
                pipeline(face, "fs_normals", crate::ssao::NORMAL_FORMAT, crate::ssao::PREPASS_DEPTH, 1),
            );
        }
        Some(ClusterPipelines { scene, prepass })
    }

    /// Cull this frame's clustered batches: what is kept is on the list,
    /// each batch's slot of arguments counting it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn cull(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        jobs: &[Job],
        instances: &wgpu::Buffer,
        view_projection: glam::Mat4,
        eye: Vec3,
        hiz: Option<Hiz>,
    ) {
        self.this_frame.clear();
        self.slots = 0;
        if !self.enabled || jobs.is_empty() {
            return;
        }
        let count = jobs.len() as u64;
        if count > self.job_capacity {
            self.job_capacity = count.next_power_of_two();
            self.jobs = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cluster jobs"),
                size: JOB_STRIDE * self.job_capacity,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if count > self.args_capacity {
            self.args_capacity = count.next_power_of_two();
            self.args = storage(
                gpu,
                "cluster args",
                self.args_capacity * 16,
                wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
            );
        }
        let pairs: u64 = jobs.iter().map(|j| j.instances as u64 * j.clusters.count as u64).sum();
        if pairs > self.drawn_capacity {
            self.drawn_capacity = pairs.next_power_of_two();
            self.drawn = storage(gpu, "clusters kept", self.drawn_capacity * 16, wgpu::BufferUsages::empty());
        }
        let (hiz_view, hiz_vp, hiz_size, on) = match &hiz {
            Some(h) => (h.view, h.view_projection, [h.size.0 as f32, h.size.1 as f32, h.levels as f32, 0.0], 1.0),
            None => (&self.blank, glam::Mat4::IDENTITY, [1.0, 1.0, 1.0, 0.0], 0.0),
        };
        let mut uniforms = Vec::with_capacity(jobs.len());
        let mut args = Vec::with_capacity(jobs.len() * 4);
        let mut out = 0u32;
        for (slot, job) in jobs.iter().enumerate() {
            uniforms.push(JobUniform {
                view_projection: view_projection.to_cols_array_2d(),
                hiz_view_projection: hiz_vp.to_cols_array_2d(),
                eye: [eye.x, eye.y, eye.z, on],
                hiz_size,
                counts: [job.first_instance, job.instances, job.clusters.count, out],
                slot: [
                    slot as u32,
                    match job.face {
                        RenderFace::Front => 0,
                        RenderFace::Back => 1,
                        RenderFace::Both => 2,
                    },
                    0,
                    0,
                ],
                pad: [0; 16],
            });
            // Each kept cluster an instance of 372 vertices, starting at
            // this batch's part of the list.
            args.extend_from_slice(&[TRIANGLES * 3, 0, 0, out]);
            out += job.instances * job.clusters.count;
        }
        gpu.queue.write_buffer(&self.jobs, 0, bytemuck::cast_slice(&uniforms));
        gpu.queue.write_buffer(&self.args, 0, bytemuck::cast_slice(&args));
        let groups: Vec<wgpu::BindGroup> = jobs
            .iter()
            .map(|job| {
                gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("clusters cull"),
                    layout: &self.cull_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &self.jobs,
                                offset: 0,
                                size: wgpu::BufferSize::new(JOB_STRIDE),
                            }),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: job.clusters.buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: instances.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: self.args.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: self.drawn.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::TextureView(hiz_view),
                        },
                    ],
                })
            })
            .collect();
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("runity::clusters"),
                timestamp_writes: crate::gpu_timer::compute("clusters"),
            });
            pass.set_pipeline(&self.cull);
            for (slot, (job, group)) in jobs.iter().zip(&groups).enumerate() {
                let n = job.instances * job.clusters.count;
                pass.set_bind_group(0, group, &[(slot as u64 * JOB_STRIDE) as u32]);
                pass.dispatch_workgroups(n.div_ceil(64), 1, 1);
            }
        }
        for (slot, job) in jobs.iter().enumerate() {
            let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("clusters draw"),
                layout: &self.draw_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: job.vertices.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: job.indices.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: instances.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: self.drawn.as_entire_binding(),
                    },
                ],
            });
            self.this_frame.insert(job.batch, (slot as u32, group));
        }
        self.slots = jobs.len() as u32;
    }

    /// Draw batch `batch` by its clusters, when it was culled so.
    pub(crate) fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>, batch: usize) -> bool {
        let Some((slot, group)) = self.this_frame.get(&batch) else {
            return false;
        };
        pass.set_bind_group(3, group, &[]);
        pass.draw_indirect(&self.args, *slot as u64 * 16);
        true
    }

    /// How many clusters the last culling kept, read back — waiting on the
    /// GPU, so for tests and tools.
    pub(crate) fn kept(&self, gpu: &Gpu) -> Option<u32> {
        if self.slots == 0 {
            return None;
        }
        let size = self.slots as u64 * 16;
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("clusters kept read"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&self.args, 0, &read, 0, size);
        gpu.queue.submit(Some(encoder.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;
        let view = read.slice(..).get_mapped_range().ok()?;
        let args: &[u32] = bytemuck::cast_slice(&view);
        Some(args.chunks(4).map(|a| a[1]).sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dense_sphere_is_cut_into_whole_compact_clusters_with_cones() {
        let sphere = crate::builtin::sphere(1.0, 96, 48);
        let (indices, clusters) = build(&sphere.vertices, &sphere.indices).expect("dense enough");
        let triangles = sphere.indices.len() / 3;
        assert_eq!(indices.len(), sphere.indices.len(), "every triangle kept");
        assert_eq!(clusters.iter().map(|c| c.count as usize).sum::<usize>(), triangles);
        assert!(clusters.iter().all(|c| c.count <= TRIANGLES));
        // The same triangles, in another order.
        let key = |i: &[u32]| {
            let mut t: Vec<[u32; 3]> = i.chunks_exact(3).map(|t| {
                let mut k = [t[0], t[1], t[2]];
                k.sort_unstable();
                k
            }).collect();
            t.sort_unstable();
            t
        };
        assert_eq!(key(&indices), key(&sphere.indices));
        // Small: a sphere of radius one cut in a few hundred is pieces a
        // fraction of it across, and most face one way.
        let mean_radius = clusters.iter().map(|c| c.sphere[3]).sum::<f32>() / clusters.len() as f32;
        assert!(mean_radius < 0.3, "compact: {mean_radius}");
        let coned = clusters.iter().filter(|c| c.cone[3] < 0.9).count();
        assert!(coned * 10 > clusters.len() * 8, "{coned} of {} have a cone", clusters.len());
        // Too few triangles: not cut.
        let small = crate::builtin::sphere(1.0, 16, 8);
        assert!(build(&small.vertices, &small.indices).is_none());
    }
}
