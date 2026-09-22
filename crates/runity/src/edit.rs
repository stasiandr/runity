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

/// Add an entity at the end of the roots, or under a parent. Returns its
/// flattened index.
pub fn add(scene: &mut Scene, parent: Option<usize>, desc: EntityDesc) -> Option<usize> {
    match parent {
        None => {
            scene.entities.push(desc);
        }
        Some(parent) => nth_mut(scene, parent)?.children.push(desc),
    }
    // Recomputed rather than guessed: an index is a position in a depth-first
    // walk, and appending a child moves everything after that subtree.
    let count = scene.flatten().len();
    Some(match parent {
        None => count - 1,
        Some(parent) => index_of_last_child(scene, parent)?,
    })
}

/// Remove an entity and everything under it.
pub fn remove(scene: &mut Scene, index: usize) -> Option<EntityDesc> {
    let (parent, position) = locate(scene, index)?;
    Some(match parent {
        None => scene.entities.remove(position),
        Some(parent) => nth_mut(scene, parent)?.children.remove(position),
    })
}

/// Copy an entity, with its children, as a sibling of the original.
pub fn duplicate(scene: &mut Scene, index: usize) -> Option<usize> {
    let (parent, position) = locate(scene, index)?;
    let copy = match parent {
        None => scene.entities.get(position)?.clone(),
        Some(parent) => nth_mut(scene, parent)?.children.get(position)?.clone(),
    };
    match parent {
        None => scene.entities.insert(position + 1, copy),
        Some(parent) => nth_mut(scene, parent)?.children.insert(position + 1, copy),
    }
    // The copy sits immediately after the original's whole subtree.
    Some(index + subtree_size(scene, index)?)
}

/// Move an entity under a different parent, or to the roots.
///
/// Refuses to make something its own ancestor, which is the one way a tree
/// stops being a tree — and the one an editor's drag-and-drop tries first.
pub fn reparent(scene: &mut Scene, index: usize, new_parent: Option<usize>) -> bool {
    if let Some(parent) = new_parent {
        if parent == index || is_descendant(scene, parent, index) {
            return false;
        }
    }
    let Some(desc) = remove(scene, index) else {
        return false;
    };
    // Removing shifts every index after it, so a parent that came later has
    // moved. Recomputing beats trusting the caller's index across a mutation.
    let parent = new_parent.map(|p| {
        if p > index {
            p - subtree_size_of(&desc)
        } else {
            p
        }
    });
    match parent {
        None => scene.entities.push(desc),
        Some(parent) => match nth_mut(scene, parent) {
            Some(target) => target.children.push(desc),
            None => {
                // The parent vanished with the removal, which means it was
                // inside the subtree — already refused above, but putting
                // the entity back beats losing it.
                scene.entities.push(desc);
                return false;
            }
        },
    }
    true
}

fn subtree_size_of(desc: &EntityDesc) -> usize {
    1 + desc.children.iter().map(subtree_size_of).sum::<usize>()
}

fn subtree_size(scene: &Scene, index: usize) -> Option<usize> {
    let flat = scene.flatten();
    let (desc, _) = flat.get(index)?;
    Some(subtree_size_of(desc))
}

fn index_of_last_child(scene: &Scene, parent: usize) -> Option<usize> {
    let flat = scene.flatten();
    let (desc, _) = flat.get(parent)?;
    let last = desc.children.last()?;
    flat.iter().position(|(e, _)| std::ptr::eq(*e, last))
}

/// Whether `candidate` is somewhere under `ancestor`.
fn is_descendant(scene: &Scene, candidate: usize, ancestor: usize) -> bool {
    let flat = scene.flatten();
    let Some((ancestor_desc, _)) = flat.get(ancestor) else {
        return false;
    };
    let Some((candidate_desc, _)) = flat.get(candidate) else {
        return false;
    };
    fn walk(node: &EntityDesc, target: &EntityDesc) -> bool {
        node.children
            .iter()
            .any(|child| std::ptr::eq(child, target) || walk(child, target))
    }
    walk(ancestor_desc, candidate_desc)
}

/// An entity's parent index and its position among that parent's children.
fn locate(scene: &Scene, index: usize) -> Option<(Option<usize>, usize)> {
    let flat = scene.flatten();
    let (target, _) = flat.get(index)?;
    if let Some(position) = scene.entities.iter().position(|e| std::ptr::eq(e, *target)) {
        return Some((None, position));
    }
    for (parent_index, (parent, _)) in flat.iter().enumerate() {
        if let Some(position) = parent
            .children
            .iter()
            .position(|c| std::ptr::eq(c, *target))
        {
            return Some((Some(parent_index), position));
        }
    }
    None
}

