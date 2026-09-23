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
//! The frame is drawn the way URP draws one: into a high-dynamic-range,
//! multisampled buffer — sky, opaque things, lights with no ceiling on
//! how bright — and then turned into a picture by post-processing
//! ([`crate::post`]): bloom, grading, tonemapping. Tools drawn over the
//! scene (gizmos, outlines) go on after that, onto the finished picture,
//! so no tonemapper or bloom touches a handle's colour.

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
    /// Orthographic, with this many metres from the middle of the image to
    /// its top edge; `None` is perspective. A plan of a level from above,
    /// with no vanishing point: Unity's orthographic camera and the Scene
    /// view's axis views.
    pub ortho: Option<f32>,
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
            ortho: None,
        }
    }
}

impl Camera {
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        // `perspective_rh` and not `_gl`: wgpu's clip space runs z from 0 to
        // 1, and the `_gl` variant's -1..1 would halve the depth buffer.
        let aspect = aspect.max(1e-3);
        let projection = match self.ortho {
            Some(half) => {
                let half = half.max(1e-3);
                Mat4::orthographic_rh(
                    -half * aspect,
                    half * aspect,
                    -half,
                    half,
                    self.near,
                    self.far,
                )
            }
            None => {
                Mat4::perspective_rh(self.fov_y_degrees.to_radians(), aspect, self.near, self.far)
            }
        };
        projection * Mat4::look_at_rh(self.position, self.target, self.up)
    }

    /// How far away a point looks: its distance, or with an orthographic
    /// camera the distance at which the perspective one would show as much.
    /// What keeps a handle or an outline the same size on screen either way.
    pub fn apparent_distance(&self, point: Vec3) -> f32 {
        match self.ortho {
            Some(half) => half / (self.fov_y_degrees.to_radians() * 0.5).tan().max(1e-3),
            None => (point - self.position).length(),
        }
    }

    /// Where fog and shine are measured from: the camera, or for an
    /// orthographic one, where a perspective camera showing as much would
    /// stand — its real place is far back, and the whole plan would drown
    /// in fog.
    pub fn apparent_eye(&self) -> Vec3 {
        match self.ortho {
            Some(_) => {
                let back = (self.position - self.target).normalize_or_zero();
                self.target + back * self.apparent_distance(self.target)
            }
            None => self.position,
        }
    }

    /// The ray through a point of a `size`-pixel image, from the top left —
    /// where a click lands: Unity's `ScreenPointToRay`. As (origin on the
    /// near plane, unit direction); hand it to a physics ray cast.
    pub fn ray_through(&self, pixel: glam::Vec2, size: glam::Vec2) -> (Vec3, Vec3) {
        let ndc_x = pixel.x / size.x.max(1.0) * 2.0 - 1.0;
        let ndc_y = 1.0 - pixel.y / size.y.max(1.0) * 2.0;
        let inverse = self.view_projection(size.x / size.y.max(1.0)).inverse();
        let near = inverse.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let far = inverse.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        (near, (far - near).normalize_or_zero())
    }

    /// Where a point of the world lands in a `size`-pixel image, from the top
    /// left — a name over a head, a marker on a door: `WorldToScreenPoint`.
    /// `None` behind the camera.
    pub fn screen_point(&self, world: Vec3, size: glam::Vec2) -> Option<glam::Vec2> {
        let clip = self.view_projection(size.x / size.y.max(1.0)) * world.extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(glam::Vec2::new(
            (ndc.x + 1.0) * 0.5 * size.x,
            (1.0 - ndc.y) * 0.5 * size.y,
        ))
    }
}

#[cfg(test)]
mod camera_tests {
    use super::*;

    #[test]
    fn an_orthographic_camera_casts_parallel_rays_and_keeps_sizes() {
        let camera = Camera {
            position: Vec3::new(0.0, 200.0, 0.0),
            target: Vec3::ZERO,
            up: Vec3::NEG_Z,
            ortho: Some(10.0),
            ..Camera::default()
        };
        let size = glam::Vec2::new(200.0, 100.0);
        let (a, da) = camera.ray_through(glam::Vec2::new(10.0, 10.0), size);
        let (b, db) = camera.ray_through(glam::Vec2::new(150.0, 90.0), size);
        assert!(da.abs_diff_eq(Vec3::NEG_Y, 1e-4) && db.abs_diff_eq(Vec3::NEG_Y, 1e-4));
        assert!((a - b).length() > 1.0, "from different places");
        // Ten metres up the image is its top edge, and +x is to the right,
        // at any height.
        for y in [0.0, 50.0] {
            let top = camera.screen_point(Vec3::new(0.0, y, -10.0), size).unwrap();
            assert!(top.abs_diff_eq(glam::Vec2::new(100.0, 0.0), 1e-2), "{top}");
            let right = camera.screen_point(Vec3::new(5.0, y, 0.0), size).unwrap();
            assert!(right.x > 100.0);
        }
        // Fog is measured from where a perspective camera would stand.
        let eye = camera.apparent_eye();
        assert!(eye.y > 5.0 && eye.y < 50.0, "{eye}");
    }

