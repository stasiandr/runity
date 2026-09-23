//! The scene file: what the editor writes and the game reads.
//!
//! Scenes are RON, not code. That is the whole point of the split — a mouse
//! drag in the editor and a change made by an agent produce the same kind of
//! diff, and `git` can show either one. Nothing here knows about rendering or
//! physics; it is the description they are both built from.

use std::collections::BTreeMap;
use std::path::Path;

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::id::EntityId;
use crate::material::Material;

/// Position, rotation and scale, in the form a person can edit.
///
/// Rotation is stored as Euler degrees rather than a quaternion on purpose:
/// a quaternion in a text file is unreadable and uneditable, and the editor
/// converts on the way in and out. Order is Y (yaw), X (pitch), Z (roll),
/// which is what a turntable-style gizmo produces.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    #[serde(default = "zero")]
    pub position: Vec3,
    #[serde(default = "zero")]
    pub rotation_deg: Vec3,
    #[serde(default = "one")]
    pub scale: Vec3,
}

// `serde(default = "...")` names a function, and glam's consts are not one.
fn zero() -> Vec3 {
    Vec3::ZERO
}
fn one() -> Vec3 {
    Vec3::ONE
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation_deg: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    pub fn rotation(&self) -> Quat {
        let r = self.rotation_deg * std::f32::consts::PI / 180.0;
        Quat::from_euler(glam::EulerRot::YXZ, r.y, r.x, r.z)
    }

    pub fn set_rotation(&mut self, q: Quat) {
        let (y, x, z) = q.to_euler(glam::EulerRot::YXZ);
        self.rotation_deg = Vec3::new(x, y, z) * 180.0 / std::f32::consts::PI;
    }

    pub fn matrix(&self) -> glam::Mat4 {
        glam::Mat4::from_scale_rotation_translation(self.scale, self.rotation(), self.position)
    }
}

/// The shape physics uses for an entity, which is not the mesh.
///
/// A collider is always a primitive here. Colliding against the triangles a
/// thing is drawn from is available and almost always the wrong trade: it is
/// slower, it cannot be a dynamic body at all in most engines, and a box
/// around a crate behaves better than the crate's own bevelled corners.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Collider {
    #[default]
    None,
    /// Half the size on each axis, before the transform's scale, around
    /// `center` in the entity's own space: a model whose origin is at its
    /// foot has its box half its height up. Unity's BoxCollider center.
    Box {
        half: Vec3,
        #[serde(default, skip_serializing_if = "is_zero_vec3")]
        center: Vec3,
    },
    Sphere {
        radius: f32,
    },
    /// A cylinder with hemispherical caps: what a person is, because a box
    /// catches on corners and a sphere rolls.
    Capsule {
        half_height: f32,
        radius: f32,
    },
    /// An upright cylinder, centred: what fits `builtin:cylinder` with
    /// `half_height: 0.5, radius: 0.5`.
    Cylinder {
        half_height: f32,
        radius: f32,
    },
    /// A wedge, high at the back (−z): what fits `builtin:ramp` with
    /// `half: (0.5, 0.5, 0.5)`. Walkable, which a box standing in for a
    /// slope is not.
    Ramp {
        half: Vec3,
    },
    /// The shape of the entity's own model: a triangle mesh for a static
    /// body, its convex hull for a dynamic one — a solver cannot keep a
    /// hollow mesh from tunnelling. What a level modelled elsewhere, or a
    /// terrain, collides as.
    Model,
    /// A flight of steps rising toward the back: what fits
    /// `builtin:stairs` with `half: (0.5, 0.5, 0.5), steps: 4`.
    Stairs {
        half: Vec3,
        steps: u32,
    },
}

/// What holds a body to another, or to the world: a door's hinge, a lamp's
/// chain, a drawer's runner. On the line of the body that moves, naming the
/// one it hangs from by `id` — or nothing, for a fixed point in the world.
///
/// ```text
/// joint: Hinge(to: "5f1c09aa3e7b2d10", anchor: (-0.5, 0.0, 0.0), axis: (0.0, 1.0, 0.0), limits_deg: (0.0, 110.0)),
/// ```
///
/// `anchor` and `axis` are in this entity's own space, so a door's hinge is
/// "its left edge, turning about its up" wherever the door is placed. A
/// prefab's joints name other parts by their ids in the prefab file, and
/// every instance gets its own.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Joint {
    #[default]
    None,
    /// Held rigidly: two bodies that move as one but break apart as two.
    Fixed {
        #[serde(default, skip_serializing_if = "EntityId::is_unassigned")]
        to: EntityId,
    },
    /// Turns about one axis through the anchor, within limits if given.
    Hinge {
        #[serde(default, skip_serializing_if = "EntityId::is_unassigned")]
        to: EntityId,
        #[serde(default)]
        anchor: Vec3,
        #[serde(default = "up")]
        axis: Vec3,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
        limits_deg: Option<(f32, f32)>,
        /// Driven: turning at a speed (degrees a second), or held at an
        /// angle like a spring — a door that swings shut.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
        motor: Option<Motor>,
    },
    /// Turns any way about the anchor: a chain, a ball-and-socket.
    Ball {
        #[serde(default, skip_serializing_if = "EntityId::is_unassigned")]
        to: EntityId,
        #[serde(default)]
        anchor: Vec3,
    },
    /// Pulls its anchor toward the other body's, like a rubber band:
    /// Unity's SpringJoint. `stiffness` is how hard, `damping` how fast the
    /// bounce dies. Free to turn and to move otherwise.
    Spring {
        #[serde(default, skip_serializing_if = "EntityId::is_unassigned")]
        to: EntityId,
        #[serde(default)]
        anchor: Vec3,
        #[serde(default = "spring_stiffness")]
        stiffness: f32,
        #[serde(default = "spring_damping")]
        damping: f32,
    },
    /// Slides along one axis, within limits in metres if given.
    Slider {
        #[serde(default, skip_serializing_if = "EntityId::is_unassigned")]
        to: EntityId,
        #[serde(default = "up")]
        axis: Vec3,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
        limits: Option<(f32, f32)>,
        /// Driven: sliding at a speed (metres a second), or held at a
        /// position like a spring — a lift, a piston.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
        motor: Option<Motor>,
    },
}

/// What drives a hinge or a slider — Unity's joint Motor and Spring in one:
/// `(speed: 90.0)` turns it on and on, `(hold: 0.0, strength: 50.0)` pulls
/// it back to closed however it is pushed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Motor {
    /// Degrees (a hinge) or metres (a slider) a second.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub speed: f32,
    /// An angle or position to pull toward instead, like a spring.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub hold: Option<f32>,
    /// How hard: the spring's stiffness, or how firmly the speed is kept.
    #[serde(default = "motor_strength")]
    pub strength: f32,
}

fn motor_strength() -> f32 {
    10.0
}

fn up() -> Vec3 {
    Vec3::Y
}

fn spring_stiffness() -> f32 {
    50.0
}

fn spring_damping() -> f32 {
    5.0
}

impl Joint {
    pub fn is_none(&self) -> bool {
        *self == Joint::None
    }

    /// The body it hangs from; unassigned for the world.
    pub fn to(&self) -> Option<EntityId> {
        match *self {
            Joint::None => None,
            Joint::Fixed { to }
            | Joint::Hinge { to, .. }
            | Joint::Ball { to, .. }
            | Joint::Spring { to, .. }
            | Joint::Slider { to, .. } => Some(to),
        }
    }

    /// The same joint, hanging from another body.
    pub fn with_to(mut self, other: EntityId) -> Self {
        match &mut self {
            Joint::None => {}
            Joint::Fixed { to }
            | Joint::Hinge { to, .. }
            | Joint::Ball { to, .. }
            | Joint::Spring { to, .. }
            | Joint::Slider { to, .. } => *to = other,
        }
        self
    }
}