/// The `index`th entity in the flattened scene, mutably.
pub fn nth_mut(scene: &mut Scene, index: usize) -> Option<&mut EntityDesc> {
    fn walk<'a>(
        entities: &'a mut [EntityDesc],
        index: usize,
        seen: &mut usize,
    ) -> Option<&'a mut EntityDesc> {
        for entity in entities {
            if *seen == index {
                return Some(entity);
            }
            *seen += 1;
            let found = walk(&mut entity.children, index, seen).map(|e| e as *mut _);
            if let Some(found) = found {
                // SAFETY: `found` came from this call's own borrow of
                // `entity.children`, which outlives the return. The pointer
                // is only used to end the parent's borrow early, which the
                // borrow checker cannot see through a recursive call.
                return Some(unsafe { &mut *found });
            }
        }
        None
    }
    let mut seen = 0;
    walk(&mut scene.entities, index, &mut seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Transform;
    use glam::Vec3;

    fn entity(name: &str, children: Vec<EntityDesc>) -> EntityDesc {
        EntityDesc {
            name: name.into(),
            model: "builtin:cube".into(),
            prefab: String::new(),
            transform: Transform::default(),
            material: Default::default(),
            body: Default::default(),
            collider: Default::default(),
            children,
        }
    }

    /// root
    ///   child
    ///     grandchild
    /// other
    fn scene() -> Scene {
        Scene {
            entities: vec![
                entity(
                    "root",
                    vec![entity("child", vec![entity("grandchild", vec![])])],
                ),
                entity("other", vec![]),
            ],
            ..Default::default()
        }
    }

    fn names(scene: &Scene) -> Vec<String> {
        scene
            .flatten()
            .into_iter()
            .map(|(e, _)| e.name.clone())
            .collect()
    }

    #[test]
    fn undo_puts_back_exactly_what_was_there() {
        let mut history = History::new(scene(), 32);
        assert!(!history.can_undo());

        let before = history.scene().clone();
        remove(history.edit(), 1);
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
        remove(history.edit(), 3);
        history.undo();
        assert!(history.can_redo());

        remove(history.edit(), 1);
        assert!(!history.can_redo());
    }

    #[test]
    fn a_drag_is_one_step_however_many_times_it_moves() {
        // Sixty mutations a second must not be sixty undo steps.
        let mut history = History::new(scene(), 32);
        history.snapshot();
        for i in 1..=60 {
            let scene = history.scene_mut_untracked();
            nth_mut(scene, 0).unwrap().transform.position = Vec3::new(i as f32, 0.0, 0.0);
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
        remove(history.edit(), 1);
        history.replace(Scene::default());
        assert!(!history.can_undo() && !history.can_redo());
    }

    #[test]
    fn removing_an_entity_takes_its_children_with_it() {
        let mut scene = scene();
        remove(&mut scene, 1);
        assert_eq!(names(&scene), ["root", "other"], "child and grandchild too");
    }

    #[test]
    fn a_duplicate_lands_beside_the_original_and_copies_the_subtree() {
        let mut scene = scene();
        let copy = duplicate(&mut scene, 0).expect("root is duplicable");
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
        assert_eq!(copy, 3, "the copy starts after the original's subtree");
    }

    #[test]
    fn reparenting_moves_a_subtree_under_a_new_parent() {
        let mut scene = scene();
        assert!(reparent(&mut scene, 3, Some(1)), "other under child");
        assert_eq!(names(&scene), ["root", "child", "grandchild", "other"]);
        // And it really is nested now, not merely ordered that way.
        assert_eq!(scene.entities.len(), 1);
    }

    #[test]
    fn nothing_can_become_its_own_ancestor() {
        // The one way a tree stops being a tree, and the first thing an
        // editor's drag-and-drop tries.
        let mut scene = scene();
        assert!(
            !reparent(&mut scene, 0, Some(2)),
            "root under its grandchild"
        );
        assert!(!reparent(&mut scene, 0, Some(0)), "root under itself");
        assert_eq!(names(&scene), ["root", "child", "grandchild", "other"]);
    }

    #[test]
    fn adding_under_a_parent_puts_it_where_the_index_says() {
        let mut scene = scene();
        let added = add(&mut scene, Some(1), entity("new", vec![])).expect("added");
        assert_eq!(
            names(&scene),
            ["root", "child", "grandchild", "new", "other"]
        );
        assert_eq!(added, 3);
    }
}
