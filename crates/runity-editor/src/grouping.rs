//! Making order in the Hierarchy: empty entities, groups, select all.
//!
//! Unity's GameObject menu: Create Empty (Ctrl Shift N) where the view
//! looks, Create Empty Parent (Ctrl Shift G) around the selection, and
//! Ctrl A. A greybox grows as a heap of cubes; these are how it becomes
//! "the hut", "the bridge", "the north wall" — things that move and hide
//! and turn into prefabs as one.

use runity::glam::{Mat4, Vec3};
use runity::{EntityDesc, EntityId, Transform};

use crate::{EditError, EditResult, Session};

impl Session {
    /// An empty entity where the view looks — Create Empty — selected.
    /// One undo step; its id.
    pub fn create_empty(&mut self, name: &str) -> EditResult<EntityId> {
        let id = self.add_entity(
            None,
            EntityDesc {
                name: if name.is_empty() {
                    "entity".into()
                } else {
                    name.to_string()
                },
                transform: Transform {
                    position: self.camera.target,
                    ..Default::default()
                },
                ..Default::default()
            },
        )?;
        self.select(Some(id))?;
        Ok(id)
    }

    /// Put the selection under a new empty entity in the middle of it, at
    /// the ground of its box — Create Empty Parent. Nothing moves in the
    /// world. The group sits where the first selected one was in the tree.
    /// One undo step; the group's id, selected.
    pub fn group_selection(&mut self, name: &str) -> EditResult<EntityId> {
        self.refuse_while_playing()?;
        let roots = self.selection_roots();
        let first = *roots
            .first()
            .ok_or(EditError::Scene("nothing selected to group".into()))?;
        if self.history.scene().get(first).is_none() {
            return Err(EditError::Scene(format!(
                "{first} is a part of a prefab; group the instance, or edit the prefab"
            )));
        }
        let boxes: Vec<(Vec3, Vec3)> = roots.iter().filter_map(|r| self.world_bounds(*r)).collect();
        let at = if boxes.is_empty() {
            self.world_position(first).unwrap_or_default()
        } else {
            let low = boxes.iter().fold(Vec3::splat(f32::MAX), |a, b| a.min(b.0));
            let high = boxes.iter().fold(Vec3::splat(f32::MIN), |a, b| a.max(b.1));
            Vec3::new((low.x + high.x) * 0.5, low.y, (low.z + high.z) * 0.5)
        };
        let parent = self.parent_of(first);
        let parent_world = parent
            .and_then(|p| self.placed(p).map(|(_, m)| m))
            .unwrap_or(Mat4::IDENTITY);
        let index = self.sibling_index(first);
        let before = self.history.depth();
        let group = self.add_entity(
            parent,
            EntityDesc {
                name: if name.is_empty() {
                    "group".into()
                } else {
                    name.to_string()
                },
                transform: Transform {
                    position: parent_world.inverse().transform_point3(at),
                    ..Default::default()
                },
                ..Default::default()
            },
        )?;
        self.move_in_hierarchy(group, parent, index)?;
        for id in &roots {
            self.move_in_hierarchy(*id, Some(group), None)?;
        }
        self.history.squash(self.history.depth() - before);
        self.select(Some(group))?;
        Ok(group)
    }

    /// A box collider that fits the entity's model — what Unity does when
    /// a BoxCollider is added: its size and centre from the model's bounds,
    /// in the entity's own space. One undo step; `false` for a thing with
    /// no model to fit.
    pub fn fit_collider(&mut self, id: EntityId) -> EditResult<bool> {
        let Some(model) = self.line(id).map(|l| l.model.clone()) else {
            return Err(EditError::NoEntity(id));
        };
        let Some((low, high)) = self.bounds_of(&model) else {
            return Ok(false);
        };
        let collider = runity::scene::Collider::Box {
            half: (high - low) * 0.5,
            center: (high + low) * 0.5,
        };
        self.update(id, |desc| desc.collider = collider)?;
        Ok(true)
    }

    /// Select every line of the document at the top — Ctrl A.
    pub fn select_everything(&mut self) -> EditResult<usize> {
        let tops: Vec<EntityId> = self.history.scene().entities.iter().map(|e| e.id).collect();
        self.select(None)?;
        for id in &tops {
            self.add_to_selection(*id)?;
        }
        Ok(tops.len())
    }

    /// The line an entity is under in the document, if any.
    fn parent_of(&self, id: EntityId) -> Option<EntityId> {
        self.history
            .scene()
            .flatten()
            .into_iter()
            .find(|(d, _)| d.children.iter().any(|c| c.id == id))
            .map(|(d, _)| d.id)
    }

    /// Where an entity is among its siblings.
    fn sibling_index(&self, id: EntityId) -> Option<usize> {
        let scene = self.history.scene();
        let siblings = match self.parent_of(id) {
            Some(parent) => &scene.get(parent)?.children,
            None => &scene.entities,
        };
        siblings.iter().position(|e| e.id == id)
    }
}
