//! The scene's signed distance field, drawn with (docs/simulation.md,
//! item 12): the same field soft things collide with, here darkening what
//! is near other things (distance-field ambient occlusion) and softening
//! the sun's shadows where the shadow map is too coarse to — marched toward
//! the sun, the nearest a ray passes to anything is how much of the sun's
//! disc it sees (Quilez; Unreal's mesh distance fields).
//!
//! Whoever bakes the field hands it over as bytes; the render uploads it
//! when it changes and reads it in the main pass.

use glam::Vec3;

/// A baked field for the render: `range` metres either side of the
/// surface as 0–255, the surface at 128.
#[derive(Debug, Clone, PartialEq)]
pub struct DistanceField {
    pub low: Vec3,
    pub high: Vec3,
    pub size: [u32; 3],
    pub range: f32,
    pub cells: std::sync::Arc<Vec<u8>>,
}

/// The largest field the render takes along each way.
pub const MOST: u32 = 256;

/// The field as the shader reads it: its box, 1 when there is one, and
/// its range.
pub(crate) fn uniform(field: Option<&DistanceField>) -> [[f32; 4]; 2] {
    match field {
        Some(f) => [
            [f.low.x, f.low.y, f.low.z, 1.0],
            [f.high.x, f.high.y, f.high.z, f.range],
        ],
        None => [[0.0; 4]; 2],
    }
}

/// The field's texture, kept until the field changes.
pub(crate) struct DistanceTexture {
    pub view: wgpu::TextureView,
    /// Which field it holds: its cells' address and its size.
    key: Option<(usize, [u32; 3])>,
}

impl DistanceTexture {
    pub fn new(gpu: &crate::gpu::Gpu) -> Self {
        Self { view: texture(gpu, [1, 1, 1], &[255]), key: None }
    }

    /// The field in, if it is not what is there; whether it changed, for
    /// the bind group to be made again.
    pub fn update(&mut self, gpu: &crate::gpu::Gpu, field: Option<&DistanceField>) -> bool {
        let key = field.map(|f| (std::sync::Arc::as_ptr(&f.cells) as usize, f.size));
        if key == self.key {
            return false;
        }
        self.key = key;
        self.view = match field {
            Some(f) if f.size.iter().all(|s| (1..=MOST).contains(s)) && f.cells.len() == (f.size[0] * f.size[1] * f.size[2]) as usize => {
                texture(gpu, f.size, &f.cells)
            }
            _ => texture(gpu, [1, 1, 1], &[255]),
        };
        true
    }
}

fn texture(gpu: &crate::gpu::Gpu, size: [u32; 3], cells: &[u8]) -> wgpu::TextureView {
    let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: size[2] };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("distance field"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        texture.as_image_copy(),
        cells,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size[0]), rows_per_image: Some(size[1]) },
        extent,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
