//! Hardware ray tracing — an experiment.
//!
//! Where the device has hardware ray queries (Apple silicon from M3,
//! RTX, RDNA 2 and later, through wgpu's `EXPERIMENTAL_RAY_QUERY`), the
//! lit shader can ask the scene itself whether a point sees something,
//! rather than a shadow map or the depth buffer guessing:
//!
//! * **Sun shadows** by rays towards the sun, jittered across its disc — a
//!   penumbra that widens with distance from the caster, as a real one does,
//!   with no cascades, no bias to tune and no shadow distance.
//! * **Shadows from point and spot lights**, which the rasterized path does
//!   not have at all: a ray to each lamp.
//! * **Ambient occlusion** by short rays over the hemisphere, which sees
//!   what is off screen and behind things, where SSAO cannot.
//!
//! What it is not, yet: temporal — each pixel's few rays are the whole
//! answer, so penumbrae and occlusion are grainy up close; nothing is
//! accumulated between frames or denoised. Everything in the acceleration
//! structure is opaque: a cut-out leaf shadows as its whole card, and a
//! skinned mesh is traced in its bind pose. And it is off unless a frame
//! asks ([`RayTracing`]); on a device without ray queries the ask changes
//! nothing and the frame is drawn as ever. DNA, postulate 7, wants one
//! render path: this is a second, and is kept as an experiment — to see
//! what it gives — until a decision says otherwise.

use serde::{Deserialize, Serialize};

use crate::gpu::Gpu;

/// What the rays are asked for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RayTracing {
    /// The sun's shadows by rays, instead of the cascades.
    pub sun_shadows: bool,
    /// Point and spot lights cast shadows, by a ray each.
    pub light_shadows: bool,
    /// Ambient occlusion by rays, instead of SSAO.
    pub ambient_occlusion: bool,
    /// The sun's disc, degrees across: how wide its shadows' penumbrae
    /// open. The real one is about half a degree.
    pub sun_size: f32,
    /// Rays towards the sun per pixel.
    pub sun_rays: u32,
    /// Rays over the hemisphere per pixel, for occlusion.
    pub occlusion_rays: u32,
    /// How far an occlusion ray looks, in metres.
    pub occlusion_radius: f32,
}

impl Default for RayTracing {
    /// Off: an experiment is asked for, not assumed.
    fn default() -> Self {
        Self {
            sun_shadows: false,
            light_shadows: false,
            ambient_occlusion: false,
            sun_size: 1.0,
            sun_rays: 4,
            occlusion_rays: 6,
            occlusion_radius: 1.0,
        }
    }
}

impl RayTracing {
    /// Everything on, at the defaults' counts.
    pub fn all() -> Self {
        Self {
            sun_shadows: true,
            light_shadows: true,
            ambient_occlusion: true,
            ..Self::default()
        }
    }

    pub fn any(&self) -> bool {
        self.sun_shadows || self.light_shadows || self.ambient_occlusion
    }
}

/// The traced half of the lit shader: `ray_visible`, and the binding it
/// reads. Put in place of the stub in `render.wgsl` on a device that traces.
pub const SHADER: &str = include_str!("ray.wgsl");

/// Where the stub is in `render.wgsl`.
const STUB_BEGIN: &str = "// ray: stub begin";
const STUB_END: &str = "// ray: stub end";

/// The lit shader for a device that traces: the stub replaced by the real
/// thing, and the extension enabled at the top.
pub(crate) fn traced(source: &str) -> String {
    let (Some(begin), Some(end)) = (source.find(STUB_BEGIN), source.find(STUB_END)) else {
        return source.to_string();
    };
    format!(
        "enable wgpu_ray_query;\n{}{}{}",
        &source[..begin],
        SHADER,
        &source[end + STUB_END.len()..]
    )
}