/// How a body feels to what touches it: Unity's physic material and
/// rigidbody mass in one line, and only what differs from the default is
/// written — `physics: (bounce: 0.8)` is a ball.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BodyProps {
    /// Grip, from 0 (ice) up; 0.5 is dry wood on wood.
    #[serde(default = "half", skip_serializing_if = "is_half")]
    pub friction: f32,
    /// How much of a hit comes back, 0 (a sandbag) to 1 (a superball).
    /// When two things meet, the bouncier one decides — a ball bounces off
    /// a floor that says nothing about bounce.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub bounce: f32,
    /// Mass per cubic metre of its collider, relative: 1 is the default,
    /// 8 is a crate of iron that a wooden one does not push aside.
    #[serde(default = "unit", skip_serializing_if = "is_one")]
    pub density: f32,
    /// How fast it slows by itself, per second: air, water, a sled on snow.
    /// Unity's drag.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub drag: f32,
    /// How fast its spin dies down, per second: Unity's angular drag.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub spin_drag: f32,
    /// How much gravity pulls it: 0 floats, 1 is everything else, 2 falls
    /// twice as hard.
    #[serde(default = "unit", skip_serializing_if = "is_one")]
    pub gravity: f32,
    /// How much the scene's wind carries it: 0 not at all, 1 a tumbleweed
    /// bounding over the sand. Dragged toward the wind's speed, now and
    /// then a hop.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub blown: f32,
    /// Checked between steps as well as at them, so something fast and
    /// small — a thrown stone, a bullet — cannot pass through a wall between
    /// one step and the next. Costs more; Unity's continuous collision
    /// detection.
    #[serde(default, skip_serializing_if = "is_false")]
    pub fast: bool,
    /// Axes it may not move along, as letters: `"y"` keeps it at its height.
    /// Unity's Freeze Position.
    #[serde(default, skip_serializing_if = "Axes::is_none")]
    pub freeze_move: Axes,
    /// Axes it may not turn about: `"xz"` keeps a character upright
    /// whatever knocks it. Unity's Freeze Rotation.
    #[serde(default, skip_serializing_if = "Axes::is_none")]
    pub freeze_turn: Axes,
}

/// Some of x, y and z, written as the letters: `"xz"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Axes {
    pub x: bool,
    pub y: bool,
    pub z: bool,
}

impl Axes {
    pub fn is_none(&self) -> bool {
        !(self.x || self.y || self.z)
    }
}

impl Serialize for Axes {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut text = String::new();
        for (on, letter) in [(self.x, 'x'), (self.y, 'y'), (self.z, 'z')] {
            if on {
                text.push(letter);
            }
        }
        s.serialize_str(&text)
    }
}

impl<'de> Deserialize<'de> for Axes {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        let mut axes = Axes::default();
        for c in text.chars() {
            match c {
                'x' => axes.x = true,
                'y' => axes.y = true,
                'z' => axes.z = true,
                other => {
                    return Err(serde::de::Error::custom(format!(
                        "`{other}` in \"{text}\": axes are x, y and z"
                    )))
                }
            }
        }
        Ok(axes)
    }
}

impl Default for BodyProps {
    fn default() -> Self {
        Self {
            friction: 0.5,
            bounce: 0.0,
            density: 1.0,
            drag: 0.0,
            spin_drag: 0.0,
            gravity: 1.0,
            blown: 0.0,
            fast: false,
            freeze_move: Axes::default(),
            freeze_turn: Axes::default(),
        }
    }
}

impl BodyProps {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn half() -> f32 {
    0.5
}
fn unit() -> f32 {
    1.0
}
fn is_half(v: &f32) -> bool {
    *v == 0.5
}
fn is_one(v: &f32) -> bool {
    *v == 1.0
}
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}
fn is_true(v: &bool) -> bool {
    *v
}
fn is_false(v: &bool) -> bool {
    !*v
}

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
    /// A spot light instead: shines along the entity's +z in a cone this
    /// many degrees across — a torch, a searchlight, headlights. Unity's
    /// Spot Light.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub cone_deg: Option<f32>,
    /// Casts shadows — on unless `shadows: false`, as a lamp in URP; the
    /// nearest lamps the camera sees get them first.
    #[serde(default = "yes_look", skip_serializing_if = "is_true")]
    pub shadows: bool,
    /// A lens flare on the lamp itself — a glow round it and ghosts
    /// across the picture — this bright, gone when something hides the
    /// lamp. Unity's Lens Flare (SRP) component. 0 is none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub flare: f32,
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
    /// A planar mirror instead: the entity's plane (its up the way it
    /// faces) reflects what the screen's camera sees, no `camera` of its
    /// own needed; its material shows the picture with `screen_map:
    /// Mirror`. What is behind the plane is left out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror: bool,
}

/// A sound this entity makes — the radio on the table, the fire's
/// crackle, the music of the menu: Unity's AudioSource. `sound: (clip:
/// "radio", looped: true)` plays the sound asset `radio` from here, from the
/// moment the entity is in the world and switched on, fading with distance
/// from the listener between `near` (1 m, full) and `far` (40 m, silent).
/// `spatial: false` is everywhere at `volume`: music. `pitch` 0.8 plays it
/// slower and lower. `on_start: false`
/// waits for the game to start it. `group` is the mixer group its volume
/// slider is (see [`crate::audio::Audio::set_group_volume`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundSource {
    pub clip: crate::AssetLink,
    #[serde(default = "unit", skip_serializing_if = "is_one")]
    pub volume: f32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub looped: bool,
    /// Faster and higher above 1, slower and lower below.
    #[serde(default = "unit", skip_serializing_if = "is_one")]
    pub pitch: f32,
    #[serde(default = "yes_sound", skip_serializing_if = "is_true")]
    pub on_start: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    #[serde(default = "yes_sound", skip_serializing_if = "is_true")]
    pub spatial: bool,
    #[serde(default = "unit", skip_serializing_if = "is_one")]
    pub near: f32,
    #[serde(default = "forty", skip_serializing_if = "is_forty")]
    pub far: f32,
}

impl Default for SoundSource {
    fn default() -> Self {
        Self {
            clip: crate::AssetLink::default(),
            volume: 1.0,
            looped: false,
            pitch: 1.0,
            on_start: true,
            group: String::new(),
            spatial: true,
            near: 1.0,
            far: 40.0,
        }
    }
}

fn yes_sound() -> bool {
    true
}

fn forty() -> f32 {
    40.0
}

fn is_forty(v: &f32) -> bool {
    *v == 40.0
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

/// A way an entity travels by itself — a moving platform, a lift, a boat
/// on a loop, a cart on a track: Unity's Splines with SplineAnimate.
/// `route: (points: [(0.0, 0.0, 0.0), (0.0, 4.0, 0.0)], speed: 1.5, ends:
/// Back)` — points relative to where the entity stands in its parent,
/// passed through on a smooth curve (`smooth: false` for straight legs).
/// Give it a `Kinematic` body and what stands on it rides along.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub points: Vec<Vec3>,
    /// Metres a second along the way.
    #[serde(default = "unit")]
    pub speed: f32,
    #[serde(default)]
    pub ends: RouteEnds,
    #[serde(default = "yes_route")]
    pub smooth: bool,
    /// Seconds to wait at each end (Back) or at the start of each lap (Loop).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pause: f32,
}

/// What a route does at its last point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RouteEnds {
    /// Back the way it came: a lift, a door that slides.
    #[default]
    Back,
    /// On from the last point to the first: a loop.
    Loop,
    /// Stop there: a drawbridge lowered once.
    Stop,
}

fn yes_route() -> bool {
    true
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
fn is_zero_vec3(v: &Vec3) -> bool {
    *v == Vec3::ZERO
}
fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

/// How an entity takes part in the physics world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Body {
    /// Drawn only; rapier never sees it.
    #[default]
    None,
    /// Never moves, but everything collides with it — terrain, trunks, walls.
    Static,
    /// Moved by the solver.
    Dynamic,
    /// Moved by the game — a system sets its transform — and never by the
    /// solver: it pushes dynamic bodies out of its way and is not pushed
    /// back. Platforms, lifts, doors. Unity's kinematic rigidbody.
    Kinematic,
    /// Solid to nothing; knows what is inside it. A zone — a checkpoint, a
    /// door's sensor, water — whose entity gets a
    /// [`Contacts`](crate::physics::Contacts) saying who came in and who
    /// left each step. Follows its transform like a kinematic body. Unity's
    /// `isTrigger`.
    Trigger,
}

