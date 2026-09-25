//! How a line of a scene is solid: the physics module's fields — `body`,
//! `collider`, `physics`, `joint`, `joint_break`, `collision_model`, `layer` — as types, and
//! the reading of them off a line ([`PhysicsLine`]). See docs/modules.md.

use glam::Vec3;
use serde::{Deserialize, Serialize};

#[allow(unused_imports)]
use crate::defaults::*;
use crate::id::EntityId;
use crate::scene::{EntityDesc, Override};

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
    /// A ball around `center` in the entity's own space: Unity's
    /// SphereCollider center.
    Sphere {
        radius: f32,
        #[serde(default, skip_serializing_if = "is_zero_vec3")]
        center: Vec3,
    },
    /// A cylinder with hemispherical caps: what a person is, because a box
    /// catches on corners and a sphere rolls. Around `center`, along `axis`
    /// (0 x, 1 y — standing, the default — 2 z): Unity's CapsuleCollider
    /// center and direction.
    Capsule {
        half_height: f32,
        radius: f32,
        #[serde(default, skip_serializing_if = "is_zero_vec3")]
        center: Vec3,
        #[serde(default = "y_axis", skip_serializing_if = "is_y_axis")]
        axis: u8,
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
        /// Where the joint meets the other body, in its own space (scaled
        /// with it): Unity's connected anchor, when not configured from
        /// where the two stand. Unset, it is wherever `anchor` is when the
        /// joint is made.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
        connected: Option<Vec3>,
    },
    /// Turns any way about the anchor: a chain, a ball-and-socket.
    Ball {
        #[serde(default, skip_serializing_if = "EntityId::is_unassigned")]
        to: EntityId,
        #[serde(default)]
        anchor: Vec3,
        /// Where the joint meets the other body, in its own space (scaled
        /// with it): Unity's connected anchor, when not configured from
        /// where the two stand. Unset, it is wherever `anchor` is when the
        /// joint is made.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
        connected: Option<Vec3>,
        /// How far it may turn about each of its axes (its x along the
        /// joint's frame), degrees low to high: Unity's ConfigurableJoint
        /// angular limits — a rope's segment that bends only so far.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
        limits_deg: Option<[(f32, f32); 3]>,
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
        /// Where the joint meets the other body, in its own space (scaled
        /// with it): Unity's connected anchor, when not configured from
        /// where the two stand. Unset, it is wherever `anchor` is when the
        /// joint is made.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
        connected: Option<Vec3>,
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
    /// How fast a spring's bounce dies (Unity's damper); a fifth of the
    /// strength when not said.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub damping: Option<f32>,
}

impl Motor {
    /// The damping a spring pulls with.
    pub fn damping(&self) -> f32 {
        self.damping.unwrap_or(self.strength.max(0.0) * 0.2).max(0.0)
    }
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
    /// Its mass in kilograms, when said: a shovel of 2 kg is 2 kg whatever
    /// its collider's size. Wins over `density`. Unity's Rigidbody mass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mass: Option<f32>,
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
    /// one step and the next: Unity's continuous collision detection. Every
    /// dynamic body is swept so now (a thin stick falling on mesh ground
    /// went through it); this stays for the lines that say it.
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
            mass: None,
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
    /// One of the colliders of the nearest ancestor with a body of its own:
    /// a shovel's blade and handle are two parts of one shovel, moving and
    /// weighing as one — Unity's colliders under a Rigidbody, which make one
    /// compound body. With no such ancestor it stands still, as a collider
    /// with no Rigidbody anywhere above it does in Unity.
    Part,
    /// A zone that is part of an ancestor's body: it moves with it, is
    /// solid to nothing and, with [`Contacts`](crate::physics::Contacts) on
    /// it, knows what is inside — a shovel's head, a bucket's mouth.
    TriggerPart,
}

impl Body {
    /// A collider of an ancestor's body rather than a body of its own.
    pub fn is_part(self) -> bool {
        matches!(self, Body::Part | Body::TriggerPart)
    }
}



/// `collision_model: "rock_lod2"` — the model a `Model` collider is made
/// of, where it is not the one drawn: Unity's MeshCollider names its own
/// mesh, and a thing can be solid by a mesh it does not show.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CollisionModel(pub crate::AssetLink);

/// `joint_break: 400.0` — the joint breaks when pulled harder than this
/// many newtons: Unity's Break Force (`PhysicsWorld::broken`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JointBreak(pub f32);


crate::impl_parts! {
    Body => "body", default if |b| *b == Body::None;
    Collider => "collider", default if |c| *c == Collider::None;
    BodyProps => "physics", default if |p| p.is_default(), fractions ["bounce", "blown"];
    Joint => "joint", default if |j| j.is_none();
    JointBreak => "joint_break";
    CollisionModel => "collision_model", default if |m| m.0.is_empty();
}

/// How a line of a scene is solid, read off it: what was `desc.body` before
/// a line's fields were its modules' (docs/modules.md).
pub trait PhysicsLine {
    fn body(&self) -> Body;
    fn collider(&self) -> Collider;
    fn physics(&self) -> BodyProps;
    fn joint(&self) -> Joint;
    fn joint_break(&self) -> Option<f32>;
    fn set_joint_break(&mut self, newtons: Option<f32>);
}

impl PhysicsLine for EntityDesc {
    fn body(&self) -> Body {
        self.part_or_default()
    }
    fn collider(&self) -> Collider {
        self.part_or_default()
    }
    fn physics(&self) -> BodyProps {
        self.part_or_default()
    }
    fn joint(&self) -> Joint {
        self.part_or_default()
    }
    fn joint_break(&self) -> Option<f32> {
        self.part::<JointBreak>().map(|j| j.0)
    }
    fn set_joint_break(&mut self, newtons: Option<f32>) {
        self.set_part_opt(newtons.map(JointBreak).as_ref())
    }
}

