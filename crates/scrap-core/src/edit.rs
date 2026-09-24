//! Changing a scene, and being able to take it back.
//!
//! Undo is snapshots, not inverted commands. A scene is a few hundred
//! entities of plain data, so a copy costs microseconds and a handful of
//! kilobytes — and the alternative is writing an inverse for every operation
//! and getting one of them subtly wrong. The bug that costs is not "undo is
//! slow", it is "undo restored almost the right thing", and snapshots cannot
//! have it.
//!
//! What they do cost is discipline about *when* to take one. A gizmo drag
//! produces sixty mutations a second and must be one undo step, so a
//! snapshot is taken when a drag begins and not while it runs.

use std::collections::HashSet;

use crate::id::EntityId;
use crate::scene::{EntityDesc, Scene};

/// A scene and the states it has been in.
pub struct History {
    scene: Scene,
    past: Vec<Scene>,
    future: Vec<Scene>,
    limit: usize,
    /// Counts every change of any kind: what a window compares to know the
    /// document moved, without comparing documents.
    revision: u64,
    /// A gesture in progress — a slider dragged, a number scrubbed — and
    /// whether its first edit has taken the snapshot yet: every edit after
    /// that one is the same step.
    gesture: Option<bool>,
}

impl History {
    /// `limit` is how many steps back it is possible to go. Beyond it, the
    /// oldest is dropped: an editor left open all day should not grow
    /// without bound, and nobody undoes a hundred steps.
    pub fn new(scene: Scene, limit: usize) -> Self {
        Self {
            scene,
            revision: 0,
            past: Vec::new(),
            future: Vec::new(),
            limit: limit.max(1),
            gesture: None,
        }
    }

    /// Start a gesture: from here until [`Self::end_gesture`], however many
    /// edits are made undo as one — the state before the first of them.
    pub fn begin_gesture(&mut self) {
        self.gesture = Some(false);
    }

