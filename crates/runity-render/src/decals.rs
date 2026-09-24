//! Decals: URP's Decal Projector — a picture pressed onto whatever lies in
//! a box. A puddle on a road, a crack up a wall, soot round a campfire,
//! a painted sign, without a single extra vertex.
//!
//! An entity's `decal: (size: (2.0, 1.0, 2.0))` is the box, centred on the
//! entity and turned and scaled with it, and the entity's `material` is
//! what is pressed: its base colour and base map (alpha is how much), its
//! normal map, its smoothness. The picture is pressed down the box's −y —
//! a decal on the ground needs no turning, one on a wall is turned to face
//! it — and fades on surfaces that lean away from that, so it does not
//! smear down the sides of what it lands on.
//!
//! Decals are applied in the lit shader, before the light, to the surface
//! itself: what a decal paints is lit, shadowed and reflects as the
//! surface does (URP's DBuffer technique does the same with extra
//! targets). They are clustered with the lights ([`crate::lights`]), so a
//! pixel looks only at the few whose boxes reach its cell. Their pictures
//! are copied into two atlases — colour and normals — of
//! [`ATLAS_LAYERS`] layers each, [`ATLAS_SIZE`] square, made the first time
//! a decal needs one.

use glam::{Mat4, Vec3};

use crate::material::Material;
use crate::render::TextureHandle;

/// A decal as a frame carries it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decal {
    /// The unit box (−0.5 to 0.5 on each axis) into the world: the
    /// entity's placing times the decal's size.
    pub transform: Mat4,
    pub material: Material,
    pub shape: DecalShape,
}

/// What a decal presses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecalShape {
    /// Its material's picture — or its plain colour over the whole box.
    #[default]
    Picture,
    /// A footprint, drawn by the shader ([`crate::footprints`]): a sole
    /// and a heel pressed in the material's `normal_scale` metres, a rim
    /// pushed up round them, the inside the material's colour. Toes to the
    /// box's −z; a box turned over along x is the other foot.
    Footprint,
}

/// The most decals a frame applies; the nearest the camera sees win.
pub const MAX_DECALS: usize = 256;
/// Each picture in the atlas, square.
pub const ATLAS_SIZE: u32 = 512;
/// Pictures each atlas holds.
pub const ATLAS_LAYERS: u32 = 16;
const ATLAS_MIPS: u32 = 10;

/// One decal as the shader reads it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuDecal {
    /// World into the unit box.
    pub world_to_box: [[f32; 4]; 4],
    /// Linear colour and alpha.
    pub color: [f32; 4],
    /// Colour layer, normal layer (−1 for none), normal scale, smoothness.
    pub maps: [f32; 4],
    /// The box's axes in the world, for the normal map's frame: x, and
    /// which way is up (the way the decal is pressed, turned round).
    pub axis_x: [f32; 4],
    pub axis_up: [f32; 4],
}

impl GpuDecal {
    pub(crate) fn new(decal: &Decal, color_layer: Option<u32>, normal_layer: Option<u32>) -> Self {
        let m = &decal.material;
        let linear = |c: f32| crate::material::srgb_to_linear(c.clamp(0.0, 1.0));
        let x = decal.transform.x_axis.truncate().normalize_or(Vec3::X);
        let up = decal.transform.y_axis.truncate().normalize_or(Vec3::Y);
        Self {
            world_to_box: decal.transform.inverse().to_cols_array_2d(),
            color: [
                linear(m.base_color[0]),
                linear(m.base_color[1]),
                linear(m.base_color[2]),
                m.alpha.clamp(0.0, 1.0),
            ],
            maps: [
                match decal.shape {
                    DecalShape::Picture => color_layer.map_or(-1.0, |l| l as f32),
                    DecalShape::Footprint => FOOTPRINT,
                },
                normal_layer.map_or(-1.0, |l| l as f32),
                m.normal_scale,
                m.smoothness.clamp(0.0, 1.0),
            ],
            axis_x: [x.x, x.y, x.z, 0.0],
            axis_up: [up.x, up.y, up.z, 0.0],
        }
    }
}

/// What the shader reads, where a picture's layer would be, as "a
/// footprint".
const FOOTPRINT: f32 = -2.0;

/// Where a decal's box reaches: its centre and the radius of a sphere
/// around it — what the light cells take it by.
pub(crate) fn bounds(decal: &Decal) -> (Vec3, f32) {
    let centre = decal.transform.w_axis.truncate();
    // Half the box's diagonal, however it is turned.
    let radius = 0.5
        * (decal.transform.x_axis.truncate().length_squared()
            + decal.transform.y_axis.truncate().length_squared()
            + decal.transform.z_axis.truncate().length_squared())
        .sqrt();
    (centre, radius)
}

const BLIT: &str = r#"
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

// A picture into an atlas layer, or a mip down. Sampled with the pixel's
// own derivatives, so a picture bigger than the layer is read from its
// matching mip rather than skipped over.
@fragment
fn fs_copy(in: Varyings) -> @location(0) vec4<f32> {
    return textureSample(source, linear_sampler, in.uv);
}
"#;

