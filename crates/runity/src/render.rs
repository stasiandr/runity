//! Drawing, with no `wgpu` in any signature.
//!
//! Callers hand over meshes, a camera and a list of things to draw, and get a
//! frame back. Nothing outside this module names a wgpu type, which is the
//! rule that keeps two doors open: wgpu breaks its API every release, and a
//! hand-written Vulkan backend may eventually be worth it. Both should be an
//! internal swap, not a rewrite of the game.
//!
//! The lighting is the one `docs/design/07-look.md` asks for and nothing
//! more: one directional sun, flat shading, colour instead of material. Two
//! things the old renderer lacked and this one has from the start, because
//! their absence was most of why frames looked wrong:
//!
//! * **Hemisphere ambient.** A constant ambient term gives every surface
//!   turned away from the sun the same dead colour. Splitting it into a sky
//!   colour above and a ground colour below costs one `mix` and makes a
//!   shaded side read as shaded rather than as painted.
//! * **Distance fog, on by default.** Without it a near trunk and a far one
//!   are the same value, and a forest reads as a flat wall of brown.
//!
//! Shadows are not here yet, and they are the next thing that matters.

use glam::{Mat4, Vec3};

use crate::asset::{ArchivedMeshAsset, ArchivedTextureAsset};
use crate::gpu::{Gpu, OffscreenTarget};
use crate::material::{Material, Shading};

/// A mesh that lives on the GPU. Opaque on purpose — the index is ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshHandle(u32);

/// An image that lives on the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureHandle(u32);

impl TextureHandle {
    /// The one every renderer has: a single white pixel.
    ///
    /// It exists so the shader always samples something and never needs a
    /// branch, and so an untextured material is a colour times one rather
    /// than a second pipeline.
    pub const WHITE: TextureHandle = TextureHandle(0);
}

impl MeshHandle {
    /// A handle that refers to nothing, for tests that build a draw list
    /// without a device. Rendering with it draws nothing rather than
    /// panicking, which is also what a handle from an unloaded asset does.
    pub const TEST: MeshHandle = MeshHandle(u32::MAX);
}

/// Where the eye is and what it can see.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fov_y_degrees: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 2.0, 6.0),
            target: Vec3::new(0.0, 1.0, 0.0),
            up: Vec3::Y,
            fov_y_degrees: 55.0,
            near: 0.1,
            far: 500.0,
        }
    }
}

impl Camera {
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        // `perspective_rh` and not `_gl`: wgpu's clip space runs z from 0 to
        // 1, and the `_gl` variant's -1..1 would halve the depth buffer.
        let projection = Mat4::perspective_rh(
            self.fov_y_degrees.to_radians(),
            aspect.max(1e-3),
            self.near,
            self.far,
        );
        projection * Mat4::look_at_rh(self.position, self.target, self.up)
    }
}

/// The sun, and the sky it hangs in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lighting {
    /// Direction the light travels, i.e. from the sun toward the ground.
    pub sun_direction: Vec3,
    pub sun_color: Vec3,
    pub sun_intensity: f32,
    /// Ambient seen by a surface facing straight up.
    pub sky_color: Vec3,
    /// Ambient seen by a surface facing straight down — bounce off the
    /// ground, standing in for the global illumination we do not compute.
    pub ground_color: Vec3,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            sun_direction: Vec3::new(-0.35, -0.85, -0.4).normalize(),
            sun_color: Vec3::new(1.0, 0.96, 0.88),
            sun_intensity: 1.15,
            sky_color: Vec3::new(0.24, 0.28, 0.34),
            ground_color: Vec3::new(0.10, 0.09, 0.07),
        }
    }
}

/// Linear distance fog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogSettings {
    pub color: Vec3,
    pub start: f32,
    pub end: f32,
}

impl Default for FogSettings {
    fn default() -> Self {
        Self {
            color: Vec3::new(0.62, 0.68, 0.74),
            start: 30.0,
            end: 180.0,
        }
    }
}