    #[test]
    fn a_click_is_a_ray_and_a_point_lands_back_where_it_was_clicked() {
        let camera = Camera::default();
        let size = glam::Vec2::new(1280.0, 720.0);
        let (_, along) = camera.ray_through(size * 0.5, size);
        let straight = (camera.target - camera.position).normalize();
        assert!(
            (along - straight).length() < 1e-4,
            "the middle looks where the camera does"
        );
        let corner = glam::Vec2::new(100.0, 600.0);
        let (origin, along) = camera.ray_through(corner, size);
        let point = origin + along * 7.0;
        let back = camera.screen_point(point, size).unwrap();
        assert!((back - corner).length() < 0.01, "{back:?}");
        assert!(
            camera
                .screen_point(camera.position - straight, size)
                .is_none(),
            "behind"
        );
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

/// How fog thickens with distance — URP's three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FogMode {
    /// None at `start`, all at `end`.
    #[default]
    Linear,
    /// `1 - e^(-density·d)`: thin near, never quite whole.
    Exponential,
    /// `1 - e^(-(density·d)²)`: clear near, closing in fast.
    ExponentialSquared,
}

/// Distance fog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogSettings {
    pub color: Vec3,
    pub start: f32,
    pub end: f32,
    pub mode: FogMode,
    /// For the exponential modes: how thick, per metre.
    pub density: f32,
}

impl Default for FogSettings {
    fn default() -> Self {
        Self {
            color: Vec3::new(0.62, 0.68, 0.74),
            start: 30.0,
            end: 180.0,
            mode: FogMode::Linear,
            density: 0.01,
        }
    }
}

/// What is behind everything: URP's skybox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SkyMode {
    /// A gradient from the horizon to the zenith, the ground below, and
    /// the sun's disc where the sun is: URP's Procedural skybox.
    #[default]
    Procedural,
    /// A flat colour: the frame's `clear_color`. URP's Solid Color.
    Color,
}

/// The sky, in linear light: HDR, so the sun's disc is far past white and
/// blooms.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Sky {
    pub mode: SkyMode,
    /// Straight up.
    pub zenith: [f32; 3],
    /// At the horizon — best the fog's colour, so the far hills fade into
    /// the sky rather than against it.
    pub horizon: [f32; 3],
    /// Below the horizon.
    pub ground: [f32; 3],
    /// The sun's disc, degrees across; 0 hides it.
    pub sun_size: f32,
    /// How bright the whole sky is.
    pub exposure: f32,
}

impl Default for Sky {
    fn default() -> Self {
        Self {
            mode: SkyMode::Procedural,
            zenith: [0.22, 0.38, 0.66],
            horizon: [0.62, 0.68, 0.74],
            ground: [0.30, 0.28, 0.25],
            sun_size: 1.5,
            exposure: 1.0,
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
    /// How far from the camera shadows are drawn, in metres.
    ///
    /// This is the single biggest lever on how a shadow looks. The map has a
    /// fixed number of texels, and they are spread over whatever the frustum
    /// covers — so fitting the light to the whole scene spends most of them
    /// on ground nobody is looking at, and a valley's worth of them leaves
    /// each shadow a staircase. Fitting to the near part of the view instead
    /// is what makes the same map sharp.
    pub max_distance: f32,
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            resolution: 2048,
            depth_bias: 0.0015,
            normal_bias: 0.05,
            max_distance: 40.0,
        }
    }
}

impl ShadowSettings {
    pub const OFF: ShadowSettings = ShadowSettings {
        enabled: false,
        resolution: 1,
        depth_bias: 0.0,
        normal_bias: 0.0,
        max_distance: 0.0,
    };
}

/// The most joints one pose may have.
///
/// Sized so a pose fits comfortably in a uniform buffer on the downlevel
/// limits this engine asks for. A skeleton past this is split or simplified
/// at import; silently dropping joints would put a limb at the origin.
pub const MAX_JOINTS: usize = 64;

/// A skeleton's joints, already turned into skinning matrices.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose(pub Vec<Mat4>);

