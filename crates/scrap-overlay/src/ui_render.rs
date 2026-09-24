//! Drawing the screen-space layer.
//!
//! Two pipelines, because the two things are genuinely different: filled
//! rectangles are four vertices generated from an instance, and text is a
//! glyph atlas that has to be shaped, cached and re-uploaded when new
//! characters appear. glyphon does the second; the first is not worth a
//! dependency.
//!
//! Both draw after the scene, with no depth test, in the order the list
//! holds them.

use glyphon::{
    Attrs, Buffer, Cache, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport,
};

use crate::gpu::Gpu;
use crate::ui::Ui;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct QuadInstance {
    /// x, y, width, height in pixels from the top left.
    rect: [f32; 4],
    color: [f32; 4],
    /// The corner radius, and padding.
    shape: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ScreenUniform {
    /// Width and height in pixels, plus padding to sixteen bytes.
    size: [f32; 4],
}

/// Everything needed to put a [`Ui`] on screen.
pub struct UiRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    screen: wgpu::Buffer,
    instances: wgpu::Buffer,
    capacity: u64,
    font_system: FontSystem,
    swash: SwashCache,
    atlas: TextAtlas,
    viewport: Viewport,
    text: TextRenderer,
    /// The font the game gave, by its family name; the system's sans-serif
    /// until then.
    family: Option<String>,
}

impl UiRenderer {
    /// Draw every word with this font from now on: a `.ttf` or `.otf`'s
    /// bytes — the game's own face over the system's.
    pub fn use_font(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        let db = self.font_system.db_mut();
        let before = db.faces().count();
        db.load_font_data(bytes);
        let face = db.faces().nth(before).ok_or("not a font: no face in it")?;
        let name = face
            .families
            .first()
            .map(|(name, _)| name.clone())
            .ok_or("a font with no family name")?;
        self.family = Some(name);
        Ok(())
    }

    /// Build an overlay renderer for an offscreen target.
    pub fn new(gpu: &Gpu, target: &crate::gpu::OffscreenTarget) -> Self {
        Self::with_format(gpu, target.format)
    }

    /// Build one for a window's surface.
    pub fn for_surface(gpu: &Gpu, surface: &crate::surface::Surface) -> Self {
        Self::with_format(gpu, surface.format())
    }