/// One atlas: its texture, the view the lit shader reads, and which
/// picture is in which layer.
struct Atlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    layers: std::collections::HashMap<TextureHandle, u32>,
}

/// The decals' pictures on the GPU.
pub(crate) struct DecalAtlases {
    colour: Option<Atlas>,
    normal: Option<Atlas>,
    /// One texel, bound until an atlas is made.
    pub(crate) blank: wgpu::TextureView,
    layout: wgpu::BindGroupLayout,
    pipelines: std::collections::HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
}

impl DecalAtlases {
    pub(crate) fn new(gpu: &crate::gpu::Gpu) -> Self {
        let blank = gpu
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("no decals"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("decal copy"),
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
                label: Some("decal copy"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("runity::decals"),
                source: wgpu::ShaderSource::Wgsl(BLIT.into()),
            });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("decal copy"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Self {
            colour: None,
            normal: None,
            blank,
            layout,
            pipelines: std::collections::HashMap::new(),
            shader,
            pipeline_layout,
            sampler,
        }
    }

    pub(crate) fn colour_view(&self) -> &wgpu::TextureView {
        self.colour.as_ref().map_or(&self.blank, |a| &a.view)
    }

    pub(crate) fn normal_view(&self) -> &wgpu::TextureView {
        self.normal.as_ref().map_or(&self.blank, |a| &a.view)
    }

    fn pipeline(&mut self, gpu: &crate::gpu::Gpu, format: wgpu::TextureFormat) {
        if self.pipelines.contains_key(&format) {
            return;
        }
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("decal copy"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs_copy"),
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
        self.pipelines.insert(format, pipeline);
    }

    /// The layer `texture` is in, copied there now if it is not yet —
    /// `None` when the atlas is full. The flag says an atlas was made, so
    /// the frame's bind group must be made again.
    pub(crate) fn layer_for(
        &mut self,
        gpu: &crate::gpu::Gpu,
        texture: TextureHandle,
        source: &wgpu::TextureView,
        normal: bool,
    ) -> (Option<u32>, bool) {
        let format = if normal {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        };
        let slot = if normal {
            &mut self.normal
        } else {
            &mut self.colour
        };
        let mut made = false;
        if slot.is_none() {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(if normal {
                    "decal normals"
                } else {
                    "decal colours"
                }),
                size: wgpu::Extent3d {
                    width: ATLAS_SIZE,
                    height: ATLAS_SIZE,
                    depth_or_array_layers: ATLAS_LAYERS,
                },
                mip_level_count: ATLAS_MIPS,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
            *slot = Some(Atlas {
                texture,
                view,
                layers: std::collections::HashMap::new(),
            });
            made = true;
        }
        let atlas = slot.as_mut().expect("made above");
        if let Some(&layer) = atlas.layers.get(&texture) {
            return (Some(layer), made);
        }
        if atlas.layers.len() as u32 >= ATLAS_LAYERS {
            // Full — most likely of pictures uploaded again under new
            // handles. Start over: the ones still in use come back as they
            // are asked for.
            atlas.layers.clear();
        }
        let layer = atlas.layers.len() as u32;
        atlas.layers.insert(texture, layer);
        let atlas_texture = atlas.texture.clone();
        self.pipeline(gpu, format);
        let pipeline = &self.pipelines[&format];
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("decal picture"),
            });
        let view_of = |mip: u32| {
            atlas_texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                base_mip_level: mip,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        for mip in 0..ATLAS_MIPS {
            let from = if mip == 0 {
                source.clone()
            } else {
                view_of(mip - 1)
            };
            let to = view_of(mip);
            let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("decal copy"),
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
                label: Some("decal picture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &to,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: crate::gpu_timer::render("decal pictures"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit(Some(encoder.finish()));
        (Some(layer), made)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decal_reaches_as_far_as_its_box_corners() {
        let decal = Decal {
            transform: Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0))
                * Mat4::from_scale(Vec3::new(2.0, 1.0, 2.0)),
            material: Material::default(),
            shape: DecalShape::Picture,
        };
        let (centre, radius) = bounds(&decal);
        assert_eq!(centre, Vec3::new(1.0, 0.0, 0.0));
        assert!((radius - 1.5).abs() < 1e-5, "{radius}");
    }

    #[test]
    fn the_shader_frame_turns_the_world_into_the_box() {
        let decal = Decal {
            transform: Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0))
                * Mat4::from_scale(Vec3::new(4.0, 1.0, 4.0)),
            material: Material::default(),
            shape: DecalShape::Picture,
        };
        let gpu = GpuDecal::new(&decal, Some(3), None);
        let into = Mat4::from_cols_array_2d(&gpu.world_to_box);
        let corner = into.transform_point3(Vec3::new(2.0, 2.5, -2.0));
        assert!(
            (corner - Vec3::new(0.5, 0.5, -0.5)).length() < 1e-5,
            "{corner}"
        );
        assert_eq!(gpu.maps[0], 3.0);
        assert_eq!(gpu.maps[1], -1.0);
    }
}
