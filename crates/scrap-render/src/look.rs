//! What a line of a scene looks like, and how the scene looks: the render
//! module's fields — `model`, `material`, `camera`, `light`, `particles`,
//! `decal`… and the scene's `view`, `sun`, `fog`, `post`… — as types, and
//! the reading of them off a line ([`LookLine`]) and a scene
//! ([`SceneLook`]). See docs/modules.md.

use glam::Vec3;
use serde::{Deserialize, Serialize};

#[allow(unused_imports)]
use crate::defaults::*;
use crate::material::Material;
use crate::scene::{EntityDesc, Override, Scene};

/// A camera on an entity: what the game sees through, looking along the
/// entity's +z with its y up — Unity's Camera component, and like it,
/// carried by whatever the entity is under: a camera that is a child of
/// the player follows the player.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Lens {
    /// Vertical field of view, degrees.
    #[serde(default = "lens_fov")]
    pub fov_deg: f32,
    /// With several cameras, the highest looks.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub priority: i32,
    /// Orthographic, this many metres from the middle of the image to its
    /// top: `camera: (ortho: 12.0)` for a top-down or isometric game.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub ortho: Option<f32>,
    /// Keep after another entity and look at it — Cinemachine's Follow and
    /// Look At: `follow: (target: "<id>", offset: (0.0, 3.0, -6.0),
    /// damping: 0.3)`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub follow: Option<Follow>,
    /// Blend into this camera when it takes over rather than cut to it —
    /// Cinemachine's blend: `blend: (seconds: 1.0, ease: InOutCubic)`.
    /// Real seconds: slow motion does not stretch it (docs/feel.md).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub blend: Option<Blend>,
}

/// How a camera comes in when it takes over.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Blend {
    /// Seconds from the camera before to this one.
    #[serde(default = "unit")]
    pub seconds: f32,
    /// How the way is covered: slow at both ends by default.
    #[serde(default = "blend_ease")]
    pub ease: crate::ease::Ease,
}

fn blend_ease() -> crate::ease::Ease {
    crate::ease::Ease::InOutCubic
}

/// A camera keeping after a target.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Follow {
    /// The entity followed, by id.
    pub target: crate::id::EntityId,
    /// Where the camera keeps, from the target, in the world's axes.
    #[serde(default = "follow_offset")]
    pub offset: Vec3,
    /// Seconds to close most of a gap: 0 is stuck to it, 0.3 lags softly.
    #[serde(default = "follow_damping")]
    pub damping: f32,
    /// Turn to look at the target.
    #[serde(default = "yes_look")]
    pub look: bool,
    /// Seconds to turn most of the way toward the target, as `damping`
    /// is for the place: 0 turns at once.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub look_damping: f32,
    /// Where on the target to look, from its origin, in the world's axes:
    /// `(0.0, 1.5, 0.0)` for a person's head rather than their feet.
    #[serde(default, skip_serializing_if = "is_zero_vec3")]
    pub look_offset: Vec3,
    /// Metres the target may stray from where the camera keeps before the
    /// camera goes after it — a dead zone: small steps do not move it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dead_zone: f32,
}

fn follow_offset() -> Vec3 {
    Vec3::new(0.0, 3.0, -6.0)
}

fn follow_damping() -> f32 {
    0.3
}

fn yes_look() -> bool {
    true
}

/// A light at an entity, shining every way and fading to nothing at
/// `range` metres — a campfire, a lamp, a torch in a greybox corridor:
/// Unity's Point Light, casting shadows. `light: (color: (1.0, 0.6, 0.3),
/// intensity: 2.0, range: 6.0)`; the colour is as a colour picker says it
/// (sRGB, 0 to 1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Light {
    #[serde(default = "white")]
    pub color: (f32, f32, f32),
    #[serde(default = "unit")]
    pub intensity: f32,
    #[serde(default = "light_range")]
    pub range: f32,
    /// A spot light instead: shines along the entity's −z (the way a
    /// camera looks, and a Unity light's +z brought over the mirror) in a
    /// cone this many degrees across — a torch, a searchlight, headlights.
    /// Unity's Spot Light.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub cone_deg: Option<f32>,
    /// A spot's inner cone, degrees across: all its light inside, fading
    /// to none at `cone_deg` — URP's Inner Spot Angle. Without it the
    /// light fades over the cone's last tenth.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub inner_cone_deg: Option<f32>,
    /// Casts shadows — on unless `shadows: false`, as a lamp in URP; the
    /// nearest lamps the camera sees get them first.
    #[serde(default = "yes_look", skip_serializing_if = "is_true")]
    pub shadows: bool,
    /// A lens flare on the lamp itself — a glow round it and ghosts
    /// across the picture — this bright, gone when something hides the
    /// lamp. Unity's Lens Flare (SRP) component. 0 is none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub flare: f32,
    /// How it fades on its way to `range`: smooth unless said.
    #[serde(default, skip_serializing_if = "is_smooth")]
    pub falloff: crate::render::Falloff,
    /// Its colour temperature, kelvin, laid over `color` — Unity's light
    /// Temperature ([`color_temperature`]). None is white light.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub temperature: Option<f32>,
}