    fn with_format(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("scrap::ui"),
                source: wgpu::ShaderSource::Wgsl(include_str!("ui.wgsl").into()),
            });

        let screen = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen"),
            size: std::mem::size_of::<ScreenUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("screen"),
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
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("screen"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen.as_entire_binding(),
            }],
        });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("scrap::ui"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });

        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("scrap::ui"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<QuadInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4],
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        // Straight alpha over what is already there. The
                        // layer is an overlay; it never replaces the frame.
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    // A strip, because the vertex shader emits four corners
                    // in strip order. The default is a triangle list, which
                    // turns those four into one triangle and half a quad —
                    // and a half quad is easy to miss, because any point in
                    // the surviving half looks correct.
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                // No depth at all: the list's order is the order, and a
                // depth test would let the scene occlude the overlay.
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let capacity = 64;
        let instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui quads"),
            size: capacity * std::mem::size_of::<QuadInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let cache = Cache::new(&gpu.device);
        let mut atlas = TextAtlas::new(&gpu.device, &gpu.queue, &cache, format);
        let text = TextRenderer::new(
            &mut atlas,
            &gpu.device,
            wgpu::MultisampleState::default(),
            None,
        );

        Self {
            pipeline,
            bind_group,
            screen,
            instances,
            capacity,
            font_system: FontSystem::new(),
            swash: SwashCache::new(),
            atlas,
            viewport: Viewport::new(&gpu.device, &cache),
            text,
            family: None,
        }
    }

    /// Draw the list over an offscreen target's existing contents.
    pub fn render(&mut self, gpu: &Gpu, target: &crate::gpu::OffscreenTarget, ui: &Ui) {
        self.render_into(gpu, &target.view, target.width, target.height, ui);
    }

    /// Draw the list over a window's current frame.
    ///
    /// Takes the surface texture rather than acquiring one: the scene has
    /// already drawn into this frame, and acquiring a second would present
    /// an empty one over it.
    pub fn render_to_frame(&mut self, gpu: &Gpu, frame: &crate::surface::AcquiredFrame, ui: &Ui) {
        self.render_into(gpu, &frame.view, frame.width, frame.height, ui);
    }

    /// Draw the list over whatever is already in `view`.
    ///
    /// Loads rather than clears: this runs after the scene, and clearing
    /// would throw the frame away.
    pub fn render_into(
        &mut self,
        gpu: &Gpu,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        ui: &Ui,
    ) {
        if ui.is_empty() {
            return;
        }
        gpu.queue.write_buffer(
            &self.screen,
            0,
            bytemuck::bytes_of(&ScreenUniform {
                size: [width as f32, height as f32, 0.0, 0.0],
            }),
        );

        let quads: Vec<QuadInstance> = ui
            .quads
            .iter()
            .map(|q| QuadInstance {
                rect: [q.x, q.y, q.width, q.height],
                color: q.color.to_array(),
                shape: [q.radius, 0.0, 0.0, 0.0],
            })
            .collect();
        if quads.len() as u64 > self.capacity {
            self.capacity = (quads.len() as u64).next_power_of_two();
            self.instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui quads"),
                size: self.capacity * std::mem::size_of::<QuadInstance>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !quads.is_empty() {
            gpu.queue
                .write_buffer(&self.instances, 0, bytemuck::cast_slice(&quads));
        }

        // Text is shaped every frame. Caching shaped buffers is the obvious
        // optimisation and the obvious way to show stale text, so it waits
        // until something measures that it matters.
        self.viewport
            .update(&gpu.queue, Resolution { width, height });
        let mut buffers: Vec<(Buffer, &crate::ui::TextRun)> = Vec::new();
        for run in &ui.texts {
            let mut buffer = Buffer::new(
                &mut self.font_system,
                Metrics::new(run.size, run.size * 1.25),
            );
            buffer.set_size(Some(width as f32), Some(height as f32));
            buffer.set_text(
                &run.text,
                &Attrs::new().family(match &self.family {
                    Some(name) => Family::Name(name),
                    None => Family::SansSerif,
                }),
                // Advanced, not basic: basic shaping cannot handle scripts
                // that need it, and the text in this project is Cyrillic
                // before it is anything else.
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut self.font_system, false);
            buffers.push((buffer, run));
        }
        let areas: Vec<TextArea> = buffers
            .iter()
            .map(|(buffer, run)| TextArea {
                buffer,
                left: match run.within {
                    Some((box_width, across)) => {
                        let wide = buffer
                            .layout_runs()
                            .map(|line| line.line_w)
                            .fold(0.0, f32::max);
                        run.x + (box_width - wide).max(0.0) * across
                    }
                    None => run.x,
                },
                top: run.y,
                scale: 1.0,
                bounds: match run.clip {
                    Some([left, top, right, bottom]) => TextBounds {
                        left: left.max(0.0) as i32,
                        top: top.max(0.0) as i32,
                        right: (right as i32).min(width as i32),
                        bottom: (bottom as i32).min(height as i32),
                    },
                    None => TextBounds {
                        left: 0,
                        top: 0,
                        right: width as i32,
                        bottom: height as i32,
                    },
                },
                default_color: glyphon::Color::rgba(
                    (run.color.x * 255.0) as u8,
                    (run.color.y * 255.0) as u8,
                    (run.color.z * 255.0) as u8,
                    (run.color.w * 255.0) as u8,
                ),
                custom_glyphs: &[],
            })
            .collect();
        let prepared = self.text.prepare(
            &gpu.device,
            &gpu.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash,
        );

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("scrap::ui"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scrap::ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: crate::gpu_timer::render("ui"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !quads.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.instances.slice(..));
                pass.draw(0..4, 0..quads.len() as u32);
            }
            if prepared.is_ok() {
                let _ = self.text.render(&self.atlas, &self.viewport, &mut pass);
            }
        }
        gpu.queue.submit(Some(encoder.finish()));
    }
}
