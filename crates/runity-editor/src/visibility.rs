//! What the Scene view shows, and choosing by dragging a box.
//!
//! Unity's scene visibility: `H` hides the selection from the view and
//! `Shift H` shows it alone, so the inside of a house can be worked on
//! without its roof in the way. A view setting, like the camera — never in
//! the scene file, never in the game, never an undo step — and what the
//! view does not show, a click does not pick and a box does not take.
//!
//! A box is dragged from empty space; everything whose drawn shape falls
//! partly inside it is selected, as in Unity.

use std::collections::HashSet;

use runity::glam::{Vec2, Vec3};
use runity::EntityId;

use crate::{EditResult, Session};

impl Session {
    /// Hide these entities, and what is under them, from the Scene view —
    /// or show them again.
    pub fn set_hidden(&mut self, ids: &[EntityId], hidden: bool) -> EditResult<()> {
        for id in ids {
            self.require(*id)?;
            if hidden {
                self.hidden.insert(*id);
            } else {
                self.hidden.remove(id);
            }
        }
        Ok(())
    }

    /// Hide the selection, or show it when all of it is hidden already: `H`.
    pub fn toggle_hidden(&mut self) -> EditResult<bool> {
        let selection = self.selection();
        let hide = !selection.iter().all(|id| self.hidden.contains(id));
        self.set_hidden(&selection, hide)?;
        Ok(hide)
    }

    /// Let clicks and boxes take these (and what is under them), or not —
    /// Unity's scene pickability: the ground stays drawn and stops being
    /// grabbed by every box drawn over it. A view setting, like hiding.
    pub fn set_pickable(&mut self, ids: &[EntityId], pickable: bool) -> EditResult<()> {
        for id in ids {
            self.require(*id)?;
            if pickable {
                self.unpickable.remove(id);
            } else {
                self.unpickable.insert(*id);
            }
        }
        Ok(())
    }

    /// Whether a click or a box can take this document entity.
    pub fn is_pickable(&self, id: EntityId) -> bool {
        !self.unpickable.iter().any(|u| self.is_within(id, *u))
    }

    /// What is hidden, by the lines it was asked of.
    pub fn hidden(&self) -> Vec<EntityId> {
        let mut out: Vec<EntityId> = self.hidden.iter().copied().collect();
        out.sort();
        out
    }

    /// Show only these and what is under them, until [`Session::isolate`] is
    /// called with nothing.
    pub fn isolate(&mut self, ids: &[EntityId]) -> EditResult<()> {
        for id in ids {
            self.require(*id)?;
        }
        self.isolated = ids.to_vec();
        Ok(())
    }

    /// What is shown alone; empty when nothing is isolated.
    pub fn isolated(&self) -> &[EntityId] {
        &self.isolated
    }

    /// Show everything again: nothing hidden, nothing isolated.
    pub fn show_all(&mut self) {
        self.unpickable.clear();
        self.hidden.clear();
        self.isolated.clear();
    }

    /// The lines of the expanded scene the view leaves out, parts included.
    pub(crate) fn unseen(&self) -> HashSet<EntityId> {
        if self.hidden.is_empty() && self.isolated.is_empty() {
            return HashSet::new();
        }
        let under = |roots: &mut dyn Iterator<Item = &EntityId>| -> HashSet<EntityId> {
            let scene = self.history.scene();
            roots
                .filter_map(|id| scene.get(*id))
                .flat_map(|line| line.flatten())
                .map(|(d, _)| d.id)
                .collect()
        };
        let hidden = under(&mut self.hidden.iter());
        let isolated = under(&mut self.isolated.iter());
        self.instanced
            .scene
            .flatten()
            .into_iter()
            .map(|(desc, _)| desc.id)
            .filter(|id| {
                let owner = self.instanced.owner_of(*id).unwrap_or(*id);
                hidden.contains(&owner) || (!self.isolated.is_empty() && !isolated.contains(&owner))
            })
            .collect()
    }

    /// Select everything whose drawn shape falls partly inside the box
    /// between two pixels; `add` keeps what was selected. Returns what the
    /// box took.
    pub fn select_in_rect(&mut self, a: Vec2, b: Vec2, add: bool) -> EditResult<Vec<EntityId>> {
        let (low, high) = (a.min(b), a.max(b));
        let (w, h) = self.size();
        let size = Vec2::new(w as f32, h as f32);
        let unseen = self.unseen();
        let mut taken: Vec<EntityId> = Vec::new();
        for (desc, world) in self.instanced.scene.flatten() {
            if unseen.contains(&desc.id) {
                continue;
            }
            let Some((min, max)) = self.bounds_of(&desc.model) else {
                continue;
            };
            let Some(owner) = self.instanced.owner_of(desc.id) else {
                continue;
            };
            if !self.is_pickable(owner) {
                continue;
            }
            // The box the shape covers on screen, from its corners in front
            // of the camera.
            let mut on_screen: Option<(Vec2, Vec2)> = None;
            for corner in 0..8u32 {
                let pick = |axis: usize| {
                    if corner & (1 << axis) == 0 {
                        min[axis]
                    } else {
                        max[axis]
                    }
                };
                let point = world.transform_point3(Vec3::new(pick(0), pick(1), pick(2)));
                if let Some(p) = self.camera.screen_point(point, size) {
                    on_screen = Some(match on_screen {
                        None => (p, p),
                        Some((lo, hi)) => (lo.min(p), hi.max(p)),
                    });
                }
            }
            let Some((lo, hi)) = on_screen else {
                continue;
            };
            let overlaps = lo.x <= high.x && hi.x >= low.x && lo.y <= high.y && hi.y >= low.y;
            if overlaps && !taken.contains(&owner) {
                taken.push(owner);
            }
        }
        if !add {
            self.select(None)?;
        }
        for id in &taken {
            self.add_to_selection(*id)?;
        }
        Ok(taken)
    }

    /// The box being dragged in the Scene view, from where it began to the
    /// cursor, for the window to draw.
    pub fn marquee(&self) -> Option<(Vec2, Vec2)> {
        self.marquee
    }
}
