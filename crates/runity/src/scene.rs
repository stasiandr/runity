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
    /// Half the size on each axis, before the transform's scale.
    Box {
        half: Vec3,
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
    },
    /// Turns any way about the anchor: a chain, a ball-and-socket.
    Ball {
        #[serde(default, skip_serializing_if = "EntityId::is_unassigned")]
        to: EntityId,
        #[serde(default)]
        anchor: Vec3,
    },
    /// Slides along one axis, within limits in metres if given.
    Slider {
        #[serde(default, skip_serializing_if = "EntityId::is_unassigned")]
        to: EntityId,
        #[serde(default = "up")]
        axis: Vec3,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
        limits: Option<(f32, f32)>,
    },
}

fn up() -> Vec3 {
    Vec3::Y
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
}

impl Default for BodyProps {
    fn default() -> Self {
        Self {
            friction: 0.5,
            bounce: 0.0,
            density: 1.0,
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
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EntityDesc {
    /// Who this is, for as long as it exists: what an edit, a selection, an
    /// undo, a merge and a network message all point at. First in the block
    /// so that every entity in a diff starts with its identity.
    ///
    /// Optional in the file: an entity written without one — by hand, or by
    /// an agent — gets one when the scene loads, and keeps it from the first
    /// save on. See [`crate::id`].
    #[serde(default, skip_serializing_if = "EntityId::is_unassigned")]
    pub id: EntityId,
    /// Shown in the editor's tree; not required to be unique.
    pub name: String,
    /// A model in `assets/` by file stem — `pine_large` for
    /// `assets/models/pine_large.obj` — or a builtin, `builtin:cone`. May be left
    /// out of the file: a group, or a prefab instance, draws nothing itself.
    #[serde(default)]
    pub model: String,
    /// The prefab this entity is an instance of, by file stem, or empty.
    ///
    /// An instance is one line: what it is, where it stands, and what it is
    /// called. Its model and its children come from the prefab, so `model`
    /// is ignored while this is set. Expanding it is
    /// [`prefab::instantiate`](crate::prefab::instantiate), and everything
    /// downstream sees the expansion rather than the reference.
    ///
    /// An empty string rather than an `Option`, for the reason
    /// [`MaterialRef`] is not one either: RON spells an option out as
    /// `Some(...)`, and a wrapper in every line of every scene earns
    /// nothing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prefab: String,
    #[serde(default)]
    pub transform: Transform,
    /// A material by name — a `.rmat` asset in the library, or one of the
    /// engine's builtins (`grass`, `bark`, `ember`, …) — or a material
    /// written out in full. Named first because that is what keeps a scene
    /// readable and a palette consistent: a colour spelled out in twenty
    /// places drifts in nineteen of them.
    /// Left out of the file entirely when it is the default, so a scene
    /// full of plain grey things stays readable.
    #[serde(default, skip_serializing_if = "MaterialRef::is_default")]
    pub material: MaterialRef,
    #[serde(default)]
    pub body: Body,
    /// The shape physics sees. Without one, `body` does nothing: a thing can
    /// be declared solid and still have no shape to be solid with, and
    /// saying so in one place beats guessing a box from the mesh.
    #[serde(default)]
    pub collider: Collider,
    /// Friction, bounce and density; see [`BodyProps`].
    #[serde(default, skip_serializing_if = "BodyProps::is_default")]
    pub physics: BodyProps,
    /// What holds this body to another; see [`Joint`].
    #[serde(default, skip_serializing_if = "Joint::is_none")]
    pub joint: Joint,
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
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "trimmed_components"
    )]
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
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
    #[serde(default)]
    pub children: Vec<EntityDesc>,
}

/// What one instance changes about one part of its prefab. Every field is
/// optional in the file and in meaning: absent is "as the prefab has it".
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Override {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub model: Option<String>,
    /// The part's whole transform, relative to its parent in the prefab.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub transform: Option<Transform>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub material: Option<MaterialRef>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub body: Option<Body>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub collider: Option<Collider>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub physics: Option<BodyProps>,
    /// Components set on the part, one by one.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "trimmed_components"
    )]
    pub components: BTreeMap<String, ComponentValue>,
}

impl Override {
    /// Write what this override says onto a part.
    pub fn apply(&self, part: &mut EntityDesc) {
        if let Some(name) = &self.name {
            part.name = name.clone();
        }
        if let Some(model) = &self.model {
            part.model = model.clone();
        }
        if let Some(transform) = self.transform {
            part.transform = transform;
        }
        if let Some(material) = &self.material {
            part.material = material.clone();
        }
        if let Some(body) = self.body {
            part.body = body;
        }
        if let Some(collider) = self.collider {
            part.collider = collider;
        }
        if let Some(physics) = self.physics {
            part.physics = physics;
        }
        for (name, value) in &self.components {
            part.components.insert(name.clone(), value.clone());
        }
    }