/// One thing to draw: a mesh, where it is, and what colour it takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Draw {
    pub mesh: MeshHandle,
    pub transform: Mat4,
    /// Multiplied by the material's colour. [`TextureHandle::WHITE`] leaves
    /// the colour alone, which is what an untextured surface wants.
    pub texture: TextureHandle,
    /// Index into [`Frame::poses`], for a skinned mesh. `None` draws the
    /// mesh in its bind pose through the ordinary pipeline.
    pub pose: Option<u32>,
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
    /// What is behind everything; with [`SkyMode::Color`], `clear_color`.
    pub sky: Sky,
    pub clear_color: Vec3,
    /// What is done to the finished frame: bloom, grading, tonemapping.
    pub post: crate::post::PostProcess,
    pub draws: Vec<Draw>,
    /// Drawn after everything else with the depth test off, so they are
    /// never hidden by the scene.
    ///
    /// For handles, outlines and anything else that is a tool rather than a
    /// thing in the world. A gizmo half inside the object it sits on is the
    /// reason this exists: you cannot grab what you cannot see, and burying
    /// it is exactly what depth testing does.
    pub overlay_draws: Vec<Draw>,
    /// Point lights besides the sun.
    pub lights: Vec<PointLight>,
    /// Skinning matrices, one entry per animated thing on screen. Held here
    /// rather than on each draw so that two draws sharing a skeleton share
    /// one upload.
    pub poses: Vec<Pose>,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            camera: Camera::default(),
            lighting: Lighting::default(),
            fog: FogSettings::default(),
            shadows: ShadowSettings::default(),
            sky: Sky::default(),
            clear_color: Vec3::new(0.62, 0.68, 0.74),
            post: crate::post::PostProcess::default(),
            draws: Vec::new(),
            overlay_draws: Vec::new(),
            lights: Vec::new(),
            poses: Vec::new(),
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
    /// `start`, `end`, the mode (0 linear, 1 exponential, 2 squared) and
    /// the density.
    fog_range: [f32; 4],
    camera_position: [f32; 4],
    /// World space to the shadow map's clip space.
    light_view_projection: [[f32; 4]; 4],
    /// `depth_bias`, `normal_bias`, texel size in world units, and `1.0` when
    /// shadows are on. The last one is what lets the shader skip the lookup
    /// without a second pipeline.
    shadow_params: [f32; 4],
    /// Up to [`MAX_LIGHTS`] lights, three vectors each: where and how far
    /// it reaches; its colour times its intensity; for a spot, which way it
    /// shines and the cosine of half its cone (−2 for every way).
    lights: [[f32; 4]; MAX_LIGHTS * 3],
    /// How many of those are on, in `x`.
    light_count: [f32; 4],
    /// Clip space back to the world: the sky is drawn by asking, for each
    /// pixel, which way it looks.
    inverse_view_projection: [[f32; 4]; 4],
    /// Zenith colour; `w` is 1 for a procedural sky.
    sky_zenith: [f32; 4],
    /// Horizon colour; `w` is the cosine of the sun disc's radius.
    sky_horizon: [f32; 4],
    /// Ground colour; `w` is the sky's exposure.
    sky_ground: [f32; 4],
}

/// The most point lights a frame lights with; the nearest to the camera
/// win when there are more.
pub const MAX_LIGHTS: usize = 8;

/// A light at a point, fading to nothing at `range` — a campfire, a lamp,
/// a torch. Unity's Point Light, without shadows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointLight {
    pub position: Vec3,
    /// Linear, already times its intensity.
    pub color: Vec3,
    pub range: f32,
    /// For a spot light: which way it shines, and its cone, degrees
    /// across. `None` shines every way.
    pub spot: Option<(Vec3, f32)>,
}

/// One vertex's binding to the skeleton, in its own buffer.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SkinVertex {
    joints: [u16; 4],
    weights: [f32; 4],
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
    /// Joint indices and weights, when the mesh has them.
    skin: Option<wgpu::Buffer>,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Kept so the shadow pass can fit its frustum to what is actually being
    /// drawn. A map spread over an empty hundred metres wastes every texel.
    bounds: crate::asset::Bounds,
}

/// Holds the pipeline, the uploaded meshes and the buffers a frame needs.
pub struct Renderer {
    pipelines: Pipelines,
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
    /// Samples per pixel the scene is drawn with: 4 where the device can.
    samples: u32,
    /// The scene, in high dynamic range: multisampled, and resolved.
    scene: SceneTargets,
    post: crate::post::PostRenderer,
    pose_layout: wgpu::BindGroupLayout,
    pose_bind_group: wgpu::BindGroup,
    poses: wgpu::Buffer,
    /// One pose's slot, padded up to the device's dynamic-offset alignment.
    pose_stride: u64,
    pose_capacity: u64,
    stats: FrameStats,
    meshes: Vec<GpuMesh>,
    textures: Vec<GpuTexture>,
    texture_layout: wgpu::BindGroupLayout,
    texture_sampler: wgpu::Sampler,
    /// Kept to rebuild the pipelines when the shader is reloaded.
    pipeline_layout: wgpu::PipelineLayout,
    shadow_pipeline_layout: wgpu::PipelineLayout,
    skinned_layout: wgpu::PipelineLayout,
    sky_layout: wgpu::PipelineLayout,
}

/// Where the scene is drawn before post-processing turns it into a picture.
struct SceneTargets {
    /// Drawn into, `samples` per pixel; `None` when there is one sample.
    multisampled: Option<wgpu::TextureView>,
    /// Resolved into: what post-processing reads.
    resolved: wgpu::TextureView,
}