/// One thing in the valley.
///
/// The core's fields are its identity, its place and its tree; everything
/// else on the line — `model`, `material`, `body`, `light`, `sound`… — is a
/// part a module owns, kept as the text it was written as and read by type
/// (`desc.part::<Light>()`). See [`crate::parts`] and docs/modules.md.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntityDesc {
    /// Who this is, for as long as it exists: what an edit, a selection, an
    /// undo, a merge and a network message all point at. First in the block
    /// so that every entity in a diff starts with its identity.
    ///
    /// Optional in the file: an entity written without one — by hand, or by
    /// an agent — gets one when the scene loads, and keeps it from the first
    /// save on. See [`crate::id`].
    pub id: EntityId,
    /// Shown in the editor's tree; not required to be unique.
    pub name: String,
    /// The prefab this entity is an instance of, by file stem, or empty.
    ///
    /// An instance is one line: what it is, where it stands, and what it is
    /// called. Its model and its children come from the prefab, so `model`
    /// is ignored while this is set. Expanding it is
    /// [`prefab::instantiate`](crate::prefab::instantiate), and everything
    /// downstream sees the expansion rather than the reference.
    pub prefab: crate::AssetLink,
    pub transform: Transform,
    /// Under which part of its parent's prefab it goes, when its parent is
    /// a prefab instance: the part's key, as `overrides` names parts. A
    /// scene's torch in the hand of a placed statue. Absent is under the
    /// instance itself.
    pub in_part: Option<crate::id::EntityId>,
    /// Switched off, and everything under it: not drawn, not solid, not
    /// heard — there, for the game to switch on (`world::set_active`).
    /// Unity's inactive GameObject.
    pub inactive: bool,
    /// Every field a module owns, as written: what it looks like, how it
    /// is solid, what light, sound or particles it gives off. See
    /// [`crate::parts`].
    pub parts: crate::parts::Parts,
    /// The game's own components, by the name the game registered each
    /// under (see [`crate::components`]), each value in RON:
    ///
    /// ```text
    /// components: { "door": (open_angle: 90.0), "loot": (table: "chest") },
    /// ```
    ///
    /// This is where a designer's numbers live — what Unity keeps in a
    /// MonoBehaviour's serialized fields — and there is no base class to
    /// inherit: a component is a plain struct the game registers, and a
    /// line is simply the set of them it carries. Kept as the text it was
    /// written as, so the engine never needs the game's types to load,
    /// save or diff a scene, and a value it did not touch is written back
    /// byte for byte.
    pub components: BTreeMap<String, ComponentValue>,
    /// For a prefab instance: changes to the prefab's parts in this
    /// instance only, by the part's `id` in the prefab file —
    ///
    /// ```text
    /// overrides: { "00000000000000c2": (material: "moss") },
    /// ```
    ///
    /// Each names only what differs; the rest still comes from the prefab,
    /// so a later change to the prefab reaches this instance everywhere it
    /// did not say otherwise.
    pub overrides: BTreeMap<EntityId, Override>,
    /// Things attached to this one. A child's transform is relative to its
    /// parent, so moving the parent moves the lot — which is what makes a
    /// cart with wheels, or a settler carrying a log, one thing to place
    /// rather than several to keep in step.
    ///
    /// Nested rather than a `parent:` field pointing at a name, because a
    /// tree written as a tree cannot describe a cycle or a dangling parent,
    /// and both of those are states an editor would otherwise have to guard
    /// against every time it saves.
    pub children: Vec<EntityDesc>,
}

/// What one instance changes about one part of its prefab. Every field is
/// optional in the file and in meaning: absent is "as the prefab has it".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Override {
    pub name: Option<String>,
    /// The part's whole transform, relative to its parent in the prefab.
    pub transform: Option<Transform>,
    /// Switched off (or on) in this instance.
    pub inactive: Option<bool>,
    /// A module's field set on the part — a model, a material, a body, a
    /// light — changed, or added where the prefab has none. Taking one
    /// away is `removed`.
    pub parts: crate::parts::Parts,
    /// Components set on the part, one by one.
    pub components: BTreeMap<String, ComponentValue>,
    /// What this instance takes off the part: fields by name (`"light"`,
    /// `"collider"`, `"model"`…) and the game's components by theirs —
    /// Unity's removed components. It used to be that only an edit of the
    /// prefab took things away; a prefab placed with one lamp dark and one
    /// wall without its collider is common enough in Unity (Dacha has 505)
    /// to say it here.
    pub removed: Vec<String>,
}

impl Override {
    /// Write what this override says onto a part.
    pub fn apply(&self, part: &mut EntityDesc) {
        if let Some(name) = &self.name {
            part.name = name.clone();
        }
        if let Some(transform) = self.transform {
            part.transform = transform;
        }
        if let Some(inactive) = self.inactive {
            part.inactive = inactive;
        }
        part.parts.overlay(&self.parts);
        for (name, value) in &self.components {
            part.components.insert(name.clone(), value.clone());
        }
        // A name taken away is a module's field when the line has one by
        // that name, a component otherwise.
        for name in &self.removed {
            if !part.parts.remove(name) {
                part.components.remove(name);
            }
        }
    }

    /// What it takes to turn `prefab` — the part as the prefab has it —
    /// into `edited`: only the fields that differ.
    pub fn between(prefab: &EntityDesc, edited: &EntityDesc) -> Self {
        let differs = |a: bool| a.then_some(());
        let mut parts = crate::parts::Parts::default();
        for (name, text) in edited.parts.iter() {
            if prefab.parts.raw(name) != Some(text) {
                let _ = parts.set_raw(name, text);
            }
        }
        Override {
            name: differs(prefab.name != edited.name).map(|_| edited.name.clone()),
            transform: differs(prefab.transform != edited.transform).map(|_| edited.transform),
            inactive: differs(prefab.inactive != edited.inactive).map(|_| edited.inactive),
            parts,
            components: edited
                .components
                .iter()
                .filter(|(name, value)| prefab.components.get(*name) != Some(*value))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            removed: prefab
                .parts
                .names()
                .filter(|name| !edited.parts.contains(name))
                .map(str::to_string)
                .chain(
                    prefab
                        .components
                        .keys()
                        .filter(|name| !edited.components.contains_key(*name))
                        .cloned(),
                )
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Override::default()
    }

    /// Add `later` on top: what it says wins, what it does not say stays.
    pub fn merge(&mut self, later: Override) {
        let Override {
            name,
            transform,
            inactive,
            parts,
            components,
            removed,
        } = later;
        self.name = name.or(self.name.take());
        self.transform = transform.or(self.transform);
        self.inactive = inactive.or(self.inactive);
        // Set again after it was taken away: back.
        self.removed
            .retain(|name| !components.contains_key(name) && !parts.contains(name));
        self.parts.overlay(&parts);
        self.components.extend(components);
        for name in removed {
            self.components.remove(&name);
            self.parts.remove(&name);
            if !self.removed.contains(&name) {
                self.removed.push(name);
            }
        }
    }
}

/// An optional field written as its value, not as `Some(value)`: absent
/// means `None`, present means `Some`. RON would otherwise want `Some(…)`
/// spelled out around every override, which is noise to read and a trap to
/// write.
mod plain {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(
        value: &Option<T>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(value) => value.serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<T>, D::Error> {
        T::deserialize(d).map(Some)
    }
}

/// One component's value, in RON, as written.
pub type ComponentValue = Box<ron::value::RawValue>;

impl EntityDesc {
    /// Set a component's value from RON text. The text is checked to be
    /// RON, not to fit the component: the engine does not know the game's
    /// types, and the game says so when it reads the scene.
    pub fn set_component(&mut self, name: &str, ron: &str) -> Result<(), String> {
        let value = ron::value::RawValue::from_boxed_ron(ron.trim().into())
            .map_err(|e| format!("component {name}: {e}"))?;
        self.components.insert(name.to_string(), value);
        Ok(())
    }

    /// This entity and everything under it, depth first, each with the
    /// transform that stacks its ancestors' on top of its own.
    pub fn flatten(&self) -> Vec<(&EntityDesc, glam::Mat4)> {
        let mut out = Vec::new();
        self.flatten_into(glam::Mat4::IDENTITY, &mut out);
        out
    }

