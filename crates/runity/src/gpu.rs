//! Getting a device, with or without a graphics card.
//!
//! The engine is a guest (see the crate docs), so the interesting half of
//! this module is the half that has no window at all: a device that renders
//! into a texture and hands back pixels. That is what CI runs, what a golden
//! image is made of, and what an agent looks at when it is asked whether a
//! change made the frame better or worse.
//!
//! It works without a GPU because Vulkan has a software implementation —
//! Mesa's lavapipe — and wgpu will use it like any other adapter. On a
//! machine with a card, the same code picks the card instead.
//!
//! One thing this buys and one it does not:
//!
//! * It buys a frame on any machine, including a container with no display
//!   and no hardware. The whole headless workflow rests on that.
//! * It does **not** buy a frame identical to the one a real GPU draws.
//!   Rasterization rules, filtering and floating point all differ between
//!   implementations. So a golden image is a golden image *of one adapter* —
//!   pick it, record which one it was, and compare like with like.

use std::sync::Arc;

/// A device and the queue that feeds it.
///
/// Held behind `Arc` because everything that allocates a buffer needs it and
/// none of them own it.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    /// Whether the device traces rays in hardware — an experiment
    /// ([`crate::render::RayTracing`]), on where the adapter can and
    /// `RUNITY_NO_RAY_TRACING` is not set.
    pub ray_tracing: bool,
    /// Whether the device runs mesh shaders — task and mesh stages in place
    /// of the vertex one — an experiment the terrain can be drawn with
    /// ([`crate::terrain`]): on where the adapter has them and
    /// `RUNITY_MESH_SHADERS` is set. Off by default: measured on an M5
    /// through wgpu 30, the terrain by mesh shaders costs a frame more
    /// than by the vertex shader.
    pub mesh_shaders: bool,
}

/// Set to leave hardware ray tracing off even where the adapter has it.
pub const NO_RAY_TRACING_VAR: &str = "RUNITY_NO_RAY_TRACING";
/// Set to draw terrain with mesh shaders where the adapter has them.
pub const MESH_SHADERS_VAR: &str = "RUNITY_MESH_SHADERS";

/// Why a device could not be created. Worth its own type because "no adapter"
/// and "adapter refused the limits we asked for" need different fixes, and a
/// string would flatten them.
#[derive(Debug)]
pub enum GpuError {
    /// Nothing to render with: no hardware, and no software implementation
    /// installed either.
    NoAdapter,
    /// An adapter exists but would not give us a device.
    NoDevice(String),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuError::NoAdapter => write!(
                f,
                "no graphics adapter; install a software one (mesa-vulkan-drivers \
                 provides lavapipe) to render without a card"
            ),
            GpuError::NoDevice(e) => write!(f, "adapter refused a device: {e}"),
        }
    }
}

impl std::error::Error for GpuError {}

impl Gpu {
    /// Open a device with no window and no display.
    ///
    /// `prefer_software` forces the fallback adapter even where a card
    /// exists, which is what a golden-image run wants: the reference should
    /// not change because the machine running it has a different card.
    pub async fn headless(prefer_software: bool) -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::None,
                force_fallback_adapter: prefer_software,
                compatible_surface: None,
                // Defaults: we ask for downlevel limits anyway, so there is
                // nothing to bucket.
                apply_limit_buckets: Default::default(),
            })
            .await
            .map_err(|_| GpuError::NoAdapter)?;

        // Hardware ray queries, where there are any: an experiment, so
        // asked for on top of the downlevel defaults rather than instead of
        // them — everything else still has to run without.
        let ray_tracing = adapter
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
            && std::env::var_os(NO_RAY_TRACING_VAR).is_none();
        // Mesh shaders likewise: an experiment on top, the terrain's fine
        // grid drawn by task and mesh stages where there are any.
        let mesh_shaders = adapter
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_MESH_SHADER)
            && std::env::var_os(MESH_SHADERS_VAR).is_some();
        let mut required_limits =
            wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits());
        if ray_tracing {
            required_limits = required_limits.using_acceleration_structure_values(adapter.limits());
            // The lit shader reads one storage buffer more when it traces:
            // what each thing is made of, for reflections' hits.
            required_limits.max_storage_buffers_per_shader_stage = required_limits
                .max_storage_buffers_per_shader_stage
                .max(5)
                .min(adapter.limits().max_storage_buffers_per_shader_stage);
        }
        if mesh_shaders {
            required_limits = required_limits.using_recommended_minimum_mesh_shader_values();
        }
        let mut required_features = wgpu::Features::empty();
        // Timestamps at the passes' ends, for the GPU profiler.
        if adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            required_features |= wgpu::Features::TIMESTAMP_QUERY;
        }
        if ray_tracing {
            required_features |= wgpu::Features::EXPERIMENTAL_RAY_QUERY;
        }
        if mesh_shaders {
            required_features |= wgpu::Features::EXPERIMENTAL_MESH_SHADER;
        }
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("runity"),
                // Deliberately the downlevel defaults: whatever runs here has
                // to run on a phone, and asking for desktop limits is how you
                // find that out two months late.
                required_limits,
                required_features,
                // SAFETY: wgpu's experimental features may misbehave or
                // change; ray queries are used only by the renderer's ray
                // tracing, an experiment that is off unless a frame asks,
                // and mesh shaders only by the terrain, which has a way
                // without them.
                experimental_features: if ray_tracing || mesh_shaders {
                    unsafe { wgpu::ExperimentalFeatures::enabled() }
                } else {
                    wgpu::ExperimentalFeatures::disabled()
                },
                ..Default::default()
            })
            .await
            .map_err(|e| GpuError::NoDevice(e.to_string()))?;

        Ok(Self {
            instance,
            adapter,
            device: Arc::new(device),
            queue: Arc::new(queue),
            ray_tracing,
            mesh_shaders,
        })
    }

    /// Same, for callers that are not already async.
    pub fn headless_blocking(prefer_software: bool) -> Result<Self, GpuError> {
        pollster::block_on(Self::headless(prefer_software))
    }

    /// One line naming what we ended up on — worth printing next to every
    /// golden image, since the image is only meaningful against it.
    pub fn describe(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?}, {:?})", info.name, info.device_type, info.backend)
    }
}

