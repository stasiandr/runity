//! Occlusion culling on the GPU, and the draws that follow from it: the
//! start of drawing driven by the GPU.
//!
//! After the prepass, its depth is folded into a pyramid (Hi-Z): each level
//! half the last, each texel the farthest depth of the four under it. Next
//! frame, before anything is drawn, a compute pass takes every instance of
//! the colour pass's batches and projects its box with the view that
//! pyramid was made from: if even its nearest point lies behind the
//! farthest depth over the part of the screen it covers, something stood in
//! front of it and it is left out. What is kept is packed, batch by batch,
//! into an instance buffer of its own, and each batch's count goes into the
//! arguments of an indirect draw — the prepass and the colour pass draw
//! those, not what the CPU listed.
//!
//! One frame behind, as such culling is: what the camera turns to see was
//! not in last frame's depth, and is drawn a frame late — so after a cut
//! (the camera jumping) nothing is culled. What moves by its own vertex
//! shader (the terrain's grid) is never culled: its box is not where it is
//! drawn. Shadows are not culled this way; a caster out of sight still
//! throws its shadow.

use glam::{Mat4, Vec3};

use crate::gpu::Gpu;

/// The pyramid's format: one far depth a texel.
const HIZ_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

const HIZ_SHADER: &str = r#"
@group(0) @binding(0) var source_depth: texture_depth_2d;
@group(0) @binding(1) var source_level: texture_2d<f32>;
@group(0) @binding(2) var level_out: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8)
fn cs_first(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(level_out);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    // Half the depth's size: the farthest of the 2x2 under it, and the
    // odd row or column at the edge.
    let above = vec2<i32>(textureDimensions(source_depth));
    let base = vec2<i32>(id.xy) * 2;
    var far = 0.0;
    for (var y = 0; y < 3; y = y + 1) {
        for (var x = 0; x < 3; x = x + 1) {
            if (x == 2 && above.x - base.x != 3) || (y == 2 && above.y - base.y != 3) {
                continue;
            }
            let at = min(base + vec2<i32>(x, y), above - 1);
            far = max(far, textureLoad(source_depth, at, 0));
        }
    }
    textureStore(level_out, id.xy, vec4<f32>(far, 0.0, 0.0, 0.0));
}

// The farthest of what lies under: the 2x2, and the odd row or column
// when the level above has one, so nothing is lost.
@compute @workgroup_size(8, 8)
fn cs_down(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(level_out);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let above = vec2<i32>(textureDimensions(source_level));
    let base = vec2<i32>(id.xy) * 2;
    var far = 0.0;
    for (var y = 0; y < 3; y = y + 1) {
        for (var x = 0; x < 3; x = x + 1) {
            if (x == 2 && above.x - base.x != 3) || (y == 2 && above.y - base.y != 3) {
                continue;
            }
            let at = min(base + vec2<i32>(x, y), above - 1);
            far = max(far, textureLoad(source_level, at, 0).r);
        }
    }
    textureStore(level_out, id.xy, vec4<f32>(far, 0.0, 0.0, 0.0));
}
"#;

const CULL_SHADER: &str = r#"
struct Cull {
    // the view last frame's depth was drawn with
    view_projection: mat4x4<f32>,
    // the pyramid's size and levels; the instances; where the colour
    // pass's instances start in the frame's buffer
    size: vec4<f32>,
    counts: vec4<u32>,
};

struct Box {
    // its world box; w of `max` is 1 for what is never culled
    min: vec4<f32>,
    max: vec4<f32>,
};

// An instance, as the vertex buffer has it: thirteen vec4.
const STRIDE: u32 = 13u;

@group(0) @binding(0) var<uniform> cull: Cull;
@group(0) @binding(1) var<storage, read> boxes: array<Box>;
@group(0) @binding(2) var<storage, read> batch_of: array<u32>;
@group(0) @binding(3) var<storage, read_write> args: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read> source: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read_write> kept: array<vec4<f32>>;
@group(0) @binding(6) var hiz: texture_2d<f32>;