/// The colour of a black body `kelvin` hot, linear, its brightest channel
/// 1: Unity's `Mathf.CorrelatedColorTemperatureToRGB` (Krystek's
/// approximation of the Planckian locus, into sRGB's primaries), what URP
/// multiplies a light's colour by when it uses a temperature.
pub fn color_temperature(kelvin: f32) -> glam::Vec3 {
    let t = kelvin.clamp(1000.0, 20000.0) as f64;
    let u = (0.860117757 + 1.54118254e-4 * t + 1.28641212e-7 * t * t)
        / (1.0 + 8.42420235e-4 * t + 7.08145163e-7 * t * t);
    let v = (0.317398726 + 4.22806245e-5 * t + 4.20481691e-8 * t * t)
        / (1.0 - 2.89741816e-5 * t + 1.61456053e-7 * t * t);
    let x = 3.0 * u / (2.0 * u - 8.0 * v + 4.0);
    let y = 2.0 * v / (2.0 * u - 8.0 * v + 4.0);
    let (big_x, big_y, big_z) = (x / y, 1.0, (1.0 - x - y) / y);
    let r = 3.2404542 * big_x - 1.5371385 * big_y - 0.4985314 * big_z;
    let g = -0.9692660 * big_x + 1.8760108 * big_y + 0.0415560 * big_z;
    let b = 0.0556434 * big_x - 0.2040259 * big_y + 1.0572252 * big_z;
    let top = r.max(g).max(b);
    glam::Vec3::new((r / top).max(0.0) as f32, (g / top).max(0.0) as f32, (b / top).max(0.0) as f32)
}

fn is_smooth(f: &crate::render::Falloff) -> bool {
    *f == crate::render::Falloff::Smooth
}

/// A box whose surroundings polished things in it reflect — URP's baked
/// Reflection Probe, at the entity. `reflection_probe: (size: (8.0, 4.0,
/// 8.0))`: the box, metres, centred on the entity and not turned with it;
/// `box_projection: false` reflects as if the room were infinitely far;
/// `blend_distance` (1 m) fades it out inside its edge. See
/// [`crate::reflections`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Probe {
    #[serde(default = "probe_size")]
    pub size: Vec3,
    #[serde(default = "yes_look", skip_serializing_if = "is_true")]
    pub box_projection: bool,
    #[serde(default = "unit", skip_serializing_if = "is_one")]
    pub blend_distance: f32,
}

/// A camera that draws into a picture a material shows — a mirror, a
/// security monitor — rather than onto the screen: Unity's camera with a
/// Render Texture. `render_texture: (name: "mirror", hide: ["Player
/// head"])` on an entity with a `camera`; a material shows it with
/// `base_map: "render:mirror"`. `hide` leaves those layers out of its
/// picture (Unity's culling mask); whatever shows the picture itself is
/// left out on its own. The picture is the size of the screen's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderTexture {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
    /// A planar mirror instead: the entity's plane (facing `facing`)
    /// reflects what the screen's camera sees, no `camera` of its own
    /// needed; its material shows the picture with `screen_map: Mirror`.
    /// What is behind the plane is clipped away, a pixel at a time.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror: bool,
    /// Which of its own axes a mirror's glass looks along: its up, or
    /// `(0.0, 0.0, 1.0)` for Unity's Quad (its −z, the scene mirrored) —
    /// Dacha's `PlanarReflectionMirror.facing`.
    #[serde(default = "up_axis", skip_serializing_if = "is_up_axis")]
    pub facing: Vec3,
}

fn up_axis() -> Vec3 {
    Vec3::Y
}

fn is_up_axis(v: &Vec3) -> bool {
    *v == Vec3::Y
}

/// A place that looks different — the cellar darker and greener, the
/// sauna hazy — URP's local Volume. `post_volume: (size: (6.0, 3.0, 6.0),
/// post: (exposure: -0.5, saturation: 0.6))`: inside the box (metres,
/// centred on the entity, turned and scaled with it) the camera sees
/// `post`; `blend_distance` (2 m) outside it, the scene's own; between,
/// the two mixed. Settings `post` does not say are the defaults, not the
/// scene's. Where volumes overlap, the higher `priority` is laid last.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PostVolume {
    #[serde(default = "probe_size")]
    pub size: Vec3,
    #[serde(default = "two", skip_serializing_if = "is_two")]
    pub blend_distance: f32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub priority: i32,
    #[serde(default)]
    pub post: crate::post::PostProcess,
}

fn two() -> f32 {
    2.0
}

fn is_two(v: &f32) -> bool {
    *v == 2.0
}

/// A picture pressed onto what lies in a box — URP's Decal Projector.
/// `decal: (size: (2.0, 1.0, 2.0))`: the box, metres, centred on the
/// entity and turned with it, pressed down its −y (a decal on the ground
/// needs no turning). The entity's `material` is the picture: its base
/// colour and map (alpha is how much), normal map and smoothness. See
/// [`crate::decals`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Decal {
    #[serde(default = "decal_size")]
    pub size: Vec3,
}