/// What one frame cost, in things drawn and things skipped.
///
/// Worth reporting rather than merely doing: culling that silently removes
/// something you meant to see is indistinguishable from a bug in placement,
/// and the count is the first thing to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameStats {
    /// Draws the frame asked for.
    pub submitted: u32,
    /// Draws that reached the colour pass.
    pub drawn: u32,
    /// Draws outside the camera's frustum.
    pub culled: u32,
    /// Draws in the shadow pass, which is not culled by the camera: a
    /// caster behind you still throws a shadow in front of you.
    pub shadow_casters: u32,
}

/// How shadows are cast, or that they are not.
///
/// A shadow map is the cheapest way to make something touch the ground, and
/// nothing else in a renderer does as much for how a frame reads. Everything
/// here is a knob because the right value depends on the scene's size: a
/// bias that works over a hundred metres stripes a room.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSettings {
    pub enabled: bool,
    /// Side of the square depth map. 2048 is enough for a valley; 1024 for a
    /// clearing; more costs memory and fill, not detail, once the map is
    /// finer than the screen.
    pub resolution: u32,
    /// Pushes the comparison away from the surface to stop it shadowing
    /// itself. Too little gives acne, too much makes shadows float free of
    /// what casts them ("peter-panning").
    pub depth_bias: f32,
    /// Extra offset along the surface normal, in world units. Handles the
    /// grazing angles a constant bias cannot, because the error there grows
    /// with the slope rather than with depth.
    pub normal_bias: f32,
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            resolution: 2048,
            depth_bias: 0.0015,
            normal_bias: 0.05,
        }
    }
}

impl ShadowSettings {
    pub const OFF: ShadowSettings = ShadowSettings {
        enabled: false,
        resolution: 1,
        depth_bias: 0.0,
        normal_bias: 0.0,
    };
}

/// One thing to draw: a mesh, where it is, and what colour it takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Draw {
    pub mesh: MeshHandle,
    pub transform: Mat4,
    /// Multiplied by the material's colour. [`TextureHandle::WHITE`] leaves
    /// the colour alone, which is what an untextured surface wants.
    pub texture: TextureHandle,
    /// What the surface is made of. One mesh drawn with four materials is
    /// how four settlers get four shirts (`07-look.md`, "тинт инстанса").
    pub material: Material,
}

