//! Several cameras in one picture: split screen (docs/split-screen.md).
//!
//! A [`Frame`] with [`Frame::views`] is drawn once per view — the whole
//! pipeline, lit, shadowed, post-processed, its particles — each at its
//! rect's size and aspect, and laid into its rect of the target. What is
//! the same for every view is done once: the cameras' pictures, the
//! reflection probes, the sky's tables, the live meshes, the lightning.
//! What a view keeps between frames — its antialiasing history, last
//! frame's depth for occlusion culling, its shadow cascades, the eye's
//! exposure clock, its targets — is its own ([`ViewState`]), so two
//! players' pictures never bleed into each other's history. The first
//! view draws with the renderer's own state: a game of one view is drawn
//! as it always was.
//!
//! Not per view yet (shared, so wrong under split screen when on): the
//! upscaler's history (MetalFX temporal, dynamic resolution) and ReSTIR's
//! reservoirs. Volumetric fog keeps no history: its froxels are made
//! again for each view's camera, so it is each view's own. The lamps'
//! shadow maps are drawn again for every view, though they do not depend
//! on it.

use glam::Mat4;

use super::{
    depth_view, scene_targets, CascadeCache, Frame, Renderer, SceneTargets,
};
use crate::gpu::Gpu;

/// One camera's part of the picture.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameView {
    pub camera: super::Camera,
    pub rect: ViewRect,
    /// Drawn in this view only, besides the frame's own draws — Unity's
    /// culling mask, said from the other end: what one player must see
    /// and the other must not. A wall cut down to a stub on the side of
    /// this view's camera while the other view sees it whole, a marker
    /// over one player's own hero. They light, shadow and cull as the
    /// frame's draws do, in this view's pass alone.
    pub draws: Vec<super::Draw>,
}

impl FrameView {
    /// `camera` drawing into `rect`, with nothing of its own.
    pub fn new(camera: super::Camera, rect: ViewRect) -> Self {
        Self { camera, rect, draws: Vec::new() }
    }

    /// With `draws` seen in this view alone.
    pub fn with_draws(mut self, draws: Vec<super::Draw>) -> Self {
        self.draws = draws;
        self
    }
}

/// Where in the target a view is drawn, from its top left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewRect {
    /// Shares of the target's width and height, 0 to 1: the left half is
    /// `Share { x: 0.0, y: 0.0, width: 0.5, height: 1.0 }`.
    Share { x: f32, y: f32, width: f32, height: f32 },
    /// Pixels.
    Pixels { x: u32, y: u32, width: u32, height: u32 },
}

impl ViewRect {
    /// The left half of the target.
    pub const LEFT: ViewRect = ViewRect::Share { x: 0.0, y: 0.0, width: 0.5, height: 1.0 };
    /// The right half.
    pub const RIGHT: ViewRect = ViewRect::Share { x: 0.5, y: 0.0, width: 0.5, height: 1.0 };
    /// The top half.
    pub const TOP: ViewRect = ViewRect::Share { x: 0.0, y: 0.0, width: 1.0, height: 0.5 };
    /// The bottom half.
    pub const BOTTOM: ViewRect = ViewRect::Share { x: 0.0, y: 0.5, width: 1.0, height: 0.5 };

    /// In pixels of a target of `size`: x, y, width, height, cut to the
    /// target and at least a pixel each way. Shares round so that two
    /// halves meet without a gap or an overlap.
    pub fn pixels(&self, size: (u32, u32)) -> (u32, u32, u32, u32) {
        let (w, h) = (size.0.max(1), size.1.max(1));
        let (x0, y0, x1, y1) = match *self {
            ViewRect::Share { x, y, width, height } => {
                let at = |share: f32, of: u32| ((share.clamp(0.0, 1.0) * of as f32).round() as u32).min(of);
                (at(x, w), at(y, h), at(x + width, w), at(y + height, h))
            }
            ViewRect::Pixels { x, y, width, height } => (
                x.min(w),
                y.min(h),
                x.saturating_add(width).min(w),
                y.saturating_add(height).min(h),
            ),
        };
        let x0 = x0.min(w - 1);
        let y0 = y0.min(h - 1);
        (x0, y0, x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1))
    }

    /// Width over height in a target of `size`: the aspect its camera is
    /// drawn with.
    pub fn aspect(&self, size: (u32, u32)) -> f32 {
        let (_, _, w, h) = self.pixels(size);
        w as f32 / h as f32
    }
}