/// What an override of a prefab's part says about how it is solid.
pub trait PhysicsOverride {
    fn body(&self) -> Option<Body>;
    fn collider(&self) -> Option<Collider>;
    fn physics(&self) -> Option<BodyProps>;
}

impl PhysicsOverride for Override {
    fn body(&self) -> Option<Body> {
        self.part()
    }
    fn collider(&self) -> Option<Collider> {
        self.part()
    }
    fn physics(&self) -> Option<BodyProps> {
        self.part()
    }
}

/// Segments round a ring of an outline.
const OUTLINE_RING: usize = 24;

impl Collider {
    /// This collider as line segments — the shape physics sees, at the
    /// sizes the physics world builds (sphere radius by the largest scale,
    /// capsule and cylinder by the larger of x and z) — in the frame it
    /// returns: `placed` without its scale. What an editor draws over the
    /// model, so a box half a metre off is seen and not walked into. A
    /// `Model` collider has no outline here; it is the model.
    pub fn outline(&self, placed: glam::Mat4) -> (Vec<(Vec3, Vec3)>, glam::Mat4) {
        use glam::Mat4;
        let (scale, rotation, translation) = placed.to_scale_rotation_translation();
        // Placed without scale: sizes below are already scaled, as physics
        // scales them.
        let frame = Mat4::from_rotation_translation(rotation, translation);
        let mut segments: Vec<(Vec3, Vec3)> = Vec::new();
        fn circle(out: &mut Vec<(Vec3, Vec3)>, centre: Vec3, u: Vec3, v: Vec3, radius: f32) {
            let step = std::f32::consts::TAU / OUTLINE_RING as f32;
            for i in 0..OUTLINE_RING {
                let (a, b) = (i as f32 * step, (i + 1) as f32 * step);
                out.push((
                    centre + (u * a.cos() + v * a.sin()) * radius,
                    centre + (u * b.cos() + v * b.sin()) * radius,
                ));
            }
        }
        let box_edges = |h: Vec3| -> Vec<(Vec3, Vec3)> {
            let corner = |x: f32, y: f32, z: f32| Vec3::new(x * h.x, y * h.y, z * h.z);
            let mut edges = Vec::new();
            for (a, b) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
                edges.push((corner(-1.0, a, b), corner(1.0, a, b)));
                edges.push((corner(a, -1.0, b), corner(a, 1.0, b)));
                edges.push((corner(a, b, -1.0), corner(a, b, 1.0)));
            }
            edges
        };
        // A ramp: high at the back (−z), down to nothing at the front.
        let ramp_edges = |h: Vec3| -> Vec<(Vec3, Vec3)> {
            let c = |x: f32, y: f32, z: f32| Vec3::new(x * h.x, y * h.y, z * h.z);
            vec![
                (c(-1.0, -1.0, -1.0), c(1.0, -1.0, -1.0)),
                (c(1.0, -1.0, -1.0), c(1.0, -1.0, 1.0)),
                (c(1.0, -1.0, 1.0), c(-1.0, -1.0, 1.0)),
                (c(-1.0, -1.0, 1.0), c(-1.0, -1.0, -1.0)),
                (c(-1.0, -1.0, -1.0), c(-1.0, 1.0, -1.0)),
                (c(1.0, -1.0, -1.0), c(1.0, 1.0, -1.0)),
                (c(-1.0, 1.0, -1.0), c(1.0, 1.0, -1.0)),
                (c(-1.0, 1.0, -1.0), c(-1.0, -1.0, 1.0)),
                (c(1.0, 1.0, -1.0), c(1.0, -1.0, 1.0)),
            ]
        };
        match self {
            Collider::None | Collider::Model => return (Vec::new(), frame),
            Collider::Box { half, center } => {
                let c = center * scale;
                segments.extend(
                    box_edges(half * scale)
                        .into_iter()
                        .map(|(a, b)| (a + c, b + c)),
                );
            }
            Collider::Stairs { half, .. } => {
                segments.extend(box_edges(half * scale));
            }
            Collider::Ramp { half } => segments.extend(ramp_edges(half * scale)),
            Collider::Sphere { radius, center } => {
                let r = radius * scale.max_element();
                let c = center * scale;
                circle(&mut segments, c, Vec3::X, Vec3::Y, r);
                circle(&mut segments, c, Vec3::Y, Vec3::Z, r);
                circle(&mut segments, c, Vec3::Z, Vec3::X, r);
            }
            Collider::Capsule {
                half_height,
                radius,
                ..
            }
            | Collider::Cylinder {
                half_height,
                radius,
            } => {
                let (h, r) = (half_height * scale.y, radius * scale.x.max(scale.z));
                circle(&mut segments, Vec3::Y * h, Vec3::X, Vec3::Z, r);
                circle(&mut segments, -Vec3::Y * h, Vec3::X, Vec3::Z, r);
                for side in [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z] {
                    segments.push((side * r - Vec3::Y * h, side * r + Vec3::Y * h));
                }
                if matches!(self, Collider::Capsule { .. }) {
                    // The caps, as half-rings over the top and under the bottom.
                    circle(&mut segments, Vec3::Y * h, Vec3::X, Vec3::Y, r);
                    circle(&mut segments, -Vec3::Y * h, Vec3::Z, Vec3::Y, r);
                }
            }
        }
        (segments, frame)
    }
}

/// A capsule stands along y unless it says otherwise.
fn y_axis() -> u8 {
    1
}

fn is_y_axis(axis: &u8) -> bool {
    *axis == 1
}