/// A texture to draw into, and the machinery to read it back as pixels.
pub struct OffscreenTarget {
    pub width: u32,
    pub height: u32,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    /// Readback needs rows padded to `COPY_BYTES_PER_ROW_ALIGNMENT`; this is
    /// the padded stride, which is usually larger than `width * 4`.
    padded_bytes_per_row: u32,
    buffer: wgpu::Buffer,
}

impl OffscreenTarget {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        // Rgba8UnormSrgb rather than Bgra: a surface would want the platform's
        // order, but nothing here is a surface, and matching the byte order a
        // PNG wants saves a swizzle on every readback.
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            // The same bytes seen as plain RGBA: see `ui_view`.
            view_formats: &[format.remove_srgb_suffix()],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let unpadded = width * 4;
        let padded_bytes_per_row = unpadded.div_ceil(align) * align;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded_bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            width,
            height,
            format,
            texture,
            view,
            padded_bytes_per_row,
            buffer,
        }
    }

    /// The texture's colour format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// The texture seen as plain bytes rather than sRGB: what a UI draws
    /// into. Blending then happens on the sRGB values, as a browser and every
    /// design tool does — a 7% white over dark grey is the 7% the design
    /// file means, not a much lighter linear-light 7%.
    pub fn ui_view(&self) -> wgpu::TextureView {
        self.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.format.remove_srgb_suffix()),
            ..Default::default()
        })
    }

    /// The texture itself, to show in a UI: the Scene view's frame as a
    /// picture, on the same device, with no copy.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Copy the texture back into RGBA8 pixels, row padding removed.
    ///
    /// Synchronous by design: every caller is a test or a tool that has
    /// nothing else to do until the pixels arrive.
    pub fn read_rgba(&self, gpu: &Gpu) -> Vec<u8> {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(encoder.finish()));

        let slice = self.buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        // The map only completes once the queue has caught up, so the poll is
        // not optional: without it this blocks forever.
        let _ = gpu.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        receiver
            .recv()
            .expect("the map callback outlives this function")
            .expect("mapping a buffer we own for reading");

        let data = slice
            .get_mapped_range()
            .expect("the buffer was just mapped for reading");
        let mut pixels = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in 0..self.height {
            let start = (row * self.padded_bytes_per_row) as usize;
            pixels.extend_from_slice(&data[start..start + (self.width * 4) as usize]);
        }
        drop(data);
        self.buffer.unmap();
        pixels
    }

    /// The pixel at `(x, y)` as RGBA, for tests that want to name one spot
    /// rather than compare a whole image.
    pub fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest thing that proves the whole path: a clear color, one
    /// triangle over it, and the pixels back in main memory.
    fn draw_triangle(gpu: &Gpu, target: &OffscreenTarget) {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("triangle"),
                source: wgpu::ShaderSource::Wgsl(
                    r#"
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // A triangle covering the middle of the frame, from the vertex index
    // alone — no buffers, so nothing here can fail for a reason that is not
    // the device itself.
    var p = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 0.6),
        vec2<f32>(-0.6, -0.6),
        vec2<f32>(0.6, -0.6),
    );
    return vec4<f32>(p[i], 0.0, 1.0);
}

@fragment
fn fs() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}
"#
                    .into(),
                ),
            });

        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("triangle"),
                layout: None,
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
                    targets: &[Some(target.format.into())],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("triangle"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit(Some(encoder.finish()));
    }

    #[test]
    fn a_frame_is_drawn_and_read_back_without_a_graphics_card() {
        // Not `unwrap`: a machine with no adapter at all should say so in one
        // sentence rather than fail with a backtrace into wgpu.
        let gpu = match Gpu::headless_blocking(false) {
            Ok(gpu) => gpu,
            Err(e) => {
                eprintln!("skipping: {e}");
                return;
            }
        };
        eprintln!("rendering on {}", gpu.describe());

        let target = OffscreenTarget::new(&gpu, 64, 64);
        draw_triangle(&gpu, &target);
        let pixels = target.read_rgba(&gpu);

        assert_eq!(pixels.len(), 64 * 64 * 4);
        let middle = OffscreenTarget::pixel(&pixels, 64, 32, 32);
        assert!(
            middle[0] > 200 && middle[2] < 60,
            "the triangle should cover the center, got {middle:?}"
        );
        let corner = OffscreenTarget::pixel(&pixels, 64, 1, 1);
        assert!(
            corner[2] > 200 && corner[0] < 60,
            "the clear color should survive in the corner, got {corner:?}"
        );
    }
}