fn scene_targets(gpu: &Gpu, width: u32, height: u32, samples: u32) -> SceneTargets {
    let make = |label, samples, usage| {
        gpu.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: width.max(1),
                    height: height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: crate::post::HDR_FORMAT,
                usage,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    };
    SceneTargets {
        multisampled: (samples > 1).then(|| {
            make(
                "scene (multisampled)",
                samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        }),
        resolved: make(
            "scene",
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        ),
    }
}

/// Four samples per pixel where the device can draw the HDR format and
/// depth that way, one where it cannot. DNA, postulate 7: antialiasing is
/// on without anyone asking.
fn sample_count(gpu: &Gpu) -> u32 {
    let can = |format| {
        gpu.adapter
            .get_texture_format_features(format)
            .flags
            .sample_count_supported(4)
    };
    if can(crate::post::HDR_FORMAT) && can(DEPTH_FORMAT) {
        4
    } else {
        1
    }
}

/// The engine's shader, as compiled in.
pub const SHADER: &str = include_str!("render.wgsl");

/// Where the engine's shader source was when the engine was built — for
/// watching it while working on the engine; see [`ShaderFile`].
pub const SHADER_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/render.wgsl");

/// A shader source file, reloaded into a renderer when it changes. DNA,
/// postulate 1: shaders reload too.
pub struct ShaderFile {
    path: std::path::PathBuf,
    stamp: Option<std::time::SystemTime>,
}

impl ShaderFile {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        let path = path.into();
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        Self { path, stamp }
    }

    /// Reload the shader if the file changed: `None` when it did not, and
    /// the compiler's words — file, line, column — when it does not build.
    /// The renderer keeps the last shader that did.
    pub fn poll(&mut self, renderer: &mut Renderer, gpu: &Gpu) -> Option<Result<(), String>> {
        let now = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok();
        if now == self.stamp {
            return None;
        }
        self.stamp = now;
        let source = match std::fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(e) => return Some(Err(format!("{}: {e}", self.path.display()))),
        };
        Some(
            renderer
                .reload_shader(gpu, &source)
                .map_err(|e| format!("{}:\n{e}", self.path.display())),
        )
    }
}

/// Every pipeline the renderer draws with.
struct Pipelines {
    main: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    skinned: wgpu::RenderPipeline,
    overlay: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
}

/// The layouts the pipelines are built against, kept to rebuild them when
/// the shader is reloaded.
struct Layouts<'a> {
    main: &'a wgpu::PipelineLayout,
    shadow: &'a wgpu::PipelineLayout,
    skinned: &'a wgpu::PipelineLayout,
    sky: &'a wgpu::PipelineLayout,
}

/// Every pipeline the renderer draws with, from one shader module: at
/// start, and again when the shader is reloaded. The scene draws into the
/// HDR format, multisampled `samples` times; overlays onto `output`, after
/// post-processing.
fn build_pipelines(
    gpu: &Gpu,
    shader: &wgpu::ShaderModule,
    output: wgpu::TextureFormat,
    samples: u32,
    layouts: &Layouts,
) -> Pipelines {
    let format = crate::post::HDR_FORMAT;
    let (pipeline_layout, shadow_pipeline_layout, skinned_layout) =
        (layouts.main, layouts.shadow, layouts.skinned);
    let multisample = wgpu::MultisampleState {
        count: samples,
        ..Default::default()
    };
    let pipeline = gpu
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("runity::render"),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
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
                module: shader,
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
            multisample,
            multiview_mask: None,
            cache: None,
        });

    // Depth only: no fragment stage at all, because nothing is written
    // but depth and a colour target would only cost fill.
    let shadow_pipeline = gpu
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("runity::shadow"),
            layout: Some(shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
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

    let skinned_pipeline = gpu
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("runity::skinned"),
            layout: Some(skinned_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_skinned"),
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
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<SkinVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![8 => Uint16x4, 9 => Float32x4],
                    }),
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState {
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
            multisample,
            multiview_mask: None,
            cache: None,
        });

    // The same shader and the same vertex layout, with the depth test
    // turned off and depth writes suppressed — so an overlay neither
    // hides behind the scene nor blocks anything drawn after it.
    let overlay_pipeline = gpu
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("runity::overlay"),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
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
                module: shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(output.into())],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

    // The sky: one triangle over the screen at the far plane, drawn after
    // the opaque things so only what they left uncovered is shaded.
    let sky = gpu
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("runity::sky"),
            layout: Some(layouts.sky),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_sky"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample,
            multiview_mask: None,
            cache: None,
        });

    Pipelines {
        main: pipeline,
        shadow: shadow_pipeline,
        skinned: skinned_pipeline,
        overlay: overlay_pipeline,
        sky,
    }
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