    /// What it takes to turn `prefab` — the part as the prefab has it —
    /// into `edited`: only the fields that differ.
    pub fn between(prefab: &EntityDesc, edited: &EntityDesc) -> Self {
        let differs = |a: bool| a.then_some(());
        Override {
            name: differs(prefab.name != edited.name).map(|_| edited.name.clone()),
            model: differs(prefab.model != edited.model).map(|_| edited.model.clone()),
            transform: differs(prefab.transform != edited.transform).map(|_| edited.transform),
            material: differs(prefab.material != edited.material).map(|_| edited.material.clone()),
            body: differs(prefab.body != edited.body).map(|_| edited.body),
            collider: differs(prefab.collider != edited.collider).map(|_| edited.collider),
            physics: differs(prefab.physics != edited.physics).map(|_| edited.physics),
            components: edited
                .components
                .iter()
                .filter(|(name, value)| prefab.components.get(*name) != Some(*value))
                .map(|(name, value)| (name.clone(), value.clone()))
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
            model,
            transform,
            material,
            body,
            collider,
            physics,
            components,
        } = later;
        self.name = name.or(self.name.take());
        self.model = model.or(self.model.take());
        self.transform = transform.or(self.transform);
        self.material = material.or(self.material.take());
        self.body = body.or(self.body);
        self.collider = collider.or(self.collider);
        self.physics = physics.or(self.physics);
        self.components.extend(components);
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

/// Component values without the whitespace around them, so that where a
/// value sat in the file does not become part of it.
fn trimmed_components<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, ComponentValue>, D::Error> {
    let raw = BTreeMap::<String, ComponentValue>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|(name, value)| (name, value.trim_boxed()))
        .collect())
}

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaterialRef {
    /// A material asset in the library by file stem, or one of the engine's
    /// builtins. `builtin:stone` forces the builtin even when a project has
    /// an asset of that name.
    Named(String),
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
    pub fn material_from(&self, lookup: impl Fn(&str) -> Option<Material>) -> Material {
        match &self.material {
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
}

impl Default for Sun {
    fn default() -> Self {
        Self {
            hour: 9.0,
            intensity: 1.15,
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
        let day = ((self.hour - 6.0) / 12.0).clamp(0.0, 1.0);
        let angle = day * std::f32::consts::PI;
        // The height is floored well above zero: a sun exactly on the
        // horizon lights nothing but the horizon, and every shadow in the
        // scene becomes a stripe reaching to the far plane.
        Vec3::new(-angle.cos(), -angle.sin().max(0.15), -0.35).normalize()
    }

    /// How warm the light is: white overhead, orange near the horizon.
    ///
    /// Not physics — the sky is not scattering anything here — but the one
    /// cue that reads as a time of day at a glance, and cheaper than every
    /// scene hand-picking a colour to go with its hour.
    pub fn color(&self) -> Vec3 {
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
}

impl Default for Fog {
    fn default() -> Self {
        Self {
            color: [0.62, 0.68, 0.74],
            start: 30.0,
            end: 180.0,
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

/// A whole scene, as it sits on disk.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Scene {
    #[serde(default)]
    pub view: View,
    #[serde(default)]
    pub sun: Sun,
    #[serde(default)]
    pub fog: Fog,
    #[serde(default)]
    pub entities: Vec<EntityDesc>,
}

impl Scene {
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
                physics: Default::default(),
                joint: Default::default(),
                overrides: Default::default(),
                components: Default::default(),
                id: Default::default(),
                name: "crate".into(),
                model: "builtin:cube".into(),
                prefab: String::new(),
                transform: Transform {
                    position: Vec3::new(1.0, 2.0, 3.0),
                    rotation_deg: Vec3::new(0.0, 45.0, 0.0),
                    scale: Vec3::splat(2.0),
                },
                material: MaterialRef::Inline(Material::new(0.3, 0.2, 0.1)),
                body: Body::Dynamic,
                collider: Collider::Box {
                    half: Vec3::splat(0.5),
                },
                children: vec![EntityDesc {
                    physics: Default::default(),
                    joint: Default::default(),
                    overrides: Default::default(),
                    components: Default::default(),
                    id: Default::default(),
                    name: "lid".into(),
                    model: "builtin:cube".into(),
                    prefab: String::new(),
                    material: MaterialRef::Named("stone".into()),
                    body: Body::None,
                    collider: Collider::None,
                    transform: Transform::default(),
                    children: Vec::new(),
                }],
            }],
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
            view: View::default(),
            sun: Sun {
                hour: 17.5,
                intensity: 0.8,
            },
            fog: Fog::default(),
            entities: vec![EntityDesc {
                physics: Default::default(),
                joint: Default::default(),
                overrides: Default::default(),
                components: Default::default(),
                id: Default::default(),
                name: "pine".into(),
                model: "models/pine_large.obj".into(),
                prefab: String::new(),
                transform: Transform {
                    position: Vec3::new(1.0, 0.0, -3.0),
                    rotation_deg: Vec3::new(0.0, 45.0, 0.0),
                    scale: Vec3::splat(1.2),
                },
                material: MaterialRef::Named("needle".into()),
                body: Body::Static,
                collider: Collider::None,
                children: Vec::new(),
            }],
        };
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
        let scene = Scene {
            view: View {
                position: Vec3::new(3.0, 9.0, -2.0),
                target: Vec3::new(0.0, 1.0, 0.0),
                fov_deg: 35.0,
            },
            ..Scene::default()
        };
        let dir = std::env::temp_dir().join("runity-scene-view");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("view.ron");
        scene.save(&path).unwrap();
        assert_eq!(Scene::load(&path).unwrap().view, scene.view);

        // And a scene written before there was a camera in the format still
        // opens, framed the way everything used to be.
        let older: Scene = ron::from_str(r#"(entities: [])"#).unwrap();
        assert_eq!(older.view, View::default());
    }

    #[test]
    fn a_missing_field_falls_back_rather_than_failing_to_load() {
        // An agent writing a scene by hand should not have to spell out
        // every default, and an older file should still open.
        let text = r#"(entities: [(name: "rock", model: "models/boulder.obj")])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        assert_eq!(scene.entities[0].transform, Transform::default());
        assert_eq!(scene.entities[0].body, Body::None);
        assert_eq!(scene.entities[0].material(), Material::default());
        assert_eq!(scene.sun.hour, 9.0);
    }
}
