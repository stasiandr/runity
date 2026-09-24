//! Upscaling and dynamic resolution: the scene drawn at fewer pixels than
//! the screen has and made up to it, the way DLSS, FSR and MetalFX do.
//!
//! With `post: (upscaling: (enabled: true, scale: 0.67))` everything up to
//! the lens (the scene, its shadows and lights, SSAO, SSR, TAA, depth of
//! field) is drawn at two thirds of the screen's width and height — under
//! half its pixels — and the picture, still in HDR, is made up to the
//! screen's size before the tonemapper, bloom, the vignette and the
//! editor's handles, which are drawn at the screen's own.
//!
//! What makes it up, best first, the first the device has:
//!
//! * **MetalFX temporal** (Apple): TAA and the upscale in one — each frame
//!   moved by a fraction of a pixel of the smaller picture and blended into
//!   a history at the screen's size, through the depth and where each pixel
//!   was last frame, so detail finer than the pixels drawn builds up over a
//!   few frames. It takes TAA's place.
//! * **MetalFX spatial** (Apple): a single frame's edges, found and
//!   sharpened, after TAA.
//! * **The engine's own**: Catmull-Rom, held inside the texels round each
//!   pixel, and a touch of sharpening, after TAA. Any device.
//!
//! **Dynamic resolution** (`dynamic: (enabled: true, target_ms: 16.6)`)
//! picks the scale itself: from the GPU's time for the frame (the GPU
//! profiler's, which it turns on), down when a frame takes longer than the
//! target, up when well under, never below `min_scale`. A change makes the
//! frame's targets again, so it waits twenty frames between them.

use glam::{Mat4, Vec2};
use serde::{Deserialize, Serialize};

use crate::gpu::Gpu;

pub const SHADER: &str = include_str!("upscale.wgsl");

/// What makes the smaller picture up to the screen's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Method {
    /// The best the device has: MetalFX temporal where there is TAA to
    /// replace, then MetalFX spatial, then the engine's own.
    #[default]
    Auto,
    Temporal,
    Spatial,
    /// The engine's own, on any device.
    Shader,
}

/// The scale chosen by the frame's time on the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DynamicResolution {
    pub enabled: bool,
    /// Milliseconds the GPU may spend on a frame: 16.6 for 60 frames a
    /// second.
    pub target_ms: f32,
    /// The least share of the screen's width it goes down to.
    pub min_scale: f32,
}

impl DynamicResolution {
    pub const OFF: Self = Self {
        enabled: false,
        target_ms: 16.6,
        min_scale: 0.5,
    };
}

impl Default for DynamicResolution {
    fn default() -> Self {
        Self::OFF
    }
}

/// Drawing at fewer pixels and making the picture up to the screen's size.
/// Off by default: at the screen's own size nothing is lost.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Upscaling {
    pub enabled: bool,
    /// The share of the screen's width and height drawn, 0.33 to 1: 0.67
    /// is DLSS's and FSR's Quality, 0.5 their Performance. With `dynamic`
    /// on, where it starts.
    pub scale: f32,
    pub method: Method,
    pub dynamic: DynamicResolution,
    /// How much the engine's own upscaler sharpens, 0 to 1.
    pub sharpness: f32,
}

impl Upscaling {
    pub const OFF: Self = Self {
        enabled: false,
        scale: 0.67,
        method: Method::Auto,
        dynamic: DynamicResolution::OFF,
        sharpness: 0.3,
    };
}

impl Default for Upscaling {
    fn default() -> Self {
        Self::OFF
    }
}

/// Which upscaler made the last frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Used {
    MetalFxTemporal,
    MetalFxSpatial,
    Shader,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct UpscaleUniform {
    inverse_view_projection: [[f32; 4]; 4],
    view_projection: [[f32; 4]; 4],
    previous_view_projection: [[f32; 4]; 4],
    input: [f32; 4],
    output: [f32; 4],
    params: [f32; 4],
}