    pub fn end_gesture(&mut self) {
        self.gesture = None;
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The scene, without recording anything.
    ///
    /// For a change already inside a transaction — a drag in progress. A
    /// caller reaching for this outside one is about to make an
    /// unrecoverable edit.
    pub fn scene_mut_untracked(&mut self) -> &mut Scene {
        self.revision += 1;
        &mut self.scene
    }

    /// Record the current state, then hand over the scene to change.
    pub fn edit(&mut self) -> &mut Scene {
        self.revision += 1;
        self.snapshot();
        &mut self.scene
    }

    /// Record the current state without changing anything.
    ///
    /// This is what a drag calls when it starts: everything that follows,
    /// until the next snapshot, undoes as one step.
    pub fn snapshot(&mut self) {
        self.revision += 1;
        match self.gesture {
            Some(true) => return,
            Some(false) => self.gesture = Some(true),
            None => {}
        }
        self.past.push(self.scene.clone());
        if self.past.len() > self.limit {
            self.past.remove(0);
        }
        // A new edit discards the redo stack. Keeping it would let a redo
        // reapply a state that the new edit never came from.
        self.future.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    /// How many steps undo can take back.
    /// A number that changes whenever anything about the history does —
    /// an edit, an undo, a squash. Cheap to ask every frame.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn depth(&self) -> usize {
        self.past.len()
    }

    /// Make the last `steps` steps one: what an edit of several things at
    /// once, made of an edit of each, calls afterwards so one undo takes
    /// it all back.
    pub fn squash(&mut self, steps: usize) {
        self.revision += 1;
        if steps > 1 {
            // The state before the first of them stays; the ones between go.
            let first = self.past.len().saturating_sub(steps);
            self.past.truncate(first + 1);
        }
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        self.revision += 1;
        let Some(previous) = self.past.pop() else {
            return false;
        };
        self.future
            .push(std::mem::replace(&mut self.scene, previous));
        true
    }

    pub fn redo(&mut self) -> bool {
        self.revision += 1;
        let Some(next) = self.future.pop() else {
            return false;
        };
        self.past.push(std::mem::replace(&mut self.scene, next));
        true
    }

    /// What undo would take back, in words — "move `crate`" — read off the
    /// two states rather than recorded, so it cannot disagree with them.
    pub fn undo_description(&self) -> Option<String> {
        self.past.last().map(|before| describe(before, &self.scene))
    }

    /// Every step that can be undone, oldest first, in words — Unity's Undo
    /// History window. Read off the states, like the undo label.
    pub fn steps(&self) -> Vec<String> {
        let mut states: Vec<&Scene> = self.past.iter().collect();
        states.push(&self.scene);
        states.windows(2).map(|w| describe(w[0], w[1])).collect()
    }

    /// What redo would put back, in words.
    pub fn redo_description(&self) -> Option<String> {
        self.future.last().map(|after| describe(&self.scene, after))
    }

    /// Change the scene and every state it has been in, the same way,
    /// without recording a step.
    ///
    /// For a change that is not an edit of this scene but a fact about the
    /// world around it — an asset renamed on disk. Undo afterwards must not
    /// bring back the old name: it would restore a reference to a file that
    /// is no longer there.
    pub fn rewrite_all(&mut self, mut change: impl FnMut(&mut Scene)) {
        self.revision += 1;
        change(&mut self.scene);
        self.past.iter_mut().for_each(&mut change);
        self.future.iter_mut().for_each(&mut change);
    }

    /// Replace the scene entirely, as opening a file does. Clears both
    /// stacks: undoing across an open would put a different document's
    /// entities into this one.
    pub fn replace(&mut self, scene: Scene) {
        self.revision += 1;
        self.scene = scene;
        self.past.clear();
        self.future.clear();
    }
}

/// What changed from one state of a scene to another, in the words an
/// editor's Undo menu uses: "move `crate`", "rename `tree` to `pine`",
/// "delete `rock`", "add 12 entities".
pub fn describe(before: &Scene, after: &Scene) -> String {
    use std::collections::HashMap;
    fn index<'a>(
        entities: &'a [EntityDesc],
        parent: Option<EntityId>,
        out: &mut HashMap<EntityId, (&'a EntityDesc, Option<EntityId>)>,
    ) {
        for e in entities {
            out.insert(e.id, (e, parent));
            index(&e.children, Some(e.id), out);
        }
    }
    let (mut old, mut new) = (HashMap::new(), HashMap::new());
    index(&before.entities, None, &mut old);
    index(&after.entities, None, &mut new);
    let name = |e: &EntityDesc| format!("`{}`", e.name);

    // Only the tops of what came and went: deleting a hut is one thing, not
    // a hut and its door.
    let added: Vec<&EntityDesc> = new
        .iter()
        .filter(|(id, (_, parent))| {
            !old.contains_key(*id) && parent.is_none_or(|p| old.contains_key(&p))
        })
        .map(|(_, (e, _))| *e)
        .collect();
    let removed: Vec<&EntityDesc> = old
        .iter()
        .filter(|(id, (_, parent))| {
            !new.contains_key(*id) && parent.is_none_or(|p| new.contains_key(&p))
        })
        .map(|(_, (e, _))| *e)
        .collect();
    let mut changed: Vec<(&EntityDesc, &EntityDesc, bool)> = Vec::new();
    for (id, (after_e, after_parent)) in &new {
        if let Some((before_e, before_parent)) = old.get(id) {
            let mut a = (*before_e).clone();
            let mut b = (*after_e).clone();
            a.children.clear();
            b.children.clear();
            let moved = before_parent != after_parent;
            if a != b || moved {
                changed.push((before_e, after_e, moved));
            }
        }
    }

    let count = |n: usize| {
        if n == 1 {
            "1 entity".to_string()
        } else {
            format!("{n} entities")
        }
    };
    match (added.len(), removed.len(), changed.len()) {
        (0, 0, 0) => {
            let parts: Vec<String> = differing(&before.parts, &after.parts)
                .into_iter()
                .map(|name| format!("the {}", name.replace('_', " ")))
                .collect();
            if parts.is_empty() {
                "nothing".into()
            } else {
                format!("change {}", parts.join(" and "))
            }
        }
        (1, 0, 0) => format!("add {}", name(added[0])),
        (0, 1, 0) => format!("delete {}", name(removed[0])),
        (n, 0, 0) => format!("add {}", count(n)),
        (0, n, 0) => format!("delete {}", count(n)),
        (0, 0, 1) => {
            let (a, b, moved) = changed[0];
            let mut what = Vec::new();
            if a.transform.position != b.transform.position {
                what.push("move");
            }
            if a.transform.rotation_deg != b.transform.rotation_deg {
                what.push("rotate");
            }
            if a.transform.scale != b.transform.scale {
                what.push("scale");
            }
            if moved {
                what.push("reparent");
            }
            // A module's field by its name: "change the light of".
            let fields: Vec<String> = differing(&a.parts, &b.parts)
                .into_iter()
                .map(|name| format!("change the {} of", name.replace('_', " ")))
                .collect();
            what.extend(fields.iter().map(String::as_str));
            if a.components != b.components {
                what.push("change the components of");
            }
            if a.overrides != b.overrides {
                what.push("override a part of");
            }
            if a.name != b.name && what.is_empty() {
                return format!("rename {} to {}", name(a), name(b));
            }
            match what.as_slice() {
                [one] => format!("{one} {}", name(b)),
                [] => format!("change {}", name(b)),
                _ => format!("change {}", name(b)),
            }
        }
        (0, 0, n) => format!("change {}", count(n)),
        (a, r, c) => {
            let mut parts = Vec::new();
            if a > 0 {
                parts.push(format!("add {}", count(a)));
            }
            if r > 0 {
                parts.push(format!("delete {}", count(r)));
            }
            if c > 0 {
                parts.push(format!("change {}", count(c)));
            }
            parts.join(", ")
        }
    }
}

/// Add an entity at the end of the roots, or under a parent. Returns its ID.
///
/// The entity and everything under it get IDs if they have none, and new
/// ones where theirs are already taken in this scene — adding the same
/// description twice adds two things, not one thing twice. `None` when the
/// parent is not in the scene.
pub fn add(scene: &mut Scene, parent: Option<EntityId>, mut desc: EntityDesc) -> Option<EntityId> {
    if let Some(parent) = parent {
        scene.get(parent)?;
    }
    let mut taken: HashSet<EntityId> = scene.ids().into_iter().collect();
    crate::scene::assign_ids(std::slice::from_mut(&mut desc), &mut taken);
    let id = desc.id;
    match parent {
        None => scene.entities.push(desc),
        Some(parent) => scene.get_mut(parent)?.children.push(desc),
    }
    Some(id)
}

/// Remove an entity and everything under it.
pub fn remove(scene: &mut Scene, id: EntityId) -> Option<EntityDesc> {
    fn walk(entities: &mut Vec<EntityDesc>, id: EntityId) -> Option<EntityDesc> {
        if let Some(position) = entities.iter().position(|e| e.id == id) {
            return Some(entities.remove(position));
        }
        entities.iter_mut().find_map(|e| walk(&mut e.children, id))
    }
    walk(&mut scene.entities, id)
}

/// Copy an entity, with its children, as the sibling right after it.
/// Returns the copy's ID.
///
/// The copy and everything under it get new IDs. A copy is a new thing: if
/// it kept the original's IDs, an edit meant for one would land on whichever
/// of the two was found first, and a merge could not tell them apart.
pub fn duplicate(scene: &mut Scene, id: EntityId) -> Option<EntityId> {
    fn walk(
        entities: &mut Vec<EntityDesc>,
        id: EntityId,
        taken: &mut HashSet<EntityId>,
    ) -> Option<EntityId> {
        if let Some(position) = entities.iter().position(|e| e.id == id) {
            let mut copy = entities[position].clone();
            copy.name = numbered(entities, &copy.name);
            forget_ids(&mut copy);
            crate::scene::assign_ids(std::slice::from_mut(&mut copy), taken);
            let copied = copy.id;
            entities.insert(position + 1, copy);
            return Some(copied);
        }
        entities
            .iter_mut()
            .find_map(|e| walk(&mut e.children, id, taken))
    }
    let mut taken: HashSet<EntityId> = scene.ids().into_iter().collect();
    walk(&mut scene.entities, id, &mut taken)
}

/// A copy's name among its siblings, as Unity numbers them: `crate` then
/// `crate (1)`, and a copy of `crate (1)` is the next free number, not
/// `crate (1) (1)`. Twelve posts in a row are twelve names in the tree.
fn numbered(siblings: &[EntityDesc], name: &str) -> String {
    let base_of = |name: &str| -> (String, Option<u32>) {
        if let Some(open) = name.rfind(" (") {
            let number = &name[open + 2..];
            if let Some(n) = number.strip_suffix(')').and_then(|n| n.parse().ok()) {
                return (name[..open].to_string(), Some(n));
            }
        }
        (name.to_string(), None)
    };
    let (base, _) = base_of(name);
    let highest = siblings
        .iter()
        .map(|e| base_of(&e.name))
        .filter(|(b, _)| *b == base)
        .map(|(_, n)| n.unwrap_or(0))
        .max()
        .unwrap_or(0);
    format!("{base} ({})", highest + 1)
}

/// `count` more copies of an entity, each `step` further along from the
/// last, in its parent's space — posts along a fence, pillars down a hall,
/// the greybox way of placing a row. Each copy is a [`duplicate`], with new
/// IDs, beside the original in the tree and in order. The copies' IDs, or
/// empty when `id` is not in the scene.
pub fn array(scene: &mut Scene, id: EntityId, count: usize, step: glam::Vec3) -> Vec<EntityId> {
    let Some(start) = scene.get(id).map(|e| e.transform.position) else {
        return Vec::new();
    };
    let mut copies = Vec::with_capacity(count);
    let mut last = id;
    for i in 1..=count {
        let Some(copy) = duplicate(scene, last) else {
            break;
        };
        if let Some(entity) = scene.get_mut(copy) {
            entity.transform.position = start + step * i as f32;
        }
        copies.push(copy);
        last = copy;
    }
    copies
}

/// Move an entity under a different parent, or to the roots.
///
/// Refuses to make something its own ancestor, which is the one way a tree
/// stops being a tree — and the one an editor's drag-and-drop tries first.
/// Also refuses a parent that is not in the scene, rather than losing the
/// entity on the way.
pub fn reparent(scene: &mut Scene, id: EntityId, new_parent: Option<EntityId>) -> bool {
    reparent_at(scene, id, new_parent, None)
}

/// [`reparent`] to a place among the new siblings — `Some(0)` first, `None`
/// or past the end last: a line dragged between two others in the
/// Hierarchy. The same parent is fine; that is reordering.
pub fn reparent_at(
    scene: &mut Scene,
    id: EntityId,
    new_parent: Option<EntityId>,
    index: Option<usize>,
) -> bool {
    let Some(moving) = scene.get(id) else {
        return false;
    };
    if let Some(parent) = new_parent {
        if parent == id || contains(moving, parent) || scene.get(parent).is_none() {
            return false;
        }
    }
    let Some(desc) = remove(scene, id) else {
        return false;
    };
    let put = |siblings: &mut Vec<EntityDesc>, desc: EntityDesc| {
        let at = index.unwrap_or(siblings.len()).min(siblings.len());
        siblings.insert(at, desc);
    };
    match new_parent {
        None => put(&mut scene.entities, desc),
        // Checked above, before anything was removed; an ID does not move
        // when something else is taken out, which is the point of having
        // one.
        Some(parent) => match scene.get_mut(parent) {
            Some(target) => put(&mut target.children, desc),
            None => put(&mut scene.entities, desc),
        },
    }
    true
}

/// Whether `id` is somewhere under `ancestor`.
fn contains(ancestor: &EntityDesc, id: EntityId) -> bool {
    ancestor
        .children
        .iter()
        .any(|child| child.id == id || contains(child, id))
}

fn forget_ids(desc: &mut EntityDesc) {
    desc.id = EntityId::UNASSIGNED;
    for child in &mut desc.children {
        forget_ids(child);
    }
}

/// How to lay out a scatter: `count` things in a disc of `radius`, no two
/// closer than `spacing`, each turned and sized at random.
#[derive(Debug, Clone, PartialEq)]
pub struct Scatter {
    pub radius: f32,
    pub count: u32,
    /// The least distance between two; `0.0` for none. When the disc is too
    /// full for it, fewer than `count` come out rather than a clump.
    pub spacing: f32,
    /// The same seed lays the same things out the same way, so a scatter
    /// in a scene file is reproducible and its diff is not noise.
    pub seed: u64,
    /// Uniform scale, between these two.
    pub scale: (f32, f32),
    /// Turn each one about the vertical at random.
    pub turn: bool,
}

impl Default for Scatter {
    fn default() -> Self {
        Self {
            radius: 10.0,
            count: 20,
            spacing: 1.0,
            seed: 1,
            scale: (0.8, 1.2),
            turn: true,
        }
    }
}

/// Where each thing of a scatter goes, relative to the scatter's centre.
///
/// Evenly over the disc (not bunched at the middle, which picking a random
/// angle and a random distance would do), rejecting a point that lands
/// within `spacing` of one already placed.
pub fn scatter(layout: &Scatter) -> Vec<crate::Transform> {
    let mut state = layout.seed ^ 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        // splitmix64: small, specified, and the same on every machine.
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    };
    let mut placed: Vec<crate::Transform> = Vec::new();
    let attempts = layout.count.saturating_mul(30);
    for _ in 0..attempts {
        if placed.len() as u32 >= layout.count {
            break;
        }
        let angle = next() * std::f32::consts::TAU;
        let distance = next().sqrt() * layout.radius.max(0.0);
        let position = glam::Vec3::new(angle.cos() * distance, 0.0, angle.sin() * distance);
        let (yaw, size) = (next(), next());
        if placed
            .iter()
            .any(|t| t.position.distance(position) < layout.spacing)
        {
            continue;
        }
        let scale = layout.scale.0 + (layout.scale.1 - layout.scale.0) * size;
        placed.push(crate::Transform {
            position,
            rotation_deg: glam::Vec3::new(0.0, if layout.turn { yaw * 360.0 } else { 0.0 }, 0.0),
            scale: glam::Vec3::splat(scale),
        });
    }
    placed
}


