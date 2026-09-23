//! What the camera's lens and shutter do: URP's Depth of Field and Motion
//! Blur, with URP's names and meanings, on the high-dynamic-range frame
//! before bloom and grading — so a lamp out of focus is a bright disc.
//!
//! Both are off by default: a blurred background or a smeared turn is a
//! choice a game makes (a cutscene, a racer), not something 90% of games
//! want on every frame (DNA, postulate 7). Both read the solid scene's
//! depth from the prepass SSAO already draws; asking for either runs that
//! prepass even with SSAO off.
//!
//! * **Depth of field.** `Gaussian` blurs what is past `gaussian_start`,
//!   fully by `gaussian_end`, as URP's Gaussian mode — for a background
//!   that should go soft. `Bokeh` is a thin lens: `focus_distance`,
//!   `focal_length` and `aperture` give each distance its circle of
//!   confusion, near and far, as URP's Bokeh mode. One gathering pass
//!   either way, taps on a spiral, each counted where its own circle
//!   reaches — so what is out of focus spreads over what is behind it.
//! * **Motion blur.** URP's Camera mode: from the depth and last frame's
//!   view, where each point was a frame ago, and the picture averaged along
//!   the way — what the camera's turn sweeps, not what moves by itself.
//!   `intensity` is the share of the frame's movement blurred, `clamp` the
//!   most, as a share of the screen, so a camera cut is not a smear.

use glam::Mat4;
use serde::{Deserialize, Serialize};

use crate::gpu::Gpu;

/// URP's Depth of Field modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FocusMode {
    #[default]
    Off,
    /// The background goes soft from a distance on.
    Gaussian,
    /// A lens in focus at one distance, blurring nearer and farther.
    Bokeh,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DepthOfField {
    pub mode: FocusMode,
    /// Gaussian: where the blur starts, and where it is whole, in metres.
    pub gaussian_start: f32,
    pub gaussian_end: f32,
    /// Gaussian: how wide the blur is at its most, 0.5 to 1.5 as URP.
    pub gaussian_max_radius: f32,
    /// Bokeh: the distance in focus, metres.
    pub focus_distance: f32,
    /// Bokeh: the lens, millimetres. Longer is a shallower focus.
    pub focal_length: f32,
    /// Bokeh: the f-number. Smaller opens the lens and blurs more.
    pub aperture: f32,
}

impl Default for DepthOfField {
    fn default() -> Self {
        Self {
            mode: FocusMode::Off,
            gaussian_start: 10.0,
            gaussian_end: 30.0,
            gaussian_max_radius: 1.0,
            focus_distance: 10.0,
            focal_length: 50.0,
            aperture: 5.6,
        }
    }
}

impl DepthOfField {
    pub const OFF: DepthOfField = DepthOfField {
        mode: FocusMode::Off,
        gaussian_start: 10.0,
        gaussian_end: 30.0,
        gaussian_max_radius: 1.0,
        focus_distance: 10.0,
        focal_length: 50.0,
        aperture: 5.6,
    };

    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let f = |a: f32, b: f32| a + (b - a) * t;
        Self {
            mode: if t >= 0.5 { other.mode } else { self.mode },
            gaussian_start: f(self.gaussian_start, other.gaussian_start),
            gaussian_end: f(self.gaussian_end, other.gaussian_end),
            gaussian_max_radius: f(self.gaussian_max_radius, other.gaussian_max_radius),
            focus_distance: f(self.focus_distance, other.focus_distance),
            focal_length: f(self.focal_length, other.focal_length),
            aperture: f(self.aperture, other.aperture),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MotionBlur {
    /// 0 to 1: the share of each frame's movement blurred. 0 is off.
    pub intensity: f32,
    /// The longest blur, as a share of the screen.
    pub clamp: f32,
    /// Taps along the way: URP's Quality, 4 low to 16 high.
    pub samples: u32,
}

impl Default for MotionBlur {
    fn default() -> Self {
        Self::OFF
    }
}

impl MotionBlur {
    pub const OFF: MotionBlur = MotionBlur {
        intensity: 0.0,
        clamp: 0.05,
        samples: 8,
    };

    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let f = |a: f32, b: f32| a + (b - a) * t;
        Self {
            intensity: f(self.intensity, other.intensity),
            clamp: f(self.clamp, other.clamp),
            samples: if t >= 0.5 {
                other.samples
            } else {
                self.samples
            },
        }
    }
}

/// Hot air: the view shimmers over hot ground, more with distance and
/// towards the horizon, and far off at the horizon the ground turns to a
/// mirror of the sky — the desert's "water" that is not there (an inferior
/// mirage). Off by default; a desert at noon wants both.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeatHaze {
    /// How much the air shimmers, 0 to 1.
    pub intensity: f32,
    /// How much the far ground mirrors the sky, 0 to 1.
    pub mirage: f32,
    /// Where the shimmer and the mirage start, metres from the eye.
    pub distance: f32,
}