/// A mesh as the rays see it.
pub(crate) fn blas(
    gpu: &Gpu,
    vertices: &wgpu::Buffer,
    vertex_count: u32,
    indices: &wgpu::Buffer,
    index_count: u32,
) -> wgpu::Blas {
    let size = wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count,
        index_format: Some(wgpu::IndexFormat::Uint32),
        index_count: Some(index_count),
        flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
    };
    let blas = gpu.device.create_blas(
        &wgpu::CreateBlasDescriptor {
            label: Some("mesh (rays)"),
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        },
        wgpu::BlasGeometrySizeDescriptors::Triangles {
            descriptors: vec![size.clone()],
        },
    );
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("build a mesh's rays"),
        });
    encoder.build_acceleration_structures(
        std::iter::once(&wgpu::BlasBuildEntry {
            blas: &blas,
            geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                size: &size,
                vertex_buffer: vertices,
                first_vertex: 0,
                vertex_stride: std::mem::size_of::<crate::asset::Vertex>() as u64,
                index_buffer: Some(indices),
                first_index: Some(0),
                transform_buffer: None,
                transform_buffer_offset: None,
            }]),
        }),
        std::iter::empty(),
    );
    gpu.queue.submit(Some(encoder.finish()));
    blas
}

/// The scene as the rays see it: every solid draw of the frame, placed.
pub(crate) struct RayScene {
    pub(crate) tlas: wgpu::Tlas,
    capacity: u32,
    /// A triangle far below everything, so the structure is never empty.
    placeholder: wgpu::Blas,
    _placeholder_buffers: (wgpu::Buffer, wgpu::Buffer),
}

fn rows(m: glam::Mat4) -> [f32; 12] {
    let r = |i| m.row(i);
    let (a, b, c) = (r(0), r(1), r(2));
    [a.x, a.y, a.z, a.w, b.x, b.y, b.z, b.w, c.x, c.y, c.z, c.w]
}

impl RayScene {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        use wgpu::util::DeviceExt;
        let vertices: [crate::asset::Vertex; 3] = std::array::from_fn(|i| crate::asset::Vertex {
            position: [i as f32 * 1e-3, -1.0e6, (i / 2) as f32 * 1e-3],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        });
        let usage = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::BLAS_INPUT;
        let vb = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("placeholder"),
                contents: bytemuck::cast_slice(&vertices),
                usage,
            });
        let ib = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("placeholder"),
                contents: bytemuck::cast_slice(&[0u32, 1, 2]),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::BLAS_INPUT,
            });
        let placeholder = blas(gpu, &vb, 3, &ib, 3);
        let capacity = 64;
        let mut scene = Self {
            tlas: Self::tlas(gpu, capacity),
            capacity,
            placeholder,
            _placeholder_buffers: (vb, ib),
        };
        // Built once now, so it can be bound before any frame fills it.
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("first rays"),
            });
        scene.update(gpu, &mut encoder, &[]);
        gpu.queue.submit(Some(encoder.finish()));
        scene
    }

    fn tlas(gpu: &Gpu, capacity: u32) -> wgpu::Tlas {
        gpu.device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("scene (rays)"),
            max_instances: capacity,
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        })
    }

    /// Put this frame's solid draws in, and build. `true` when the structure
    /// was remade bigger, so what binds it must be too.
    pub(crate) fn update(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        instances: &[(&wgpu::Blas, glam::Mat4)],
    ) -> bool {
        let needed = instances.len() as u32 + 1;
        let mut remade = false;
        if needed > self.capacity {
            self.capacity = needed.next_power_of_two();
            self.tlas = Self::tlas(gpu, self.capacity);
            remade = true;
        }
        let count = self.capacity as usize;
        if let Some(slots) = self.tlas.get_mut_slice(0..count) {
            for slot in slots.iter_mut() {
                *slot = None;
            }
            slots[0] = Some(wgpu::TlasInstance::new(
                &self.placeholder,
                rows(glam::Mat4::IDENTITY),
                0,
                0xff,
            ));
            for (slot, (blas, transform)) in slots[1..].iter_mut().zip(instances) {
                *slot = Some(wgpu::TlasInstance::new(blas, rows(*transform), 0, 0xff));
            }
        }
        encoder.build_acceleration_structures(std::iter::empty(), std::iter::once(&self.tlas));
        remade
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_is_replaced_and_the_extension_enabled() {
        let source = "struct A { x: f32 };\n// ray: stub begin\nfn ray_visible() -> f32 { return 1.0; }\n// ray: stub end\nfn after() {}\n";
        let traced = traced(source);
        assert!(traced.starts_with("enable wgpu_ray_query;"));
        assert!(!traced.contains("return 1.0"));
        assert!(traced.contains("rayQueryInitialize") && traced.contains("fn after()"));
    }

    #[test]
    fn a_transform_goes_in_by_rows() {
        let m = glam::Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 3.0));
        let r = rows(m);
        assert_eq!([r[3], r[7], r[11]], [1.0, 2.0, 3.0]);
    }
}