/// One face of a thing's box, in its own axes: `+x` is the face its local
/// X points out of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl Face {
    /// The axis, 0 for X, 1 for Y, 2 for Z.
    pub fn axis(self) -> usize {
        match self {
            Face::PosX | Face::NegX => 0,
            Face::PosY | Face::NegY => 1,
            Face::PosZ | Face::NegZ => 2,
        }
    }

    pub fn positive(self) -> bool {
        matches!(self, Face::PosX | Face::PosY | Face::PosZ)
    }
}

impl std::str::FromStr for Face {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        Ok(match text.trim().to_lowercase().as_str() {
            "+x" | "x" => Face::PosX,
            "-x" => Face::NegX,
            "+y" | "y" | "top" => Face::PosY,
            "-y" | "bottom" => Face::NegY,
            "+z" | "z" => Face::PosZ,
            "-z" => Face::NegZ,
            other => {
                return Err(format!(
                    "no face `{other}`: +x, -x, +y (top), -y (bottom), +z or -z, in the thing's own axes"
                ))
            }
        })
    }
}

/// Push one face of a box out by `metres` — or pull it in, negative — with
/// the opposite face staying exactly where it was: the greybox move, a wall
/// made two metres longer at one end without walking it back into place.
///
/// `min` and `max` are the model's own bounds, before the transform; the
/// transform is the thing's local one, so the result is too, and a rotated
/// wall grows along itself. `None` when the face would pass through the one
/// opposite it, or the model is flat along that axis — there is no size to
/// change there.
pub fn push_face(
    transform: &crate::scene::Transform,
    min: glam::Vec3,
    max: glam::Vec3,
    face: Face,
    metres: f32,
) -> Option<crate::scene::Transform> {
    let axis = face.axis();
    let extent = (max - min)[axis];
    if extent <= f32::EPSILON {
        return None;
    }
    let scale = transform.scale[axis];
    let size = extent * scale.abs();
    let new_size = size + metres;
    if new_size <= 1e-3 {
        return None;
    }
    let mut new_scale = transform.scale;
    new_scale[axis] = new_size / extent * scale.signum();
    // The opposite face, in the model's own coordinates along the axis.
    let opposite = if face.positive() == (scale >= 0.0) {
        min[axis]
    } else {
        max[axis]
    };
    let mut along = glam::Vec3::ZERO;
    along[axis] = opposite * (transform.scale[axis] - new_scale[axis]);
    let mut out = *transform;
    out.position += transform.rotation() * along;
    out.scale = new_scale;
    Some(out)
}