fn decal_size() -> Vec3 {
    Vec3::ONE
}

fn probe_size() -> Vec3 {
    Vec3::new(10.0, 10.0, 10.0)
}

/// Bits given off from an entity and falling away — sparks over a fire,
/// dust from a cart, spray from a fountain: Unity's Particle System, the
/// few knobs a greybox needs. `particles: (rate: 30.0, life: 0.8, speed:
/// 2.0, spread_deg: 20.0, size: 0.06, gravity: -1.0, color: (1.0, 0.6,
/// 0.2))`; they leave along the entity's up, within `spread_deg` of it,
/// and shrink to nothing as they age.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Emitter {
    /// How many a second.
    #[serde(default = "emit_rate")]
    pub rate: f32,
    /// Seconds each lasts.
    #[serde(default = "unit")]
    pub life: f32,
    /// Metres a second, leaving.
    #[serde(default = "unit")]
    pub speed: f32,
    /// How far from straight up they may leave, degrees.
    #[serde(default = "emit_spread")]
    pub spread_deg: f32,
    /// Metres across, new.
    #[serde(default = "emit_size")]
    pub size: f32,
    /// Metres a second, every second, along y: negative falls, positive
    /// rises like smoke.
    #[serde(default)]
    pub gravity: f32,
    #[serde(default = "white")]
    pub color: (f32, f32, f32),
    /// The colour each fades to by the end of its life: Unity's Color over
    /// Lifetime. Unset, it keeps `color`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub end_color: Option<(f32, f32, f32)>,
    /// How opaque each is when new, and when it dies: smoke thinning to
    /// nothing is `end_alpha: 0.0`. Below 1, they are drawn see-through.
    #[serde(default = "unit", skip_serializing_if = "is_one")]
    pub alpha: f32,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub end_alpha: Option<f32>,
    /// How big each is at the end: Unity's Size over Lifetime. Unset, it
    /// shrinks to nothing.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub end_size: Option<f32>,
    /// Drawn longer the faster it goes, along its way: a spark's streak,
    /// Unity's Stretched Billboard. Extra length per metre a second.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub stretch: f32,
    /// A streak left behind each as it goes, as long as it goes in this
    /// many seconds of its own time, `size` wide: a spark's line of light.
    /// Unity's Trails, the trail's lifetime times the particle's.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub trail: f32,
    /// They move with the emitter — a torch's flame follows the torch —
    /// instead of staying where they were given off. Unity's Simulation
    /// Space: Local.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local: bool,
    /// Seconds one play lasts: a puff of dust is short and once.
    #[serde(default = "emit_duration", skip_serializing_if = "is_five")]
    pub duration: f32,
    /// Plays once and stops — an effect a game starts with
    /// `Emitting::play` — instead of going round forever.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub once: bool,
    /// Waits for `Emitting::play` instead of starting with the scene.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub waits: bool,
    /// So many at once, so many seconds into each play: `[(0.0, 30)]` is
    /// the puff a spade makes. Unity's Emission bursts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bursts: Vec<(f32, u32)>,
    /// Which way they leave, in the entity's own axes: up when not said.
    /// Unity's cones point along forward — an imported one says so here.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub direction: Option<Vec3>,
    /// How fast it plays: 0.5 at half speed, every second of it twice as
    /// long. Unity's Simulation Speed.
    #[serde(default = "unit", skip_serializing_if = "is_one")]
    pub time_scale: f32,
    /// Metres a second past which each is slowed, by `dampen` of what it
    /// is over each thirtieth of a second: a burst that flies out and
    /// stops. Unity's Limit Velocity over Lifetime.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub speed_limit: Option<f32>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dampen: f32,
    /// How big each is over its life, times `size`: (share of life,
    /// factor) keys, straight between them. Unity's Size over Lifetime
    /// curve. Set, it stands in for `end_size`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub size_keys: Vec<(f32, f32)>,
    /// Its colour over its life, times `color`: (share of life, linear
    /// rgb) keys, past 1 for a glow. Unity's Color over Lifetime. Set, it
    /// stands in for `end_color`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub color_keys: Vec<(f32, (f32, f32, f32))>,
    /// Its opacity over its life, times `alpha`. Set, it stands in for
    /// `end_alpha`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alpha_keys: Vec<(f32, f32)>,
    /// Numbers each carries to its material's shader over its life,
    /// written into the material's own numbers from `slot` on: Unity's
    /// Custom Data — a dissolve's progress, a highlight's colour.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom: Vec<CustomStream>,
    /// Its material's picture is a sheet of this many frames across and
    /// down, and each is drawn as one of them. Unity's Texture Sheet
    /// Animation.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub sheet: Option<(u32, u32)>,
    /// Which frame of the sheet, 0 the first and 1 past the last: from the
    /// first number to the second over each one's life — or, with
    /// `frames_random`, one picked between them when it is born.
    #[serde(default = "all_frames", skip_serializing_if = "is_all_frames")]
    pub frames: (f32, f32),
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub frames_random: bool,
    /// Sizes times the entity's own scale in the world — a small emitter's
    /// puffs are small — and its speeds too, as its shape is: a small
    /// emitter's sparks fly as far as it is big. Unity's Scaling Mode Local
    /// and Hierarchy.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scaled: bool,
    /// Where they are born, in the entity's own axes: its origin when not
    /// said. Unity's Shape position.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub from: Option<Vec3>,
    /// Born anywhere in a box this size about `from` — dust over a whole
    /// level — turned by `shape_turn_deg`. Unity's Box shape.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub box_size: Option<Vec3>,
    /// Born anywhere within this many metres of `from`: across the cone's
    /// mouth, or in a ball when they leave every way. Unity's radius.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub radius: f32,
    /// How the box is turned in the entity's axes, degrees. Unity's Shape
    /// rotation.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub shape_turn_deg: Option<Vec3>,
    /// Each is a flat square turned to the camera — smoke, sparks, dust
    /// with a picture — instead of a small solid. Unity's Billboard.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub facing: bool,
    /// What each is drawn as; a small cube when not said (a square when
    /// `facing`).
    #[serde(default, skip_serializing_if = "str::is_empty")]
    pub model: crate::AssetLink,
    /// A material — a picture, transparency, a glow — instead of the plain
    /// `color`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub material: Option<crate::AssetLink>,
    /// On the GPU (see [`crate::particles_gpu`]): tens of thousands at
    /// once — a blizzard, a sandstorm's grit, a fountain's spray — each a
    /// soft disc of `color` turned to the camera, lit by nothing, rather
    /// than a small model with a material. `model` and `material` are not
    /// used then.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gpu: bool,
    /// On the GPU, they bounce off what is drawn: each step a particle
    /// that has gone behind the scene's depth is put back on the surface
    /// there and bounced off it — sparks off a floor, rain off a roof —
    /// with no colliders at all (screen-space collision). Off screen they
    /// pass through.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collide: bool,
    /// On the GPU, an effect graph (`shaders/<name>.vfx.ron`, see
    /// [`scrap_shadergraph::effect`]) says where each is born, how it moves
    /// and how it looks, instead of the numbers above alone — which it
    /// still reads. Unity's VFX Graph.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub graph: String,
    /// Up to eight numbers its effect graph reads as its `params`, in their
    /// order: Unity's VFX exposed properties.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<f32>,
}

