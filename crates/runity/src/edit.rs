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
}

impl History {
    /// `limit` is how many steps back it is possible to go. Beyond it, the
    /// oldest is dropped: an editor left open all day should not grow
    /// without bound, and nobody undoes a hundred steps.
    pub fn new(scene: Scene, limit: usize) -> Self {
        Self {
            scene,
            past: Vec::new(),
            future: Vec::new(),
            limit: limit.max(1),
        }
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
        &mut self.scene
    }

    /// Record the current state, then hand over the scene to change.
    pub fn edit(&mut self) -> &mut Scene {
        self.snapshot();
        &mut self.scene
    }

    /// Record the current state without changing anything.
    ///
    /// This is what a drag calls when it starts: everything that follows,
    /// until the next snapshot, undoes as one step.
    pub fn snapshot(&mut self) {
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

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.past.pop() else {
            return false;
        };
        self.future
            .push(std::mem::replace(&mut self.scene, previous));
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.future.pop() else {
            return false;
        };
        self.past.push(std::mem::replace(&mut self.scene, next));
        true
    }

    /// Replace the scene entirely, as opening a file does. Clears both
    /// stacks: undoing across an open would put a different document's
    /// entities into this one.
    pub fn replace(&mut self, scene: Scene) {
        self.scene = scene;
        self.past.clear();
        self.future.clear();
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

/// Move an entity under a different parent, or to the roots.
///
/// Refuses to make something its own ancestor, which is the one way a tree
/// stops being a tree — and the one an editor's drag-and-drop tries first.
/// Also refuses a parent that is not in the scene, rather than losing the
/// entity on the way.
pub fn reparent(scene: &mut Scene, id: EntityId, new_parent: Option<EntityId>) -> bool {
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
    match new_parent {
        None => scene.entities.push(desc),
        // Checked above, before anything was removed; an ID does not move
        // when something else is taken out, which is the point of having
        // one.
        Some(parent) => match scene.get_mut(parent) {
            Some(target) => target.children.push(desc),
            None => scene.entities.push(desc),
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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn entity(name: &str, children: Vec<EntityDesc>) -> EntityDesc {
        EntityDesc {
            name: name.into(),
            model: "builtin:cube".into(),
            children,
            ..Default::default()
        }
    }

    /// root
    ///   child
    ///     grandchild
    /// other
    fn scene() -> Scene {
        let mut scene = Scene {
            entities: vec![
                entity(
                    "root",
                    vec![entity("child", vec![entity("grandchild", vec![])])],
                ),
                entity("other", vec![]),
            ],
            ..Default::default()
        };
        scene.assign_ids();
        scene
    }

    fn names(scene: &Scene) -> Vec<String> {
        scene
            .flatten()
            .into_iter()
            .map(|(e, _)| e.name.clone())
            .collect()
    }

    fn id(scene: &Scene, name: &str) -> EntityId {
        scene.find(name).unwrap_or_else(|| panic!("{name}")).id
    }

    #[test]
    fn undo_puts_back_exactly_what_was_there() {
        let mut history = History::new(scene(), 32);
        assert!(!history.can_undo());

        let before = history.scene().clone();
        let child = id(&before, "child");
        remove(history.edit(), child);
        assert_eq!(names(history.scene()), ["root", "other"]);

        assert!(history.undo());
        assert_eq!(*history.scene(), before, "byte for byte, not nearly");
        assert!(!history.can_undo());
        assert!(history.can_redo());

        assert!(history.redo());
        assert_eq!(names(history.scene()), ["root", "other"]);
    }

    #[test]
    fn a_new_edit_after_an_undo_throws_the_redo_away() {
        // Keeping it would let a redo reapply a state the new edit never
        // came from — the classic way an undo stack corrupts a document.
        let mut history = History::new(scene(), 32);
        let (other, child) = (id(history.scene(), "other"), id(history.scene(), "child"));
        remove(history.edit(), other);
        history.undo();
        assert!(history.can_redo());

        remove(history.edit(), child);
        assert!(!history.can_redo());
    }

    #[test]
    fn a_drag_is_one_step_however_many_times_it_moves() {
        // Sixty mutations a second must not be sixty undo steps.
        let mut history = History::new(scene(), 32);
        let root = id(history.scene(), "root");
        history.snapshot();
        for i in 1..=60 {
            let scene = history.scene_mut_untracked();
            scene.get_mut(root).unwrap().transform.position = Vec3::new(i as f32, 0.0, 0.0);
        }
        assert_eq!(history.scene().entities[0].transform.position.x, 60.0);

        assert!(history.undo());
        assert_eq!(history.scene().entities[0].transform.position.x, 0.0);
        assert!(!history.can_undo(), "one step, not sixty");
    }

    #[test]
    fn the_history_does_not_grow_without_bound() {
        let mut history = History::new(scene(), 3);
        for _ in 0..10 {
            history.snapshot();
        }
        for _ in 0..3 {
            assert!(history.undo());
        }
        assert!(!history.undo(), "three deep, as asked");
    }

    #[test]
    fn opening_a_document_clears_the_stacks() {
        // Undoing across an open would put another document's entities into
        // this one.
        let mut history = History::new(scene(), 32);
        let child = id(history.scene(), "child");
        remove(history.edit(), child);
        history.replace(Scene::default());
        assert!(!history.can_undo() && !history.can_redo());
    }

    #[test]
    fn removing_an_entity_takes_its_children_with_it() {
        let mut scene = scene();
        let child = id(&scene, "child");
        remove(&mut scene, child);
        assert_eq!(names(&scene), ["root", "other"], "child and grandchild too");
    }

    #[test]
    fn an_id_does_not_move_when_something_before_it_goes() {
        // The whole reason for IDs. With positions, removing "root" would
        // make "other" answer to the number "child" used to have, and the
        // next edit meant for one would land on the other.
        let mut scene = scene();
        let other = id(&scene, "other");
        let root = id(&scene, "root");
        remove(&mut scene, root);
        assert_eq!(scene.get(other).map(|e| e.name.as_str()), Some("other"));
    }

    #[test]
    fn a_duplicate_lands_beside_the_original_and_copies_the_subtree() {
        let mut scene = scene();
        let root = id(&scene, "root");
        let copy = duplicate(&mut scene, root).expect("root is duplicable");
        assert_eq!(
            names(&scene),
            [
                "root",
                "child",
                "grandchild",
                "root",
                "child",
                "grandchild",
                "other"
            ]
        );
        assert_ne!(copy, root);
        assert_eq!(scene.entities[1].id, copy, "right after the original");
    }

    #[test]
    fn a_duplicate_is_a_new_thing_all_the_way_down() {
        // A copy that kept its children's IDs would have two grandchildren
        // answering to one name: an edit would hit whichever was found first
        // and a merge could not tell them apart.
        let mut scene = scene();
        let root = id(&scene, "root");
        duplicate(&mut scene, root).unwrap();
        let ids = scene.ids();
        let unique: HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "every ID once: {ids:?}");
    }

    #[test]
    fn reparenting_moves_a_subtree_under_a_new_parent() {
        let mut scene = scene();
        let (other, child) = (id(&scene, "other"), id(&scene, "child"));
        assert!(
            reparent(&mut scene, other, Some(child)),
            "other under child"
        );
        assert_eq!(names(&scene), ["root", "child", "grandchild", "other"]);
        // And it really is nested now, not merely ordered that way.
        assert_eq!(scene.entities.len(), 1);
        assert_eq!(scene.get(other).unwrap().name, "other", "same entity");
    }

    #[test]
    fn nothing_can_become_its_own_ancestor() {
        // The one way a tree stops being a tree, and the first thing an
        // editor's drag-and-drop tries.
        let mut scene = scene();
        let (root, grandchild) = (id(&scene, "root"), id(&scene, "grandchild"));
        assert!(
            !reparent(&mut scene, root, Some(grandchild)),
            "root under its grandchild"
        );
        assert!(!reparent(&mut scene, root, Some(root)), "root under itself");
        assert_eq!(names(&scene), ["root", "child", "grandchild", "other"]);
    }

    #[test]
    fn a_parent_that_is_not_there_is_refused_rather_than_losing_the_entity() {
        let mut scene = scene();
        let other = id(&scene, "other");
        assert!(!reparent(&mut scene, other, Some(EntityId::fresh())));
        assert!(scene.get(other).is_some(), "still in the scene");
    }

    #[test]
    fn adding_under_a_parent_puts_it_at_the_end_of_that_parent() {
        let mut scene = scene();
        let child = id(&scene, "child");
        let added = add(&mut scene, Some(child), entity("new", vec![])).expect("added");
        assert_eq!(
            names(&scene),
            ["root", "child", "grandchild", "new", "other"]
        );
        assert_eq!(scene.get(added).unwrap().name, "new");
        assert!(
            add(&mut scene, Some(EntityId::fresh()), entity("lost", vec![])).is_none(),
            "a parent that is not there"
        );
    }

    #[test]
    fn adding_something_whose_id_is_taken_gives_it_a_new_one() {
        // Adding the same description twice adds two things, not one thing
        // twice.
        let mut scene = scene();
        let other = scene.find("other").unwrap().clone();
        let added = add(&mut scene, None, other.clone()).unwrap();
        assert_ne!(added, other.id);
    }
}