/// Everything needed to produce one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub camera: Camera,
    pub lighting: Lighting,
    pub fog: FogSettings,
    pub shadows: ShadowSettings,
    pub clear_color: Vec3,
    pub draws: Vec<Draw>,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            camera: Camera::default(),
            lighting: Lighting::default(),
            fog: FogSettings::default(),
            shadows: ShadowSettings::default(),
            clear_color: Vec3::new(0.62, 0.68, 0.74),
            draws: Vec::new(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniform {
    view_projection: [[f32; 4]; 4],
    sun_direction: [f32; 4],
    sun_color: [f32; 4],
    sky_color: [f32; 4],
    ground_color: [f32; 4],
    fog_color: [f32; 4],
    /// `start`, `end`, and two words of padding: a uniform buffer's members
    /// are 16-byte aligned, and naming the padding is cheaper than debugging
    /// a silently shifted field.
    fog_range: [f32; 4],
    camera_position: [f32; 4],
    /// World space to the shadow map's clip space.
    light_view_projection: [[f32; 4]; 4],
    /// `depth_bias`, `normal_bias`, texel size in world units, and `1.0` when
    /// shadows are on. The last one is what lets the shader skip the lookup
    /// without a second pipeline.
    shadow_params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InstanceRaw {
    model: [[f32; 4]; 4],
    /// rgb is the base colour; w is 1.0 for an unlit surface, which the
    /// shader uses to skip both the light and the fog.
    color_and_shading: [f32; 4],
}

struct GpuTexture {
    bind_group: wgpu::BindGroup,
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Kept so the shadow pass can fit its frustum to what is actually being
    /// drawn. A map spread over an empty hundred metres wastes every texel.
    bounds: crate::asset::Bounds,
}

/// Holds the pipeline, the uploaded meshes and the buffers a frame needs.
pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    /// The uniform alone. The shadow pass writes the map it is drawing into,
    /// so it cannot bind a group that also samples it — wgpu rejects a
    /// texture used as attachment and resource in one pass, and it is right
    /// to.
    shadow_bind_group: wgpu::BindGroup,
    frame_buffer: wgpu::Buffer,
    instances: wgpu::Buffer,
    instance_capacity: u64,
    depth: wgpu::TextureView,
    depth_size: (u32, u32),
    shadow_map: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    shadow_resolution: u32,
    format: wgpu::TextureFormat,
    stats: FrameStats,
    meshes: Vec<GpuMesh>,
    textures: Vec<GpuTexture>,
    texture_layout: wgpu::BindGroupLayout,
    texture_sampler: wgpu::Sampler,
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

impl Renderer {
    /// Build a renderer for an offscreen target.
    pub fn new(gpu: &Gpu, target: &OffscreenTarget) -> Self {
        Self::with_format(gpu, target.format, target.width, target.height)
    }

    /// Build a renderer for a window's surface.
    pub fn for_surface(gpu: &Gpu, surface: &crate::surface::Surface) -> Self {
        Self::with_format(gpu, surface.format(), surface.width(), surface.height())
    }

    pub(crate) fn with_format(
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("runity::render"),
                source: wgpu::ShaderSource::Wgsl(include_str!("render.wgsl").into()),
            });

        let frame_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("frame"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        // A comparison sampler, not an ordinary one: the
                        // hardware does the depth test and the filtering
                        // together, so a single fetch is already a 2x2 PCF
                        // and the edge comes out soft for free.
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    },
                ],
            });
        let shadow_resolution = ShadowSettings::default().resolution;
        let shadow_map = shadow_view(gpu, shadow_resolution);
        let shadow_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let bind_group =
            frame_bind_group(gpu, &layout, &frame_buffer, &shadow_map, &shadow_sampler);

        let texture_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("surface texture"),
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
        let texture_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("surface"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("runity::render"),
                bind_group_layouts: &[Some(&layout), Some(&texture_layout)],
                immediate_size: 0,
            });

        let shadow_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shadow frame"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let shadow_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow frame"),
            layout: &shadow_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });
        let shadow_pipeline_layout =
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("runity::shadow"),
                    bind_group_layouts: &[Some(&shadow_layout)],
                    immediate_size: 0,
                });

        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("runity::render"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<crate::asset::Vertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![
                                0 => Float32x3, 1 => Float32x3, 2 => Float32x2
                            ],
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<InstanceRaw>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &wgpu::vertex_attr_array![
                                3 => Float32x4, 4 => Float32x4, 5 => Float32x4,
                                6 => Float32x4, 7 => Float32x4
                            ],
                        }),
                    ],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: wgpu::PrimitiveState {
                    // Back faces are dropped, which is why the importer cares
                    // about winding: a model wound inside out disappears.
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        // Depth only: no fragment stage at all, because nothing is written
        // but depth and a colour target would only cost fill.
        let shadow_pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("runity::shadow"),
                layout: Some(&shadow_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_shadow"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<crate::asset::Vertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![
                                0 => Float32x3, 1 => Float32x3, 2 => Float32x2
                            ],
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<InstanceRaw>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &wgpu::vertex_attr_array![
                                3 => Float32x4, 4 => Float32x4, 5 => Float32x4,
                                6 => Float32x4, 7 => Float32x4
                            ],
                        }),
                    ],
                },
                fragment: None,
                primitive: wgpu::PrimitiveState {
                    // Front faces are culled here, not back ones. Drawing
                    // only the far side of an object into the map moves the
                    // self-shadowing error behind the surface that would
                    // have shown it, which removes most acne before any bias
                    // is applied.
                    cull_mode: Some(wgpu::Face::Front),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 2,
                        slope_scale: 2.0,
                        clamp: 0.0,
                    },
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let instance_capacity = 256;
        let instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: instance_capacity * std::mem::size_of::<InstanceRaw>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut renderer = Self {
            pipeline,
            shadow_pipeline,
            layout,
            bind_group,
            shadow_bind_group,
            frame_buffer,
            instances,
            instance_capacity,
            depth: depth_view(gpu, width, height),
            depth_size: (width, height),
            shadow_map,
            shadow_sampler,
            shadow_resolution,
            format,
            stats: FrameStats::default(),
            meshes: Vec::new(),
            textures: Vec::new(),
            texture_layout,
            texture_sampler,
        };

        // Handle 0 is always the white pixel, so `TextureHandle::WHITE` is a
        // constant rather than something every caller has to be handed.
        renderer.upload_texture_rgba(gpu, 1, 1, &[255, 255, 255, 255], true);
        renderer
    }

    /// Upload a mesh straight out of an imported asset.
    ///
    /// The archived vertices are already the layout the vertex buffer wants,
    /// so this is a copy, not a conversion — which is the whole reason the
    /// asset format exists.
    pub fn upload_mesh(&mut self, gpu: &Gpu, mesh: &ArchivedMeshAsset) -> MeshHandle {
        let indices: Vec<u32> = mesh.indices.iter().map(|i| i.to_native()).collect();
        self.upload(gpu, vertex_slice(mesh), &indices)
    }

    fn upload(
        &mut self,
        gpu: &Gpu,
        vertices: &[crate::asset::Vertex],
        indices: &[u32],
    ) -> MeshHandle {
        let bounds = crate::asset::Bounds::of(vertices);
        use wgpu::util::DeviceExt;

        let vertex_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vertices"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("indices"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            });

        self.meshes.push(GpuMesh {
            vertices: vertex_buffer,
            indices: index_buffer,
            index_count: indices.len() as u32,
            bounds,
        });
        MeshHandle(self.meshes.len() as u32 - 1)
    }

    /// Upload an image straight out of an imported asset.
    pub fn upload_texture(&mut self, gpu: &Gpu, texture: &ArchivedTextureAsset) -> TextureHandle {
        let mut levels: Vec<(u32, u32, &[u8])> = vec![(
            texture.width.to_native(),
            texture.height.to_native(),
            texture.pixels.as_slice(),
        )];
        for mip in texture.mips.iter() {
            levels.push((
                mip.width.to_native(),
                mip.height.to_native(),
                mip.pixels.as_slice(),
            ));
        }
        self.upload_texture_levels(gpu, &levels, texture.srgb)
    }

    /// Upload raw RGBA8 pixels.
    ///
    /// `srgb` says whether the bytes are colour. A colour map is sRGB and has
    /// to be decoded on the way in so that shading stays linear; a normal
    /// map, a roughness map or a mask is not, and decoding one bends every
    /// value in it.
    pub fn upload_texture_rgba(
        &mut self,
        gpu: &Gpu,
        width: u32,
        height: u32,
        pixels: &[u8],
        srgb: bool,
    ) -> TextureHandle {
        self.upload_texture_levels(gpu, &[(width, height, pixels)], srgb)
    }

    /// Upload a whole mip chain. Level 0 first, each half the last.
    pub fn upload_texture_levels(
        &mut self,
        gpu: &Gpu,
        levels: &[(u32, u32, &[u8])],
        srgb: bool,
    ) -> TextureHandle {
        let Some(&(width, height, _)) = levels.first() else {
            return TextureHandle::WHITE;
        };
        let format = if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let size = wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        };
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("surface texture"),
            size,
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (w, h, pixels)) in levels.iter().enumerate() {
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(*h),
                },
                wgpu::Extent3d {
                    width: *w,
                    height: *h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("surface texture"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.texture_sampler),
                },
            ],
        });
        self.textures.push(GpuTexture { bind_group });
        TextureHandle(self.textures.len() as u32 - 1)
    }

    /// Upload a mesh that is already in memory rather than in an asset.
    ///
    /// The builtins come this way. Everything else should go through the
    /// asset pipeline, which is why this is separate rather than the only
    /// entry point: an import is a decision, and making it as easy to skip
    /// as to do is how a codebase ends up parsing OBJ at startup again.
    pub fn upload_mesh_owned(&mut self, gpu: &Gpu, mesh: &crate::asset::MeshAsset) -> MeshHandle {
        self.upload(gpu, &mesh.vertices, &mesh.indices)
    }

    /// Issue the grouped draws. Shared by both passes so that what casts a
    /// shadow and what is drawn can never drift apart — the commonest way a
    /// shadow ends up belonging to nothing.
    fn draw_batches<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        batches: &[((MeshHandle, TextureHandle), Vec<InstanceRaw>)],
        base: u32,
        textured: bool,
    ) {
        let mut first = base;
        for ((handle, texture), list) in batches {
            let Some(mesh) = self.meshes.get(handle.0 as usize) else {
                continue;
            };
            if textured {
                // Falls back to white rather than skipping the draw: a
                // missing texture should leave a flat-coloured object, not a
                // hole where one used to be.
                let bound = self
                    .textures
                    .get(texture.0 as usize)
                    .or_else(|| self.textures.first());
                if let Some(bound) = bound {
                    pass.set_bind_group(1, &bound.bind_group, &[]);
                }
            }
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_vertex_buffer(1, self.instances.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            let count = list.len() as u32;
            pass.draw_indexed(0..mesh.index_count, 0, first..first + count);
            first += count;
        }
    }

    /// The world-space box around everything being drawn.
    fn scene_bounds(&self, frame: &Frame) -> Option<(Vec3, Vec3)> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        for draw in &frame.draws {
            let Some(mesh) = self.meshes.get(draw.mesh.0 as usize) else {
                continue;
            };
            let (lo, hi) = (
                Vec3::from_array(mesh.bounds.min),
                Vec3::from_array(mesh.bounds.max),
            );
            // All eight corners, because a rotation turns the box and taking
            // only two of them would clip whatever swung outside.
            for i in 0..8 {
                let corner = Vec3::new(
                    if i & 1 == 0 { lo.x } else { hi.x },
                    if i & 2 == 0 { lo.y } else { hi.y },
                    if i & 4 == 0 { lo.z } else { hi.z },
                );
                let world = draw.transform.transform_point3(corner);
                min = min.min(world);
                max = max.max(world);
                any = true;
            }
        }
        any.then_some((min, max))
    }

    /// An orthographic frustum along the sun, fitted to the scene.
    ///
    /// Fitted rather than fixed because the map has a texel budget and
    /// spreading it over empty ground is how shadows turn to stairs.
    fn fit_light_frustum(&self, frame: &Frame, sun: Vec3) -> Mat4 {
        let Some((min, max)) = self.scene_bounds(frame) else {
            return Mat4::IDENTITY;
        };
        let centre = (min + max) * 0.5;
        let radius = (max - min).length() * 0.5;
        if radius <= f32::EPSILON || sun.length_squared() < 0.5 {
            return Mat4::IDENTITY;
        }

        // A sun straight overhead makes the usual up vector degenerate.
        let up = if sun.dot(Vec3::Y).abs() > 0.99 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let eye = centre - sun * radius * 2.0;
        let view = Mat4::look_at_rh(eye, centre, up);
        // The near plane sits behind the scene so that a caster outside the
        // camera's view still lands in the map; something off screen can
        // easily be the thing casting the shadow you are looking at.
        let projection =
            Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.01, radius * 4.0);
        projection * view
    }

    /// How much world a single shadow texel covers.
    fn light_texel_size(&self, frame: &Frame, sun: Vec3) -> f32 {
        match self.scene_bounds(frame) {
            Some((min, max)) if sun.length_squared() > 0.5 => {
                let radius = (max - min).length() * 0.5;
                2.0 * radius / self.shadow_resolution.max(1) as f32
            }
            _ => 0.0,
        }
    }

    /// Draw one frame into a window's surface.
    ///
    /// Returns [`SurfaceError::Outdated`] when the swapchain needs
    /// rebuilding, which is routine: a resize, a move between monitors, a
    /// display waking up. The caller reconfigures and draws again.
    pub fn render_to_surface(
        &mut self,
        gpu: &Gpu,
        surface: &crate::surface::Surface,
        frame: &Frame,
    ) -> Result<(), crate::surface::SurfaceError> {
        if !self.format_matches(surface.format()) {
            // A renderer is built for one pixel format and cannot draw into
            // another. Saying so here beats the backend's version of this
            // error, which names neither format.
            return Err(crate::surface::SurfaceError::Other(format!(
                "renderer built for {:?}, surface is {:?}",
                self.format,
                surface.format()
            )));
        }
        let (width, height) = (surface.width(), surface.height());
        if self.depth_size != (width, height) {
            self.depth = depth_view(gpu, width, height);
            self.depth_size = (width, height);
        }
        let acquired = surface.begin_frame()?;
        self.render_to_frame(gpu, &acquired, frame);
        acquired.present(gpu);
        Ok(())
    }

    /// What the last frame cost.
    pub fn stats(&self) -> FrameStats {
        self.stats
    }

    /// Draw into a frame someone else acquired, leaving it unpresented.
    ///
    /// This is the one to use when an overlay follows: the caller holds the
    /// frame, draws the scene, draws the UI over it, and presents once.
    pub fn render_to_frame(
        &mut self,
        gpu: &Gpu,
        acquired: &crate::surface::AcquiredFrame,
        frame: &Frame,
    ) {
        let (width, height) = (acquired.width, acquired.height);
        if self.depth_size != (width, height) {
            self.depth = depth_view(gpu, width, height);
            self.depth_size = (width, height);
        }
        self.render_into(gpu, &acquired.view, width, height, frame);
    }

    /// Whether this renderer's pipeline matches a target's pixel format.
    ///
    /// A renderer built for one format cannot draw into another, and the
    /// error a backend gives for that names neither of them.
    pub(crate) fn format_matches(&self, format: wgpu::TextureFormat) -> bool {
        self.format == format
    }

    /// Draw one frame into an offscreen target.
    pub fn render(&mut self, gpu: &Gpu, target: &OffscreenTarget, frame: &Frame) {
        if self.depth_size != (target.width, target.height) {
            self.depth = depth_view(gpu, target.width, target.height);
            self.depth_size = (target.width, target.height);
        }
        self.render_into(gpu, &target.view, target.width, target.height, frame);
    }

    pub(crate) fn render_into(
        &mut self,
        gpu: &Gpu,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        frame: &Frame,
    ) {
        let aspect = width as f32 / height.max(1) as f32;

        if frame.shadows.enabled && self.shadow_resolution != frame.shadows.resolution {
            self.shadow_resolution = frame.shadows.resolution.max(1);
            self.shadow_map = shadow_view(gpu, self.shadow_resolution);
            self.bind_group = frame_bind_group(
                gpu,
                &self.layout,
                &self.frame_buffer,
                &self.shadow_map,
                &self.shadow_sampler,
            );
        }

        let sun = frame.lighting.sun_direction.normalize_or_zero();
        let light_view_projection = if frame.shadows.enabled {
            self.fit_light_frustum(frame, sun)
        } else {
            Mat4::IDENTITY
        };
        // Half the world span one texel covers, which is what the normal
        // offset has to clear at a grazing angle.
        let texel_world = self.light_texel_size(frame, sun);

        let uniform = FrameUniform {
            view_projection: frame.camera.view_projection(aspect).to_cols_array_2d(),
            sun_direction: extend(frame.lighting.sun_direction.normalize_or_zero(), 0.0),
            sun_color: extend(frame.lighting.sun_color * frame.lighting.sun_intensity, 0.0),
            sky_color: extend(frame.lighting.sky_color, 0.0),
            ground_color: extend(frame.lighting.ground_color, 0.0),
            fog_color: extend(frame.fog.color, 0.0),
            fog_range: [frame.fog.start, frame.fog.end, 0.0, 0.0],
            camera_position: extend(frame.camera.position, 1.0),
            light_view_projection: light_view_projection.to_cols_array_2d(),
            shadow_params: [
                frame.shadows.depth_bias,
                frame.shadows.normal_bias + texel_world,
                1.0 / self.shadow_resolution as f32,
                if frame.shadows.enabled { 1.0 } else { 0.0 },
            ],
        };
        gpu.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniform));

        // Draws are grouped by mesh and texture so that one mesh drawn a
        // hundred times costs one call. A forest is the same tree over and
        // over, so this is not a micro-optimisation, it is the difference
        // between one draw and a thousand.
        //
        // Two sets, because the shadow pass must not use the camera's
        // frustum: something behind you can cast a shadow in front of you,
        // and culling it leaves a hole in the ground where its shadow was.
        let planes = frustum_planes(frame.camera.view_projection(aspect));
        let mut shadow_batches: Vec<((MeshHandle, TextureHandle), Vec<InstanceRaw>)> = Vec::new();
        let mut batches: Vec<((MeshHandle, TextureHandle), Vec<InstanceRaw>)> = Vec::new();
        let mut stats = FrameStats {
            submitted: frame.draws.len() as u32,
            ..Default::default()
        };
        for draw in &frame.draws {
            let unlit = match draw.material.shading {
                Shading::Lit => 0.0,
                Shading::Unlit => 1.0,
            };
            let raw = InstanceRaw {
                model: draw.transform.to_cols_array_2d(),
                color_and_shading: extend(draw.material.color(), unlit),
            };
            let key = (draw.mesh, draw.texture);
            push(&mut shadow_batches, key, raw);
            stats.shadow_casters += 1;

            let visible = match self.meshes.get(draw.mesh.0 as usize) {
                Some(mesh) => aabb_in_frustum(&planes, mesh.bounds, draw.transform),
                // A handle pointing at nothing draws nothing; it should not
                // also be reported as culled.
                None => false,
            };
            if visible {
                push(&mut batches, key, raw);
                stats.drawn += 1;
            } else {
                stats.culled += 1;
            }
        }
        self.stats = stats;

        let shadow_total: u64 = shadow_batches.iter().map(|(_, l)| l.len() as u64).sum();
        let total: u64 = shadow_total + batches.iter().map(|(_, l)| l.len() as u64).sum::<u64>();
        if total > self.instance_capacity {
            self.instance_capacity = total.next_power_of_two();
            self.instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instances"),
                size: self.instance_capacity * std::mem::size_of::<InstanceRaw>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        // Both sets share one buffer: the shadow pass's instances first, then
        // the colour pass's, which is why the colour pass draws from
        // `shadow_total` onward.
        let flat: Vec<InstanceRaw> = shadow_batches
            .iter()
            .chain(batches.iter())
            .flat_map(|(_, l)| l.iter().copied())
            .collect();
        if !flat.is_empty() {
            gpu.queue
                .write_buffer(&self.instances, 0, bytemuck::cast_slice(&flat));
        }

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("runity::render"),
            });
        if frame.shadows.enabled {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("runity::shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_map,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        // Kept, not discarded: the colour pass reads it.
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.shadow_bind_group, &[]);
            self.draw_batches(&mut pass, &shadow_batches, 0, false);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("runity::render"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: frame.clear_color.x as f64,
                            g: frame.clear_color.y as f64,
                            b: frame.clear_color.z as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            self.draw_batches(&mut pass, &batches, shadow_total as u32, true);
        }
        gpu.queue.submit(Some(encoder.finish()));
    }
}