/// One of an emitter's streams of numbers to its particles' shader: from
/// `slot` of the material's eight numbers on, one number per component,
/// keyed over each particle's life.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomStream {
    pub slot: u8,
    /// Which of Unity's streams it was, `custom0` or `custom1`: how an
    /// importer finds its slot in a shader that names them. Not read here.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// (share of life, values) keys, straight between them.
    pub keys: Vec<(f32, Vec<f32>)>,
}

/// Straight between (share of life, value) keys; the ends held.
pub fn keyed(keys: &[(f32, f32)], t: f32) -> f32 {
    match keys {
        [] => 1.0,
        [only] => only.1,
        _ => {
            if t <= keys[0].0 {
                return keys[0].1;
            }
            for w in keys.windows(2) {
                if t <= w[1].0 {
                    let span = (w[1].0 - w[0].0).max(1e-6);
                    return w[0].1 + (w[1].1 - w[0].1) * ((t - w[0].0) / span);
                }
            }
            keys[keys.len() - 1].1
        }
    }
}

fn all_frames() -> (f32, f32) {
    (0.0, 1.0)
}
fn is_all_frames(f: &(f32, f32)) -> bool {
    *f == (0.0, 1.0)
}
fn emit_duration() -> f32 {
    5.0
}

fn is_five(v: &f32) -> bool {
    *v == 5.0
}

impl Default for Emitter {
    fn default() -> Self {
        ron::from_str("()").expect("every field has a default")
    }
}

fn emit_rate() -> f32 {
    10.0
}

fn emit_spread() -> f32 {
    15.0
}

fn emit_size() -> f32 {
    0.1
}

fn white() -> (f32, f32, f32) {
    (1.0, 1.0, 1.0)
}

fn light_range() -> f32 {
    5.0
}

fn lens_fov() -> f32 {
    60.0
}


fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaterialRef {
    /// A material asset in the library — by link, its name and ID
    /// (docs/refs.md) — or one of the engine's builtins by name.
    /// `builtin:stone` forces the builtin even when a project has an asset
    /// of that name.
    Named(crate::AssetLink),
    /// Spelled out, for a colour that has not earned a name yet.
    Inline(Material),
}

impl Default for MaterialRef {
    fn default() -> Self {
        MaterialRef::Inline(Material::default())
    }
}

impl MaterialRef {
    fn is_default(&self) -> bool {
        *self == MaterialRef::default()
    }
}

