//! What is see-through and unlit — smoke, dust, sparks, glows: most of a
//! frame's particles — drawn at half the size across, then laid over the
//! picture: a quarter of the pixels for what is soft anyway, and what a
//! screen full of overlapping sprites costs is its pixels.
//!
//! The sprites are drawn, farthest first as ever, into a picture of their
//! own cleared to nothing, by the same pipelines as at full size: over,
//! premultiplied and additive blending each leave in it the light the
//! sprites add and, in alpha, how much of what is behind them they cover.
//! Laid over, `picture · (1 − alpha) + light` is what drawing them one by
//! one over the picture would have made. Multiplying ones need the
//! picture itself, and stay at full size.
//!
//! Their depth test is against the prepass's depth brought down to half
//! size by the nearest of each two by two: a sprite behind a thin post is
//! not drawn over it, at the cost of a pixel's gap round the post's edge
//! where a sprite is behind the far side of the texel.
//!
//! Only where it is the same picture: one sample (TAA smooths the edges),
//! the prepass drawn, and no fog in the air or dust wall — what the unlit
//! shader looks up by the pixel would be looked up at the wrong one.
//! `SCRAP_HALF_PARTICLES=0` draws them at full size, to compare.

use crate::gpu::Gpu;

const SHADER: &str = r#"
@group(0) @binding(0) var full_depth: texture_depth_2d;
@group(1) @binding(0) var low: texture_2d<f32>;
@group(1) @binding(1) var low_sampler: sampler;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

// The nearest of the two by two under a half-size texel.
@fragment
fn fs_depth(in: Out) -> @builtin(frag_depth) f32 {
    let limit = vec2<i32>(textureDimensions(full_depth)) - vec2<i32>(1);
    let base = vec2<i32>(in.position.xy) * 2;
    var d = 1.0;
    for (var y = 0; y < 2; y = y + 1) {
        for (var x = 0; x < 2; x = x + 1) {
            d = min(d, textureLoad(full_depth, min(base + vec2<i32>(x, y), limit), 0));
        }
    }
    return d;
}

@fragment
fn fs_composite(in: Out) -> @location(0) vec4<f32> {
    return textureSampleLevel(low, low_sampler, in.uv, 0.0);
}
"#;

pub(crate) struct LowRes {
    pub(crate) enabled: bool,
    size: (u32, u32),
    color: Option<(wgpu::TextureView, wgpu::TextureView)>,
    depth_layout: wgpu::BindGroupLayout,
    low_layout: wgpu::BindGroupLayout,
    depth_pipeline: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

impl LowRes {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scrap::half-size see-through"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("half-size depth"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let low_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("half-size picture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let depth_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scrap::half-size depth"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("half-size depth"),
                bind_group_layouts: &[Some(&depth_layout)],
                immediate_size: 0,
            })),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs_depth"), targets: &[], compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::render::DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scrap::half-size see-through over"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("half-size over"),
                bind_group_layouts: &[Some(&depth_layout), Some(&low_layout)],
                immediate_size: 0,
            })),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_composite"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: crate::post::HDR_FORMAT,
                    // picture · (1 − alpha) + light; the picture's alpha kept.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("half-size see-through"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            enabled: std::env::var("SCRAP_HALF_PARTICLES").map_or(true, |v| v != "0"),
            size: (0, 0),
            color: None,
            depth_layout,
            low_layout,
            depth_pipeline,
            composite,
            sampler,
        }
    }

    /// The half-size picture and depth for a frame `full` pixels across.
    pub(crate) fn prepare(&mut self, gpu: &Gpu, full: (u32, u32)) {
        let size = (full.0.div_ceil(2).max(1), full.1.div_ceil(2).max(1));
        if self.size == size && self.color.is_some() {
            return;
        }
        self.size = size;
        let texture = |label, format| {
            gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        self.color = Some((
            texture("half-size see-through", crate::post::HDR_FORMAT),
            texture("half-size depth", crate::render::DEPTH_FORMAT),
        ));
    }

    /// After [`Self::prepare`]: bring the prepass's `depth` down to half
    /// size, and begin the pass
    /// the sprites are drawn in: its picture cleared to nothing, its depth
    /// the half-size one.
    pub(crate) fn begin<'e>(&'e self, gpu: &Gpu, encoder: &'e mut wgpu::CommandEncoder, depth: &wgpu::TextureView) -> wgpu::RenderPass<'e> {
        let (color, low_depth) = self.color.as_ref().expect("made by prepare");
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("half-size depth"),
            layout: &self.depth_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) }],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scrap::half-size depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: low_depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.depth_pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scrap::half-size see-through"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: low_depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: crate::gpu_timer::render("particles"),
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    /// The half-size picture, laid over `picture`.
    pub(crate) fn lay_over(&self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, picture: &wgpu::TextureView, depth: &wgpu::TextureView) {
        let Some((color, _)) = self.color.as_ref() else { return };
        let depth_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("half-size depth"),
            layout: &self.depth_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) }],
        });
        let low_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("half-size picture"),
            layout: &self.low_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(color) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scrap::half-size see-through over"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: picture,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: crate::gpu_timer::render("particles"),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &depth_group, &[]);
        pass.set_bind_group(1, &low_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The half-size picture, for the Frame Debugger.
    pub(crate) fn picture(&self) -> Option<&wgpu::TextureView> {
        self.color.as_ref().map(|c| &c.0)
    }
}