#[cfg(test)]
mod steps_tests {
    use super::*;

    #[test]
    fn the_history_lists_every_step_in_words() {
        let mut scene: Scene =
            ron::from_str(r#"(entities: [(name: "crate", model: "builtin:cube")])"#).unwrap();
        scene.assign_ids();
        let id = scene.entities[0].id;
        let mut history = History::new(scene, 10);
        history.edit().get_mut(id).unwrap().transform.position.x = 2.0;
        history.edit().get_mut(id).unwrap().name = "box".into();
        assert_eq!(history.steps(), ["move `crate`", "rename `crate` to `box`"]);
        history.undo();
        assert_eq!(history.steps(), ["move `crate`"]);
    }
}

#[cfg(test)]
mod array_tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn an_array_places_copies_in_a_row_in_order() {
        let mut scene: Scene = ron::from_str(
            r#"(entities: [
                (name: "post", model: "builtin:cube", transform: (position: (1.0, 0.5, 0.0))),
                (name: "gate", model: "builtin:cube"),
            ])"#,
        )
        .unwrap();
        scene.assign_ids();
        let post = scene.entities[0].id;
        let copies = array(&mut scene, post, 3, Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(copies.len(), 3);
        let xs: Vec<f32> = scene
            .entities
            .iter()
            .map(|e| e.transform.position.x)
            .collect();
        assert_eq!(xs, [1.0, 3.0, 5.0, 7.0, 0.0], "in a row, before the gate");
        assert!(copies.iter().all(|c| *c != post));
        assert!(array(&mut scene, EntityId::fresh(), 2, Vec3::X).is_empty());
    }
}