/// Frames between two changes of the dynamic scale: a change makes the
/// frame's targets again, and the GPU's times need a few frames to show it.
const SETTLE: u32 = 20;
/// The dynamic scale moves in steps of this, so a frame a little slower
/// than the last does not make the targets again.
const STEP: f32 = 0.05;

/// The Halton sequence in base `b`, the `i`-th point in 0..1.
fn halton(mut i: u32, b: u32) -> f32 {
    let mut f = 1.0;
    let mut r = 0.0;
    while i > 0 {
        f /= b as f32;
        r += f * (i % b) as f32;
        i /= b;
    }
    r
}

/// The scale a frame that took `ms` of `target` asks for, from `scale`: the
/// pixels go as its square, so the width as the root of the time's share —
/// a little under, down; up at most a tenth at a time. `None` when it is
/// near enough.
pub(crate) fn next_scale(scale: f32, ms: f32, target: f32, min_scale: f32) -> Option<f32> {
    if ms <= 0.0 || target <= 0.0 {
        return None;
    }
    let share = (target / ms).sqrt();
    let wanted = if ms > target * 1.02 {
        scale * share * 0.97
    } else if ms < target * 0.85 {
        scale * share.min(1.1)
    } else {
        return None;
    };
    let stepped = ((wanted / STEP).floor() * STEP).clamp(min_scale.clamp(0.25, 1.0), 1.0);
    ((stepped - scale).abs() > STEP * 0.5).then_some(stepped)
}