fn seen(b: Box) -> bool {
    if b.max.w > 0.5 {
        return true;
    }
    var lo = vec3<f32>(1e9);
    var hi = vec3<f32>(-1e9);
    for (var i = 0u; i < 8u; i = i + 1u) {
        let corner = vec3<f32>(
            select(b.min.x, b.max.x, (i & 1u) != 0u),
            select(b.min.y, b.max.y, (i & 2u) != 0u),
            select(b.min.z, b.max.z, (i & 4u) != 0u),
        );
        let clip = cull.view_projection * vec4<f32>(corner, 1.0);
        // Reaching behind the eye: drawn, not guessed at.
        if clip.w <= 1e-4 {
            return true;
        }
        let ndc = clip.xyz / clip.w;
        lo = min(lo, ndc);
        hi = max(hi, ndc);
    }
    // Off the screen then: the frustum is the CPU's to judge.
    if hi.x < -1.0 || lo.x > 1.0 || hi.y < -1.0 || lo.y > 1.0 {
        return true;
    }
    let size = cull.size.xy;
    let uv_lo = clamp(vec2<f32>(lo.x * 0.5 + 0.5, 0.5 - hi.y * 0.5), vec2<f32>(0.0), vec2<f32>(1.0));
    let uv_hi = clamp(vec2<f32>(hi.x * 0.5 + 0.5, 0.5 - lo.y * 0.5), vec2<f32>(0.0), vec2<f32>(1.0));
    let span = (uv_hi - uv_lo) * size;
    // The level where it covers two texels or fewer across.
    let level = clamp(ceil(log2(max(max(span.x, span.y), 1.0) * 0.5)), 0.0, cull.size.z - 1.0);
    let dims = vec2<f32>(textureDimensions(hiz, i32(level)));
    let a = vec2<i32>(clamp(uv_lo * dims, vec2<f32>(0.0), dims - 1.0));
    let c = vec2<i32>(clamp(uv_hi * dims, vec2<f32>(0.0), dims - 1.0));
    var far = 0.0;
    for (var y = a.y; y <= c.y; y = y + 1) {
        for (var x = a.x; x <= c.x; x = x + 1) {
            far = max(far, textureLoad(hiz, vec2<i32>(x, y), i32(level)).r);
        }
    }
    return lo.z <= far + 1e-5;
}

@compute @workgroup_size(64)
fn cs_cull(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= cull.counts.x {
        return;
    }
    if !seen(boxes[i]) {
        return;
    }
    let batch = batch_of[i];
    let slot = atomicAdd(&args[batch * 5u + 1u], 1u);
    let first = atomicLoad(&args[batch * 5u + 4u]);
    let src = (cull.counts.y + i) * STRIDE;
    let dst = (first + slot) * STRIDE;
    for (var k = 0u; k < STRIDE; k = k + 1u) {
        kept[dst + k] = source[src + k];
    }
}
"#;

/// A batch of the colour pass, as the culling sees it.
pub(crate) struct CullBatch {
    pub index_count: u32,
    pub count: u32,
}

/// The pyramid, its size, a view of each level to write, and one of all
/// of them to read.
type Pyramid = (wgpu::Texture, (u32, u32), Vec<wgpu::TextureView>, wgpu::TextureView);

pub(crate) struct Occlusion {
    hiz_first: wgpu::ComputePipeline,
    hiz_down: wgpu::ComputePipeline,
    hiz_layout: wgpu::BindGroupLayout,
    cull: wgpu::ComputePipeline,
    cull_layout: wgpu::BindGroupLayout,
    /// The pyramid, its size, and a view of each level.
    pyramid: Option<Pyramid>,
    blank: wgpu::TextureView,
    /// The view the pyramid was drawn with, when it holds a frame.
    made_with: Option<Mat4>,
    /// Where the camera was, to tell a cut.
    last_eye: Option<(Vec3, Vec3)>,
    uniform: wgpu::Buffer,
    boxes: Buffer,
    batch_of: Buffer,
    pub(crate) args: Buffer,
    pub(crate) kept: Buffer,
    /// Culling is on this frame: the colour pass and the prepass draw by
    /// the arguments.
    pub(crate) active: bool,
    pub(crate) enabled: bool,
    /// The device can: its indirect draws start past their first instance.
    pub(crate) can: bool,
    /// The batches of the last frame culled.
    batches: u32,
}

