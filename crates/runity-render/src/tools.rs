//! What goes over the finished picture: handles and outlines.
//!
//! Three passes, after post-processing, at the picture's own size:
//!
//! * **the outline mask** — what [`crate::render::Frame::outline_draws`]
//!   covers, each pixel the number of its colour and whether it is seen
//!   there (the prepass's depth says what is in front);
//! * **the overlay** — [`crate::render::Frame::overlay_draws`], into a
//!   picture of its own, cleared to nothing and multisampled four times:
//!   a handle's edge is smooth whatever the scene's antialiasing is, and
//!   translucent parts (a plane handle, a swept angle) blend in it;
//! * **the composite** — one triangle over the screen that finds the
//!   mask's edges, draws them in their colours, and lays the overlay over
//!   that.
//!
//! The edge is found on the screen rather than drawn as lines round a box,
//! so what is outlined is the shape itself, as in Unity. The pass looks a
//! few pixels round each pixel; a pixel with nothing near it gives up after
//! nine taps, so a frame where the selection is small pays for little more
//! than the one read.

use crate::gpu::Gpu;

/// Colours an outline can be at once: the selection's, its children's,
/// and two to spare.
pub const OUTLINE_COLORS: usize = 4;

/// The mask: red is the colour's number out of 255 (0 nothing), green 1
/// where it is seen.
pub(crate) const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub(crate) const MASK_DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// How strong an outline is where something is in front of it.
const HIDDEN: f32 = 0.3;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    colors: [[f32; 4]; OUTLINE_COLORS],
    /// Width in pixels, 1 when there is an outline, how strong a hidden
    /// one is, unused.
    shape: [f32; 4],
}

const SHADER: &str = r#"
struct Params {
    colors: array<vec4<f32>, 4>,
    shape: vec4<f32>,
};
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var mask: texture_2d<f32>;
@group(0) @binding(2) var overlay: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn colour_at(m: vec4<f32>) -> i32 {
    return i32(round(m.r * 255.0));
}

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(at.xy);
    var line = vec4<f32>(0.0);
    if params.shape.y > 0.5 {
        let size = vec2<i32>(textureDimensions(mask)) - vec2<i32>(1);
        let mine = colour_at(textureLoad(mask, p, 0));
        let width = params.shape.x;
        let reach = i32(ceil(width));
        // Nothing within reach at the corners and the middle of the edges:
        // no silhouette is thinner than that, and most of the screen stops
        // here.
        var near = mine > 0;
        for (var j = -1; j <= 1 && !near; j++) {
            for (var i = -1; i <= 1; i++) {
                let q = clamp(p + vec2<i32>(i, j) * reach, vec2<i32>(0), size);
                if colour_at(textureLoad(mask, q, 0)) > 0 {
                    near = true;
                }
            }
        }
        if near {
            var best = 0;
            var nearest = 1e9;
            var seen = 0.0;
            for (var j = -reach; j <= reach; j++) {
                for (var i = -reach; i <= reach; i++) {
                    let q = clamp(p + vec2<i32>(i, j), vec2<i32>(0), size);
                    let m = textureLoad(mask, q, 0);
                    let k = colour_at(m);
                    // Outside a silhouette its colour's edge; inside one,
                    // the edge of a later colour (a child in its parent).
                    if k > 0 && k > mine {
                        let d = length(vec2<f32>(f32(i), f32(j)));
                        if d < nearest - 0.01 || (d < nearest + 0.01 && k < best) {
                            nearest = d;
                            best = k;
                            seen = m.g;
                        }
                    }
                }
            }
            if best > 0 {
                // Coverage by distance from the edge, which lies half a
                // pixel short of the nearest covered pixel: smooth, not
                // stepped.
                let a = clamp(width + 0.5 - nearest, 0.0, 1.0) * mix(params.shape.z, 1.0, seen);
                let c = params.colors[min(best - 1, 3)].rgb;
                line = vec4<f32>(c * a, a);
            }
        }
    }
    let o = textureLoad(overlay, p, 0);
    return o + line * (1.0 - o.a);
}
"#;

/// The targets and the composite pass, made at the picture's size.
pub(crate) struct Tools {
    pub(crate) size: (u32, u32),
    pub(crate) samples: u32,
    /// Where the overlay is drawn when multisampled, resolved into `color`.
    pub(crate) multisampled: Option<wgpu::TextureView>,
    pub(crate) color: wgpu::TextureView,
    pub(crate) mask: wgpu::TextureView,
    pub(crate) mask_depth: wgpu::TextureView,
    pub(crate) pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    pub(crate) group: wgpu::BindGroup,
}

impl Tools {
    pub(crate) fn new(gpu: &Gpu, format: wgpu::TextureFormat, samples: u32, size: (u32, u32)) -> Self {
        let texture = |label, format, samples, usage| {
            gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0.max(1),
                        height: size.1.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let drawn = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let read = drawn | wgpu::TextureUsages::TEXTURE_BINDING;
        let multisampled =
            (samples > 1).then(|| texture("tools (multisampled)", format, samples, drawn));
        let color = texture("tools", format, 1, read);
        let mask = texture("outline mask", MASK_FORMAT, 1, read);
        let mask_depth = texture("outline mask depth", MASK_DEPTH, 1, drawn);

        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("runity::tools"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let image = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("runity::tools"),
                entries: &[
                    entry(
                        0,
                        wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                    ),
                    entry(1, image),
                    entry(2, image),
                ],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("runity::tools"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("runity::tools"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("runity::tools"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("runity::tools"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&mask),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&color),
                },
            ],
        });
        Self {
            size,
            samples,
            multisampled,
            color,
            mask,
            mask_depth,
            pipeline,
            uniform,
            group,
        }
    }

    /// This frame's outline colours (linear) and width in pixels; no
    /// colours, no outline.
    pub(crate) fn write(&self, gpu: &Gpu, colors: &[[f32; 3]], width: f32) {
        let mut params = Params {
            colors: [[0.0; 4]; OUTLINE_COLORS],
            shape: [
                width.clamp(0.5, 8.0),
                if colors.is_empty() { 0.0 } else { 1.0 },
                HIDDEN,
                0.0,
            ],
        };
        for (slot, c) in params.colors.iter_mut().zip(colors) {
            *slot = [c[0], c[1], c[2], 1.0];
        }
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&params));
    }
}