impl HeatHaze {
    pub const OFF: HeatHaze = HeatHaze {
        intensity: 0.0,
        mirage: 0.0,
        distance: 40.0,
    };

    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let f = |a: f32, b: f32| a + (b - a) * t;
        Self {
            intensity: f(self.intensity, other.intensity),
            mirage: f(self.mirage, other.mirage),
            distance: f(self.distance, other.distance),
        }
    }
}

impl Default for HeatHaze {
    fn default() -> Self {
        Self::OFF
    }
}

pub const SHADER: &str = include_str!("lens.wgsl");

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LensUniform {
    inverse_view_projection: [[f32; 4]; 4],
    previous_view_projection: [[f32; 4]; 4],
    depth_range: [f32; 4],
    focus: [f32; 4],
    blur: [f32; 4],
    size: [f32; 4],
    motion: [f32; 4],
    view_projection: [[f32; 4]; 4],
    /// The eye; w time in seconds.
    eye: [f32; 4],
    /// Heat haze: shimmer, mirage, where they start (m).
    heat: [f32; 4],
}

/// Where the camera is this frame, and where it looked last frame.
pub(crate) struct View {
    pub view_projection: Mat4,
    pub previous: Mat4,
    pub near: f32,
    pub far: f32,
    pub orthographic: bool,
    pub eye: glam::Vec3,
    /// Seconds, for what moves by itself — the shimmer.
    pub time: f32,
}

/// The widest blur, in pixels, and pixels to a metre of the sensor, for a
/// frame `height` tall.
fn blur_scale(dof: &DepthOfField, height: u32) -> (f32, f32) {
    let h = height.max(1) as f32;
    let largest = match dof.mode {
        FocusMode::Off => 0.0,
        FocusMode::Gaussian => dof.gaussian_max_radius.clamp(0.0, 1.5) * 10.0 * h / 1080.0,
        FocusMode::Bokeh => 24.0 * h / 1080.0,
    };
    // A full-frame sensor is 24 mm tall.
    (largest, h / 0.024)
}

pub(crate) struct LensRenderer {
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    depth_of_field: wgpu::RenderPipeline,
    motion_blur: wgpu::RenderPipeline,
    heat_haze: wgpu::RenderPipeline,
    /// Two targets to go back and forth between.
    targets: Vec<(wgpu::Texture, wgpu::TextureView)>,
    size: (u32, u32),
}