    pub(crate) fn flatten_into<'a>(
        &'a self,
        parent: glam::Mat4,
        out: &mut Vec<(&'a EntityDesc, glam::Mat4)>,
    ) {
        let world = parent * self.transform.matrix();
        out.push((self, world));
        for child in &self.children {
            child.flatten_into(world, out);
        }
    }
}

/// How an entity names its surface.
///
/// Not an `Option`: RON wants `Some(...)` spelled out around an optional
/// field, and `material: Some("grass")` is noise in every line of every
/// scene. A default variant costs nothing and reads better.
// A line of a scene, not a frame's data: the inline material's size does
// not matter, and a box around it would be in every match on it.
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

impl EntityDesc {
    /// The material to draw with, using the engine's builtins only.
    ///
    /// What a test and the reference scene want, since neither has a library.
    /// Anything that does have one calls [`EntityDesc::material_from`].
    pub fn material(&self) -> Material {
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
    pub fn material_from(
        &self,
        lookup: impl Fn(&crate::AssetLink) -> Option<Material>,
    ) -> Material {
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

/// Mint IDs for a subtree: unassigned ones, and ones already in `seen`.
///
/// Fresh random IDs, for things being created: an added or duplicated
/// entity is a new thing and gets a new identity.
pub(crate) fn assign_ids(
    entities: &mut [EntityDesc],
    seen: &mut std::collections::HashSet<EntityId>,
) -> usize {
    assign(entities, seen, None)
}

/// As [`assign_ids`], but an entity with no ID gets one derived from where
/// it stands — its parent's ID, its name, and how many siblings before it
/// share that name — rather than a random one.
///
/// For files read from disk. A hand-written entity without an ID then gets
/// the same one every time its file is read, so reloading the file while
/// the game runs finds it again rather than replacing it with a stranger.
/// Inserting a sibling above it does not change it; renaming it does. It is
/// a stopgap until the file is saved and the ID written down, not a second
/// kind of identity: a repeated ID is still re-minted at random.
pub(crate) fn derive_ids(
    entities: &mut [EntityDesc],
    seen: &mut std::collections::HashSet<EntityId>,
) -> usize {
    assign(entities, seen, Some(EntityId::UNASSIGNED))
}

fn assign(
    entities: &mut [EntityDesc],
    seen: &mut std::collections::HashSet<EntityId>,
    parent: Option<EntityId>,
) -> usize {
    let mut minted = 0;
    let mut same_name: std::collections::HashMap<String, u64> = Default::default();
    for entity in entities {
        let nth = same_name.entry(entity.name.clone()).or_insert(0);
        let position = position_key(&entity.name, *nth);
        *nth += 1;
        if entity.id.is_unassigned() || seen.contains(&entity.id) {
            let derived = parent
                .filter(|_| entity.id.is_unassigned())
                .map(|parent| parent.within(EntityId::from_raw(position)));
            let mut id = derived.unwrap_or_else(EntityId::fresh);
            while seen.contains(&id) {
                id = EntityId::fresh();
            }
            entity.id = id;
            minted += 1;
        }
        seen.insert(entity.id);
        minted += assign(&mut entity.children, seen, parent.map(|_| entity.id));
    }
    minted
}

/// FNV-1a over the name, then the count: specified, so every build derives
/// the same ID from the same file.
fn position_key(name: &str, nth: u64) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes().chain(nth.to_le_bytes()) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A whole scene, as it sits on disk: how it looks — `view`, `sun`, `fog`,
/// `post`… — and what is in it.
///
/// Its look is the render module's, kept as parts the way a line's module
/// fields are ([`crate::parts`]): `scene.part::<Sun>()`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene {
    pub parts: crate::parts::Parts,
    pub entities: Vec<EntityDesc>,
}

/// A component's link to an entity the scene does not have.
#[derive(Debug, Clone, PartialEq)]
pub struct BrokenLink {
    /// Who holds the link, by id and name, and in which component.
    pub holder: EntityId,
    pub holder_name: String,
    pub component: String,
    /// The entity it names.
    pub target: EntityId,
}

/// A curve through points, in its entity's space. Straight between the
/// points for now; the points are what is edited.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Spline {
    pub points: Vec<Vec3>,
    /// Back from the last point to the first: a fence round a field.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
}

impl Spline {
    /// Where things go along it, `spacing` apart from the first point, and
    /// which way the curve runs there: `(position, direction)`.
    pub fn stations(&self, spacing: f32) -> Vec<(Vec3, Vec3)> {
        let spacing = spacing.max(0.05);
        let mut points = self.points.clone();
        if self.closed && points.len() > 2 {
            points.push(points[0]);
        }
        let mut out = Vec::new();
        // How far into the next segment the next station is.
        let mut carry = 0.0;
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let length = (b - a).length();
            if length < 1e-4 {
                continue;
            }
            let direction = (b - a) / length;
            let mut at = carry;
            while at <= length + 1e-4 {
                out.push((a + direction * at, direction));
                at += spacing;
                if out.len() >= 10_000 {
                    return out;
                }
            }
            carry = at - length;
        }
        out
    }
}

/// What a spline carries: copies of a model, `spacing` apart, each turned
/// to face along the curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Along {
    pub model: crate::AssetLink,
    #[serde(default = "one_metre")]
    pub spacing: f32,
    /// The copies' scale.
    #[serde(default = "unit_scale", skip_serializing_if = "is_unit_scale")]
    pub scale: Vec3,
}

fn one_metre() -> f32 {
    1.0
}

fn unit_scale() -> Vec3 {
    Vec3::ONE
}

fn is_unit_scale(v: &Vec3) -> bool {
    *v == Vec3::ONE
}

impl Along {
    /// The copies for a spline, as children of the entity carrying both:
    /// named after the model and numbered, with IDs derived from the
    /// carrier's and their number, so the same fence gives the same IDs.
    pub fn grow(&self, carrier: EntityId, spline: &Spline) -> Vec<EntityDesc> {
        let name = self.model.trim_start_matches("builtin:").to_string();
        spline
            .stations(self.spacing)
            .into_iter()
            .enumerate()
            .map(|(i, (position, direction))| {
                let mut transform = Transform {
                    position,
                    scale: self.scale,
                    ..Default::default()
                };
                let yaw = (-direction.z).atan2(direction.x);
                transform.set_rotation(glam::Quat::from_rotation_y(yaw));
                let mut copy = EntityDesc {
                    id: carrier.within(EntityId::from_raw(i as u64 + 1)),
                    name: format!("{name} {}", i + 1),
                    transform,
                    ..Default::default()
                };
                copy.set_model(self.model.clone());
                copy
            })
            .collect()
    }
}

impl Scene {
    /// Every [`crate::EntityRef`] in a component whose entity is not in the
    /// scene. Asked of an expanded scene, so that a link to a part of a
    /// prefab instance counts as there.
    pub fn broken_links(&self) -> Vec<BrokenLink> {
        let all = self.flatten();
        let ids: std::collections::HashSet<EntityId> = all.iter().map(|(e, _)| e.id).collect();
        let mut out = Vec::new();
        for (desc, _) in &all {
            for (component, value) in &desc.components {
                for target in crate::EntityRef::find_in(value.get_ron()) {
                    if !ids.contains(&target) {
                        out.push(BrokenLink {
                            holder: desc.id,
                            holder_name: desc.name.clone(),
                            component: component.clone(),
                            target,
                        });
                    }
                }
            }
        }
        out
    }

    /// Every entity in the scene, roots and descendants alike, each with the
    /// world transform its ancestors give it.
    ///
    /// Anything that asks "what is in this scene" wants this rather than
    /// `entities`, which holds only the roots. An editor's tree, a search, a
    /// count — all of them get the nesting wrong exactly once and then use
    /// this.
    pub fn flatten(&self) -> Vec<(&EntityDesc, glam::Mat4)> {
        let mut out = Vec::new();
        for entity in &self.entities {
            entity.flatten_into(glam::Mat4::IDENTITY, &mut out);
        }
        out
    }

    /// The first entity with this name, at any depth.
    ///
    /// For people and tests. Names are not unique; anything that has to hit
    /// the same entity twice holds its [`EntityId`] and uses [`Scene::get`].
    pub fn find(&self, name: &str) -> Option<&EntityDesc> {
        self.flatten()
            .into_iter()
            .find(|(e, _)| e.name == name)
            .map(|(e, _)| e)
    }

    /// The entity with this ID, at any depth.
    pub fn get(&self, id: EntityId) -> Option<&EntityDesc> {
        fn walk(entities: &[EntityDesc], id: EntityId) -> Option<&EntityDesc> {
            entities
                .iter()
                .find_map(|e| (e.id == id).then_some(e).or_else(|| walk(&e.children, id)))
        }
        walk(&self.entities, id)
    }

    /// The entity with this ID, at any depth, to change.
    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut EntityDesc> {
        fn walk(entities: &mut [EntityDesc], id: EntityId) -> Option<&mut EntityDesc> {
            for entity in entities {
                if entity.id == id {
                    return Some(entity);
                }
                if let Some(found) = walk(&mut entity.children, id) {
                    return Some(found);
                }
            }
            None
        }
        walk(&mut self.entities, id)
    }

