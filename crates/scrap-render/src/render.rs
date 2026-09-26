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
use crate::material::{Blend, Material, RenderFace, Shading};

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

    /// A normal map that bends nothing: straight up in tangent space. What
    /// a surface without one samples.
    pub const FLAT_NORMAL: TextureHandle = TextureHandle(1);
}

/// A surface's maps, as bound: base, normal, mask, emission, then the
/// four textures its material hands its own shader (white where none).
type Maps = [TextureHandle; 8];

/// Textures a material's own shader reads, at most: its `texture_at`
/// slots.
pub const MATERIAL_TEXTURES: usize = 4;

/// What a material shader's `// scrap:textures` line names, in slot
/// order: the names its material's textures go by (Unity's property
/// names), at most [`MATERIAL_TEXTURES`] of them.
pub fn declared_textures(shader: &str) -> Vec<String> {
    shader
        .lines()
        .find_map(|l| l.trim().strip_prefix("// scrap:textures"))
        .map(|rest| {
            rest.split_whitespace()
                .take(MATERIAL_TEXTURES)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// An instance's eight map handles, two to a number: the four maps in
/// the low halves, the material's own textures in the high — what the
/// bindless shader indexes by ([`crate::bindless`]; its array holds
/// fewer than 65536). Without bindless the shader does not read them.
fn packed(maps: Maps) -> [u32; 4] {
    std::array::from_fn(|i| (maps[i].0 & 0xffff) | (maps[i + 4].0 << 16))
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
    /// What the sun's colour temperature does to its colour, linear:
    /// Unity's light Temperature, a filter over `sun_color` (URP uses
    /// it on every light). White when it has none.
    pub sun_filter: Vec3,
    pub sun_intensity: f32,
    /// Ambient seen by a surface facing straight up.
    pub sky_color: Vec3,
    /// Ambient seen by a surface facing straight down — bounce off the
    /// ground, standing in for the global illumination we do not compute.
    pub ground_color: Vec3,
    /// The ground's colour, linear: with a physical sky, what the light
    /// from below is worked out from.
    pub ground_albedo: Vec3,
    /// At dusk and night, the sun where it really is (under the horizon)
    /// and how bright it is, for the sky to be lit by — the light above,
    /// `sun_direction`, is then the moon's.
    pub sky_sun: Option<(Vec3, f32)>,
    /// How much it is night, 0 to 1: the stars come out.
    pub night: f32,
    /// The light from all round as the scene says it, linear: above, at
    /// the horizon, below. Set, it is what faces see instead of the sky
    /// and the ground's bounce.
    pub ambient: Option<[Vec3; 3]>,
    /// How dark the sun's shadow is, 0 to 1: 1 none of its light gets in,
    /// 0.8 a fifth does — Unity's light Strength under Shadows.
    pub sun_shadow_strength: f32,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            sun_direction: Vec3::new(-0.35, -0.85, -0.4).normalize(),
            sun_color: Vec3::new(1.0, 0.96, 0.88),
            sun_filter: Vec3::ONE,
            sun_intensity: 1.15,
            sky_color: Vec3::new(0.24, 0.28, 0.34),
            ground_color: Vec3::new(0.10, 0.09, 0.07),
            ground_albedo: Vec3::new(0.107, 0.089, 0.069),
            sky_sun: None,
            night: 0.0,
            ambient: None,
            sun_shadow_strength: 1.0,
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
    /// Unity's Procedural skybox, as every URP scene starts with: the air
    /// a thin shell the sun lights ([`crate::procedural_sky`]) — deep blue
    /// overhead and pale at the horizon, warmer as the sun goes down, the
    /// sky's `ground` below and the sun's disc where the sun is. `tint`,
    /// `thickness`, `ground`, `exposure` and `sun_size` are its knobs.
    Procedural,
    /// A gradient from the horizon to the zenith, the ground below, and
    /// the sun's disc where the sun is. Its colours are the scene's to
    /// pick, for a look the air would not give.
    Gradient,
    /// A flat colour: the frame's `clear_color`. URP's Solid Color.
    Color,
    /// Sunlight scattered by the air: blue at noon, orange at sunset, with
    /// the sun's colour and the light from all round worked out from it —
    /// HDRP's Physically Based Sky ([`crate::atmosphere`]). The default:
    /// the hour alone gives a sky, a sun and a light that agree.
    #[default]
    Physical,
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
    /// For [`SkyMode::Procedural`], linear: which light the air scatters
    /// most — middle grey (0.214) is Unity's blue sky, more red a warmer
    /// one. Unity's Sky Tint.
    pub tint: [f32; 3],
    /// For [`SkyMode::Procedural`]: how much air, 1 Earth's; thicker is
    /// bluer overhead and redder at the horizon. Unity's Atmosphere
    /// Thickness.
    pub thickness: f32,
    /// The air, for [`SkyMode::Physical`].
    pub atmosphere: crate::atmosphere::Atmosphere,
    /// Clouds over it ([`crate::clouds`]); none by default.
    pub clouds: crate::clouds::Clouds,
    /// How much of the sky polished things reflect: Unity's Environment
    /// Reflections Intensity Multiplier (the scene's lighting settings).
    pub reflection_intensity: f32,
}

impl Default for Sky {
    fn default() -> Self {
        Self {
            mode: SkyMode::Physical,
            zenith: [0.22, 0.38, 0.66],
            horizon: [0.62, 0.68, 0.74],
            ground: [0.30, 0.28, 0.25],
            sun_size: 1.5,
            exposure: 1.0,
            tint: [0.214, 0.214, 0.214],
            thickness: 1.0,
            atmosphere: crate::atmosphere::Atmosphere::default(),
            clouds: crate::clouds::Clouds::default(),
            reflection_intensity: 1.0,
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
    /// Triangles the colour pass draws, at the levels of detail picked.
    pub triangles: u64,
    /// The sun's shadow cascades drawn this frame: fewer than the frame
    /// has when the far ones are drawn in turn.
    pub cascades_drawn: u32,
    /// Batches of the colour pass: draws that share mesh, look (and,
    /// without bindless, maps) go in one.
    pub batches: u32,
}

/// How shadows are cast, or that they are not: URP's main light shadows,
/// with its names and its meaning — a scene says `shadows: (...)`.
///
/// A shadow map is the cheapest way to make something touch the ground, and
/// nothing else in a renderer does as much for how a frame reads. Everything
/// here is a knob because the right value depends on the scene's size: a
/// bias that works over a hundred metres stripes a room.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ShadowSettings {
    pub enabled: bool,
    /// Side of each cascade's square depth map. 2048 is enough for a
    /// valley; 1024 for a clearing; more costs memory and fill, not
    /// detail, once the map is finer than the screen. (URP packs its
    /// cascades into one atlas: four in a 2048 atlas are 1024 each.)
    pub resolution: u32,
    /// URP's Depth Bias, in texels of the cascade: the caster is pushed
    /// that far away from the sun as it is drawn into the map, so a surface
    /// does not shadow itself. Too little gives acne, too much makes
    /// shadows float free of what casts them ("peter-panning").
    pub depth_bias: f32,
    /// URP's Normal Bias, in texels of the cascade: the caster is shrunk
    /// that far into itself along its normals as it is drawn, most where it
    /// is edge-on to the sun — where the depth error grows with the slope
    /// and no depth bias is enough. A thin pole's shadow thins with it.
    ///
    /// Both biases grow with the soft filter's reach, as URP's do: its
    /// kernel looks further from the centre texel.
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
    /// URP's Cascade Count, 1 to [`MAX_CASCADES`]: the view is cut into
    /// that many slices along its depth, each with a map of its own, so
    /// the near ground gets fine texels and the far ground coarse ones —
    /// sharp at your feet and still there at the shadow distance.
    pub cascades: u32,
    /// Where the first cascades end, as shares of `max_distance`; the last
    /// ends at it. URP's defaults for four.
    pub cascade_splits: [f32; 3],
    /// URP's Last Border: the share of the shadow distance over which the
    /// shadow fades out before it ends (linearly in the squared distance,
    /// as URP does it).
    pub cascade_border: f32,
    /// How the edge of a shadow is filtered: URP's Soft Shadows quality.
    pub soft: SoftShadows,
    /// Side of each lamp's shadow map — URP's Additional Lights shadow
    /// resolution. A spot has one, a point six.
    pub light_resolution: u32,
    /// Contact shadows: metres a short ray from each point toward the sun
    /// is marched through the depth of what is on the screen, for the small
    /// dark where things meet — a cup on a table, a foot on the ground —
    /// that a cascade's texel is too coarse to hold. 0 is none. URP has
    /// none.
    pub contact: f32,
    /// The sun's shadow from virtual shadow maps instead of the cascades
    /// ([`crate::vsm`]): pages of 1.5 cm texels near, coarser far, each
    /// drawn once and kept. `max_distance` still bounds it; `resolution`
    /// is the pool's side.
    pub virtual_maps: bool,
}

/// How a shadow's edge is filtered: URP's hard shadows and its three soft
/// shadow qualities, each a tent over more texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SoftShadows {
    /// One comparison, bilinear between the four texels around it.
    Hard,
    /// Four taps half a texel out: a 3x3 tent.
    Low,
    /// A 5x5 tent, nine taps.
    #[default]
    Medium,
    /// A 7x7 tent, sixteen taps.
    High,
}

impl SoftShadows {
    /// How far the filter reaches, in texels: what URP scales both biases
    /// by (`ShadowUtils.GetShadowBias`).
    pub fn kernel_radius(self) -> f32 {
        match self {
            SoftShadows::Hard => 1.0,
            SoftShadows::Low => 1.5,
            SoftShadows::Medium => 2.5,
            SoftShadows::High => 3.5,
        }
    }

    fn index(self) -> f32 {
        match self {
            SoftShadows::Hard => 0.0,
            SoftShadows::Low => 1.0,
            SoftShadows::Medium => 2.0,
            SoftShadows::High => 3.0,
        }
    }
}

impl Default for ShadowSettings {
    /// URP's defaults, but for its extras: a 2048 map a cascade and short
    /// contact shadows.
    fn default() -> Self {
        Self {
            enabled: true,
            resolution: 2048,
            depth_bias: 1.0,
            normal_bias: 1.0,
            max_distance: 50.0,
            cascades: 4,
            cascade_splits: [0.067, 0.2, 0.467],
            cascade_border: 0.2,
            soft: SoftShadows::Medium,
            light_resolution: 512,
            contact: 0.35,
            virtual_maps: false,
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
        cascades: 1,
        cascade_splits: [0.067, 0.2, 0.467],
        cascade_border: 0.2,
        soft: SoftShadows::Medium,
        light_resolution: 1,
        contact: 0.0,
        virtual_maps: false,
    };

    /// Where each cascade ends, in metres from the eye.
    pub fn cascade_ends(&self) -> Vec<f32> {
        let count = self.cascades.clamp(1, MAX_CASCADES as u32) as usize;
        let shares: Vec<f32> = match count {
            1 => vec![],
            2 => vec![0.25],
            3 => vec![0.1, 0.3],
            _ => self.cascade_splits.to_vec(),
        };
        let mut ends: Vec<f32> = shares
            .iter()
            .map(|s| s.clamp(0.0, 1.0) * self.max_distance)
            .collect();
        ends.push(self.max_distance);
        ends
    }
}

/// URP's GetScaleAndBiasForLinearDistanceFade: the shadow fades out over
/// the last `border` of `distance`, linearly in the squared distance from
/// the eye — `saturate(d² · scale + bias)` is how far it has faded.
fn shadow_fade(distance: f32, border: f32) -> (f32, f32) {
    let far = distance * distance;
    if border < 0.0001 {
        return (1000.0, -far * 1000.0);
    }
    let kept = (1.0 - border) * (1.0 - border);
    let near = kept * far;
    let span = (far - near).max(1e-6);
    (1.0 / span, -near / span)
}

/// The most shadow cascades a frame has.
pub const MAX_CASCADES: usize = 4;

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
    /// Crevices and corners darkened: URP's SSAO.
    pub ambient_occlusion: crate::ssao::AmbientOcclusion,
    /// Hardware rays, where the device has them: an experiment, off by
    /// default ([`crate::ray`]).
    pub ray_tracing: crate::ray::RayTracing,
    pub draws: Vec<Draw>,
    /// Drawn after everything else with the depth test off, so they are
    /// never hidden by the scene.
    ///
    /// For handles, outlines and anything else that is a tool rather than a
    /// thing in the world. A gizmo half inside the object it sits on is the
    /// reason this exists: you cannot grab what you cannot see, and burying
    /// it is exactly what depth testing does.
    pub overlay_draws: Vec<Draw>,
    /// Outlined on the finished picture in their material's colour: the
    /// edge of what they cover on screen, as Unity outlines a selection —
    /// full where they are seen, faint where something is in front. Their
    /// silhouettes go into a mask and the edge is found there, so what is
    /// outlined is the shape itself, not a box round it. Up to
    /// [`crate::tools::OUTLINE_COLORS`] colours; where two meet, the later
    /// colour's edge is drawn (a child's inside its parent's).
    pub outline_draws: Vec<Draw>,
    /// How wide an outline is, in pixels of the picture.
    pub outline_width: f32,
    /// Point lights besides the sun.
    pub lights: Vec<PointLight>,
    /// Lamps with a lens flare of their own: where each is, its colour
    /// and how bright its flare. Drawn by the post pass.
    pub flares: Vec<Flare>,
    /// Meshes the game changes as it goes — water, a rope — drawn from
    /// their data and uploaded again only when it changed.
    pub live_meshes: Vec<LiveMeshDraw>,
    /// Pictures other cameras take first, for materials to show — a
    /// mirror's: each drawn with everything but post-processing's
    /// history, the size of this frame.
    pub texture_views: Vec<TextureView>,
    /// Screens on things in the world, drawn into their pictures before
    /// the frame by whoever draws the overlay ([`crate::world::WorldUi`]).
    pub ui_pictures: Vec<UiPicture>,
    /// Boxes whose surroundings are baked for reflections
    /// ([`crate::reflections`]).
    pub reflection_probes: Vec<crate::reflections::ReflectionProbe>,
    /// Boxes of probes the rays keep lit, for diffuse light that bounces
    /// ([`crate::ddgi`]): the first is lit, on a device that traces.
    pub irradiance_volumes: Vec<crate::ddgi::PlacedVolume>,
    /// Pictures pressed onto what lies in their boxes ([`crate::decals`]).
    pub decals: Vec<crate::decals::Decal>,
    /// Light seen in the air ([`crate::volume`]); off by default.
    pub volumetric_fog: crate::volume::VolumetricFog,
    /// Balls of dust in the air ([`crate::volume::Puff`]).
    pub puffs: Vec<crate::volume::Puff>,
    /// Smoke and fire from grids, in the fog ([`crate::volume::Smoke`]),
    /// the nearest the eye first.
    pub smoke: Vec<crate::volume::Smoke>,
    /// The scene's signed distance field, for occlusion and soft sun
    /// shadows ([`crate::distance`]); none by default.
    pub distance_field: Option<crate::distance::DistanceField>,
    /// Emitters whose particles are on the GPU ([`crate::particles_gpu`]).
    pub gpu_particles: Vec<crate::particles_gpu::GpuEmitter>,
    /// Where sand may blow off dune crests ([`crate::volume::Plume`]):
    /// how much does, the wind decides.
    pub plumes: Vec<crate::volume::Plume>,
    /// Shaped ground drawn finely round the camera, its ripples in the
    /// geometry ([`crate::terrain::TerrainSurface`]).
    pub terrain: Option<crate::terrain::TerrainSurface>,
    /// What sways foliage, and what bends grass ([`crate::foliage`]).
    pub wind: crate::foliage::Wind,
    pub benders: Vec<crate::foliage::Bender>,
    /// Seconds, for what moves by itself — foliage in the wind. `None`
    /// takes the renderer's own clock; a test that wants the same picture
    /// twice says a time.
    pub time: Option<f32>,
    /// Rain, snow, wet ground and puddles ([`crate::weather`]); clear by
    /// default.
    pub weather: crate::weather::Weather,
    /// Reflections marched across the screen
    /// ([`crate::reflections::ScreenSpaceReflections`]); off by default.
    pub screen_space_reflections: crate::reflections::ScreenSpaceReflections,
    /// Skinning matrices, one entry per animated thing on screen. Held here
    /// rather than on each draw so that two draws sharing a skeleton share
    /// one upload.
    pub poses: Vec<Pose>,
}

impl Frame {
    /// A copy to draw from, without what the renderer has already taken
    /// out of it: the live meshes (whose vertices, a head of hair or a
    /// sheet of water, are the largest thing in a frame) and the cameras'
    /// frames (drawn into their textures before this one). Every field is
    /// named, so a new one is a compile error here rather than a copy that
    /// quietly leaves it out. `clone` on every field, `Copy` or not, so a
    /// field that stops being `Copy` does not break it.
    #[allow(clippy::clone_on_copy)]
    fn shallow(&self) -> Frame {
        let Frame {
            camera,
            lighting,
            fog,
            shadows,
            sky,
            clear_color,
            post,
            ambient_occlusion,
            ray_tracing,
            draws,
            overlay_draws,
            outline_draws,
            outline_width,
            lights,
            flares,
            ui_pictures,
            reflection_probes,
            irradiance_volumes,
            decals,
            volumetric_fog,
            puffs,
            smoke,
            distance_field,
            gpu_particles,
            plumes,
            terrain,
            wind,
            benders,
            time,
            weather,
            screen_space_reflections,
            poses,
            live_meshes: _,
            texture_views: _,
        } = self;
        Frame {
            camera: camera.clone(),
            lighting: lighting.clone(),
            fog: fog.clone(),
            shadows: shadows.clone(),
            sky: sky.clone(),
            clear_color: clear_color.clone(),
            post: post.clone(),
            ambient_occlusion: ambient_occlusion.clone(),
            ray_tracing: ray_tracing.clone(),
            draws: draws.clone(),
            overlay_draws: overlay_draws.clone(),
            outline_draws: outline_draws.clone(),
            outline_width: outline_width.clone(),
            lights: lights.clone(),
            flares: flares.clone(),
            ui_pictures: ui_pictures.clone(),
            reflection_probes: reflection_probes.clone(),
            irradiance_volumes: irradiance_volumes.clone(),
            decals: decals.clone(),
            volumetric_fog: volumetric_fog.clone(),
            puffs: puffs.clone(),
            smoke: smoke.clone(),
            distance_field: distance_field.clone(),
            gpu_particles: gpu_particles.clone(),
            plumes: plumes.clone(),
            terrain: terrain.clone(),
            wind: wind.clone(),
            benders: benders.clone(),
            time: time.clone(),
            weather: weather.clone(),
            screen_space_reflections: screen_space_reflections.clone(),
            poses: poses.clone(),
            live_meshes: Vec::new(),
            texture_views: Vec::new(),
        }
    }
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
            ambient_occlusion: crate::ssao::AmbientOcclusion::default(),
            ray_tracing: crate::ray::RayTracing::default(),
            draws: Vec::new(),
            overlay_draws: Vec::new(),
            outline_draws: Vec::new(),
            outline_width: 2.0,
            lights: Vec::new(),
            flares: Vec::new(),
            live_meshes: Vec::new(),
            texture_views: Vec::new(),
            ui_pictures: Vec::new(),
            reflection_probes: Vec::new(),
            irradiance_volumes: Vec::new(),
            decals: Vec::new(),
            volumetric_fog: crate::volume::VolumetricFog::OFF,
            puffs: Vec::new(),
            smoke: Vec::new(),
            distance_field: None,
            gpu_particles: Vec::new(),
            plumes: Vec::new(),
            terrain: None,
            wind: crate::foliage::Wind::default(),
            benders: Vec::new(),
            time: None,
            weather: crate::weather::Weather::default(),
            screen_space_reflections: Default::default(),
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
    /// World space to each cascade's clip space.
    light_view_projection: [[[f32; 4]; 4]; MAX_CASCADES],
    /// The soft filter (0 hard .. 3 high), unused, a texel of a cascade in
    /// its map's units, and how many cascades there are — 0 when shadows
    /// are off, which lets the shader skip the lookup without a second
    /// pipeline.
    shadow_params: [f32; 4],
    /// The camera's view matrix's third row: `-dot(row, p)` is how deep a
    /// point is, which picks its slice of the light clusters.
    view_depth: [f32; 4],
    /// The cluster grid: cells across, down and deep; `w` how many lights
    /// there are.
    clusters: [f32; 4],
    /// The near plane, `ln(far / near)`, and the target's width and height
    /// in pixels.
    cluster_depth: [f32; 4],
    /// Lamps' shadows: one texel of their maps, in UV.
    light_shadow: [f32; 4],
    /// Clip space back to the world: the sky is drawn by asking, for each
    /// pixel, which way it looks.
    inverse_view_projection: [[f32; 4]; 4],
    /// Zenith colour; `w` is 1 for a procedural sky.
    sky_zenith: [f32; 4],
    /// Horizon colour; `w` is the cosine of the sun disc's radius.
    sky_horizon: [f32; 4],
    /// Ground colour; `w` is the sky's exposure.
    sky_ground: [f32; 4],
    /// Each cascade's sphere: centre, and its radius squared. A point is
    /// in the first one whose sphere holds it.
    cascade_spheres: [[f32; 4]; MAX_CASCADES],
    /// Each cascade's texel, metres: how far inside a thing a point is
    /// looked up to find how thick it is toward the sun.
    cascade_bias: [f32; 4],
    /// The shadow's fade toward the shadow distance, URP's: the scale and
    /// bias of the squared distance from the eye (x, y).
    cascade_depth_bias: [f32; 4],
    /// 1 when there is ambient occlusion to read; the share of the direct
    /// light it darkens too; how far contact shadows' rays go; how far the
    /// marches' noise is turned this frame — 0 without TAA, which alone
    /// averages it away, so a still picture stays still.
    ambient_occlusion: [f32; 4],
    /// Traced: sun shadows, lamp shadows, occlusion, each 1 where asked and
    /// the device traces; `w` the tangent of the sun disc's radius.
    ray: [f32; 4],
    /// Rays to the sun, occlusion rays, occlusion reach.
    ray_params: [f32; 4],
    /// How many reflection probes there are, and their last mip; 1 when
    /// reflections are traced, and the roughest surface that is.
    probe_params: [f32; 4],
    /// Each probe: its centre and blend distance; its half size and 1 to
    /// bend reflections to its box.
    probes: [[f32; 4]; crate::reflections::MAX_PROBES * 2],
    /// Each face's projection of a direction.
    probe_faces: [[[f32; 4]; 4]; 6],
    /// Volumetric fog: 1 when on, how far its cells reach, the near plane.
    volume: [f32; 4],
    /// The air's colour and its density at the base height.
    fog_medium: [f32; 4],
    /// Base height, falloff per metre, anisotropy, the sky's share.
    fog_shape: [f32; 4],
    /// The lamps' share.
    fog_lamps: [f32; 4],
    /// What is behind everything with a plain-colour sky.
    clear_color: [f32; 4],
    /// Wind and benders, for the vertex shaders.
    foliage: crate::foliage::FoliageUniform,
    /// The physical sky: 1 when on, the aerial grid's far end in metres.
    air: [f32; 4],
    /// Wetness, puddles, snow, rain; snowfall.
    weather: [[f32; 4]; 3],
    /// Up to four water surfaces, two vectors each: height and 1 when
    /// there; the rectangle it covers (min x, min z, max x, max z).
    waters: [[f32; 4]; 8],
    /// Clouds: coverage, base, thickness, density; drift x and z, size,
    /// how dark their shadows are.
    clouds: [[f32; 4]; 2],
    /// Last frame's camera, for reading the last frame's colour.
    previous_view_projection: [[f32; 4]; 4],
    /// Screen-space reflections: 1 when on and there is a last frame, how
    /// far, how thick, how many steps.
    ssr: [f32; 4],
    /// Dust in the air, two vectors each: centre and radius; linear colour
    /// and density. How many is `volume`'s w.
    puffs: [[f32; 4]; 2 * crate::volume::MOST_PUFFS],
    /// 1 when the clouds' pass marched dust devils or crest plumes: the
    /// picture it made is laid over what is behind them.
    dust: [f32; 4],
    /// How much it is night (the stars); the moon's disc's size.
    night: [f32; 4],
    /// The terrain drawn finely: the world into its own space, and back.
    terrain_to_local: [[f32; 4]; 4],
    terrain_to_world: [[f32; 4]; 4],
    /// Its size, its cells, 1 when there is one, the finest spacing.
    terrain: [f32; 4],
    /// Its lowest and highest ground in the world; patches per ring side.
    terrain_bounds: [f32; 4],
    /// What it is drawn with, for the mesh shader: the instance's numbers
    /// (colour and shading, surface, emission, uv, detail, params).
    terrain_look: [[f32; 4]; 7],
    /// Glass by rays: 1 when it bends what is seen through it, and its
    /// index of refraction; the lightning's points, and its flash.
    glass: [f32; 4],
    /// A stroke of lightning's channel: points, brightness in `w`.
    bolt: [[f32; 4]; crate::weather::BOLT_POINTS],
    /// The irradiance volume ([`crate::ddgi`]): its grid, and its biases.
    ddgi: [[f32; 4]; 4],
    /// Virtual shadow maps ([`crate::vsm`]): the light's frame, the levels'
    /// windows.
    vsm: [[f32; 4]; 8],
    /// ReSTIR ([`crate::restir`]): on, the reservoirs' size, the frame.
    restir: [f32; 4],
    /// The scene's distance field: its box and 1 when there is one; its
    /// far corner and range.
    distance: [[f32; 4]; 2],
    /// The light a face turned sideways sees, when the scene says its
    /// light from all round; `w` is 1 then.
    ambient_equator: [f32; 4],
}

/// What the shadow pass needs for one cascade.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CasterUniform {
    view_projection: [[f32; 4]; 4],
    /// Foliage bends in the shadow passes as it does in the frame.
    foliage: crate::foliage::FoliageUniform,
    /// A sun cascade's URP bias: the way to the sun, and how far the
    /// caster is pushed from it (w, metres).
    toward: [f32; 4],
    /// How far a sun cascade's caster is shrunk along its normal where
    /// edge-on (x, metres).
    inset: [f32; 4],
}

pub use crate::lights::MAX_LIGHTS;

/// A light at a point, fading to nothing at `range` — a campfire, a lamp,
/// a torch. Unity's Point Light, or with `spot` its Spot Light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointLight {
    pub position: Vec3,
    /// Linear, already times its intensity.
    pub color: Vec3,
    pub range: f32,
    /// For a spot light: which way it shines, and its cone, degrees
    /// across. `None` shines every way.
    pub spot: Option<(Vec3, f32)>,
    /// Casts shadows, if a shadow map is left for it ([`crate::lights`]).
    pub shadows: bool,
    /// How it fades with distance.
    pub falloff: Falloff,
    /// For a spot: its inner cone, degrees across — all its light inside,
    /// fading to none at its edge, as URP's Inner Spot Angle. `None`
    /// fades over the cone's last tenth.
    pub inner_cone: Option<f32>,
}

/// How a lamp's light fades on its way to `range`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Falloff {
    /// `(1 − d/range)²`: a soft pool, its colour times its intensity at
    /// the lamp itself — the numbers a person picks by eye.
    #[default]
    Smooth,
    /// As the square of the distance, eased to nothing at `range`: URP's
    /// (`1/d² · (1 − (d/range)⁴)²`), so a Unity lamp's intensity means
    /// what it did there — its light a metre off.
    InverseSquare,
}

/// A mesh the game rewrites as it goes, as a frame carries it. `key`
/// tells one from another between frames; `version` changes when the data
/// did, and only then is it uploaded again.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveMeshDraw {
    pub key: u64,
    pub version: u64,
    pub vertices: std::sync::Arc<Vec<crate::asset::Vertex>>,
    pub indices: std::sync::Arc<Vec<u32>>,
    pub transform: Mat4,
    pub material: Material,
}

/// A screen on a thing in the world: its widgets drawn into a picture of
/// `size` pixels, over `background`, for the thing's material to show.
#[derive(Debug, Clone, PartialEq)]
pub struct UiPicture {
    pub id: crate::asset::AssetId,
    pub size: (u32, u32),
    pub background: glam::Vec4,
    pub ui: crate::ui::Ui,
}

/// A camera's picture: under what id materials find it
/// ([`crate::asset::AssetId::render_target`]), and what it sees.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureView {
    pub id: crate::asset::AssetId,
    pub frame: Box<Frame>,
}

/// A lamp's own lens flare, in the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flare {
    pub position: Vec3,
    /// Linear.
    pub color: Vec3,
    pub intensity: f32,
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
    /// rgb is the base colour; w is 0 lit, 1 unlit, 2 lit with the grid.
    color_and_shading: [f32; 4],
    /// Metallic, smoothness, alpha, alpha-clip threshold.
    surface: [f32; 4],
    /// Emission, linear and times its intensity; `w` packs the switches
    /// ([`FLAG_SPECULAR`] and the rest).
    emission: [f32; 4],
    /// Tiling xy, offset zw.
    uv: [f32; 4],
    /// Normal scale, occlusion strength.
    detail: [f32; 4],
    /// The material's own numbers, for its shader.
    params: [[f32; 4]; 2],
    /// Light under the surface: its colour (linear), and how far it goes.
    subsurface: [f32; 4],
    /// Its maps' handles — base, normal, mask, emission, and its
    /// material's own textures — for the bindless shader, two to a number
    /// ([`packed`]).
    maps: [u32; 4],
}

/// Bits of [`InstanceRaw::emission`]'s `w`.
const FLAG_SPECULAR: u32 = 1;
const FLAG_REFLECTIONS: u32 = 2;
const FLAG_SHADOWS: u32 = 4;
const FLAG_PREMULTIPLY: u32 = 8;
/// The base map is read on the screen: a camera's picture seen through it.
const FLAG_SCREEN: u32 = 16;
/// The same, flipped across: a mirror's.
const FLAG_MIRROR: u32 = 32;
/// Clay: mud when wet, cracking as it dries ([`Material::clay`]).
const FLAG_CLAY: u32 = 64;
/// Placed mirrored (a scale of -1): its faces wind the other way, so the
/// side the GPU calls front is its back.
const FLAG_INSIDE_OUT: u32 = 128;
/// Both faces drawn, the back lit as the front: its normal is not turned
/// ([`RenderFace::BothAsFront`]).
const FLAG_BACK_AS_FRONT: u32 = 256;

/// What the GPU is told about one draw.
fn instance_of(transform: Mat4, material: &Material) -> InstanceRaw {
    let shading = match material.shading {
        Shading::Lit => 0.0,
        Shading::Unlit => 1.0,
        Shading::Grid => 2.0,
        Shading::Water => 3.0,
        Shading::Sand => 4.0,
    };
    let mut flags = 0;
    if material.specular_highlights {
        flags |= FLAG_SPECULAR;
    }
    if material.environment_reflections {
        flags |= FLAG_REFLECTIONS;
    }
    if material.receive_shadows {
        flags |= FLAG_SHADOWS;
    }
    if material.clay {
        flags |= FLAG_CLAY;
    }
    if transform.determinant() < 0.0 {
        flags |= FLAG_INSIDE_OUT;
    }
    if material.render_face == RenderFace::BothAsFront {
        flags |= FLAG_BACK_AS_FRONT;
    }
    if material.is_transparent() && material.blend == Blend::Premultiply {
        flags |= FLAG_PREMULTIPLY;
    }
    flags |= match material.screen_map {
        crate::material::ScreenMap::Off => 0,
        crate::material::ScreenMap::Screen => FLAG_SCREEN,
        crate::material::ScreenMap::Mirror => FLAG_MIRROR,
    };
    InstanceRaw {
        model: transform.to_cols_array_2d(),
        color_and_shading: extend(material.color(), shading),
        // Water reads its own: how far down one sees, and its foam.
        surface: if material.shading == Shading::Water {
            [
                material.clarity.max(0.05),
                material.foam.clamp(0.0, 1.0),
                1.0,
                0.0,
            ]
        } else {
            [
                material.metallic.clamp(0.0, 1.0),
                material.smoothness.clamp(0.0, 1.0),
                if material.is_transparent() || material.alpha_clip > 0.0 {
                    material.alpha.clamp(0.0, 1.0)
                } else {
                    1.0
                },
                material.alpha_clip.clamp(0.0, 1.0),
            ]
        },
        emission: [
            material.emission[0].max(0.0),
            material.emission[1].max(0.0),
            material.emission[2].max(0.0),
            flags as f32,
        ],
        uv: [
            material.tiling[0],
            material.tiling[1],
            material.offset[0],
            material.offset[1],
        ],
        detail: [
            material.normal_scale,
            material.occlusion_strength.clamp(0.0, 1.0),
            material.wind.max(0.0),
            material.translucency.clamp(0.0, 1.0),
        ],
        params: [
            [
                material.params[0],
                material.params[1],
                material.params[2],
                material.params[3],
            ],
            [
                material.params[4],
                material.params[5],
                material.params[6],
                material.params[7],
            ],
        ],
        subsurface: [
            material.subsurface[0].max(0.0),
            material.subsurface[1].max(0.0),
            material.subsurface[2].max(0.0),
            material.subsurface_radius.max(0.0005),
        ],
        maps: [TextureHandle::WHITE.0, TextureHandle::FLAT_NORMAL.0, TextureHandle::WHITE.0, TextureHandle::WHITE.0],
    }
}

/// A group of draws of one mesh with one texture, and the pipeline they
/// take — `None` in the shadow and overlay passes, which set their own.
type BatchKey = (Option<Look>, MeshHandle, Maps);

struct GpuTexture {
    view: wgpu::TextureView,
}

fn gpu_has_first_instance(occlusion: &crate::occlusion::Occlusion) -> bool {
    occlusion.can
}

/// A mesh handle with this bit set is a coarser level of a mesh, kept
/// apart so the handles of what is uploaded stay in order.
const LOD_HANDLE: u32 = 1 << 31;

struct GpuMesh {
    vertices: wgpu::Buffer,
    /// Its vertices' painted colours, when it has any; the renderer's
    /// white ones stand in when it has none.
    colors: Option<wgpu::Buffer>,
    /// Joint indices and weights, when the mesh has them.
    skin: Option<wgpu::Buffer>,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Kept so the shadow pass can fit its frustum to what is actually being
    /// drawn. A map spread over an empty hundred metres wastes every texel.
    bounds: crate::asset::Bounds,
    /// What rays hit, on a device that traces.
    blas: Option<wgpu::Blas>,
    /// Its clusters, when it is dense enough to be culled by them
    /// ([`crate::cluster`]): its indices are then in their order.
    clusters: Option<crate::cluster::MeshClusters>,
}