fn target(gpu: &Gpu, label: &str, size: (u32, u32), format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

pub(crate) struct Upscaler {
    /// The scale drawn at now: the settings', or the dynamic one.
    pub(crate) scale: f32,
    frames_since_change: u32,
    /// The picture made, at the screen's size.
    output: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    /// Where each pixel was last frame, at the size drawn.
    #[cfg_attr(not(all(target_vendor = "apple", feature = "metalfx")), allow(dead_code))]
    motion: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    /// Frames blended into the temporal history since it was last made,
    /// and where the camera was.
    frames: u32,
    eye: Option<(glam::Vec3, glam::Vec3)>,
    pub(crate) used: Option<Used>,
    layout: wgpu::BindGroupLayout,
    #[cfg_attr(not(all(target_vendor = "apple", feature = "metalfx")), allow(dead_code))]
    motion_pipeline: wgpu::RenderPipeline,
    upscale_pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// A picture the size of a pixel, for the depth the spatial pass does
    /// not read.
    no_depth: wgpu::TextureView,
    #[cfg(all(target_vendor = "apple", feature = "metalfx"))]
    metal: metal::MetalFx,
}

impl Upscaler {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("runity::upscale"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("upscale"),
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
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("runity::upscale"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = |entry: &str, format: wgpu::TextureFormat| {
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("runity::upscale"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_fullscreen"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        targets: &[Some(format.into())],
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
        };
        let motion_pipeline = pipeline("fs_motion", MOTION_FORMAT);
        let upscale_pipeline = pipeline("fs_upscale", crate::post::HDR_FORMAT);
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upscale"),
            size: std::mem::size_of::<UpscaleUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("upscale"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let no_depth = target(
            gpu,
            "upscale no depth",
            (1, 1),
            crate::ssao::PREPASS_DEPTH,
            wgpu::TextureUsages::TEXTURE_BINDING,
        )
        .create_view(&Default::default());
        Self {
            scale: 1.0,
            frames_since_change: 0,
            output: None,
            motion: None,
            frames: 0,
            eye: None,
            used: None,
            layout,
            motion_pipeline,
            upscale_pipeline,
            uniform,
            sampler,
            no_depth,
            #[cfg(all(target_vendor = "apple", feature = "metalfx"))]
            metal: metal::MetalFx::new(gpu),
        }
    }

    /// The size to draw at for a screen of `output`: the settings' scale,
    /// or the dynamic one.
    pub(crate) fn render_size(&mut self, settings: &Upscaling, output: (u32, u32)) -> (u32, u32) {
        if !settings.dynamic.enabled {
            self.scale = settings.scale;
        } else if self.used.is_none() {
            self.scale = settings.scale.max(settings.dynamic.min_scale);
        }
        self.scale = self.scale.clamp(0.33, 1.0);
        (
            ((output.0 as f32 * self.scale).round() as u32).max(1),
            ((output.1 as f32 * self.scale).round() as u32).max(1),
        )
    }

    /// The frame just drawn took `ms` on the GPU: the dynamic scale follows.
    pub(crate) fn adjust(&mut self, settings: &Upscaling, ms: Option<f32>) {
        self.frames_since_change = self.frames_since_change.saturating_add(1);
        let dynamic = settings.dynamic;
        let Some(ms) = ms.filter(|_| dynamic.enabled && self.frames_since_change >= SETTLE) else {
            return;
        };
        if let Some(next) = next_scale(self.scale, ms, dynamic.target_ms, dynamic.min_scale) {
            self.scale = next;
            self.frames_since_change = 0;
        }
    }

    /// Whether MetalFX temporal will make this frame up, taking TAA's place.
    pub(crate) fn temporal(&self, settings: &Upscaling, taa: bool) -> bool {
        #[cfg(all(target_vendor = "apple", feature = "metalfx"))]
        {
            taa && matches!(settings.method, Method::Auto | Method::Temporal) && self.metal.temporal_supported
        }
        #[cfg(not(all(target_vendor = "apple", feature = "metalfx")))]
        {
            let _ = (settings, taa);
            false
        }
    }

    /// The camera now: a cut starts the temporal history again, as TAA's.
    pub(crate) fn follow(&mut self, eye: glam::Vec3, forward: glam::Vec3) {
        if let Some((was, looked)) = self.eye {
            if eye.distance(was) > 3.0 || forward.dot(looked) < 0.7 {
                self.frames = 0;
            }
        }
        self.eye = Some((eye, forward));
    }

    /// The temporal upscaler's jitter for a picture of `size`, in clip
    /// space: more phases the smaller the picture, so every pixel of the
    /// screen is sampled (eight per pixel drawn, over the scale squared).
    pub(crate) fn jitter(&self, size: (u32, u32)) -> Vec2 {
        let phases = (8.0 / (self.scale * self.scale)).ceil().clamp(8.0, 72.0) as u32;
        let i = self.frames % phases + 1;
        let pixel = Vec2::new(halton(i, 2), halton(i, 3)) - 0.5;
        Vec2::new(
            pixel.x * 2.0 / size.0.max(1) as f32,
            -pixel.y * 2.0 / size.1.max(1) as f32,
        )
    }

    fn output(&mut self, gpu: &Gpu, size: (u32, u32)) {
        if self.output.as_ref().is_some_and(|o| o.2 == size) {
            return;
        }
        let texture = target(
            gpu,
            "upscaled",
            size,
            crate::post::HDR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING,
        );
        let view = texture.create_view(&Default::default());
        self.output = Some((texture, view, size));
        self.frames = 0;
    }

    fn group(&self, gpu: &Gpu, picture: &wgpu::TextureView, depth: &wgpu::TextureView) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("upscale"),
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
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    fn fullscreen(
        encoder: &mut wgpu::CommandEncoder,
        label: &'static str,
        into: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        group: &wgpu::BindGroup,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: into,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: crate::gpu_timer::render(label),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Make `picture` (drawn at `input`) up to `output`. `encoder` is the
    /// frame's so far; MetalFX records into a command buffer of its own, so
    /// with it the frame so far is submitted and a new encoder is left in
    /// `encoder` for what comes after.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        settings: &Upscaling,
        temporal: bool,
        picture: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        input: (u32, u32),
        output: (u32, u32),
        cameras: [Mat4; 3],
        jitter: Vec2,
    ) -> &wgpu::TextureView {
        self.output(gpu, output);
        let [jittered, now, previous] = cameras;
        let uniform = UpscaleUniform {
            inverse_view_projection: jittered.inverse().to_cols_array_2d(),
            view_projection: now.to_cols_array_2d(),
            previous_view_projection: previous.to_cols_array_2d(),
            input: [input.0 as f32, input.1 as f32, 1.0 / input.0 as f32, 1.0 / input.1 as f32],
            output: [output.0 as f32, output.1 as f32, 1.0 / output.0 as f32, 1.0 / output.1 as f32],
            params: [settings.sharpness.clamp(0.0, 1.0), 0.0, 0.0, 0.0],
        };
        gpu.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));

        #[cfg(all(target_vendor = "apple", feature = "metalfx"))]
        {
            let spatial = matches!(settings.method, Method::Auto | Method::Spatial) && self.metal.spatial_supported;
            if temporal || spatial {
                if temporal && self.motion.as_ref().is_none_or(|m| m.2 != input) {
                    let texture = target(
                        gpu,
                        "motion",
                        input,
                        MOTION_FORMAT,
                        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    );
                    let view = texture.create_view(&Default::default());
                    self.motion = Some((texture, view, input));
                }
                if temporal {
                    let group = self.group(gpu, picture, depth);
                    let motion = &self.motion.as_ref().expect("made above").1;
                    Self::fullscreen(encoder, "motion", motion, &self.motion_pipeline, &group);
                }
                let before = std::mem::replace(
                    encoder,
                    gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("runity::metalfx"),
                    }),
                );
                gpu.queue.submit(Some(before.finish()));
                let (out, _, _) = self.output.as_ref().expect("made above");
                let made = if temporal {
                    let reset = self.frames == 0;
                    // In the picture's pixels, y down as Metal's are (the
                    // other sign measures half again as far off).
                    let pixel = Vec2::new(
                        jitter.x * input.0 as f32 * 0.5,
                        -jitter.y * input.1 as f32 * 0.5,
                    );
                    self.metal.temporal(
                        encoder,
                        picture.texture(),
                        depth.texture(),
                        &self.motion.as_ref().expect("made above").0,
                        out,
                        input,
                        output,
                        pixel,
                        reset,
                    )
                } else {
                    self.metal.spatial(encoder, picture.texture(), out, input, output)
                };
                let after = std::mem::replace(
                    encoder,
                    gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("runity::after upscale"),
                    }),
                );
                gpu.queue.submit(Some(after.finish()));
                if made {
                    self.frames = self.frames.saturating_add(1);
                    self.used = Some(if temporal { Used::MetalFxTemporal } else { Used::MetalFxSpatial });
                    return &self.output.as_ref().expect("made above").1;
                }
            }
        }
        let _ = (temporal, jitter, depth);
        let group = self.group(gpu, picture, &self.no_depth);
        let (_, out, _) = self.output.as_ref().expect("made above");
        Self::fullscreen(encoder, "upscale", out, &self.upscale_pipeline, &group);
        self.used = Some(Used::Shader);
        &self.output.as_ref().expect("made above").1
    }
}

