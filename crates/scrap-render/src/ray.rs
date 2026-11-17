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
//! * **Shadows from point and spot lights** by a ray to each lamp, instead
//!   of their shadow maps — every lamp, with no budget of maps to share.
//! * **Ambient occlusion** by short rays over the hemisphere, which sees
//!   what is off screen and behind things, where SSAO cannot.
//! * **Reflections** by a ray along the mirror direction: what is behind
//!   the camera, off the screen or hidden from it shows in chrome and
//!   polish, where screen-space reflections have nothing to read. The hit
//!   is lit simply — its material's colour, the sun (with a shadow ray),
//!   the sky's hemisphere and the lamps (with theirs) — and its facing is
//!   found by two more rays beside the first, since the hardware here
//!   does not hand back the triangle it hit. Textures are not read there:
//!   a reflected thing shows its material's colour.
//! * **Refraction** through glass: in through its near side, bent; out
//!   through its far side, found by a ray that sees only glass, bent
//!   again (or turned back, past the critical angle); then on to what is
//!   behind, lit as a reflection's hit is.
//!
//! What it is not, yet: temporal by itself — each pixel's few rays are
//! the frame's whole answer, and only TAA's history, where it is on,
//! smooths the grain; there is no denoiser. The terrain is seen as its
//! coarse heightfield, so its rays start past a gap (`RAY_TERRAIN_GAP` in
//! render.wgsl) rather than crack its ripples with shadow. Everything in the acceleration
//! structure is opaque: a cut-out leaf shadows as its whole card. A
//! skinned mesh is traced in its frame's pose ([`Posed`]): its vertices
//! skinned again by a compute pass, its own structure built from them. And it is off unless a frame
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
    /// Point and spot lights' shadows by a ray each, instead of their maps.
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
    /// How big a lamp is, its radius in metres: the ray to it aims at a
    /// different point of it each frame and pixel, so its shadow is sharp
    /// where it touches its caster and soft away, as a flame's or a bulb's.
    /// 0 is a point, and a hard edge.
    pub lamp_size: f32,
    /// Reflections by rays, instead of the screen's and the probes'.
    pub reflections: bool,
    /// The roughest surface that reflects by rays: rougher ones keep the
    /// probes' blur. 0 to 1, perceptual.
    pub reflection_roughness: f32,
    /// Glass bends what is seen through it: a ray in through its near
    /// side, out through its far one, bent at each, on to what is behind.
    pub refractions: bool,
    /// How much glass bends light: 1.5 is window glass, 1.33 water.
    pub index_of_refraction: f32,
    /// The lamps' light and shadows by ReSTIR ([`crate::restir`]): one
    /// shadow ray a pixel however many lamps there are, instead of one a
    /// lamp.
    pub restir: bool,
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
            lamp_size: 0.0,
            reflections: false,
            reflection_roughness: 0.45,
            refractions: false,
            index_of_refraction: 1.5,
            restir: false,
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
            reflections: true,
            refractions: true,
            ..Self::default()
        }
    }

    pub fn any(&self) -> bool {
        self.sun_shadows || self.light_shadows || self.ambient_occlusion || self.reflections || self.refractions || self.restir
    }
}

/// The traced half of the lit shader: `ray_clear`, and the binding it
/// reads. Put in place of the stub in `render.wgsl` on a device that traces.
#[cfg(feature = "ray-tracing")]
pub const SHADER: &str = include_str!("ray.wgsl");

/// Where the stub is in `render.wgsl`.
#[cfg(feature = "ray-tracing")]
const STUB_BEGIN: &str = "// ray: stub begin";
#[cfg(feature = "ray-tracing")]
const STUB_END: &str = "// ray: stub end";

/// The lit shader for a device that traces: the stub replaced by the real
/// thing, and the extension enabled at the top.
#[cfg(feature = "ray-tracing")]
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
#[cfg(feature = "ray-tracing")]
pub(crate) fn blas(
    gpu: &Gpu,
    vertices: &wgpu::Buffer,
    vertex_count: u32,
    indices: &wgpu::Buffer,
    index_count: u32,
    live: bool,
) -> wgpu::Blas {
    let blas = gpu.device.create_blas(
        &wgpu::CreateBlasDescriptor {
            label: Some("mesh (rays)"),
            // A mesh rewritten every frame is built every frame: quick to
            // build over quick to trace.
            flags: if live {
                wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD
            } else {
                wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE
            },
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        },
        wgpu::BlasGeometrySizeDescriptors::Triangles {
            descriptors: vec![triangles(vertex_count, index_count)],
        },
    );
    rebuild(gpu, &blas, vertices, vertex_count, indices, index_count);
    blas
}

