//! The Frame Debugger: one frame taken apart, as Unity's is — every pass
//! it ran and every draw in each, in order, and the picture as it stood
//! after any one of them.
//!
//! [`Renderer::debug_frame`](crate::Renderer::debug_frame) asks for the
//! next screen frame to be recorded, stopped at an event or not. While it
//! is drawn, each pass says its name as it begins (the same names the GPU
//! profiler times, through `crate::gpu_timer`), and each draw says what it
//! draws — the mesh by name, its triangles, instances and pipeline —
//! through a thread-local, as the profiler's timestamps do: the passes are
//! spread over a dozen modules. Stopped at a draw, the draws after it in
//! its pass are not issued; when the pass is done, what it drew into is
//! copied into the debugger's picture, made something an eye can read —
//! HDR tonemapped, depth as distance, normals as colours. Passes after it
//! run as ever, so nothing the next frame keeps (TAA's history, the Hi-Z
//! pyramid) is left half drawn.
//!
//! What is recorded is a list, [`FrameCapture`], with [`FrameCapture::text`]
//! for an agent or a log; the shell draws it over the game (F9), the
//! picture under it. A frame not asked for records nothing: every hook is a
//! thread-local read of `None`.

use std::cell::RefCell;

/// What a pass is to the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassKind {
    Render,
    Compute,
}

/// One draw call, as the debugger lists it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DrawCall {
    /// What is drawn: the mesh's name, or what the pass draws ("sky",
    /// "gpu particles").
    pub what: String,
    /// Triangles of one instance, at the level of detail drawn.
    pub triangles: u64,
    /// Instances in the call. With [`Self::culled_on_gpu`], the most there
    /// can be: the GPU decides how many it keeps.
    pub instances: u32,
    pub culled_on_gpu: bool,
    /// The pipeline it is drawn with: face, blend, shader, skinning.
    pub pipeline: String,
    /// The textures bound, by name where known.
    pub textures: Vec<String>,
}

impl DrawCall {
    /// A pass's one full-screen triangle, named.
    pub fn fullscreen(what: &str) -> Self {
        Self { what: what.into(), triangles: 1, instances: 1, pipeline: what.into(), ..Default::default() }
    }
}

/// A step of the frame: a pass beginning, or a draw inside one.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Pass(PassKind),
    Draw(DrawCall),
}

/// One event of a recorded frame.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameEvent {
    /// The pass it is in (or begins).
    pub pass: &'static str,
    /// Which pass of the frame, counting from 0: the shadow cascades are
    /// four passes of one name.
    pub pass_number: u32,
    pub step: Step,
}

/// What the debugger's picture shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    /// The pass it was taken after.
    pub pass: &'static str,
    /// What of the pass: "hdr", "depth", "normals", "occlusion".
    pub target: &'static str,
    pub size: (u32, u32),
}

/// A frame taken apart: see the module.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrameCapture {
    pub events: Vec<FrameEvent>,
    /// The event it was stopped at, if any.
    pub stop: Option<usize>,
    /// What the picture shows: `None` when not stopped, or stopped in a
    /// pass whose target the debugger cannot show — then the frame
    /// itself is the picture.
    pub picture: Option<Picture>,
    /// The frame's size.
    pub size: (u32, u32),
}

impl FrameCapture {
    /// Draw calls in the frame.
    pub fn draws(&self) -> usize {
        self.events.iter().filter(|e| matches!(e.step, Step::Draw(_))).count()
    }

    /// Triangles over every draw, instances counted.
    pub fn triangles(&self) -> u64 {
        self.events
            .iter()
            .map(|e| match &e.step {
                Step::Draw(d) => d.triangles * d.instances as u64,
                Step::Pass(_) => 0,
            })
            .sum()
    }