/// The sun, which is the only light the valley has.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sun {
    /// Hour of the day in `[0, 24)`. Direction and color are derived from it
    /// by the game, so a scene stores the hour, not a vector.
    pub hour: f32,
    pub intensity: f32,
    /// What the ground under the sun is, as a colour picker says it (sRGB):
    /// the light it throws back up onto everything from below. Sand throws
    /// a lot, and warm; dark earth little.
    #[serde(default = "default_ground", skip_serializing_if = "is_default_ground")]
    pub ground: [f32; 3],
    /// Which way the light travels, when a scene brought from elsewhere
    /// says exactly: the hour's arc goes east to west only, and a sun from
    /// Unity can stand anywhere. Unset, the hour decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toward: Option<Vec3>,
    /// The light's colour as a picker says it (sRGB), when it is not the
    /// hour's: white overhead, orange low.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<[f32; 3]>,
    /// The light from all round, when a scene says it rather than the
    /// sky working it out: what a face turned up, one turned sideways and
    /// one turned down each see — Unity's gradient ambient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<Ambient>,
    /// How dark its shadow is, 0 to 1: 1 black but for the light from
    /// all round, less lets some of the sun in — Unity's Strength.
    #[serde(default = "full", skip_serializing_if = "is_full")]
    pub shadow_strength: f32,
    /// Its colour temperature, kelvin: the colour of a black body that
    /// hot laid over `tint` — Unity's light Temperature, which URP always
    /// uses. None is white light.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

fn full() -> f32 {
    1.0
}

fn is_full(v: &f32) -> bool {
    *v == 1.0
}

/// Light from all round as three colours (sRGB, as a picker says them,
/// and as bright as they are meant): above, at the horizon, below. A face
/// sees a blend of the two it turns between.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ambient {
    pub sky: [f32; 3],
    pub equator: [f32; 3],
    pub ground: [f32; 3],
}

fn default_ground() -> [f32; 3] {
    [0.36, 0.33, 0.29]
}

fn is_default_ground(g: &[f32; 3]) -> bool {
    *g == default_ground()
}

impl Default for Sun {
    fn default() -> Self {
        Self {
            ground: default_ground(),
            hour: 9.0,
            intensity: 1.15,
            toward: None,
            tint: None,
            ambient: None,
            shadow_strength: 1.0,
            temperature: None,
        }
    }
}

impl Sun {
    /// Which way the light travels: from the sun toward the ground.
    ///
    /// A crude arc — up at noon, along the ground at either end — and
    /// deliberately the engine's rather than each tool's. It lived in
    /// `scene_shot` while the editor and the walk-around used a fixed
    /// default, which meant one scene had three different suns depending on
    /// which program opened it, and a screenshot could not be compared with
    /// what the editor showed. A game that wants a real ephemeris replaces
    /// this; a scene's `hour` has to mean one thing first.
    pub fn direction(&self) -> Vec3 {
        if let Some(toward) = self.toward.filter(|t| t.length_squared() > 1e-8) {
            return toward.normalize();
        }
        let day = ((self.hour - 6.0) / 12.0).clamp(0.0, 1.0);
        let angle = day * std::f32::consts::PI;
        // The height is floored well above zero: a sun exactly on the
        // horizon lights nothing but the horizon, and every shadow in the
        // scene becomes a stripe reaching to the far plane.
        Vec3::new(-angle.cos(), -angle.sin().max(0.15), -0.35).normalize()
    }

    /// How high the sun really stands, −1 to 1: the sine of its height
    /// over the horizon on the hour's arc, below it at night — where
    /// [`Self::direction`] keeps it just over the horizon so there is a
    /// light to see by. With `toward` set, that says it.
    pub fn elevation(&self) -> f32 {
        if let Some(toward) = self.toward.filter(|t| t.length_squared() > 1e-8) {
            return -toward.normalize().y;
        }
        let angle = (self.hour - 6.0) / 12.0 * std::f32::consts::PI;
        angle.sin()
    }

    /// Which way the sun's light would travel if the sun were where it
    /// really is — under the ground at night: what the sky is lit by.
    pub fn true_direction(&self) -> Vec3 {
        if let Some(toward) = self.toward.filter(|t| t.length_squared() > 1e-8) {
            return toward.normalize();
        }
        let angle = (self.hour - 6.0) / 12.0 * std::f32::consts::PI;
        Vec3::new(-angle.cos(), -angle.sin(), -0.35).normalize()
    }

    /// Which way the moon's light travels: the moon across the sky from
    /// the sun, as when it is full, never lower than the sun is let be.
    pub fn moon_direction(&self) -> Vec3 {
        let angle = (self.hour - 18.0) / 12.0 * std::f32::consts::PI;
        Vec3::new(-angle.cos(), -angle.sin().max(0.2), 0.3).normalize()
    }

    /// How much it is night, 0 to 1: nothing while the sun is up, whole
    /// once it is well below the horizon, the dusk between.
    pub fn night(&self) -> f32 {
        ((0.02 - self.elevation()) / 0.14).clamp(0.0, 1.0)
    }

    /// How warm the light is: white overhead, orange near the horizon.
    ///
    /// Not physics — the sky is not scattering anything here — but the one
    /// cue that reads as a time of day at a glance, and cheaper than every
    /// scene hand-picking a colour to go with its hour.
    /// What its temperature does to its colour: white without one.
    pub fn filter(&self) -> Vec3 {
        self.temperature.map_or(Vec3::ONE, color_temperature)
    }

