//! Putting things down on other things with the mouse.
//!
//! Unity's surface snapping — Ctrl Shift while moving — for a greybox: the
//! selection follows the cursor over whatever is under it and sits on it,
//! a crate onto a table, a lamp onto a wall's top, a rock onto a slope. It
//! rests on the surface by the bottom of its box rather than by its pivot,
//! as Unity does: a greybox's pivots are wherever the primitive put them,
//! and what a person means is "on".

use runity::glam::{Mat4, Vec3};
use runity::EntityId;

use crate::{EditResult, Session};

impl Session {
    /// Put the selection on whatever the pixel shows, as one undo step.
    /// `false` when the cursor is over nothing to stand on.
    pub fn place_on_surface(&mut self, x: u32, y: u32) -> EditResult<bool> {
        self.refuse_while_playing()?;
        let Some(moves) = self.surface_moves(x, y) else {
            return Ok(false);
        };
        let scene = self.history.edit();
        for (id, position) in moves {
            if let Some(desc) = scene.get_mut(id) {
                desc.transform.position = position;
            }
        }
        self.surface = None;
        self.respawn();
        Ok(true)
    }

    /// The same inside a handle's drag, which is already one undo step.
    pub(crate) fn surface_drag(&mut self, x: u32, y: u32) -> EditResult<bool> {
        self.refuse_while_playing()?;
        if self.drag.is_none() {
            return Ok(false);
        }
        let Some(moves) = self.surface_moves(x, y) else {
            return Ok(false);
        };
        let scene = self.history.scene_mut_untracked();
        for (id, position) in moves {
            if let Some(desc) = scene.get_mut(id) {
                desc.transform.position = position;
            }
        }
        self.respawn();
        Ok(true)
    }

    /// Where each selected root goes, in its parent's space, for the gizmo's
    /// entity to sit on what the pixel shows; the rest keep their places
    /// relative to it.
    fn surface_moves(&mut self, x: u32, y: u32) -> Option<Vec<(EntityId, Vec3)>> {
        let roots = self.selection_roots();
        let anchor = self.selected?;
        let anchor = *roots
            .iter()
            .find(|r| self.is_within(anchor, **r))
            .unwrap_or(&anchor);
        if self.surface.is_none() {
            self.surface = Some(self.solid_without(&roots));
        }
        let (from, direction) = self.ray(x, y);
        let far = self.camera.far;
        let (point, _, _) = self
            .surface
            .as_ref()?
            .cast_ray_with_normal(from, direction, far, false)?;
        let (low, high) = self.world_bounds(anchor)?;
        let bottom = Vec3::new((low.x + high.x) * 0.5, low.y, (low.z + high.z) * 0.5);
        let shift = point - bottom;
        let mut out = Vec::new();
        for id in roots {
            let parent = self.parent_world(id).unwrap_or(Mat4::IDENTITY);
            let desc = self.history.scene().get(id)?;
            let local = parent.inverse().transform_vector3(shift);
            out.push((id, desc.transform.position + local));
        }
        Some(out)
    }
}
