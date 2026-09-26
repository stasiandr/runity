//! Reflection probes: URP's baked Reflection Probe, so polished metal and
//! wet stone reflect the room they are in rather than the sky.
//!
//! A probe is a box in the scene — an entity's `reflection_probe: (size:
//! (8.0, 4.0, 8.0))` — and a picture of everything around its centre, six
//! faces of a cube, taken with the renderer itself: lit, shadowed, with its
//! sky. What stands in the box reflects that picture instead of the sky's
//! gradient; with `box_projection` (on, as URP's) the reflection is bent to
//! where the box's walls are, so a floor's reflection of a wall meets the
//! wall. Near the box's edge it fades, over `blend_distance`, into the
//! next probe or the sky.
//!
//! The picture has mips, each a blur of the one above, and a rougher
//! surface reads a blurrier one — a mirror the sharp picture, brushed
//! steel a smear of it. Baked when the probes change (placed, moved,
//! resized), not every frame: what moves in front of a probe afterwards is
//! not in it, as with URP's baked ones; [`crate::Renderer::rebake_reflections`]
//! takes the pictures again.
//!
//! The faces are kept as six layers of an array rather than a cube map: a
//! cube map's faces are mirrored, and drawing them mirrored would turn
//! every triangle's winding. Each face is taken a shade wider than a right
//! angle, so the filter at a face's edge still reads what lies past it.

use glam::{Mat4, Vec3};

/// A probe: where, how big, and how its reflection is bent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReflectionProbe {
    pub position: Vec3,
    /// Half the box's size along each axis.
    pub extents: Vec3,
    /// Bend reflections to the box's walls.
    pub box_projection: bool,
    /// Metres inside the box's edge over which it fades out.
    pub blend_distance: f32,
}

/// The most probes a frame reads.
pub const MAX_PROBES: usize = 8;
/// A face's side, in pixels.
pub const PROBE_SIZE: u32 = 128;
/// Mips, 128 down to 4.
pub const PROBE_MIPS: u32 = 6;
/// Each face's half-width, as a tangent: a shade past 45°.
pub(crate) const FACE_SPREAD: f32 = 1.1;
pub(crate) const PROBE_NEAR: f32 = 0.05;
/// How far a probe sees.
pub(crate) const PROBE_FAR: f32 = 500.0;

/// The six ways a probe looks, and which way is up for each: +x, −x, +y,
/// −y, +z, −z — the order the shader picks them in, by the largest axis.
pub(crate) const FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::Z),
    (Vec3::Z, Vec3::Y),
    (Vec3::NEG_Z, Vec3::Y),
];

pub(crate) fn face_fov_degrees() -> f32 {
    (2.0 * FACE_SPREAD.atan()).to_degrees()
}

/// A face's projection of a direction: what the shader multiplies a
/// reflected ray by to find where on that face it lands.
pub(crate) fn face_matrix(face: usize) -> Mat4 {
    let (forward, up) = FACES[face];
    Mat4::perspective_rh(face_fov_degrees().to_radians(), 1.0, PROBE_NEAR, PROBE_FAR)
        * Mat4::look_at_rh(Vec3::ZERO, forward, up)
}

/// The camera that takes a probe's face.
pub(crate) fn face_camera(probe: &ReflectionProbe, face: usize) -> crate::render::Camera {
    let (forward, up) = FACES[face];
    crate::render::Camera {
        position: probe.position,
        target: probe.position + forward,
        up,
        fov_y_degrees: face_fov_degrees(),
        near: PROBE_NEAR,
        far: PROBE_FAR,
        ortho: None,
        clip: None,
    }
}

const SHADER: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> Varyings {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var out: Varyings;
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

// A mip down, a little wider than a plain 2x2: four bilinear taps a texel
// out, so each step blurs as a rougher surface wants.
@fragment
fn fs_down(in: Varyings) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(source));
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 4; i = i + 1) {
        let o = vec2<f32>(f32(i & 1) * 2.0 - 1.0, f32(i >> 1u) * 2.0 - 1.0) * texel;
        sum = sum + textureSampleLevel(source, linear_sampler, in.uv + o, 0.0).rgb;
    }
    return vec4<f32>(sum * 0.25, 1.0);
}
"#;

/// The probes' pictures on the GPU, and what makes their mips.
pub(crate) struct ProbeStore {
    texture: wgpu::Texture,
    /// All of them, every mip: what the lit shader reads.
    pub(crate) view: wgpu::TextureView,
    pub(crate) sampler: wgpu::Sampler,
    layout: wgpu::BindGroupLayout,
    down: wgpu::RenderPipeline,
    /// The probes the pictures are of.
    pub(crate) baked: Vec<ReflectionProbe>,
    /// One black texel, bound while the pictures are being taken: a pass
    /// cannot read the texture it draws into.
    pub(crate) blank: wgpu::TextureView,
    /// Taking the pictures now.
    pub(crate) baking: bool,
    /// The last round's pictures, what the next round is lit by: a pass
    /// cannot read the texture it draws into.
    previous: wgpu::Texture,
    pub(crate) previous_view: wgpu::TextureView,
    /// Taking a later round, lit by the last one's pictures.
    pub(crate) bouncing: bool,
}