/// Where each pixel was, as a share of the screen: half floats are fine to
/// a hundredth of a pixel on a screen 4K wide.
const MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

#[cfg(all(target_vendor = "apple", feature = "metalfx"))]
mod metal {
    //! MetalFX, through wgpu's Metal handles.

    use objc2::rc::Retained;
    use objc2::Message;
    use objc2::runtime::ProtocolObject;
    use objc2_metal::{MTLDevice, MTLPixelFormat, MTLTexture};
    use objc2_metal_fx::{
        MTLFXSpatialScaler, MTLFXSpatialScalerBase, MTLFXSpatialScalerColorProcessingMode,
        MTLFXSpatialScalerDescriptor, MTLFXTemporalScaler, MTLFXTemporalScalerBase,
        MTLFXTemporalScalerDescriptor,
    };
    use wgpu::hal::api::Metal;

    use crate::gpu::Gpu;

    type Sizes = ((u32, u32), (u32, u32));

    pub(super) struct MetalFx {
        device: Option<Retained<ProtocolObject<dyn MTLDevice>>>,
        pub(super) temporal_supported: bool,
        pub(super) spatial_supported: bool,
        spatial: Option<(Sizes, Retained<ProtocolObject<dyn MTLFXSpatialScaler>>)>,
        temporal: Option<(Sizes, Retained<ProtocolObject<dyn MTLFXTemporalScaler>>)>,
    }