fn triangles(vertex_count: u32, index_count: u32) -> wgpu::BlasTriangleGeometrySizeDescriptor {
    wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count,
        index_format: Some(wgpu::IndexFormat::Uint32),
        index_count: Some(index_count),
        flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
    }
}

/// Build `blas` again from its buffers, rewritten in place (the same
/// sizes as it was made with): a live mesh's rays follow it.
#[cfg(feature = "ray-tracing")]
pub(crate) fn rebuild(
    gpu: &Gpu,
    blas: &wgpu::Blas,
    vertices: &wgpu::Buffer,
    vertex_count: u32,
    indices: &wgpu::Buffer,
    index_count: u32,
) {
    let size = triangles(vertex_count, index_count);
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("build a mesh's rays"),
        });
    encoder.build_acceleration_structures(
        std::iter::once(&wgpu::BlasBuildEntry {
            blas,
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
}

/// What a reflection ray reads of the thing it hits: `RayMaterial` in
/// ray.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct RayMaterial {
    /// Linear colour; metallic, or −1 for what is unlit (a light itself).
    pub(crate) color_metal: [f32; 4],
    /// Emission, linear; smoothness.
    pub(crate) emission_smooth: [f32; 4],
}

impl RayMaterial {
    pub(crate) fn of(material: &crate::Material) -> Self {
        let c = material.color();
        let unlit = material.shading == crate::material::Shading::Unlit;
        RayMaterial {
            color_metal: [c.x, c.y, c.z, if unlit { -1.0 } else { material.metallic.clamp(0.0, 1.0) }],
            emission_smooth: [
                material.emission[0].max(0.0),
                material.emission[1].max(0.0),
                material.emission[2].max(0.0),
                material.smoothness.clamp(0.0, 1.0),
            ],
        }
    }
}

/// The scene as the rays see it: every solid draw of the frame, placed.
#[cfg(feature = "ray-tracing")]
pub(crate) struct RayScene {
    pub(crate) tlas: wgpu::Tlas,
    /// Each instance's material, by its slot: what a reflection ray that
    /// hits it reads (`RayMaterial` in ray.wgsl).
    pub(crate) materials: wgpu::Buffer,
    capacity: u32,
    /// A triangle far below everything, so the structure is never empty.
    placeholder: wgpu::Blas,
    _placeholder_buffers: (wgpu::Buffer, wgpu::Buffer),
}

#[cfg(feature = "ray-tracing")]
fn rows(m: glam::Mat4) -> [f32; 12] {
    let r = |i| m.row(i);
    let (a, b, c) = (r(0), r(1), r(2));
    [a.x, a.y, a.z, a.w, b.x, b.y, b.z, b.w, c.x, c.y, c.z, c.w]
}

#[cfg(feature = "ray-tracing")]
impl RayScene {
    /// What the lit pass binds of it: the structure and the materials.
    pub(crate) fn bindings(&self) -> (&wgpu::Tlas, &wgpu::Buffer) {
        (&self.tlas, &self.materials)
    }

    pub(crate) fn new(gpu: &Gpu) -> Self {
        let vertices: [crate::asset::Vertex; 3] = std::array::from_fn(|i| crate::asset::Vertex {
            position: [i as f32 * 1e-3, -1.0e6, (i / 2) as f32 * 1e-3],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        });
        let usage = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::BLAS_INPUT;
        let vb = gpu.buffer_init("placeholder", bytemuck::cast_slice(&vertices), usage);
        let ib = gpu.buffer_init("placeholder", bytemuck::cast_slice(&[0u32, 1, 2]), wgpu::BufferUsages::INDEX | wgpu::BufferUsages::BLAS_INPUT);
        let placeholder = blas(gpu, &vb, 3, &ib, 3, false);
        let capacity = 64;
        let mut scene = Self {
            tlas: Self::tlas(gpu, capacity),
            materials: Self::materials(gpu, capacity),
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

    fn materials(gpu: &Gpu, capacity: u32) -> wgpu::Buffer {
        gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene materials (rays)"),
            size: capacity as u64 * std::mem::size_of::<RayMaterial>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
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
        instances: &[(&wgpu::Blas, glam::Mat4, u8, RayMaterial)],
    ) -> bool {
        let needed = instances.len() as u32 + 1;
        let mut remade = false;
        if needed > self.capacity {
            self.capacity = needed.next_power_of_two();
            self.tlas = Self::tlas(gpu, self.capacity);
            self.materials = Self::materials(gpu, self.capacity);
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
            for (i, (slot, (blas, transform, mask, _))) in slots[1..].iter_mut().zip(instances).enumerate() {
                // Its custom index is its slot: where its material is.
                *slot = Some(wgpu::TlasInstance::new(blas, rows(*transform), i as u32 + 1, *mask));
            }
        }
        let mut materials = vec![RayMaterial::default(); instances.len() + 1];
        for (i, (_, _, _, material)) in instances.iter().enumerate() {
            materials[i + 1] = *material;
        }
        gpu.queue
            .write_buffer(&self.materials, 0, bytemuck::cast_slice(&materials));
        encoder.build_acceleration_structures(std::iter::empty(), std::iter::once(&self.tlas));
        remade
    }
}

/// The rays' scene for a device that traces them; `None` where it does
/// not, or in a build without the `ray-tracing` feature.
pub(crate) fn scene(gpu: &Gpu) -> Option<RayScene> {
    #[cfg(feature = "ray-tracing")]
    return gpu.ray_tracing.then(|| RayScene::new(gpu));
    #[cfg(not(feature = "ray-tracing"))]
    {
        let _ = gpu;
        None
    }
}

/// The posing pass for skinned meshes, where there is a rays' scene.
pub(crate) fn posed(gpu: &Gpu) -> Option<Posed> {
    #[cfg(feature = "ray-tracing")]
    return gpu.ray_tracing.then(|| Posed::new(gpu));
    #[cfg(not(feature = "ray-tracing"))]
    {
        let _ = gpu;
        None
    }
}

/// Skinned meshes as the rays see them: each skinned draw's vertices posed
/// as the frame poses it (the vertex shader's skinning, in a compute pass)
/// and a structure of its own built from them — else a character's shadow
/// stands in its bind pose while he walks. Slots are kept by (mesh, nth
/// posed draw of it) and reused frame to frame.
#[cfg(feature = "ray-tracing")]
pub(crate) struct Posed {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    slots: std::collections::HashMap<(u32, u32), PosedSlot>,
}

#[cfg(feature = "ray-tracing")]
struct PosedSlot {
    out: wgpu::Buffer,
    joints: wgpu::Buffer,
    blas: wgpu::Blas,
    bind: wgpu::BindGroup,
    vertex_count: u32,
}

/// The joints a pose may have: the vertex shader's `Pose`.
#[cfg(feature = "ray-tracing")]
const POSE_JOINTS: usize = 128;

#[cfg(feature = "ray-tracing")]
const POSE_SHADER: &str = r#"
// A skinned mesh's vertices posed: render.wgsl's skinned_vertex, for rays.
@group(0) @binding(0) var<storage, read> src: array<f32>;        // Vertex: position, normal, uv
@group(0) @binding(1) var<storage, read> skin: array<u32>;       // SkinVertex: joints u16 x4, weights f32 x4
@group(0) @binding(2) var<storage, read> joints: array<mat4x4<f32>>;
@group(0) @binding(3) var<storage, read_write> dst: array<f32>;

@compute @workgroup_size(64)
fn pose(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= arrayLength(&src) / 8u { return; }
    let s = i * 6u;
    let j = vec4<u32>(skin[s] & 0xffffu, skin[s] >> 16u, skin[s + 1u] & 0xffffu, skin[s + 1u] >> 16u);
    let w = vec4<f32>(bitcast<f32>(skin[s + 2u]), bitcast<f32>(skin[s + 3u]), bitcast<f32>(skin[s + 4u]), bitcast<f32>(skin[s + 5u]));
    var m = joints[j.x] * w.x + joints[j.y] * w.y + joints[j.z] * w.z + joints[j.w] * w.w;
    if w.x + w.y + w.z + w.w < 0.0001 {
        m = mat4x4<f32>(vec4<f32>(1.0, 0.0, 0.0, 0.0), vec4<f32>(0.0, 1.0, 0.0, 0.0), vec4<f32>(0.0, 0.0, 1.0, 0.0), vec4<f32>(0.0, 0.0, 0.0, 1.0));
    }
    let v = i * 8u;
    let p = m * vec4<f32>(src[v], src[v + 1u], src[v + 2u], 1.0);
    dst[v] = p.x;
    dst[v + 1u] = p.y;
    dst[v + 2u] = p.z;
    for (var k = 3u; k < 8u; k = k + 1u) {
        dst[v + k] = src[v + k];
    }
}
"#;

#[cfg(feature = "ray-tracing")]
impl Posed {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pose for rays"),
            source: wgpu::ShaderSource::Wgsl(POSE_SHADER.into()),
        });
        let entry = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pose for rays"),
            entries: &[entry(0, true), entry(1, true), entry(2, true), entry(3, false)],
        });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pose for rays"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pose for rays"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("pose"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { pipeline, layout, slots: Default::default() }
    }

    /// Pose slot `key`'s vertices as `joints` say, and build its structure
    /// again. `vertices` and `skin` are the mesh's own (STORAGE), `indices`
    /// its whole index buffer (BLAS_INPUT).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pose(
        &mut self,
        gpu: &Gpu,
        key: (u32, u32),
        vertices: &wgpu::Buffer,
        vertex_count: u32,
        skin: &wgpu::Buffer,
        indices: &wgpu::Buffer,
        index_count: u32,
        joints: &[glam::Mat4],
    ) {
        let fresh = self.slots.get(&key).is_none_or(|s| s.vertex_count != vertex_count);
        if fresh {
            let out = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("posed vertices"),
                size: vertices.size(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::BLAS_INPUT,
                mapped_at_creation: false,
            });
            let joints_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("posed joints"),
                size: (POSE_JOINTS * 64) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pose for rays"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: vertices.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: skin.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: joints_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: out.as_entire_binding() },
                ],
            });
            let blas = blas(gpu, &out, vertex_count, indices, index_count, true);
            self.slots.insert(key, PosedSlot { out, joints: joints_buffer, blas, bind, vertex_count });
        }
        let slot = &self.slots[&key];
        let mut padded = [glam::Mat4::IDENTITY; POSE_JOINTS];
        for (to, from) in padded.iter_mut().zip(joints) {
            *to = *from;
        }
        gpu.queue.write_buffer(&slot.joints, 0, bytemuck::cast_slice(&padded));
        // The pass that writes the posed vertices, waited for before the
        // build that reads them: wgpu does not order a compute write before
        // an acceleration structure's input (on Metal the build read the
        // buffer before the pass had written it, in one encoder or two).
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pose for rays") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pose for rays"), timestamp_writes: None });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &slot.bind, &[]);
            pass.dispatch_workgroups(vertex_count.div_ceil(64), 1, 1);
        }
        gpu.queue.submit(Some(encoder.finish()));
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("build posed rays") });
        let size = triangles(vertex_count, index_count);
        encoder.build_acceleration_structures(
            std::iter::once(&wgpu::BlasBuildEntry {
                blas: &slot.blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                    size: &size,
                    vertex_buffer: &slot.out,
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
    }

    /// The structure slot `key` was last posed into.
    pub(crate) fn blas(&self, key: (u32, u32)) -> Option<&wgpu::Blas> {
        self.slots.get(&key).map(|s| &s.blas)
    }
}

// Built without the `ray-tracing` feature: no rays' scene can be made, so
// nothing below is ever called — what stands in for it is so the lit pass
// compiles unchanged.
#[cfg(not(feature = "ray-tracing"))]
pub(crate) enum RayScene {}

#[cfg(not(feature = "ray-tracing"))]
impl RayScene {
    pub(crate) fn bindings(&self) -> (&wgpu::Tlas, &wgpu::Buffer) {
        match *self {}
    }

    pub(crate) fn update(
        &mut self,
        _: &Gpu,
        _: &mut wgpu::CommandEncoder,
        _: &[(&wgpu::Blas, glam::Mat4, u8, RayMaterial)],
    ) -> bool {
        match *self {}
    }
}

#[cfg(not(feature = "ray-tracing"))]
pub(crate) enum Posed {}

#[cfg(not(feature = "ray-tracing"))]
impl Posed {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pose(&mut self, _: &Gpu, _: (u32, u32), _: &wgpu::Buffer, _: u32, _: &wgpu::Buffer, _: &wgpu::Buffer, _: u32, _: &[glam::Mat4]) {
        match *self {}
    }

    pub(crate) fn blas(&self, _: (u32, u32)) -> Option<&wgpu::Blas> {
        match *self {}
    }
}

#[cfg(not(feature = "ray-tracing"))]
pub(crate) fn traced(source: &str) -> String {
    source.to_string()
}

#[cfg(not(feature = "ray-tracing"))]
pub(crate) fn blas(_: &Gpu, _: &wgpu::Buffer, _: u32, _: &wgpu::Buffer, _: u32, _: bool) -> wgpu::Blas {
    unreachable!("built without ray tracing: there is no rays' scene to build for")
}

#[cfg(not(feature = "ray-tracing"))]
pub(crate) fn rebuild(_: &Gpu, _: &wgpu::Blas, _: &wgpu::Buffer, _: u32, _: &wgpu::Buffer, _: u32) {
    unreachable!("built without ray tracing: there is no rays' scene to build for")
}

#[cfg(all(test, feature = "ray-tracing"))]
mod tests {
    use super::*;

    #[test]
    fn the_stub_is_replaced_and_the_extension_enabled() {
        let source = "struct A { x: f32 };\n// ray: stub begin\nfn ray_clear() -> f32 { return 1.0; }\n// ray: stub end\nfn after() {}\n";
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
