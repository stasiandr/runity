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
    Buffer, Cache, ColorMode, ContentType, CustomGlyph, Metrics, RasterizeCustomGlyphRequest,
    RasterizedCustomGlyph, Resolution, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer,
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
    /// The text area an icon hangs from: icons have no text.
    empty: Buffer,
    /// Icons parsed so far, by number.
    svgs: HashMap<u16, resvg::usvg::Tree>,
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
            empty: Buffer::new_empty(Metrics::new(1.0, 1.0)),
            svgs: HashMap::new(),
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

    /// Show a picture from memory — RGBA, sRGB, `width` × `height` — wherever
    /// a node shows `image`: a thumbnail, a preview.
    pub fn set_image_rgba(
        &mut self,
        gpu: &Gpu,
        image: ImageId,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("runity-ui picture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.set_image(gpu, image, &view);
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
                has_text: !layer.texts.is_empty() || !layer.icons.is_empty(),
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
        let bounds_of = |clip: Rect| {
            let c = physical(clip, scale);
            TextBounds {
                left: c[0].floor() as i32,
                top: c[1].floor() as i32,
                right: (c[0] + c[2]).ceil() as i32,
                bottom: (c[1] + c[3]).ceil() as i32,
            }
        };
        let color = |c: crate::Color| glyphon::Color::rgba(c.r, c.g, c.b, c.a);
        // Icons are glyphon's custom glyphs: one per icon, in a text area
        // of its own so that it is clipped on its own.
        let icon_glyphs: Vec<Vec<[CustomGlyph; 1]>> = layers
            .iter()
            .map(|layer| {
                layer
                    .icons
                    .iter()
                    .map(|icon| {
                        [CustomGlyph {
                            id: icon.icon,
                            left: 0.0,
                            top: 0.0,
                            width: icon.rect.width,
                            height: icon.rect.height,
                            color: Some(color(icon.color)),
                            snap_to_physical_pixel: true,
                            metadata: 0,
                        }]
                    })
                    .collect()
            })
            .collect();
        let Self {
            texts,
            atlas,
            viewport,
            swash,
            empty,
            svgs,
            ..
        } = self;
        for (i, layer) in layers.iter().enumerate() {
            let mut areas: Vec<TextArea> = layer
                .texts
                .iter()
                .filter_map(|t| {
                    Some(TextArea {
                        buffer: buffers.get(t.node)?,
                        left: t.x * scale,
                        top: t.y * scale,
                        scale,
                        bounds: bounds_of(t.clip),
                        default_color: color(t.color),
                        custom_glyphs: &[],
                    })
                })
                .collect();
            for (icon, glyph) in layer.icons.iter().zip(&icon_glyphs[i]) {
                areas.push(TextArea {
                    buffer: empty,
                    left: icon.rect.x * scale,
                    top: icon.rect.y * scale,
                    scale,
                    bounds: bounds_of(icon.clip),
                    default_color: color(icon.color),
                    custom_glyphs: glyph,
                });
            }
            if let Err(e) = texts[i].prepare_with_custom(
                &gpu.device,
                &gpu.queue,
                fonts,
                atlas,
                viewport,
                areas,
                swash,
                |request| rasterize_icon(svgs, request),
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

/// Draw an icon at the size the atlas asks for, as a mask the glyph's
/// colour fills.
fn rasterize_icon(
    svgs: &mut HashMap<u16, resvg::usvg::Tree>,
    request: RasterizeCustomGlyphRequest,
) -> Option<RasterizedCustomGlyph> {
    let svg = match svgs.entry(request.id) {
        std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
        std::collections::hash_map::Entry::Vacant(e) => {
            let (_, data) = crate::icons::ICONS.get(request.id as usize)?;
            e.insert(resvg::usvg::Tree::from_data(data, &Default::default()).ok()?)
        }
    };
    let size = svg.size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(request.width as u32, request.height as u32)?;
    let transform = resvg::usvg::Transform::from_scale(
        request.width as f32 / size.width(),
        request.height as f32 / size.height(),
    )
    .post_translate(request.x_bin.as_float(), request.y_bin.as_float());
    resvg::render(svg, transform, &mut pixmap.as_mut());
    Some(RasterizedCustomGlyph {
        data: pixmap.pixels().iter().map(|p| p.alpha()).collect(),
        content_type: ContentType::Mask,
    })
}