/// Holds the pipeline, the uploaded meshes and the buffers a frame needs.
pub struct Renderer {
    pipelines: Pipelines,
    /// The standard shader's source now — [`SHADER`], or what was reloaded.
    base_shader: String,
    /// Materials' own `surface` functions, by id: built again whenever the
    /// standard shader is reloaded.
    material_shaders: std::collections::HashMap<crate::asset::AssetId, String>,
    /// What each of those reads through `texture_at`, by its
    /// `// scrap:textures` line: the names of its slots.
    shader_textures: scrap_core::hash::FastMap<crate::asset::AssetId, Vec<String>>,
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
    /// White, a vertex's worth for every vertex of the largest mesh with no
    /// colours of its own: what such a mesh is drawn with in their slot, so
    /// one pipeline draws both and a mesh with none carries none.
    white_colors: wgpu::Buffer,
    /// How many vertices `white_colors` covers.
    white_capacity: u64,
    depth: wgpu::TextureView,
    depth_size: (u32, u32),
    /// Every cascade's map, as one array to sample.
    shadow_map: wgpu::TextureView,
    /// Each cascade's layer, to draw into.
    shadow_layers: Vec<wgpu::TextureView>,
    /// Each cascade's matrix for the shadow pass, a slot apiece.
    casters: wgpu::Buffer,
    caster_stride: u64,
    shadow_sampler: wgpu::Sampler,
    shadow_resolution: u32,
    format: wgpu::TextureFormat,
    /// Samples per pixel the scene is drawn with: 4 where the device can.
    samples: u32,
    /// The scene pass being drawn started from the prepass's depth.
    depth_prepassed: bool,
    /// What handles and outlines are drawn into, made when first wanted
    /// and again when the picture changes size ([`crate::tools`]).
    tools: Option<crate::tools::Tools>,
    /// Occlusion culling asked for, however little there is to cull.
    occlusion_always: bool,
    /// Smokes whose air is simulated on the GPU ([`crate::smoke_gpu`]).
    smoke_sim: crate::smoke_gpu::SmokeSim,
    /// The pipelines of the sample count drawn with before this one.
    other_samples: Option<SamplePipelines>,
    /// The scene, in high dynamic range: multisampled, and resolved.
    scene: SceneTargets,
    post: crate::post::PostRenderer,
    /// Depth of field and motion blur, before the post stack.
    lens: crate::lens::LensRenderer,
    /// Last frame's camera, for motion blur.
    previous_view_projection: Option<Mat4>,
    /// The reflection probes' pictures.
    reflections: crate::reflections::ProbeStore,
    /// The frame's decals, and their pictures.
    decal_buffer: wgpu::Buffer,
    decal_atlases: crate::decals::DecalAtlases,
    ssao: crate::ssao::SsaoRenderer,
    /// Temporal antialiasing's history ([`crate::taa`]).
    taa: crate::taa::Taa,
    /// Drawing at fewer pixels and making the picture up
    /// ([`crate::upscale`]).
    upscaler: crate::upscale::Upscaler,
    /// Drawing a camera's picture for a texture, not the screen: no
    /// antialiasing history is touched.
    picturing: bool,
    /// When the exposure was last metered, on the frame's clock.
    metered_at: Option<f32>,
    /// Terrain's fine grid round the camera, uploaded when first wanted.
    clipmap: Option<MeshHandle>,
    /// The drawn terrain's heights, for its vertex shader; and what they
    /// were made from.
    terrain_heights: wgpu::TextureView,
    /// How the grass round the camera is trampled, and its texture
    /// ([`crate::foliage::TrampleMap`]).
    trample: crate::foliage::TrampleMap,
    trample_texture: wgpu::Texture,
    trample_view: wgpu::TextureView,
    /// The scene's distance field's texture.
    distance: crate::distance::DistanceTexture,
    terrain_made: Option<crate::terrain::Terrain>,
    /// The scene as rays see it, on a device that traces.
    ray: Option<crate::ray::RayScene>,
    /// Which passes run, whatever the frame asks for: the game's graphics
    /// settings ([`crate::passes`]).
    passes: crate::passes::Passes,
    /// The graphics preset, if one is chosen.
    quality: Option<crate::quality::Quality>,
    /// The frame's lights ([`crate::lights`]), each cell's run of them, and
    /// the runs themselves.
    light_buffer: wgpu::Buffer,
    cell_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_capacity: u64,
    /// Each lamp shadow map's view of the world.
    light_view_buffer: wgpu::Buffer,
    /// The lamps' shadow maps, as one array to sample, and a layer at a
    /// time to draw into.
    light_shadow_map: wgpu::TextureView,
    light_shadow_layers: Vec<wgpu::TextureView>,
    light_shadow_resolution: u32,
    pose_layout: wgpu::BindGroupLayout,
    pose_bind_group: wgpu::BindGroup,
    poses: wgpu::Buffer,
    /// One pose's slot, padded up to the device's dynamic-offset alignment.
    pose_stride: u64,
    pose_capacity: u64,
    stats: FrameStats,
    meshes: Vec<GpuMesh>,
    /// Slots of meshes released, for the next ones uploaded.
    free_meshes: Vec<u32>,
    /// Texture assets' resident levels and what the frames need
    /// ([`crate::streaming_textures`]).
    streams: scrap_core::hash::FastMap<TextureHandle, crate::streaming_textures::TextureStream>,
    /// The last view's batch lists, by which list and which batch: their
    /// instance buffers taken up again — cleared, not freed — rather than
    /// grown from nothing by doubling every frame (a batch of thousands is
    /// megabytes copied over and over). Only the batches the last view
    /// had are kept.
    batch_pool: BatchPool,
    /// The physical sky's light for a sun of strength one, and the air,
    /// height and sun it was found for.
    sky_light: Option<(crate::atmosphere::Atmosphere, f32, Vec3, (Vec3, Vec3))>,
    /// Every instance of a view, one after another, as uploaded: kept.
    flat: Vec<InstanceRaw>,
    /// Live meshes by their key: the mesh each is drawn with, and the
    /// version last uploaded.
    live: std::collections::HashMap<u64, (MeshHandle, u64)>,
    /// Cameras' pictures by their id: the texture drawn into, its size.
    targets: std::collections::HashMap<crate::asset::AssetId, (wgpu::Texture, (u32, u32))>,
    textures: Vec<GpuTexture>,
    /// Which handle each texture asset was uploaded as, so a material's
    /// maps — asset ids — find theirs.
    by_asset: scrap_core::hash::FastMap<crate::asset::AssetId, TextureHandle>,
    /// Each mesh's own look, from its file's materials: drawn with when
    /// the material has no base map and the entity no texture of its own.
    looks: scrap_core::hash::FastMap<MeshHandle, TextureHandle>,
    /// A bind group per set of four maps in use, made before the frame's
    /// passes and kept.
    map_groups: scrap_core::hash::FastMap<Maps, wgpu::BindGroup>,
    /// Every texture in one array the shader indexes, where the device can
    /// ([`crate::bindless`]).
    bindless: Option<crate::bindless::Bindless>,
    texture_layout: wgpu::BindGroupLayout,
    texture_sampler: wgpu::Sampler,
    /// Kept to rebuild the pipelines when the shader is reloaded.
    pipeline_layout: wgpu::PipelineLayout,
    /// The terrain's mesh-shader group and pipeline layouts, its uniform,
    /// and the group bound (remade with the heights).
    terrain_mesh_layouts: Option<(wgpu::BindGroupLayout, wgpu::PipelineLayout)>,
    terrain_mesh_buffer: wgpu::Buffer,
    terrain_mesh_group: Option<wgpu::BindGroup>,
    shadow_pipeline_layout: wgpu::PipelineLayout,
    shadow_clip_layout: wgpu::PipelineLayout,
    skinned_layout: wgpu::PipelineLayout,
    sky_layout: wgpu::PipelineLayout,
    fog_inject_layout: wgpu::PipelineLayout,
    fog_integrate_layout: wgpu::PipelineLayout,
    /// Volumetric fog's grid.
    volumes: crate::volume::Volumes,
    /// The clock foliage sways by when a frame does not say the time.
    started: web_time::Instant,
    /// The stroke of lightning of the frame being drawn, if any.
    bolt: Option<crate::weather::Bolt>,
    /// Particles on the GPU: their pipelines and pools.
    gpu_particles: crate::particles_gpu::GpuParticles,
    /// Occlusion culling against last frame's depth ([`crate::occlusion`]).
    occlusion: crate::occlusion::Occlusion,
    /// Dense meshes culled a cluster at a time ([`crate::cluster`]).
    clusters: crate::cluster::Clusters,
    /// Probes the rays keep lit ([`crate::ddgi`]).
    ddgi: crate::ddgi::Ddgi,
    /// The sun's virtual shadow maps ([`crate::vsm`]).
    vsm: crate::vsm::VirtualShadows,
    /// The lamps by ReSTIR ([`crate::restir`]).
    restir: crate::restir::Restir,
    /// The last screen frame's passes as a graph ([`crate::graph`]).
    graph: crate::graph::FrameGraph,
    /// Coarser levels of the meshes that have them ([`crate::lod`]): each
    /// mesh's levels, their handles (with [`LOD_HANDLE`] set) and the least
    /// share of the screen each is drawn for.
    lods: scrap_core::hash::FastMap<u32, Vec<(MeshHandle, f32)>>,
    lod_meshes: Vec<GpuMesh>,
    /// How long each pass takes on the GPU, when asked ([`Self::profile_gpu`]).
    timer: Option<crate::gpu_timer::GpuTimer>,
    /// The Frame Debugger's recording and picture ([`crate::frame_debugger`]).
    debugger: crate::frame_debugger::Debugger,
    /// The lit pipelines built lean, for frames that may be drawn so
    /// ([`crate::lean`]), and the shader modules they are built from: the
    /// standard one's (`None`) and each material shader's.
    lean: crate::lean::Lean<LeanKey>,
    /// Far clusters left out of the prepass (`SCRAP_FAR_PREPASS=1` keeps
    /// them), and whether this frame's were: then the lit pass's depth is
    /// the whole one, and what comes after it reads that.
    far_off_prepass: bool,
    split_prepass: bool,
    /// The error a clustered mesh is drawn with, in pixels
    /// ([`crate::cluster_lod`]).
    cluster_error: f32,
    /// The screen's shadow cascades as last drawn, and whose turn it is of
    /// the far ones; off by `SCRAP_SHADOW_STAGGER=0`.
    cascade_cache: Option<CascadeCache>,
    shadow_turn: bool,
    shadow_stagger: bool,
    /// Unlit see-through things at half size ([`crate::lowres`]).
    lowres: crate::lowres::LowRes,
    /// Whether the last screen frame used it.
    lowres_drawn: bool,
    lean_modules: std::collections::HashMap<Option<crate::asset::AssetId>, wgpu::ShaderModule>,
    /// Meshes' and textures' names from their assets, for the debugger to
    /// call a draw by.
    mesh_names: std::collections::HashMap<u32, std::sync::Arc<str>>,
    texture_names: std::collections::HashMap<u32, std::sync::Arc<str>>,
    timing: bool,
    /// The physical sky's table and aerial grid.
    atmosphere: crate::atmosphere::AtmosphereRenderer,
    /// What Unity's procedural sky was last baked into the sky-view table
    /// for: the way to the sun and the sky's numbers.
    unity_sky_baked: Option<[f32; 11]>,
    /// One texel of depth, bound in place of the prepass's while it draws.
    blank_depth: wgpu::TextureView,
    /// The clouds' picture.
    clouds: crate::clouds::CloudRenderer,
    /// The frame's bind group with the fog left out, for the passes that
    /// make the fog.
    fog_bind_group: wgpu::BindGroup,
    /// The two frame groups again with the occlusion culling's kept
    /// instances for `instance_data` (binding 33): what it draws numbers
    /// its instances in that list, not in the frame's. With the buffer
    /// they were made for, to make them again when it is.
    kept_groups: (wgpu::Buffer, wgpu::BindGroup, wgpu::BindGroup),
}

/// Where the scene is drawn before post-processing turns it into a picture.
struct SceneTargets {
    /// Drawn into, `samples` per pixel; `None` when there is one sample.
    multisampled: Option<wgpu::TextureView>,
    /// Resolved into: what post-processing reads.
    resolved: wgpu::TextureView,
    resolved_texture: wgpu::Texture,
    /// The last frame, resolved: what screen-space reflections read.
    history: wgpu::Texture,
    history_view: wgpu::TextureView,
    /// Whether `history` holds a frame yet.
    has_history: bool,
}

fn scene_targets(gpu: &Gpu, width: u32, height: u32, samples: u32) -> SceneTargets {
    let texture = |label, samples, usage| {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
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
    };
    let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
    let resolved = texture(
        "scene",
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
    );
    let history = texture(
        "last frame",
        1,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    SceneTargets {
        multisampled: (samples > 1).then(|| {
            view(&texture(
                "scene (multisampled)",
                samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ))
        }),
        resolved: view(&resolved),
        resolved_texture: resolved,
        history_view: view(&history),
        history,
        has_history: false,
    }
}

/// Instances in the colour pass from which occlusion culling pays.
const OCCLUSION_FROM: usize = 2000;

/// Rays a pixel this frame, of the `asked` a still frame takes: half of
/// them when TAA's history gathers the frames' (at least one).
fn rays_a_frame(asked: u32, taa: bool) -> u32 {
    let asked = asked.clamp(1, 64);
    if taa {
        asked.div_ceil(2)
    } else {
        asked
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

/// One of a surface's maps in its bind group.
fn map_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// The engine's shader, as compiled in.
pub const SHADER: &str = include_str!("render.wgsl");
/// Terrain's task and mesh stages, appended to the renderer's shader where
/// the device has mesh shaders.
const TERRAIN_MESH: &str = include_str!("terrain_mesh.wgsl");
/// Ray masks, as render.wgsl's RAY_THINGS, RAY_TERRAIN and RAY_GLASS.
const RAY_THINGS: u8 = 1;
const RAY_TERRAIN: u8 = 2;
const RAY_GLASS: u8 = 4;

/// Where the engine's shader source was when the engine was built — for
/// watching it while working on the engine; see [`ShaderFile`].
pub const SHADER_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/render.wgsl");

/// The standard shader with a material's `surface` put in place of its
/// own, between the `scrap:surface` marks.
pub fn with_surface(base: &str, surface: &str) -> Result<String, String> {
    const OPEN: &str = "// scrap:surface {";
    const CLOSE: &str = "// scrap:surface }";
    let start = base
        .find(OPEN)
        .ok_or("the standard shader has no `scrap:surface` mark")?;
    let end = base[start..]
        .find(CLOSE)
        .map(|i| start + i + CLOSE.len())
        .ok_or("the standard shader's `scrap:surface` mark is not closed")?;
    if !surface.contains("fn surface(") {
        return Err(
            "a material's shader has to have `fn surface(in: SurfaceIn, out: Surface) -> Surface`"
                .into(),
        );
    }
    Ok(format!("{}{surface}\n{}", &base[..start], &base[end..]))
}

/// Every material shader in a folder — `shaders/water.wgsl` for
/// `shader: "water"`, `shaders/lava.graph.ron` for `shader: "lava"` — and
/// every particle effect graph — `shaders/sparks.vfx.ron` for an emitter's
/// `graph: "sparks"` — put into a renderer, and again when one changes.
pub struct MaterialShaders {
    dir: std::path::PathBuf,
    stamps: std::collections::HashMap<std::path::PathBuf, Option<std::time::SystemTime>>,
}

impl MaterialShaders {
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            stamps: Default::default(),
        }
    }

    /// Put in whatever is new or changed since the last call: each one's
    /// name, and what went wrong with it if it did not build.
    pub fn poll(
        &mut self,
        renderer: &mut Renderer,
        gpu: &Gpu,
    ) -> Vec<(String, Result<(), String>)> {
        let Ok(entries) = scrap_core::files::read_dir(&self.dir) else {
            return Vec::new();
        };
        // What is new or changed, read; then built all together.
        let mut out = Vec::new();
        let mut read = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let effect = path.file_name().and_then(|f| f.to_str()).and_then(scrap_shadergraph::effect_name).map(str::to_string);
            let Some(name) = material_shader_name(&path).or(effect.clone()) else {
                continue;
            };
            let stamp = scrap_core::files::modified(&path);
            if self.stamps.get(&path) == Some(&stamp) {
                continue;
            }
            self.stamps.insert(path.clone(), stamp);
            if effect.is_some() {
                let result = effect_source(&path)
                    .and_then(|source| renderer.set_effect_graph(gpu, &name, &source))
                    .map_err(|e| format!("{}:\n{e}", path.display()));
                out.push((name, result));
                continue;
            }
            match material_shader_source(&path) {
                Ok(source) => read.push((name, path, source)),
                Err(e) => out.push((name, Err(format!("{}:\n{e}", path.display())))),
            }
        }
        let shaders: Vec<(crate::asset::AssetId, String)> =
            read.iter().map(|(name, _, source)| (crate::asset::shader_id(name), source.clone())).collect();
        let built = renderer.set_material_shaders(gpu, &shaders);
        for ((name, path, _), result) in read.into_iter().zip(built) {
            out.push((name, result.map_err(|e| format!("{}:\n{e}", path.display()))));
        }
        out
    }
}

/// A particle effect's functions from its file, `shaders/<name>.vfx.ron`:
/// the graph compiled ([`scrap_shadergraph::effect`]). Built into the
/// particles' shader by [`crate::particles_gpu::check_effect`] and
/// [`Renderer::set_effect_graph`].
pub fn effect_source(path: &std::path::Path) -> Result<String, String> {
    let text = scrap_core::files::read_to_string(path).map_err(|e| e.to_string())?;
    let graph = scrap_shadergraph::effect::parse(&text)?;
    let from = path.file_name().map(|f| format!("shaders/{}", f.to_string_lossy())).unwrap_or_default();
    scrap_shadergraph::effect::to_wgsl(&graph, &from)
}

/// Whether a material's `surface` builds over the standard shader, without
/// a GPU: spliced in, parsed and validated as the renderer would, the
/// error with its line and column. What `scrap check` asks of every shader.
pub fn check_material_shader(surface: &str) -> Result<(), String> {
    use wgpu::naga;
    let full = with_surface(SHADER, surface)?;
    let module = naga::front::wgsl::parse_str(&full).map_err(|e| e.emit_to_string(&full))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .map_err(|e| e.emit_to_string(&full))?;
    Ok(())
}

/// The shader a file in `shaders/` is, by name: `water.wgsl` and
/// `lava.graph.ron` (a shader graph) are `water` and `lava`. `None` for
/// anything else there.
pub fn material_shader_name(path: &std::path::Path) -> Option<String> {
    let file = path.file_name()?.to_str()?;
    if let Some(name) = scrap_shadergraph::shader_name(file) {
        return Some(name.to_string());
    }
    (path.extension()? == "wgsl").then(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))?
}

/// A material shader's `surface` source from its file: the file itself for
/// `.wgsl`, the compiled graph for `.graph.ron`. A graph with a `.wgsl` of
/// its name beside it is refused: one shader, one file.
pub fn material_shader_source(path: &std::path::Path) -> Result<String, String> {
    let text = scrap_core::files::read_to_string(path).map_err(|e| e.to_string())?;
    let Some(file) = path.file_name().and_then(|f| f.to_str()) else {
        return Ok(text);
    };
    let Some(name) = scrap_shadergraph::shader_name(file) else {
        return Ok(text);
    };
    let twin = path.with_file_name(format!("{name}.wgsl"));
    if scrap_core::files::read_to_string(&twin).is_ok() {
        return Err(format!(
            "`{name}.wgsl` beside it is shader `{name}` too: one shader, one file — keep the graph or the WGSL"
        ));
    }
    let graph = scrap_shadergraph::surface::parse(&text)?;
    scrap_shadergraph::surface::to_wgsl(&graph, &format!("shaders/{file}"))
}

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

/// Which scene pipeline a draw takes: skinned or not, which faces, and —
/// for a transparent surface — how it blends. Every combination is built
/// when the renderer is, so no frame ever waits on a pipeline (DNA,
/// postulate 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Look {
    skinned: bool,
    face: RenderFace,
    /// `None` is opaque.
    blend: Option<Blend>,
    /// Water, drawn by its own fragment shader.
    water: bool,
    /// A material's own shader ([`Renderer::set_material_shader`]); `None`
    /// is the standard one.
    shader: Option<crate::asset::AssetId>,
    /// Drawn over everything, walls included: see-through only.
    on_top: bool,
    /// Terrain's fine grid, placed and raised by its own vertex shader.
    terrain: bool,
    /// See-through and lit by nothing — smoke, dust, a glow — shaded by
    /// `fs_unlit`, which skips all the light's work and what goes before
    /// it: over a screen full of sprites that is most of the frame.
    unlit: bool,
}

impl Look {
    /// What its scene pipeline is built from.
    fn describe(self, prepassed: bool, samples: u32) -> crate::lean::Describe {
        crate::lean::Describe {
            skinned: self.skinned,
            terrain: self.terrain,
            water: self.water,
            unlit: self.unlit,
            prepassed,
            on_top: self.on_top,
            face: self.face,
            blend: self.blend.map(blend_state),
            samples,
        }
    }

    fn all() -> Vec<Look> {
        let mut out = Vec::new();
        for skinned in [false, true] {
            for face in [RenderFace::Front, RenderFace::Back, RenderFace::Both] {
                for blend in [
                    None,
                    Some(Blend::Alpha),
                    Some(Blend::Premultiply),
                    Some(Blend::Additive),
                    Some(Blend::Multiply),
                ] {
                    for on_top in [false, true] {
                        if on_top && blend.is_none() {
                            continue;
                        }
                        for unlit in [false, true] {
                            if unlit && (skinned || blend.is_none()) {
                                continue;
                            }
                            out.push(Look {
                                skinned,
                                face,
                                blend,
                                water: false,
                                shader: None,
                                on_top,
                                terrain: false,
                                unlit,
                            });
                        }
                    }
                }
            }
        }
        for face in [RenderFace::Front, RenderFace::Both] {
            out.push(Look {
                skinned: false,
                face,
                blend: Some(Blend::Premultiply),
                water: true,
                shader: None,
                on_top: false,
                terrain: false,
                unlit: false,
            });
        }
        // Terrain's fine grid: solid, its front faces.
        out.push(Look {
            skinned: false,
            face: RenderFace::Front,
            blend: None,
            water: false,
            shader: None,
            on_top: false,
            terrain: true,
            unlit: false,
        });
        out
    }

    fn of(material: &Material, skinned: bool) -> Look {
        if material.shading == Shading::Water {
            return Look {
                skinned: false,
                face: if material.render_face.culled() == RenderFace::Both {
                    RenderFace::Both
                } else {
                    RenderFace::Front
                },
                blend: Some(Blend::Premultiply),
                water: true,
                shader: None,
                on_top: false,
                terrain: false,
                unlit: false,
            };
        }
        Look {
            skinned,
            face: material.render_face.culled(),
            blend: material.is_transparent().then_some(material.blend),
            water: false,
            shader: material.shader,
            on_top: material.on_top && material.is_transparent(),
            terrain: false,
            unlit: material.shading == Shading::Unlit && material.is_transparent() && !skinned,
        }
    }
}

/// What of the renderer's pipelines depends on how many samples a pixel
/// the scene has, for one count.
struct SamplePipelines {
    samples: u32,
    scene: Pipelines,
    clusters: Option<crate::cluster::ClusterPipelines>,
    particles: crate::particles_gpu::Draws,
}

/// Every pipeline the renderer draws with.
#[derive(Clone)]
struct Pipelines {
    scene: std::collections::HashMap<Look, wgpu::RenderPipeline>,
    /// The solid looks again, for a scene pass that starts from the
    /// prepass's depth: equal to it, and with nothing to discard.
    prepassed: std::collections::HashMap<Look, wgpu::RenderPipeline>,
    shadow: wgpu::RenderPipeline,
    /// The same, back faces culled: the sun's cascades' one-sided casters.
    shadow_front: wgpu::RenderPipeline,
    /// The shadow pass for what is cut out by its alpha.
    shadow_clip: wgpu::RenderPipeline,
    shadow_clip_front: wgpu::RenderPipeline,
    /// Depth and normals of what is solid, for ambient occlusion: by
    /// skinned and render face.
    prepass: std::collections::HashMap<(bool, RenderFace, bool), wgpu::RenderPipeline>,
    overlay: wgpu::RenderPipeline,
    /// How many samples the overlay is drawn with.
    overlay_samples: u32,
    outline_mask: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    /// Rain and snow falling, over the frame.
    precipitation: wgpu::RenderPipeline,
    /// Volumetric fog: what each cell scatters, and the sums along the view.
    fog_inject: wgpu::ComputePipeline,
    fog_integrate: wgpu::ComputePipeline,
    /// Terrain's fine grid by task and mesh shaders — drawn and in the
    /// prepass — where the device has them and they built.
    terrain_mesh: Option<(wgpu::RenderPipeline, wgpu::RenderPipeline)>,
}

/// The layouts the pipelines are built against, kept to rebuild them when
/// the shader is reloaded.
struct Layouts<'a> {
    main: &'a wgpu::PipelineLayout,
    shadow: &'a wgpu::PipelineLayout,
    shadow_clip: &'a wgpu::PipelineLayout,
    skinned: &'a wgpu::PipelineLayout,
    sky: &'a wgpu::PipelineLayout,
    fog_inject: &'a wgpu::PipelineLayout,
    fog_integrate: &'a wgpu::PipelineLayout,
    /// The terrain's mesh-shader pipelines: the main groups and its own.
    terrain_mesh: Option<&'a wgpu::PipelineLayout>,
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 13] = wgpu::vertex_attr_array![
    3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4,
    7 => Float32x4, 10 => Float32x4, 11 => Float32x4, 12 => Float32x4, 13 => Float32x4,
    14 => Float32x4, 15 => Float32x4, 16 => Float32x4, 17 => Uint32x4
];
const SKIN_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![8 => Uint16x4, 9 => Float32x4];
const COLOR_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![18 => Unorm8x4];

/// A buffer of white vertex colours, `count` of them.
fn white_buffer(gpu: &Gpu, count: u64) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("white vertex colours"),
        contents: &vec![255u8; count as usize * 4],
        usage: wgpu::BufferUsages::VERTEX,
    })
}

pub(crate) fn vertex_buffers(skinned: bool) -> Vec<Option<wgpu::VertexBufferLayout<'static>>> {
    let mut out = vec![
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::asset::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<InstanceRaw>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRIBUTES,
        }),
        // Slot 2: the vertices' painted colours (or the white stand-in).
        Some(wgpu::VertexBufferLayout {
            array_stride: 4,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &COLOR_ATTRIBUTES,
        }),
    ];
    if skinned {
        out.push(Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SkinVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &SKIN_ATTRIBUTES,
        }));
    }
    out
}

/// URP's blending modes as blend states: what a transparent surface does
/// to what is already there.
fn blend_state(blend: Blend) -> wgpu::BlendState {
    use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation::Add};
    let over = |src| C {
        src_factor: src,
        dst_factor: F::OneMinusSrcAlpha,
        operation: Add,
    };
    match blend {
        Blend::Alpha => wgpu::BlendState {
            color: over(F::SrcAlpha),
            alpha: over(F::One),
        },
        Blend::Premultiply => wgpu::BlendState {
            color: over(F::One),
            alpha: over(F::One),
        },
        Blend::Additive => wgpu::BlendState {
            color: C {
                src_factor: F::SrcAlpha,
                dst_factor: F::One,
                operation: Add,
            },
            alpha: C {
                src_factor: F::Zero,
                dst_factor: F::One,
                operation: Add,
            },
        },
        Blend::Multiply => wgpu::BlendState {
            color: C {
                src_factor: F::Dst,
                dst_factor: F::Zero,
                operation: Add,
            },
            alpha: C {
                src_factor: F::Zero,
                dst_factor: F::One,
                operation: Add,
            },
        },
    }
}

/// The scene's pipelines for these looks, from one shader module: the
/// standard shader's at start, a material's own when it is set.
fn scene_pipelines(
    gpu: &Gpu,
    shader: &wgpu::ShaderModule,
    samples: u32,
    layouts: &Layouts,
    looks: Vec<Look>,
) -> (
    std::collections::HashMap<Look, wgpu::RenderPipeline>,
    std::collections::HashMap<Look, wgpu::RenderPipeline>,
    Option<wgpu::Error>,
) {
    let scene_pipeline = |look: Look, prepassed: bool| {
        crate::lean::scene_pipeline(
            &gpu.device,
            shader,
            if look.skinned { layouts.skinned } else { layouts.main },
            look.describe(prepassed, samples),
            false,
        )
    };
    // Each pipeline is the driver compiling the whole shader once more:
    // dozens of them, the most of a renderer's start. They do not depend
    // on each other and the device takes them from any thread, so they are
    // compiled across the cores (one after another on the web).
    let mut wanted: Vec<(Look, bool)> = looks.iter().map(|&look| (look, false)).collect();
    wanted.extend(
        looks
            .iter()
            .filter(|look| look.blend.is_none() && !look.water && !look.on_top)
            .map(|&look| (look, true)),
    );
    let (built, error) = compiled(gpu, &wanted, |&(look, prepassed)| scene_pipeline(look, prepassed));
    let (mut scene, mut prepassed) = (std::collections::HashMap::new(), std::collections::HashMap::new());
    for ((look, pre), pipeline) in wanted.into_iter().zip(built) {
        if pre {
            prepassed.insert(look, pipeline);
        } else {
            scene.insert(look, pipeline);
        }
    }
    (scene, prepassed, error)
}

/// What a material shader is built on: the standard shader and how the
/// renderer's pipelines are laid out.
struct MaterialBase<'a> {
    shader: &'a str,
    traced: bool,
    bindless: bool,
    samples: u32,
    layouts: &'a Layouts<'a>,
}

/// A material shader's module and its pipelines: the standard shader with
/// `surface` put in, checked, and built for every lit look.
#[allow(clippy::type_complexity)]
fn material_pipelines(
    gpu: &Gpu,
    base: &MaterialBase,
    id: crate::asset::AssetId,
    surface: &str,
) -> Result<
    (
        wgpu::ShaderModule,
        std::collections::HashMap<Look, wgpu::RenderPipeline>,
        std::collections::HashMap<Look, wgpu::RenderPipeline>,
    ),
    String,
> {
    let composed = with_surface(base.shader, surface)?;
    let source = crate::bindless::prepared(
        &if base.traced {
            crate::ray::traced(&composed)
        } else {
            composed
        },
        base.bindless,
    );
    use wgpu::naga;
    let module = naga::front::wgsl::parse_str(&source).map_err(|e| e.emit_to_string(&source))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .map_err(|e| e.emit_to_string(&source))?;
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scrap::material shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    if let Some(error) = pollster::block_on(scope.pop()) {
        return Err(format!("the shader does not fit the renderer: {error}"));
    }
    // Not the `fs_unlit` looks: each is another pipeline to compile for
    // every material shader, for the few see-through unlit ones.
    let looks: Vec<Look> = Look::all()
        .into_iter()
        .filter(|look| !look.unlit)
        .map(|look| Look { shader: Some(id), ..look })
        .collect();
    let (scene, prepassed, error) = scene_pipelines(gpu, &shader, base.samples, base.layouts, looks);
    if let Some(error) = error {
        return Err(format!("the shader does not fit the renderer: {error}"));
    }
    Ok((shader, scene, prepassed))
}

/// `make` over `items` on the workers, each under an error scope of its
/// own — wgpu's scopes are the thread's, so one the caller pushed would not
/// see what went wrong on another — and the first error back with the
/// results, for the caller to report as its own scope would have.
fn compiled<T: Sync, R: Send>(
    gpu: &Gpu,
    items: &[T],
    make: impl Fn(&T) -> R + Sync,
) -> (Vec<R>, Option<wgpu::Error>) {
    let made = scrap_core::jobs::map(items, 1, |item| scoped(gpu, || make(item)));
    let mut first = None;
    let out = made
        .into_iter()
        .map(|(result, error)| {
            if first.is_none() {
                first = error;
            }
            result
        })
        .collect();
    (out, first)
}

/// `make` under a validation error scope of its own, on this thread.
fn scoped<R>(gpu: &Gpu, make: impl FnOnce() -> R) -> (R, Option<wgpu::Error>) {
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let result = make();
    (result, pollster::block_on(scope.pop()))
}

/// Every pipeline the renderer draws with, from one shader module: at
/// start, and again when the shader is reloaded. The scene draws into the
/// HDR format, multisampled `samples` times; overlays onto `output`, after
/// post-processing.
/// The terrain functions of the renderer's shader, for the mesh stages:
/// the same code reading their own group (`frame.` as `tframe.`, the
/// heights as `mesh_heights`), each name with `_m`.
fn mesh_stage_copy(source: &str) -> Option<String> {
    let begin = source.find("// terrain-stage:begin")?;
    let end = source.find("// terrain-stage:end")?;
    let mut copy = source[begin..end]
        .replace("frame.", "tframe.")
        .replace("terrain_heights", "mesh_heights");
    for name in [
        "relief_height",
        "sand_wind",
        "ripples_at",
        "terrain_ground",
        "terrain_vertex",
    ] {
        copy = copy.replace(&format!("{name}("), &format!("{name}_m("));
    }
    Some(copy)
}

/// What of the frame the terrain's mesh stages read: `TerrainFrame` in
/// terrain_mesh.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TerrainFrameUniform {
    view_projection: [[f32; 4]; 4],
    camera_position: [f32; 4],
    wind: [f32; 4],
    terrain_to_local: [[f32; 4]; 4],
    terrain_to_world: [[f32; 4]; 4],
    terrain: [f32; 4],
    terrain_bounds: [f32; 4],
    terrain_look: [[f32; 4]; 7],
}

/// The terrain's mesh-shader pipelines, from the renderer's shader `source`
/// with the task and mesh stages after it: `None` where the device has no
/// mesh shaders or they do not build (the vertex-shader grid draws then).
fn terrain_mesh_pipelines(
    gpu: &Gpu,
    source: &str,
    samples: u32,
    layouts: &Layouts,
) -> Option<(wgpu::RenderPipeline, wgpu::RenderPipeline)> {
    let layout = layouts.terrain_mesh?;
    let full = format!(
        "enable wgpu_mesh_shader;\n{source}\n{}\n{TERRAIN_MESH}",
        mesh_stage_copy(source)?
    );
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    // SAFETY: the task and mesh stages index only arrays they size
    // themselves, within their loops' bounds; naga's added checks (and
    // clearing the workgroups' memory) cost the mesh draw most of its time.
    let module = unsafe {
        gpu.device.create_shader_module_trusted(
            wgpu::ShaderModuleDescriptor {
                label: Some("scrap::terrain mesh"),
                source: wgpu::ShaderSource::Wgsl(full.into()),
            },
            wgpu::ShaderRuntimeChecks::unchecked(),
        )
    };
    let pipeline = |fragment: &str, target: wgpu::ColorTargetState, depth, samples| {
        gpu.device
            .create_mesh_pipeline(&wgpu::MeshPipelineDescriptor {
                label: Some("scrap::terrain mesh"),
                layout: Some(layout),
                task: Some(wgpu::TaskState {
                    module: &module,
                    entry_point: Some("ts_terrain"),
                    compilation_options: Default::default(),
                }),
                mesh: wgpu::MeshState {
                    module: &module,
                    entry_point: Some("ms_terrain"),
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(target)],
                }),
                multiview: None,
                cache: None,
            })
    };
    let drawn = pipeline(
        "fs",
        wgpu::ColorTargetState {
            format: crate::post::HDR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        },
        DEPTH_FORMAT,
        samples,
    );
    let prepass = pipeline(
        "fs_normals",
        crate::ssao::NORMAL_FORMAT.into(),
        crate::ssao::PREPASS_DEPTH,
        1,
    );
    match pollster::block_on(scope.pop()) {
        Some(error) => {
            eprintln!("terrain by mesh shaders did not build, drawn without: {error}");
            None
        }
        None => Some((drawn, prepass)),
    }
}

