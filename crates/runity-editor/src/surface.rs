//! Putting things down on other things with the mouse: on a surface, and
//! corner to corner.
//!
//! Unity's surface snapping — Ctrl Shift while moving — for a greybox: the
//! selection follows the cursor over whatever is under it and sits on it,
//! a crate onto a table, a lamp onto a wall's top, a rock onto a slope. It
//! rests on the surface by the bottom of its box rather than by its pivot,
//! as Unity does: a greybox's pivots are wherever the primitive put them,
//! and what a person means is "on".

use runity::glam::{Mat4, Vec2, Vec3};
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

/// How near, in pixels, a vertex has to be to the cursor to be taken.
const VERTEX_REACH: f32 = 24.0;

/// A vertex snap in progress: the selection's vertex that was grabbed,
/// and where each selected root started.
#[derive(Debug, Clone)]
pub(crate) struct VertexGrab {
    from: Vec3,
    roots: Vec<(EntityId, Vec3, Mat4)>,
}

impl Session {
    /// Vertex snapping, Unity's V: grab the selection by its vertex nearest
    /// the cursor. `false` when none is within reach. Starts one undo step
    /// that lasts until [`Session::vertex_end`].
    pub fn vertex_begin(&mut self, x: u32, y: u32) -> EditResult<bool> {
        self.refuse_while_playing()?;
        let roots = self.selection_roots();
        if roots.is_empty() {
            return Ok(false);
        }
        let Some(from) = self.nearest_vertex(x, y, |owner, session| {
            roots.iter().any(|r| session.is_within(owner, *r))
        }) else {
            return Ok(false);
        };
        let roots = roots
            .iter()
            .filter_map(|r| {
                let position = self.history.scene().get(*r)?.transform.position;
                Some((*r, position, self.parent_matrix(*r)))
            })
            .collect();
        self.history.snapshot();
        self.vertex_grab = Some(VertexGrab { from, roots });
        Ok(true)
    }

    /// Carry the grabbed vertex onto the vertex of anything else nearest the
    /// cursor. `false` when there is none within reach, and it stays put.
    pub fn vertex_drag(&mut self, x: u32, y: u32) -> EditResult<bool> {
        self.refuse_while_playing()?;
        let Some(grab) = self.vertex_grab.clone() else {
            return Ok(false);
        };
        let moving: Vec<EntityId> = grab.roots.iter().map(|(id, _, _)| *id).collect();
        let Some(to) = self.nearest_vertex(x, y, |owner, session| {
            !moving.iter().any(|r| session.is_within(owner, *r))
        }) else {
            return Ok(false);
        };
        let went = to - grab.from;
        let scene = self.history.scene_mut_untracked();
        for (id, start, parent) in &grab.roots {
            if let Some(desc) = scene.get_mut(*id) {
                desc.transform.position = *start + parent.inverse().transform_vector3(went);
            }
        }
        self.respawn();
        Ok(true)
    }

    /// Let go of a vertex snap. Safe without one.
    pub fn vertex_end(&mut self) {
        self.vertex_grab = None;
    }

    /// Whether a vertex snap is in progress.
    pub fn is_vertex_snapping(&self) -> bool {
        self.vertex_grab.is_some()
    }

    /// The world position of the vertex nearest the pixel on screen, among
    /// the drawn things `take` says yes to by their document owner.
    fn nearest_vertex(
        &self,
        x: u32,
        y: u32,
        take: impl Fn(EntityId, &Session) -> bool,
    ) -> Option<Vec3> {
        let (w, h) = self.size();
        let size = Vec2::new(w as f32, h as f32);
        let cursor = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
        let unseen = self.unseen();
        let mut best: Option<(f32, Vec3)> = None;
        for (desc, placed) in self.instanced.scene.flatten() {
            if unseen.contains(&desc.id) {
                continue;
            }
            let Some(owner) = self.instanced.owner_of(desc.id) else {
                continue;
            };
            if !take(owner, self) {
                continue;
            }
            for local in self.vertices_of(&desc.model) {
                let world = placed.transform_point3(local);
                let Some(at) = self.camera.screen_point(world, size) else {
                    continue;
                };
                let d = at.distance(cursor);
                if d <= VERTEX_REACH && best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, world));
                }
            }
        }
        best.map(|(_, p)| p)
    }

    /// A model's vertices, in its own space.
    fn vertices_of(&self, model: &str) -> Vec<Vec3> {
        if let Some(mesh) = runity::builtin::by_name(model) {
            return mesh
                .vertices
                .iter()
                .map(|v| Vec3::from_array(v.position))
                .collect();
        }
        let Some(mesh) = self.library.as_ref().and_then(|l| l.mesh_by_name(model)) else {
            return Vec::new();
        };
        mesh.vertices
            .iter()
            .map(|v| {
                let p = &v.position;
                Vec3::new(p[0].to_native(), p[1].to_native(), p[2].to_native())
            })
            .collect()
    }
}