    pub fn color(&self) -> Vec3 {
        if let Some(t) = self.tint {
            let l = |c: f32| crate::material::srgb_to_linear(c.clamp(0.0, 1.0));
            return Vec3::new(l(t[0]), l(t[1]), l(t[2]));
        }
        let noon = Vec3::new(1.0, 0.96, 0.88);
        let low = Vec3::new(1.0, 0.72, 0.48);
        let height = (-self.direction().y).clamp(0.0, 1.0);
        low + (noon - low) * height
    }
}

/// Distance fog. On by default, unlike the old renderer's — without it
/// nothing in a forest reads as far away.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Fog {
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
    /// Linear between `start` and `end`, or thickening by `density` —
    /// URP's three. Written only when not linear.
    #[serde(default, skip_serializing_if = "is_linear")]
    pub mode: crate::render::FogMode,
    #[serde(default = "fog_density", skip_serializing_if = "is_fog_density")]
    pub density: f32,
}

fn is_linear(mode: &crate::render::FogMode) -> bool {
    *mode == crate::render::FogMode::Linear
}

fn fog_density() -> f32 {
    0.01
}

fn is_fog_density(density: &f32) -> bool {
    *density == fog_density()
}

impl Default for Fog {
    fn default() -> Self {
        Self {
            color: [0.62, 0.68, 0.74],
            start: 30.0,
            end: 180.0,
            mode: crate::render::FogMode::Linear,
            density: fog_density(),
        }
    }
}

/// Where the scene is looked at from.
///
/// In the file, because a picture has to be reproducible. The loop this
/// engine is built for is "change something, render it, look" — and a
/// viewpoint that lives in whichever tool happened to render means the
/// picture moves for reasons the scene never recorded, which makes two
/// renders impossible to compare. Putting it here also gives an agent a way
/// to frame a shot: it is a line of text like everything else.
///
/// Three numbers and a field of view, not a whole [`Camera`](crate::Camera).
/// Near and far planes and an up vector are renderer business; a scene says
/// where it is looked at from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub position: Vec3,
    pub target: Vec3,
    /// Vertical field of view, in degrees.
    pub fov_deg: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 3.4, 12.0),
            target: Vec3::new(0.0, 1.4, -4.0),
            fov_deg: 55.0,
        }
    }
}

/// One of the scene's look fields ([`crate::moods::LOOK_FIELDS`]) as the
/// file writes it; `None` for an optional one the scene does not set.
pub fn look_field(scene: &Scene, field: &str) -> Result<String, String> {
    fn text<T: Serialize>(value: &T) -> String {
        ron::to_string(value).unwrap_or_default()
    }
    fn optional<T: Serialize>(value: &Option<T>) -> String {
        value.as_ref().map_or_else(|| "None".to_string(), text)
    }
    Ok(match field {
        "sun" => text(&scene.sun()),
        "fog" => text(&scene.fog()),
        "sky" => optional(&scene.sky()),
        "post" => optional(&scene.post()),
        "ambient_occlusion" => optional(&scene.ambient_occlusion()),
        "shadows" => optional(&scene.shadows()),
        "volumetric_fog" => optional(&scene.volumetric_fog()),
        "weather" => optional(&scene.weather()),
        "wind" => optional(&scene.wind()),
        "screen_space_reflections" => optional(&scene.screen_space_reflections()),
        "ray_tracing" => optional(&scene.ray_tracing()),
        other => {
            return Err(format!(
                "`{other}` is not part of the scene's look — there are {}",
                crate::moods::LOOK_FIELDS.join(", ")
            ))
        }
    })
}

/// Set one of the scene's look fields from RON; `None` clears an optional
/// one back to the engine's default.
pub fn set_look_field(scene: &mut Scene, field: &str, ron_text: &str) -> Result<(), String> {
    fn parse<T: serde::de::DeserializeOwned>(field: &str, text: &str) -> Result<T, String> {
        ron::from_str(text).map_err(|e| format!("{field}: {e}"))
    }
    fn optional<T: serde::de::DeserializeOwned>(
        field: &str,
        text: &str,
    ) -> Result<Option<T>, String> {
        if text.trim() == "None" {
            Ok(None)
        } else {
            parse(field, text).map(Some)
        }
    }
    match field {
        "sun" => scene.set_part::<Sun>(&parse(field, ron_text)?),
        "fog" => scene.set_part::<Fog>(&parse(field, ron_text)?),
        "sky" => scene.set_part_opt::<crate::render::Sky>(optional(field, ron_text)?.as_ref()),
        "post" => {
            scene.set_part_opt::<crate::post::PostProcess>(optional(field, ron_text)?.as_ref())
        }
        "ambient_occlusion" => {
            scene.set_part_opt::<crate::ssao::AmbientOcclusion>(optional(field, ron_text)?.as_ref())
        }
        "shadows" => {
            scene.set_part_opt::<crate::render::ShadowSettings>(optional(field, ron_text)?.as_ref())
        }
        "volumetric_fog" => {
            scene.set_part_opt::<crate::volume::VolumetricFog>(optional(field, ron_text)?.as_ref())
        }
        "weather" => {
            scene.set_part_opt::<crate::weather::Weather>(optional(field, ron_text)?.as_ref())
        }
        "wind" => scene.set_part_opt::<crate::foliage::Wind>(optional(field, ron_text)?.as_ref()),
        "screen_space_reflections" => scene
            .set_part_opt::<crate::reflections::ScreenSpaceReflections>(
                optional(field, ron_text)?.as_ref(),
            ),
        "ray_tracing" => {
            scene.set_part_opt::<crate::ray::RayTracing>(optional(field, ron_text)?.as_ref())
        }
        other => {
            return Err(format!(
                "`{other}` is not part of the scene's look — there are {}",
                crate::moods::LOOK_FIELDS.join(", ")
            ))
        }
    }
    Ok(())
}