#[cfg(test)]
mod face_tests {
    use super::*;
    use crate::scene::Transform;
    use glam::Vec3;

    const CUBE: (Vec3, Vec3) = (Vec3::splat(-0.5), Vec3::splat(0.5));

    fn wall() -> Transform {
        Transform {
            position: Vec3::new(0.0, 1.5, 0.0),
            scale: Vec3::new(8.0, 3.0, 0.3),
            ..Transform::default()
        }
    }

    #[test]
    fn pushing_a_face_moves_it_and_leaves_the_opposite_one() {
        let pushed = push_face(&wall(), CUBE.0, CUBE.1, Face::PosX, 2.0).unwrap();
        assert!((pushed.scale.x - 10.0).abs() < 1e-5);
        // Was x in [-4, 4]; now [-4, 6].
        assert!(
            (pushed.position.x - 1.0).abs() < 1e-5,
            "{:?}",
            pushed.position
        );
        let pulled = push_face(&wall(), CUBE.0, CUBE.1, Face::NegX, -3.0).unwrap();
        // [-4, 4] with the -x face pulled in 3: [-1, 4].
        assert!((pulled.scale.x - 5.0).abs() < 1e-5);
        assert!(
            (pulled.position.x - 1.5).abs() < 1e-5,
            "{:?}",
            pulled.position
        );
        assert!(
            push_face(&wall(), CUBE.0, CUBE.1, Face::PosZ, -0.3).is_none(),
            "not through itself"
        );
    }

