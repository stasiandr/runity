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

/// Mint IDs for a subtree: unassigned ones, and ones already in `seen`.
///
/// Fresh random IDs, for things being created: an added or duplicated
/// entity is a new thing and gets a new identity.
#[doc(hidden)]
pub fn assign_ids(
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
#[doc(hidden)]
pub fn derive_ids(
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

/// `layer: "props"` — the collision layer, by the name `layers.ron` gives
/// it; absent is `default`. See [`crate::layers`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LayerName(pub String);

crate::impl_parts! {
    LayerName => "layer", default if |l| l.0.is_empty();
}

/// A line's layer — the core's, since what collides and what a camera
/// shows both go by it.
impl EntityDesc {
    pub fn layer(&self) -> String {
        self.part::<LayerName>().map(|l| l.0).unwrap_or_default()
    }

    pub fn set_layer(&mut self, layer: impl Into<String>) {
        self.set_part(&LayerName(layer.into()))
    }
}

impl Override {
    pub fn layer(&self) -> Option<String> {
        self.part::<LayerName>().map(|l| l.0)
    }
}