fn push(
    batches: &mut Vec<((MeshHandle, TextureHandle), Vec<InstanceRaw>)>,
    key: (MeshHandle, TextureHandle),
    raw: InstanceRaw,
) {
    match batches.iter_mut().find(|(k, _)| *k == key) {
        Some((_, list)) => list.push(raw),
        None => batches.push((key, vec![raw])),
    }
}

/// The six planes of a view-projection's frustum, each as `(normal, d)` with
/// the inside on the positive side.
///
/// Pulled straight out of the matrix rows (the Gribb-Hartmann trick) rather
/// than reconstructed from corners: the matrix already contains them, and
/// rebuilding the frustum from a field of view and an aspect is how the two
/// end up disagreeing after someone changes the projection.
fn frustum_planes(view_projection: Mat4) -> [glam::Vec4; 6] {
    let m = view_projection.to_cols_array_2d();
    let row = |i: usize| glam::Vec4::new(m[0][i], m[1][i], m[2][i], m[3][i]);
    let (x, y, z, w) = (row(0), row(1), row(2), row(3));
    // Near is `z` alone, not `w + z`: wgpu's clip space runs z from 0 to 1,
    // where OpenGL's runs -1 to 1. Using the OpenGL form here keeps things
    // alive for one extra near-plane's depth, which is invisible until
    // something large sits behind the camera.
    let mut planes = [w + x, w - x, w + y, w - y, z, w - z];
    for plane in &mut planes {
        let length = plane.truncate().length();
        if length > 1e-6 {
            *plane /= length;
        }
    }
    planes
}