fn build_pipelines(
    gpu: &Gpu,
    shader: &wgpu::ShaderModule,
    source: &str,
    output: wgpu::TextureFormat,
    samples: u32,
    layouts: &Layouts,
) -> (Pipelines, Option<wgpu::Error>) {
    let format = crate::post::HDR_FORMAT;
    let multisample = wgpu::MultisampleState {
        count: samples,
        ..Default::default()
    };
    let (scene, prepassed, error) = scene_pipelines(gpu, shader, samples, layouts, Look::all());
    let prepass_pipeline = |skinned: bool, face: RenderFace, terrain: bool| {
        let buffers = vertex_buffers(skinned);
        gpu.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("scrap::prepass"),
                layout: Some(if skinned {
                    layouts.skinned
                } else {
                    layouts.main
                }),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some(if terrain {
                        "vs_terrain"
                    } else if skinned {
                        "vs_skinned"
                    } else {
                        "vs"
                    }),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_normals"),
                    compilation_options: Default::default(),
                    targets: &[Some(crate::ssao::NORMAL_FORMAT.into())],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: match face {
                        RenderFace::Front => Some(wgpu::Face::Back),
                        RenderFace::Back => Some(wgpu::Face::Front),
                        RenderFace::Both | RenderFace::BothAsFront => None,
                    },
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: crate::ssao::PREPASS_DEPTH,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
    };
    let mut prepass = std::collections::HashMap::new();
    for skinned in [false, true] {
        for face in [RenderFace::Front, RenderFace::Back, RenderFace::Both] {
            prepass.insert(
                (skinned, face, false),
                prepass_pipeline(skinned, face, false),
            );
        }
    }
    prepass.insert(
        (false, RenderFace::Front, true),
        prepass_pipeline(false, RenderFace::Front, true),
    );

    // Depth only: no fragment stage at all, because nothing is written
    // but depth and a colour target would only cost fill.
    let buffers = vertex_buffers(false);
    let shadow_pipeline = |cull_mode: Option<wgpu::Face>| {
        gpu.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("scrap::shadow"),
                layout: Some(layouts.shadow),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_shadow"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                fragment: None,
                primitive: wgpu::PrimitiveState {
                    cull_mode,
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
            })
    };
    // Both sides: a card, a leaf, a flag that faces the sun has no far
    // side, and the lamps' maps and the virtual pages draw everything so.
    let shadow = shadow_pipeline(None);
    // Front faces only, for the sun's cascades: what one-sided things
    // cast, as URP's caster pass culls as the material does. Its normal
    // bias shrinks a caster only while its back faces stay out.
    let shadow_front = shadow_pipeline(Some(wgpu::Face::Back));

    // Tools, with the same vertex layout, into the tools' own picture
    // (crate::tools): no depth at all, so an overlay neither hides behind
    // the scene nor blocks anything drawn after it; blended, premultiplied,
    // so a translucent handle is; four samples where the format has them,
    // so a thin one is smooth.
    let overlay_samples = if gpu
        .adapter
        .get_texture_format_features(output)
        .flags
        .sample_count_supported(4)
    {
        4
    } else {
        1
    };
    let tool_pipeline = |label, entry, format, blend, samples, depth: Option<wgpu::DepthStencilState>| {
        gpu.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layouts.main),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_tool"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: depth,
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
    };
    let overlay = tool_pipeline(
        "scrap::overlay",
        "fs_tool",
        output,
        Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        overlay_samples,
        None,
    );
    // What is outlined, into the mask: the nearest outlined surface a
    // pixel's, whatever else is in front of it.
    let outline_mask = tool_pipeline(
        "scrap::outline mask",
        "fs_outline_mask",
        crate::tools::MASK_FORMAT,
        None,
        1,
        Some(wgpu::DepthStencilState {
            format: crate::tools::MASK_DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
    );

    // The sky: one triangle over the screen at the far plane, drawn after
    // the opaque things so only what they left uncovered is shaded.
    let sky = gpu
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scrap::sky"),
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

    // Rain and snow: over everything, added, depth left alone.
    let precipitation = gpu
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scrap::precipitation"),
            layout: Some(layouts.sky),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_precipitation"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(blend_state(Blend::Premultiply)),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample,
            multiview_mask: None,
            cache: None,
        });

    let shadow_clip_pipeline = |cull_mode: Option<wgpu::Face>| {
        gpu.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("scrap::shadow (clipped)"),
                layout: Some(layouts.shadow_clip),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_shadow_clip"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_shadow_clip"),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode,
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
            })
    };
    // A cut-out is usually a card seen from both sides.
    let shadow_clip = shadow_clip_pipeline(None);
    let shadow_clip_front = shadow_clip_pipeline(Some(wgpu::Face::Back));

    let compute = |layout: &wgpu::PipelineLayout, entry: &str| {
        gpu.device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module: shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
    };
    let pipelines = Pipelines {
        scene,
        prepassed,
        shadow,
        shadow_front,
        shadow_clip,
        shadow_clip_front,
        prepass,
        overlay,
        overlay_samples,
        outline_mask,
        sky,
        precipitation,
        fog_inject: compute(layouts.fog_inject, "cs_fog_inject"),
        fog_integrate: compute(layouts.fog_integrate, "cs_fog_integrate"),
        terrain_mesh: terrain_mesh_pipelines(gpu, source, samples, layouts),
    };
    (pipelines, error)
}

pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

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
        // What was kept for the other sample count is the old shader's.
        self.other_samples = None;
        self.rebuild_pipelines(gpu, source)
    }

    /// Every pipeline made again from `source`, with the samples now.
    fn rebuild_pipelines(&mut self, gpu: &Gpu, source: &str) -> Result<(), String> {
        let original = source;
        use wgpu::naga;
        let prepared = crate::bindless::prepared(
            &if self.ray.is_some() {
                crate::ray::traced(source)
            } else {
                source.to_string()
            },
            self.bindless.is_some(),
        );
        let source = prepared.as_str();
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
                label: Some("scrap::render (reloaded)"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let (pipelines, built_error) = build_pipelines(
            gpu,
            &shader,
            source,
            self.format,
            self.samples,
            &Layouts {
                main: &self.pipeline_layout,
                shadow: &self.shadow_pipeline_layout,
                shadow_clip: &self.shadow_clip_layout,
                skinned: &self.skinned_layout,
                sky: &self.sky_layout,
                fog_inject: &self.fog_inject_layout,
                fog_integrate: &self.fog_integrate_layout,
                terrain_mesh: self.terrain_mesh_layouts.as_ref().map(|(_, p)| p),
            },
        );
        let cluster_pipelines = self.clusters.make_pipelines(gpu, &shader, self.samples);
        let ddgi_pipelines = self.ddgi.make_pipelines(gpu, &shader, self.ray.is_some());
        let restir_pipelines = self.restir.make_pipelines(gpu, &shader, self.ray.is_some());
        if let Some(error) = built_error.or(pollster::block_on(scope.pop())) {
            return Err(format!("the shader does not fit the renderer: {error}"));
        }
        self.pipelines = pipelines;
        self.lean_modules.insert(None, shader.clone());
        self.lean.forget();
        self.clusters.pipelines = cluster_pipelines;
        (self.ddgi.trace, self.ddgi.update) = ddgi_pipelines;
        (self.restir.initial, self.restir.spatial) = restir_pipelines;
        self.base_shader = original.to_string();
        // The materials' own shaders are the standard one with their
        // surface in: built again on the new one.
        let own: Vec<(crate::asset::AssetId, String)> = self
            .material_shaders
            .iter()
            .map(|(id, s)| (*id, s.clone()))
            .collect();
        for e in self.set_material_shaders(gpu, &own).into_iter().filter_map(Result::err) {
            eprintln!("a material's shader no longer builds on the reloaded one: {e}");
        }
        Ok(())
    }

    /// Draw the scene with `samples` a pixel: its targets made again on
    /// the next frame, its pipelines now.
    ///
    /// The pipelines it drew with are kept: going back (the editor's Game
    /// view and Scene view) swaps them in, and only the first change
    /// makes any — a third of a second of them.
    fn set_samples(&mut self, gpu: &Gpu, samples: u32) {
        let before = self.samples;
        let cached = self.other_samples.take();
        let kept = match cached {
            Some(set) if set.samples == samples => {
                let now = SamplePipelines {
                    samples: before,
                    scene: std::mem::replace(&mut self.pipelines, set.scene),
                    clusters: std::mem::replace(&mut self.clusters.pipelines, set.clusters),
                    particles: self.gpu_particles.swap_draw(gpu, set.particles),
                };
                self.samples = samples;
                now
            }
            _ => {
                let particles = self.gpu_particles.make_draw(gpu, samples);
                let shader = self.base_shader.clone();
                self.samples = samples;
                // Built into place: what it replaces is what is kept.
                let old_scene = self.pipelines.clone();
                let old_clusters = self.clusters.pipelines.clone();
                if let Err(e) = self.rebuild_pipelines(gpu, &shader) {
                    eprintln!("the scene's pipelines do not build with {samples} samples: {e}");
                    self.samples = before;
                    return;
                }
                SamplePipelines {
                    samples: before,
                    scene: old_scene,
                    clusters: old_clusters,
                    particles: self.gpu_particles.swap_draw(gpu, particles),
                }
            }
        };
        self.other_samples = Some(kept);
        self.depth_size = (0, 0);
        self.lean.forget();
    }

    /// The pipeline for a look: a material's own shader's, or the standard
    /// one's while that shader is not in (not yet written, or broken).
    /// Over the prepass's depth ([`Renderer::depth_prepassed`]), a solid
    /// look takes its equal-depth pipeline.
    fn scene_pipeline(&self, look: Look) -> Option<&wgpu::RenderPipeline> {
        let (key, prepassed) = self.scene_key(look)?;
        if self.lean.on {
            if let Some(lean) = self.lean.ready.get(&LeanKey::Scene(key, prepassed)) {
                return Some(lean);
            }
        }
        let map = if prepassed { &self.pipelines.prepassed } else { &self.pipelines.scene };
        map.get(&key)
    }

    /// Which pipeline draws a look: the look it is filed under (a
    /// material's own shader has no `fs_unlit` looks — its shade()
    /// returns early for unlit anyway — so its standard one, then the
    /// standard shader's), and whether over the prepass's depth.
    fn scene_key(&self, look: Look) -> Option<(Look, bool)> {
        let prepassed = self.depth_prepassed && look.blend.is_none() && !self.cuts(look);
        let map = if prepassed { &self.pipelines.prepassed } else { &self.pipelines.scene };
        [look, Look { unlit: false, ..look }, Look { shader: None, ..look }]
            .into_iter()
            .find(|l| map.contains_key(l))
            .map(|l| (l, prepassed))
    }

    /// Ask for the lean pipelines of the looks this frame draws, both over
    /// the prepass's depth and not, where they would be drawn so.
    fn ask_lean(&mut self, gpu: &Gpu, looks: impl Iterator<Item = Look>) {
        let was = self.depth_prepassed;
        let mut wanted = Vec::new();
        for look in looks {
            for prepassed in [false, true] {
                self.depth_prepassed = prepassed;
                if let Some(key) = self.scene_key(look) {
                    if !self.lean.asked(&LeanKey::Scene(key.0, key.1)) && !wanted.contains(&key) {
                        wanted.push(key);
                    }
                }
            }
        }
        self.depth_prepassed = was;
        for (look, prepassed) in wanted {
            let Some(module) = self.lean_modules.get(&look.shader).cloned() else {
                continue;
            };
            let layout = if look.skinned { self.skinned_layout.clone() } else { self.pipeline_layout.clone() };
            let describe = look.describe(prepassed, self.samples);
            let build: crate::lean::Build = Box::new(move |device: &wgpu::Device| {
                crate::lean::scene_pipeline(device, &module, &layout, describe, true)
            });
            self.lean.ask(&gpu.device, LeanKey::Scene(look, prepassed), build);
        }
    }

    /// Ask for the lean pipelines drawing culled clusters of these faces.
    fn ask_lean_clusters(&mut self, gpu: &Gpu, wanted: impl Iterator<Item = (RenderFace, bool)>) {
        let Some(module) = self.lean_modules.get(&None).cloned() else {
            return;
        };
        for (face, water) in wanted {
            let key = LeanKey::Cluster(face, water);
            if !self.lean.asked(&key) {
                let build = self.clusters.lean_build(&module, face, water, self.samples);
                self.lean.ask(&gpu.device, key, build);
            }
        }
    }

    /// Whether a look's own shader cuts its surface out (`discard`): the
    /// prepass, which does not run it, would lay the whole of it, so it is
    /// left out there and drawn over the prepass's depth as a see-through
    /// thing is, by its own test.
    fn cuts(&self, look: Look) -> bool {
        look.shader
            .and_then(|id| self.material_shaders.get(&id))
            .is_some_and(|s| s.contains("discard"))
    }

    /// Give materials whose `shader` is `id` their own `surface` function
    /// (see `render.wgsl`): the standard shader with it put in, checked,
    /// and its pipelines built. Refused in words — file, line, column —
    /// with whatever it had before kept drawing.
    /// Put in effect `name` for the GPU's particles, from its graph's
    /// functions ([`scrap_shadergraph::effect::to_wgsl`]): emitters with
    /// `graph: "<name>"` move and look by it from the next frame. Refused
    /// in words when it does not build, and the one before goes on.
    pub fn set_effect_graph(&mut self, gpu: &Gpu, name: &str, effect: &str) -> Result<(), String> {
        self.other_samples = None;
        self.gpu_particles.set_effect(gpu, name, effect)
    }

    /// Whether effect `name` is in for the GPU's particles.
    pub fn has_effect_graph(&self, name: &str) -> bool {
        !name.is_empty() && self.gpu_particles.has_effect(name)
    }

    pub fn set_material_shader(
        &mut self,
        gpu: &Gpu,
        id: crate::asset::AssetId,
        surface: &str,
    ) -> Result<(), String> {
        self.set_material_shaders(gpu, &[(id, surface.to_string())]).remove(0)
    }

    /// [`Renderer::set_material_shader`] for several at once, built side
    /// by side on every core: a game's shaders at its start. Each one's
    /// result, in their order.
    pub fn set_material_shaders(
        &mut self,
        gpu: &Gpu,
        shaders: &[(crate::asset::AssetId, String)],
    ) -> Vec<Result<(), String>> {
        self.other_samples = None;
        let layouts = Layouts {
            main: &self.pipeline_layout,
            shadow: &self.shadow_pipeline_layout,
            shadow_clip: &self.shadow_clip_layout,
            skinned: &self.skinned_layout,
            sky: &self.sky_layout,
            fog_inject: &self.fog_inject_layout,
            fog_integrate: &self.fog_integrate_layout,
            terrain_mesh: self.terrain_mesh_layouts.as_ref().map(|(_, p)| p),
        };
        let base = MaterialBase {
            shader: &self.base_shader,
            traced: self.ray.is_some(),
            bindless: self.bindless.is_some(),
            samples: self.samples,
            layouts: &layouts,
        };
        let built = scrap_core::jobs::map(shaders, 1, |(id, surface)| material_pipelines(gpu, &base, *id, surface));
        let mut out = Vec::with_capacity(shaders.len());
        for ((id, surface), built) in shaders.iter().zip(built) {
            out.push(built.map(|(shader, scene, prepassed)| {
                self.pipelines.scene.extend(scene);
                self.pipelines.prepassed.extend(prepassed);
                self.lean_modules.insert(Some(*id), shader);
                self.material_shaders.insert(*id, surface.clone());
                self.shader_textures.insert(*id, declared_textures(surface));
            }));
        }
        if out.iter().any(|r| r.is_ok()) {
            self.lean.forget();
        }
        out
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
                label: Some("scrap::render"),
                source: wgpu::ShaderSource::Wgsl(
                    crate::bindless::prepared(
                        &if gpu.ray_tracing {
                            crate::ray::traced(SHADER)
                        } else {
                            SHADER.to_string()
                        },
                        gpu.bindless && !gpu.mesh_shaders,
                    )
                    .into(),
                ),
            });

        let frame_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut frame_entries = vec![
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
                    view_dimension: wgpu::TextureViewDimension::D2Array,
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
            // Ambient occlusion, read a texel per pixel.
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ];
        // Lights, clustered: the lights, each cell's run, the runs; the
        // lamps' shadow maps and their views.
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        frame_entries.extend([
            storage(6),
            storage(7),
            storage(8),
            wgpu::BindGroupLayoutEntry {
                binding: 9,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            // The lamps' shadow views: a uniform, not storage — a stage has
            // as few as four storage buffers on some devices.
            wgpu::BindGroupLayoutEntry {
                binding: 10,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // The reflection probes' pictures, six layers each, and how
            // they are filtered across mips.
            wgpu::BindGroupLayoutEntry {
                binding: 11,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 12,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            // Decals, and their pictures: colours and normals.
            storage(13),
            wgpu::BindGroupLayoutEntry {
                binding: 14,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 15,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
        ]);
        // Volumetric fog, summed eye to cell, and how it is filtered.
        frame_entries.extend([
            wgpu::BindGroupLayoutEntry {
                binding: 16,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 17,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            // The physical sky's table, and its aerial grid.
            wgpu::BindGroupLayoutEntry {
                binding: 18,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            // Terrain heights, read by the fine grid's vertex shader.
            wgpu::BindGroupLayoutEntry {
                binding: 23,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            // How the grass is trampled, read by the vertex shader.
            wgpu::BindGroupLayoutEntry {
                binding: 30,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            // The frame's instances, read by the fragment stage (`expand`).
            wgpu::BindGroupLayoutEntry {
                binding: 33,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // The scene's distance field, for occlusion and soft shadows.
            wgpu::BindGroupLayoutEntry {
                binding: 31,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
            // The last frame, for screen-space reflections.
            wgpu::BindGroupLayoutEntry {
                binding: 22,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            // The clouds, a quarter of the frame.
            wgpu::BindGroupLayoutEntry {
                binding: 21,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            // The solid scene's depth, from the prepass: what water sees
            // under itself.
            wgpu::BindGroupLayoutEntry {
                binding: 20,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 19,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
        ]);
        // ReSTIR's reservoirs, for the lit shader.
        frame_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 27,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        // The virtual shadow maps' page table.
        frame_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 26,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        // The irradiance volume's probes' pictures.
        frame_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 25,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        // The fog's compute passes read the frame as the lit shader does.
        for entry in &mut frame_entries {
            entry.visibility |= wgpu::ShaderStages::COMPUTE;
        }
        // The scene as rays see it, on a device that traces.
        if gpu.ray_tracing {
            frame_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 5,
                // The fog's cells trace too: shafts through what stands in
                // the light.
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::AccelerationStructure {
                    vertex_return: false,
                },
                count: None,
            });
            // What each thing in it is made of, for reflections' hits and
            // the irradiance volume's rays.
            frame_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 24,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
        }
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("frame"),
                entries: &frame_entries,
            });
        let ssao = crate::ssao::SsaoRenderer::new(gpu);
        let ray = crate::ray::scene(gpu);
        let shadow_resolution = ShadowSettings::default().resolution;
        let (shadow_map, shadow_layers) = shadow_view(gpu, shadow_resolution);
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
        let storage_buffer = |label, size: u64| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let light_buffer = storage_buffer(
            "lights",
            (crate::lights::MAX_LIGHTS * std::mem::size_of::<crate::lights::GpuLight>()) as u64,
        );
        let cell_buffer = storage_buffer(
            "light cells",
            (crate::lights::TILES_X * crate::lights::TILES_Y * crate::lights::SLICES) as u64 * 16,
        );
        let index_capacity = 1024;
        let index_buffer = storage_buffer("light lists", index_capacity * 4);
        let light_view_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lamp shadow views"),
            size: crate::lights::SHADOW_LAYERS as u64 * 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let light_shadow_resolution = ShadowSettings::default().light_resolution;
        let (light_shadow_map, light_shadow_layers) = shadow_layers_view(
            gpu,
            light_shadow_resolution,
            crate::lights::SHADOW_LAYERS as u32,
        );
        let reflections = crate::reflections::ProbeStore::new(gpu);
        let decal_buffer = storage_buffer(
            "decals",
            (crate::decals::MAX_DECALS * std::mem::size_of::<crate::decals::GpuDecal>()) as u64,
        );
        let decal_atlases = crate::decals::DecalAtlases::new(gpu);
        let volumes = crate::volume::Volumes::new(gpu);
        let atmosphere = crate::atmosphere::AtmosphereRenderer::new(gpu);
        let clouds = crate::clouds::CloudRenderer::new(gpu);
        let terrain_heights = terrain_height_view(gpu, 1, &[0.0]);
        let mut ddgi = crate::ddgi::Ddgi::new(gpu, &layout);
        let vsm_table = crate::vsm::table_buffer(gpu);
        let mut restir = crate::restir::Restir::new(gpu, &layout);
        let trample_texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("trample"),
            size: wgpu::Extent3d {
                width: crate::foliage::TRAMPLE_CELLS,
                height: crate::foliage::TRAMPLE_CELLS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let trample_view = trample_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let distance = crate::distance::DistanceTexture::new(gpu);
        let instance_capacity = 256;
        let instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: instance_capacity * std::mem::size_of::<InstanceRaw>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let bind_group = frame_bind_group(
            gpu,
            &layout,
            &FrameInputs {
                ddgi: &ddgi.probes,
                vsm_pages: &vsm_table,
                restir: &restir.shade,
                terrain_heights: &terrain_heights,
                trample: &trample_view,
                distance: &distance.view,
                instances: &instances,
                // Any texel will do until the scene's targets exist; the
                // renderer rebinds once it is built.
                history: clouds.view(),
                clouds: clouds.view(),
                scene_depth: &ssao.depth,
                sky_view: &atmosphere.sky_view,
                aerial: &atmosphere.aerial,
                fog: &volumes.integrated,
                fog_sampler: &volumes.sampler,
                decals: &decal_buffer,
                decal_colours: decal_atlases.colour_view(),
                decal_normals: decal_atlases.normal_view(),
                probes: &reflections.view,
                probe_sampler: &reflections.sampler,
                frame: &frame_buffer,
                shadow_map: &shadow_map,
                shadow_sampler: &shadow_sampler,
                occlusion: &ssao.result,
                rays: ray.as_ref().map(|r| r.bindings()),
                lights: &light_buffer,
                cells: &cell_buffer,
                indices: &index_buffer,
                light_views: &light_view_buffer,
                light_shadow_map: &light_shadow_map,
            },
        );

        // Bindless where the device can and the terrain is not drawn by
        // mesh shaders (whose stage carries no maps).
        let bindless_on = gpu.bindless && !gpu.mesh_shaders;
        let texture_layout = if bindless_on {
            crate::bindless::layout(gpu)
        } else {
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("surface maps"),
                    entries: &[
                        map_entry(0),
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                        map_entry(2),
                        map_entry(3),
                        map_entry(4),
                        // The material's own four, for its shader.
                        map_entry(5),
                        map_entry(6),
                        map_entry(7),
                        map_entry(8),
                    ],
                })
        };
        let texture_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("surface"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            // A floor or a road seen along its length stays sharp.
            anisotropy_clamp: 16,
            ..Default::default()
        });

        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("scrap::render"),
                bind_group_layouts: &[Some(&layout), Some(&texture_layout)],
                immediate_size: 0,
            });
        // The terrain's task and mesh stages: a group of their own (see
        // terrain_mesh.wgsl for why), after the main ones.
        let terrain_mesh_layouts = gpu.mesh_shaders.then(|| {
            let stages = wgpu::ShaderStages::TASK | wgpu::ShaderStages::MESH;
            let group = gpu
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("terrain mesh"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: stages,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: stages,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                    ],
                });
            let pipeline = gpu
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("scrap::terrain mesh"),
                    bind_group_layouts: &[Some(&layout), Some(&texture_layout), None, Some(&group)],
                    immediate_size: 0,
                });
            (group, pipeline)
        });
        let terrain_mesh_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain mesh"),
            size: std::mem::size_of::<TerrainFrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // The shadow pass sees one cascade's matrix, picked by a dynamic
        // offset: binding 3, beside the frame's, so the one shader module
        // serves both.
        let caster_size = std::mem::size_of::<CasterUniform>() as u64;
        let caster_alignment = gpu
            .device
            .limits()
            .min_uniform_buffer_offset_alignment
            .max(1) as u64;
        let caster_stride = caster_size.div_ceil(caster_alignment) * caster_alignment;
        let casters = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("casters"),
            size: caster_stride * (MAX_CASCADES + crate::lights::SHADOW_LAYERS) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shadow frame"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(caster_size),
                    },
                    count: None,
                }],
            });
        let shadow_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow frame"),
            layout: &shadow_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &casters,
                    offset: 0,
                    size: wgpu::BufferSize::new(caster_size),
                }),
            }],
        });
        let shadow_pipeline_layout =
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("scrap::shadow"),
                    bind_group_layouts: &[Some(&shadow_layout)],
                    immediate_size: 0,
                });
        // Cut-out casters need their texture: a leaf's shadow is a leaf.
        let shadow_clip_layout =
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("scrap::shadow (clipped)"),
                    bind_group_layouts: &[Some(&shadow_layout), Some(&texture_layout)],
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
                label: Some("scrap::skinned"),
                bind_group_layouts: &[Some(&layout), Some(&texture_layout), Some(&pose_layout)],
                immediate_size: 0,
            });

        let sky_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("scrap::sky"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let fog_layout = |label, second: &wgpu::BindGroupLayout| {
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    // Group 3: the render pipelines' groups 1 and 2 (maps,
                    // poses) share the one shader module.
                    bind_group_layouts: &[Some(&layout), None, None, Some(second)],
                    immediate_size: 0,
                })
        };
        let fog_inject_layout = fog_layout("scrap::fog inject", &volumes.inject_layout);
        let fog_integrate_layout = fog_layout("scrap::fog integrate", &volumes.integrate_layout);
        // One sample: a frame is smoothed by TAA unless it says otherwise,
        // and the pipelines made now are the ones it draws with. A frame
        // with no smoothing at all makes them again for four
        // (`set_samples`), once.
        let samples = 1;
        let shader_source = crate::bindless::prepared(
            &if gpu.ray_tracing {
                crate::ray::traced(SHADER)
            } else {
                SHADER.to_string()
            },
            bindless_on,
        );
        let (pipelines, built_error) = build_pipelines(
            gpu,
            &shader,
            &shader_source,
            format,
            samples,
            &Layouts {
                main: &pipeline_layout,
                shadow: &shadow_pipeline_layout,
                shadow_clip: &shadow_clip_layout,
                skinned: &skinned_layout,
                sky: &sky_layout,
                fog_inject: &fog_inject_layout,
                fog_integrate: &fog_integrate_layout,
                terrain_mesh: terrain_mesh_layouts.as_ref().map(|(_, p)| p),
            },
        );


        let white_capacity = 4096;
        let white_colors = white_buffer(gpu, white_capacity);

        let vsm = crate::vsm::VirtualShadows::new(gpu, &shadow_layout, caster_stride, DEPTH_FORMAT, shadow_resolution, vsm_table);
        let mut clusters = crate::cluster::Clusters::new(gpu, &layout, &texture_layout);
        // Each of these compiles the renderer's shader again: side by side,
        // each under its own error scope (see `compiled`).
        let ((made_clusters, e1), ((made_ddgi, e2), (made_restir, e3))) = scrap_core::jobs::join(
            || scoped(gpu, || clusters.make_pipelines(gpu, &shader, samples)),
            || {
                scrap_core::jobs::join(
                    || scoped(gpu, || ddgi.make_pipelines(gpu, &shader, gpu.ray_tracing)),
                    || scoped(gpu, || restir.make_pipelines(gpu, &shader, gpu.ray_tracing)),
                )
            },
        );
        // The renderer's own shader not building for its own pipelines is
        // a bug in the engine, not in a game: said as loudly as wgpu says
        // an error nobody scoped.
        if let Some(error) = built_error.or(e1).or(e2).or(e3) {
            panic!("the renderer's own pipelines do not build: {error}");
        }
        clusters.pipelines = made_clusters;
        (ddgi.trace, ddgi.update) = made_ddgi;
        (restir.initial, restir.spatial) = made_restir;

        let fog_bind_group = bind_group.clone();
        let kept_groups = (instances.clone(), bind_group.clone(), bind_group.clone());
        let mut renderer = Self {
            pipelines,
            clusters,
            ddgi,
            vsm,
            restir,
            graph: crate::graph::FrameGraph::new(),
            base_shader: SHADER.to_string(),
            material_shaders: std::collections::HashMap::new(),
            shader_textures: scrap_core::hash::FastMap::default(),
            layout,
            bind_group,
            shadow_bind_group,
            frame_buffer,
            instances,
            instance_capacity,
            white_colors,
            white_capacity,
            depth: depth_view(gpu, width, height, samples),
            depth_size: (width, height),
            samples,
            depth_prepassed: false,
            tools: None,
            occlusion_always: false,
            smoke_sim: crate::smoke_gpu::SmokeSim::new(gpu),
            other_samples: None,
            scene: scene_targets(gpu, width, height, samples),
            post: crate::post::PostRenderer::new(gpu, format),
            lens: crate::lens::LensRenderer::new(gpu),
            previous_view_projection: None,
            reflections,
            decal_buffer,
            decal_atlases,
            ssao,
            taa: crate::taa::Taa::new(gpu),
            upscaler: crate::upscale::Upscaler::new(gpu),
            picturing: false,
            metered_at: None,
            clipmap: None,
            terrain_made: None,
            ray,
            passes: crate::passes::Passes::MAX,
            quality: None,
            light_buffer,
            cell_buffer,
            index_buffer,
            index_capacity,
            light_view_buffer,
            light_shadow_map,
            light_shadow_layers,
            light_shadow_resolution,
            shadow_map,
            shadow_layers,
            casters,
            caster_stride,
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
            free_meshes: Vec::new(),
            streams: scrap_core::hash::FastMap::default(),
            batch_pool: BatchPool::default(),
            sky_light: None,
            flat: Vec::new(),
            live: std::collections::HashMap::new(),
            targets: std::collections::HashMap::new(),
            textures: Vec::new(),
            by_asset: scrap_core::hash::FastMap::default(),
            looks: scrap_core::hash::FastMap::default(),
            map_groups: scrap_core::hash::FastMap::default(),
            bindless: bindless_on.then(crate::bindless::Bindless::new),
            texture_layout,
            texture_sampler,
            pipeline_layout,
            terrain_mesh_layouts,
            terrain_mesh_buffer,
            terrain_mesh_group: None,
            shadow_pipeline_layout,
            shadow_clip_layout,
            skinned_layout,
            sky_layout,
            fog_inject_layout,
            fog_integrate_layout,
            volumes,
            fog_bind_group,
            kept_groups,
            started: web_time::Instant::now(),
            bolt: None,
            occlusion: crate::occlusion::Occlusion::new(gpu),
            gpu_particles: crate::particles_gpu::GpuParticles::new(
                gpu,
                crate::post::HDR_FORMAT,
                DEPTH_FORMAT,
                samples,
            ),
            lods: scrap_core::hash::FastMap::default(),
            lod_meshes: Vec::new(),
            timer: None,
            debugger: Default::default(),
            lean: Default::default(),
            lowres: crate::lowres::LowRes::new(gpu),
            lowres_drawn: false,
            cascade_cache: None,
            cluster_error: 1.0,
            far_off_prepass: std::env::var("SCRAP_FAR_PREPASS").map_or(true, |v| v != "1"),
            split_prepass: false,
            shadow_turn: false,
            shadow_stagger: std::env::var("SCRAP_SHADOW_STAGGER").map_or(true, |v| v != "0"),
            lean_modules: [(None, shader.clone())].into_iter().collect(),
            mesh_names: Default::default(),
            texture_names: Default::default(),
            timing: std::env::var_os("SCRAP_GPU_TIMES").is_some(),
            atmosphere,
            unity_sky_baked: None,
            clouds,
            terrain_heights,
            trample: crate::foliage::TrampleMap::default(),
            trample_texture,
            trample_view,
            distance,
            blank_depth: gpu
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("no depth"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: crate::ssao::PREPASS_DEPTH,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default()),
        };
        renderer.rebind(gpu);

        // Handle 0 is always the white pixel, so `TextureHandle::WHITE` is a
        // constant rather than something every caller has to be handed.
        renderer.upload_texture_rgba(gpu, 1, 1, &[255, 255, 255, 255], true);
        // Handle 1 the flat normal, for the same reason.
        renderer.upload_texture_rgba(gpu, 1, 1, &[128, 128, 255, 255], false);
        renderer
    }

    /// Make the frame's bind group again, after something in it was remade.
    /// While probes are being baked their pictures are left out of it:
    /// they are being drawn.
    fn rebind(&mut self, gpu: &Gpu) {
        self.bind_group = self.frame_group(gpu, false, &self.instances);
        self.fog_bind_group = self.frame_group(gpu, true, &self.instances);
        self.rebind_kept(gpu);
    }

    /// [`Renderer::kept_groups`] made again, on the kept buffer as it is.
    fn rebind_kept(&mut self, gpu: &Gpu) {
        let kept = self.occlusion.kept.buffer.clone();
        let group = self.frame_group(gpu, false, &kept);
        let fog = self.frame_group(gpu, true, &kept);
        self.kept_groups = (kept, group, fog);
    }

    /// The frame's bind group; with `making_fog`, the fog's grid and the
    /// prepass's depth left out, for the passes that fill them.
    fn frame_group(&self, gpu: &Gpu, making_fog: bool, instances: &wgpu::Buffer) -> wgpu::BindGroup {
        let baking = self.reflections.baking;
        frame_bind_group(
            gpu,
            &self.layout,
            &FrameInputs {
                ddgi: &self.ddgi.probes,
                vsm_pages: &self.vsm.table,
                restir: &self.restir.shade,
                terrain_heights: &self.terrain_heights,
                trample: &self.trample_view,
                distance: &self.distance.view,
                instances,
                history: &self.scene.history_view,
                clouds: self.clouds.view(),
                scene_depth: if making_fog {
                    &self.blank_depth
                } else {
                    &self.ssao.depth
                },
                sky_view: &self.atmosphere.sky_view,
                aerial: &self.atmosphere.aerial,
                fog: if making_fog {
                    &self.volumes.blank
                } else {
                    &self.volumes.integrated
                },
                fog_sampler: &self.volumes.sampler,
                decals: &self.decal_buffer,
                decal_colours: self.decal_atlases.colour_view(),
                decal_normals: self.decal_atlases.normal_view(),
                probes: if baking && self.reflections.bouncing {
                    &self.reflections.previous_view
                } else if baking {
                    &self.reflections.blank
                } else {
                    &self.reflections.view
                },
                probe_sampler: &self.reflections.sampler,
                frame: &self.frame_buffer,
                shadow_map: &self.shadow_map,
                shadow_sampler: &self.shadow_sampler,
                occlusion: &self.ssao.result,
                rays: self.ray.as_ref().map(|r| r.bindings()),
                lights: &self.light_buffer,
                cells: &self.cell_buffer,
                indices: &self.index_buffer,
                light_views: &self.light_view_buffer,
                light_shadow_map: &self.light_shadow_map,
            },
        )
    }

    /// Upload a mesh straight out of an imported asset.
    ///
    /// The archived vertices are already the layout the vertex buffer wants,
    /// so this is a copy, not a conversion — which is the whole reason the
    /// asset format exists.
    pub fn upload_mesh(&mut self, gpu: &Gpu, mesh: &ArchivedMeshAsset) -> MeshHandle {
        let indices: Vec<u32> = mesh.indices.iter().map(|i| i.to_native()).collect();
        let handle = self.upload(gpu, vertex_slice(mesh), mesh.colors.as_slice(), &indices);
        self.mesh_names.insert(handle.0, mesh.name.as_str().into());
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
        if let Some(look) = mesh.look.as_ref() {
            let texture = self.upload_texture(gpu, look);
            self.looks.insert(handle, texture);
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
        colors: &[[u8; 4]],
        indices: &[u32],
    ) -> MeshHandle {
        let mesh = self.gpu_mesh(gpu, vertices, colors, indices, true, false);
        let handle = self.take_slot(mesh);
        self.make_lods(gpu, handle, vertices, colors, indices);
        handle
    }

    /// A mesh the game rewrites as it goes ([`LiveMeshDraw`]): no
    /// clusters, no coarser levels — both are of a shape it will not keep
    /// — and, where rays are traced, a structure quick to build, built
    /// again in place as it changes ([`Renderer::update_mesh`]).
    fn upload_live(&mut self, gpu: &Gpu, vertices: &[crate::asset::Vertex], indices: &[u32]) -> MeshHandle {
        let mesh = self.gpu_mesh(gpu, vertices, &[], indices, true, true);
        self.take_slot(mesh)
    }

    /// A slot for `mesh`: one given back by `release_mesh` first.
    fn take_slot(&mut self, mesh: GpuMesh) -> MeshHandle {
        match self.free_meshes.pop() {
            Some(slot) => {
                self.meshes[slot as usize] = mesh;
                MeshHandle(slot)
            }
            None => {
                self.meshes.push(mesh);
                MeshHandle(self.meshes.len() as u32 - 1)
            }
        }
    }

    /// Give back a mesh's GPU memory — its buffers, its coarser levels,
    /// its rays' structure — once nothing draws it: what a streamed region
    /// does when the camera leaves it (`scrap::streaming`). The handle
    /// then draws nothing, and its slot goes to the next mesh uploaded, so
    /// nothing may still hold it.
    pub fn release_mesh(&mut self, gpu: &Gpu, handle: MeshHandle) {
        let slot = handle.0 as usize;
        if handle.0 & LOD_HANDLE != 0 || slot >= self.meshes.len() || self.free_meshes.contains(&handle.0) {
            return;
        }
        let blank = crate::asset::Vertex {
            position: [0.0; 3],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0; 2],
        };
        self.meshes[slot] = self.gpu_mesh(gpu, &[blank], &[], &[0, 0, 0], false, true);
        if let Some(levels) = self.lods.remove(&handle.0) {
            for (level, _) in levels {
                let at = (level.0 & !LOD_HANDLE) as usize;
                if at < self.lod_meshes.len() {
                    self.lod_meshes[at] = self.gpu_mesh(gpu, &[blank], &[], &[0, 0, 0], false, true);
                }
            }
        }
        self.looks.remove(&handle);
        self.free_meshes.push(handle.0);
    }

    /// A mesh's triangles as uploaded (its finest level), and its bounds'
    /// size: what a profile names a heavy draw by.
    pub fn mesh_size(&self, mesh: MeshHandle) -> Option<(u32, Vec3)> {
        let m = self.meshes.get(mesh.0 as usize)?;
        Some((m.index_count / 3, Vec3::from(m.bounds.max) - Vec3::from(m.bounds.min)))
    }

    /// A mesh's vertices as uploaded.
    pub fn mesh_vertices(&self, mesh: MeshHandle) -> Option<u32> {
        let m = self.meshes.get(mesh.0 as usize)?;
        Some((m.vertices.size() / std::mem::size_of::<crate::asset::Vertex>() as u64) as u32)
    }

    /// Meshes uploaded and not released.
    pub fn mesh_count(&self) -> usize {
        self.meshes.len() - self.free_meshes.len()
    }

    /// A mesh of enough triangles gets its coarser levels ([`crate::lod`]).
    fn make_lods(
        &mut self,
        gpu: &Gpu,
        handle: MeshHandle,
        vertices: &[crate::asset::Vertex],
        colors: &[[u8; 4]],
        indices: &[u32],
    ) {
        self.lods.remove(&handle.0);
        if indices.len() / 3 < crate::lod::FROM_TRIANGLES {
            return;
        }
        let bounds = crate::asset::Bounds::of(vertices);
        let diagonal = (Vec3::from_array(bounds.max) - Vec3::from_array(bounds.min)).length();
        let mut levels = Vec::new();
        for (share, below) in crate::lod::LEVELS {
            let Some((v, c, i)) = crate::lod::simplify(vertices, colors, indices, diagonal * share) else {
                break;
            };
            let mesh = self.gpu_mesh(gpu, &v, &c, &i, false, false);
            self.lod_meshes.push(mesh);
            levels.push((MeshHandle(LOD_HANDLE | (self.lod_meshes.len() as u32 - 1)), below));
        }
        if !levels.is_empty() {
            self.lods.insert(handle.0, levels);
        }
    }

    /// A mesh on the GPU; a structure for rays too when `traced`.
    /// `live`: rewritten as it goes — never cut into clusters, and its
    /// rays' structure made to be built again quickly.
    fn gpu_mesh(
        &mut self,
        gpu: &Gpu,
        vertices: &[crate::asset::Vertex],
        colors: &[[u8; 4]],
        indices: &[u32],
        traced: bool,
        live: bool,
    ) -> GpuMesh {
        let bounds = crate::asset::Bounds::of(vertices);
        use wgpu::util::DeviceExt;

        // Its own colours, one to a vertex; the white ones otherwise, grown
        // to reach its last vertex.
        let colors = (colors.len() == vertices.len() && !colors.is_empty()).then(|| {
            gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vertex colours"),
                contents: bytemuck::cast_slice(colors),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        if colors.is_none() && vertices.len() as u64 > self.white_capacity {
            self.white_capacity = (vertices.len() as u64).next_power_of_two();
            self.white_colors = white_buffer(gpu, self.white_capacity);
        }

        let traced = traced && self.ray.is_some();
        let mut usage = if traced {
            wgpu::BufferUsages::BLAS_INPUT
        } else {
            wgpu::BufferUsages::empty()
        };
        // Dense: cut into clusters, its triangles in their order, its
        // buffers readable by the vertex shader that pulls from them.
        let clustered = (self.clusters.can && !live)
            .then(|| crate::cluster::build(vertices, indices))
            .flatten();
        if clustered.is_some() {
            usage |= wgpu::BufferUsages::STORAGE;
        }
        // The mesh itself is the first part of a clustered one's indices;
        // its coarser levels come after, for its clusters alone.
        let own = indices.len();
        let indices = clustered.as_ref().map_or(indices, |(sorted, _)| sorted.as_slice());
        let vertex_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vertices"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | usage,
            });
        let index_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("indices"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST | usage,
            });
        let blas = (traced && !indices.is_empty()).then(|| {
            crate::ray::blas(
                gpu,
                &vertex_buffer,
                vertices.len() as u32,
                &index_buffer,
                own as u32,
                live,
            )
        });
        GpuMesh {
            vertices: vertex_buffer,
            colors,
            skin: None,
            indices: index_buffer,
            index_count: own as u32,
            bounds,
            blas,
            clusters: clustered.map(|(_, c)| crate::cluster::MeshClusters::new(gpu, &c)),
        }
    }

    /// What a mesh's vertex colours are read from: its own, or white.
    fn colors_of<'a>(&'a self, mesh: &'a GpuMesh) -> &'a wgpu::Buffer {
        mesh.colors.as_ref().unwrap_or(&self.white_colors)
    }

    /// A mesh by its handle, a coarser level's too.
    fn mesh(&self, handle: MeshHandle) -> Option<&GpuMesh> {
        if handle.0 & LOD_HANDLE != 0 {
            self.lod_meshes.get((handle.0 & !LOD_HANDLE) as usize)
        } else {
            self.meshes.get(handle.0 as usize)
        }
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
        let id = crate::asset::AssetId::from(&texture.id);
        let handle = self.upload_texture_levels(gpu, &levels, texture.srgb);
        self.by_asset.insert(id, handle);
        self.texture_names.insert(handle.0, texture.name.as_str().into());
        self.streams.insert(
            handle,
            crate::streaming_textures::TextureStream::new(id, &levels.iter().map(|l| (l.0, l.1)).collect::<Vec<_>>()),
        );
        handle
    }

    /// The handle a texture asset was uploaded as, if it was.
    pub fn texture_for(&self, id: crate::asset::AssetId) -> Option<TextureHandle> {
        self.by_asset.get(&id).copied()
    }

    /// The maps a draw takes: see [`DrawLookup::maps_of`].
    fn maps_of(&self, draw: &Draw) -> Maps {
        self.lookup().maps_of(draw)
    }

    /// What preparing a draw reads of the renderer: shareable across the
    /// jobs that prepare them.
    fn lookup(&self) -> DrawLookup<'_> {
        DrawLookup {
            meshes: &self.meshes,
            lods: &self.lods,
            looks: &self.looks,
            by_asset: &self.by_asset,
            shader_textures: &self.shader_textures,
        }
    }

    /// The maps a batch is keyed by: its own, or with bindless one set for
    /// all ([`crate::bindless`]).
    fn batch_maps(&self, maps: Maps) -> Maps {
        if self.bindless.is_some() {
            crate::bindless::KEY
        } else {
            maps
        }
    }

    /// Make the bind groups for sets of maps not seen before — or, bindless,
    /// the one group of every texture, when there are new ones.
    fn prepare_maps(&mut self, gpu: &Gpu, sets: impl IntoIterator<Item = Maps>) {
        if let Some(bindless) = &mut self.bindless {
            let views: Vec<&wgpu::TextureView> = self.textures.iter().map(|t| &t.view).collect();
            if let Some(group) = bindless.group(gpu, &self.texture_layout, &views, &self.texture_sampler) {
                self.map_groups.insert(crate::bindless::KEY, group);
            }
            return;
        }
        for maps in sets {
            if self.map_groups.contains_key(&maps) {
                continue;
            }
            let view = |handle: TextureHandle| {
                &self
                    .textures
                    .get(handle.0 as usize)
                    .or_else(|| self.textures.first())
                    .expect("the white texture is always there")
                    .view
            };
            let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("surface maps"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view(maps[0])),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.texture_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(view(maps[1])),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(view(maps[2])),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(view(maps[3])),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(view(maps[4])),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(view(maps[5])),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(view(maps[6])),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(view(maps[7])),
                    },
                ],
            });
            self.map_groups.insert(maps, group);
        }
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
        if levels.is_empty() {
            return TextureHandle::WHITE;
        }
        let view = self.texture_view(gpu, levels, srgb);
        self.textures.push(GpuTexture { view });
        TextureHandle(self.textures.len() as u32 - 1)
    }

    /// A texture of these levels, uploaded, as a view.
    fn texture_view(&self, gpu: &Gpu, levels: &[(u32, u32, &[u8])], srgb: bool) -> wgpu::TextureView {
        let (width, height) = levels.first().map_or((1, 1), |l| (l.0, l.1));
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
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Bring each texture asset's resident levels to what the last frame
    /// needed (mip streaming): a texture seen large gets its finer levels
    /// back, one seen small or not at all for a while gives them up.
    /// `texture` finds an asset's picture — the library's — and at most
    /// `most` textures are uploaded again a call. How many were.
    pub fn stream_textures<'a>(
        &mut self,
        gpu: &Gpu,
        texture: impl Fn(crate::asset::AssetId) -> Option<&'a ArchivedTextureAsset>,
        most: usize,
    ) -> usize {
        let mut changes: Vec<(TextureHandle, u32)> = self
            .streams
            .iter()
            .filter_map(|(handle, s)| {
                let want = s.wanted();
                (want != s.first).then_some((*handle, want))
            })
            .collect();
        // Finer first: what is blurry on the screen now matters most.
        changes.sort_by_key(|(h, want)| (*want as i64 - self.streams[h].first as i64, h.0));
        let mut done = 0;
        for (handle, want) in changes.into_iter().take(most) {
            let Some(stream) = self.streams.get(&handle).copied() else { continue };
            let Some(asset) = texture(stream.id) else { continue };
            let mut levels: Vec<(u32, u32, &[u8])> = vec![(asset.width.to_native(), asset.height.to_native(), asset.pixels.as_slice())];
            for mip in asset.mips.iter() {
                levels.push((mip.width.to_native(), mip.height.to_native(), mip.pixels.as_slice()));
            }
            let first = (want as usize).min(levels.len().saturating_sub(1));
            let view = self.texture_view(gpu, &levels[first..], asset.srgb);
            self.textures[handle.0 as usize] = GpuTexture { view };
            if let Some(s) = self.streams.get_mut(&handle) {
                s.first = first as u32;
            }
            done += 1;
        }
        if done > 0 {
            // Groups made with the old pictures point at nothing.
            self.map_groups.clear();
            if let Some(bindless) = &mut self.bindless {
                bindless.invalidate();
            }
        }
        done
    }

    /// Texture memory the streamed textures hold now, and would at every
    /// level, bytes.
    pub fn texture_residency(&self) -> (u64, u64) {
        self.streams.values().fold((0, 0), |(now, full), s| (now + s.bytes_from(s.first), full + s.bytes_from(0)))
    }

    /// Upload a mesh that is already in memory rather than in an asset.
    ///
    /// The builtins come this way. Everything else should go through the
    /// asset pipeline, which is why this is separate rather than the only
    /// entry point: an import is a decision, and making it as easy to skip
    /// as to do is how a codebase ends up parsing OBJ at startup again.
    pub fn upload_mesh_owned(&mut self, gpu: &Gpu, mesh: &crate::asset::MeshAsset) -> MeshHandle {
        let handle = self.upload(gpu, &mesh.vertices, &mesh.colors, &mesh.indices);
        if !mesh.name.is_empty() {
            self.mesh_names.insert(handle.0, mesh.name.as_str().into());
        }
        handle
    }

    /// Issue the grouped draws. Shared by both passes so that what casts a
    /// shadow and what is drawn can never drift apart — the commonest way a
    /// shadow ends up belonging to nothing.
    fn draw_batches<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        batches: &[(BatchKey, Vec<InstanceRaw>)],
        base: u32,
        textured: bool,
    ) {
        self.draw_batches_with(pass, batches, base, textured, false, false);
    }

    /// [`Renderer::draw_batches`], in the depth-and-normals prepass when
    /// `prepass`.
    fn draw_batches_with<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        batches: &[(BatchKey, Vec<InstanceRaw>)],
        base: u32,
        textured: bool,
        prepass: bool,
        culled: bool,
    ) {
        // The colour pass's batches, culled on the GPU: each drawn by its
        // arguments, from the instances kept.
        let occluded = culled && self.occlusion.active;
        let mut first = base;
        let mut current: Option<Look> = None;
        for (k, ((look, handle, texture), list)) in batches.iter().enumerate() {
            let count = list.len() as u32;
            let Some(mesh) = self.mesh(*handle) else {
                first += count;
                continue;
            };
            // Culled a cluster at a time: drawn by what was kept.
            if let (true, Some(look), Some(pipelines)) = (culled, look, &self.clusters.pipelines) {
                let pipeline = if prepass {
                    pipelines.prepass.get(&look.face)
                } else {
                    self.lean
                        .ready
                        .get(&LeanKey::Cluster(look.face, look.water))
                        .filter(|_| self.lean.on)
                        .or_else(|| pipelines.scene.get(&(look.face, look.water)))
                };
                if let (Some(pipeline), true) = (pipeline, self.clusters.this_frame.contains_key(&k)) {
                    if !crate::frame_debugger::draw(|| self.describe(*handle, Some(look), texture, count, true, prepass, textured)) {
                        first += count;
                        continue;
                    }
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, self.frame_group_for(prepass), &[]);
                    if textured {
                        self.bind_maps(pass, *texture);
                    }
                    self.clusters.draw(pass, k, prepass);
                    current = None;
                    first += count;
                    continue;
                }
            }
            if prepass && look.is_some_and(|l| self.cuts(l)) {
                first += count;
                continue;
            }
            if !crate::frame_debugger::draw(|| self.describe(*handle, look.as_ref(), texture, count, occluded, prepass, textured)) {
                first += count;
                continue;
            }
            if let Some(look) = look {
                if current != Some(*look) {
                    let pipeline = if prepass {
                        self.pipelines
                            .prepass
                            .get(&(look.skinned, look.face, look.terrain))
                    } else {
                        self.scene_pipeline(*look)
                    };
                    if let Some(pipeline) = pipeline {
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(
                            0,
                            if occluded { self.kept_group_for(prepass) } else { self.frame_group_for(prepass) },
                            &[],
                        );
                    }
                    current = Some(*look);
                }
            }
            if textured {
                self.bind_maps(pass, *texture);
            }
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_vertex_buffer(2, self.colors_of(mesh).slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            if occluded {
                pass.set_vertex_buffer(1, self.occlusion.kept.buffer.slice(..));
                pass.draw_indexed_indirect(&self.occlusion.args.buffer, k as u64 * 20);
            } else {
                pass.set_vertex_buffer(1, self.instances.slice(..));
                pass.draw_indexed(0..mesh.index_count, 0, first..first + count);
            }
            first += count;
        }
    }

    /// Shadow casters into one cascade: of each batch, the runs of its
    /// instances whose bit is set in `masks` (indexed as the instances
    /// are, from 0 at the first caster), each run one call.
    #[allow(clippy::too_many_arguments)]
    fn draw_casters<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        batches: &[(BatchKey, Vec<InstanceRaw>)],
        base: u32,
        textured: bool,
        masks: &[u8],
        bit: u8,
        cascade: Option<(Mat4, f32)>,
    ) {
        let mut first = base;
        for ((look, handle, texture), list) in batches {
            let count = list.len() as u32;
            let Some(mesh) = self.mesh(*handle) else {
                first += count;
                continue;
            };
            let mut bound = false;
            let mut at = first;
            let end = first + count;
            while at < end {
                if masks.get(at as usize).is_some_and(|m| m & bit == 0) {
                    at += 1;
                    continue;
                }
                let start = at;
                while at < end && masks.get(at as usize).is_none_or(|m| m & bit != 0) {
                    at += 1;
                }
                // A dense mesh a cluster at a time: of a mountain range a
                // kilometre round, what is over this cascade's square.
                if let (Some(clusters), Some((cascade, texel))) = (mesh.clusters.as_ref(), cascade) {
                    for instance in start..at {
                        let Some(model) = list.get((instance - first) as usize).map(|r| Mat4::from_cols_array_2d(&r.model)) else {
                            continue;
                        };
                        let mut runs = Vec::new();
                        clusters.runs_in(model, cascade, texel, &mut runs);
                        if runs.is_empty() {
                            continue;
                        }
                        let triangles: u32 = runs.iter().map(|r| (r.1 - r.0) / 3).sum();
                        if !crate::frame_debugger::draw(|| {
                            let mut d = self.describe(*handle, look.as_ref(), texture, 1, false, false, textured);
                            d.what += &format!(" ({} of {} clusters' runs)", runs.len(), clusters.count);
                            d.triangles = triangles as u64;
                            d
                        }) {
                            continue;
                        }
                        if textured {
                            self.bind_maps(pass, *texture);
                        }
                        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                        pass.set_vertex_buffer(1, self.instances.slice(..));
                        pass.set_vertex_buffer(2, self.colors_of(mesh).slice(..));
                        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                        for (a, b) in runs {
                            pass.draw_indexed(a..b, 0, instance..instance + 1);
                        }
                    }
                    continue;
                }
                if !crate::frame_debugger::draw(|| self.describe(*handle, look.as_ref(), texture, at - start, false, false, textured)) {
                    continue;
                }
                if !bound {
                    if textured {
                        self.bind_maps(pass, *texture);
                    }
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_vertex_buffer(1, self.instances.slice(..));
                    pass.set_vertex_buffer(2, self.colors_of(mesh).slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    bound = true;
                }
                pass.draw_indexed(0..mesh.index_count, 0, start..at);
            }
            first = end;
        }
    }

    /// A draw as the Frame Debugger lists it: the mesh by name, what it
    /// is drawn with. Made only while a frame is recorded.
    #[allow(clippy::too_many_arguments)]
    fn describe(
        &self,
        handle: MeshHandle,
        look: Option<&Look>,
        maps: &Maps,
        instances: u32,
        culled_on_gpu: bool,
        prepass: bool,
        textured: bool,
    ) -> crate::frame_debugger::DrawCall {
        let triangles = self.mesh(handle).map_or(0, |m| m.index_count as u64 / 3);
        // A level of detail is called by the mesh it is a level of.
        let (source, level) = if handle.0 & LOD_HANDLE != 0 {
            self.lods
                .iter()
                .find_map(|(m, levels)| levels.iter().position(|(l, _)| *l == handle).map(|i| (*m, Some(i + 1))))
                .unwrap_or((handle.0, None))
        } else {
            (handle.0, None)
        };
        let mut what = self
            .mesh_names
            .get(&source)
            .map_or_else(|| format!("mesh {source}"), |n| n.to_string());
        if let Some(level) = level {
            what += &format!(" (LOD{level})");
        }
        let pipeline = match (look, prepass, textured) {
            (_, false, false) => "shadow caster".to_string(),
            (Some(l), true, _) => format!("prepass, {:?} faces{}", l.face, if l.skinned { ", skinned" } else { "" }),
            (Some(l), false, _) => {
                let mut p = match l.blend {
                    None => "opaque".to_string(),
                    Some(b) => format!("{b:?} blend"),
                };
                p += &format!(", {:?} faces", l.face);
                if let Some(shader) = l.shader {
                    p += &format!(", shader {shader:?}");
                }
                for (on, name) in [(l.skinned, "skinned"), (l.water, "water"), (l.terrain, "terrain"), (l.unlit, "unlit"), (l.on_top, "on top")] {
                    if on {
                        p += &format!(", {name}");
                    }
                }
                p
            }
            (None, _, _) => "default".to_string(),
        };
        let textures = if textured {
            maps.iter()
                .filter(|t| **t != TextureHandle::WHITE)
                .map(|t| self.texture_names.get(&t.0).map_or_else(|| format!("texture {}", t.0), |n| n.to_string()))
                .collect()
        } else {
            Vec::new()
        };
        crate::frame_debugger::DrawCall { what, triangles, instances, culled_on_gpu, pipeline, textures }
    }

    /// Handles and outlines over the finished picture ([`crate::tools`]):
    /// the outline mask, the overlay, and the two laid over `view`.
    #[allow(clippy::too_many_arguments)]
    fn draw_tools(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: (u32, u32),
        overlay: &Batches,
        outlines: &Batches,
        colors: &[[f32; 3]],
        overlay_base: u32,
        outline_base: u32,
        width: f32,
        view_projection: Mat4,
    ) {
        // Only where something is: the composite reads a few pixels round
        // each one it writes, and a selection is a small part of the view.
        let reach = width.ceil() + 2.0;
        let Some(area) = union(
            self.screen_box(overlay, view_projection, size, 2.0),
            self.screen_box(outlines, view_projection, size, reach),
        ) else {
            return;
        };
        let samples = self.pipelines.overlay_samples;
        if !self
            .tools
            .as_ref()
            .is_some_and(|t| t.size == size && t.samples == samples)
        {
            self.tools = Some(crate::tools::Tools::new(gpu, self.format, samples, size));
        }
        let Some(tools) = self.tools.as_ref() else {
            return;
        };
        tools.write(gpu, if outlines.is_empty() { &[] } else { colors }, width);
        let clear = wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        };
        if !outlines.is_empty() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scrap::outline mask"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &tools.mask,
                    depth_slice: None,
                    resolve_target: None,
                    ops: clear,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &tools.mask_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: crate::gpu_timer::render("outline"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipelines.outline_mask);
            pass.set_bind_group(0, &self.bind_group, &[]);
            self.draw_batches(&mut pass, outlines, outline_base, true);
        }
        {
            let (target, resolve) = match &tools.multisampled {
                Some(many) => (many, Some(&tools.color)),
                None => (&tools.color, None),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scrap::overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: crate::gpu_timer::render("overlay"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !overlay.is_empty() {
                pass.set_pipeline(&self.pipelines.overlay);
                pass.set_bind_group(0, &self.bind_group, &[]);
                self.draw_batches(&mut pass, overlay, overlay_base, true);
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scrap::tools"),
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
            timestamp_writes: crate::gpu_timer::render("tools"),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_scissor_rect(area[0], area[1], area[2] - area[0], area[3] - area[1]);
        pass.set_pipeline(&tools.pipeline);
        pass.set_bind_group(0, &tools.group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The pixels batches cover on a `size` picture, `margin` more each
    /// way, as (left, top, right, bottom); `None` when they cover none. From
    /// their meshes' boxes, so a little more than they cover — and the whole
    /// picture when one reaches behind the eye.
    fn screen_box(
        &self,
        batches: &Batches,
        view_projection: Mat4,
        size: (u32, u32),
        margin: f32,
    ) -> Option<[u32; 4]> {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let mut low = glam::Vec2::splat(f32::MAX);
        let mut high = glam::Vec2::splat(f32::MIN);
        for ((_, handle, _), list) in batches {
            let Some(mesh) = self.mesh(*handle) else {
                continue;
            };
            let (a, b) = (Vec3::from(mesh.bounds.min), Vec3::from(mesh.bounds.max));
            for raw in list {
                let clip = view_projection * Mat4::from_cols_array_2d(&raw.model);
                for corner in 0..8u32 {
                    let pick = |axis: usize| if corner & (1 << axis) == 0 { a[axis] } else { b[axis] };
                    let p = clip * glam::Vec4::new(pick(0), pick(1), pick(2), 1.0);
                    if p.w <= 1e-4 {
                        return Some([0, 0, size.0, size.1]);
                    }
                    let at = glam::Vec2::new(
                        (p.x / p.w * 0.5 + 0.5) * w,
                        (0.5 - p.y / p.w * 0.5) * h,
                    );
                    low = low.min(at);
                    high = high.max(at);
                }
            }
        }
        if low.x > high.x {
            return None;
        }
        let low = (low - margin).max(glam::Vec2::ZERO).min(glam::Vec2::new(w, h));
        let high = (high + margin).max(glam::Vec2::ZERO).min(glam::Vec2::new(w, h));
        let area = [
            low.x.floor() as u32,
            low.y.floor() as u32,
            high.x.ceil() as u32,
            high.y.ceil() as u32,
        ];
        (area[2] > area[0] && area[3] > area[1]).then_some(area)
    }

    /// Whether terrain is drawn by mesh shaders here: asked for, the device
    /// has them, and they built.
    pub fn terrain_by_mesh_shaders(&self) -> bool {
        self.pipelines.terrain_mesh.is_some()
    }

    /// The terrain's fine grid by mesh shaders: every patch of every ring
    /// to the task shader, which keeps what is worth drawing.
    fn draw_mesh_terrain<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        maps: Option<Maps>,
        prepass: bool,
    ) {
        let (Some(maps), Some((drawn, depth)), Some(group)) = (
            maps,
            self.pipelines.terrain_mesh.as_ref(),
            self.terrain_mesh_group.as_ref(),
        ) else {
            return;
        };
        if !crate::frame_debugger::draw(|| crate::frame_debugger::DrawCall {
            what: "terrain (mesh shader)".into(),
            instances: 1,
            culled_on_gpu: true,
            pipeline: if prepass { "terrain prepass" } else { "terrain" }.into(),
            ..Default::default()
        }) {
            return;
        }
        pass.set_pipeline(if prepass { depth } else { drawn });
        pass.set_bind_group(0, self.frame_group_for(prepass), &[]);
        self.bind_maps(pass, maps);
        pass.set_bind_group(3, group, &[]);
        let patches = crate::terrain::CLIPMAP_LEVELS * crate::terrain::PATCHES_PER_RING;
        // Sixty-four task invocations a group, eight patches each (see
        // terrain_mesh.wgsl): a task group costs the same kept or not.
        pass.draw_mesh_tasks(patches.div_ceil(64 * 8), 1, 1);
    }

    /// The frame's bind group — or, for the prepass, the one without the
    /// prepass's own depth in it, which the prepass is drawing.
    fn frame_group_for(&self, prepass: bool) -> &wgpu::BindGroup {
        if prepass {
            &self.fog_bind_group
        } else {
            &self.bind_group
        }
    }

    /// [`Renderer::frame_group_for`] for draws of the occlusion culling's
    /// kept instances.
    fn kept_group_for(&self, prepass: bool) -> &wgpu::BindGroup {
        if prepass {
            &self.kept_groups.2
        } else {
            &self.kept_groups.1
        }
    }

    /// Bind a surface's maps, made beforehand by [`Renderer::prepare_maps`].
    fn bind_maps<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>, maps: Maps) {
        if let Some(group) = self.map_groups.get(&maps) {
            pass.set_bind_group(1, group, &[]);
        }
    }

    /// One draw on its own: a skinned one (its own pose) or a transparent
    /// one (its own place in the back-to-front order).
    #[allow(clippy::too_many_arguments)]
    fn draw_single<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        look: Look,
        mesh: MeshHandle,
        texture: Maps,
        pose: u32,
        instance: u32,
        prepass: bool,
    ) {
        self.draw_run(pass, look, mesh, texture, pose, instance, 1, prepass);
    }

    /// [`Self::draw_single`] of `count` instances from `instance` on, one
    /// call: a run of the same see-through thing.
    #[allow(clippy::too_many_arguments)]
    fn draw_run<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        look: Look,
        mesh: MeshHandle,
        texture: Maps,
        pose: u32,
        instance: u32,
        count: u32,
        prepass: bool,
    ) {
        let handle = mesh;
        let Some(mesh) = self.mesh(mesh) else {
            return;
        };
        if prepass && self.cuts(look) {
            return;
        }
        let pipeline = if prepass {
            self.pipelines
                .prepass
                .get(&(look.skinned, look.face, look.terrain))
        } else {
            self.scene_pipeline(look)
        };
        let Some(pipeline) = pipeline else {
            return;
        };
        if !crate::frame_debugger::draw(|| self.describe(handle, Some(&look), &texture, count, false, prepass, true)) {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, self.frame_group_for(prepass), &[]);
        self.bind_maps(pass, texture);
        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
        pass.set_vertex_buffer(1, self.instances.slice(..));
        pass.set_vertex_buffer(2, self.colors_of(mesh).slice(..));
        if look.skinned {
            let Some(skin) = mesh.skin.as_ref() else {
                return;
            };
            pass.set_bind_group(2, &self.pose_bind_group, &[pose * self.pose_stride as u32]);
            pass.set_vertex_buffer(3, skin.slice(..));
        }
        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.index_count, 0, instance..instance + count);
    }

    /// The world-space box around everything being drawn.
    fn scene_bounds(&self, frame: &Frame) -> Option<(Vec3, Vec3)> {
        // Each draw's world box on every core, then folded.
        let meshes = &self.meshes;
        let boxes = scrap_core::jobs::map(&frame.draws, 1024, |draw| {
            let mesh = meshes.get(draw.mesh.0 as usize)?;
            // All eight corners (world_box), because a rotation turns the box
            // and taking only two of them would clip whatever swung outside.
            Some(world_box(mesh.bounds, draw.transform))
        });
        boxes
            .into_iter()
            .flatten()
            .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
    }

    /// The sphere one cascade covers: the slice of what the camera sees
    /// between `near` and `far` metres.
    ///
    /// A sphere rather than a box, and centred on the view rather than on
    /// the world, for one reason: the size of what a map covers must not
    /// change as the camera turns. A box fitted to the frustum's corners
    /// grows and shrinks with every rotation, and the shadows crawl as its
    /// texels resize under them.
    fn slice_sphere(&self, frame: &Frame, aspect: f32, near: f32, far: f32) -> Option<(Vec3, f32)> {
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
        let far = far.min(camera.far).max(camera.near);
        let near = near.clamp(camera.near, far);
        let forward = (camera.target - camera.position).normalize_or_zero();
        if forward.length_squared() < 0.5 {
            return None;
        }
        // The smallest sphere round the slice, as Unity's cascades have it
        // (its conservative enclosing sphere): on the view's axis, as far
        // along as puts the near and far corners at one distance from it —
        // or at the far end's middle, when the far corners alone are wider.
        let tan = (camera.fov_y_degrees.to_radians() * 0.5).tan();
        let spread = tan * (1.0 + aspect * aspect).sqrt();
        let along = ((1.0 + spread * spread) * (far + near) * 0.5).clamp(near, far);
        let radius = ((far - along).powi(2) + (far * spread).powi(2)).sqrt();
        let centre = camera.position + forward * along;
        // Rounded up, so a radius that wobbles by a hair as the camera
        // turns does not resize every texel under the shadows.
        Some((centre, (radius * 16.0).ceil().max(0.16) / 16.0))
    }

    /// Every cascade: its matrix, the sphere it covers, and how much world
    /// one of its texels is.
    fn cascades(&self, frame: &Frame, sun: Vec3, aspect: f32) -> Vec<(Mat4, Vec3, f32, f32, f32)> {
        if sun.length_squared() < 0.5 {
            return Vec::new();
        }
        let ends = if frame.camera.ortho.is_some() {
            vec![frame.shadows.max_distance]
        } else {
            frame.shadows.cascade_ends()
        };
        // A sun straight overhead makes the usual up vector degenerate.
        let up = if sun.dot(Vec3::Y).abs() > 0.99 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let turn = Mat4::look_at_rh(Vec3::ZERO, sun, up);
        let extent = self
            .scene_bounds(frame)
            .map(|(min, max)| (max - min).length());
        let mut out = Vec::new();
        let mut near = frame.camera.near;
        for far in ends {
            let Some((centre, radius)) = self.slice_sphere(frame, aspect, near, far) else {
                break;
            };
            near = far;
            let texel = 2.0 * radius / self.shadow_resolution.max(1) as f32;
            // The centre moved only in whole texels, seen from the sun: a
            // map that slides by a fraction of a texel as the camera walks
            // makes every shadow edge shimmer.
            let mut seen = turn.transform_point3(centre);
            seen.x = (seen.x / texel).floor() * texel;
            seen.y = (seen.y / texel).floor() * texel;
            let centre = turn.inverse().transform_point3(seen);
            // Pulled back far enough that casters behind the view still land
            // in the map: something off screen is often the thing casting
            // the shadow you are looking at. The scene's own extent is how
            // far back that has to be.
            let behind = extent.map_or(radius * 2.0, |e| e.max(radius * 2.0));
            let eye = centre - sun * behind;
            let view = Mat4::look_at_rh(eye, centre, up);
            let depth = behind + radius * 2.0;
            let projection = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.01, depth);
            out.push((projection * view, centre, radius, texel, depth));
        }
        out
    }

    /// How much world a single texel of the finest cascade covers.
    pub fn shadow_texel_size(&self, frame: &Frame, aspect: f32) -> f32 {
        let sun = frame.lighting.sun_direction.normalize_or_zero();
        self.cascades(frame, sun, aspect)
            .first()
            .map_or(0.0, |(_, _, _, texel, _)| *texel)
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

    /// Cull what last frame's depth says is hidden, on the GPU, or not
    /// ([`crate::occlusion`]). Left alone, a frame is culled when it has
    /// enough to cull — thousands of things, or dense meshes; asked for,
    /// always.
    pub fn set_occlusion_culling(&mut self, on: bool) {
        self.occlusion_always = on;
        self.occlusion.enabled = on && gpu_has_first_instance(&self.occlusion);
        if !on {
            self.occlusion.forget();
        }
    }

    /// How many of the colour pass's instances the GPU's occlusion culling
    /// kept last frame — `None` when it did not cull. Waits on the GPU: for
    /// tests and tools.
    pub fn culled_kept(&self, gpu: &Gpu) -> Option<u32> {
        self.occlusion.kept_count(gpu)
    }

    /// Slots held for particles on the GPU, over all their emitters.
    pub fn gpu_particle_slots(&self) -> u32 {
        self.gpu_particles.slots()
    }

    /// Draw frames that may be by the lean lit shaders ([`crate::lean`]),
    /// or always by the standard ones: on by default (`SCRAP_LEAN=0` starts
    /// it off).
    pub fn set_lean_shaders(&mut self, on: bool) {
        self.lean.enabled = on;
    }

    /// How far, in pixels, a dense mesh's surface may stand from the mesh's
    /// as it is drawn: its clusters are drawn at the coarsest level
    /// ([`crate::cluster_lod`]) whose error is under it. One (the default)
    /// is no difference an eye can see; 0 draws every mesh whole.
    pub fn set_cluster_error(&mut self, pixels: f32) {
        self.cluster_error = pixels.max(0.0);
    }

    /// Leave clusters past the ambient occlusion's reach out of the depth
    /// prepass where nothing else reads it there (on by default;
    /// `SCRAP_FAR_PREPASS=1` starts it off).
    pub fn set_far_off_prepass(&mut self, on: bool) {
        self.far_off_prepass = on;
    }

    /// Draw the far shadow cascades every other frame, in turn (on by
    /// default; `SCRAP_SHADOW_STAGGER=0` starts it off), or every frame.
    pub fn set_shadow_stagger(&mut self, on: bool) {
        self.shadow_stagger = on;
        self.cascade_cache = None;
    }

    /// Draw unlit see-through things — smoke, dust, glows — at half size
    /// and lay them over the picture ([`crate::lowres`]) where the frame
    /// allows, or always at full size: on by default
    /// (`SCRAP_HALF_PARTICLES=0` starts it off).
    pub fn set_half_size_particles(&mut self, on: bool) {
        self.lowres.enabled = on;
    }

    /// Whether the last screen frame drew its unlit see-through things at
    /// half size. For tests and tools.
    pub fn halved_particles(&self) -> bool {
        self.lowres_drawn
    }

    /// How many of the last frame's lit pipelines were lean ones in: 0 when
    /// it was not drawn lean, or none was built yet. For tests and tools.
    pub fn lean_pipelines(&self) -> usize {
        if self.lean.on {
            self.lean.ready.len()
        } else {
            0
        }
    }

    /// Record the screen's frames for the Frame Debugger
    /// ([`crate::frame_debugger`]) — every frame from now until
    /// [`Self::stop_debugging`], each stopped at event `stop` (none: drawn
    /// whole). [`Self::frame_capture`] is the last one recorded.
    pub fn debug_frame(&mut self, stop: Option<usize>) {
        self.debugger.wanted = Some(stop);
    }

    /// No more frames recorded, and the last one forgotten.
    pub fn stop_debugging(&mut self) {
        self.debugger.wanted = None;
        self.debugger.captured = None;
    }

    /// The last frame recorded for the Frame Debugger.
    pub fn frame_capture(&self) -> Option<&crate::frame_debugger::FrameCapture> {
        self.debugger.captured.as_ref()
    }

    /// Whether the last frame recorded was stopped where there is a
    /// picture to show — else the frame itself is the one to look at.
    pub fn has_debug_picture(&self) -> bool {
        self.debugger.has_picture()
    }

    /// The Frame Debugger's picture drawn over `view` (a target of this
    /// renderer's format), fitted inside `area`: x, y, width, height in
    /// pixels. Nothing when there is no picture.
    pub fn show_debug_picture(&mut self, gpu: &Gpu, view: &wgpu::TextureView, area: (f32, f32, f32, f32)) {
        self.debugger.show(gpu, view, self.format, area);
    }

    /// The Frame Debugger's picture as RGBA pixels (sRGB) and its size —
    /// waits on the GPU: for tests and tools.
    pub fn read_debug_picture(&self, gpu: &Gpu) -> Option<(Vec<u8>, (u32, u32))> {
        self.debugger.read_picture(gpu)
    }

    /// Time each pass of the screen's frame on the GPU, or stop: see
    /// [`Self::gpu_times`]. `SCRAP_GPU_TIMES=1` starts it on.
    pub fn profile_gpu(&mut self, on: bool) {
        self.timing = on;
    }

    /// Forget the passes' times so far: a pass that no longer runs leaves
    /// the list, and the averages start again (an A/B test's next side).
    pub fn reset_gpu_times(&mut self) {
        if let Some(timer) = self.timer.as_mut() {
            timer.reset();
        }
    }

    /// Whether the screen's frame is being timed: [`Self::profile_gpu`],
    /// or `SCRAP_GPU_TIMES` at the start.
    pub fn profiling_gpu(&self) -> bool {
        self.timing
    }

    /// Each pass's time on the GPU, milliseconds, averaged over the last
    /// frames it was timed, in the order the passes run — nothing when
    /// not timing or on a device without timestamps. A frame or two
    /// behind: the frame does not wait for them.
    pub fn gpu_times(&self) -> Vec<(String, f32)> {
        self.timer.as_ref().map(|t| t.times()).unwrap_or_default()
    }

    /// The last screen frame's passes as a graph: which ran, what each
    /// read and wrote ([`crate::graph`]).
    pub fn frame_graph(&self) -> &crate::graph::FrameGraph {
        &self.graph
    }

    /// The sun's virtual shadow maps' pages: how many are kept drawn, and
    /// how many the last frame drew.
    pub fn virtual_shadow_pages(&self) -> (usize, usize) {
        self.vsm.stats()
    }

    /// Cull dense meshes a cluster at a time, where the device can
    /// ([`crate::cluster`]): on by default.
    pub fn set_cluster_culling(&mut self, on: bool) {
        self.clusters.enabled = on && self.clusters.can;
    }

    /// How many clusters the last frame kept of those it culled, read back
    /// — waiting on the GPU, so for tests and tools; `None` when nothing
    /// was culled by clusters.
    pub fn clusters_kept(&self, gpu: &Gpu) -> Option<u32> {
        self.clusters.kept(gpu)
    }

    /// What made the last frame up to the screen's size, and the share of
    /// its width it was drawn at ([`crate::upscale`]); nothing when it was
    /// drawn at the screen's own size.
    pub fn upscaled(&self) -> Option<(crate::upscale::Used, f32)> {
        self.upscaler.used.map(|u| (u, self.upscaler.scale))
    }

    /// The frame as a stroke of lightning lights it, when one is coming
    /// down now; and the bolt for the sky to draw.
    fn lightning(&mut self, frame: &Frame) -> Option<Frame> {
        self.bolt = None;
        if frame.weather.lightning <= 0.0 {
            return None;
        }
        let time = frame
            .time
            .unwrap_or_else(|| self.started.elapsed().as_secs_f32());
        let bolt = frame.weather.bolt(time)?;
        let mut lit = frame.shallow();
        let l = &mut lit.lighting;
        let strength = bolt.flash * 7.0;
        if strength > l.sun_intensity {
            let toward = (frame.camera.position * Vec3::new(1.0, 0.0, 1.0) - bolt.from).normalize_or(Vec3::NEG_Y);
            l.sun_direction = toward;
            l.sun_color = Vec3::new(0.78, 0.84, 1.0);
            l.sun_intensity = strength;
        }
        let glow = Vec3::new(0.4, 0.45, 0.6) * bolt.flash;
        l.sky_color += glow * 1.2;
        l.ground_color += glow * 0.3;
        self.bolt = Some(bolt);
        Some(lit)
    }

    /// Take the reflection probes' pictures again on the next frame — after
    /// what is around them changed.
    pub fn rebake_reflections(&mut self) {
        self.reflections.baked.clear();
    }

    pub(crate) fn render_into(
        &mut self,
        gpu: &Gpu,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        frame: &Frame,
    ) {
        for picture in &frame.texture_views {
            self.render_picture(gpu, picture, (width, height));
        }
        // A stroke of lightning coming down: for its moment the light of
        // the scene is its light — from where it struck, bright and blue,
        // with hard shadows — and the whole sky flares.
        let mut owned = self.lightning(frame);
        if !frame.live_meshes.is_empty() {
            let drawn = owned.get_or_insert_with(|| frame.shallow());
            for live in &frame.live_meshes {
                // Nothing to draw yet — water not poured, a mesh not made:
                // an empty buffer is no buffer to the device.
                if live.vertices.is_empty() || live.indices.is_empty() {
                    continue;
                }
                let mesh = match self.live.get(&live.key) {
                    Some(&(mesh, version)) if version == live.version => mesh,
                    Some(&(mesh, _)) => {
                        self.update_mesh(gpu, mesh, &live.vertices, &live.indices);
                        mesh
                    }
                    None => self.upload_live(gpu, &live.vertices, &live.indices),
                };
                self.live.insert(live.key, (mesh, live.version));
                drawn.draws.push(Draw {
                    mesh,
                    transform: live.transform,
                    texture: TextureHandle::WHITE,
                    material: live.material,
                    pose: None,
                });
            }
        }
        let frame = owned.as_ref().unwrap_or(frame);
        self.bake_unity_sky(gpu, frame);
        self.bake_probes(gpu, frame);
        self.render_view(gpu, Some(view), width, height, frame, None);
    }

    /// Which passes run: the game's graphics settings. [`Passes::MAX`]
    /// (the default) runs whatever each frame asks for.
    ///
    /// [`Passes::MAX`]: crate::passes::Passes::MAX
    pub fn set_passes(&mut self, passes: crate::passes::Passes) {
        self.passes = passes;
    }

    pub fn passes(&self) -> crate::passes::Passes {
        self.passes
    }

    /// A graphics menu's preset ([`crate::quality`]), over the passes
    /// switched; `None` (the default) draws each frame as it asks.
    pub fn set_quality(&mut self, quality: Option<crate::quality::Quality>) {
        self.quality = quality;
    }

    pub fn quality(&self) -> Option<crate::quality::Quality> {
        self.quality
    }

    /// Draw a frame's world screens into their pictures with the UI
    /// module's renderer: call before the frame is rendered, so the things
    /// showing them show this frame's.
    pub fn draw_ui_pictures(
        &mut self,
        gpu: &Gpu,
        ui: &mut crate::ui_render::UiRenderer,
        frame: &Frame,
    ) {
        for picture in &frame.ui_pictures {
            let view = self.picture_target(gpu, picture.id, picture.size);
            self.clear_picture(gpu, &view, picture.background);
            ui.render_into(gpu, &view, picture.size.0, picture.size.1, &picture.ui);
        }
    }

    /// The texture a picture of `id` is drawn into, `size` pixels, made
    /// (or made again at a new size) and registered for materials to show.
    pub fn picture_target(
        &mut self,
        gpu: &Gpu,
        id: crate::asset::AssetId,
        size: (u32, u32),
    ) -> wgpu::TextureView {
        let stale = self.targets.get(&id).is_none_or(|(_, at)| *at != size);
        if stale {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("camera picture"),
                size: wgpu::Extent3d {
                    width: size.0.max(1),
                    height: size.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            match self.by_asset.get(&id) {
                Some(handle) => self.textures[handle.0 as usize] = GpuTexture { view },
                None => {
                    self.textures.push(GpuTexture { view });
                    let handle = TextureHandle(self.textures.len() as u32 - 1);
                    self.by_asset.insert(id, handle);
                }
            }
            // Bind groups made with the old picture point at nothing.
            self.map_groups.clear();
            if let Some(bindless) = &mut self.bindless {
                bindless.invalidate();
            }
            self.targets.insert(id, (texture, size));
        }
        self.targets[&id]
            .0
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Fill a picture with one colour: a world screen's background.
    pub fn clear_picture(&self, gpu: &Gpu, view: &wgpu::TextureView, color: glam::Vec4) {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("clear picture"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear picture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: color.x as f64,
                            g: color.y as f64,
                            b: color.z as f64,
                            a: color.w as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        gpu.queue.submit(Some(encoder.finish()));
    }

    /// A camera's picture, drawn into its texture before the frame that
    /// shows it. What would show the picture in itself is left out: a
    /// texture cannot be drawn into and read in one pass.
    fn render_picture(&mut self, gpu: &Gpu, picture: &TextureView, size: (u32, u32)) {
        self.picture_target(gpu, picture.id, size);
        let mut frame = picture.frame.shallow();
        let shows = |m: &Material| m.maps().any(|id| id == picture.id);
        frame.draws.retain(|d| !shows(&d.material));
        let view = self.targets[&picture.id]
            .0
            .create_view(&wgpu::TextureViewDescriptor::default());
        // Motion blur's history is the screen camera's, not this one's.
        let history = self.previous_view_projection;
        self.picturing = true;
        self.render_view(gpu, Some(&view), size.0, size.1, &frame, None);
        self.picturing = false;
        self.previous_view_projection = history;
    }

    /// Put new vertices and indices into a mesh already uploaded: the same
    /// buffers when the counts are the same, new ones in the same place
    /// when not. What a surface that moves every frame is drawn with.
    pub fn update_mesh(
        &mut self,
        gpu: &Gpu,
        mesh: MeshHandle,
        vertices: &[crate::asset::Vertex],
        indices: &[u32],
    ) {
        let Some(old) = self.meshes.get(mesh.0 as usize) else {
            return;
        };
        // What moves every frame is drawn as it is: its old levels are stale.
        self.lods.remove(&mesh.0);
        // The same sizes, and triangles in the order given (not a dense
        // mesh's, sorted into clusters): written over in place, and its
        // rays' structure built again from them.
        let same = old.clusters.is_none()
            && old.vertices.size() == std::mem::size_of_val(vertices) as u64
            && old.indices.size() == std::mem::size_of_val(indices) as u64;
        if same {
            gpu.queue
                .write_buffer(&old.vertices, 0, bytemuck::cast_slice(vertices));
            gpu.queue
                .write_buffer(&old.indices, 0, bytemuck::cast_slice(indices));
            if let Some(blas) = &old.blas {
                crate::ray::rebuild(gpu, blas, &old.vertices, vertices.len() as u32, &old.indices, indices.len() as u32);
            }
            self.meshes[mesh.0 as usize].bounds = crate::asset::Bounds::of(vertices);
            return;
        }
        let replaced = self.gpu_mesh(gpu, vertices, &[], indices, true, true);
        self.meshes[mesh.0 as usize] = replaced;
    }

    /// Unity's procedural sky into the sky-view table, when the frame's sky
    /// is it: what reflections read, a texel for what the shader would work
    /// out twice a pixel. Only when the sun or the sky's numbers moved —
    /// the table is a few thousand texels, worked out on every core.
    fn bake_unity_sky(&mut self, gpu: &Gpu, frame: &Frame) {
        if frame.sky.mode != SkyMode::Procedural {
            self.unity_sky_baked = None;
            return;
        }
        let to_sun = -frame.lighting.sun_direction.normalize_or(Vec3::NEG_Y);
        let sky = crate::procedural_sky::UnitySky::new(&frame.sky);
        let key = [
            to_sun.x,
            to_sun.y,
            to_sun.z,
            sky.inv_wavelength.x,
            sky.inv_wavelength.y,
            sky.inv_wavelength.z,
            sky.rayleigh,
            sky.ground.x,
            sky.ground.y,
            sky.ground.z,
            sky.exposure,
        ];
        // A tenth of a degree of sun is no change a reflection shows.
        let close = self.unity_sky_baked.is_some_and(|old| {
            Vec3::new(old[0], old[1], old[2]).dot(to_sun) > 0.999_998 && old[3..] == key[3..]
        });
        if close {
            return;
        }
        self.unity_sky_baked = Some(key);
        let (w, h) = crate::atmosphere::SKY_VIEW;
        let rows = scrap_core::jobs::map_range(h as usize, 4, |y| {
            let mut row = Vec::with_capacity(w as usize * 4);
            for x in 0..w {
                // The table's own mapping (physical_sky in render.wgsl):
                // azimuth across, the square root of latitude up.
                let u = (x as f32 + 0.5) / w as f32;
                let v = (y as f32 + 0.5) / h as f32;
                let azimuth = (u - 0.5) * std::f32::consts::TAU;
                let t = v * 2.0 - 1.0;
                let latitude = t.signum() * t * t * std::f32::consts::FRAC_PI_2;
                let d = Vec3::new(latitude.cos() * azimuth.cos(), latitude.sin(), latitude.cos() * azimuth.sin());
                let c = sky.radiance(d, to_sun);
                for channel in [c.x, c.y, c.z, 1.0] {
                    row.push(half_bits(channel));
                }
            }
            row
        });
        let texels: Vec<u16> = rows.into_iter().flatten().collect();
        gpu.queue.write_texture(
            self.atmosphere.sky_view.texture().as_image_copy(),
            bytemuck::cast_slice(&texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 8),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    /// The probes' pictures, when the probes are not the ones they are of:
    /// each face the scene as seen from the probe, lit and with its sky,
    /// but with no probes of its own, no post-processing and no tools.
    fn bake_probes(&mut self, gpu: &Gpu, frame: &Frame) {
        let wanted: Vec<crate::reflections::ReflectionProbe> = frame
            .reflection_probes
            .iter()
            .take(crate::reflections::MAX_PROBES)
            .copied()
            .collect();
        if wanted == self.reflections.baked {
            return;
        }
        self.reflections.baked.clear();
        self.reflections.baking = true;
        self.reflections.bouncing = false;
        self.rebind(gpu);
        let size = crate::reflections::PROBE_SIZE;
        let layers = (wanted.len() * 6) as u32;
        for round in 0..crate::reflections::BOUNCES {
            if round > 0 {
                // Lit by the last round: its pictures kept aside, and the
                // probes they are of in the frame.
                self.reflections.keep_round(gpu, layers);
                self.reflections.baked = wanted.clone();
                self.reflections.bouncing = true;
                self.rebind(gpu);
            }
            for (i, probe) in wanted.iter().enumerate() {
                for face in 0..6 {
                    let seen = Frame {
                        camera: crate::reflections::face_camera(probe, face),
                        // The clouds' picture is the camera's, not the probe's.
                        weather: crate::weather::Weather {
                            dust_wall: 0.0,
                            ..frame.weather
                        },
                        sky: Sky {
                            clouds: crate::clouds::Clouds {
                                coverage: 0.0,
                                ..frame.sky.clouds
                            },
                            ..frame.sky
                        },
                        post: crate::post::PostProcess::OFF,
                        ambient_occlusion: crate::ssao::AmbientOcclusion::OFF,
                        ray_tracing: crate::ray::RayTracing::default(),
                        overlay_draws: Vec::new(),
                        outline_draws: Vec::new(),
                        reflection_probes: Vec::new(),
            irradiance_volumes: Vec::new(),
                        ..frame.clone()
                    };
                    self.render_view(gpu, None, size, size, &seen, Some((i * 6 + face) as u32));
                }
            }
            self.reflections.make_mips(gpu, 0..layers);
        }
        self.reflections.bouncing = false;
        self.reflections.baked = wanted;
        self.reflections.baking = false;
        self.rebind(gpu);
    }

    /// One view of the frame: into `view` with everything, or with
    /// `probe` into that layer of the probes' pictures, lit and nothing
    /// more.
    fn render_view(
        &mut self,
        gpu: &Gpu,
        view: Option<&wgpu::TextureView>,
        width: u32,
        height: u32,
        frame: &Frame,
        probe: Option<u32>,
    ) {
        // Graphics settings: what is switched off is taken out of the frame,
        // by the preset and by the passes switched.
        let masked;
        let frame = if self.passes == crate::passes::Passes::MAX && self.quality.is_none() {
            frame
        } else {
            // One copy for both, and without the live meshes and the
            // cameras' frames, which the view does not read.
            let mut copy = frame.shallow();
            if let Some(quality) = self.quality {
                quality.apply_to(&mut copy);
            }
            self.passes.apply_to(&mut copy);
            masked = copy;
            &masked
        };
        let aspect = width as f32 / height.max(1) as f32;
        // Upscaled: the screen's own view drawn at fewer pixels, up to the
        // lens, and made up to `output` before post.
        let output = (width, height);
        let upscaling = frame.post.upscaling;
        let screen = probe.is_none() && view.is_some() && !self.picturing;
        let scaling = upscaling.enabled && frame.post.enabled && screen;
        let (width, height) = if scaling {
            self.upscaler.render_size(&upscaling, output)
        } else {
            output
        };
        if !scaling {
            self.upscaler.used = None;
        }
        // The screen's frame timed, when asked — or when dynamic resolution
        // needs the GPU's time: not a probe's face or a picture's.
        let timed = (self.timing || (scaling && upscaling.dynamic.enabled)) && screen;
        // The Frame Debugger's frame: the screen's, recorded pass by pass.
        let debugged = screen && self.debugger.wanted.is_some();
        if debugged {
            crate::frame_debugger::begin(self.debugger.wanted.flatten());
        }
        // The irradiance volume, for the screen's frame: its probes made
        // again when its grid changes, and the frame's group with them.
        if screen && self.ddgi.prepare(gpu, frame.irradiance_volumes.first().copied()) {
            self.rebind(gpu);
        }
        if timed {
            // Made the first time it is wanted: a renderer that is never
            // timed never holds a query set.
            if self.timer.is_none() {
                self.timer = crate::gpu_timer::GpuTimer::new(gpu);
            }
            if let Some(timer) = self.timer.as_mut() {
                timer.begin(gpu);
            }
        }
        // Multisampling only where nothing else smooths the edges: TAA,
        // MetalFX temporal (on only with TAA, in its place) and FXAA do it
        // at a fraction of four samples' cost — which is three to eight
        // times a whole frame's without them.
        if screen {
            let smoothed = frame.post.enabled && (frame.post.taa || frame.post.fxaa);
            let wanted = if smoothed { 1 } else { sample_count(gpu) };
            if wanted != self.samples {
                self.set_samples(gpu, wanted);
            }
        }
        if self.depth_size != (width, height) {
            self.depth = depth_view(gpu, width, height, self.samples);
            self.scene = scene_targets(gpu, width, height, self.samples);
            self.depth_size = (width, height);
            self.rebind(gpu);
        }
        if probe.is_none() && self.ssao.resize(gpu, (width, height)) {
            self.rebind(gpu);
        }
        // Temporal antialiasing: the screen's own view only, moved a
        // fraction of a pixel each frame it has a history to blend into.
        let taa_on = frame.post.taa && probe.is_none() && view.is_some() && !self.picturing;
        // MetalFX temporal takes TAA's place: its jitter, its history.
        let temporal = scaling && self.upscaler.temporal(&upscaling, taa_on);
        // At the screen's own size only MetalFX temporal has anything to
        // do: it is the antialiasing then.
        let upscale_on = temporal || (scaling && (width, height) != output);
        if scaling && !upscale_on {
            self.upscaler.used = None;
        }
        let taa_run = taa_on && !temporal;
        if temporal {
            self.upscaler.follow(
                frame.camera.position,
                (frame.camera.target - frame.camera.position).normalize_or(Vec3::NEG_Z),
            );
        }
        if taa_run {
            self.taa.resize(gpu, (width, height));
            self.taa.follow(
                frame.camera.position,
                (frame.camera.target - frame.camera.position).normalize_or(Vec3::NEG_Z),
            );
        }
        let jitter = if temporal {
            self.upscaler.jitter((width, height))
        } else if taa_run {
            self.taa.jitter()
        } else {
            glam::Vec2::ZERO
        };
        // Shaped ground drawn finely round the screen's camera: its heights
        // up for the vertex shader, remade when it changes.
        let fine_terrain = frame
            .terrain
            .filter(|_| probe.is_none() && view.is_some() && !self.picturing);
        if let Some(t) = fine_terrain {
            if self.clipmap.is_none() {
                self.clipmap = Some(self.upload_mesh_owned(gpu, &crate::terrain::clipmap_mesh()));
            }
            if self.terrain_made != Some(t.terrain) {
                let cells = t.terrain.cells.clamp(2, 2048);
                self.terrain_heights = terrain_height_view(gpu, cells + 1, &t.terrain.heights());
                self.terrain_made = Some(t.terrain);
                self.rebind(gpu);
                self.terrain_mesh_group = self.terrain_mesh_layouts.as_ref().map(|(layout, _)| {
                    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("terrain mesh"),
                        layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: self.terrain_mesh_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&self.terrain_heights),
                            },
                        ],
                    })
                });
            }
        }
        // What is drawn with: the camera, moved by the jitter.
        let drawn = Mat4::from_translation(Vec3::new(jitter.x, jitter.y, 0.0))
            * frame.camera.view_projection(aspect);
        // Dust near the ground — a wall, a devil, sand off a crest — is
        // marched at half size; clouds alone at a quarter.
        let near_dust = frame.weather.dust_wall > 0.0
            || frame.weather.dust_devils > 0.0
            || (!frame.plumes.is_empty()
                && (frame.wind.strength > 1.0 || frame.weather.sandstorm > 0.0));
        if probe.is_none()
            && (frame.sky.clouds.coverage > 0.0 || near_dust)
            && self
                .clouds
                .resize(gpu, (width, height), if near_dust { 2 } else { 4 })
        {
            self.rebind(gpu);
        }

        if frame.shadows.enabled && self.shadow_resolution != frame.shadows.resolution {
            self.shadow_resolution = frame.shadows.resolution.max(1);
            (self.shadow_map, self.shadow_layers) = shadow_view(gpu, self.shadow_resolution);
            self.vsm.resize(self.shadow_resolution);
            self.rebind(gpu);
        }

        if frame.shadows.enabled
            && self.light_shadow_resolution != frame.shadows.light_resolution.max(1)
        {
            self.light_shadow_resolution = frame.shadows.light_resolution.max(1);
            (self.light_shadow_map, self.light_shadow_layers) = shadow_layers_view(
                gpu,
                self.light_shadow_resolution,
                crate::lights::SHADOW_LAYERS as u32,
            );
            self.rebind(gpu);
        }

        // The lights: those the view sees, in cells, and the nearest given
        // shadow maps — unless rays shadow every lamp instead.
        let traced_lamps = self.ray.is_some() && frame.ray_tracing.light_shadows;
        let decal_bounds: Vec<(glam::Vec3, f32)> =
            frame.decals.iter().map(crate::decals::bounds).collect();
        let clustered = crate::lights::cluster(
            &frame.lights,
            &decal_bounds,
            &frame.camera,
            aspect,
            frame.shadows.enabled && !traced_lamps,
            self.light_shadow_resolution,
        );
        if !clustered.lights.is_empty() {
            gpu.queue.write_buffer(
                &self.light_buffer,
                0,
                bytemuck::cast_slice(&clustered.lights),
            );
        }
        gpu.queue
            .write_buffer(&self.cell_buffer, 0, bytemuck::cast_slice(&clustered.cells));
        if clustered.indices.len() as u64 > self.index_capacity {
            self.index_capacity = (clustered.indices.len() as u64).next_power_of_two();
            self.index_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("light lists"),
                size: self.index_capacity * 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.rebind(gpu);
        }
        if !clustered.indices.is_empty() {
            gpu.queue.write_buffer(
                &self.index_buffer,
                0,
                bytemuck::cast_slice(&clustered.indices),
            );
        }
        // The decals the cells name, their pictures put in the atlases.
        let mut remade = false;
        let decals: Vec<crate::decals::GpuDecal> = clustered
            .decals
            .iter()
            .map(|&i| {
                let decal = &frame.decals[i];
                let mut layer = |id: Option<crate::asset::AssetId>, normal: bool| {
                    let handle = self.texture_for(id?)?;
                    let source = &self.textures.get(handle.0 as usize)?.view;
                    let (layer, made) = self.decal_atlases.layer_for(gpu, handle, source, normal);
                    remade |= made;
                    layer
                };
                let colour = layer(decal.material.base_map, false);
                let normal = layer(decal.material.normal_map, true);
                crate::decals::GpuDecal::new(decal, colour, normal)
            })
            .collect();
        if remade {
            self.rebind(gpu);
        }
        if !decals.is_empty() {
            gpu.queue
                .write_buffer(&self.decal_buffer, 0, bytemuck::cast_slice(&decals));
        }
        let light_views: Vec<[[f32; 4]; 4]> = clustered
            .shadow_views
            .iter()
            .map(|m| m.to_cols_array_2d())
            .collect();
        if !light_views.is_empty() {
            gpu.queue.write_buffer(
                &self.light_view_buffer,
                0,
                bytemuck::cast_slice(&light_views),
            );
        }

        let sun = frame.lighting.sun_direction.normalize_or_zero();
        // Traced sun shadows need no maps.
        let traced_sun = self.ray.is_some() && frame.ray_tracing.sun_shadows;
        // Contact shadows read the prepass's depth, the screen's: not for a
        // probe's face, and not when rays already find every shadow.
        let contact_on = frame.shadows.enabled
            && frame.shadows.contact > 0.0
            && probe.is_none()
            && !traced_sun;
        // The screen's sun from virtual shadow maps, where asked: a probe's
        // face and a picture keep the cascades, so the pages stay the
        // screen's.
        let virtual_on = frame.shadows.enabled
            && frame.shadows.virtual_maps
            && !traced_sun
            && probe.is_none()
            && view.is_some()
            && !self.picturing
            && sun.length_squared() > 0.5;
        // The lamps by ReSTIR, for the screen's frame on a device that
        // traces: its reservoirs the size of the frame.
        let restir_on = self.ray.is_some()
            && frame.ray_tracing.restir
            && self.restir.initial.is_some()
            && probe.is_none()
            && view.is_some()
            && !self.picturing;
        if restir_on && self.restir.resize(gpu, (width, height)) {
            self.rebind(gpu);
        }
        let mut cascades = if frame.shadows.enabled && !traced_sun && !virtual_on {
            self.cascades(frame, sun, aspect)
        } else {
            Vec::new()
        };
        // The far cascades drawn every other frame, each on its turn: the
        // one not drawn keeps its map and the view it was drawn from, so
        // the lit pass reads it as it was made. Something far that moves
        // is a frame late in its shadow; the near cascades, where a
        // shadow is looked at, are drawn every frame. Only the screen's
        // own maps are kept: a probe's face or a picture draws into the
        // same layers, and then everything is drawn again.
        let mut kept_cascades = [false; MAX_CASCADES];
        if !cascades.is_empty() {
            // Not while the Frame Debugger takes the frame apart: the
            // same frame drawn again must list the same passes.
            let own = screen && self.shadow_stagger && self.debugger.wanted.is_none();
            let fits = self.cascade_cache.as_ref().is_some_and(|c| {
                c.settings == frame.shadows
                    && c.resolution == self.shadow_resolution
                    && c.views.len() == cascades.len()
                    && c.sun.dot(sun) > 0.9999
            });
            if own && fits && cascades.len() >= 3 {
                self.shadow_turn = !self.shadow_turn;
                let cache = self.cascade_cache.as_ref().expect("fits");
                for i in 2..cascades.len() {
                    if (i % 2 == 0) == self.shadow_turn {
                        cascades[i] = cache.views[i];
                        kept_cascades[i] = true;
                    }
                }
            }
            self.cascade_cache = own.then(|| CascadeCache {
                settings: frame.shadows,
                resolution: self.shadow_resolution,
                sun,
                views: cascades.clone(),
            });
        }
        let mut light_view_projection = [Mat4::IDENTITY.to_cols_array_2d(); MAX_CASCADES];
        let mut cascade_spheres = [[0.0f32; 4]; MAX_CASCADES];
        let mut cascade_bias = [0.0f32; 4];
        let mut caster_bias = [[0.0f32; 2]; MAX_CASCADES];
        for (i, (matrix, centre, radius, texel, _)) in cascades.iter().enumerate() {
            light_view_projection[i] = matrix.to_cols_array_2d();
            cascade_spheres[i] = extend(*centre, radius * radius);
            cascade_bias[i] = *texel;
            // URP's ShadowUtils.GetShadowBias: the biases in the cascade's
            // texels, grown by how far its soft filter reaches.
            let reach = texel * frame.shadows.soft.kernel_radius();
            caster_bias[i] = [frame.shadows.depth_bias * reach, frame.shadows.normal_bias * reach];
        }
        let cascade_depth_bias = {
            let (scale, bias) = shadow_fade(frame.shadows.max_distance, frame.shadows.cascade_border);
            [scale, bias, 0.0, 0.0]
        };
        // The fog as the weather leaves it: a sandstorm thickens both.
        let time = frame
            .time
            .unwrap_or_else(|| self.started.elapsed().as_secs_f32());
        // The weather where the camera is: inside a dust wall, a storm.
        let weather = frame.weather.at(frame.camera.position, &frame.wind, time);
        let mut volumetric = weather.storm_fog(&frame.volumetric_fog);
        // Dust in the air is in the fog's grid: with any, the grid runs,
        // clear where there is none.
        let puffs = if probe.is_none() {
            let mut near: Vec<&crate::volume::Puff> = frame.puffs.iter().collect();
            let eye = frame.camera.position;
            near.sort_by(|a, b| {
                (a.position - eye)
                    .length_squared()
                    .total_cmp(&(b.position - eye).length_squared())
            });
            near.truncate(crate::volume::MOST_PUFFS);
            near
        } else {
            Vec::new()
        };
        // Dust devils where the weather has them; sand off the crests as
        // much as the wind is strong (past a stiff breeze) or a storm blows.
        let devils = if probe.is_none() {
            weather.devils(&frame.wind, time)
        } else {
            Vec::new()
        };
        let blowing = (((frame.wind.strength - 1.0) / 1.5).clamp(0.0, 1.0))
            .max(weather.sandstorm.clamp(0.0, 1.0));
        let plumes: Vec<crate::volume::Plume> = if probe.is_none() && blowing > 0.0 {
            let eye = frame.camera.position;
            let mut near: Vec<crate::volume::Plume> = frame
                .plumes
                .iter()
                .filter(|p| (p.position - eye).length_squared() < 150.0 * 150.0)
                .copied()
                .collect();
            near.sort_by(|a, b| {
                (a.position - eye)
                    .length_squared()
                    .total_cmp(&(b.position - eye).length_squared())
            });
            near.truncate(crate::volume::MOST_PLUMES);
            // Thinning toward the edge of those taken, so where the list
            // ends there is no line across the sky.
            let edge = near
                .last()
                .map_or(1.0, |p| (p.position - eye).length())
                .max(1.0);
            for p in &mut near {
                let d = (p.position - eye).length();
                p.strength *= blowing * ((edge - d) / (edge * 0.35)).clamp(0.0, 1.0);
            }
            near
        } else {
            Vec::new()
        };
        // Devils and plumes are marched with the clouds, precisely: too
        // thin and too far for the fog's grid.
        let local_dust = !devils.is_empty() || !plumes.is_empty();
        let smoke = if probe.is_none() { &frame.smoke[..] } else { &[] };
        if (!puffs.is_empty() || !smoke.is_empty()) && !volumetric.enabled {
            volumetric = crate::volume::VolumetricFog {
                enabled: true,
                density: 0.0,
                ..crate::volume::VolumetricFog::OFF
            };
        }

        let fog = weather.storm_distance(&frame.fog);
        // With a physical sky, the sun's colour and the light from all
        // round come from the air, not from the scene's picked colours.
        let physical = frame.sky.mode == SkyMode::Physical;
        let to_sun = -frame.lighting.sun_direction.normalize_or(Vec3::NEG_Y);
        let altitude = frame.camera.position.y.max(1.0);
        // The sun the sky is lit by: at night, under the horizon — the
        // light above is then the moon's.
        let (sky_to_sun, sky_sun_intensity) = match frame.lighting.sky_sun {
            Some((direction, intensity)) => (-direction.normalize_or(Vec3::NEG_Y), intensity),
            None => (to_sun, frame.lighting.sun_intensity),
        };
        let night = frame.lighting.night.clamp(0.0, 1.0);
        let (sun_light, sky_light, ground_light) = if physical {
            // The same air, height and sun as the last frame (the camera
            // standing, the sun barely moving): the same light, not found
            // again through thirty thousand steps of air.
            let air = frame.sky.atmosphere;
            let (sun, sky) = match self.sky_light {
                Some((was, at, towards, light)) if was == air && at == altitude && towards == sky_to_sun => light,
                _ => {
                    let light = air.light_of_one(altitude, sky_to_sun);
                    self.sky_light = Some((air, altitude, sky_to_sun, light));
                    light
                }
            };
            let (sun, sky) = (sun * sky_sun_intensity, sky * sky_sun_intensity);
            // At night the moon, as the scene's lighting has it, and the
            // night sky's own faint light on top of what the air still
            // glows with.
            let (sun, sky) = if frame.lighting.sky_sun.is_some() {
                let moon = frame.lighting.sun_color * frame.lighting.sun_filter * frame.lighting.sun_intensity;
                (
                    sun.lerp(moon, (night * 2.0 - 1.0).clamp(0.0, 1.0)),
                    sky + frame.lighting.sky_color * night,
                )
            } else {
                (sun, sky)
            };
            // The ground lit by them, sending its colour back up.
            let ground = frame.lighting.ground_albedo * (sun * to_sun.y.max(0.0) + sky * 0.5);
            (sun, sky, ground)
        } else {
            (
                frame.lighting.sun_color * frame.lighting.sun_filter * frame.lighting.sun_intensity,
                frame.lighting.sky_color,
                frame.lighting.ground_color,
            )
        };
        // In a sandstorm the sun is a dim orange disc through the sand, and
        // what light there is comes from the glowing air all round.
        let storm = weather.sandstorm.clamp(0.0, 1.0);
        let sun_light =
            sun_light * (1.0 - 0.8 * storm) * Vec3::new(1.0, 1.0 - 0.25 * storm, 1.0 - 0.5 * storm);
        // A scene that says its light from all round has it, whatever the
        // sky would work out: above, the horizon, below.
        // Unity's ambient probe: the scene's gradient as the probe has it,
        // or, when the scene says no light of its own, Unity's procedural
        // sky's — what a face turned up, sideways and down sees.
        let unity_sky = crate::procedural_sky::UnitySky::new(&frame.sky);
        let ambient = match frame.lighting.ambient {
            Some([sky, equator, ground]) => {
                Some(crate::procedural_sky::gradient_seen(sky, equator, ground))
            }
            None if frame.sky.mode == SkyMode::Procedural => Some(unity_sky.ambient(
                sky_to_sun,
                frame.lighting.sun_color * frame.lighting.sun_filter * sky_sun_intensity,
                frame.sky.sun_size,
            )),
            None => None,
        };
        let (sky_light, ground_light, equator) = match ambient {
            Some([up, side, down]) => (up, down, extend(side, 1.0)),
            None => (sky_light, ground_light, [0.0; 4]),
        };
        let sky_light = sky_light.lerp(Vec3::new(0.55, 0.4, 0.26), storm * 0.7);
        let ground_light = ground_light.lerp(Vec3::new(0.35, 0.25, 0.15), storm * 0.7);
        let mut foliage = crate::foliage::FoliageUniform::new(
            &frame.wind,
            &frame.benders,
            frame.camera.position,
            frame
                .time
                .unwrap_or_else(|| self.started.elapsed().as_secs_f32()),
        );
        // The pages this frame needs, and those to draw.
        let vsm_jobs = if virtual_on {
            let mut casters = Vec::new();
            for draw in &frame.draws {
                if draw.material.is_transparent() {
                    continue;
                }
                let Some(mesh) = self.meshes.get(draw.mesh.0 as usize) else { continue };
                let (lo, hi) = world_box(mesh.bounds, draw.transform);
                use std::hash::{Hash, Hasher};
                // Only compared with the last frame's key, in this process:
                // no attacker picks it, and SipHash here cost a word's worth
                // of rounds per float of every caster, every frame.
                let mut hash = scrap_core::hash::FastHasher::default();
                draw.mesh.0.hash(&mut hash);
                for v in draw.transform.to_cols_array() {
                    v.to_bits().hash(&mut hash);
                }
                draw.material.alpha_clip.to_bits().hash(&mut hash);
                casters.push(crate::vsm::Caster {
                    rect: self.vsm.rect_of(sun, lo, hi),
                    key: hash.finish(),
                });
            }
            let camera = &frame.camera;
            // Texels half a screen pixel: the filter's nine taps then stay
            // inside the pixel's own width.
            let footprint = 0.5
                * match camera.ortho {
                    Some(half) => -(2.0 * half / height.max(1) as f32),
                    None => 2.0 * (camera.fov_y_degrees.to_radians() * 0.5).tan() / height.max(1) as f32,
                };
            let plan_view = crate::vsm::View {
                eye: camera.position,
                inverse_view_projection: camera.view_projection(aspect).inverse(),
                near: camera.near,
                far: camera.far,
                footprint,
                max_distance: frame.shadows.max_distance,
            };
            self.vsm.plan(gpu, sun, &plan_view, casters, bytemuck::bytes_of(&foliage))
        } else {
            Vec::new()
        };
        // A new distance field: into its texture, and bound anew.
        if self.distance.update(gpu, frame.distance_field.as_ref()) {
            self.rebind(gpu);
        }
        // The benders press into the trample map, which springs back as
        // the clock runs; a reflection probe's picture leaves it alone.
        if probe.is_none() && !self.picturing {
            self.trample.press(&frame.benders, frame.camera.position, foliage.wind[3]);
            gpu.queue.write_texture(
                self.trample_texture.as_image_copy(),
                bytemuck::cast_slice(&self.trample.texels()),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(crate::foliage::TRAMPLE_CELLS * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: crate::foliage::TRAMPLE_CELLS,
                    height: crate::foliage::TRAMPLE_CELLS,
                    depth_or_array_layers: 1,
                },
            );
        }
        foliage.trample = [self.trample.centre.x, self.trample.centre.y, crate::foliage::TRAMPLE_SIZE, 1.0];
        let to_sun = -sun;
        let casters: Vec<u8> = light_view_projection
            .iter()
            .enumerate()
            .map(|(i, m)| (m, caster_bias[i]))
            .chain(light_views.iter().map(|m| (m, [0.0; 2])))
            .flat_map(|(matrix, [depth, normal])| {
                let mut slot = vec![0u8; self.caster_stride as usize];
                let one = CasterUniform {
                    view_projection: *matrix,
                    foliage,
                    toward: extend(to_sun, depth),
                    inset: [normal, 0.0, 0.0, 0.0],
                };
                slot[..std::mem::size_of::<CasterUniform>()]
                    .copy_from_slice(bytemuck::bytes_of(&one));
                slot
            })
            .collect();
        gpu.queue.write_buffer(&self.casters, 0, &casters);

        let uniform = FrameUniform {
            view_projection: drawn.to_cols_array_2d(),
            sun_direction: extend(frame.lighting.sun_direction.normalize_or_zero(), 0.0),
            sun_color: extend(sun_light, 1.0 - frame.lighting.sun_shadow_strength.clamp(0.0, 1.0)),
            sky_color: extend(sky_light, frame.sky.reflection_intensity.max(0.0)),
            ground_color: extend(ground_light, 0.0),
            fog_color: extend(fog.color, 0.0),
            fog_range: [
                fog.start,
                fog.end,
                match fog.mode {
                    FogMode::Linear => 0.0,
                    FogMode::Exponential => 1.0,
                    FogMode::ExponentialSquared => 2.0,
                },
                fog.density.max(0.0),
            ],
            camera_position: extend(frame.camera.apparent_eye(), 1.0),
            light_view_projection,
            shadow_params: [
                frame.shadows.soft.index(),
                0.0,
                1.0 / self.shadow_resolution as f32,
                cascades.len() as f32,
            ],
            view_depth: {
                let view = crate::lights::view_of(&frame.camera);
                view.row(2).to_array()
            },
            clusters: [
                crate::lights::TILES_X as f32,
                crate::lights::TILES_Y as f32,
                crate::lights::SLICES as f32,
                clustered.lights.len() as f32,
            ],
            cluster_depth: {
                let near = frame.camera.near.max(1e-3);
                let far = frame.camera.far.max(frame.camera.near + 1e-2);
                [near, (far / near).ln(), width as f32, height as f32]
            },
            light_shadow: [1.0 / self.light_shadow_resolution as f32, 0.0, 0.0, 0.0],
            inverse_view_projection: drawn.inverse().to_cols_array_2d(),
            // Unity's procedural sky wants its own numbers where the
            // gradient's colours go: 1/λ⁴ for the zenith, Rayleigh's
            // constant and the sun's radius (radians) for the horizon.
            sky_zenith: {
                let c = if frame.sky.mode == SkyMode::Procedural {
                    unity_sky.inv_wavelength.to_array()
                } else {
                    frame.sky.zenith
                };
                [
                    c[0],
                    c[1],
                    c[2],
                    match frame.sky.mode {
                        SkyMode::Color => 0.0,
                        SkyMode::Gradient => 1.0,
                        SkyMode::Physical => 2.0,
                        SkyMode::Procedural => 3.0,
                    },
                ]
            },
            sky_horizon: {
                // No disc in a probe's picture: a probe's texels are coarse,
                // and the disc would come back from every mirror as a square
                // — the sun's highlight is the lights' own.
                let radius = if probe.is_some() {
                    0.0
                } else {
                    frame.sky.sun_size.max(0.0).to_radians() * 0.5
                };
                let c = if frame.sky.mode == SkyMode::Procedural {
                    [unity_sky.rayleigh, radius, 0.0]
                } else {
                    frame.sky.horizon
                };
                [c[0], c[1], c[2], radius.cos()]
            },
            sky_ground: [
                frame.sky.ground[0],
                frame.sky.ground[1],
                frame.sky.ground[2],
                frame.sky.exposure.max(0.0),
            ],
            cascade_spheres,
            cascade_bias,
            cascade_depth_bias,
            ambient_occlusion: [
                if frame.ambient_occlusion.enabled {
                    1.0
                } else {
                    0.0
                },
                frame
                    .ambient_occlusion
                    .direct_lighting_strength
                    .clamp(0.0, 1.0),
                // Contact shadows: how far their rays go.
                if contact_on { frame.shadows.contact } else { 0.0 },
                if taa_on { (time * 37.0).fract() * 0.618_034 } else { 0.0 },
            ],
            ray: {
                let on = |asked: bool| {
                    if asked && self.ray.is_some() {
                        1.0
                    } else {
                        0.0
                    }
                };
                let rt = &frame.ray_tracing;
                [
                    on(rt.sun_shadows),
                    on(rt.light_shadows),
                    on(rt.ambient_occlusion),
                    (rt.sun_size.max(0.0).to_radians() * 0.5).tan(),
                ]
            },
            // With TAA the rays turn every frame and the history adds them
            // up, ten frames' worth: half each frame is enough.
            ray_params: [
                rays_a_frame(frame.ray_tracing.sun_rays, taa_on) as f32,
                rays_a_frame(frame.ray_tracing.occlusion_rays, taa_on) as f32,
                frame.ray_tracing.occlusion_radius.max(0.01),
                frame.ray_tracing.lamp_size.max(0.0),
            ],
            probe_params: [
                self.reflections.baked.len() as f32,
                (crate::reflections::PROBE_MIPS - 1) as f32,
                // Reflections by rays, and up to what roughness.
                if frame.ray_tracing.reflections && self.ray.is_some() && probe.is_none() {
                    1.0
                } else {
                    0.0
                },
                frame.ray_tracing.reflection_roughness.clamp(0.0, 1.0),
            ],
            probes: {
                let mut out = [[0.0; 4]; crate::reflections::MAX_PROBES * 2];
                for (i, probe) in self.reflections.baked.iter().enumerate() {
                    out[i * 2] = extend(probe.position, probe.blend_distance.max(0.0));
                    out[i * 2 + 1] = extend(
                        probe.extents.abs(),
                        if probe.box_projection { 1.0 } else { 0.0 },
                    );
                }
                out
            },
            probe_faces: std::array::from_fn(|f| {
                crate::reflections::face_matrix(f).to_cols_array_2d()
            }),
            volume: {
                let v = &volumetric;
                let near = frame.camera.near.max(1e-3);
                [
                    if v.enabled { 1.0 } else { 0.0 },
                    v.distance.max(near * 2.0),
                    near,
                    puffs.len() as f32,
                ]
            },
            fog_medium: {
                let v = &volumetric;
                [v.color[0], v.color[1], v.color[2], v.density.max(0.0)]
            },
            fog_shape: {
                let v = &volumetric;
                [
                    v.base_height,
                    v.height_falloff.max(0.0),
                    v.anisotropy.clamp(-0.95, 0.95),
                    v.ambient.max(0.0),
                ]
            },
            fog_lamps: [volumetric.lamps.max(0.0), 0.0, 0.0, 0.0],
            clear_color: extend(frame.clear_color, time),
            foliage,
            air: [if physical { 1.0 } else { 0.0 }, frame.camera.far, 0.0, 0.0],
            weather: {
                let mut u = weather.uniform();
                u[1][3] = weather.dust_front(&frame.wind, time);
                u
            },
            previous_view_projection: self
                .previous_view_projection
                .unwrap_or_else(|| frame.camera.view_projection(aspect))
                .to_cols_array_2d(),
            dust: [
                if local_dust { 1.0 } else { 0.0 },
                // The dust wall's height, for the shadow it throws.
                if weather.dust_wall > 0.0 { weather.dust_wall_height.max(10.0) } else { 0.0 },
                0.0,
                0.0,
            ],
            night: [night, 0.0, 0.0, 0.0],
            terrain_to_local: fine_terrain
                .map_or(Mat4::IDENTITY, |t| t.placed.inverse())
                .to_cols_array_2d(),
            terrain_to_world: fine_terrain
                .map_or(Mat4::IDENTITY, |t| t.placed)
                .to_cols_array_2d(),
            terrain: match fine_terrain {
                Some(t) => [
                    t.terrain.size.max(1.0),
                    t.terrain.cells.clamp(2, 2048) as f32,
                    1.0,
                    crate::terrain::CLIPMAP_FINEST,
                ],
                None => [1.0, 2.0, 0.0, crate::terrain::CLIPMAP_FINEST],
            },
            terrain_bounds: match fine_terrain {
                Some(t) => {
                    let base = t.placed.w_axis.y;
                    [
                        base - 0.5,
                        base + t.terrain.dunes.height * 1.3 + 0.5,
                        crate::terrain::PATCHES_PER_RING as f32,
                        0.0,
                    ]
                }
                None => [0.0; 4],
            },
            glass: [
                if frame.ray_tracing.refractions && self.ray.is_some() && probe.is_none() {
                    1.0
                } else {
                    0.0
                },
                frame.ray_tracing.index_of_refraction.max(1.0),
                self.bolt.as_ref().map_or(0.0, |b| b.points.len() as f32),
                self.bolt.as_ref().map_or(0.0, |b| b.flash),
            ],
            bolt: {
                let mut out = [[0.0; 4]; crate::weather::BOLT_POINTS];
                if let Some(b) = &self.bolt {
                    for (o, p) in out.iter_mut().zip(&b.points) {
                        *o = p.to_array();
                    }
                }
                out
            },
            ddgi: if probe.is_none() { self.ddgi.uniform() } else { [[0.0; 4]; 4] },
            vsm: if virtual_on { self.vsm.uniform } else { crate::vsm::OFF },
            restir: self.restir.uniform(restir_on),
            distance: crate::distance::uniform(frame.distance_field.as_ref()),
            ambient_equator: equator,
            terrain_look: fine_terrain
                .and_then(|t| frame.draws.iter().find(|d| d.mesh == t.mesh))
                .map_or([[0.0; 4]; 7], |d| {
                    let raw = instance_of(d.transform, &d.material);
                    [
                        raw.color_and_shading,
                        raw.surface,
                        raw.emission,
                        raw.uv,
                        raw.detail,
                        raw.params[0],
                        raw.params[1],
                    ]
                }),
            puffs: {
                let mut out = [[0.0; 4]; 2 * crate::volume::MOST_PUFFS];
                for (i, p) in puffs.iter().enumerate() {
                    out[2 * i] = [p.position.x, p.position.y, p.position.z, p.radius.max(0.01)];
                    out[2 * i + 1] = [p.color[0], p.color[1], p.color[2], p.density.max(0.0)];
                }
                out
            },
            ssr: {
                let s = &frame.screen_space_reflections;
                [
                    if s.enabled && probe.is_none() && self.scene.has_history {
                        1.0
                    } else {
                        0.0
                    },
                    s.max_distance.max(0.1),
                    s.thickness.max(0.01),
                    s.steps.clamp(4, 128) as f32,
                ]
            },
            clouds: {
                let (shape, mut drift) = frame.sky.clouds.vectors(&frame.wind);
                drift[3] = frame.sky.clouds.shadows.clamp(0.0, 1.0);
                if frame.sky.mode == SkyMode::Color {
                    [[0.0; 4], drift]
                } else {
                    [shape, drift]
                }
            },
            waters: {
                let mut out = [[0.0f32; 4]; 8];
                let planes = frame
                    .draws
                    .iter()
                    .filter(|d| d.material.shading == Shading::Water)
                    .filter_map(|d| {
                        let bounds = self.meshes.get(d.mesh.0 as usize)?.bounds;
                        Some(world_box(bounds, d.transform))
                    })
                    .take(4);
                for (i, (min, max)) in planes.enumerate() {
                    out[i * 2] = [max.y, 1.0, 0.0, 0.0];
                    out[i * 2 + 1] = [min.x, min.z, max.x, max.z];
                }
                out
            },
        };
        gpu.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniform));
        // Lean, when nothing the lean shader leaves out is asked for.
        self.lean.collect();
        self.lean.on = self.lean.enabled && lean_allowed(&uniform, frame, decals.is_empty());

        if fine_terrain.is_some() && self.terrain_mesh_group.is_some() {
            let t = TerrainFrameUniform {
                view_projection: uniform.view_projection,
                camera_position: uniform.camera_position,
                wind: foliage.wind,
                terrain_to_local: uniform.terrain_to_local,
                terrain_to_world: uniform.terrain_to_world,
                terrain: uniform.terrain,
                terrain_bounds: uniform.terrain_bounds,
                terrain_look: uniform.terrain_look,
            };
            gpu.queue
                .write_buffer(&self.terrain_mesh_buffer, 0, bytemuck::bytes_of(&t));
        }

        // Draws are grouped by pipeline, mesh and texture so that one mesh
        // drawn a hundred times costs one call. A forest is the same tree
        // over and over, so this is not a micro-optimisation, it is the
        // difference between one draw and a thousand.
        //
        // The shadow pass has its own set, because it must not use the
        // camera's frustum: something behind you can cast a shadow in front
        // of you, and culling it leaves a hole in the ground where its
        // shadow was. What is see-through casts none.
        //
        // What is transparent is drawn after everything solid and the sky,
        // one at a time, farthest first: blending is not commutative, and
        // near glass drawn before far glass hides it.
        let planes = frustum_planes(frame.camera.view_projection(aspect));
        let mut shadow_batches: Vec<(BatchKey, Vec<InstanceRaw>)> = Vec::new();
        let mut clip_batches: Vec<(BatchKey, Vec<InstanceRaw>)> = Vec::new();
        let mut batches: Vec<(BatchKey, Vec<InstanceRaw>)> = Vec::new();
        let mut singles: Vec<(Look, MeshHandle, Maps, u32, InstanceRaw)> = Vec::new();
        // The terrain's maps, when mesh shaders draw it.
        let mut mesh_terrain: Option<Maps> = None;
        let mut transparent: Vec<(f32, Look, MeshHandle, Maps, u32, InstanceRaw)> = Vec::new();
        let mut stats = FrameStats {
            submitted: frame.draws.len() as u32,
            ..Default::default()
        };
        let eye = frame.camera.apparent_eye();
        let terrain_mesh = fine_terrain.map(|t| t.mesh);
        // What each draw is, worked out on every core (scrap_core::jobs):
        // its instance, maps, level of detail and whether it is in view —
        // reading, not writing, so the draws split freely.
        let this = self.lookup();
        let prepared = scrap_core::jobs::map(&frame.draws, 256, |draw| {
            let mut raw = instance_of(draw.transform, &draw.material);
            let maps = this.maps_of(draw);
            raw.maps = packed(maps);
            let skinned = draw.pose.is_some()
                && this
                    .meshes
                    .get(draw.mesh.0 as usize)
                    .is_some_and(|m| m.skin.is_some());
            // The level of detail it is drawn at, by how much of the screen
            // it covers; none when it covers too little to see. The terrain
            // is always itself: it is drawn finely near and coarse far by
            // its own grid.
            let level = if Some(draw.mesh) == terrain_mesh || probe.is_some() {
                Some(draw.mesh)
            } else {
                this.level_of(draw, eye, &frame.camera, skinned)
            };
            let visible = match this.meshes.get(draw.mesh.0 as usize) {
                Some(mesh) => aabb_in_frustum(&planes, mesh.bounds, draw.transform),
                // A handle pointing at nothing draws nothing; it should not
                // also be reported as culled.
                None => false,
            };
            let covers = if Some(draw.mesh) == terrain_mesh {
                // The ground fills the view and tiles its maps finely.
                1.0
            } else {
                this.coverage(draw, eye, &frame.camera)
            };
            Prepared {
                raw,
                maps,
                skinned,
                level,
                visible,
                covers,
            }
        });
        let streaming_textures = !self.streams.is_empty() && probe.is_none() && view.is_some() && !self.picturing;
        let mut pool = std::mem::take(&mut self.batch_pool);
        let mut shadow_index = BatchIndex::tagged(0);
        let mut clip_index = BatchIndex::tagged(1);
        // What is drawn on both sides casts from both into the sun's
        // cascades; the rest from its front faces only, as URP's shadow
        // caster pass culls as the material does — which is what lets the
        // normal bias shrink a caster without its back faces, turned
        // inside out past its middle, swelling it again. (Their lists join
        // the one-sided ones after, and the pool with them.)
        let mut shadow_both: Vec<(BatchKey, Vec<InstanceRaw>)> = Vec::new();
        let mut clip_both: Vec<(BatchKey, Vec<InstanceRaw>)> = Vec::new();
        let mut shadow_both_index = BatchIndex::tagged(0);
        let mut clip_both_index = BatchIndex::tagged(1);
        let mut colour_index = BatchIndex::tagged(2);
        for (draw, prepared) in frame.draws.iter().zip(prepared) {
            let Prepared {
                raw,
                maps,
                skinned,
                level,
                visible,
                covers,
            } = prepared;
            // What its maps are seen at, for mip streaming: its height on
            // the screen in pixels, times how often it tiles them.
            if streaming_textures && visible && level.is_some() {
                let texels = covers * height as f32 * raw.uv[0].abs().max(raw.uv[1].abs()).max(1.0);
                for (i, handle) in maps.iter().enumerate() {
                    if let Some(stream) = self.streams.get_mut(handle) {
                        // A shader's own textures are tiled as only it
                        // knows — a road forty times across a field — so
                        // they are asked for whole.
                        stream.see(if i < 4 { texels } else { f32::MAX });
                    }
                }
            }
            // Bindless: the maps travel with the instance, and draws are not
            // split by them.
            let maps = self.batch_maps(maps);
            if !draw.material.is_transparent() {
                let key = (None, level.unwrap_or(draw.mesh), maps);
                // A mirrored one winds the other way: both, to be safe.
                let one_sided = draw.material.render_face == RenderFace::Front
                    && draw.transform.determinant() > 0.0;
                match (draw.material.alpha_clip > 0.0, one_sided) {
                    (true, true) => clip_index.push(&mut pool, &mut clip_batches, key, raw),
                    (true, false) => clip_both_index.push(&mut pool, &mut clip_both, key, raw),
                    (false, true) => shadow_index.push(&mut pool, &mut shadow_batches, key, raw),
                    (false, false) => shadow_both_index.push(&mut pool, &mut shadow_both, key, raw),
                }
                stats.shadow_casters += 1;
            }

            let Some(mesh) = level.filter(|_| visible) else {
                stats.culled += 1;
                continue;
            };
            stats.drawn += 1;
            let mut look = Look::of(&draw.material, skinned);
            // Mirrored — a right hand that is a left one scaled by -1 —
            // its faces wind the other way round: the other side culled.
            if draw.transform.determinant() < 0.0 {
                look.face = match look.face {
                    RenderFace::Front => RenderFace::Back,
                    RenderFace::Back => RenderFace::Front,
                    face => face,
                };
            }
            // The terrain near the camera: its fine grid instead, placed and
            // raised by its own vertex shader. Its mesh still casts shadows.
            if fine_terrain.is_some_and(|t| t.mesh == draw.mesh)
                && self.pipelines.terrain_mesh.is_some()
                && self.terrain_mesh_group.is_some()
                && !draw.material.is_transparent()
            {
                // By mesh shaders, after the batches: its maps are all it
                // needs from the draw; the rest is in the frame.
                mesh_terrain = Some(maps);
                continue;
            }
            if let (Some(t), Some(grid)) = (fine_terrain, self.clipmap) {
                if draw.mesh == t.mesh && !draw.material.is_transparent() {
                    let look = Look {
                        terrain: true,
                        face: RenderFace::Front,
                        ..look
                    };
                    colour_index.push(&mut pool, &mut batches, (Some(look), grid, maps), raw);
                    continue;
                }
            }
            let pose = draw.pose.unwrap_or(0);
            if draw.material.is_transparent() {
                let distance = (draw.transform.w_axis.truncate() - eye).length_squared();
                transparent.push((distance, look, mesh, maps, pose, raw));
            } else if skinned {
                // Skinned draws are not batched: each one has its own pose,
                // so two of them cannot share an instanced call anyway.
                singles.push((look, mesh, maps, pose, raw));
            } else {
                colour_index.push(&mut pool, &mut batches, (Some(look), mesh, maps), raw);
            }
        }
        if streaming_textures {
            for stream in self.streams.values_mut() {
                stream.end_frame();
            }
        }
        stats.batches = batches.len() as u32;
        stats.triangles = batches
            .iter()
            .map(|((_, mesh, _), list)| (list.len(), *mesh))
            .chain(singles.iter().map(|(_, mesh, _, _, _)| (1, *mesh)))
            .chain(transparent.iter().map(|(_, _, mesh, _, _, _)| (1, *mesh)))
            .map(|(count, mesh)| self.mesh(mesh).map_or(0, |m| m.index_count as u64 / 3) * count as u64)
            .sum();
        self.stats = stats;
        if self.lean.on {
            let looks: Vec<Look> = batches
                .iter()
                .filter_map(|((look, _, _), _)| *look)
                .chain(singles.iter().map(|s| s.0))
                .chain(transparent.iter().map(|t| t.1))
                .collect();
            self.ask_lean(gpu, looks.into_iter());
            let faces: Vec<(RenderFace, bool)> = batches
                .iter()
                .filter_map(|((look, handle, _), _)| {
                    let look = (*look)?;
                    self.mesh(*handle)?.clusters.as_ref()?;
                    Some((look.face, look.water))
                })
                .collect();
            self.ask_lean_clusters(gpu, faces.into_iter());
        }
        // One-sided first, then both: each range with its own culling.
        let shadow_one_sided = shadow_batches.len();
        shadow_batches.extend(shadow_both);
        let clip_one_sided = clip_batches.len();
        clip_batches.extend(clip_both);
        // Each lamp shadow map's casters: what its own view sees.
        let mut lamp_batches: Vec<(Batches, Batches)> = Vec::new();
        for (lamp, view) in clustered.shadow_views.iter().enumerate() {
            let planes = frustum_planes(*view);
            let (mut solid, mut clipped) = (Vec::new(), Vec::new());
            let lamp = 3 + 2 * lamp as u32;
            let (mut solid_index, mut clipped_index) = (BatchIndex::tagged(lamp), BatchIndex::tagged(lamp + 1));
            // What is unlit is a light itself: a lamp's bulb would
            // otherwise put everything around it in its shadow.
            for draw in frame
                .draws
                .iter()
                .filter(|d| !d.material.is_transparent() && d.material.shading != Shading::Unlit)
            {
                let Some(mesh) = self.meshes.get(draw.mesh.0 as usize) else {
                    continue;
                };
                if !aabb_in_frustum(&planes, mesh.bounds, draw.transform) {
                    continue;
                }
                let (list, index) = if draw.material.alpha_clip > 0.0 {
                    (&mut clipped, &mut clipped_index)
                } else {
                    (&mut solid, &mut solid_index)
                };
                let maps = self.maps_of(draw);
                let mut raw = instance_of(draw.transform, &draw.material);
                raw.maps = packed(maps);
                index.push(&mut pool, list, (None, draw.mesh, self.batch_maps(maps)), raw);
            }
            lamp_batches.push((solid, clipped));
        }
        // Farthest first.
        transparent.sort_by(|a, b| b.0.total_cmp(&a.0));
        // One pipeline change per look rather than per mesh.
        batches.sort_by_key(|((look, mesh, texture), _)| {
            let look = look.expect("colour batches carry their look");
            (
                look.skinned,
                look.face as u8,
                look.blend.map(|b| b as u8),
                mesh.0,
                texture.map(|t| t.0),
            )
        });

        // Overlays are not culled and cast no shadow: they are tools, not
        // things in the world.
        // In the order they came, not grouped by mesh: nothing tests their
        // depth, so a later one is drawn over an earlier one, and a handle
        // made of a translucent square and its edge needs the edge on top.
        let mut overlay_batches: Vec<(BatchKey, Vec<InstanceRaw>)> = Vec::new();
        for draw in &frame.overlay_draws {
            let maps = self.maps_of(draw);
            let mut raw = instance_of(draw.transform, &draw.material);
            raw.maps = packed(maps);
            push_in_order(&mut overlay_batches, (None, draw.mesh, self.batch_maps(maps)), raw);
        }
        // Outlined things, into the mask: each colour a number (the red
        // channel, out of 255), the colours themselves to the outline pass.
        let mut outline_colors: Vec<[f32; 3]> = Vec::new();
        let mut outline_batches: Vec<(BatchKey, Vec<InstanceRaw>)> = Vec::new();
        for draw in &frame.outline_draws {
            let color = draw.material.base_color;
            let index = match outline_colors.iter().position(|c| *c == color) {
                Some(i) => i,
                None if outline_colors.len() < crate::tools::OUTLINE_COLORS => {
                    outline_colors.push(color);
                    outline_colors.len() - 1
                }
                None => outline_colors.len() - 1,
            };
            let mut material = draw.material;
            material.base_color = [(index + 1) as f32 / 255.0, 0.0, 0.0];
            let mut raw = instance_of(draw.transform, &material);
            raw.maps = packed(self.maps_of(draw));
            push(
                &mut outline_batches,
                (None, draw.mesh, self.batch_maps(self.maps_of(draw))),
                raw,
            );
        }
        let sets: Vec<Maps> = shadow_batches
            .iter()
            .chain(clip_batches.iter())
            .chain(batches.iter())
            .chain(overlay_batches.iter())
            .chain(outline_batches.iter())
            .chain(
                lamp_batches
                    .iter()
                    .flat_map(|(a, b)| a.iter().chain(b.iter())),
            )
            .map(|((_, _, maps), _)| *maps)
            .chain(singles.iter().map(|single| single.2))
            .chain(transparent.iter().map(|t| t.3))
            .chain(mesh_terrain)
            .collect();
        self.prepare_maps(gpu, sets);

        // Every set shares one buffer, one after another: shadow casters,
        // batched colour draws, the single ones, the transparent ones,
        // overlays.
        let solid_casters: u32 = shadow_batches.iter().map(|(_, l)| l.len() as u32).sum();
        let shadow_total: u32 = solid_casters
            + clip_batches
                .iter()
                .map(|(_, l)| l.len() as u32)
                .sum::<u32>();
        let batched_total: u32 = batches.iter().map(|(_, l)| l.len() as u32).sum();
        // Every instance goes up in this order, written straight into the
        // queue's staging memory from where it already is: not gathered
        // into one list first, which was a copy of every instance (208
        // bytes each) a frame. Only the singles and the see-through ones,
        // which are not in lists of their own, are gathered.
        let mut loose = std::mem::take(&mut self.flat);
        loose.clear();
        loose.extend(singles.iter().map(|single| single.4));
        loose.extend(transparent.iter().map(|t| t.5));
        let parts: Vec<&[InstanceRaw]> = shadow_batches
            .iter()
            .chain(clip_batches.iter())
            .chain(batches.iter())
            .map(|(_, l)| l.as_slice())
            .chain(std::iter::once(loose.as_slice()))
            .chain(overlay_batches.iter().map(|(_, l)| l.as_slice()))
            .chain(outline_batches.iter().map(|(_, l)| l.as_slice()))
            .chain(
                lamp_batches
                    .iter()
                    .flat_map(|(a, b)| a.iter().chain(b.iter()))
                    .map(|(_, l)| l.as_slice()),
            )
            .filter(|part| !part.is_empty())
            .collect();
        let total: usize = parts.iter().map(|part| part.len()).sum();
        if total as u64 > self.instance_capacity {
            self.instance_capacity = (total as u64).next_power_of_two();
            self.instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instances"),
                size: self.instance_capacity * std::mem::size_of::<InstanceRaw>() as u64,
                // Read by the occlusion culling too.
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            // The fragment stage reads them from the frame's group.
            self.rebind(gpu);
        }
        if let Some(size) = wgpu::BufferSize::new((total * std::mem::size_of::<InstanceRaw>()) as u64) {
            let mut view = gpu
                .queue
                .write_buffer_with(&self.instances, 0, size)
                .expect("the instance buffer holds them all");
            let mut at = 0;
            for part in &parts {
                let bytes: &[u8] = bytemuck::cast_slice(part);
                view.slice(at..at + bytes.len()).copy_from_slice(bytes);
                at += bytes.len();
            }
        }
        drop(parts);
        self.flat = loose;

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
                label: Some("scrap::render"),
            });
        // What last frame's depth says is hidden, left out on the GPU —
        // where there is enough to leave out. A few hundred things are
        // cheaper drawn than culled: the pyramid, the test and the indirect
        // draws cost more than the vertices they save (a quarter of a
        // millisecond at 1080p, on the example's scenes). Dense meshes are
        // culled a cluster at a time by the same pyramid, and keep it.
        let occlusion_worth = {
            let instances: usize = batches.iter().map(|(_, list)| list.len()).sum();
            self.occlusion_always
                || instances >= OCCLUSION_FROM
                || batches
                    .iter()
                    .any(|((_, handle, _), _)| self.mesh(*handle).is_some_and(|m| m.clusters.is_some()))
        };
        {
            let culling = probe.is_none() && view.is_some() && !self.picturing && occlusion_worth;
            let mut boxes: Vec<[f32; 8]> = Vec::new();
            let mut listed: Vec<crate::occlusion::CullBatch> = Vec::new();
            if culling {
                for ((look, handle, _), list) in &batches {
                    let (min, max) = self
                        .mesh(*handle)
                        .map_or((Vec3::ZERO, Vec3::ZERO), |m| {
                            (Vec3::from_array(m.bounds.min), Vec3::from_array(m.bounds.max))
                        });
                    let never = look.is_none_or(|l| l.terrain);
                    for raw in list {
                        let m = Mat4::from_cols_array_2d(&raw.model);
                        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                        for c in 0..8 {
                            let corner = Vec3::new(
                                if c & 1 == 0 { min.x } else { max.x },
                                if c & 2 == 0 { min.y } else { max.y },
                                if c & 4 == 0 { min.z } else { max.z },
                            );
                            let w = m.transform_point3(corner);
                            lo = lo.min(w);
                            hi = hi.max(w);
                        }
                        boxes.push([lo.x, lo.y, lo.z, 0.0, hi.x, hi.y, hi.z, if never { 1.0 } else { 0.0 }]);
                    }
                    listed.push(crate::occlusion::CullBatch {
                        index_count: self.mesh(*handle).map_or(0, |m| m.index_count),
                        count: list.len() as u32,
                    });
                }
                let forward = (frame.camera.target - frame.camera.position).normalize_or(Vec3::NEG_Z);
                self.occlusion.cull(
                    gpu,
                    &mut encoder,
                    &listed,
                    &boxes,
                    &self.instances,
                    shadow_total,
                    frame.camera.position,
                    forward,
                );
                if self.occlusion.kept.buffer != self.kept_groups.0 {
                    self.rebind_kept(gpu);
                }
                // Dense meshes a cluster at a time: what can be drawn so —
                // opaque, the standard shader, neither skinned nor terrain.
                let mut jobs = Vec::new();
                let mut first = shadow_total;
                let (meshes, lod_meshes) = (&self.meshes, &self.lod_meshes);
                let mesh_of = |handle: MeshHandle| {
                    if handle.0 & LOD_HANDLE != 0 {
                        lod_meshes.get((handle.0 & !LOD_HANDLE) as usize)
                    } else {
                        meshes.get(handle.0 as usize)
                    }
                };
                for (k, ((look, handle, _), list)) in batches.iter().enumerate() {
                    let count = list.len() as u32;
                    if let (Some(look), Some(mesh)) = (look, mesh_of(*handle)) {
                        let plain = look.blend.is_none()
                            && look.shader.is_none()
                            && !look.skinned
                            && !look.terrain
                            && !look.on_top;
                        if let (true, Some(clusters)) = (plain, mesh.clusters.as_ref()) {
                            jobs.push(crate::cluster::Job {
                                batch: k,
                                first_instance: first,
                                instances: count,
                                face: look.face,
                                clusters,
                                vertices: &mesh.vertices,
                                indices: &mesh.indices,
                            });
                        }
                    }
                    first += count;
                }
                let hiz = self.occlusion.hiz();
                // Clusters at the level whose error is under a pixel
                // (crate::cluster_lod): how many pixels a metre covers a
                // metre off, or with an orthographic camera anywhere.
                let lod = match frame.camera.ortho {
                    Some(half) => [height as f32 / (2.0 * half.max(1e-3)), frame.camera.near, self.cluster_error, 1.0],
                    None => [
                        height as f32 / (2.0 * (frame.camera.fov_y_degrees.to_radians() * 0.5).tan()),
                        frame.camera.near,
                        self.cluster_error,
                        0.0,
                    ],
                };
                // Clusters past where the prepass's depth is read for
                // anything (the ambient occlusion's reach) are left out of
                // it and drawn by the lit pass alone, whose depth the
                // passes after it read — where nothing in the lit pass
                // reads the prepass's (a lean frame) and there is one
                // sample (its depth can be read).
                let split = if self.far_off_prepass && self.lean.on && screen && self.samples == 1 {
                    if frame.ambient_occlusion.enabled {
                        frame.ambient_occlusion.falloff_distance.max(1.0) * 1.2
                    } else {
                        30.0
                    }
                } else {
                    1.0e30
                };
                self.split_prepass = split < 1.0e29;
                self.clusters.cull(
                    gpu,
                    &mut encoder,
                    &jobs,
                    &self.instances,
                    drawn,
                    frame.camera.position,
                    hiz,
                    lod,
                    split,
                );
            } else {
                self.occlusion.active = false;
                self.clusters.this_frame.clear();
                self.split_prepass = false;
            }
        }
        // The scene as rays see it: every solid draw, seen or not — what is
        // behind the camera still shadows what is in front of it.
        // The irradiance volume's probes trace too, whatever else does.
        let lit_by_probes = probe.is_none() && view.is_some() && !self.picturing && self.ddgi.on();
        let traced = self.ray.is_some() && (frame.ray_tracing.any() || lit_by_probes);
        if traced {
            let meshes = &self.meshes;
            // The terrain apart from the rest (RAY_TERRAIN in render.wgsl):
            // rays start past its coarse triangles.
            let terrain = frame.terrain.as_ref().map(|t| t.mesh);
            let instances: Vec<(&wgpu::Blas, Mat4, u8, crate::ray::RayMaterial)> = frame
                .draws
                .iter()
                // Glass lets the light through, and what is unlit is a light
                // itself — a lamp's bulb must not shadow its own lamp.
                // Glass goes in apart (RAY_GLASS), for what is seen through
                // it to find its far side; water has its own way.
                .filter(|d| {
                    d.material.shading != Shading::Unlit
                        && d.material.shading != Shading::Water
                        && (!d.material.is_transparent() || frame.ray_tracing.refractions)
                })
                .filter_map(|d| {
                    let blas = meshes.get(d.mesh.0 as usize)?.blas.as_ref()?;
                    let mask = if d.material.is_transparent() {
                        RAY_GLASS
                    } else if Some(d.mesh) == terrain {
                        RAY_TERRAIN
                    } else {
                        RAY_THINGS
                    };
                    Some((blas, d.transform, mask, crate::ray::RayMaterial::of(&d.material)))
                })
                .collect();
            let remade = self
                .ray
                .as_mut()
                .is_some_and(|ray| ray.update(gpu, &mut encoder, &instances));
            if remade {
                self.rebind(gpu);
            }
        }
        // Which cascades each caster lands in: its box, seen from the sun,
        // over the cascade's square. Most of a level's casters are in one
        // or two of them — and a mountain range a kilometre off, past where
        // shadows end, in none.
        let caster_masks: Vec<u8> = shadow_batches
            .iter()
            .chain(clip_batches.iter())
            .flat_map(|((_, handle, _), list)| {
                let bounds = self.mesh(*handle).map(|m| m.bounds);
                list.iter().map(move |raw| (bounds, raw.model))
            })
            .map(|(bounds, model)| match bounds {
                Some(bounds) => caster_cascades(bounds, Mat4::from_cols_array_2d(&model), &cascades),
                None => 0,
            })
            .collect();
        for (cascade, layer) in self.shadow_layers.iter().enumerate().take(cascades.len()) {
            if kept_cascades[cascade] {
                continue;
            }
            if screen {
                self.stats.cascades_drawn += 1;
            }
            let mark = crate::frame_debugger::mark();
            {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scrap::shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: layer,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        // Kept, not discarded: the colour pass reads it.
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: crate::gpu_timer::render("shadows"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let offset = [(cascade as u64 * self.caster_stride) as u32];
            let count = |b: &[(BatchKey, Vec<InstanceRaw>)]| b.iter().map(|(_, l)| l.len() as u32).sum::<u32>();
            let bit = 1u8 << cascade;
            let (one, both) = shadow_batches.split_at(shadow_one_sided);
            pass.set_pipeline(&self.pipelines.shadow_front);
            pass.set_bind_group(0, &self.shadow_bind_group, &offset);
            self.draw_casters(&mut pass, one, 0, false, &caster_masks, bit, Some((cascades[cascade].0, cascades[cascade].3)));
            pass.set_pipeline(&self.pipelines.shadow);
            pass.set_bind_group(0, &self.shadow_bind_group, &offset);
            self.draw_casters(&mut pass, both, count(one), false, &caster_masks, bit, Some((cascades[cascade].0, cascades[cascade].3)));
            if !clip_batches.is_empty() {
                let (one, both) = clip_batches.split_at(clip_one_sided);
                pass.set_pipeline(&self.pipelines.shadow_clip_front);
                pass.set_bind_group(0, &self.shadow_bind_group, &offset);
                self.draw_casters(&mut pass, one, solid_casters, true, &caster_masks, bit, Some((cascades[cascade].0, cascades[cascade].3)));
                pass.set_pipeline(&self.pipelines.shadow_clip);
                pass.set_bind_group(0, &self.shadow_bind_group, &offset);
                self.draw_casters(&mut pass, both, solid_casters + count(one), true, &caster_masks, bit, Some((cascades[cascade].0, cascades[cascade].3)));
            }
            }
            if debugged {
                self.debugger.snapshot(gpu, &mut encoder, mark, crate::frame_debugger::Source::LinearDepth(layer));
            }
        }

        // The virtual shadow maps' pages this frame draws, each into its
        // tile of the pool, with the casters over it.
        if !vsm_jobs.is_empty() {
            let pool = &self.shadow_layers[MAX_CASCADES];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scrap::virtual shadows"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: pool,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: crate::gpu_timer::render("virtual shadows"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // Each caster's rectangle across the light, in the instance
            // buffer's order.
            let rects: Vec<Vec<[f32; 4]>> = shadow_batches
                .iter()
                .chain(clip_batches.iter())
                .map(|((_, handle, _), list)| {
                    let bounds = self.mesh(*handle).map(|m| m.bounds);
                    list.iter()
                        .map(|raw| match bounds {
                            Some(b) => {
                                let (lo, hi) = world_box(b, Mat4::from_cols_array_2d(&raw.model));
                                self.vsm.rect_of(sun, lo, hi)
                            }
                            None => [0.0; 4],
                        })
                        .collect()
                })
                .collect();
            for (i, job) in vsm_jobs.iter().enumerate() {
                let (x, y, side) = self.vsm.tile(job.physical);
                pass.set_viewport(x, y, side, side, 0.0, 1.0);
                pass.set_scissor_rect(x as u32, y as u32, side as u32, side as u32);
                pass.set_pipeline(&self.vsm.clear);
                pass.draw(0..3, 0..1);
                let offset = [self.vsm.offset(i)];
                let over = |r: &[f32; 4]| r[0] <= job.rect[1] && r[1] >= job.rect[0] && r[2] <= job.rect[3] && r[3] >= job.rect[2];
                let mut first = 0u32;
                for (b, ((_, handle, maps), list)) in shadow_batches.iter().chain(clip_batches.iter()).enumerate() {
                    let count = list.len() as u32;
                    let clipped = b >= shadow_batches.len();
                    let Some(mesh) = self.mesh(*handle) else {
                        first += count;
                        continue;
                    };
                    let mut bound = false;
                    let mut k = 0usize;
                    while k < list.len() {
                        if !over(&rects[b][k]) {
                            k += 1;
                            continue;
                        }
                        let start = k;
                        while k < list.len() && over(&rects[b][k]) {
                            k += 1;
                        }
                        if !bound {
                            pass.set_pipeline(if clipped { &self.pipelines.shadow_clip } else { &self.pipelines.shadow });
                            pass.set_bind_group(0, &self.vsm.page_group, &offset);
                            if clipped {
                                self.bind_maps(&mut pass, *maps);
                            }
                            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                            pass.set_vertex_buffer(1, self.instances.slice(..));
                            pass.set_vertex_buffer(2, self.colors_of(mesh).slice(..));
                            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                            bound = true;
                        }
                        pass.draw_indexed(0..mesh.index_count, 0, first + start as u32..first + k as u32);
                    }
                    first += count;
                }
            }
        }

        // The lamps' maps, each with the casters its own view sees, after
        // everything else in the instance buffer.
        let overlay_total: u32 = overlay_batches.iter().map(|(_, l)| l.len() as u32).sum();
        let outline_total: u32 = outline_batches.iter().map(|(_, l)| l.len() as u32).sum();
        let mut base = shadow_total
            + batched_total
            + singles.len() as u32
            + transparent.len() as u32
            + overlay_total
            + outline_total;
        for (i, (solid, clipped)) in lamp_batches.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scrap::lamp shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.light_shadow_layers[i],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: crate::gpu_timer::render("lamp shadows"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let offset = [((MAX_CASCADES + i) as u64 * self.caster_stride) as u32];
            pass.set_pipeline(&self.pipelines.shadow);
            pass.set_bind_group(0, &self.shadow_bind_group, &offset);
            self.draw_batches(&mut pass, solid, base, false);
            base += solid.iter().map(|(_, l)| l.len() as u32).sum::<u32>();
            if !clipped.is_empty() {
                pass.set_pipeline(&self.pipelines.shadow_clip);
                pass.set_bind_group(0, &self.shadow_bind_group, &offset);
                self.draw_batches(&mut pass, clipped, base, true);
                base += clipped.iter().map(|(_, l)| l.len() as u32).sum::<u32>();
            }
        }

        // The physical sky's table and aerial grid.
        if physical {
            let a = &frame.sky.atmosphere;
            self.atmosphere.run(
                gpu,
                &mut encoder,
                &crate::atmosphere::AtmosphereUniform {
                    inverse_view_projection: drawn.inverse().to_cols_array_2d(),
                    to_sun: extend(sky_to_sun, altitude),
                    eye: extend(frame.camera.position, frame.camera.far),
                    amounts: [
                        a.rayleigh,
                        a.mie,
                        a.ozone,
                        a.mie_anisotropy.clamp(0.0, 0.99),
                    ],
                    scale: [
                        a.brightness.max(0.0),
                        frame.lighting.sun_intensity,
                        a.aerial_scale.max(0.0),
                        a.ground_albedo.clamp(0.0, 1.0),
                    ],
                },
            );
        }

        // The irradiance volume's probes, lit by this frame's rays.
        if traced && lit_by_probes {
            self.ddgi.run(gpu, &mut encoder, &self.bind_group);
        }

        // The fog in the air, once every shadow it looks through is drawn.
        if volumetric.enabled {
            let simulated = self.volumes.set_smoke(gpu, &frame.smoke);
            // The smokes simulated here step once a frame, on the screen's
            // own; a probe's face or a picture sees them as they are.
            if screen && !simulated.is_empty() {
                self.smoke_sim.run(gpu, &mut encoder, &simulated, &self.volumes.smoke_storage);
            }
            self.volumes.run(
                &mut encoder,
                &self.fog_bind_group,
                &self.pipelines.fog_inject,
                &self.pipelines.fog_integrate,
            );
        }

        // Ambient occlusion: the solid things' depth and normals, and the
        // occlusion made from them, before the lit pass reads it.
        let traced_occlusion = self.ray.is_some() && frame.ray_tracing.ambient_occlusion;
        let ssao_on = frame.ambient_occlusion.enabled && !traced_occlusion;
        // The lens reads the same depth.
        let lens_on = crate::lens::LensRenderer::wanted(&frame.post);
        // Water reads it too: how deep it is below its surface.
        let water_on = frame
            .draws
            .iter()
            .any(|d| d.material.shading == Shading::Water);
        // So do screen-space reflections.
        let ssr_on = frame.screen_space_reflections.enabled && probe.is_none();
        // Bounced light reads it as well, and the last frame.
        let bounce_on = ssao_on && frame.ambient_occlusion.bounce > 0.0 && probe.is_none();
        // And the dust wall, to stand behind what is in front of it.
        let wall_on = frame.weather.dust_wall > 0.0 && probe.is_none();
        // The frame's passes as a graph (crate::graph): the prepass runs
        // when something live reads its depth or normals.
        use crate::graph::Kind;
        let mut graph = crate::graph::FrameGraph::new();
        graph.pass("prepass", Kind::Render, &[], &["depth", "normals"]);
        if ssao_on {
            graph.pass("ssao", Kind::Render, &["depth", "normals"], &["occlusion"]);
        }
        if restir_on {
            graph.pass("restir", Kind::Compute, &["depth", "normals", "rays"], &["reservoirs"]);
        }
        let mut scene_reads: Vec<&'static str> = vec!["shadow map"];
        if ssao_on {
            scene_reads.push("occlusion");
        }
        if restir_on {
            scene_reads.push("reservoirs");
        }
        // Water, the screen's reflections, dust in the air and contact
        // shadows read the depth themselves.
        if water_on || ssr_on || wall_on || local_dust || contact_on {
            scene_reads.push("depth");
        }
        graph.pass("scene", Kind::Render, &scene_reads, &["hdr"]);
        let mut picture = "hdr";
        if taa_on {
            graph.output("taa", Kind::Render, &[picture, "depth"], &["hdr antialiased"]);
            picture = "hdr antialiased";
        }
        if lens_on {
            graph.pass("lens", Kind::Render, &[picture, "depth"], &["hdr lensed"]);
            picture = "hdr lensed";
        }
        graph.output("post", Kind::Render, &[picture], &["screen"]);
        // A selection's outline tells where it is seen from where it is
        // hidden by the prepass's depth.
        if !frame.outline_draws.is_empty() && screen {
            graph.output("outline", Kind::Render, &["depth"], &["outline"]);
        }
        graph.resolve();
        debug_assert!(
            graph.problems(&["shadow map", "rays"]).is_empty(),
            "{:?}",
            graph.problems(&["shadow map", "rays"])
        );
        let prepass_drawn = graph.runs("prepass");
        if probe.is_none() && view.is_some() && !self.picturing {
            self.graph = graph;
        }
        if prepass_drawn {
            let mark = crate::frame_debugger::mark();
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("scrap::prepass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.ssao.normals,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.ssao.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: crate::gpu_timer::render("prepass"),
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                self.draw_batches_with(&mut pass, &batches, shadow_total, true, true, true);
                self.draw_mesh_terrain(&mut pass, mesh_terrain, true);
                let first = shadow_total + batched_total;
                for (instance, (look, mesh, texture, pose, _)) in (first..).zip(&singles) {
                    self.draw_single(&mut pass, *look, *mesh, *texture, *pose, instance, true);
                }
            }
            if debugged {
                self.debugger.snapshot(gpu, &mut encoder, mark, crate::frame_debugger::Source::Normals(&self.ssao.normals));
            }
            // This frame's depth into the pyramid, for the next frame's
            // culling.
            if probe.is_none() && view.is_some() && !self.picturing {
                if occlusion_worth {
                    self.occlusion.build(gpu, &mut encoder, &self.ssao.depth, (width, height), drawn);
                } else {
                    // Not made this frame: what the old one says is stale.
                    self.occlusion.forget();
                }
            }
            if ssao_on {
                let mark = crate::frame_debugger::mark();
                let now = drawn;
                self.ssao.run(
                    gpu,
                    &mut encoder,
                    now,
                    self.previous_view_projection.unwrap_or(now),
                    frame.camera.apparent_eye(),
                    &frame.ambient_occlusion,
                    (bounce_on && self.scene.has_history).then_some(&self.scene.history_view),
                    // Under TAA the bounce's rays turn each frame, and half
                    // as many do: the history adds them up.
                    (taa_run && self.taa.frames() > 0).then(|| (self.taa.frames() as f32 * 0.618_034).fract()),
                    [
                        frame.camera.near,
                        frame.camera.far,
                        if frame.camera.ortho.is_some() { 1.0 } else { 0.0 },
                        0.0,
                    ],
                );
                if debugged {
                    self.debugger.snapshot(gpu, &mut encoder, mark, crate::frame_debugger::Source::Alpha(&self.ssao.result));
                }
            }
        }

        // The clouds and the dust wall, from where the camera stands, each
        // ray stopped at what the prepass saw — so a wall of dust stands
        // behind what is in front of it.
        let dust_on = probe.is_none() && frame.weather.dust_wall > 0.0;
        let clouds_on =
            probe.is_none() && frame.sky.mode != SkyMode::Color && frame.sky.clouds.coverage > 0.0;
        if clouds_on || dust_on || local_dust {
            let (mut shape, drift) = frame.sky.clouds.vectors(&frame.wind);
            if !clouds_on {
                shape[0] = 0.0;
            }
            let w = &weather;
            let near = frame.camera.near.max(1e-3);
            self.clouds.run(
                gpu,
                &mut encoder,
                crate::clouds::CloudUniform {
                    inverse_view_projection: drawn.inverse().to_cols_array_2d(),
                    eye: extend(frame.camera.position, foliage.wind[3]),
                    to_sun: extend(to_sun, 1.0),
                    sun: extend(sun_light, 0.0),
                    ambient: extend(sky_light, 0.0),
                    shape,
                    drift,
                    // With TAA, where the march's steps fall turns each
                    // frame and the history averages it: fewer steps band
                    // no more than many.
                    size: [0.0, 0.0, if taa_run && self.taa.frames() > 0 { (self.taa.frames() as f32 * 0.618_034).fract().max(1e-3) } else { 0.0 }, 0.0],
                    dust: [
                        w.dust_wall.clamp(0.0, 1.0),
                        w.dust_front(&frame.wind, time),
                        w.dust_wall_height.max(10.0),
                        if prepass_drawn { 1.0 } else { 0.0 },
                    ],
                    view_depth: crate::lights::view_of(&frame.camera).row(2).to_array(),
                    depth_range: [
                        near,
                        frame.camera.far.max(near + 0.01),
                        foliage.wind[0],
                        foliage.wind[1],
                    ],
                    dust_box: crate::clouds::CloudUniform::dust_box(
                        frame.camera.position,
                        weather.dust_wall_height.max(10.0),
                    ),
                    local: [devils.len() as f32, plumes.len() as f32, 0.0, 0.0],
                    devils: {
                        let mut out = [[0.0; 4]; 2 * crate::volume::MOST_DEVILS];
                        for (i, d) in devils.iter().enumerate() {
                            out[2 * i] =
                                [d.position.x, d.position.y, d.position.z, d.radius.max(0.2)];
                            out[2 * i + 1] =
                                [d.height.max(1.0), d.strength.clamp(0.0, 1.0), d.spin, 0.0];
                        }
                        out
                    },
                    plumes: {
                        let mut out = [[0.0; 4]; 2 * crate::volume::MOST_PLUMES];
                        for (i, p) in plumes.iter().enumerate() {
                            out[2 * i] = [
                                p.position.x,
                                p.position.y,
                                p.position.z,
                                p.half_length.max(0.1),
                            ];
                            out[2 * i + 1] =
                                [p.along.x, p.along.y, p.along.z, p.strength.clamp(0.0, 1.0)];
                        }
                        out
                    },
                },
                &self.ssao.depth,
            );
        }

        // The lamps' reservoirs, from the prepass's depth and normals.
        if restir_on {
            self.restir.run(gpu, &mut encoder, &self.bind_group, &self.ssao.depth, &self.ssao.normals);
        }

        // Particles on the GPU given off and stepped, for the colour pass.
        if probe.is_none() {
            self.gpu_particles.run(
                gpu,
                &mut encoder,
                &frame.gpu_particles,
                drawn,
                frame.camera.apparent_eye(),
                if prepass_drawn { &self.ssao.depth } else { &self.blank_depth },
                prepass_drawn,
            );
        }
        // The prepass's depth is where the scene's starts, when they are
        // the same size and sampling: what is hidden is known before it is
        // shaded, and a pixel is shaded once — the GPU's own hidden
        // surface removal gives up on cut-outs, which discard.
        let reuse_depth = prepass_drawn && probe.is_none() && self.samples == 1 && self.ssao.size == (width, height);
        if reuse_depth {
            encoder.copy_texture_to_texture(
                self.ssao.depth.texture().as_image_copy(),
                self.depth.texture().as_image_copy(),
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.depth_prepassed = reuse_depth;
        // Unlit see-through things at half size, where it is the same
        // picture (crate::lowres).
        let halved = self.lowres.enabled
            && screen
            && self.samples == 1
            && prepass_drawn
            && !volumetric.enabled
            && !dust_on
            && !local_dust
            && transparent.iter().any(|t| halved_look(t.1));
        if screen {
            self.lowres_drawn = halved;
        }
        let scene_mark = crate::frame_debugger::mark();
        {
            // A probe's face is drawn straight into its layer.
            let face = probe.map(|layer| self.reflections.layer(layer, 0));
            let resolved = face.as_ref().unwrap_or(&self.scene.resolved);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scrap::render"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.scene.multisampled.as_ref().unwrap_or(resolved),
                    depth_slice: None,
                    resolve_target: self.scene.multisampled.as_ref().map(|_| resolved),
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
                        load: if reuse_depth {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(1.0)
                        },
                        // Kept when far clusters were left out of the prepass:
                        // what comes after reads this depth then.
                        store: if self.split_prepass { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard },
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: crate::gpu_timer::render("scene"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.draw_batches_with(&mut pass, &batches, shadow_total, true, false, true);
            self.draw_mesh_terrain(&mut pass, mesh_terrain, false);
            let mut instance = shadow_total + batched_total;
            for (look, mesh, texture, pose, _) in &singles {
                self.draw_single(&mut pass, *look, *mesh, *texture, *pose, instance, false);
                instance += 1;
            }
            // The sky last among what is solid: only where nothing was
            // drawn is it shaded at all. A plain colour needs no pass —
            // unless there is fog in the air in front of it.
            if (frame.sky.mode != SkyMode::Color || volumetric.enabled)
                && crate::frame_debugger::draw(|| crate::frame_debugger::DrawCall::fullscreen("sky"))
            {
                pass.set_pipeline(&self.pipelines.sky);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            // Farthest first, one at a time — but a run of the same thing
            // (an emitter's sprites, nearest their neighbours) in one call:
            // a call draws its instances in order, so the blend is the same.
            let mut i = 0;
            while i < transparent.len() {
                let (_, look, mesh, texture, pose, _) = &transparent[i];
                let mut run = 1;
                while !look.skinned
                    && transparent.get(i + run).is_some_and(|(_, l, m, t, _, _)| l == look && m == mesh && t == texture)
                {
                    run += 1;
                }
                if !(halved && halved_look(*look)) {
                    self.draw_run(&mut pass, *look, *mesh, *texture, *pose, instance, run as u32, false);
                }
                instance += run as u32;
                i += run;
            }
            // Particles on the GPU, among what is see-through.
            if probe.is_none()
                && self.gpu_particles.slots() > 0
                && crate::frame_debugger::draw(|| crate::frame_debugger::DrawCall {
                    what: "gpu particles".into(),
                    instances: self.gpu_particles.slots(),
                    culled_on_gpu: true,
                    pipeline: "particles".into(),
                    ..Default::default()
                })
            {
                self.gpu_particles.draw(&mut pass);
            }
            // What falls, in front of it all.
            if weather.falling() && crate::frame_debugger::draw(|| crate::frame_debugger::DrawCall::fullscreen("rain or snow")) {
                pass.set_pipeline(&self.pipelines.precipitation);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        self.depth_prepassed = false;
        // The whole depth for what comes after: the lit pass's when far
        // clusters were left out of the prepass.
        let after_depth = if self.split_prepass { &self.depth } else { &self.ssao.depth };
        if halved {
            self.lowres.prepare(gpu, (width, height));
            let low_mark = crate::frame_debugger::mark();
            {
                let mut pass = self.lowres.begin(gpu, &mut encoder, after_depth);
                let mut instance = shadow_total + batched_total + singles.len() as u32;
                let mut i = 0;
                while i < transparent.len() {
                    let (_, look, mesh, texture, pose, _) = &transparent[i];
                    let mut run = 1;
                    while !look.skinned
                        && transparent.get(i + run).is_some_and(|(_, l, m, t, _, _)| l == look && m == mesh && t == texture)
                    {
                        run += 1;
                    }
                    if halved_look(*look) {
                        self.draw_run(&mut pass, *look, *mesh, *texture, *pose, instance, run as u32, false);
                    }
                    instance += run as u32;
                    i += run;
                }
            }
            if debugged {
                if let Some(low) = self.lowres.picture() {
                    self.debugger.snapshot(gpu, &mut encoder, low_mark, crate::frame_debugger::Source::Hdr(low));
                }
            }
            self.lowres.lay_over(gpu, &mut encoder, &self.scene.resolved, after_depth);
        }
        if debugged {
            self.debugger.snapshot(gpu, &mut encoder, scene_mark, crate::frame_debugger::Source::Hdr(&self.scene.resolved));
        }

        // This frame, kept for the next one's screen-space reflections and
        // bounced light.
        if ssr_on || bounce_on {
            encoder.copy_texture_to_texture(
                self.scene.resolved_texture.as_image_copy(),
                self.scene.history.as_image_copy(),
                self.scene.resolved_texture.size(),
            );
            self.scene.has_history = true;
        }
        let Some(view) = view else {
            // A probe's face: lit, and that is all.
            gpu.queue.submit(Some(encoder.finish()));
            self.keep_batches(pool, [shadow_batches, clip_batches, batches], lamp_batches);
            return;
        };
        let view_projection = frame.camera.view_projection(aspect);
        let previous = self
            .previous_view_projection
            .replace(view_projection)
            .unwrap_or(view_projection);
        let taa_mark = crate::frame_debugger::mark();
        let picture = if taa_run {
            self.taa.run(
                gpu,
                &mut encoder,
                &self.scene.resolved,
                after_depth,
                drawn,
                previous,
            )
        } else {
            &self.scene.resolved
        };
        if debugged {
            self.debugger.snapshot(gpu, &mut encoder, taa_mark, crate::frame_debugger::Source::Hdr(picture));
        }
        let lens_mark = crate::frame_debugger::mark();
        let lensed = self.lens.run(
            gpu,
            &mut encoder,
            picture,
            after_depth,
            (width, height),
            &frame.post,
            &crate::lens::View {
                view_projection,
                previous,
                near: frame.camera.near,
                far: frame.camera.far,
                orthographic: frame.camera.ortho.is_some(),
                eye: frame.camera.position,
                time: foliage.wind[3],
            },
        );
        self.post.flares = frame
            .flares
            .iter()
            .filter_map(|f| {
                let clip = view_projection * f.position.extend(1.0);
                if clip.w <= 1e-4 {
                    return None;
                }
                let ndc = clip.truncate() / clip.w;
                let uv = glam::Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
                // A little way off the picture still throws its ghosts in.
                if uv.min_element() < -0.2 || uv.max_element() > 1.2 || ndc.z > 1.0 {
                    return None;
                }
                let hue = f.color / f.color.max_element().max(1e-4);
                Some(([uv.x, uv.y, f.intensity, 0.0], hue.extend(0.0).to_array()))
            })
            .take(crate::post::FLARES)
            .collect();
        self.post.night = frame.lighting.night;
        self.post.fov_y_degrees = match frame.camera.ortho {
            Some(_) => 0.0,
            None => frame.camera.fov_y_degrees,
        };
        // The eye's clock: the frame's own time where it says one.
        let metered = (!self.picturing).then(|| {
            let now = frame
                .time
                .unwrap_or_else(|| self.started.elapsed().as_secs_f32());
            let dt = self.metered_at.map_or(0.0, |was| (now - was).max(0.0));
            self.metered_at = Some(now);
            dt
        });
        // At night the eye does not get used to the dark the whole way:
        // a moonlit desert stays a night. The shade of a passage by day it
        // opens up to by stops; the night it hardly does.
        let mut post = frame.post;
        let night = frame.lighting.night.clamp(0.0, 1.0);
        post.auto_exposure.compensation -= 1.6 * night;
        let most = post.auto_exposure.max_ev;
        post.auto_exposure.max_ev = most + (most.min(1.0) - most) * night;
        if let (true, Some(lensed)) = (debugged, lensed) {
            self.debugger.snapshot(gpu, &mut encoder, lens_mark, crate::frame_debugger::Source::Hdr(lensed));
        }
        let picture = lensed.unwrap_or(picture);
        let picture = if upscale_on {
            self.upscaler.run(
                gpu,
                &mut encoder,
                &upscaling,
                temporal,
                picture,
                after_depth,
                (width, height),
                output,
                [drawn, view_projection, previous],
                jitter,
            )
        } else {
            picture
        };
        self.post.run(gpu, &mut encoder, picture, view, output, &post, metered);

        // Tools go on the finished picture: no tonemapper, bloom or
        // vignette touches a handle's colour.
        if !overlay_batches.is_empty() || !outline_batches.is_empty() {
            let base =
                shadow_total + batched_total + singles.len() as u32 + transparent.len() as u32;
            self.draw_tools(
                gpu,
                &mut encoder,
                view,
                output,
                &overlay_batches,
                &outline_batches,
                &outline_colors,
                base,
                base + overlay_total,
                frame.outline_width,
                drawn,
            );
        }
        if scaling {
            let ms = self.timer.as_ref().and_then(|t| t.frame_ms());
            self.upscaler.adjust(&upscaling, ms);
        }
        if debugged {
            self.debugger.captured = crate::frame_debugger::finish(output);
        }
        let timer = self.timer.as_mut().filter(|_| timed);
        match timer {
            Some(timer) => {
                timer.end(&mut encoder);
                gpu.queue.submit(Some(encoder.finish()));
                timer.submitted();
            }
            None => {
                gpu.queue.submit(Some(encoder.finish()));
            }
        }
        self.keep_batches(pool, [shadow_batches, clip_batches, batches], lamp_batches);
    }

    /// This view's batch lists into the pool for the next (see
    /// `batch_pool`): what was not used again is let go.
    fn keep_batches(&mut self, old: BatchPool, lists: [Batches; 3], lamps: Vec<(Batches, Batches)>) {
        drop(old);
        let mut pool = BatchPool::default();
        let lamps = lamps.into_iter().enumerate().flat_map(|(i, (solid, clipped))| {
            let tag = 3 + 2 * i as u32;
            [(tag, solid), (tag + 1, clipped)]
        });
        for (tag, list) in (0u32..).zip(lists).chain(lamps) {
            for (key, mut instances) in list {
                instances.clear();
                pool.insert((tag, key), instances);
            }
        }
        self.batch_pool = pool;
    }
}

/// Draws grouped by what they share, each group's instances in order.
type Batches = Vec<(BatchKey, Vec<InstanceRaw>)>;

/// The renderer's meshes, levels of detail and textures, as preparing a
/// draw reads them ([`Renderer::lookup`]).
#[derive(Clone, Copy)]
struct DrawLookup<'a> {
    meshes: &'a [GpuMesh],
    lods: &'a scrap_core::hash::FastMap<u32, Vec<(MeshHandle, f32)>>,
    looks: &'a scrap_core::hash::FastMap<MeshHandle, TextureHandle>,
    by_asset: &'a scrap_core::hash::FastMap<crate::asset::AssetId, TextureHandle>,
    shader_textures: &'a scrap_core::hash::FastMap<crate::asset::AssetId, Vec<String>>,
}

impl DrawLookup<'_> {
    /// The maps a draw takes: its material's four, where uploaded, else
    /// the draw's own texture for the colour and neutral ones for the rest
    /// — a missing map leaves a plain surface, not a hole. Then its own
    /// shader's slots: slot `i` the material's texture named by the
    /// shader's `i`th `// scrap:textures` name, white where it has none
    /// or it is not uploaded.
    fn maps_of(&self, draw: &Draw) -> Maps {
        let m = &draw.material;
        let find = |id: Option<crate::asset::AssetId>| id.and_then(|id| self.by_asset.get(&id).copied());
        // No map of its own and no texture on the entity: the model's look.
        let own = match self.looks.get(&draw.mesh) {
            Some(look) if draw.texture == TextureHandle::WHITE => *look,
            _ => draw.texture,
        };
        let mut slots = [TextureHandle::WHITE; MATERIAL_TEXTURES];
        // Only a material with textures and a shader that reads some
        // looks its set up; every other draw is done here.
        if let (false, Some(names)) = (
            m.textures.is_empty(),
            m.shader.and_then(|s| self.shader_textures.get(&s)),
        ) {
            let entries = m.textures.entries();
            for (slot, name) in slots.iter_mut().zip(names) {
                let id = entries.iter().find(|e| e.name == *name).map(|e| e.texture);
                *slot = find(id).unwrap_or(TextureHandle::WHITE);
            }
        }
        [
            find(m.base_map).unwrap_or(own),
            find(m.normal_map).unwrap_or(TextureHandle::FLAT_NORMAL),
            find(m.mask_map).unwrap_or(TextureHandle::WHITE),
            find(m.emission_map).unwrap_or(TextureHandle::WHITE),
            slots[0],
            slots[1],
            slots[2],
            slots[3],
        ]
    }

    /// Which level of its mesh a draw is drawn with, from how much of the
    /// screen it covers — `None` when it is too small to draw at all. A
    /// skinned mesh keeps its own: its joints are bound to its vertices.
    /// How much of the screen's height a draw covers.
    fn coverage(&self, draw: &Draw, eye: Vec3, camera: &Camera) -> f32 {
        let Some(base) = self.meshes.get(draw.mesh.0 as usize) else {
            return 0.0;
        };
        let (min, max) = (Vec3::from_array(base.bounds.min), Vec3::from_array(base.bounds.max));
        let (scale, _, _) = draw.transform.to_scale_rotation_translation();
        let centre = draw.transform.transform_point3((min + max) * 0.5);
        let radius = ((max - min) * scale.abs()).length() * 0.5;
        match camera.ortho {
            Some(half) => radius / half.max(1e-3),
            None => crate::lod::coverage(radius, centre.distance(eye), camera.fov_y_degrees),
        }
    }

    fn level_of(&self, draw: &Draw, eye: Vec3, camera: &Camera, skinned: bool) -> Option<MeshHandle> {
        let base = self.meshes.get(draw.mesh.0 as usize)?;
        let (min, max) = (Vec3::from_array(base.bounds.min), Vec3::from_array(base.bounds.max));
        let (scale, _, _) = draw.transform.to_scale_rotation_translation();
        let centre = draw.transform.transform_point3((min + max) * 0.5);
        let radius = ((max - min) * scale.abs()).length() * 0.5;
        let covers = match camera.ortho {
            Some(half) => radius / half.max(1e-3),
            None => crate::lod::coverage(radius, centre.distance(eye), camera.fov_y_degrees),
        };
        if covers < crate::lod::TOO_SMALL {
            return None;
        }
        if skinned {
            return Some(draw.mesh);
        }
        let mut pick = draw.mesh;
        if let Some(levels) = self.lods.get(&draw.mesh.0) {
            for (handle, below) in levels {
                if covers < *below {
                    pick = *handle;
                }
            }
        }
        Some(pick)
    }
}

/// A draw, as the frame's preparing on every core works it out.
struct Prepared {
    raw: InstanceRaw,
    maps: Maps,
    skinned: bool,
    level: Option<MeshHandle>,
    visible: bool,
    /// The share of the screen's height it covers.
    covers: f32,
}

/// Where each batch of a list is, by its key: thousands of draws into
/// hundreds of batches without looking through them all each time.
/// The last batch pushed to is looked at first: draws of one thing come
/// together, and then no key is hashed at all.
#[derive(Default)]
struct BatchIndex {
    at: scrap_core::hash::FastMap<BatchKey, usize>,
    last: Option<(BatchKey, usize)>,
    /// Which list it indexes, for the pool.
    tag: u32,
}

/// Instance buffers of the last view's batches, by list and key.
type BatchPool = scrap_core::hash::FastMap<(u32, BatchKey), Vec<InstanceRaw>>;

impl BatchIndex {
    fn tagged(tag: u32) -> Self {
        BatchIndex { tag, ..Default::default() }
    }

    fn push(&mut self, pool: &mut BatchPool, batches: &mut Batches, key: BatchKey, raw: InstanceRaw) {
        if let Some((last, at)) = self.last {
            if last == key {
                batches[at].1.push(raw);
                return;
            }
        }
        let at = match self.at.get(&key) {
            Some(&at) => at,
            None => {
                self.at.insert(key, batches.len());
                batches.push((key, pool.remove(&(self.tag, key)).unwrap_or_default()));
                batches.len() - 1
            }
        };
        batches[at].1.push(raw);
        self.last = Some((key, at));
    }
}

/// Two areas of the screen as one that holds both.
fn union(a: Option<[u32; 4]>, b: Option<[u32; 4]>) -> Option<[u32; 4]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]),
        (a, b) => a.or(b),
    }
}