    /// Every entity's ID, in the order [`Scene::flatten`] walks them — which
    /// is the order an editor's tree shows them in.
    pub fn ids(&self) -> Vec<EntityId> {
        self.flatten().into_iter().map(|(e, _)| e.id).collect()
    }

    /// Give every entity without an ID one, and every entity whose ID is
    /// already taken a new one. Returns how many were minted.
    ///
    /// Called on load, so nothing downstream ever sees an entity it cannot
    /// name. A missing ID is derived from the entity's place in the file,
    /// so reading the same file twice names it the same way. A repeated ID is re-minted rather than trusted: it comes from a
    /// block copy-pasted by hand or a merge gone strange, and two entities
    /// answering to one name is how an edit lands on the wrong thing. The
    /// first one in the file keeps it.
    pub fn assign_ids(&mut self) -> usize {
        derive_ids(&mut self.entities, &mut std::collections::HashSet::new())
    }

    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        // The path goes into the message: ron says where in the file and what
        // it expected, and without the file that is half an answer.
        let mut scene: Scene =
            ron::from_str(&text).map_err(|e| anyhow::anyhow!("{}:{e}", path.display()))?;
        let minted = scene.assign_ids();
        if minted > 0 {
            tracing::debug!(
                "{}: gave {minted} entities an id; they are written on the next save",
                path.display()
            );
        }
        Ok(scene)
    }

    /// Write the scene back out, pretty-printed so that a diff is readable.
    ///
    /// This is the editor's save button, and the reason the editor never has
    /// to be the only way to change a scene.
    pub fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        // Struct names deliberately off. With them on, a material is written
        // as `Material(...)`, and an untagged enum cannot match a named
        // struct — so a scene the editor saved would not open again. A
        // round trip that only fails on the way back is the worst kind.
        //
        // What the file already says is kept as it says it: comments,
        // spacing, one-line entities stay, and only what changed is
        // rewritten (see `ron_text`). A trailing newline, because every text
        // editor adds one.
        let pretty = ron::ser::PrettyConfig::new().depth_limit(4);
        crate::ron_text::write_preserving(path.as_ref(), self, pretty)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_with_every_field_set_survives_a_round_trip() {
        // The round trip that matters is the one an editor does: write a
        // scene out, open it again, get the same thing. It failed for a
        // while because struct names were on and an untagged enum cannot
        // match a named struct — a file that saved cleanly and would not
        // reopen.
        let mut scene = Scene {
            entities: vec![EntityDesc {
                parts: Default::default(),
                in_part: None,
                inactive: false,
                overrides: Default::default(),
                components: Default::default(),
                id: Default::default(),
                name: "crate".into(),
                prefab: Default::default(),
                transform: Transform {
                    position: Vec3::new(1.0, 2.0, 3.0),
                    rotation_deg: Vec3::new(0.0, 45.0, 0.0),
                    scale: Vec3::splat(2.0),
                },
                children: vec![EntityDesc {
                    parts: Default::default(),
                    in_part: None,
                    inactive: false,
                    overrides: Default::default(),
                    components: Default::default(),
                    id: Default::default(),
                    name: "lid".into(),
                    prefab: Default::default(),
                    transform: Transform::default(),
                    children: Vec::new(),
                }
                .with(crate::scene::ModelRef("builtin:cube".into()))
                .with(MaterialRef::Named("stone".into()))
                .with(Body::None)
                .with(Collider::None)],
            }
            .with(crate::scene::ModelRef("builtin:cube".into()))
            .with(MaterialRef::Inline(Material::new(0.3, 0.2, 0.1)))
            .with(Body::Dynamic)
            .with(Collider::Box {
                half: Vec3::splat(0.5),
                center: glam::Vec3::ZERO,
            })],
            ..Default::default()
        };
        // IDs as a loaded scene would have them: code-built entities start
        // unassigned, and a round trip would otherwise compare minted IDs
        // against zeros.
        scene.assign_ids();
        let dir = std::env::temp_dir().join("runity-scene-full");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("full.ron");
        scene.save(&path).unwrap();
        assert_eq!(Scene::load(&path).unwrap(), scene);
    }

    #[test]
    fn a_scene_survives_a_round_trip_through_the_file() {
        let mut scene = Scene {
            parts: Default::default(),
            entities: vec![EntityDesc {
                parts: Default::default(),
                in_part: None,
                inactive: false,
                overrides: Default::default(),
                components: Default::default(),
                id: Default::default(),
                name: "pine".into(),
                prefab: Default::default(),
                transform: Transform {
                    position: Vec3::new(1.0, 0.0, -3.0),
                    rotation_deg: Vec3::new(0.0, 45.0, 0.0),
                    scale: Vec3::splat(1.2),
                },
                children: Vec::new(),
            }
            .with(crate::scene::ModelRef("models/pine_large.obj".into()))
            .with(MaterialRef::Named("needle".into()))
            .with(Body::Static)
            .with(Collider::None)],
        }
        .with(View::default())
        .with(Sun {
            hour: 17.5,
            intensity: 0.8,
            ..Sun::default()
        })
        .with(Fog::default())
        .with(crate::post::PostProcess {
            saturation: -30.0,
            ..Default::default()
        });
        scene.assign_ids();
        let dir = std::env::temp_dir().join("runity-scene-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.ron");
        scene.save(&path).unwrap();
        assert_eq!(Scene::load(&path).unwrap(), scene);
    }