impl Renderer {
    /// Build a renderer for an offscreen target.
    pub fn new(gpu: &Gpu, target: &OffscreenTarget) -> Self {
        Self::with_format(gpu, target.format, target.width, target.height)
    }

    /// Draw with another shader from now on: `source` is WGSL with the
    /// entry points and bindings of [`SHADER`] — `vs`, `fs`, `vs_shadow`,
    /// `vs_skinned`, `vs_sky`, `fs_sky`.
    ///
    /// Checked before anything changes: parsed and validated, with the line
    /// and column of what is wrong, and then built under an error scope, so
    /// a shader that compiles but does not fit the pipelines is refused in
    /// words too. Either way the old shader keeps drawing — a typo saved
    /// mid-edit costs a message, not a black screen or a crash.
    pub fn reload_shader(&mut self, gpu: &Gpu, source: &str) -> Result<(), String> {
        use wgpu::naga;
        let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|e| e.emit_to_string(source))?;

        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("runity::render (reloaded)"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let pipelines = build_pipelines(
            gpu,
            &shader,
            self.format,
            self.samples,
            &Layouts {
                main: &self.pipeline_layout,
                shadow: &self.shadow_pipeline_layout,
                skinned: &self.skinned_layout,
                sky: &self.sky_layout,
            },
        );
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(format!("the shader does not fit the renderer: {error}"));
        }
        self.pipelines = pipelines;
        Ok(())
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
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
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