/// Onto the last batch when it has the same key, else a new one: batches
/// that keep the order the draws came in.
fn push_in_order(batches: &mut Batches, key: BatchKey, raw: InstanceRaw) {
    match batches.last_mut() {
        Some((k, list)) if *k == key => list.push(raw),
        _ => batches.push((key, vec![raw])),
    }
}

fn push(batches: &mut Batches, key: BatchKey, raw: InstanceRaw) {
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
/// A mesh's box, placed: the world box round its eight corners.
fn world_box(bounds: crate::asset::Bounds, transform: Mat4) -> (Vec3, Vec3) {
    // The box round all eight corners, without the eight (Arvo): the
    // centre carried over, and each axis of the box reaching as far along
    // the world's axes as its turned and scaled column says. Run for every
    // draw several times a frame; the same box as the corners give, to the
    // last bits of rounding.
    let (lo, hi) = (Vec3::from_array(bounds.min), Vec3::from_array(bounds.max));
    let centre = transform.transform_point3((lo + hi) * 0.5);
    let half = (hi - lo) * 0.5;
    let reach = transform.x_axis.truncate().abs() * half.x
        + transform.y_axis.truncate().abs() * half.y
        + transform.z_axis.truncate().abs() * half.z;
    (centre - reach, centre + reach)
}

/// The cascades (a bit each) a caster's box lands in, seen from the sun:
/// its corners in the cascade's clip space overlapping the square, and not
/// all past its far end. What is between the sun and the square is kept —
/// it casts onto it.
fn caster_cascades(bounds: crate::asset::Bounds, transform: Mat4, cascades: &[(Mat4, Vec3, f32, f32, f32)]) -> u8 {
    let (lo, hi) = (Vec3::from_array(bounds.min), Vec3::from_array(bounds.max));
    let corners: [Vec3; 8] = std::array::from_fn(|i| {
        transform.transform_point3(Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        ))
    });
    let mut mask = 0u8;
    for (i, (matrix, ..)) in cascades.iter().enumerate().take(8) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for c in &corners {
            let p = matrix.project_point3(*c);
            min = min.min(p);
            max = max.max(p);
        }
        if max.x >= -1.0 && min.x <= 1.0 && max.y >= -1.0 && min.y <= 1.0 && min.z <= 1.0 {
            mask |= 1 << i;
        }
    }
    mask
}

