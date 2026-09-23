//! Drawing a [`Ui`] with the engine's `wgpu`, on the engine's device.
//!
//! The paint list becomes one instance buffer of rounded rectangles and
//! pictures plus glyphon's text, a text renderer per layer so that a popup's
//! text is not drawn under the popup. Nothing is uploaded unless the UI's
//! [`Ui::revision`] changed: an idle frame is a handful of draw calls over
//! buffers already on the GPU.
//!
//! The target is the *plain-bytes* view of an sRGB texture
//! (`OffscreenTarget::ui_view`): colours stay the sRGB values the design
//! file wrote and blend as they do in a browser.

use std::collections::HashMap;

use glyphon::{
    Cache, ColorMode, Resolution, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer,
    Viewport,
};
use runity::gpu::Gpu;

use crate::{ImageId, Layer, Rect, Ui};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Shape {
    rect: [f32; 4],
    fill: [f32; 4],
    border: [f32; 4],
    params: [f32; 4],
    clip: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Screen {
    size: [f32; 4],
}

/// One layer's draws: its rectangles as a range of instances, its pictures
/// one instance each, then its text.
struct Batch {
    rects: std::ops::Range<u32>,
    pictures: Vec<(u32, ImageId)>,
    text: usize,
    has_text: bool,
}

pub struct UiRenderer {
    rects: wgpu::RenderPipeline,
    pictures: wgpu::RenderPipeline,
    screen: wgpu::Buffer,
    screen_group: wgpu::BindGroup,
    picture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    instances: wgpu::Buffer,
    capacity: u64,
    batches: Vec<Batch>,
    texts: Vec<TextRenderer>,
    atlas: TextAtlas,
    viewport: Viewport,
    swash: SwashCache,
    images: HashMap<ImageId, wgpu::BindGroup>,
    /// What is on the GPU: the UI's revision and the target's size.
    uploaded: Option<(u64, u32, u32)>,
}

fn physical(r: Rect, scale: f32) -> [f32; 4] {
    [r.x * scale, r.y * scale, r.width * scale, r.height * scale]
}

impl UiRenderer {
    /// A renderer for targets of `format` — the plain (non-sRGB) format of
    /// the view the UI draws into.
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("runity-ui"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ui.wgsl").into()),
        });
        let screen = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("runity-ui screen"),
            size: std::mem::size_of::<Screen>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let screen_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("runity-ui screen"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let screen_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("runity-ui screen"),
            layout: &screen_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen.as_entire_binding(),
            }],
        });
        let picture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("runity-ui picture"),
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("runity-ui picture"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let pipeline = |label: &str, layouts: &[Option<&wgpu::BindGroupLayout>], fragment: &str| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Shape>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x4, 1 => Float32x4, 2 => Float32x4,
                            3 => Float32x4, 4 => Float32x4
                        ],
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let rects = pipeline("runity-ui rects", &[Some(&screen_layout)], "fs");
        let pictures = pipeline(
            "runity-ui pictures",
            &[Some(&screen_layout), Some(&picture_layout)],
            "fs_picture",
        );

        let capacity = 256;
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("runity-ui shapes"),
            size: capacity * std::mem::size_of::<Shape>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cache = Cache::new(device);
        // Web: glyphs are blended as sRGB values into a plain target, the
        // same way the rectangles are.
        let atlas = TextAtlas::with_color_mode(device, &gpu.queue, &cache, format, ColorMode::Web);
        let viewport = Viewport::new(device, &cache);

        Self {
            rects,
            pictures,
            screen,
            screen_group,
            picture_layout,
            sampler,
            instances,
            capacity,
            batches: Vec::new(),
            texts: Vec::new(),
            atlas,
            viewport,
            swash: SwashCache::new(),
            images: HashMap::new(),
            uploaded: None,
        }
    }

    /// Show `view` wherever a node shows `image` — the Scene view's frame,
    /// a thumbnail. The texture is read where it is, on this device.
    pub fn set_image(&mut self, gpu: &Gpu, image: ImageId, view: &wgpu::TextureView) {
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("runity-ui picture"),
            layout: &self.picture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.images.insert(image, group);
    }

    /// Upload what changed since the last call. `width` and `height` are
    /// the target's, in physical pixels.
    pub fn prepare(&mut self, gpu: &Gpu, ui: &mut Ui, width: u32, height: u32) {
        let layers: Vec<Layer> = ui.paint().to_vec();
        let key = (ui.revision(), width, height);
        if self.uploaded == Some(key) {
            return;
        }
        let scale = ui.viewport().2;
        gpu.queue.write_buffer(
            &self.screen,
            0,
            bytemuck::bytes_of(&Screen {
                size: [width as f32, height as f32, 0.0, 0.0],
            }),
        );

        let mut shapes: Vec<Shape> = Vec::new();
        self.batches.clear();
        for (i, layer) in layers.iter().enumerate() {
            let start = shapes.len() as u32;
            for r in &layer.rects {
                shapes.push(Shape {
                    rect: physical(r.rect, scale),
                    fill: r.fill.to_array(),
                    border: r.border.to_array(),
                    params: [r.radius * scale, r.border_width * scale, 0.0, 0.0],
                    clip: physical(r.clip, scale),
                });
            }
            let end = shapes.len() as u32;
            let mut pictures = Vec::new();
            for p in &layer.images {
                pictures.push((shapes.len() as u32, p.image));
                shapes.push(Shape {
                    rect: physical(p.rect, scale),
                    fill: [1.0; 4],
                    border: [0.0; 4],
                    params: [p.radius * scale, 0.0, 0.0, 0.0],
                    clip: physical(p.clip, scale),
                });
            }
            self.batches.push(Batch {
                rects: start..end,
                pictures,
                text: i,
                has_text: !layer.texts.is_empty(),
            });
        }
        if shapes.len() as u64 > self.capacity {
            self.capacity = (shapes.len() as u64).next_power_of_two();
            self.instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("runity-ui shapes"),
                size: self.capacity * std::mem::size_of::<Shape>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !shapes.is_empty() {
            gpu.queue
                .write_buffer(&self.instances, 0, bytemuck::cast_slice(&shapes));
        }

        self.viewport
            .update(&gpu.queue, Resolution { width, height });
        while self.texts.len() < layers.len() {
            self.texts.push(TextRenderer::new(
                &mut self.atlas,
                &gpu.device,
                wgpu::MultisampleState::default(),
                None,
            ));
        }
        let (fonts, buffers) = ui.text_parts();
        for (i, layer) in layers.iter().enumerate() {
            let areas = layer.texts.iter().filter_map(|t| {
                let buffer = buffers.get(t.node)?;
                let clip = physical(t.clip, scale);
                Some(TextArea {
                    buffer,
                    left: t.x * scale,
                    top: t.y * scale,
                    scale,
                    bounds: TextBounds {
                        left: clip[0].floor() as i32,
                        top: clip[1].floor() as i32,
                        right: (clip[0] + clip[2]).ceil() as i32,
                        bottom: (clip[1] + clip[3]).ceil() as i32,
                    },
                    default_color: glyphon::Color::rgba(t.color.r, t.color.g, t.color.b, t.color.a),
                    custom_glyphs: &[],
                })
            });
            if let Err(e) = self.texts[i].prepare(
                &gpu.device,
                &gpu.queue,
                fonts,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash,
            ) {
                eprintln!("runity-ui: text did not fit the atlas: {e}");
            }
        }
        self.atlas.trim();
        self.uploaded = Some(key);
    }

    /// Draw into a pass whose target is the size [`UiRenderer::prepare`]
    /// was told.
    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.set_bind_group(0, &self.screen_group, &[]);
        for batch in &self.batches {
            if !batch.rects.is_empty() {
                pass.set_pipeline(&self.rects);
                pass.draw(0..4, batch.rects.clone());
            }
            for (index, image) in &batch.pictures {
                let Some(group) = self.images.get(image) else {
                    continue;
                };
                pass.set_pipeline(&self.pictures);
                pass.set_bind_group(1, group, &[]);
                pass.draw(0..4, *index..index + 1);
            }
            if batch.has_text {
                let _ = self.texts[batch.text].render(&self.atlas, &self.viewport, pass);
                // Text rendering sets its own pipeline and groups.
                pass.set_vertex_buffer(0, self.instances.slice(..));
                pass.set_bind_group(0, &self.screen_group, &[]);
            }
        }
    }

    /// Prepare and draw in one go, into `view`, over what is there or over
    /// `clear`.
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        ui: &mut Ui,
        clear: Option<crate::Color>,
    ) {
        self.prepare(gpu, ui, width, height);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("runity-ui"),
            });
        {
            let load = match clear {
                Some(c) => {
                    let [r, g, b, a] = c.to_array();
                    wgpu::LoadOp::Clear(wgpu::Color {
                        r: r as f64,
                        g: g as f64,
                        b: b as f64,
                        a: a as f64,
                    })
                }
                None => wgpu::LoadOp::Load,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("runity-ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.render(&mut pass);
        }
        gpu.queue.submit(Some(encoder.finish()));
    }
}
