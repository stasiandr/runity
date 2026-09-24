//! Temporal antialiasing: HDRP's and Unreal's TAA.
//!
//! Multisampling smooths the edges of triangles and nothing else. What
//! still crawls — a thin blade of grass a pixel wide, a glint, the ripples
//! in sand, a shadow's stair-steps, a highlight on a curve — is aliasing
//! inside the shading, and moving it is what makes a picture look cheap.
//! So each frame the whole view is moved by a fraction of a pixel (a
//! Halton sequence of eight), and the frame is blended into what came
//! before: each pixel reads where it was last frame, through the depth and
//! last frame's camera, and keeps nine parts of that to one of this. Over
//! eight frames every pixel has been sampled at eight places inside it.
//!
//! What was there last frame may not be there now — something moved, or
//! came out from behind — so the history is clipped to the colours this
//! frame has round the pixel (the variance of its 3×3 neighbours), and
//! what falls outside is pulled in: a ghost cannot outlast a frame or two.
//! The blend is weighted by brightness, so a sun glint does not smear.
//!
//! Where there is no history — the first frame, a new size — the frame is
//! drawn unmoved and passed through, so a single shot is as sharp as it
//! was. Things moving by themselves are reprojected as if they stood
//! still (there are no motion vectors per object); the clip keeps what
//! that costs to a soft trail.

use glam::{Mat4, Vec2};

use crate::gpu::Gpu;

pub const SHADER: &str = include_str!("taa.wgsl");

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TaaUniform {
    /// This frame's camera, moved as it was drawn, the other way: screen
    /// and depth into the world.
    inverse_view_projection: [[f32; 4]; 4],
    /// Last frame's camera, unmoved.
    previous_view_projection: [[f32; 4]; 4],
    /// width, height, 1/width, 1/height
    size: [f32; 4],
    /// 1 when there is a history to read; how much of this frame to take.
    params: [f32; 4],
}

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

pub(crate) struct Taa {
    /// Two pictures, each the history of the other in turn.
    pictures: [wgpu::Texture; 2],
    views: [wgpu::TextureView; 2],
    /// Which one this frame writes.
    now: usize,
    size: (u32, u32),
    /// Frames drawn into the history since it was last made.
    frames: u32,
    /// Where the camera was and which way it looked, last frame: a jump
    /// from there is a cut, and a cut starts the history again.
    eye: Option<(glam::Vec3, glam::Vec3)>,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

fn picture(gpu: &Gpu, size: (u32, u32)) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("taa history"),
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
    })
}

impl Taa {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("runity::taa"),
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
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("taa"),
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
                    texture(1, wgpu::TextureSampleType::Float { filterable: true }),
                    texture(2, wgpu::TextureSampleType::Float { filterable: true }),
                    texture(3, wgpu::TextureSampleType::Depth),
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("runity::taa"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("runity::taa"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_resolve"),
                    compilation_options: Default::default(),
                    targets: &[Some(crate::post::HDR_FORMAT.into())],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("taa"),
            size: std::mem::size_of::<TaaUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("taa"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let pictures = [picture(gpu, (1, 1)), picture(gpu, (1, 1))];
        let views = [
            pictures[0].create_view(&Default::default()),
            pictures[1].create_view(&Default::default()),
        ];
        Self {
            pictures,
            views,
            now: 0,
            size: (1, 1),
            frames: 0,
            eye: None,
            layout,
            pipeline,
            uniform,
            sampler,
        }
    }

    /// Sized to the frame: a new size starts the history again.
    pub(crate) fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) {
        if self.size == size {
            return;
        }
        self.size = size;
        self.pictures = [picture(gpu, size), picture(gpu, size)];
        self.views = [
            self.pictures[0].create_view(&Default::default()),
            self.pictures[1].create_view(&Default::default()),
        ];
        self.frames = 0;
    }

    /// The camera now: a cut from where it was — three metres in a frame,
    /// or a quarter turn — and what came before is forgotten, so the new
    /// shot does not start through a ghost of the old one.
    pub(crate) fn follow(&mut self, eye: glam::Vec3, forward: glam::Vec3) {
        if let Some((was, looked)) = self.eye {
            if eye.distance(was) > 3.0 || forward.dot(looked) < 0.7 {
                self.frames = 0;
            }
        }
        self.eye = Some((eye, forward));
    }

    /// Frames blended into the history since it was last started.
    pub(crate) fn frames(&self) -> u32 {
        self.frames
    }

    /// How far this frame is moved, in clip space: nothing when there is
    /// no history to blend it with.
    pub(crate) fn jitter(&self) -> Vec2 {
        if self.frames == 0 {
            return Vec2::ZERO;
        }
        let i = self.frames % 8 + 1;
        let pixel = Vec2::new(halton(i, 2), halton(i, 3)) - 0.5;
        Vec2::new(
            pixel.x * 2.0 / self.size.0.max(1) as f32,
            -pixel.y * 2.0 / self.size.1.max(1) as f32,
        )
    }

    /// The frame, blended with its history into the next one; returns the
    /// blended picture.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        current: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        jittered: Mat4,
        previous: Mat4,
    ) -> &wgpu::TextureView {
        let (w, h) = self.size;
        let uniform = TaaUniform {
            inverse_view_projection: jittered.inverse().to_cols_array_2d(),
            previous_view_projection: previous.to_cols_array_2d(),
            size: [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32],
            params: [if self.frames > 0 { 1.0 } else { 0.0 }, 0.1, 0.0, 0.0],
        };
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        let (now, before) = (self.now, 1 - self.now);
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("taa"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(current),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.views[before]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("runity::taa"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.views[now],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: crate::gpu_timer::render("taa"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.now = before;
        self.frames = self.frames.saturating_add(1);
        &self.views[now]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_jitter_stays_inside_a_pixel_and_covers_it() {
        let points: Vec<Vec2> = (1..=8)
            .map(|i| Vec2::new(halton(i, 2), halton(i, 3)))
            .collect();
        assert!(points
            .iter()
            .all(|p| p.x > 0.0 && p.x < 1.0 && p.y > 0.0 && p.y < 1.0));
        // Each quarter of the pixel gets two.
        for (x, y) in [(0.0, 0.0), (0.5, 0.0), (0.0, 0.5), (0.5, 0.5)] {
            let n = points
                .iter()
                .filter(|p| p.x >= x && p.x < x + 0.5 && p.y >= y && p.y < y + 0.5)
                .count();
            assert!(n >= 1, "{points:?}");
        }
    }
}
