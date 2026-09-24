//! Clouds: a layer of them over the sky, drifting with the wind, lit
//! through by the sun and throwing their shadows on the ground.
//!
//! `sky: (clouds: (coverage: 0.5))` in a scene — with the procedural sky or
//! the physical one; 0 coverage (the default) is a clear sky. The clouds
//! are 3D noise kept to a slab of air (`base` metres up, `thickness`
//! thick), marched through by a compute pass at a quarter of the frame's
//! resolution — light from the sun through the cloud above each step, with
//! the silver lining towards the sun, and the sky's light from all round —
//! and laid over the sky. What is under a cloud is in its shadow: the lit
//! shader looks up through the same noise towards the sun, so shadows roll
//! over the valley as the clouds drift.

use serde::{Deserialize, Serialize};

use crate::gpu::Gpu;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clouds {
    /// How much of the sky is cloud, 0 to 1.
    pub coverage: f32,
    /// Their floor, metres up.
    pub base: f32,
    pub thickness: f32,
    /// How dense they are: more is darker bellies.
    pub density: f32,
    /// How big a cloud is, metres.
    pub scale: f32,
    /// How fast they drift with the scene's wind, metres a second at a
    /// wind of 1.
    pub speed: f32,
    /// How dark their shadows on the ground are, 0 to 1.
    pub shadows: f32,
}

impl Default for Clouds {
    fn default() -> Self {
        Self {
            coverage: 0.0,
            base: 1500.0,
            thickness: 1200.0,
            density: 1.0,
            scale: 2200.0,
            speed: 12.0,
            shadows: 0.8,
        }
    }
}

pub const SHADER: &str = include_str!("clouds.wgsl");

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct CloudUniform {
    pub inverse_view_projection: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub to_sun: [f32; 4],
    pub sun: [f32; 4],
    pub ambient: [f32; 4],
    pub shape: [f32; 4],
    pub drift: [f32; 4],
    pub size: [f32; 4],
    /// The dust wall: how dense, its front's distance upwind now, its
    /// height, 1 when there is a prepass depth to stop at.
    pub dust: [f32; 4],
    /// The camera view's third row: how deep a point is.
    pub view_depth: [f32; 4],
    /// Near, far; the wind's level direction.
    pub depth_range: [f32; 4],
    /// The dust volume round the camera: its corner's x and z, a cell's
    /// width and its height.
    pub dust_box: [f32; 4],
    /// How many dust devils and crest plumes follow.
    pub local: [f32; 4],
    /// Each devil: its foot and radius; height, strength, spin.
    pub devils: [[f32; 4]; 2 * crate::volume::MOST_DEVILS],
    /// Each plume: its middle on the crest and half its length; the
    /// crest's way and how much is blowing.
    pub plumes: [[f32; 4]; 2 * crate::volume::MOST_PLUMES],
}

/// Cells of the dust wall's volume: across, up, across.
/// The dust wall's light volume: light wants no finer.
pub(crate) const DUST_CELLS: (u32, u32, u32) = (128, 32, 128);
/// A cell's width, metres: the volume is nearly 4 km across, round the
/// camera.
pub(crate) const DUST_CELL: f32 = 30.0;
/// The billows' noise, made once: 128³, tileable.
const NOISE_SIZE: u32 = 128;

impl CloudUniform {
    /// The dust volume round `eye`, snapped to its cells so it does not
    /// swim as the camera moves, for a wall `height` high.
    pub(crate) fn dust_box(eye: glam::Vec3, height: f32) -> [f32; 4] {
        let half = DUST_CELL * DUST_CELLS.0 as f32 * 0.5;
        let snap = |v: f32| ((v - half) / DUST_CELL).floor() * DUST_CELL;
        [
            snap(eye.x),
            snap(eye.z),
            DUST_CELL,
            height * 1.35 / DUST_CELLS.1 as f32,
        ]
    }
}

impl Clouds {
    /// The shape and drift vectors both the cloud pass and the lit shader's
    /// shadows read: coverage, base, thickness, density; wind x and z in
    /// metres a second, size, how far they reach.
    pub(crate) fn vectors(&self, wind: &crate::foliage::Wind) -> ([f32; 4], [f32; 4]) {
        let level = glam::Vec2::new(wind.direction.x, wind.direction.z).normalize_or(glam::Vec2::X);
        let drift = level * self.speed * wind.strength.max(0.0);
        (
            [
                self.coverage.clamp(0.0, 1.0),
                self.base,
                self.thickness.max(10.0),
                self.density.max(0.0),
            ],
            [drift.x, drift.y, self.scale.max(10.0), 30_000.0],
        )
    }
}

/// The clouds' picture, a quarter of the frame, and what makes it.
pub(crate) struct CloudRenderer {
    uniforms: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    /// The dust wall: its noise, made once, and its light, a pass a frame.
    dust_noise: wgpu::ComputePipeline,
    dust_light: wgpu::ComputePipeline,
    dust_noise_group: wgpu::BindGroup,
    dust_light_group: wgpu::BindGroup,
    dust_march_group: wgpu::BindGroup,
    noise_made: std::cell::Cell<bool>,
    texture: Option<(wgpu::TextureView, (u32, u32))>,
    /// A clear sky, bound until there are clouds to show.
    blank: wgpu::TextureView,
}