    /// Each pass once, in order: its name, number, draws and triangles.
    pub fn passes(&self) -> Vec<(&'static str, u32, usize, u64)> {
        let mut out: Vec<(&'static str, u32, usize, u64)> = Vec::new();
        for e in &self.events {
            match &e.step {
                Step::Pass(_) => out.push((e.pass, e.pass_number, 0, 0)),
                Step::Draw(d) => {
                    if let Some(last) = out.last_mut().filter(|p| p.1 == e.pass_number) {
                        last.2 += 1;
                        last.3 += d.triangles * d.instances as u64;
                    }
                }
            }
        }
        out
    }

    /// One line for an event: a pass as its heading, a draw indented.
    pub fn line(&self, index: usize) -> String {
        let Some(e) = self.events.get(index) else {
            return String::new();
        };
        match &e.step {
            Step::Pass(kind) => {
                let (draws, tris) = self
                    .passes()
                    .into_iter()
                    .find(|p| p.1 == e.pass_number)
                    .map_or((0, 0), |p| (p.2, p.3));
                let kind = match kind {
                    PassKind::Render => "",
                    PassKind::Compute => " (compute)",
                };
                if draws > 0 {
                    format!("{}{kind} · {draws} draws, {} tris", e.pass, count(tris))
                } else {
                    format!("{}{kind}", e.pass)
                }
            }
            Step::Draw(d) => {
                let instances = match (d.instances, d.culled_on_gpu) {
                    (1, false) => String::new(),
                    (n, false) => format!(" ×{n}"),
                    (n, true) => format!(" ≤×{n}"),
                };
                format!("  {}{instances} · {} tris", d.what, count(d.triangles * d.instances as u64))
            }
        }
    }

    /// The whole frame as text: each pass, and under it its draws — what
    /// an agent reads instead of the window.
    pub fn text(&self) -> String {
        let mut out = format!(
            "{}x{}: {} passes, {} draws, {} triangles\n",
            self.size.0,
            self.size.1,
            self.events.iter().filter(|e| matches!(e.step, Step::Pass(_))).count(),
            self.draws(),
            count(self.triangles())
        );
        for i in 0..self.events.len() {
            let mark = if self.stop == Some(i) { "▶" } else { " " };
            out += &format!("{mark}{i:>5} {}\n", self.line(i));
        }
        out
    }

    /// An event's particulars: everything recorded about it, a line each.
    pub fn details(&self, index: usize) -> Vec<String> {
        let Some(e) = self.events.get(index) else {
            return Vec::new();
        };
        let mut out = vec![format!("event {index} in pass {} (#{})", e.pass, e.pass_number)];
        match &e.step {
            Step::Pass(kind) => {
                out.push(format!("{kind:?} pass"));
                if let Some((_, _, draws, tris)) = self.passes().into_iter().find(|p| p.1 == e.pass_number) {
                    out.push(format!("{draws} draws, {} triangles", count(tris)));
                }
            }
            Step::Draw(d) => {
                out.push(format!("draws {}", d.what));
                out.push(format!("triangles {} per instance", count(d.triangles)));
                out.push(if d.culled_on_gpu {
                    format!("instances up to {} (culled on the GPU)", d.instances)
                } else {
                    format!("instances {}", d.instances)
                });
                out.push(format!("pipeline {}", d.pipeline));
                for t in &d.textures {
                    out.push(format!("texture {t}"));
                }
            }
        }
        out
    }
}

/// A count as it reads: 1 331 727 → "1.33M".
pub fn count(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=999_999 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.2}M", n as f64 / 1e6),
    }
}