        // Skinning: a second pipeline, an extra vertex buffer of joint
        // bindings, and one bind group of matrices switched per draw with a
        // dynamic offset.
        let pose_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pose"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(
                            (MAX_JOINTS * std::mem::size_of::<[[f32; 4]; 4]>()) as u64,
                        ),
                    },
                    count: None,
                }],
            });
        let alignment = gpu
            .device
            .limits()
            .min_uniform_buffer_offset_alignment
            .max(1) as u64;
        let pose_size = (MAX_JOINTS * std::mem::size_of::<[[f32; 4]; 4]>()) as u64;
        let pose_stride = pose_size.div_ceil(alignment) * alignment;
        let pose_capacity = 16;
        let poses = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("poses"),
            size: pose_stride * pose_capacity,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pose_bind_group = pose_bind_group(gpu, &pose_layout, &poses, pose_size);

        let skinned_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("runity::skinned"),
                bind_group_layouts: &[Some(&layout), Some(&texture_layout), Some(&pose_layout)],
                immediate_size: 0,
            });

        let sky_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("runity::sky"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let samples = sample_count(gpu);
        let pipelines = build_pipelines(
            gpu,
            &shader,
            format,
            samples,
            &Layouts {
                main: &pipeline_layout,
                shadow: &shadow_pipeline_layout,
                skinned: &skinned_layout,
                sky: &sky_layout,
            },
        );

        let instance_capacity = 256;
        let instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: instance_capacity * std::mem::size_of::<InstanceRaw>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut renderer = Self {
            pipelines,
            layout,
            bind_group,
            shadow_bind_group,
            frame_buffer,
            instances,
            instance_capacity,
            depth: depth_view(gpu, width, height, samples),
            depth_size: (width, height),
            samples,
            scene: scene_targets(gpu, width, height, samples),
            post: crate::post::PostRenderer::new(gpu, format),
            shadow_map,
            shadow_sampler,
            shadow_resolution,
            format,
            pose_layout,
            pose_bind_group,
            poses,
            pose_stride,
            pose_capacity,
            stats: FrameStats::default(),
            meshes: Vec::new(),
            textures: Vec::new(),
            texture_layout,
            texture_sampler,
            pipeline_layout,
            shadow_pipeline_layout,
            skinned_layout,
            sky_layout,
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
        let handle = self.upload(gpu, vertex_slice(mesh), &indices);
        if let Some(skin) = mesh.skin.as_ref() {
            let bindings: Vec<SkinVertex> = skin
                .joints
                .iter()
                .zip(skin.weights.iter())
                .map(|(joints, weights)| SkinVertex {
                    joints: [
                        joints[0].to_native(),
                        joints[1].to_native(),
                        joints[2].to_native(),
                        joints[3].to_native(),
                    ],
                    weights: [
                        weights[0].to_native(),
                        weights[1].to_native(),
                        weights[2].to_native(),
                        weights[3].to_native(),
                    ],
                })
                .collect();
            self.attach_skin(gpu, handle, &bindings);
        }
        handle
    }

    /// Give an uploaded mesh its per-vertex joint bindings.
    fn attach_skin(&mut self, gpu: &Gpu, handle: MeshHandle, bindings: &[SkinVertex]) {
        use wgpu::util::DeviceExt;
        let buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("skin"),
                contents: bytemuck::cast_slice(bindings),
                usage: wgpu::BufferUsages::VERTEX,
            });
        if let Some(mesh) = self.meshes.get_mut(handle.0 as usize) {
            mesh.skin = Some(buffer);
        }
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
            skin: None,
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

    /// The sphere the shadow map covers: the near part of what the camera
    /// can see, not the whole scene.
    ///
    /// A sphere rather than a box, and centred on the view rather than on
    /// the world, for one reason: the size of what the map covers must not
    /// change as the camera turns. A box fitted to the frustum's corners
    /// grows and shrinks with every rotation, and the shadows crawl as its
    /// texels resize under them.
    fn shadow_sphere(&self, frame: &Frame, aspect: f32) -> Option<(Vec3, f32)> {
        let camera = &frame.camera;
        if let Some(half) = camera.ortho {
            // Everything seen is equally near: the shadows cover the image
            // around what the camera looks at, however far back it stands.
            let radius = (half * glam::Vec2::new(aspect, 1.0)).length();
            return Some((
                camera.target,
                radius.min(frame.shadows.max_distance).max(0.01),
            ));
        }
        let far = frame.shadows.max_distance.min(camera.far).max(camera.near);
        let forward = (camera.target - camera.position).normalize_or_zero();
        if forward.length_squared() < 0.5 {
            return None;
        }
        let up = camera.up.normalize_or_zero();
        let right = forward.cross(up).normalize_or_zero();
        let up = right.cross(forward);

        let tan = (camera.fov_y_degrees.to_radians() * 0.5).tan();
        let mut corners = Vec::with_capacity(8);
        for distance in [camera.near, far] {
            let half_height = tan * distance;
            let half_width = half_height * aspect;
            let centre = camera.position + forward * distance;
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                corners.push(centre + right * half_width * sx + up * half_height * sy);
            }
        }
        let centre = corners.iter().copied().sum::<Vec3>() / corners.len() as f32;
        let radius = corners
            .iter()
            .map(|c| (*c - centre).length())
            .fold(0.0f32, f32::max);
        Some((centre, radius.max(0.01)))
    }

    /// An orthographic frustum along the sun, fitted to what the camera can
    /// actually see.
    fn fit_light_frustum(&self, frame: &Frame, sun: Vec3, aspect: f32) -> Mat4 {
        let Some((centre, radius)) = self.shadow_sphere(frame, aspect) else {
            return Mat4::IDENTITY;
        };
        if sun.length_squared() < 0.5 {
            return Mat4::IDENTITY;
        }

        // A sun straight overhead makes the usual up vector degenerate.
        let up = if sun.dot(Vec3::Y).abs() > 0.99 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        // Pulled back far enough that casters behind the view still land in
        // the map: something off screen is often the thing casting the
        // shadow you are looking at. The scene's own extent is how far back
        // that has to be — anything nearer clips a caster and loses its
        // shadow, which reads as an object that does not cast one.
        let behind = self
            .scene_bounds(frame)
            .map_or(radius * 2.0, |(min, max)| (max - min).length())
            .max(radius * 2.0);
        let eye = centre - sun * behind;
        let view = Mat4::look_at_rh(eye, centre, up);
        let projection = Mat4::orthographic_rh(
            -radius,
            radius,
            -radius,
            radius,
            0.01,
            behind + radius * 2.0,
        );
        projection * view
    }

    /// How much world a single shadow texel covers.
    pub fn shadow_texel_size(&self, frame: &Frame, aspect: f32) -> f32 {
        match self.shadow_sphere(frame, aspect) {
            Some((_, radius)) => 2.0 * radius / self.shadow_resolution.max(1) as f32,
            None => 0.0,
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
        if self.depth_size != (width, height) {
            self.depth = depth_view(gpu, width, height, self.samples);
            self.scene = scene_targets(gpu, width, height, self.samples);
            self.depth_size = (width, height);
        }

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
            self.fit_light_frustum(frame, sun, aspect)
        } else {
            Mat4::IDENTITY
        };
        // Half the world span one texel covers, which is what the normal
        // offset has to clear at a grazing angle.
        let texel_world = self.shadow_texel_size(frame, aspect);

        let uniform = FrameUniform {
            view_projection: frame.camera.view_projection(aspect).to_cols_array_2d(),
            sun_direction: extend(frame.lighting.sun_direction.normalize_or_zero(), 0.0),
            sun_color: extend(frame.lighting.sun_color * frame.lighting.sun_intensity, 0.0),
            sky_color: extend(frame.lighting.sky_color, 0.0),
            ground_color: extend(frame.lighting.ground_color, 0.0),
            fog_color: extend(frame.fog.color, 0.0),
            fog_range: [
                frame.fog.start,
                frame.fog.end,
                match frame.fog.mode {
                    FogMode::Linear => 0.0,
                    FogMode::Exponential => 1.0,
                    FogMode::ExponentialSquared => 2.0,
                },
                frame.fog.density.max(0.0),
            ],
            camera_position: extend(frame.camera.apparent_eye(), 1.0),
            light_view_projection: light_view_projection.to_cols_array_2d(),
            shadow_params: [
                frame.shadows.depth_bias,
                frame.shadows.normal_bias + texel_world,
                1.0 / self.shadow_resolution as f32,
                if frame.shadows.enabled { 1.0 } else { 0.0 },
            ],
            lights: {
                let mut near: Vec<&PointLight> = frame.lights.iter().collect();
                let eye = frame.camera.position;
                near.sort_by(|a, b| {
                    (a.position - eye)
                        .length_squared()
                        .total_cmp(&(b.position - eye).length_squared())
                });
                let mut out = [[0.0; 4]; MAX_LIGHTS * 3];
                for (i, light) in near.iter().take(MAX_LIGHTS).enumerate() {
                    out[i * 3] = extend(light.position, light.range.max(0.01));
                    out[i * 3 + 1] = extend(light.color, 0.0);
                    out[i * 3 + 2] = match light.spot {
                        Some((direction, cone)) => extend(
                            direction.normalize_or_zero(),
                            (cone.clamp(1.0, 179.0).to_radians() * 0.5).cos(),
                        ),
                        None => [0.0, 0.0, 0.0, -2.0],
                    };
                }
                out
            },
            light_count: [frame.lights.len().min(MAX_LIGHTS) as f32, 0.0, 0.0, 0.0],
            inverse_view_projection: frame
                .camera
                .view_projection(aspect)
                .inverse()
                .to_cols_array_2d(),
            sky_zenith: [
                frame.sky.zenith[0],
                frame.sky.zenith[1],
                frame.sky.zenith[2],
                if frame.sky.mode == SkyMode::Procedural {
                    1.0
                } else {
                    0.0
                },
            ],
            sky_horizon: [
                frame.sky.horizon[0],
                frame.sky.horizon[1],
                frame.sky.horizon[2],
                (frame.sky.sun_size.max(0.0).to_radians() * 0.5).cos(),
            ],
            sky_ground: [
                frame.sky.ground[0],
                frame.sky.ground[1],
                frame.sky.ground[2],
                frame.sky.exposure.max(0.0),
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
        let mut skinned_draws: Vec<(MeshHandle, TextureHandle, u32, InstanceRaw)> = Vec::new();
        let mut stats = FrameStats {
            submitted: frame.draws.len() as u32,
            ..Default::default()
        };
        for draw in &frame.draws {
            let unlit = match draw.material.shading {
                Shading::Lit => 0.0,
                Shading::Unlit => 1.0,
                Shading::Grid => 2.0,
            };
            let raw = InstanceRaw {
                model: draw.transform.to_cols_array_2d(),
                color_and_shading: extend(draw.material.color(), unlit),
            };
            let key = (draw.mesh, draw.texture);
            push(&mut shadow_batches, key, raw);
            stats.shadow_casters += 1;

            let skinned = draw.pose.is_some()
                && self
                    .meshes
                    .get(draw.mesh.0 as usize)
                    .is_some_and(|m| m.skin.is_some());
            let visible = match self.meshes.get(draw.mesh.0 as usize) {
                Some(mesh) => aabb_in_frustum(&planes, mesh.bounds, draw.transform),
                // A handle pointing at nothing draws nothing; it should not
                // also be reported as culled.
                None => false,
            };
            if !visible {
                stats.culled += 1;
                continue;
            }
            stats.drawn += 1;
            if skinned {
                // Skinned draws are not batched: each one has its own pose,
                // so two of them cannot share an instanced call anyway.
                skinned_draws.push((draw.mesh, draw.texture, draw.pose.unwrap_or(0), raw));
            } else {
                push(&mut batches, key, raw);
            }
        }
        self.stats = stats;

        let shadow_total: u64 = shadow_batches.iter().map(|(_, l)| l.len() as u64).sum();
        let total: u64 = shadow_total
            + batches.iter().map(|(_, l)| l.len() as u64).sum::<u64>()
            + skinned_draws.len() as u64
            + frame.overlay_draws.len() as u64;
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
        // Overlays are not culled and cast no shadow: they are tools, not
        // things in the world.
        let mut overlay_batches: Vec<((MeshHandle, TextureHandle), Vec<InstanceRaw>)> = Vec::new();
        for draw in &frame.overlay_draws {
            let unlit = match draw.material.shading {
                Shading::Lit => 0.0,
                Shading::Unlit => 1.0,
                Shading::Grid => 2.0,
            };
            push(
                &mut overlay_batches,
                (draw.mesh, draw.texture),
                InstanceRaw {
                    model: draw.transform.to_cols_array_2d(),
                    color_and_shading: extend(draw.material.color(), unlit),
                },
            );
        }

        let flat_colour_count: u32 = batches.iter().map(|(_, l)| l.len() as u32).sum();
        let flat: Vec<InstanceRaw> = shadow_batches
            .iter()
            .chain(batches.iter())
            .flat_map(|(_, l)| l.iter().copied())
            .chain(skinned_draws.iter().map(|(_, _, _, raw)| *raw))
            .chain(overlay_batches.iter().flat_map(|(_, l)| l.iter().copied()))
            .collect();
        if !flat.is_empty() {
            gpu.queue
                .write_buffer(&self.instances, 0, bytemuck::cast_slice(&flat));
        }

        // Poses go in before the pass: one slot each, padded to the device's
        // dynamic-offset alignment, and a pose longer than MAX_JOINTS is
        // truncated rather than overrunning its neighbour's slot.
        if !frame.poses.is_empty() {
            if frame.poses.len() as u64 > self.pose_capacity {
                self.pose_capacity = (frame.poses.len() as u64).next_power_of_two();
                self.poses = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("poses"),
                    size: self.pose_stride * self.pose_capacity,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let pose_size = (MAX_JOINTS * std::mem::size_of::<[[f32; 4]; 4]>()) as u64;
                self.pose_bind_group =
                    pose_bind_group(gpu, &self.pose_layout, &self.poses, pose_size);
            }
            for (slot, pose) in frame.poses.iter().enumerate() {
                let mut matrices = [[[0.0f32; 4]; 4]; MAX_JOINTS];
                for (i, matrix) in pose.0.iter().take(MAX_JOINTS).enumerate() {
                    matrices[i] = matrix.to_cols_array_2d();
                }
                // Anything past the pose's own joints stays identity, so a
                // stray index reads as "no movement" rather than as a
                // collapse to the origin.
                for slot in matrices.iter_mut().skip(pose.0.len().min(MAX_JOINTS)) {
                    *slot = Mat4::IDENTITY.to_cols_array_2d();
                }
                gpu.queue.write_buffer(
                    &self.poses,
                    slot as u64 * self.pose_stride,
                    bytemuck::cast_slice(&matrices),
                );
            }
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
            pass.set_pipeline(&self.pipelines.shadow);
            pass.set_bind_group(0, &self.shadow_bind_group, &[]);
            self.draw_batches(&mut pass, &shadow_batches, 0, false);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("runity::render"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self
                        .scene
                        .multisampled
                        .as_ref()
                        .unwrap_or(&self.scene.resolved),
                    depth_slice: None,
                    resolve_target: self
                        .scene
                        .multisampled
                        .as_ref()
                        .map(|_| &self.scene.resolved),
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
            pass.set_pipeline(&self.pipelines.main);
            pass.set_bind_group(0, &self.bind_group, &[]);
            self.draw_batches(&mut pass, &batches, shadow_total as u32, true);

            if !skinned_draws.is_empty() {
                pass.set_pipeline(&self.pipelines.skinned);
                pass.set_bind_group(0, &self.bind_group, &[]);
                let mut instance = shadow_total as u32 + flat_colour_count;
                for (mesh_handle, texture, pose, _) in &skinned_draws {
                    let Some(mesh) = self.meshes.get(mesh_handle.0 as usize) else {
                        continue;
                    };
                    let Some(skin) = mesh.skin.as_ref() else {
                        continue;
                    };
                    if let Some(bound) = self
                        .textures
                        .get(texture.0 as usize)
                        .or_else(|| self.textures.first())
                    {
                        pass.set_bind_group(1, &bound.bind_group, &[]);
                    }
                    pass.set_bind_group(
                        2,
                        &self.pose_bind_group,
                        &[*pose * self.pose_stride as u32],
                    );
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_vertex_buffer(1, self.instances.slice(..));
                    pass.set_vertex_buffer(2, skin.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, instance..instance + 1);
                    instance += 1;
                }
            }

            // The sky last among what is solid: only where nothing was
            // drawn is it shaded at all.
            if frame.sky.mode == SkyMode::Procedural {
                pass.set_pipeline(&self.pipelines.sky);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        self.post.run(
            gpu,
            &mut encoder,
            &self.scene.resolved,
            view,
            (width, height),
            &frame.post,
        );

        // Tools go on the finished picture: no tonemapper, bloom or
        // vignette touches a handle's colour.
        if !overlay_batches.is_empty() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("runity::overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipelines.overlay);
            pass.set_bind_group(0, &self.bind_group, &[]);
            let base = shadow_total as u32 + flat_colour_count + skinned_draws.len() as u32;
            self.draw_batches(&mut pass, &overlay_batches, base, true);
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

fn pose_bind_group(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    poses: &wgpu::Buffer,
    pose_size: u64,
) -> wgpu::BindGroup {
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pose"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: poses,
                offset: 0,
                // Bound to one pose's worth, not the whole buffer: the
                // dynamic offset picks which one.
                size: wgpu::BufferSize::new(pose_size),
            }),
        }],
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

fn depth_view(gpu: &Gpu, width: u32, height: u32, samples: u32) -> wgpu::TextureView {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
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