impl LensRenderer {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("runity::lens"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let size = std::mem::size_of::<LensUniform>() as u64;
        let alignment = gpu
            .device
            .limits()
            .min_uniform_buffer_offset_alignment
            .max(1) as u64;
        let stride = size.div_ceil(alignment) * alignment;
        let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lens"),
            size: stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lens"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(size),
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
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("runity::lens"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = |entry: &str| {
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry),
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
                })
        };
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lens"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            depth_of_field: pipeline("fs_depth_of_field"),
            motion_blur: pipeline("fs_motion_blur"),
            heat_haze: pipeline("fs_heat_haze"),
            layout,
            uniforms,
            sampler,
            targets: Vec::new(),
            size: (0, 0),
        }
    }

    /// Whether a frame with these settings needs the lens at all — and so
    /// the prepass's depth.
    pub(crate) fn wanted(post: &crate::post::PostProcess) -> bool {
        post.enabled
            && (post.depth_of_field.mode != FocusMode::Off
                || post.motion_blur.intensity > 0.0
                || post.heat_haze.intensity > 0.0
                || post.heat_haze.mirage > 0.0)
    }

    fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) {
        if self.size == size && !self.targets.is_empty() {
            return;
        }
        self.size = size;
        self.targets = (0..2)
            .map(|_| {
                let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("lens"),
                    size: wgpu::Extent3d {
                        width: size.0.max(1),
                        height: size.1.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: crate::post::HDR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                (texture, view)
            })
            .collect();
    }

    /// Depth of field, then motion blur, from `scene` into a target of its
    /// own; `None` when neither is asked for, and `scene` goes on as it is.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        scene: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        size: (u32, u32),
        post: &crate::post::PostProcess,
        view: &View,
    ) -> Option<&wgpu::TextureView> {
        if !Self::wanted(post) {
            return None;
        }
        self.resize(gpu, size);
        let dof = &post.depth_of_field;
        let motion = &post.motion_blur;
        let (largest, per_metre) = blur_scale(dof, size.1);
        let uniform = LensUniform {
            inverse_view_projection: view.view_projection.inverse().to_cols_array_2d(),
            previous_view_projection: view.previous.to_cols_array_2d(),
            depth_range: [
                view.near,
                view.far,
                if view.orthographic { 1.0 } else { 0.0 },
                0.0,
            ],
            focus: [
                match dof.mode {
                    FocusMode::Off => 0.0,
                    FocusMode::Gaussian => 1.0,
                    FocusMode::Bokeh => 2.0,
                },
                dof.focus_distance.max(0.1),
                dof.focal_length.clamp(1.0, 300.0) / 1000.0,
                dof.aperture.clamp(1.0, 32.0),
            ],
            blur: [
                dof.gaussian_start.max(0.0),
                dof.gaussian_end.max(dof.gaussian_start + 1e-3),
                largest,
                per_metre,
            ],
            size: [
                size.0 as f32,
                size.1 as f32,
                1.0 / size.0.max(1) as f32,
                1.0 / size.1.max(1) as f32,
            ],
            motion: [
                motion.intensity.clamp(0.0, 1.0),
                motion.clamp.clamp(0.0, 0.2),
                motion.samples.clamp(2, 32) as f32,
                0.0,
            ],
            view_projection: view.view_projection.to_cols_array_2d(),
            eye: [view.eye.x, view.eye.y, view.eye.z, view.time],
            heat: [
                post.heat_haze.intensity.clamp(0.0, 1.0),
                post.heat_haze.mirage.clamp(0.0, 1.0),
                post.heat_haze.distance.max(0.0),
                0.0,
            ],
        };
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniform));

        let mut passes: Vec<&wgpu::RenderPipeline> = Vec::new();
        let heat = &post.heat_haze;
        if heat.intensity > 0.0 || heat.mirage > 0.0 {
            passes.push(&self.heat_haze);
        }
        if dof.mode != FocusMode::Off {
            passes.push(&self.depth_of_field);
        }
        if motion.intensity > 0.0 {
            passes.push(&self.motion_blur);
        }
        let mut from = scene;
        for (i, pipeline) in passes.iter().enumerate() {
            let to = &self.targets[i % 2].1;
            let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lens"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(from),
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
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("runity::lens"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: to,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            from = to;
        }
        Some(&self.targets[(passes.len() - 1) % 2].1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_longer_lens_opened_wider_blurs_the_background_more() {
        // The thin lens at the background, for a 1080-line frame.
        let coc = |dof: &DepthOfField, d: f32| {
            let (_, per_metre) = blur_scale(dof, 1080);
            let (s, f, n) = (dof.focus_distance, dof.focal_length / 1000.0, dof.aperture);
            f * f * (d - s).abs() / (n * d * (s - f)) * per_metre
        };
        let standard = DepthOfField {
            mode: FocusMode::Bokeh,
            ..Default::default()
        };
        let portrait = DepthOfField {
            focal_length: 85.0,
            aperture: 1.4,
            focus_distance: 3.0,
            ..standard
        };
        assert!(coc(&standard, 10.0) < 1e-3, "in focus at its distance");
        assert!(coc(&portrait, 30.0) > 4.0 * coc(&standard, 30.0));
    }

    #[test]
    fn lens_settings_read_from_a_scene_line_with_urp_names() {
        let post: crate::post::PostProcess = ron::from_str(
            "(depth_of_field: (mode: Bokeh, focus_distance: 4.0, aperture: 2.0), motion_blur: (intensity: 0.5))",
        )
        .unwrap();
        assert_eq!(post.depth_of_field.mode, FocusMode::Bokeh);
        assert_eq!(post.depth_of_field.focus_distance, 4.0);
        assert_eq!(post.depth_of_field.focal_length, 50.0);
        assert_eq!(post.motion_blur.intensity, 0.5);
        assert!(LensRenderer::wanted(&post));
        assert!(!LensRenderer::wanted(&crate::post::PostProcess::default()));
    }
}