pub use scrap_geometry::line::ModelRef;


/// `virtual_shadows: true` on a scene — the sun's shadow from virtual
/// shadow maps, sharp near and far, out to 200 m. See [`crate::vsm`].
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VirtualShadows(pub bool);

/// `bends_grass: 0.6` — grass and anything else that sways is pushed aside
/// within this many metres of it. See [`crate::foliage`].
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BendsGrass(pub f32);


crate::impl_parts! {
    // By name, or spelled out: serde's untagged enum, which a trace of
    // its type cannot see into.
    MaterialRef => "material", default if |m| m.is_default(), shape || {
        use scrap_core::shape::{self, Shape};
        Shape::OneOf(vec![
            Shape::Asset("material".into()),
            shape::with_fractions(shape::of::<Material>(), crate::material::FRACTIONS),
        ])
    };
    BendsGrass => "bends_grass", default if |b| b.0 == 0.0;
    Lens => "camera";
    Light => "light";
    Emitter => "particles", fractions ["alpha", "end_alpha", "dampen"];
    Probe => "reflection_probe";
    crate::ddgi::IrradianceVolume => "irradiance_volume";
    RenderTexture => "render_texture";
    PostVolume => "post_volume", shape || {
        use scrap_core::shape;
        let fractions: Vec<String> = crate::post::FRACTIONS.iter().map(|f| format!("post.{f}")).collect();
        let fractions: Vec<&str> = fractions.iter().map(String::as_str).collect();
        shape::with_fractions(shape::of_field::<PostVolume>("post_volume"), &fractions)
    };
    Decal => "decal";
    crate::footprints::Footprints => "footprints";
    View => "view";
    Sun => "sun";
    Fog => "fog";
    crate::render::Sky => "sky", fractions ["atmosphere.ground_albedo", "clouds.coverage", "clouds.shadows"];
    crate::post::PostProcess => "post", shape || {
        use scrap_core::shape;
        shape::with_fractions(shape::of_field::<crate::post::PostProcess>("post"), crate::post::FRACTIONS)
    };
    crate::ssao::AmbientOcclusion => "ambient_occlusion", fractions ["direct_lighting_strength"];
    crate::render::ShadowSettings => "shadows", fractions ["cascade_border"];
    crate::ray::RayTracing => "ray_tracing", fractions ["reflection_roughness"];
    VirtualShadows => "virtual_shadows", default if |v| !v.0;
    crate::volume::VolumetricFog => "volumetric_fog";
    crate::weather::Weather => "weather", fractions [
        "rain", "wetness", "puddles", "snow", "snowfall", "sandstorm", "dust_wall",
        "dust_devils", "drifted", "lightning", "drying", "mud",
    ];
    crate::reflections::ScreenSpaceReflections => "screen_space_reflections";
}

/// What a line of a scene looks like, read off it: what was `desc.model`
/// before a line's fields were its modules' (docs/modules.md).
pub trait LookLine {
    fn material_ref(&self) -> MaterialRef;
    fn camera(&self) -> Option<Lens>;
    fn light(&self) -> Option<Light>;
    fn particles(&self) -> Option<Emitter>;
    fn reflection_probe(&self) -> Option<Probe>;
    fn render_texture(&self) -> Option<RenderTexture>;
    fn post_volume(&self) -> Option<PostVolume>;
    fn decal(&self) -> Option<Decal>;
    fn footprints(&self) -> Option<crate::footprints::Footprints>;
    fn bends_grass(&self) -> f32;
    fn set_material(&mut self, material: MaterialRef);
    fn set_bends_grass(&mut self, metres: f32);
    /// The material to draw with, using the engine's builtins only.
    ///
    /// What a test and the reference scene want, since neither has a
    /// library. Anything that does have one calls
    /// [`LookLine::material_from`].
    fn material(&self) -> Material {
        self.material_from(|_| None)
    }
    /// The material to draw with, asking `lookup` first.
    ///
    /// The order is the project's palette, then the engine's builtins, then
    /// plain grey. A project's own `stone` therefore shadows the engine's,
    /// which is the useful direction: the builtins exist so that an example
    /// scene can be written before a palette exists, not to reserve seven
    /// names forever. `builtin:stone` reaches past the shadow when that is
    /// what was meant.
    ///
    /// An unknown name falls back rather than failing to load — a scene with
    /// a typo should still open, showing plain grey where the mistake is,
    /// which is more useful than an error and no scene at all.
    fn material_from(&self, lookup: impl Fn(&crate::AssetLink) -> Option<Material>) -> Material {
        match &self.material_ref() {
            MaterialRef::Named(name) => match name.strip_prefix("builtin:") {
                Some(builtin) => crate::material::builtin::by_name(builtin).unwrap_or_default(),
                None => lookup(name)
                    .or_else(|| crate::material::builtin::by_name(name))
                    .unwrap_or_default(),
            },
            MaterialRef::Inline(material) => *material,
        }
    }
}