    #[test]
    fn saving_a_scene_twice_writes_the_same_bytes() {
        // DNA, postulate 2: a save with no changes is a zero diff. IDs,
        // field order and number formatting all have to be deterministic for
        // that, and any one of them drifting shows up as noise in every
        // commit an editor makes.
        let dir = std::env::temp_dir().join("runity-scene-zero-diff");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.ron");
        std::fs::write(
            &path,
            r#"(entities: [
                (name: "crate", model: "builtin:cube",
                 transform: (position: (0.1, 0.2, 0.3), rotation_deg: (0.0, 33.3, 0.0)),
                 children: [(name: "lid", model: "builtin:cube")]),
                (name: "rock", model: "builtin:sphere", material: "stone"),
            ])"#,
        )
        .unwrap();

        let first = Scene::load(&path).unwrap();
        first.save(&path).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        Scene::load(&path).unwrap().save(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), written);
    }

    #[test]
    fn an_entity_written_without_an_id_gets_one_and_keeps_it() {
        // A person or an agent writing a scene by hand should not have to
        // invent IDs. The first load gives them, the first save writes them,
        // and from then on they are the entity's.
        let dir = std::env::temp_dir().join("runity-scene-ids");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.ron");
        std::fs::write(
            &path,
            r#"(entities: [(name: "a", model: "m", children: [(name: "b", model: "m")])])"#,
        )
        .unwrap();

        let scene = Scene::load(&path).unwrap();
        let ids = scene.ids();
        assert!(ids.iter().all(|id| !id.is_unassigned()), "{ids:?}");
        scene.save(&path).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("id: \""));
        assert_eq!(Scene::load(&path).unwrap().ids(), ids, "and keeps them");
    }

    #[test]
    fn an_entity_without_an_id_is_named_the_same_way_on_every_read() {
        // A running game reloads a hand-written scene every time it is
        // saved. If each read minted new IDs, every entity without one would
        // look new each time and be respawned, losing whatever the game had
        // done to it.
        let text = |extra: &str| {
            format!(
                r#"(entities: [{extra}
                    (name: "tree", model: "m"),
                    (name: "tree", model: "m", children: [(name: "leaf", model: "m")]),
                ])"#
            )
        };
        let read = |text: &str| {
            let mut scene: Scene = ron::from_str(text).unwrap();
            scene.assign_ids();
            scene
        };
        let first = read(&text(""));
        assert_eq!(
            first.ids(),
            read(&text("")).ids(),
            "the same file, the same IDs"
        );
        let ids = first.ids();
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            3,
            "two trees with one name are still two entities: {ids:?}"
        );

        // A sibling written above them does not move their identities.
        let inserted = read(&text(r#"(name: "rock", model: "m"),"#));
        assert_eq!(inserted.ids()[1..], ids[..]);
    }

    #[test]
    fn a_repeated_id_is_re_minted_and_the_first_keeps_it() {
        // A block copy-pasted by hand, or a merge gone strange. Two entities
        // answering to one ID is how an edit lands on the wrong one.
        let mut scene: Scene = ron::from_str(
            r#"(entities: [
                (id: "a1", name: "first", model: "m"),
                (id: "a1", name: "pasted", model: "m"),
            ])"#,
        )
        .unwrap();
        assert_eq!(scene.assign_ids(), 1);
        assert_eq!(scene.find("first").unwrap().id, "a1".parse().unwrap());
        assert_ne!(
            scene.find("pasted").unwrap().id,
            scene.find("first").unwrap().id
        );
    }

    #[test]
    fn an_entity_is_found_by_id_at_any_depth() {
        let mut scene: Scene = ron::from_str(
            r#"(entities: [(id: "1", name: "a", model: "m", children: [(id: "2", name: "b", model: "m")])])"#,
        )
        .unwrap();
        let deep = "2".parse().unwrap();
        assert_eq!(scene.get(deep).map(|e| e.name.as_str()), Some("b"));
        scene.get_mut(deep).unwrap().name = "renamed".into();
        assert_eq!(scene.find("renamed").unwrap().id, deep);
        assert!(scene.get(crate::EntityId::from_raw(3)).is_none());
    }

    #[test]
    fn euler_degrees_survive_the_trip_through_a_quaternion() {
        let t = Transform {
            rotation_deg: glam::Vec3::new(10.0, -35.0, 0.0),
            ..Transform::default()
        };
        let q = t.rotation();
        let mut back = Transform::default();
        back.set_rotation(q);
        assert!((back.rotation_deg - t.rotation_deg).length() < 0.01);
    }

    #[test]
    fn flatten_reaches_every_depth_and_stacks_the_transforms() {
        let scene: Scene = ron::from_str(
            r#"(entities: [(
                name: "cart", model: "m", transform: (position: (10.0, 0.0, 0.0)),
                children: [(
                    name: "wheel", model: "m", transform: (position: (1.0, 0.0, 0.0)),
                    children: [(name: "bolt", model: "m", transform: (position: (0.5, 0.0, 0.0)))],
                )],
            )])"#,
        )
        .unwrap();

        assert_eq!(scene.entities.len(), 1, "one root");
        let all = scene.flatten();
        assert_eq!(all.len(), 3, "three entities once the nesting is walked");
        let bolt = scene.find("bolt").expect("found at depth two");
        assert_eq!(bolt.transform.position.x, 0.5, "its own transform is local");
        let bolt_world = all
            .iter()
            .find(|(e, _)| e.name == "bolt")
            .map(|(_, m)| m.w_axis.x)
            .unwrap();
        assert_eq!(bolt_world, 11.5, "10 + 1 + 0.5");
    }

    #[test]
    fn a_material_is_named_or_spelled_out_and_a_typo_still_opens() {
        let named: Scene =
            ron::from_str(r#"(entities: [(name: "a", model: "m", material: "grass")])"#).unwrap();
        assert_eq!(
            named.entities[0].material(),
            crate::material::builtin::GRASS
        );

        let inline: Scene = ron::from_str(
            r#"(entities: [(name: "a", model: "m", material: (base_color: (0.5, 0.1, 0.1)))])"#,
        )
        .unwrap();
        assert_eq!(inline.entities[0].material().base_color, [0.5, 0.1, 0.1]);

        let typo: Scene =
            ron::from_str(r#"(entities: [(name: "a", model: "m", material: "grsas")])"#).unwrap();
        assert_eq!(typo.entities[0].material(), Material::default());
    }

    #[test]
    fn the_sun_rises_crosses_and_sets_and_warms_at_both_ends() {
        let at = |hour| Sun {
            hour,
            intensity: 1.0,
            ..Sun::default()
        };

        // Noon is overhead; morning and evening are low and on opposite
        // sides, which is what makes shadows point somewhere believable.
        let noon = at(12.0).direction();
        let morning = at(7.0).direction();
        let evening = at(17.0).direction();
        assert!(noon.y < morning.y && noon.y < evening.y, "noon is highest");
        assert!(
            morning.x.signum() != evening.x.signum(),
            "the sun should cross the sky, not wander back: {morning} then {evening}"
        );

        // Never exactly on the horizon: a sun lying flat lights nothing but
        // the horizon and turns every shadow into a stripe to the far plane.
        for hour in [0.0, 5.0, 6.0, 18.0, 23.0] {
            assert!(at(hour).direction().y < -0.1, "at {hour}");
        }

        // Warm low, white high. Not physics — the one cue that reads as a
        // time of day at a glance.
        assert!(at(12.0).color().z > at(7.0).color().z, "noon is the bluest");
        assert!(at(7.0).color().x >= at(7.0).color().z, "dawn is orange");
    }

    #[test]
    fn a_scene_remembers_where_it_is_looked_at_from() {
        let scene = Scene { ..Scene::default() }.with(View {
            position: Vec3::new(3.0, 9.0, -2.0),
            target: Vec3::new(0.0, 1.0, 0.0),
            fov_deg: 35.0,
        });
        let dir = std::env::temp_dir().join("runity-scene-view");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("view.ron");
        scene.save(&path).unwrap();
        assert_eq!(Scene::load(&path).unwrap().view(), scene.view());

        // And a scene written before there was a camera in the format still
        // opens, framed the way everything used to be.
        let older: Scene = ron::from_str(r#"(entities: [])"#).unwrap();
        assert_eq!(older.view(), View::default());
    }

    #[test]
    fn a_missing_field_falls_back_rather_than_failing_to_load() {
        // An agent writing a scene by hand should not have to spell out
        // every default, and an older file should still open.
        let text = r#"(entities: [(name: "rock", model: "models/boulder.obj")])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        assert_eq!(scene.entities[0].transform, Transform::default());
        assert_eq!(scene.entities[0].body(), Body::None);
        assert_eq!(scene.entities[0].material(), Material::default());
        assert_eq!(scene.sun().hour, 9.0);
    }
}

// ---------------------------------------------------------------------------
// A line of a scene, a prefab instance's override and a whole scene, read
// and written by hand: the core's fields by type, every other field as a
// part (crate::parts), in the order the file has them.

use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeStruct;

/// Read a struct-like RON value field by field: `core` takes the fields it
/// knows and says so; every other field is kept as a part.
fn read_fields<'de, A, F>(
    mut map: A,
    parts: &mut crate::parts::Parts,
    mut core: F,
) -> Result<(), A::Error>
where
    A: MapAccess<'de>,
    F: FnMut(&str, &mut A) -> Result<bool, A::Error>,
{
    while let Some(key) = map.next_key::<String>()? {
        if !core(&key, &mut map)? {
            let value: crate::parts::PartValue = map.next_value()?;
            parts.put(&key, value);
        }
    }
    Ok(())
}

fn write_parts<S: SerializeStruct>(
    st: &mut S,
    parts: &crate::parts::Parts,
) -> Result<(), S::Error> {
    for (name, value) in parts.entries() {
        st.serialize_field(crate::parts::static_name(name), value)?;
    }
    Ok(())
}

fn trim_components(raw: BTreeMap<String, ComponentValue>) -> BTreeMap<String, ComponentValue> {
    raw.into_iter()
        .map(|(name, value)| (name, value.trim_boxed()))
        .collect()
}

impl Serialize for EntityDesc {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("EntityDesc", 4 + self.parts.len())?;
        if !self.id.is_unassigned() {
            st.serialize_field("id", &self.id)?;
        }
        st.serialize_field("name", &self.name)?;
        if !self.prefab.is_empty() {
            st.serialize_field("prefab", &self.prefab)?;
        }
        st.serialize_field("transform", &self.transform)?;
        write_parts(&mut st, &self.parts)?;
        if let Some(part) = &self.in_part {
            st.serialize_field("in_part", part)?;
        }
        if self.inactive {
            st.serialize_field("inactive", &true)?;
        }
        if !self.components.is_empty() {
            st.serialize_field("components", &self.components)?;
        }
        if !self.overrides.is_empty() {
            st.serialize_field("overrides", &self.overrides)?;
        }
        st.serialize_field("children", &self.children)?;
        st.end()
    }
}