/// Whether a box, placed by a transform, has any part inside the frustum.
///
/// Conservative: it tests the box's worst corner against each plane, so it
/// keeps some things that are just outside. Keeping a thing that cannot be
/// seen costs a draw; dropping one that can costs a hole.
fn aabb_in_frustum(
    planes: &[glam::Vec4; 6],
    bounds: crate::asset::Bounds,
    transform: Mat4,
) -> bool {
    let (lo, hi) = (Vec3::from_array(bounds.min), Vec3::from_array(bounds.max));
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for i in 0..8 {
        let corner = Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        );
        let world = transform.transform_point3(corner);
        min = min.min(world);
        max = max.max(world);
    }
    planes.iter().all(|plane| {
        // The corner furthest along the plane's normal. If even that one is
        // behind, the whole box is.
        let furthest = Vec3::new(
            if plane.x >= 0.0 { max.x } else { min.x },
            if plane.y >= 0.0 { max.y } else { min.y },
            if plane.z >= 0.0 { max.z } else { min.z },
        );
        plane.truncate().dot(furthest) + plane.w >= 0.0
    })
}

fn extend(v: Vec3, w: f32) -> [f32; 4] {
    [v.x, v.y, v.z, w]
}

/// Borrow the archived vertices as the plain `Vertex` slice the GPU wants.
///
/// Sound because `Vertex` is `repr(C)` of three `f32` arrays: rkyv's archived
/// form of such a type is byte-identical to the native one, which is exactly
/// what makes the format zero-copy.
fn vertex_slice(mesh: &ArchivedMeshAsset) -> &[crate::asset::Vertex] {
    let archived = mesh.vertices.as_slice();
    // SAFETY: `ArchivedVertex` and `Vertex` are both `repr(C)` over `[f32; N]`
    // with identical layout and no padding; rkyv archives `f32` unchanged on
    // little-endian targets, which every platform we ship on is. The asserts
    // below fail the build if that ever stops being true.
    const _: () = assert!(
        std::mem::size_of::<crate::asset::ArchivedVertex>()
            == std::mem::size_of::<crate::asset::Vertex>()
    );
    unsafe {
        std::slice::from_raw_parts(
            archived.as_ptr() as *const crate::asset::Vertex,
            archived.len(),
        )
    }
}