/// Rounds of pictures a bake takes. The first is lit as if no probe were
/// there — a room's walls by the whole open sky; each after by the one
/// before, so what the sky reaches only through a door or down a passage
/// dims round after round toward how dark it is.
pub const BOUNCES: u32 = 3;

impl ProbeStore {
    pub(crate) fn new(gpu: &crate::gpu::Gpu) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reflection probes"),
            size: wgpu::Extent3d {
                width: PROBE_SIZE,
                height: PROBE_SIZE,
                depth_or_array_layers: (MAX_PROBES * 6) as u32,
            },
            mip_level_count: PROBE_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::post::HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let previous = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reflection probes, last round"),
            size: wgpu::Extent3d {
                width: PROBE_SIZE,
                height: PROBE_SIZE,
                depth_or_array_layers: (MAX_PROBES * 6) as u32,
            },
            mip_level_count: PROBE_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::post::HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let previous_view = previous.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("reflection probes"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            // Decals read through it too, often at a grazing angle.
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("scrap::reflections"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("probe mip"),
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
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("probe mip"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let down = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("probe mip"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_down"),
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
        let blank = gpu
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("no reflection probes"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: crate::post::HDR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        Self {
            texture,
            view,
            sampler,
            layout,
            down,
            baked: Vec::new(),
            blank,
            baking: false,
            previous,
            previous_view,
            bouncing: false,
        }
    }

    /// One layer at one mip, to draw into or read from.
    pub(crate) fn layer(&self, layer: u32, mip: u32) -> wgpu::TextureView {
        self.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("probe face"),
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: layer,
            array_layer_count: Some(1),
            base_mip_level: mip,
            mip_level_count: Some(1),
            ..Default::default()
        })
    }

    /// Each face's mips, from its sharp picture down.
    /// Keep this round's pictures, every mip, for the next to be lit by.
    pub(crate) fn keep_round(&self, gpu: &crate::gpu::Gpu, layers: u32) {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("probe round"),
            });
        for mip in 0..PROBE_MIPS {
            let side = (PROBE_SIZE >> mip).max(1);
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.previous,
                    mip_level: mip,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: layers,
                },
            );
        }
        gpu.queue.submit(Some(encoder.finish()));
    }

    pub(crate) fn make_mips(&self, gpu: &crate::gpu::Gpu, layers: std::ops::Range<u32>) {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("probe mips"),
            });
        for layer in layers {
            for mip in 1..PROBE_MIPS {
                let from = self.layer(layer, mip - 1);
                let to = self.layer(layer, mip);
                let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("probe mip"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&from),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("probe mip"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &to,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: crate::gpu_timer::render("probe mips"),
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.down);
                pass.set_bind_group(0, &group, &[]);
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
    fn each_face_sees_its_own_axis_in_its_middle() {
        for (face, (forward, _)) in FACES.iter().enumerate() {
            let clip = face_matrix(face) * forward.extend(1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(
                ndc.x.abs() < 1e-5 && ndc.y.abs() < 1e-5,
                "face {face}: {ndc}"
            );
        }
    }

    #[test]
    fn a_face_reaches_a_little_past_its_edge() {
        // Straight at the +x face's edge, 45° towards +z: still inside it.
        let clip = face_matrix(0) * Vec3::new(1.0, 0.0, 1.0).normalize().extend(1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.x.abs() < 1.0 && ndc.x.abs() > 0.85, "{ndc}");
    }
}

/// Screen-space reflections: URP's own is absent, HDRP's SSR is the model.
///
/// A smooth surface's reflection is marched across the screen along the
/// prepass's depth; where it meets something, the colour there is taken
/// from the last frame (moved to where it was then), so reflections see
/// reflections. What the march misses — off the screen, behind something —
/// falls back to the probes and the sky. The water especially gains: the
/// bank and the trees on it, exactly, where a probe only has them roughly.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ScreenSpaceReflections {
    pub enabled: bool,
    /// How far a reflected ray is followed, metres.
    pub max_distance: f32,
    /// How thick what the ray meets is taken to be, metres: too thin and
    /// rays slip behind things, too thick and they hit what they pass.
    pub thickness: f32,
    /// Steps along the ray.
    pub steps: u32,
}

impl Default for ScreenSpaceReflections {
    fn default() -> Self {
        Self {
            enabled: false,
            max_distance: 30.0,
            thickness: 0.4,
            steps: 32,
        }
    }
}