/// The frame being recorded, for the passes and draws to add to.
struct Recording {
    events: Vec<FrameEvent>,
    stop: Option<usize>,
    /// Passes begun so far.
    passes: u32,
    current: (&'static str, u32),
    /// The pass the stop is in, once it is reached; and whether the stop
    /// is the pass itself (then the whole pass is drawn).
    stop_pass: Option<u32>,
    stop_is_pass: bool,
    picture: Option<Picture>,
}

thread_local! {
    static RECORDING: RefCell<Option<Recording>> = const { RefCell::new(None) };
}

/// Start recording the frame about to be drawn, stopped at `stop`.
pub(crate) fn begin(stop: Option<usize>) {
    RECORDING.with(|r| {
        *r.borrow_mut() = Some(Recording {
            events: Vec::new(),
            stop,
            passes: 0,
            current: ("", 0),
            stop_pass: None,
            stop_is_pass: false,
            picture: None,
        })
    });
}

/// The frame recorded, and recording over.
pub(crate) fn finish(size: (u32, u32)) -> Option<FrameCapture> {
    RECORDING.with(|r| r.borrow_mut().take()).map(|r| FrameCapture {
        events: r.events,
        stop: r.stop,
        picture: r.picture,
        size,
    })
}

/// A pass begins (through `crate::gpu_timer`).
pub(crate) fn pass(name: &'static str, kind: PassKind) {
    RECORDING.with(|r| {
        let mut r = r.borrow_mut();
        let Some(r) = r.as_mut() else { return };
        let number = r.passes;
        r.passes += 1;
        r.current = (name, number);
        if r.stop == Some(r.events.len()) {
            r.stop_pass = Some(number);
            r.stop_is_pass = true;
        }
        r.events.push(FrameEvent { pass: name, pass_number: number, step: Step::Pass(kind) });
    });
}

/// Whether a frame is being recorded.
#[cfg(test)]
fn recording() -> bool {
    RECORDING.with(|r| r.borrow().is_some())
}

/// A draw about to be issued: recorded, when a frame is, and whether it
/// may be — not when it comes after the stop in the stop's pass.
pub(crate) fn draw(call: impl FnOnce() -> DrawCall) -> bool {
    RECORDING.with(|r| {
        let mut r = r.borrow_mut();
        let Some(r) = r.as_mut() else { return true };
        let index = r.events.len();
        let (pass, number) = r.current;
        if r.stop == Some(index) {
            r.stop_pass = Some(number);
        }
        r.events.push(FrameEvent { pass, pass_number: number, step: Step::Draw(call()) });
        match r.stop {
            Some(stop) => r.stop_is_pass || r.stop_pass != Some(number) || index <= stop,
            None => true,
        }
    })
}

/// Passes begun so far: taken before a pass (or a module's passes), and
/// given to [`Debugger::snapshot`] after.
pub(crate) fn mark() -> u32 {
    RECORDING.with(|r| r.borrow().as_ref().map_or(0, |r| r.passes))
}

/// Whether the stop fell in a pass begun since `mark` and its picture is
/// not taken yet: then it is taken now, of `target`.
fn stopped_since(mark: u32, target: &'static str, size: (u32, u32)) -> bool {
    RECORDING.with(|r| {
        let mut r = r.borrow_mut();
        let Some(r) = r.as_mut() else { return false };
        match r.stop_pass {
            Some(p) if p >= mark && r.picture.is_none() => {
                let pass = r.events.iter().find(|e| e.pass_number == p).map_or("", |e| e.pass);
                r.picture = Some(Picture { pass, target, size });
                true
            }
            _ => false,
        }
    })
}

/// What a snapshot is of, and how to show it.
pub(crate) enum Source<'a> {
    /// Linear HDR colour, tonemapped to be seen.
    Hdr(&'a wgpu::TextureView),
    /// Normals, -1..1 to colours.
    Normals(&'a wgpu::TextureView),
    /// The alpha channel as grey: the occlusion.
    Alpha(&'a wgpu::TextureView),
    /// An orthographic depth (a shadow map), as it is.
    LinearDepth(&'a wgpu::TextureView),
}

const SHADER: &str = r#"
struct Params {
    mode: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
};
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var colour: texture_2d<f32>;
@group(1) @binding(0) var depth: texture_depth_2d;
@group(2) @binding(0) var picture: texture_2d<f32>;
@group(2) @binding(1) var picture_sampler: sampler;

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

@fragment
fn fs_colour(in: Out) -> @location(0) vec4<f32> {
    let c = textureLoad(colour, vec2<i32>(in.position.xy), 0);
    switch params.mode {
        case 1u: { return vec4<f32>(c.xyz * 0.5 + 0.5, 1.0); }
        case 2u: { return vec4<f32>(vec3<f32>(c.a), 1.0); }
        default: {
            // ACES-ish, enough to see by.
            let x = max(c.rgb, vec3<f32>(0.0));
            return vec4<f32>(clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
        }
    }
}

@fragment
fn fs_depth(in: Out) -> @location(0) vec4<f32> {
    let d = textureLoad(depth, vec2<i32>(in.position.xy), 0);
    return vec4<f32>(vec3<f32>(1.0 - d), 1.0);
}

@fragment
fn fs_show(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(picture, picture_sampler, in.uv).rgb, 1.0);
}
"#;

/// The picture's format: sRGB bytes, as a PNG wants them.
const PICTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

struct Pipelines {
    colour: wgpu::RenderPipeline,
    depth: wgpu::RenderPipeline,
    params_layout: wgpu::BindGroupLayout,
    depth_layout: wgpu::BindGroupLayout,
    show_layout: wgpu::BindGroupLayout,
    module: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// Showing the picture on a target, by the target's format.
    show: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

/// The renderer's debugger: what was asked, what was recorded, the
/// picture, and the pipelines that make it — made the first time it is
/// asked for, so a game that never debugs a frame holds none of it.
#[derive(Default)]
pub(crate) struct Debugger {
    /// The next screen frame is recorded, stopped here.
    pub(crate) wanted: Option<Option<usize>>,
    pub(crate) captured: Option<FrameCapture>,
    pipelines: Option<Pipelines>,
    picture: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
}

impl Debugger {
    fn pipelines(&mut self, gpu: &crate::gpu::Gpu) -> &Pipelines {
        self.pipelines.get_or_insert_with(|| {
            let device = &gpu.device;
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("scrap::frame debugger"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
            let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            };
            let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("frame debugger params"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    texture(1, wgpu::TextureSampleType::Float { filterable: false }),
                ],
            });
            let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("frame debugger depth"),
                entries: &[texture(0, wgpu::TextureSampleType::Depth)],
            });
            let show_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("frame debugger show"),
                entries: &[
                    texture(0, wgpu::TextureSampleType::Float { filterable: true }),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("frame debugger"),
                bind_group_layouts: &[Some(&params_layout), Some(&depth_layout), Some(&show_layout)],
                immediate_size: 0,
            });
            let pipeline = |entry: &str, format| pipeline(device, &layout, &module, entry, format);
            let colour = pipeline("fs_colour", PICTURE_FORMAT);
            let depth = pipeline("fs_depth", PICTURE_FORMAT);
            let params = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("frame debugger params"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("frame debugger"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            Pipelines { colour, depth, params_layout, depth_layout, show_layout, module, layout, params, sampler, show: Vec::new() }
        })
    }

    /// After a pass (or a module's passes) begun since `mark`: when the
    /// stop fell in them, what they drew, into the picture.
    pub(crate) fn snapshot(
        &mut self,
        gpu: &crate::gpu::Gpu,
        encoder: &mut wgpu::CommandEncoder,
        mark: u32,
        source: Source<'_>,
    ) {
        let size = match &source {
            Source::Hdr(v) | Source::Normals(v) | Source::Alpha(v) | Source::LinearDepth(v) => {
                let t = v.texture();
                (t.width(), t.height())
            }
        };
        let target = match source {
            Source::Hdr(_) => "hdr",
            Source::Normals(_) => "normals",
            Source::Alpha(_) => "occlusion",
            Source::LinearDepth(_) => "depth",
        };
        if !stopped_since(mark, target, size) {
            return;
        }
        let size = (size.0.max(1), size.1.max(1));
        if self.picture.as_ref().is_none_or(|p| p.2 != size) {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("frame debugger picture"),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: PICTURE_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.picture = Some((texture, view, size));
        }
        let mode: u32 = match source {
            Source::Hdr(_) => 0,
            Source::Normals(_) => 1,
            Source::Alpha(_) => 2,
            Source::LinearDepth(_) => 3,
        };
        self.pipelines(gpu);
        let Some(p) = self.pipelines.as_ref() else { return };
        gpu.queue.write_buffer(&p.params, 0, bytemuck::cast_slice(&[mode, 0, 0, 0]));
        // What the layouts need that this source does not give: a stand-in.
        let blank = |format, usage| {
            gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("frame debugger blank"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let (colour, depth, pipeline) = match source {
            Source::Hdr(v) | Source::Normals(v) | Source::Alpha(v) => (
                v.clone(),
                blank(wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::TEXTURE_BINDING),
                &p.colour,
            ),
            Source::LinearDepth(v) => (
                blank(wgpu::TextureFormat::Rgba16Float, wgpu::TextureUsages::TEXTURE_BINDING),
                v.clone(),
                &p.depth,
            ),
        };
        let params_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame debugger params"),
            layout: &p.params_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: p.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&colour) },
            ],
        });
        let depth_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame debugger depth"),
            layout: &p.depth_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&depth) }],
        });
        let show_blank = blank(PICTURE_FORMAT, wgpu::TextureUsages::TEXTURE_BINDING);
        let show_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame debugger show"),
            layout: &p.show_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&show_blank) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&p.sampler) },
            ],
        });
        let Some((_, view, _)) = self.picture.as_ref() else { return };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scrap::frame debugger"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &params_group, &[]);
        pass.set_bind_group(1, &depth_group, &[]);
        pass.set_bind_group(2, &show_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Whether there is a picture of a stop to show.
    pub(crate) fn has_picture(&self) -> bool {
        self.captured.as_ref().is_some_and(|c| c.picture.is_some()) && self.picture.is_some()
    }

    /// The picture, drawn over `view` inside `area` (x, y, width, height
    /// in pixels), kept to its own proportions.
    pub(crate) fn show(
        &mut self,
        gpu: &crate::gpu::Gpu,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        area: (f32, f32, f32, f32),
    ) {
        if !self.has_picture() {
            return;
        }
        self.pipelines(gpu);
        let Some(p) = self.pipelines.as_mut() else { return };
        if !p.show.iter().any(|(f, _)| *f == format) {
            let made = pipeline(&gpu.device, &p.layout, &p.module, "fs_show", format);
            p.show.push((format, made));
        }
        let Some((_, picture, size)) = self.picture.as_ref() else { return };
        let Some((_, show)) = p.show.iter().find(|(f, _)| *f == format) else { return };
        let blank_depth = gpu
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("frame debugger blank"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let blank_colour = gpu
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("frame debugger blank"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let params_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame debugger params"),
            layout: &p.params_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: p.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&blank_colour) },
            ],
        });
        let depth_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame debugger depth"),
            layout: &p.depth_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&blank_depth) }],
        });
        let show_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame debugger show"),
            layout: &p.show_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(picture) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&p.sampler) },
            ],
        });
        // Fitted inside the area, its own aspect kept.
        let (x, y, w, h) = area;
        let aspect = size.0 as f32 / size.1.max(1) as f32;
        let (fw, fh) = if w / h.max(1.0) > aspect { (h * aspect, h) } else { (w, w / aspect) };
        let (fx, fy) = (x + (w - fw) * 0.5, y + (h - fh) * 0.5);
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame debugger show"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scrap::frame debugger show"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(fx, fy, fw.max(1.0), fh.max(1.0), 0.0, 1.0);
            pass.set_pipeline(show);
            pass.set_bind_group(0, &params_group, &[]);
            pass.set_bind_group(1, &depth_group, &[]);
            pass.set_bind_group(2, &show_group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit(Some(encoder.finish()));
    }

    /// The picture's pixels, RGBA in sRGB: for a test or a tool. Waits on
    /// the GPU.
    pub(crate) fn read_picture(&self, gpu: &crate::gpu::Gpu) -> Option<(Vec<u8>, (u32, u32))> {
        if !self.has_picture() {
            return None;
        }
        let (texture, _, size) = self.picture.as_ref()?;
        let row = size.0 * 4;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame debugger readback"),
            size: (padded * size.1) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame debugger readback"),
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(size.1) },
            },
            wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        );
        gpu.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let _ = gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        receiver.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut pixels = Vec::with_capacity((row * size.1) as usize);
        for y in 0..size.1 {
            let start = (y * padded) as usize;
            pixels.extend_from_slice(&data[start..start + row as usize]);
        }
        Some((pixels, *size))
    }
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entry: &str,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("scrap::frame debugger"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(what: &str, tris: u64) -> DrawCall {
        DrawCall { what: what.into(), triangles: tris, instances: 1, ..Default::default() }
    }

    #[test]
    fn a_frame_is_recorded_pass_by_pass_and_draw_by_draw() {
        begin(None);
        pass("shadows", PassKind::Render);
        assert!(draw(|| call("rock", 10)));
        pass("scene", PassKind::Render);
        assert!(draw(|| call("rock", 10)));
        assert!(draw(|| call("tree", 5)));
        let c = finish((10, 10)).unwrap();
        assert_eq!(c.events.len(), 5);
        assert_eq!(c.draws(), 3);
        assert_eq!(c.triangles(), 25);
        assert_eq!(c.passes(), vec![("shadows", 0, 1, 10), ("scene", 1, 2, 15)]);
        assert!(c.text().contains("scene · 2 draws, 15 tris"), "{}", c.text());
        // Nothing asked: nothing kept, and every draw goes ahead.
        assert!(!recording());
        assert!(draw(|| unreachable!("not recording: the call is never described")));
    }

    #[test]
    fn stopped_at_a_draw_the_rest_of_its_pass_is_not_drawn_and_later_passes_are() {
        begin(Some(3));
        pass("shadows", PassKind::Render); // 0
        assert!(draw(|| call("rock", 10))); // 1
        let mark = mark();
        pass("scene", PassKind::Render); // 2
        assert!(draw(|| call("rock", 10))); // 3: the stop
        assert!(!draw(|| call("tree", 5))); // 4: after it, in its pass
        assert!(stopped_since(mark, "hdr", (4, 4)));
        assert!(!stopped_since(mark, "hdr", (4, 4)), "one picture a frame");
        pass("post", PassKind::Render);
        assert!(draw(|| call("quad", 1)));
        let c = finish((4, 4)).unwrap();
        assert_eq!(c.picture.as_ref().map(|p| p.pass), Some("scene"));
    }

    #[test]
    fn stopped_at_a_pass_the_whole_pass_is_drawn() {
        begin(Some(0));
        let mark = mark();
        pass("scene", PassKind::Render);
        assert!(draw(|| call("rock", 10)));
        assert!(draw(|| call("tree", 5)));
        assert!(stopped_since(mark, "hdr", (4, 4)));
        finish((4, 4));
    }
}
