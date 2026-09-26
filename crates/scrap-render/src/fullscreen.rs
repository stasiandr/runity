//! A fullscreen graph's pass (`shaders/<name>.post.ron`,
//! [`scrap_shadergraph::fullscreen`]): once a frame over the whole picture,
//! lit and drawn, in linear light, before the post-processing tonemaps and
//! grades it — Unity's Fullscreen Shader Graph at Before Post Process. The
//! scene says which (`fullscreen: (graph: "<name>", params: [...])`); each
//! graph is its own pipeline, over one target of the picture's size.

use crate::gpu::Gpu;

/// A scene's fullscreen graph: `fullscreen: (graph: "grey", params: [0.5])`.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct FullscreenPass {
    /// Its name: `shaders/<graph>.post.ron`.
    pub graph: String,
    /// Up to eight numbers its `params` read, in their order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<f32>,
}

const SHADER: &str = r#"
struct FsParams {
    // the scene's eight numbers
    values: array<vec4<f32>, 2>,
    // seconds, width, height, 0
    time_size: vec4<f32>,
    // near, far, 1 when orthographic, 0
    depth_range: vec4<f32>,
};

struct FsIn {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var<uniform> fs: FsParams;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var picture_sampler: sampler;
@group(0) @binding(3) var depth_map: texture_depth_2d;

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> FsIn {
    let corner = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: FsIn;
    out.position = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    return out;
}

/// The picture at `at` (0 to 1 from the top left), linear.
fn fs_color(at: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(picture, picture_sampler, at, 0.0).rgb;
}

/// Metres along the view to what is drawn at `at`.
fn fs_depth(at: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(depth_map));
    let pixel = clamp(vec2<i32>(at * vec2<f32>(size)), vec2<i32>(0), size - vec2<i32>(1));
    let d = textureLoad(depth_map, pixel, 0);
    let near = fs.depth_range.x;
    let far = fs.depth_range.y;
    if fs.depth_range.z > 0.5 {
        return near + d * (far - near);
    }
    return near * far / (far - d * (far - near));
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    return vec4<f32>(max(fullscreen(in), vec3<f32>(0.0)), 1.0);
}

// scrap:fullscreen
"#;

/// The pass's shader with a graph's function in.
fn with_graph(code: &str) -> String {
    SHADER.replace("// scrap:fullscreen\n", code)
}

/// Whether a fullscreen graph's function builds into the pass, without a
/// GPU: what `scrap check` asks of every `.post.ron`.
pub fn check_fullscreen(code: &str) -> Result<(), String> {
    use wgpu::naga;
    let full = with_graph(code);
    let module = naga::front::wgsl::parse_str(&full).map_err(|e| e.emit_to_string(&full))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .map_err(|e| e.emit_to_string(&full))?;
    Ok(())
}

/// A fullscreen graph's function from its file, `shaders/<name>.post.ron`.
pub fn fullscreen_source(path: &std::path::Path) -> Result<String, String> {
    let text = scrap_core::files::read_to_string(path).map_err(|e| e.to_string())?;
    let graph = scrap_shadergraph::fullscreen::parse(&text)?;
    let from = path.file_name().map(|f| format!("shaders/{}", f.to_string_lossy())).unwrap_or_default();
    scrap_shadergraph::fullscreen::to_wgsl_with(&graph, &from, &crate::render::subgraphs_beside(path))
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    values: [[f32; 4]; 2],
    time_size: [f32; 4],
    depth_range: [f32; 4],
}

/// What the pass needs of a frame besides the pictures.
pub(crate) struct View {
    pub near: f32,
    pub far: f32,
    pub orthographic: bool,
    pub time: f32,
}

/// The renderer's fullscreen graphs: a pipeline each, and the target.
pub(crate) struct Fullscreen {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    graphs: std::collections::HashMap<String, wgpu::RenderPipeline>,
    target: Option<((u32, u32), wgpu::TextureView)>,
}

impl Fullscreen {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fullscreen graph"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fullscreen graph"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fullscreen graph"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fullscreen graph"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline_layout,
            sampler,
            uniform,
            graphs: std::collections::HashMap::new(),
            target: None,
        }
    }

    /// Put in graph `name` from its function
    /// ([`scrap_shadergraph::fullscreen::to_wgsl`]): a scene naming it is
    /// drawn through it from the next frame. Refused in words when it does
    /// not build, and the one before goes on.
    pub(crate) fn set(&mut self, gpu: &Gpu, name: &str, code: &str) -> Result<(), String> {
        check_fullscreen(code)?;
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scrap::fullscreen graph"),
            source: wgpu::ShaderSource::Wgsl(with_graph(code).into()),
        });
        let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scrap::fullscreen graph"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: crate::post::HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(format!("the graph does not fit the fullscreen pass: {error}"));
        }
        self.graphs.insert(name.to_string(), pipeline);
        Ok(())
    }

    /// Whether graph `name` is in: a frame naming one that is not draws
    /// as if it named none.
    pub(crate) fn has(&self, name: &str) -> bool {
        self.graphs.contains_key(name)
    }

    /// The picture through the scene's graph, into a target of its own;
    /// `None` when there is none to run, and `picture` goes on as it is.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        pass: Option<&FullscreenPass>,
        picture: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        size: (u32, u32),
        view: &View,
    ) -> Option<&wgpu::TextureView> {
        let pass = pass?;
        let pipeline = self.graphs.get(&pass.graph)?;
        if self.target.as_ref().is_none_or(|(s, _)| *s != size) {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("fullscreen graph"),
                size: wgpu::Extent3d {
                    width: size.0.max(1),
                    height: size.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: crate::post::HDR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            self.target = Some((size, texture.create_view(&wgpu::TextureViewDescriptor::default())));
        }
        let mut values = [0.0f32; 8];
        for (slot, n) in values.iter_mut().zip(&pass.params) {
            *slot = *n;
        }
        let uniform = Uniform {
            values: [[values[0], values[1], values[2], values[3]], [values[4], values[5], values[6], values[7]]],
            time_size: [view.time, size.0 as f32, size.1 as f32, 0.0],
            depth_range: [view.near, view.far, if view.orthographic { 1.0 } else { 0.0 }, 0.0],
        };
        gpu.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fullscreen graph"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(picture),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
            ],
        });
        let (_, target) = self.target.as_ref().expect("made above");
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scrap::fullscreen graph"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: crate::gpu_timer::render("fullscreen graph"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, &group, &[]);
            rp.draw(0..3, 0..1);
        }
        Some(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fullscreen_graph_builds_into_the_pass() {
        let g = scrap_shadergraph::fullscreen::parse(
            r#"(
                params: [("fog", Color)],
                nodes: {
                    "far": Smoothstep(low: 5.0, high: 40.0, of: "depth"),
                    "wave": Sine(of: "time"),
                    "moved": Add(a: "screen", b: "wave"),
                    "seen": SceneColor(at: "moved"),
                    "edge": Fwidth(of: "depth"),
                    "mixed": Lerp(a: "seen", b: "fog", t: "far"),
                    "lit": Add(a: "mixed", b: "edge"),
                },
                output: (color: "lit"),
            )"#,
        )
        .unwrap();
        let code = scrap_shadergraph::fullscreen::to_wgsl(&g, "shaders/haze.post.ron").unwrap();
        check_fullscreen(&code).unwrap_or_else(|e| panic!("{e}\n{code}"));
    }
}