/// What a view after the first keeps between frames, swapped into the
/// renderer while it draws that view.
pub(super) struct ViewState {
    /// The samples per pixel `depth` and `scene` were made with.
    samples: u32,
    depth: wgpu::TextureView,
    depth_size: (u32, u32),
    scene: SceneTargets,
    taa: crate::taa::Taa,
    previous_view_projection: Option<Mat4>,
    occlusion: crate::occlusion::Occlusion,
    cascade_cache: Option<CascadeCache>,
    shadow_turn: bool,
    ssao: crate::ssao::SsaoRenderer,
    metered_at: Option<f32>,
    lowres: crate::lowres::LowRes,
    lens: crate::lens::LensRenderer,
    clouds: crate::clouds::CloudRenderer,
    tools: Option<crate::tools::Tools>,
}

/// The pictures views are drawn into before they are laid into the
/// target, and what lays them.
pub(super) struct Views {
    /// Views after the first: theirs.
    states: Vec<ViewState>,
    /// Each view's picture: the texture, its view and its size.
    pictures: Vec<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    blit: Option<Blit>,
}

impl Views {
    pub(super) fn new() -> Self {
        Self { states: Vec::new(), pictures: Vec::new(), blit: None }
    }
}

struct Blit {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

const BLIT: &str = r#"
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var nearest: sampler;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    return textureSample(picture, nearest, in.uv);
}
"#;

