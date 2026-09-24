//! Screen-space ambient occlusion: URP's SSAO renderer feature.
//!
//! Crevices, corners and the ground under things darken the light that
//! comes from all around — the ambient and the sky's reflection — the way
//! a real one sees less sky. From a depth-and-normals prepass of what is
//! solid; the result is read by the lit shader, which darkens indirect
//! light by it and direct light by a share of it (URP's Direct Lighting
//! Strength). Nothing is baked: it follows whatever moves.
//!
//! The settings are [`AmbientOcclusion`] on a frame, or `ambient_occlusion:
//! (...)` in a scene.

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use crate::gpu::Gpu;

/// How ambient occlusion looks: URP's SSAO settings, with its names.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AmbientOcclusion {
    pub enabled: bool,
    /// How dark a crevice goes: the occlusion is raised to this.
    pub intensity: f32,
    /// How far around a point is looked at, in metres.
    pub radius: f32,
    /// How much of the direct light the occlusion darkens too, 0 to 1:
    /// URP's Direct Lighting Strength.
    pub direct_lighting_strength: f32,
    /// Past this many metres from the eye it fades out.
    pub falloff_distance: f32,
    /// Points looked at per pixel, up to 16: URP's Low (4), Medium (8),
    /// High (12).
    pub samples: u32,
    /// Light bounced off what is near, 0 (off) to 1 and past: HDRP's
    /// Screen Space Global Illumination. A few short rays per pixel across
    /// the screen, and where one meets something, what that was lit as last
    /// frame lights this — a red wall reddens the floor beside it, sunlit
    /// sand warms the rock's shaded side. Off by default.
    pub bounce: f32,
    /// How far a bounce ray is followed, metres.
    pub bounce_radius: f32,
    /// How the occlusion is found: GTAO (the default) or URP's SSAO.
    pub method: Method,
}

/// How ambient occlusion is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Method {
    /// Ground-truth ambient occlusion (Jimenez et al.): along slices across
    /// the screen, the highest the depth rises on either side — the
    /// horizon — and from those and the normal, how much of the sky's
    /// cosine-weighted dome the point sees, worked out exactly. Crevices
    /// darken as much as they are closed, not as a count of hits.
    #[default]
    Gtao,
    /// URP's: points in the hemisphere over the surface, the share that
    /// fall behind what the camera sees.
    Ssao,
}

impl Default for AmbientOcclusion {
    fn default() -> Self {
        Self {
            enabled: true,
            intensity: 1.5,
            radius: 0.5,
            direct_lighting_strength: 0.25,
            falloff_distance: 100.0,
            samples: 8,
            bounce: 0.0,
            bounce_radius: 3.0,
            method: Method::Gtao,
        }
    }
}

impl AmbientOcclusion {
    pub const OFF: AmbientOcclusion = AmbientOcclusion {
        enabled: false,
        intensity: 1.5,
        radius: 0.5,
        direct_lighting_strength: 0.25,
        falloff_distance: 100.0,
        samples: 8,
        bounce: 0.0,
        bounce_radius: 3.0,
        method: Method::Gtao,
    };
}

pub const SHADER: &str = include_str!("ssao.wgsl");

/// Where the prepass writes normals.
pub(crate) const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The prepass's depth: sampled, so single-sample.
pub(crate) const PREPASS_DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// How many screen pixels across one of the occlusion's is: two, a
/// quarter of the pixels, on a screen this tall or more. It changes slowly
/// across a surface, and the blur that brings it back to the whole size
/// keeps its edges to the depth's. On a small picture the blur's box
/// would be most of a corner's darkening, and the saving is little.
const HALF_FROM: u32 = 720;

fn scale_for(size: (u32, u32)) -> u32 {
    if size.1 >= HALF_FROM {
        2
    } else {
        1
    }
}

/// The bounced light in rgb, the occlusion in alpha.
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SsaoUniform {
    view_projection: [[f32; 4]; 4],
    inverse_view_projection: [[f32; 4]; 4],
    eye: [f32; 4],
    params: [f32; 4],
    size: [f32; 4],
    kernel: [[f32; 4]; 16],
    previous_view_projection: [[f32; 4]; 4],
    bounce: [f32; 4],
}

/// Sixteen points in the unit hemisphere over +z, more of them near the
/// middle: the same every frame, so the occlusion does not crawl.
fn kernel() -> [[f32; 4]; 16] {
    let mut out = [[0.0; 4]; 16];
    let mut seed = 0x9e37_79b9u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32) / (u32::MAX as f32)
    };
    for (i, slot) in out.iter_mut().enumerate() {
        // Kept off the tangent plane: a point almost in the surface finds
        // the surface's own neighbouring pixel in front of it.
        let v = Vec3::new(next() * 2.0 - 1.0, next() * 2.0 - 1.0, next().max(0.3)).normalize();
        let scale = i as f32 / 16.0;
        let scale = 0.1 + 0.9 * scale * scale;
        let v = v * next().max(0.2) * scale;
        *slot = [v.x, v.y, v.z, 0.0];
    }
    out
}