/// The screen's cascades as their maps were last drawn: what a frame that
/// does not draw a far one again reads it by.
struct CascadeCache {
    settings: ShadowSettings,
    resolution: u32,
    sun: Vec3,
    views: Vec<(Mat4, Vec3, f32, f32, f32)>,
}

/// A lean pipeline ([`crate::lean`]): a look's, over the prepass's depth
/// or not; or one drawing culled clusters of a face, water or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LeanKey {
    Scene(Look, bool),
    Cluster(RenderFace, bool),
}

/// Whether a see-through look is drawn at half size when the frame allows
/// (crate::lowres): unlit, not multiplying what is behind it, not drawn
/// over everything, not skinned.
fn halved_look(look: Look) -> bool {
    look.unlit && look.blend.is_some_and(|b| b != Blend::Multiply) && !look.on_top && !look.skinned
}

/// Whether a frame may be drawn by the lean shaders (`LEAN`, render.wgsl):
/// every switch of what they leave out is off in its uniform, and no draw
/// is of sand, clay or a surface lit from under its skin — what the lit
/// shader does whatever the weather.
fn lean_allowed(u: &FrameUniform, frame: &Frame, no_decals: bool) -> bool {
    let off = |v: f32| v <= 0.5;
    let calm = u.weather[0] == [0.0; 4]
        && u.weather[1][1] <= 0.0
        && u.weather[1][2] <= 0.0
        // Drift, drying and mud; the mud's height (w) means nothing
        // without mud.
        && u.weather[2][..3] == [0.0; 3]
        && off(u.dust[0]);
    let clear = u.clouds[0][0] <= 0.0 || u.clouds[1][3] <= 0.0;
    let dry = (0..4).all(|i| off(u.waters[i * 2][1]));
    let plain = u.ambient_occlusion[2] <= 0.0
        && off(u.vsm[3][0])
        && off(u.ddgi[1][3])
        && off(u.ssr[0])
        && off(u.air[0])
        && off(u.volume[0])
        && u.probe_params[0] == 0.0
        && off(u.probe_params[2])
        && off(u.distance[0][3])
        && off(u.ray[0])
        && off(u.ray[1])
        && off(u.ray[2])
        && off(u.restir[0])
        && off(u.glass[0]);
    let materials = frame.draws.iter().all(|d| {
        let m = &d.material;
        m.shading != Shading::Sand && !m.clay && m.subsurface == [0.0; 3]
    });
    if std::env::var_os("SCRAP_LEAN_WHY").is_some() {
        eprintln!(
            "lean: calm {calm} clear {clear} dry {dry} plain {plain} decals {no_decals} materials {materials}; weather {:?} dust {:?} ao {:?} vsm {:?} ddgi {:?} ssr {:?} air {:?} volume {:?} probes {:?} distance {:?} ray {:?} restir {:?} glass {:?}",
            u.weather, u.dust, u.ambient_occlusion, u.vsm[3], u.ddgi[1], u.ssr, u.air, u.volume, u.probe_params, u.distance[0], u.ray, u.restir, u.glass
        );
    }
    calm && clear && dry && plain && no_decals && materials
}