impl<'de> Deserialize<'de> for EntityDesc {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Line;
        impl<'de> Visitor<'de> for Line {
            type Value = EntityDesc;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an entity: `(name: …, transform: …, …)`")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<EntityDesc, A::Error> {
                let mut e = EntityDesc::default();
                let mut named = false;
                let mut parts = crate::parts::Parts::default();
                read_fields(map, &mut parts, |key, map| {
                    match key {
                        "id" => e.id = map.next_value()?,
                        "name" => {
                            e.name = map.next_value()?;
                            named = true;
                        }
                        "prefab" => e.prefab = map.next_value()?,
                        "transform" => e.transform = map.next_value()?,
                        "in_part" => e.in_part = Some(map.next_value()?),
                        "inactive" => e.inactive = map.next_value()?,
                        "components" => e.components = trim_components(map.next_value()?),
                        "overrides" => e.overrides = map.next_value()?,
                        "children" => e.children = map.next_value()?,
                        _ => return Ok(false),
                    }
                    Ok(true)
                })?;
                if !named {
                    return Err(serde::de::Error::missing_field("name"));
                }
                e.parts = parts;
                Ok(e)
            }
        }
        d.deserialize_struct(
            "EntityDesc",
            &["id", "name", "prefab", "transform", "children"],
            Line,
        )
    }
}

impl Serialize for Override {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Override", 2 + self.parts.len())?;
        if let Some(name) = &self.name {
            st.serialize_field("name", name)?;
        }
        if let Some(transform) = &self.transform {
            st.serialize_field("transform", transform)?;
        }
        write_parts(&mut st, &self.parts)?;
        if let Some(inactive) = &self.inactive {
            st.serialize_field("inactive", inactive)?;
        }
        if !self.components.is_empty() {
            st.serialize_field("components", &self.components)?;
        }
        if !self.removed.is_empty() {
            st.serialize_field("removed", &self.removed)?;
        }
        st.end()
    }
}

impl<'de> Deserialize<'de> for Override {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Change;
        impl<'de> Visitor<'de> for Change {
            type Value = Override;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("what an instance changes: `(material: …, …)`")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Override, A::Error> {
                let mut o = Override::default();
                let mut parts = crate::parts::Parts::default();
                read_fields(map, &mut parts, |key, map| {
                    match key {
                        "name" => o.name = Some(map.next_value()?),
                        "transform" => o.transform = Some(map.next_value()?),
                        "inactive" => o.inactive = Some(map.next_value()?),
                        "components" => o.components = trim_components(map.next_value()?),
                        "removed" => o.removed = map.next_value()?,
                        _ => return Ok(false),
                    }
                    Ok(true)
                })?;
                o.parts = parts;
                Ok(o)
            }
        }
        d.deserialize_struct("Override", &["name", "transform"], Change)
    }
}

impl Serialize for Scene {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Scene", 1 + self.parts.len())?;
        write_parts(&mut st, &self.parts)?;
        st.serialize_field("entities", &self.entities)?;
        st.end()
    }
}

impl<'de> Deserialize<'de> for Scene {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Whole;
        impl<'de> Visitor<'de> for Whole {
            type Value = Scene;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a scene: `(view: …, sun: …, entities: [ … ])`")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Scene, A::Error> {
                let mut scene = Scene::default();
                let mut parts = crate::parts::Parts::default();
                read_fields(map, &mut parts, |key, map| {
                    if key == "entities" {
                        scene.entities = map.next_value()?;
                        return Ok(true);
                    }
                    Ok(false)
                })?;
                scene.parts = parts;
                Ok(scene)
            }
        }
        d.deserialize_struct("Scene", &["entities"], Whole)
    }
}

impl EntityDesc {
    /// A module's field on this line, by its type: `None` when absent or
    /// when the text does not fit (which [`EntityDesc::try_part`] says).
    pub fn part<T: crate::parts::Part>(&self) -> Option<T> {
        self.parts.get()
    }

    /// A module's field, or its default when the line leaves it out.
    pub fn part_or_default<T: crate::parts::Part + Default>(&self) -> T {
        self.parts.get().unwrap_or_default()
    }

    /// A module's field, with the error in words when its text does not
    /// fit its type.
    pub fn try_part<T: crate::parts::Part>(&self) -> Result<Option<T>, String> {
        self.parts.try_get()
    }

    /// Set a module's field; its default takes it off the line.
    pub fn set_part<T: crate::parts::Part>(&mut self, value: &T) {
        self.parts.set(value)
    }

    /// Set a module's field, or take it off with `None`.
    pub fn set_part_opt<T: crate::parts::Part>(&mut self, value: Option<&T>) {
        self.parts.set_opt(value)
    }

    /// Take a module's field off the line.
    pub fn clear_part<T: crate::parts::Part>(&mut self) {
        self.parts.remove(T::NAME);
    }

    /// This line with a module's field set: `EntityDesc { name, ..
    /// }.with(Body::Static).with(Collider::Box { .. })`.
    pub fn with<T: crate::parts::Part>(mut self, value: T) -> Self {
        self.set_part(&value);
        self
    }

    /// This line with a module's field set, when there is one.
    pub fn with_opt<T: crate::parts::Part>(mut self, value: Option<T>) -> Self {
        self.set_part_opt(value.as_ref());
        self
    }

    /// This line without a module's field, by name: `.without("light")`.
    pub fn without(mut self, name: &str) -> Self {
        self.parts.remove(name);
        self
    }

    /// This line with its model: `.with_model("pine")`.
    pub fn with_model(self, model: impl Into<crate::AssetLink>) -> Self {
        self.with(ModelRef(model.into()))
    }
}

impl Override {
    pub fn part<T: crate::parts::Part>(&self) -> Option<T> {
        self.parts.get()
    }

    pub fn set_part<T: crate::parts::Part>(&mut self, value: &T) {
        // An override says what differs, default or not: it is not left
        // out for being the default.
        let _ = self.parts.set_raw(T::NAME, &crate::parts::to_text(value));
    }

    pub fn set_part_opt<T: crate::parts::Part>(&mut self, value: Option<&T>) {
        match value {
            Some(value) => self.set_part(value),
            None => {
                self.parts.remove(T::NAME);
            }
        }
    }

    /// This change with a module's field set.
    pub fn with<T: crate::parts::Part>(mut self, value: T) -> Self {
        self.set_part(&value);
        self
    }

    pub fn with_opt<T: crate::parts::Part>(mut self, value: Option<T>) -> Self {
        self.set_part_opt(value.as_ref());
        self
    }
}

impl Scene {
    /// A field of the scene's look, by its type.
    pub fn part<T: crate::parts::Part>(&self) -> Option<T> {
        self.parts.get()
    }

    pub fn part_or_default<T: crate::parts::Part + Default>(&self) -> T {
        self.parts.get().unwrap_or_default()
    }

    pub fn set_part<T: crate::parts::Part>(&mut self, value: &T) {
        self.parts.set(value)
    }

    pub fn set_part_opt<T: crate::parts::Part>(&mut self, value: Option<&T>) {
        self.parts.set_opt(value)
    }

    /// This scene with a field of its look set.
    pub fn with<T: crate::parts::Part>(mut self, value: T) -> Self {
        self.set_part(&value);
        self
    }

    pub fn with_opt<T: crate::parts::Part>(mut self, value: Option<T>) -> Self {
        self.set_part_opt(value.as_ref());
        self
    }
}

// The fields of a line the modules own, each a type that says its name.

/// `model: "pine_large"` — a model in `assets/` by file stem, or a builtin
/// (`builtin:cone`). A group, or a prefab instance, draws nothing itself.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelRef(pub crate::AssetLink);

impl crate::parts::Part for ModelRef {
    const NAME: &'static str = "model";
    fn is_default(&self) -> bool {
        self.0.is_empty()
    }
}

/// `animator: "door"` — the graph in `animators/` that moves it and the
/// things under it, with the clips in `clips/` (see [`crate::motion`]).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AnimatorRef(pub String);

impl crate::parts::Part for AnimatorRef {
    const NAME: &'static str = "animator";
    fn is_default(&self) -> bool {
        self.0.is_empty()
    }
}

/// `layer: "props"` — the collision layer, by the name `layers.ron` gives
/// it; absent is `default`. See [`crate::layers`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LayerName(pub String);

impl crate::parts::Part for LayerName {
    const NAME: &'static str = "layer";
    fn is_default(&self) -> bool {
        self.0.is_empty()
    }
}

/// `bone: "hand.R"` — held by this joint of the parent's skeleton, not by
/// the parent itself: a spade in a hand, a hat on a head. `transform` is
/// then relative to the bone.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BoneName(pub String);

impl crate::parts::Part for BoneName {
    const NAME: &'static str = "bone";
    fn is_default(&self) -> bool {
        self.0.is_empty()
    }
}

/// `bends_grass: 0.6` — grass and anything else that sways is pushed aside
/// within this many metres of it. See [`crate::foliage`].
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BendsGrass(pub f32);

impl crate::parts::Part for BendsGrass {
    const NAME: &'static str = "bends_grass";
    fn is_default(&self) -> bool {
        self.0 == 0.0
    }
}