/// A buffer that grows.
pub(crate) struct Buffer {
    pub(crate) buffer: wgpu::Buffer,
    size: u64,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl Buffer {
    fn new(gpu: &Gpu, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        Self {
            buffer: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 256,
                usage,
                mapped_at_creation: false,
            }),
            size: 256,
            usage,
            label,
        }
    }

    /// At least `size` bytes: `true` when it was made again.
    fn fit(&mut self, gpu: &Gpu, size: u64) -> bool {
        if size <= self.size {
            return false;
        }
        self.size = size.next_power_of_two();
        self.buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(self.label),
            size: self.size,
            usage: self.usage,
            mapped_at_creation: false,
        });
        true
    }
}

impl Occlusion {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        let hiz_module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scrap::hi-z"),
            source: wgpu::ShaderSource::Wgsl(HIZ_SHADER.into()),
        });
        let hiz_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hi-z"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: HIZ_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let hiz_pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hi-z"),
            bind_group_layouts: &[Some(&hiz_layout)],
            immediate_size: 0,
        });
        let compute = |module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, entry: &str| {
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("scrap::occlusion"),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let cull_module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scrap::occlusion cull"),
            source: wgpu::ShaderSource::Wgsl(CULL_SHADER.into()),
        });
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let cull_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("occlusion cull"),
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
                storage(1, true),
                storage(2, true),
                storage(3, false),
                storage(4, true),
                storage(5, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let cull_pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("occlusion cull"),
            bind_group_layouts: &[Some(&cull_layout)],
            immediate_size: 0,
        });
        let blank = gpu
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("no hi-z"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HIZ_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let storage_usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        Self {
            hiz_first: compute(&hiz_module, &hiz_pipeline_layout, "cs_first"),
            hiz_down: compute(&hiz_module, &hiz_pipeline_layout, "cs_down"),
            hiz_layout,
            cull: compute(&cull_module, &cull_pipeline_layout, "cs_cull"),
            cull_layout,
            pyramid: None,
            blank,
            made_with: None,
            last_eye: None,
            uniform: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("occlusion cull"),
                size: 96,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            boxes: Buffer::new(gpu, "occlusion boxes", storage_usage),
            batch_of: Buffer::new(gpu, "occlusion batches", storage_usage),
            args: Buffer::new(
                gpu,
                "occlusion draws",
                storage_usage | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
            ),
            kept: Buffer::new(gpu, "occlusion kept", wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX),
            active: false,
            // Its draws start past their first instance: without that, not at all.
            enabled: gpu.device.features().contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
            can: gpu.device.features().contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
            batches: 0,
        }
    }

    /// Forget last frame's depth: the next frame culls nothing.
    pub(crate) fn forget(&mut self) {
        self.made_with = None;
    }

    /// Cull this frame's colour-pass instances against last frame's depth,
    /// into `encoder`. `boxes` are their world boxes, in the order of the
    /// batches; `base` is where they start in `instances`. Sets `active`
    /// when the draws are to be made from the arguments.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn cull(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        batches: &[CullBatch],
        boxes: &[[f32; 8]],
        instances: &wgpu::Buffer,
        base: u32,
        eye: Vec3,
        forward: Vec3,
    ) {
        self.active = false;
        // A cut: the camera jumped, and last frame's depth says nothing.
        let cut = self
            .last_eye
            .is_none_or(|(e, f)| e.distance(eye) > 3.0 || f.dot(forward) < 0.7);
        self.last_eye = Some((eye, forward));
        let (Some(made_with), Some((_, size, _, whole))) = (self.made_with, self.pyramid.as_ref()) else {
            return;
        };
        if !self.enabled || cut || boxes.is_empty() {
            return;
        }
        let count = boxes.len() as u32;
        let mut args = Vec::with_capacity(batches.len() * 5);
        let mut batch_of = Vec::with_capacity(boxes.len());
        let mut first = 0u32;
        for (b, batch) in batches.iter().enumerate() {
            args.extend_from_slice(&[batch.index_count, 0, 0, 0, first]);
            batch_of.extend(std::iter::repeat_n(b as u32, batch.count as usize));
            first += batch.count;
        }
        let stride = 208u64;
        let mut remade = self.boxes.fit(gpu, boxes.len() as u64 * 32);
        remade |= self.batch_of.fit(gpu, batch_of.len() as u64 * 4);
        remade |= self.args.fit(gpu, args.len() as u64 * 4);
        remade |= self.kept.fit(gpu, count as u64 * stride);
        let _ = remade;
        gpu.queue.write_buffer(&self.boxes.buffer, 0, bytemuck::cast_slice(boxes));
        gpu.queue.write_buffer(&self.batch_of.buffer, 0, bytemuck::cast_slice(&batch_of));
        gpu.queue.write_buffer(&self.args.buffer, 0, bytemuck::cast_slice(&args));
        let levels = self.pyramid.as_ref().map_or(1, |p| p.2.len()) as f32;
        let mut uniform = Vec::with_capacity(24);
        uniform.extend_from_slice(&made_with.to_cols_array());
        uniform.extend_from_slice(&[size.0 as f32, size.1 as f32, levels, 0.0]);
        let mut bytes: Vec<u8> = bytemuck::cast_slice(&uniform).to_vec();
        bytes.extend_from_slice(bytemuck::cast_slice(&[count, base, 0u32, 0u32]));
        gpu.queue.write_buffer(&self.uniform, 0, &bytes);
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occlusion cull"),
            layout: &self.cull_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.boxes.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.batch_of.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.args.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: instances.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.kept.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(whole),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("scrap::occlusion cull"),
            timestamp_writes: crate::gpu_timer::compute("occlusion"),
        });
        pass.set_pipeline(&self.cull);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        drop(pass);
        self.active = true;
        self.batches = batches.len() as u32;
    }

    /// How many instances the last culling kept, read back — waiting on the
    /// GPU, so for tests and tools; `None` when it did not cull.
    pub(crate) fn kept_count(&self, gpu: &Gpu) -> Option<u32> {
        if !self.active {
            return None;
        }
        let size = self.batches as u64 * 20;
        if size == 0 {
            return Some(0);
        }
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occlusion read"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("occlusion read"),
        });
        encoder.copy_buffer_to_buffer(&self.args.buffer, 0, &read, 0, size);
        gpu.queue.submit(Some(encoder.finish()));
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let args: Vec<u32> = match read.slice(..).get_mapped_range() {
            Ok(view) => bytemuck::cast_slice::<u8, u32>(&view).to_vec(),
            Err(_) => return None,
        };
        Some(args.chunks(5).map(|a| a[1]).sum())
    }

    /// Last frame's depth to test against, when this frame was culled by it
    /// (on, and not a cut).
    pub(crate) fn hiz(&self) -> Option<crate::cluster::Hiz<'_>> {
        let (Some(made_with), Some((_, size, views, whole)), true) = (self.made_with, self.pyramid.as_ref(), self.active) else {
            return None;
        };
        Some(crate::cluster::Hiz {
            view: whole,
            view_projection: made_with,
            size: *size,
            levels: views.len() as u32,
        })
    }

    /// Fold this frame's prepass depth into the pyramid, for the next frame.
    pub(crate) fn build(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        size: (u32, u32),
        view_projection: Mat4,
    ) {
        if !self.enabled {
            self.made_with = None;
            return;
        }
        // Its finest level is half the depth's size: a whole screen of
        // texels written and read back is most of what the pyramid costs,
        // and a box a texel or two across tells nothing at a finer one.
        let size = (size.0.div_ceil(2).max(1), size.1.div_ceil(2).max(1));
        if self.pyramid.as_ref().is_none_or(|p| p.1 != size) {
            let levels = 32 - size.0.max(size.1).max(1).leading_zeros();
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hi-z"),
                size: wgpu::Extent3d {
                    width: size.0.max(1),
                    height: size.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HIZ_FORMAT,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let views: Vec<wgpu::TextureView> = (0..levels)
                .map(|level| {
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        base_mip_level: level,
                        mip_level_count: Some(1),
                        ..Default::default()
                    })
                })
                .collect();
            let whole = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.pyramid = Some((texture, size, views, whole));
        }
        let (_, _, views, _) = self.pyramid.as_ref().expect("made above");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("scrap::hi-z"),
            timestamp_writes: crate::gpu_timer::compute("hi-z"),
        });
        for (level, target) in views.iter().enumerate() {
            let source_level = if level == 0 { &self.blank } else { &views[level - 1] };
            let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hi-z"),
                layout: &self.hiz_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source_level),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(target),
                    },
                ],
            });
            let w = (size.0 >> level).max(1);
            let h = (size.1 >> level).max(1);
            pass.set_pipeline(if level == 0 { &self.hiz_first } else { &self.hiz_down });
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(pass);
        self.made_with = Some(view_projection);
    }
}