fn blit(gpu: &Gpu, format: wgpu::TextureFormat) -> Blit {
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("views blit"),
        source: wgpu::ShaderSource::Wgsl(BLIT.into()),
    });
    let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("views blit"),
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
    let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("views blit"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("views blit"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
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
    let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("views blit"),
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    Blit { pipeline, layout, sampler }
}

impl Renderer {
    /// A fresh view's state, its targets `size` big.
    fn view_state(&self, gpu: &Gpu, size: (u32, u32)) -> ViewState {
        let mut occlusion = crate::occlusion::Occlusion::new(gpu);
        occlusion.enabled = self.occlusion.enabled;
        let mut lowres = crate::lowres::LowRes::new(gpu);
        lowres.enabled = self.lowres.enabled;
        ViewState {
            samples: self.samples,
            depth: depth_view(gpu, size.0, size.1, self.samples),
            depth_size: size,
            scene: scene_targets(gpu, size.0, size.1, self.samples),
            taa: crate::taa::Taa::new(gpu),
            previous_view_projection: None,
            occlusion,
            cascade_cache: None,
            shadow_turn: false,
            ssao: crate::ssao::SsaoRenderer::new(gpu),
            metered_at: None,
            lowres,
            lens: crate::lens::LensRenderer::new(gpu),
            clouds: crate::clouds::CloudRenderer::new(gpu),
            tools: None,
        }
    }

    /// Trade what a view keeps for what the renderer holds now.
    fn swap_view(&mut self, s: &mut ViewState) {
        use std::mem::swap;
        swap(&mut self.depth, &mut s.depth);
        swap(&mut self.depth_size, &mut s.depth_size);
        swap(&mut self.scene, &mut s.scene);
        swap(&mut self.taa, &mut s.taa);
        swap(&mut self.previous_view_projection, &mut s.previous_view_projection);
        swap(&mut self.occlusion, &mut s.occlusion);
        swap(&mut self.cascade_cache, &mut s.cascade_cache);
        swap(&mut self.shadow_turn, &mut s.shadow_turn);
        swap(&mut self.ssao, &mut s.ssao);
        swap(&mut self.metered_at, &mut s.metered_at);
        swap(&mut self.lowres, &mut s.lowres);
        swap(&mut self.lens, &mut s.lens);
        swap(&mut self.clouds, &mut s.clouds);
        swap(&mut self.tools, &mut s.tools);
        // Targets made at another sample count are made again.
        let held = s.samples;
        s.samples = self.samples;
        if held != self.samples {
            self.depth_size = (0, 0);
        }
    }

    /// The picture view `i` is drawn into, `size` big.
    fn view_picture(&mut self, gpu: &Gpu, i: usize, size: (u32, u32)) -> wgpu::TextureView {
        let pictures = &mut self.views.pictures;
        if pictures.get(i).is_none_or(|p| p.2 != size) {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("view"),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            if i < pictures.len() {
                pictures[i] = (texture, view, size);
            } else {
                pictures.push((texture, view, size));
            }
        }
        pictures[i].1.clone()
    }

    /// Draw each of the frame's views into its rect of `target`, `size`
    /// big: what [`Renderer::render_into`] does for a frame with views.
    pub(super) fn render_views(&mut self, gpu: &Gpu, target: &wgpu::TextureView, size: (u32, u32), frame: &Frame) {
        let mut states = std::mem::take(&mut self.views.states);
        for (i, view) in frame.views.iter().enumerate() {
            let (_, _, w, h) = view.rect.pixels(size);
            let mut drawn = frame.shallow();
            drawn.camera = view.camera;
            drawn.views = Vec::new();
            drawn.draws.extend(view.draws.iter().cloned());
            let picture = self.view_picture(gpu, i, (w, h));
            if i > 0 {
                if states.len() < i {
                    let fresh = self.view_state(gpu, (w, h));
                    states.push(fresh);
                }
                self.swap_view(&mut states[i - 1]);
                self.rebind(gpu);
            }
            let samples = self.samples;
            self.render_view(gpu, Some(&picture), w, h, &drawn, None);
            if i > 0 {
                self.swap_view(&mut states[i - 1]);
                self.rebind(gpu);
            }
            // The sample count changed under this view: every view's
            // targets are made again.
            if self.samples != samples {
                self.depth_size = (0, 0);
                for s in &mut states {
                    s.depth_size = (0, 0);
                }
            }
        }
        // Fewer views than last frame: the ones gone let go.
        states.truncate(frame.views.len().saturating_sub(1));
        self.views.pictures.truncate(frame.views.len());
        self.views.states = states;
        self.lay_views(gpu, target, size, frame);
    }

    /// Lay each view's picture into its rect, what no view covers black.
    fn lay_views(&mut self, gpu: &Gpu, target: &wgpu::TextureView, size: (u32, u32), frame: &Frame) {
        if self.views.blit.is_none() {
            self.views.blit = Some(blit(gpu, self.format));
        }
        let blit = self.views.blit.as_ref().expect("made above");
        let groups: Vec<wgpu::BindGroup> = self
            .views
            .pictures
            .iter()
            .map(|(_, view, _)| {
                gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("views blit"),
                    layout: &blit.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&blit.sampler) },
                    ],
                })
            })
            .collect();
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("views") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("views"),
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
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&blit.pipeline);
            for (view, group) in frame.views.iter().zip(&groups) {
                let (x, y, w, h) = view.rect.pixels(size);
                pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                pass.set_bind_group(0, group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        gpu.queue.submit(Some(encoder.finish()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_halves_meet_without_a_gap_or_an_overlap() {
        for width in [1280, 1281, 7] {
            let (lx, _, lw, lh) = ViewRect::LEFT.pixels((width, 720));
            let (rx, _, rw, _) = ViewRect::RIGHT.pixels((width, 720));
            assert_eq!(lx, 0);
            assert_eq!(lx + lw, rx, "{width}");
            assert_eq!(rx + rw, width);
            assert_eq!(lh, 720);
        }
        assert!((ViewRect::LEFT.aspect((1280, 720)) - 640.0 / 720.0).abs() < 1e-6);
        let cut = ViewRect::Pixels { x: 1200, y: 0, width: 400, height: 100 }.pixels((1280, 720));
        assert_eq!(cut, (1200, 0, 80, 100), "cut to the target");
    }
}