    #[test]
    fn a_turned_wall_grows_along_itself_and_a_based_model_keeps_its_floor() {
        let mut turned = wall();
        turned.rotation_deg = Vec3::new(0.0, 90.0, 0.0);
        let pushed = push_face(&turned, CUBE.0, CUBE.1, Face::PosX, 2.0).unwrap();
        // Local +x is world -z after a quarter turn about y.
        assert!(pushed.position.x.abs() < 1e-4, "{:?}", pushed.position);
        assert!(
            (pushed.position.z + 1.0).abs() < 1e-4,
            "{:?}",
            pushed.position
        );

        // A model whose origin is its base (0..2 tall): pushing the top up
        // does not move the base.
        let tree = Transform::default();
        let taller = push_face(
            &tree,
            Vec3::new(-0.5, 0.0, -0.5),
            Vec3::new(0.5, 2.0, 0.5),
            Face::PosY,
            1.0,
        )
        .unwrap();
        assert!((taller.scale.y - 1.5).abs() < 1e-5);
        assert!(
            taller.position.y.abs() < 1e-5,
            "the base stays on the ground"
        );
        // Pushing the bottom down moves it.
        let deeper = push_face(
            &tree,
            Vec3::new(-0.5, 0.0, -0.5),
            Vec3::new(0.5, 2.0, 0.5),
            Face::NegY,
            1.0,
        )
        .unwrap();
        assert!(
            (deeper.position.y + 1.0).abs() < 1e-5,
            "{:?}",
            deeper.position
        );
    }

    #[test]
    fn a_face_is_named_in_words() {
        assert_eq!("+x".parse::<Face>().unwrap(), Face::PosX);
        assert_eq!("top".parse::<Face>().unwrap(), Face::PosY);
        assert!("north".parse::<Face>().unwrap_err().contains("+x, -x"));
    }
}

/// The module fields whose text differs between two lines, by name.
fn differing(a: &crate::parts::Parts, b: &crate::parts::Parts) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for name in a.names().chain(b.names()) {
        if a.raw(name) != b.raw(name) && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
}