impl CloudRenderer {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("clouds"),
            size: std::mem::size_of::<CloudUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("clouds"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::Rgba16Float,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                ],
            });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("scrap::clouds"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        // The noise and the light volume, and a layout for each pass's part.
        let texture_3d = |label, size: (u32, u32, u32), format| {
            gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: size.2,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let noise_view = texture_3d("dust noise", (NOISE_SIZE, NOISE_SIZE, NOISE_SIZE), wgpu::TextureFormat::Rgba8Unorm);
        let lit_view = texture_3d("dust light", DUST_CELLS, wgpu::TextureFormat::Rgba16Float);
        let storage = |binding, format| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension: wgpu::TextureViewDimension::D3,
            },
            count: None,
        };
        let sampled = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D3,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let group_layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries,
                })
        };
        let noise_layout = group_layout("dust noise", &[storage(0, wgpu::TextureFormat::Rgba8Unorm)]);
        let light_layout = group_layout(
            "dust light",
            &[sampled(1), storage(2, wgpu::TextureFormat::Rgba16Float), sampler_entry(5)],
        );
        let march_layout = group_layout("dust march", &[sampled(1), sampled(3), sampler_entry(4), sampler_entry(5)]);
        let dust_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("dust"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let noise_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("dust noise"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let group = |layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry]| {
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dust"),
                layout,
                entries,
            })
        };
        let view_entry = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let sampler_of = |binding, sampler| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Sampler(sampler),
        };
        let dust_noise_group = group(&noise_layout, &[view_entry(0, &noise_view)]);
        let dust_light_group = group(
            &light_layout,
            &[view_entry(1, &noise_view), view_entry(2, &lit_view), sampler_of(5, &noise_sampler)],
        );
        let dust_march_group = group(
            &march_layout,
            &[
                view_entry(1, &noise_view),
                view_entry(3, &lit_view),
                sampler_of(4, &dust_sampler),
                sampler_of(5, &noise_sampler),
            ],
        );
        let pipeline_for = |entry: &str, second: &wgpu::BindGroupLayout| {
            let pipeline_layout =
                gpu.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(entry),
                        bind_group_layouts: &[Some(&layout), Some(second)],
                        immediate_size: 0,
                    });
            gpu.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let pipeline = pipeline_for("cs_clouds", &march_layout);
        let dust_noise = pipeline_for("cs_dust_noise", &noise_layout);
        let dust_light = pipeline_for("cs_dust_light", &light_layout);
        let blank = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("no clouds"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Nothing added, everything let through: half floats 0, 0, 0, 1.
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &blank,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&[0u16, 0, 0, 0x3C00]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        Self {
            uniforms,
            layout,
            pipeline,
            dust_noise,
            dust_light,
            dust_noise_group,
            dust_light_group,
            dust_march_group,
            noise_made: std::cell::Cell::new(false),
            texture: None,
            blank: blank.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }

    /// What the sky pass reads: the clouds, or a clear sky.
    pub(crate) fn view(&self) -> &wgpu::TextureView {
        self.texture.as_ref().map_or(&self.blank, |t| &t.0)
    }

    /// Make sure the picture is `size` over `divisor` — a quarter for
    /// clouds, which are soft; a half for a dust wall, whose billows have
    /// edges.
    pub(crate) fn resize(&mut self, gpu: &Gpu, size: (u32, u32), divisor: u32) -> bool {
        let quarter = (
            size.0.div_ceil(divisor).max(1),
            size.1.div_ceil(divisor).max(1),
        );
        if self.texture.as_ref().is_some_and(|t| t.1 == quarter) {
            return false;
        }
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("clouds"),
            size: wgpu::Extent3d {
                width: quarter.0,
                height: quarter.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.texture = Some((view, quarter));
        true
    }

    pub(crate) fn run(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        mut uniform: CloudUniform,
        depth: &wgpu::TextureView,
    ) {
        let Some((view, size)) = &self.texture else {
            return;
        };
        // Made here: the scene's depth it stops at is remade with the frame.
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("clouds"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
            ],
        });
        uniform.size = [size.0 as f32, size.1 as f32, uniform.size[2], 0.0];
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniform));
        // The dust wall's light first, when there is a wall: a pass of its
        // own, timed apart.
        if uniform.dust[0] > 0.0 {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("scrap::dust light"),
                timestamp_writes: crate::gpu_timer::compute("dust light"),
            });
            pass.set_bind_group(0, &group, &[]);
            let cells = |n: u32, by: u32| n.div_ceil(by);
            // The billows' noise, the first time there is a wall.
            if !self.noise_made.get() {
                pass.set_pipeline(&self.dust_noise);
                pass.set_bind_group(1, &self.dust_noise_group, &[]);
                let n = NOISE_SIZE.div_ceil(4);
                pass.dispatch_workgroups(n, n, n);
                self.noise_made.set(true);
            }
            pass.set_pipeline(&self.dust_light);
            pass.set_bind_group(1, &self.dust_light_group, &[]);
            pass.dispatch_workgroups(
                cells(DUST_CELLS.0, 8),
                cells(DUST_CELLS.1, 8),
                cells(DUST_CELLS.2, 4),
            );
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("scrap::clouds"),
            timestamp_writes: crate::gpu_timer::compute("clouds"),
        });
        pass.set_bind_group(0, &group, &[]);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.dust_march_group, &[]);
        pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn clouds_read_from_a_sky_line_and_drift_with_the_wind() {
        let sky: crate::render::Sky = ron::from_str("(clouds: (coverage: 0.6))").unwrap();
        assert_eq!(sky.clouds.coverage, 0.6);
        assert_eq!(sky.clouds.base, 1500.0);
        let (shape, drift) = sky.clouds.vectors(&crate::foliage::Wind {
            direction: glam::Vec3::Z,
            strength: 2.0,
        });
        assert_eq!(shape[0], 0.6);
        assert_eq!((drift[0], drift[1]), (0.0, 24.0));
    }
}