fn aabb_in_frustum(
    planes: &[glam::Vec4; 6],
    bounds: crate::asset::Bounds,
    transform: Mat4,
) -> bool {
    let (min, max) = world_box(bounds, transform);
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

/// Everything the frame's bind group holds.
struct FrameInputs<'a> {
    frame: &'a wgpu::Buffer,
    shadow_map: &'a wgpu::TextureView,
    shadow_sampler: &'a wgpu::Sampler,
    occlusion: &'a wgpu::TextureView,
    rays: Option<(&'a wgpu::Tlas, &'a wgpu::Buffer)>,
    lights: &'a wgpu::Buffer,
    cells: &'a wgpu::Buffer,
    indices: &'a wgpu::Buffer,
    light_views: &'a wgpu::Buffer,
    light_shadow_map: &'a wgpu::TextureView,
    probes: &'a wgpu::TextureView,
    probe_sampler: &'a wgpu::Sampler,
    decals: &'a wgpu::Buffer,
    decal_colours: &'a wgpu::TextureView,
    decal_normals: &'a wgpu::TextureView,
    fog: &'a wgpu::TextureView,
    fog_sampler: &'a wgpu::Sampler,
    sky_view: &'a wgpu::TextureView,
    aerial: &'a wgpu::TextureView,
    scene_depth: &'a wgpu::TextureView,
    clouds: &'a wgpu::TextureView,
    history: &'a wgpu::TextureView,
    /// Terrain heights, for the vertex shader.
    terrain_heights: &'a wgpu::TextureView,
    /// The irradiance volume's probes.
    ddgi: &'a wgpu::Buffer,
    /// The virtual shadow maps' page table.
    vsm_pages: &'a wgpu::Buffer,
    /// ReSTIR's reservoirs.
    restir: &'a wgpu::Buffer,
    /// How the grass is trampled, for the vertex shader.
    trample: &'a wgpu::TextureView,
    /// The scene's distance field.
    distance: &'a wgpu::TextureView,
    /// The frame's instances.
    instances: &'a wgpu::Buffer,
}