    fn raw(texture: &wgpu::Texture) -> Option<Retained<ProtocolObject<dyn MTLTexture>>> {
        // SAFETY: the handle is only read, and only while wgpu keeps the
        // texture alive for the frame.
        let hal = unsafe { texture.as_hal::<Metal>() }?;
        Some(hal.raw_handle().retain())
    }

    impl MetalFx {
        pub(super) fn new(gpu: &Gpu) -> Self {
            // SAFETY: the device is only retained and asked about MetalFX.
            let device = unsafe { gpu.device.as_hal::<Metal>() }.map(|d| d.raw_device().clone());
            let (temporal_supported, spatial_supported) = match &device {
                Some(d) => unsafe {
                    (
                        MTLFXTemporalScalerDescriptor::supportsDevice(d),
                        MTLFXSpatialScalerDescriptor::supportsDevice(d),
                    )
                },
                None => (false, false),
            };
            let off = std::env::var_os("RUNITY_NO_METALFX").is_some();
            Self {
                device,
                temporal_supported: temporal_supported && !off,
                spatial_supported: spatial_supported && !off,
                spatial: None,
                temporal: None,
            }
        }

        fn encode(encoder: &mut wgpu::CommandEncoder, f: impl FnOnce(&ProtocolObject<dyn objc2_metal::MTLCommandBuffer>)) -> bool {
            // SAFETY: a fresh encoder, used for nothing else: MetalFX
            // records into its command buffer and wgpu ends it.
            unsafe {
                encoder.as_hal_mut::<Metal, _, _>(|hal| match hal.and_then(|e| e.raw_command_buffer()) {
                    Some(buffer) => {
                        f(buffer);
                        true
                    }
                    None => false,
                })
            }
        }

        pub(super) fn spatial(
            &mut self,
            encoder: &mut wgpu::CommandEncoder,
            color: &wgpu::Texture,
            output: &wgpu::Texture,
            input: (u32, u32),
            size: (u32, u32),
        ) -> bool {
            let Some(device) = self.device.as_ref().filter(|_| self.spatial_supported) else {
                return false;
            };
            if self.spatial.as_ref().is_none_or(|(s, _)| *s != (input, size)) {
                let made = unsafe {
                    let d = MTLFXSpatialScalerDescriptor::new();
                    d.setColorTextureFormat(MTLPixelFormat::RGBA16Float);
                    d.setOutputTextureFormat(MTLPixelFormat::RGBA16Float);
                    d.setInputWidth(input.0 as usize);
                    d.setInputHeight(input.1 as usize);
                    d.setOutputWidth(size.0 as usize);
                    d.setOutputHeight(size.1 as usize);
                    d.setColorProcessingMode(MTLFXSpatialScalerColorProcessingMode::HDR);
                    d.newSpatialScalerWithDevice(device)
                };
                let Some(made) = made else {
                    self.spatial_supported = false;
                    return false;
                };
                self.spatial = Some(((input, size), made));
            }
            let (Some(color), Some(out)) = (raw(color), raw(output)) else {
                return false;
            };
            let scaler = &self.spatial.as_ref().expect("made above").1;
            unsafe {
                scaler.setColorTexture(Some(&color));
                scaler.setOutputTexture(Some(&out));
                scaler.setInputContentWidth(input.0 as usize);
                scaler.setInputContentHeight(input.1 as usize);
            }
            Self::encode(encoder, |buffer| unsafe { scaler.encodeToCommandBuffer(buffer) })
        }