fn view(
    gpu: &Gpu,
    label: &str,
    size: (u32, u32),
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    gpu.device
        .create_texture(&wgpu::TextureDescriptor {
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
            // Copied from: the prepass's depth is where the scene's starts.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// The prepass's targets and the occlusion made from them.
pub(crate) struct SsaoRenderer {
    pub(crate) depth: wgpu::TextureView,
    pub(crate) normals: wgpu::TextureView,
    raw: wgpu::TextureView,
    /// What the lit shader reads: the blurred occlusion. (When there is
    /// none, the frame says so and the shader does not look.)
    pub(crate) result: wgpu::TextureView,
    white: wgpu::TextureView,
    pub(crate) size: (u32, u32),
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    occlusion: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
}

impl SsaoRenderer {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("runity::ssao"),
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
                label: Some("ssao"),
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
                    texture(1, wgpu::TextureSampleType::Depth),
                    texture(2, wgpu::TextureSampleType::Float { filterable: false }),
                    texture(3, wgpu::TextureSampleType::Float { filterable: false }),
                    texture(4, wgpu::TextureSampleType::Float { filterable: false }),
                ],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("runity::ssao"),
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
                        targets: &[Some(AO_FORMAT.into())],
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
        };
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ssao"),
            size: std::mem::size_of::<SsaoUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let white = view(gpu, "no occlusion", (1, 1), AO_FORMAT);
        Self {
            depth: view(gpu, "prepass depth", (1, 1), PREPASS_DEPTH),
            normals: view(gpu, "prepass normals", (1, 1), NORMAL_FORMAT),
            raw: view(gpu, "occlusion", (1, 1), AO_FORMAT),
            result: view(gpu, "occlusion (blurred)", (1, 1), AO_FORMAT),
            white,
            size: (1, 1),
            layout,
            uniform,
            occlusion: pipeline("fs_occlusion"),
            blur: pipeline("fs_blur"),
        }
    }

    /// Sized to the frame. `true` when the targets were remade, so what
    /// binds them must be too.
    pub(crate) fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) -> bool {
        if self.size == size {
            return false;
        }
        self.size = size;
        self.depth = view(gpu, "prepass depth", size, PREPASS_DEPTH);
        self.normals = view(gpu, "prepass normals", size, NORMAL_FORMAT);
        // Found at half the size across on a big screen, a pixel for each
        // two by two; the blur brings it back to the whole.
        let scale = scale_for(size);
        self.raw = view(gpu, "occlusion", (size.0.div_ceil(scale), size.1.div_ceil(scale)), AO_FORMAT);
        self.result = view(gpu, "occlusion (blurred)", size, AO_FORMAT);
        true
    }

    /// The occlusion, from the prepass already drawn, then blurred; and,
    /// given the last frame and the camera that saw it, the light bounced.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        view_projection: Mat4,
        previous_view_projection: Mat4,
        eye: Vec3,
        settings: &AmbientOcclusion,
        last_frame: Option<&wgpu::TextureView>,
        turn: Option<f32>,
    ) {
        let (w, h) = self.size;
        let uniform = SsaoUniform {
            view_projection: view_projection.to_cols_array_2d(),
            inverse_view_projection: view_projection.inverse().to_cols_array_2d(),
            // w: how far the bounce's rays turn this frame, above 0 when
            // they turn at all (and half of them are cast).
            eye: [eye.x, eye.y, eye.z, turn.map_or(0.0, |t| t.max(1e-3))],
            params: [
                settings.radius.max(0.01),
                settings.intensity.max(0.0),
                settings.falloff_distance.max(0.1),
                settings.samples.clamp(1, 16) as f32,
            ],
            size: [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32],
            kernel: kernel(),
            previous_view_projection: previous_view_projection.to_cols_array_2d(),
            bounce: [
                if last_frame.is_some() {
                    settings.bounce.max(0.0)
                } else {
                    0.0
                },
                settings.bounce_radius.max(0.1),
                if settings.method == Method::Gtao { 1.0 } else { 0.0 },
                // The occlusion's pixel is this many of the screen's across.
                scale_for(self.size) as f32,
            ],
        };
        let last_frame = last_frame.unwrap_or(&self.white);
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        let group = |source: &wgpu::TextureView| {
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ssao"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&self.normals),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(last_frame),
                    },
                ],
            })
        };
        // The occlusion reads `white` as its unused source; the blur reads
        // the occlusion.
        for (pipeline, source, target) in [
            (&self.occlusion, &self.white, &self.raw),
            (&self.blur, &self.raw, &self.result),
        ] {
            let bind = group(source);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("runity::ssao"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: crate::gpu_timer::render("ssao"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kernel_is_a_hemisphere_crowded_to_the_middle() {
        let k = kernel();
        assert!(k.iter().all(|v| v[2] > 0.0), "all above the surface");
        let near: f32 = k[..4]
            .iter()
            .map(|v| Vec3::new(v[0], v[1], v[2]).length())
            .sum();
        let far: f32 = k[12..]
            .iter()
            .map(|v| Vec3::new(v[0], v[1], v[2]).length())
            .sum();
        assert!(near < far, "the first ones close in: {near} vs {far}");
        assert!(k
            .iter()
            .all(|v| Vec3::new(v[0], v[1], v[2]).length() <= 1.0));
    }
}
