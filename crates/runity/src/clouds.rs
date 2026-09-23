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
    /// Near, far.
    pub depth_range: [f32; 4],
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
                label: Some("runity::clouds"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("runity::clouds"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("cs_clouds"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some("cs_clouds"),
                compilation_options: Default::default(),
                cache: None,
            });
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
            texture: None,
            blank: blank.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }

    /// What the sky pass reads: the clouds, or a clear sky.
    pub(crate) fn view(&self) -> &wgpu::TextureView {
        self.texture.as_ref().map_or(&self.blank, |t| &t.0)
    }

    /// Make sure the picture is a quarter of `size`; true when it was
    /// remade, so what binds it must be too.
    pub(crate) fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) -> bool {
        let quarter = (size.0.div_ceil(4).max(1), size.1.div_ceil(4).max(1));
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
        uniform.size = [size.0 as f32, size.1 as f32, 0.0, 0.0];
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniform));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("runity::clouds"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
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