/// `joint_break: 400.0` — the joint breaks when pulled harder than this
/// many newtons: Unity's Break Force (`PhysicsWorld::broken`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JointBreak(pub f32);

impl crate::parts::Part for JointBreak {
    const NAME: &'static str = "joint_break";
}

macro_rules! parts {
    ($($ty:ty => $name:literal $(, default if $default:expr)?;)*) => {
        $(
            impl crate::parts::Part for $ty {
                const NAME: &'static str = $name;
                $(fn is_default(&self) -> bool {
                    let check: fn(&Self) -> bool = $default;
                    check(self)
                })?
            }
        )*

        /// Every field of a line, an override or a scene's look the engine's
        /// modules read in this build, with how to check its text: what
        /// `check` names a field by that no module reads.
        pub fn part_kinds() -> Vec<crate::parts::PartKind> {
            let mut kinds = vec![
                crate::parts::PartKind::of::<ModelRef>(),
                crate::parts::PartKind::of::<AnimatorRef>(),
                crate::parts::PartKind::of::<LayerName>(),
                crate::parts::PartKind::of::<BoneName>(),
                crate::parts::PartKind::of::<BendsGrass>(),
                crate::parts::PartKind::of::<JointBreak>(),
            ];
            $(kinds.push(crate::parts::PartKind::of::<$ty>());)*
            kinds
        }
    };
}

parts! {
    MaterialRef => "material", default if |m| m.is_default();
    Body => "body", default if |b| *b == Body::None;
    Collider => "collider", default if |c| *c == Collider::None;
    BodyProps => "physics", default if |p| p.is_default();
    Joint => "joint", default if |j| j.is_none();
    Lens => "camera";
    Spline => "spline";
    Along => "along";
    Light => "light";
    Emitter => "particles";
    Probe => "reflection_probe";
    RenderTexture => "render_texture";
    SoundSource => "sound";
    PostVolume => "post_volume";
    Decal => "decal";
    crate::footprints::Footprints => "footprints";
    crate::terrain::Terrain => "terrain";
    Route => "route";
    View => "view";
    Sun => "sun";
    Fog => "fog";
    crate::render::Sky => "sky";
    crate::post::PostProcess => "post";
    crate::ssao::AmbientOcclusion => "ambient_occlusion";
    crate::ray::RayTracing => "ray_tracing";
    crate::volume::VolumetricFog => "volumetric_fog";
    crate::foliage::Wind => "wind";
    crate::weather::Weather => "weather";
    crate::reflections::ScreenSpaceReflections => "screen_space_reflections";
}

/// A line's module fields by name, for reading: what each was before it
/// was a part. They move to the modules' own traits as the engine is cut
/// into modules (docs/modules.md); the call sites stay as they are.
impl EntityDesc {
    pub fn model(&self) -> crate::AssetLink {
        self.part::<ModelRef>().map(|m| m.0).unwrap_or_default()
    }
    pub fn material_ref(&self) -> MaterialRef {
        self.part_or_default()
    }
    pub fn body(&self) -> Body {
        self.part_or_default()
    }
    pub fn collider(&self) -> Collider {
        self.part_or_default()
    }
    pub fn physics(&self) -> BodyProps {
        self.part_or_default()
    }
    pub fn joint(&self) -> Joint {
        self.part_or_default()
    }
    pub fn joint_break(&self) -> Option<f32> {
        self.part::<JointBreak>().map(|j| j.0)
    }
    pub fn layer(&self) -> String {
        self.part::<LayerName>().map(|l| l.0).unwrap_or_default()
    }
    pub fn bone(&self) -> String {
        self.part::<BoneName>().map(|b| b.0).unwrap_or_default()
    }
    pub fn animator(&self) -> String {
        self.part::<AnimatorRef>().map(|a| a.0).unwrap_or_default()
    }
    pub fn bends_grass(&self) -> f32 {
        self.part::<BendsGrass>().map_or(0.0, |b| b.0)
    }
    pub fn camera(&self) -> Option<Lens> {
        self.part()
    }
    pub fn spline(&self) -> Option<Spline> {
        self.part()
    }
    pub fn along(&self) -> Option<Along> {
        self.part()
    }
    pub fn light(&self) -> Option<Light> {
        self.part()
    }
    pub fn particles(&self) -> Option<Emitter> {
        self.part()
    }
    pub fn reflection_probe(&self) -> Option<Probe> {
        self.part()
    }
    pub fn render_texture(&self) -> Option<RenderTexture> {
        self.part()
    }
    pub fn sound(&self) -> Option<SoundSource> {
        self.part()
    }
    pub fn post_volume(&self) -> Option<PostVolume> {
        self.part()
    }
    pub fn decal(&self) -> Option<Decal> {
        self.part()
    }
    pub fn footprints(&self) -> Option<crate::footprints::Footprints> {
        self.part()
    }
    pub fn terrain(&self) -> Option<crate::terrain::Terrain> {
        self.part()
    }
    pub fn route(&self) -> Option<Route> {
        self.part()
    }

    pub fn set_model(&mut self, model: impl Into<crate::AssetLink>) {
        self.set_part(&ModelRef(model.into()))
    }
    pub fn set_material(&mut self, material: MaterialRef) {
        self.set_part(&material)
    }
    pub fn set_layer(&mut self, layer: impl Into<String>) {
        self.set_part(&LayerName(layer.into()))
    }
    pub fn set_bone(&mut self, bone: impl Into<String>) {
        self.set_part(&BoneName(bone.into()))
    }
    pub fn set_animator(&mut self, animator: impl Into<String>) {
        self.set_part(&AnimatorRef(animator.into()))
    }
    pub fn set_bends_grass(&mut self, metres: f32) {
        self.set_part(&BendsGrass(metres))
    }
    pub fn set_joint_break(&mut self, newtons: Option<f32>) {
        self.set_part_opt(newtons.map(JointBreak).as_ref())
    }
}

impl Override {
    pub fn model(&self) -> Option<crate::AssetLink> {
        self.part::<ModelRef>().map(|m| m.0)
    }
    pub fn material(&self) -> Option<MaterialRef> {
        self.part()
    }
    pub fn body(&self) -> Option<Body> {
        self.part()
    }
    pub fn collider(&self) -> Option<Collider> {
        self.part()
    }
    pub fn physics(&self) -> Option<BodyProps> {
        self.part()
    }
    pub fn layer(&self) -> Option<String> {
        self.part::<LayerName>().map(|l| l.0)
    }
    pub fn camera(&self) -> Option<Lens> {
        self.part()
    }
    pub fn light(&self) -> Option<Light> {
        self.part()
    }
    pub fn particles(&self) -> Option<Emitter> {
        self.part()
    }
    pub fn reflection_probe(&self) -> Option<Probe> {
        self.part()
    }
    pub fn decal(&self) -> Option<Decal> {
        self.part()
    }
    pub fn footprints(&self) -> Option<crate::footprints::Footprints> {
        self.part()
    }
    pub fn terrain(&self) -> Option<crate::terrain::Terrain> {
        self.part()
    }
    pub fn bends_grass(&self) -> Option<f32> {
        self.part::<BendsGrass>().map(|b| b.0)
    }
    pub fn route(&self) -> Option<Route> {
        self.part()
    }
}

/// A scene's look by name, for reading; see [`EntityDesc::model`].
impl Scene {
    pub fn view(&self) -> View {
        self.part_or_default()
    }
    pub fn sun(&self) -> Sun {
        self.part_or_default()
    }
    pub fn fog(&self) -> Fog {
        self.part_or_default()
    }
    pub fn sky(&self) -> Option<crate::render::Sky> {
        self.part()
    }
    pub fn post(&self) -> Option<crate::post::PostProcess> {
        self.part()
    }
    pub fn ambient_occlusion(&self) -> Option<crate::ssao::AmbientOcclusion> {
        self.part()
    }
    pub fn ray_tracing(&self) -> Option<crate::ray::RayTracing> {
        self.part()
    }
    pub fn volumetric_fog(&self) -> Option<crate::volume::VolumetricFog> {
        self.part()
    }
    pub fn wind(&self) -> Option<crate::foliage::Wind> {
        self.part()
    }
    pub fn weather(&self) -> Option<crate::weather::Weather> {
        self.part()
    }
    pub fn screen_space_reflections(&self) -> Option<crate::reflections::ScreenSpaceReflections> {
        self.part()
    }
}
