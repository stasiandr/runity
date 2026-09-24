//! Previewing a model's animations in the Scene view — Unity's Animation
//! window with its preview on.
//!
//! A skinned model's clips come with it from the library. Previewing one
//! puts an [`Animator`] on the entity *in the world*, not in the document:
//! nothing is saved, nothing is an undo step, and a respawn (an edit, an
//! undo, stopping play) clears it, as Unity's preview lets go when you
//! leave it. What a scene says about playing animations in the game is the
//! game's — this is the editor looking.

#[allow(unused_imports)]
use scrap::prelude::*;
use std::sync::Arc;

use scrap::{Animator, EntityId, SceneId};

use crate::{EditError, EditResult, Session};

impl Session {
    /// The clips of the model an entity draws, by name and length in
    /// seconds; empty for a model with no skin.
    pub fn clips(&self, id: EntityId) -> Vec<(String, f32)> {
        let Some(skin) = self.skin_of(id) else {
            return Vec::new();
        };
        skin.clips
            .iter()
            .map(|c| (c.name.clone(), c.duration))
            .collect()
    }

    /// The joints of the skeleton of the model an entity draws, by name;
    /// empty for a model with no skin. What a layer's mask and a line's
    /// `ik` name.
    pub fn joints(&self, id: EntityId) -> Vec<String> {
        self.skin_of(id)
            .map(|skin| skin.skeleton.joints.into_iter().map(|j| j.name).collect())
            .unwrap_or_default()
    }

    fn skin_of(&self, id: EntityId) -> Option<scrap::asset::MeshSkin> {
        let model = self.entity_model(id)?;
        self.library.as_ref()?.mesh_by_name(&model)?.skin_owned()
    }

    /// Play clip `clip` on the entity in the view, looping, at `speed`; or
    /// stop with `None`. The document does not change.
    pub fn preview_clip(
        &mut self,
        id: EntityId,
        clip: Option<usize>,
        speed: f32,
    ) -> EditResult<()> {
        let entity = self
            .world
            .query::<(hecs::Entity, &SceneId)>()
            .iter()
            .find(|(_, s)| s.0 == id)
            .map(|(e, _)| e)
            .ok_or(EditError::NoEntity(id))?;
        let Some(clip) = clip else {
            let _ = self.world.remove_one::<Animator>(entity);
            let _ = self.world.remove_one::<scrap::world::Posed>(entity);
            self.previewing.retain(|p| *p != id);
            return Ok(());
        };
        let skin = self.skin_of(id).ok_or_else(|| {
            EditError::Scene(format!(
                "{} draws a model with no skeleton: nothing to animate",
                self.entity_name(id).unwrap_or_default()
            ))
        })?;
        if clip >= skin.clips.len() {
            return Err(EditError::Scene(format!(
                "there are {} clips, not {}",
                skin.clips.len(),
                clip + 1
            )));
        }
        let mut animator = Animator::new(Arc::new(skin.skeleton), Arc::new(skin.clips));
        animator.play(clip, 0.0);
        animator.set_speed(speed);
        let _ = self.world.insert_one(entity, animator);
        if !self.previewing.contains(&id) {
            self.previewing.push(id);
        }
        Ok(())
    }

    /// What is being previewed.
    pub fn previewing(&self) -> &[EntityId] {
        &self.previewing
    }
}