        #[allow(clippy::too_many_arguments)]
        pub(super) fn temporal(
            &mut self,
            encoder: &mut wgpu::CommandEncoder,
            color: &wgpu::Texture,
            depth: &wgpu::Texture,
            motion: &wgpu::Texture,
            output: &wgpu::Texture,
            input: (u32, u32),
            size: (u32, u32),
            jitter: glam::Vec2,
            reset: bool,
        ) -> bool {
            let Some(device) = self.device.as_ref().filter(|_| self.temporal_supported) else {
                return false;
            };
            if self.temporal.as_ref().is_none_or(|(s, _)| *s != (input, size)) {
                let made = unsafe {
                    let d = MTLFXTemporalScalerDescriptor::new();
                    d.setColorTextureFormat(MTLPixelFormat::RGBA16Float);
                    d.setDepthTextureFormat(MTLPixelFormat::Depth32Float);
                    d.setMotionTextureFormat(MTLPixelFormat::RG16Float);
                    d.setOutputTextureFormat(MTLPixelFormat::RGBA16Float);
                    d.setInputWidth(input.0 as usize);
                    d.setInputHeight(input.1 as usize);
                    d.setOutputWidth(size.0 as usize);
                    d.setOutputHeight(size.1 as usize);
                    // The picture is in HDR, before the eye has adjusted:
                    // MetalFX finds its own exposure to weigh it by.
                    d.setAutoExposureEnabled(true);
                    d.newTemporalScalerWithDevice(device)
                };
                let Some(made) = made else {
                    self.temporal_supported = false;
                    return false;
                };
                self.temporal = Some(((input, size), made));
            }
            let (Some(color), Some(depth), Some(motion), Some(out)) = (raw(color), raw(depth), raw(motion), raw(output)) else {
                return false;
            };
            let scaler = &self.temporal.as_ref().expect("made above").1;
            unsafe {
                scaler.setColorTexture(Some(&color));
                scaler.setDepthTexture(Some(&depth));
                scaler.setMotionTexture(Some(&motion));
                scaler.setOutputTexture(Some(&out));
                scaler.setInputContentWidth(input.0 as usize);
                scaler.setInputContentHeight(input.1 as usize);
                scaler.setJitterOffsetX(jitter.x);
                scaler.setJitterOffsetY(jitter.y);
                scaler.setMotionVectorScaleX(input.0 as f32);
                scaler.setMotionVectorScaleY(input.1 as f32);
                scaler.setDepthReversed(false);
                scaler.setReset(reset);
            }
            Self::encode(encoder, |buffer| unsafe { scaler.encodeToCommandBuffer(buffer) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dynamic_scale_goes_down_when_slow_up_when_fast_and_holds_near_the_target() {
        // Twice the time: the pixels halved, the width by the root.
        let down = next_scale(1.0, 33.2, 16.6, 0.5).expect("down");
        assert!((0.5..0.72).contains(&down), "{down}");
        // Never below the least.
        assert_eq!(next_scale(0.55, 60.0, 16.6, 0.5), Some(0.5));
        // Well under: up, a tenth at most.
        let up = next_scale(0.6, 8.0, 16.6, 0.5).expect("up");
        assert!(up > 0.6 && up <= 0.66, "{up}");
        // Near enough: stays.
        assert_eq!(next_scale(0.8, 16.0, 16.6, 0.5), None);
        // At the top it stays at the top.
        assert_eq!(next_scale(1.0, 4.0, 16.6, 0.5), None);
    }
}