impl LookLine for EntityDesc {
    fn material_ref(&self) -> MaterialRef {
        self.part_or_default()
    }
    fn camera(&self) -> Option<Lens> {
        self.part()
    }
    fn light(&self) -> Option<Light> {
        self.part()
    }
    fn particles(&self) -> Option<Emitter> {
        self.part()
    }
    fn reflection_probe(&self) -> Option<Probe> {
        self.part()
    }
    fn render_texture(&self) -> Option<RenderTexture> {
        self.part()
    }
    fn post_volume(&self) -> Option<PostVolume> {
        self.part()
    }
    fn decal(&self) -> Option<Decal> {
        self.part()
    }
    fn footprints(&self) -> Option<crate::footprints::Footprints> {
        self.part()
    }
    fn bends_grass(&self) -> f32 {
        self.part::<BendsGrass>().map_or(0.0, |b| b.0)
    }
    fn set_material(&mut self, material: MaterialRef) {
        self.set_part(&material)
    }
    fn set_bends_grass(&mut self, metres: f32) {
        self.set_part(&BendsGrass(metres))
    }
}

/// What an override of a prefab's part says about its look.
pub trait LookOverride {
    fn material(&self) -> Option<MaterialRef>;
    fn camera(&self) -> Option<Lens>;
    fn light(&self) -> Option<Light>;
    fn particles(&self) -> Option<Emitter>;
    fn reflection_probe(&self) -> Option<Probe>;
    fn decal(&self) -> Option<Decal>;
    fn footprints(&self) -> Option<crate::footprints::Footprints>;
    fn bends_grass(&self) -> Option<f32>;
}

impl LookOverride for Override {
    fn material(&self) -> Option<MaterialRef> {
        self.part()
    }
    fn camera(&self) -> Option<Lens> {
        self.part()
    }
    fn light(&self) -> Option<Light> {
        self.part()
    }
    fn particles(&self) -> Option<Emitter> {
        self.part()
    }
    fn reflection_probe(&self) -> Option<Probe> {
        self.part()
    }
    fn decal(&self) -> Option<Decal> {
        self.part()
    }
    fn footprints(&self) -> Option<crate::footprints::Footprints> {
        self.part()
    }
    fn bends_grass(&self) -> Option<f32> {
        self.part::<BendsGrass>().map(|b| b.0)
    }
}

/// How a scene looks, read off it.
pub trait SceneLook {
    fn view(&self) -> View;
    fn sun(&self) -> Sun;
    fn fog(&self) -> Fog;
    fn sky(&self) -> Option<crate::render::Sky>;
    fn post(&self) -> Option<crate::post::PostProcess>;
    fn ambient_occlusion(&self) -> Option<crate::ssao::AmbientOcclusion>;
    fn shadows(&self) -> Option<crate::render::ShadowSettings>;
    fn ray_tracing(&self) -> Option<crate::ray::RayTracing>;
    fn volumetric_fog(&self) -> Option<crate::volume::VolumetricFog>;
    fn wind(&self) -> Option<crate::foliage::Wind>;
    fn weather(&self) -> Option<crate::weather::Weather>;
    fn screen_space_reflections(&self) -> Option<crate::reflections::ScreenSpaceReflections>;
}

impl SceneLook for Scene {
    fn view(&self) -> View {
        self.part_or_default()
    }
    fn sun(&self) -> Sun {
        self.part_or_default()
    }
    fn fog(&self) -> Fog {
        self.part_or_default()
    }
    fn sky(&self) -> Option<crate::render::Sky> {
        self.part()
    }
    fn post(&self) -> Option<crate::post::PostProcess> {
        self.part()
    }
    fn ambient_occlusion(&self) -> Option<crate::ssao::AmbientOcclusion> {
        self.part()
    }
    fn shadows(&self) -> Option<crate::render::ShadowSettings> {
        self.part()
    }
    fn ray_tracing(&self) -> Option<crate::ray::RayTracing> {
        self.part()
    }
    fn volumetric_fog(&self) -> Option<crate::volume::VolumetricFog> {
        self.part()
    }
    fn wind(&self) -> Option<crate::foliage::Wind> {
        self.part()
    }
    fn weather(&self) -> Option<crate::weather::Weather> {
        self.part()
    }
    fn screen_space_reflections(&self) -> Option<crate::reflections::ScreenSpaceReflections> {
        self.part()
    }
}