/// A terrain's heights as a texture of `side`² floats, read texel by
/// texel by the vertex shader (no filtering asked of the device).
fn terrain_height_view(gpu: &Gpu, side: u32, heights: &[f32]) -> wgpu::TextureView {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("terrain heights"),
        size: wgpu::Extent3d {
            width: side.max(1),
            height: side.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(heights),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(side.max(1) * 4),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: side.max(1),
            height: side.max(1),
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn frame_bind_group(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    inputs: &FrameInputs,
) -> wgpu::BindGroup {
    fn view(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        }
    }
    fn buffer(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: buffer.as_entire_binding(),
        }
    }
    let mut entries = vec![
        buffer(0, inputs.frame),
        view(1, inputs.shadow_map),
        wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::Sampler(inputs.shadow_sampler),
        },
        view(4, inputs.occlusion),
        buffer(6, inputs.lights),
        buffer(25, inputs.ddgi),
        buffer(26, inputs.vsm_pages),
        buffer(27, inputs.restir),
        buffer(7, inputs.cells),
        buffer(8, inputs.indices),
        view(9, inputs.light_shadow_map),
        buffer(10, inputs.light_views),
        view(11, inputs.probes),
        wgpu::BindGroupEntry {
            binding: 12,
            resource: wgpu::BindingResource::Sampler(inputs.probe_sampler),
        },
        buffer(13, inputs.decals),
        view(14, inputs.decal_colours),
        view(15, inputs.decal_normals),
        view(16, inputs.fog),
        wgpu::BindGroupEntry {
            binding: 17,
            resource: wgpu::BindingResource::Sampler(inputs.fog_sampler),
        },
        view(18, inputs.sky_view),
        view(19, inputs.aerial),
        view(20, inputs.scene_depth),
        view(21, inputs.clouds),
        view(22, inputs.history),
        view(23, inputs.terrain_heights),
        view(30, inputs.trample),
        view(31, inputs.distance),
        buffer(33, inputs.instances),
    ];
    if let Some((rays, materials)) = inputs.rays {
        entries.push(wgpu::BindGroupEntry {
            binding: 5,
            resource: rays.as_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 24,
            resource: materials.as_entire_binding(),
        });
    }
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("frame"),
        layout,
        entries: &entries,
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

/// The cascades' maps: one texture of [`MAX_CASCADES`] layers, viewed whole
/// to sample and a layer at a time to draw into.
fn shadow_view(gpu: &Gpu, resolution: u32) -> (wgpu::TextureView, Vec<wgpu::TextureView>) {
    // And one more: the virtual shadow maps' pool of pages.
    shadow_layers_view(gpu, resolution, MAX_CASCADES as u32 + 1)
}

/// `layers` depth maps in one array: viewed whole, and each on its own.
fn shadow_layers_view(
    gpu: &Gpu,
    resolution: u32,
    layers: u32,
) -> (wgpu::TextureView, Vec<wgpu::TextureView>) {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow map"),
        size: wgpu::Extent3d {
            width: resolution.max(1),
            height: resolution.max(1),
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let whole = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let layers = (0..layers)
        .map(|layer| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    (whole, layers)
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
        // One sample a pixel: the prepass's depth is copied in to start from.
        usage: if samples == 1 {
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING
        } else {
            wgpu::TextureUsages::RENDER_ATTACHMENT
        },
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_shadow_fades_over_the_last_border_of_its_distance_as_urps() {
        let (scale, bias) = super::shadow_fade(50.0, 0.1);
        let fade = |d: f32| (d * d * scale + bias).clamp(0.0, 1.0);
        // Squared: 90% of the way out is 81% of the squared distance.
        assert!(fade(0.9 * 50.0) < 1e-4);
        assert!((fade(50.0) - 1.0).abs() < 1e-5);
        assert!(fade(47.5) > 0.4 && fade(47.5) < 0.6, "{}", fade(47.5));
    }

    #[test]
    fn a_lights_temperature_is_unitys_black_body() {
        let warm = crate::look::color_temperature(4996.0);
        assert!((warm - glam::Vec3::new(1.0, 0.790, 0.628)).abs().max_element() < 0.01, "{warm}");
        // Unity's default 6570 K is all but white.
        let default = crate::look::color_temperature(6570.0);
        assert!(default.min_element() > 0.93, "{default}");
    }

    use super::*;

    #[test]
    fn a_shader_graph_of_every_kind_of_node_builds_over_the_standard_shader() {
        let graph = scrap_shadergraph::surface::parse(
            r#"(
                params: ["speed", "glow", "tint_r"],
                textures: ["_Main", "_Noise"],
                nodes: {
                    "t": Multiply(a: "time", b: "speed"),
                    "moved": TilingOffset(tiling: (2.0, 3.0), offset: "t"),
                    "turned": Rotate(uv: "moved", angle: "t"),
                    "main": Texture(name: "_Main", uv: "turned"),
                    "grain": Texture(name: "_Noise"),
                    "n2": Noise(at: "uv", scale: 4.0),
                    "n3": Noise(at: "position", scale: 0.5),
                    "cells": Voronoi(scale: 6.0),
                    "check": Checker(scale: 8.0),
                    "rim": Fresnel(power: 3.0),
                    "a": Add(a: "n2", b: "n3"),
                    "s": Subtract(a: "a", b: 0.5),
                    "d": Divide(a: "s", b: 2.0),
                    "p": Power(a: "d", b: 2.0),
                    "lo": Min(a: "p", b: "cells.y"),
                    "hi": Max(a: "lo", b: "check"),
                    "m": Modulo(a: "hi", b: 0.3),
                    "st": Step(edge: 0.5, of: "m"),
                    "dt": Dot(a: "normal", b: "view"),
                    "ds": Distance(a: "position", b: (0.0, 1.0, 0.0)),
                    "cr": Cross(a: "normal", b: "view"),
                    "neg": Negate(of: "dt"),
                    "om": OneMinus(of: "neg"),
                    "ab": Abs(of: "om"),
                    "fl": Floor(of: "ab"),
                    "ce": Ceil(of: "fl"),
                    "ro": Round(of: "ce"),
                    "fr": Fract(of: "ds"),
                    "sg": Sign(of: "fr"),
                    "si": Sine(of: "t"),
                    "co": Cosine(of: "si"),
                    "sq": Sqrt(of: "co"),
                    "ex": Exp(of: "sq"),
                    "sa": Saturate(of: "ex"),
                    "le": Length(of: "cr"),
                    "no": Normalize(of: "cr"),
                    "tu": Turbulence(scale: 2.0),
                    "bent": Add(a: "no", b: "tu"),
                    "mix": Lerp(a: (0.1, 0.2, 0.3), b: "main.rgb", t: "sa"),
                    "cl": Clamp(of: "mix", low: 0.0, high: 2.0),
                    "sm": Smoothstep(low: 0.2, high: 0.8, of: "st"),
                    "rm": Remap(of: "sm", from: (0.0, 1.0), to: (-1.0, 1.0)),
                    "po": Posterize(of: "rm", steps: 4.0),
                    "cb": Combine(x: "po", y: "tint_r", z: "sg", w: "ro"),
                    "tint": Multiply(a: "cl", b: "cb.rgb"),
                    "glowing": Multiply(a: "rim", b: "glow"),
                    "rc": Reciprocal(of: 2.0),
                    "lg": Log(of: 2.0),
                    "tr": Truncate(of: "t"),
                    "tn": Tangent(of: "t"),
                    "asn": Arcsin(of: 0.5),
                    "acs": Arccos(of: 0.5),
                    "atn": Arctan(of: "t"),
                    "at2": Arctan2(y: "t", x: 1.0),
                    "rad": Radians(of: 90.0),
                    "deg": Degrees(of: 1.0),
                    "il": InverseLerp(a: 0.0, b: 2.0, of: "t"),
                    "rr": RandomRange(seed: "uv"),
                    "cmp": Comparison(a: "t", b: 0.5, op: Greater),
                    "br": Branch(when: "t", yes: 1.0, no: 0.0),
                    "an": And(a: "t", b: "cmp"),
                    "orr": Or(a: "t", b: "cmp"),
                    "nt": Not(of: "t"),
                    "dx": Ddx(of: "t"),
                    "dy": Ddy(of: "t"),
                    "fw": Fwidth(of: "t"),
                    "sm2": SphereMask(at: "position", center: (0.0, 0.0, 0.0)),
                    "lum": Luminance(of: "albedo"),
                    "el": Ellipse(),
                    "rect": Rectangle(),
                    "rrect": RoundedRectangle(),
                    "poly": Polygon(sides: 5.0),
                    "sn": SimpleNoise(),
                    "rfl": Reflect(incident: "view"),
                    "rfr": Refract(incident: "view", ratio: 0.7),
                    "prj": Project(of: "normal", onto: (0.0, 1.0, 0.0)),
                    "rej": Reject(of: "normal", onto: (0.0, 1.0, 0.0)),
                    "rot": RotateAboutAxis(of: "normal", angle: "t"),
                    "bl": Blend(base: "albedo", blend: (0.5, 0.5, 0.5), mode: Screen),
                    "hue": Hue(of: "albedo", offset: 0.1),
                    "sat": Saturation(of: "albedo", amount: 0.5),
                    "con": Contrast(of: "albedo", amount: 1.2),
                    "inv": Invert(of: "albedo"),
                    "cm": ChannelMixer(of: "albedo"),
                    "rpc": ReplaceColor(of: "albedo", from: (1.0, 0.0, 0.0), to: (0.0, 1.0, 0.0), range: 0.1, fuzziness: 0.1),
                    "hsv": RgbToHsv(of: "albedo"),
                    "rgb": HsvToRgb(of: "hsv"),
                    "srgb": LinearToSrgb(of: "albedo"),
                    "lin": SrgbToLinear(of: "srgb"),
                    "grad": Gradient(t: "t", keys: [(0.0, (0.0, 0.0, 0.0)), (0.5, (1.0, 0.0, 0.0)), (1.0, (1.0, 1.0, 1.0))]),
                    "ns": NormalStrength(of: "normal", strength: 0.5),
                    "nb": NormalBlend(a: "normal", b: "ns"),
                    "nh": NormalFromHeight(height: "n2"),
                    "nft": NormalFromTexture(name: "_Noise"),
                    "scol": SceneColor(),
                    "tri": Triplanar(name: "_Noise"),
                    "fb": Flipbook(columns: 4.0, rows: 4.0, frame: "t"),
                    "pol": PolarCoordinates(),
                    "tw": Twirl(),
                    "sph": Spherize(),
                    "rsh": RadialShear(),
                    "sum0": Add(a: "rc", b: "lg"),
                    "sum1": Add(a: "sum0", b: "tr"),
                    "sum2": Add(a: "sum1", b: "tn"),
                    "sum3": Add(a: "sum2", b: "asn"),
                    "sum4": Add(a: "sum3", b: "acs"),
                    "sum5": Add(a: "sum4", b: "atn"),
                    "sum6": Add(a: "sum5", b: "at2"),
                    "sum7": Add(a: "sum6", b: "rad"),
                    "sum8": Add(a: "sum7", b: "deg"),
                    "sum9": Add(a: "sum8", b: "il"),
                    "sum10": Add(a: "sum9", b: "rr"),
                    "sum11": Add(a: "sum10", b: "cmp"),
                    "sum12": Add(a: "sum11", b: "br"),
                    "sum13": Add(a: "sum12", b: "an"),
                    "sum14": Add(a: "sum13", b: "orr"),
                    "sum15": Add(a: "sum14", b: "nt"),
                    "sum16": Add(a: "sum15", b: "dx"),
                    "sum17": Add(a: "sum16", b: "dy"),
                    "sum18": Add(a: "sum17", b: "fw"),
                    "sum19": Add(a: "sum18", b: "sm2"),
                    "sum20": Add(a: "sum19", b: "lum"),
                    "sum21": Add(a: "sum20", b: "el"),
                    "sum22": Add(a: "sum21", b: "rect"),
                    "sum23": Add(a: "sum22", b: "rrect"),
                    "sum24": Add(a: "sum23", b: "poly"),
                    "sum25": Add(a: "sum24", b: "sn"),
                    "sum26": Add(a: "sum25", b: "rfl.x"),
                    "sum27": Add(a: "sum26", b: "rfr.x"),
                    "sum28": Add(a: "sum27", b: "prj.x"),
                    "sum29": Add(a: "sum28", b: "rej.x"),
                    "sum30": Add(a: "sum29", b: "rot.x"),
                    "sum31": Add(a: "sum30", b: "bl.x"),
                    "sum32": Add(a: "sum31", b: "hue.x"),
                    "sum33": Add(a: "sum32", b: "sat.x"),
                    "sum34": Add(a: "sum33", b: "con.x"),
                    "sum35": Add(a: "sum34", b: "inv.x"),
                    "sum36": Add(a: "sum35", b: "cm.x"),
                    "sum37": Add(a: "sum36", b: "rpc.x"),
                    "sum38": Add(a: "sum37", b: "hsv.x"),
                    "sum39": Add(a: "sum38", b: "rgb.x"),
                    "sum40": Add(a: "sum39", b: "srgb.x"),
                    "sum41": Add(a: "sum40", b: "lin.x"),
                    "sum42": Add(a: "sum41", b: "grad.x"),
                    "sum43": Add(a: "sum42", b: "ns.x"),
                    "sum44": Add(a: "sum43", b: "nb.x"),
                    "sum45": Add(a: "sum44", b: "nh.x"),
                    "sum46": Add(a: "sum45", b: "nft.x"),
                    "sum47": Add(a: "sum46", b: "scol.x"),
                    "sum48": Add(a: "sum47", b: "tri.x"),
                    "sum49": Add(a: "sum48", b: "fb.x"),
                    "sum50": Add(a: "sum49", b: "pol.x"),
                    "sum51": Add(a: "sum50", b: "tw.x"),
                    "sum52": Add(a: "sum51", b: "sph.x"),
                    "sum53": Add(a: "sum52", b: "rsh.x"),
                    "sum54": Add(a: "sum53", b: "screen.x"),
                    "sum55": Add(a: "sum54", b: "depth"),
                    "sum56": Add(a: "sum55", b: "scene_depth"),
                    "sum57": Add(a: "sum56", b: "front"),
                    "sum58": Add(a: "sum57", b: "vertex_color.a"),
                    "sum59": Add(a: "sum58", b: "tangent.x"),
                    "sum60": Add(a: "sum59", b: "bitangent.x"),
                    "sum61": Add(a: "sum60", b: "camera.y"),
                    "sum62": Add(a: "sum61", b: "light_direction.y"),
                    "sum63": Add(a: "sum62", b: "light_color.r"),
                    "sum64": Add(a: "sum63", b: "ambient.g"),
                    "glowing_more": Add(a: "glowing", b: "sum64"),
                },
                surface: (
                    albedo: "tint",
                    alpha: "grain.a",
                    metallic: "le",
                    smoothness: 0.5,
                    normal: "bent",
                    emission: "glowing_more",
                    clip: 0.1,
                ),
            )"#,
        )
        .unwrap();
        let kinds: std::collections::BTreeSet<&str> = graph.nodes.values().map(|n| n.kind()).collect();
        // All but `Random`, which a surface has nothing to be random for.
        assert_eq!(kinds.len(), scrap_shadergraph::Node::KINDS.len() - 1, "every kind of node is in the graph: {kinds:?}");
        let wgsl = scrap_shadergraph::surface::to_wgsl(&graph, "shaders/every.graph.ron").unwrap();
        assert!(wgsl.starts_with("// Made from shaders/every.graph.ron"), "{wgsl}");
        assert!(wgsl.contains("// scrap:params speed glow tint_r"), "{wgsl}");
        assert_eq!(declared_textures(&wgsl), vec!["_Main".to_string(), "_Noise".to_string()]);
        check_material_shader(&wgsl).unwrap_or_else(|e| panic!("{e}\n{wgsl}"));
        assert!(scrap_shadergraph::surface::problems(&graph).is_empty());
    }

    #[test]
    fn a_world_box_is_the_box_round_its_eight_corners() {
        let bounds = crate::asset::Bounds { min: [-0.3, -1.0, 0.2], max: [0.7, 2.0, 0.9] };
        for transform in [
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(3.0, -2.0, 10.0)),
            Mat4::from_scale_rotation_translation(
                Vec3::new(2.0, 0.5, 3.0),
                glam::Quat::from_euler(glam::EulerRot::YXZ, 0.7, -1.2, 0.3),
                Vec3::new(-40.0, 5.0, 12.5),
            ),
            Mat4::from_cols_array(&[1.0, 0.2, 0.0, 0.0, -0.4, 1.5, 0.3, 0.0, 0.0, 0.1, -2.0, 0.0, 7.0, 8.0, 9.0, 1.0]),
        ] {
            let (lo, hi) = (Vec3::from_array(bounds.min), Vec3::from_array(bounds.max));
            let (mut min, mut max) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
            for i in 0..8 {
                let corner = Vec3::new(
                    if i & 1 == 0 { lo.x } else { hi.x },
                    if i & 2 == 0 { lo.y } else { hi.y },
                    if i & 4 == 0 { lo.z } else { hi.z },
                );
                let p = transform.transform_point3(corner);
                min = min.min(p);
                max = max.max(p);
            }
            let (a, b) = world_box(bounds, transform);
            assert!(a.abs_diff_eq(min, 1e-4) && b.abs_diff_eq(max, 1e-4), "{a} {b} vs {min} {max}");
        }
    }

    #[test]
    fn a_shader_names_its_texture_slots_on_a_line_of_its_own() {
        let shader = "// Road.\n// scrap:params _Speed\n  // scrap:textures _Road _Noise _A _B _Past\nfn surface() {}";
        assert_eq!(declared_textures(shader), ["_Road", "_Noise", "_A", "_B"], "four at most");
        assert!(declared_textures("fn surface() {}").is_empty());
    }

    #[test]
    fn a_thing_placed_mirrored_says_its_faces_are_turned() {
        let flags = |t: Mat4| instance_of(t, &Material::default()).emission[3] as u32;
        assert_eq!(flags(Mat4::IDENTITY) & FLAG_INSIDE_OUT, 0);
        let mirrored = Mat4::from_scale(Vec3::new(-1.5, 1.5, 1.5));
        assert_ne!(flags(mirrored) & FLAG_INSIDE_OUT, 0, "a right hand from a left one");
        let twice = Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
        assert_eq!(flags(twice) & FLAG_INSIDE_OUT, 0, "mirrored twice is a turn");
    }

    #[test]
    fn an_instances_eight_handles_go_two_to_a_number() {
        let maps: Maps = std::array::from_fn(|i| TextureHandle(i as u32 + 1));
        let packed = packed(maps);
        for i in 0..4 {
            assert_eq!(packed[i] & 0xffff, maps[i].0, "the map in the low half");
            assert_eq!(packed[i] >> 16, maps[i + 4].0, "the material's own in the high");
        }
    }

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


/// An `f32` as a half float's bits, rounded to nearest: what an
/// `Rgba16Float` texture written from the CPU wants.
fn half_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;
    if exponent == 0xff {
        // Infinity or NaN.
        return sign | 0x7c00 | if mantissa != 0 { 0x200 } else { 0 };
    }
    let e = exponent - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = (mantissa | 0x0080_0000) >> (1 - e);
        return sign | ((m + 0x1000) >> 13) as u16;
    }
    let rounded = ((e as u32) << 10 | (mantissa >> 13)) + ((mantissa >> 12) & 1);
    sign | rounded as u16
}

#[cfg(test)]
mod half_float_tests {
    #[test]
    fn half_floats_are_what_a_sky_table_holds() {
        assert_eq!(super::half_bits(0.0), 0);
        assert_eq!(super::half_bits(1.0), 0x3c00);
        assert_eq!(super::half_bits(-2.0), 0xc000);
        assert_eq!(super::half_bits(0.5), 0x3800);
        assert_eq!(super::half_bits(65504.0), 0x7bff);
        assert_eq!(super::half_bits(1e6), 0x7c00);
        assert_eq!(super::half_bits(6.103_515_6e-5), 0x0400);
        assert_eq!(super::half_bits(5.960_464_5e-8), 0x0001);
    }
}