fn frame_bind_group(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    frame_buffer: &wgpu::Buffer,
    shadow_map: &wgpu::TextureView,
    shadow_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("frame"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(shadow_map),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(shadow_sampler),
            },
        ],
    })
}

fn shadow_view(gpu: &Gpu, resolution: u32) -> wgpu::TextureView {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow map"),
        size: wgpu::Extent3d {
            width: resolution.max(1),
            height: resolution.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn depth_view(gpu: &Gpu, width: u32, height: u32) -> wgpu::TextureView {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> crate::asset::Bounds {
        crate::asset::Bounds {
            min: [-0.5, -0.5, -0.5],
            max: [0.5, 0.5, 0.5],
        }
    }

    fn looking_down_minus_z() -> [glam::Vec4; 6] {
        let camera = Camera {
            position: Vec3::new(0.0, 0.0, 5.0),
            target: Vec3::ZERO,
            ..Camera::default()
        };
        frustum_planes(camera.view_projection(16.0 / 9.0))
    }

    #[test]
    fn a_box_in_front_of_the_camera_is_inside_the_frustum() {
        let planes = looking_down_minus_z();
        assert!(aabb_in_frustum(&planes, unit_box(), Mat4::IDENTITY));
    }

    #[test]
    fn a_box_behind_the_camera_is_outside_it() {
        let planes = looking_down_minus_z();
        let behind = Mat4::from_translation(Vec3::new(0.0, 0.0, 40.0));
        assert!(!aabb_in_frustum(&planes, unit_box(), behind));
    }

    #[test]
    fn a_box_far_off_to_the_side_is_outside_it() {
        let planes = looking_down_minus_z();
        let aside = Mat4::from_translation(Vec3::new(60.0, 0.0, 0.0));
        assert!(!aabb_in_frustum(&planes, unit_box(), aside));
    }

    #[test]
    fn a_huge_box_straddling_the_camera_is_inside_it() {
        // A ground plane is bigger than the frustum and contains it. Testing
        // only the box's centre, or only its corners against each plane
        // independently, gets this one wrong and culls the floor.
        let planes = looking_down_minus_z();
        let ground = Mat4::from_scale(Vec3::new(200.0, 1.0, 200.0));
        assert!(aabb_in_frustum(&planes, unit_box(), ground));
    }
}
