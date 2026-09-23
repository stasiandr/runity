//! The editor's session: one document, open for editing.
//!
//! Everything a person does in the editor is a function here — open, select,
//! drag a handle, undo, import, press play. There is no window and no UI in
//! this crate on purpose: panels are drawn on top of it (by the GPUI shell,
//! when it exists), and anything else that wants to edit a scene calls the
//! same functions. A test does, an agent will, and the difference between
//! "the editor can do it" and "an agent can do it" should be zero (DNA,
//! postulate 5).
//!
//! This used to sit behind a C ABI for a Swift editor. The editor moved to
//! Rust (DNA, "Принятые решения"), the boundary went, and the logic stayed —
//! it had tests, and none of it was about the boundary.
//!
//! Where the picture goes is not settled here. The session renders into its
//! own image; how that image reaches a GPUI window without a copy is DNA's
//! open question 1 (an IOSurface shared with the engine, on macOS), and it
//! needs a prototype on a Mac before anything is built on it.

mod error;
pub mod history;

use std::path::{Path, PathBuf};

use runity::gizmo::{self, Drag, GizmoStyle, Handle, Motion, Tool};
use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, FogSettings, Frame, Lighting, MeshHandle};
use runity::scene::MaterialRef;
use runity::{
    builtin, EntityDesc, EntityId, Gpu, Library, Material, OffscreenTarget, Renderer, Scene,
};

pub use error::EditError;
pub use history::{Lock, Revision};

/// What a failed session call hands back.
pub type EditResult<T> = Result<T, EditError>;

/// The grid a drag lands on. Zero on any axis turns that one off.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Snap {
    pub meters: f32,
    pub degrees: f32,
    pub scale: f32,
}

/// Everything one open document needs.
pub struct Session {
    gpu: Gpu,
    renderer: Renderer,
    target: OffscreenTarget,
    world: hecs::World,
    history: runity::edit::History,
    scene_path: Option<PathBuf>,
    /// The project the open scene is in. Where prefabs, materials and the
    /// library are is the project's to say, so the editor finds exactly what
    /// the headless render and every other tool find.
    project: Option<runity::Project>,
    library: Option<Library>,
    /// Where that library sits, so the editor can import into it.
    library_dir: Option<PathBuf>,
    /// Where `.rmat` sources go: the project's `materials/`.
    material_dir: Option<PathBuf>,
    /// Prefabs a scene's instances name: the project's `prefabs/`.
    prefabs: runity::Prefabs,
    /// Where those came from, so the editor can write a new one back.
    prefab_dir: Option<PathBuf>,
    /// The document with its instances expanded: what is drawn, and what a
    /// click is tested against. Rebuilt with the world, so the two cannot
    /// disagree about what is in the scene.
    instanced: runity::Instanced,
    uploaded: Vec<(String, MeshHandle)>,
    camera: Camera,
    pixels: Vec<u8>,
    /// Which entity the gizmo is on.
    ///
    /// By ID, so it survives edits to everything else. When it was an index
    /// it had to be dropped after every delete, reparent and undo, because a
    /// position in a list that had changed shape pointed at whatever slid
    /// into the gap.
    selected: Option<EntityId>,
    gizmo_style: GizmoStyle,
    /// Which handles are shown and what a drag does with them.
    tool: Tool,
    snap: Snap,
    drag: Option<Drag>,
    /// The selected entity's transform when the drag began.
    ///
    /// Rotation and scale are asked for relative to where the gesture
    /// started, never to the last frame: a caller that multiplied a delta in
    /// every frame would accumulate its rounding, and a long drag would
    /// drift away from what the cursor says.
    drag_from: Option<runity::Transform>,
    /// A unit cube, uploaded once, that the gizmo's handles are made of.
    gizmo_arm: Option<MeshHandle>,
    /// Set while the scene is being simulated rather than edited.
    play: Option<Play>,
    /// The scene as its file last had it — read or written by this session
    /// — and when the scene's files last changed. What tells an edit made
    /// on disk from one made here.
    on_disk: Option<(Scene, runity::live::Stamps)>,
    /// Theirs, and the conflicts, while a merge of the open scene is being
    /// settled.
    merge: Option<(Scene, Vec<runity::merge::Conflict>)>,
    /// Selected besides `selected`, which stays the one the gizmo is on.
    also_selected: Vec<EntityId>,
}

/// What [`Session::reload_scene`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneReload {
    /// The files are as the session last saw them.
    Unchanged,
    /// The file changed and the session now shows it. One undo step goes
    /// back to what was shown before.
    Reloaded,
    /// The file changed **and** the session has edits it does not: neither
    /// is thrown away. Saving keeps the session's; opening the scene again
    /// takes the file's.
    Conflict,
}

/// What play mode holds while it runs.
///
/// The engine is a guest, so this does not own a loop or a thread: the
/// caller steps it with however long its frame took, and the clock inside
/// decides how many fixed steps that is worth. An editor that started a
/// thread here would be an editor whose simulation kept running while a
/// modal dialog was open.
struct Play {
    physics: runity::PhysicsWorld,
    clock: runity::Time,
    /// The document as it was when play began, restored when it stops.
    ///
    /// Play mode is a preview. Unity lets you edit while it runs and throws
    /// the changes away when you stop, which is famous for losing an hour's
    /// work; here an edit is refused with a sentence instead.
    before: Scene,
}

impl Session {
    /// Open a session that renders into its own image.
    pub fn offscreen(width: u32, height: u32) -> EditResult<Self> {
        let gpu = Gpu::headless_blocking(false).map_err(|e| EditError::Gpu(e.to_string()))?;
        let target = OffscreenTarget::new(&gpu, width.max(1), height.max(1));
        let renderer = Renderer::new(&gpu, &target);
        Ok(Self {
            gpu,
            renderer,
            target,
            world: hecs::World::new(),
            history: runity::edit::History::new(Scene::default(), 64),
            scene_path: None,
            project: None,
            library: None,
            library_dir: None,
            material_dir: None,
            prefabs: runity::Prefabs::new(),
            prefab_dir: None,
            instanced: runity::Instanced::default(),
            uploaded: Vec::new(),
            camera: Camera::default(),
            pixels: Vec::new(),
            selected: None,
            gizmo_style: GizmoStyle::default(),
            tool: Tool::default(),
            snap: Snap::default(),
            drag: None,
            drag_from: None,
            gizmo_arm: None,
            play: None,
            on_disk: None,
            merge: None,
            also_selected: Vec::new(),
        })
    }

    /// The view changed size.
    ///
    /// A new image, because an offscreen target cannot be resized in place
    /// and pretending otherwise would silently keep rendering at the old
    /// size.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.target = OffscreenTarget::new(&self.gpu, width.max(1), height.max(1));
    }

    /// The size of the image the session draws into.
    pub fn size(&self) -> (u32, u32) {
        (self.target.width, self.target.height)
    }

    // --- documents ------------------------------------------------------

    /// Point the session at a library of imported assets.
    ///
    /// Returns what was skipped and why, rather than failing: one bad asset
    /// should cost one missing model, not the library.
    pub fn set_library(&mut self, directory: impl AsRef<Path>) -> EditResult<Vec<String>> {
        let directory = directory.as_ref().to_path_buf();
        let (library, problems) =
            Library::open(&directory).map_err(|e| EditError::Io(e.to_string()))?;
        self.library = Some(library);
        self.library_dir = Some(directory);
        // Handles from the old library refer to meshes uploaded for it.
        self.uploaded.clear();
        self.respawn();
        Ok(problems
            .into_iter()
            .map(|(path, e)| format!("{}: {e}", path.display()))
            .collect())
    }

    /// Open a scene file, and the prefabs beside it.
    ///
    /// Returns the prefabs that were skipped and why.
    pub fn open_scene(&mut self, path: impl AsRef<Path>) -> EditResult<Vec<String>> {
        let path = path.as_ref().to_path_buf();
        let scene = Scene::load(&path).map_err(|e| EditError::Scene(format!("{e:#}")))?;
        // The scene says where it is looked at from, and opening it puts the
        // view there: a file that renders one way headlessly and opens
        // pointing somewhere else in the editor is a file whose picture
        // nobody can predict.
        self.camera = runity::scene_camera(&scene.view);
        // Prefabs, materials and the library are the project's, found from
        // the scene the same way every other tool finds them. An editor that
        // had to be told where they are would be an editor that shows a
        // different scene than the one CI renders.
        let project = runity::Project::find(&path).ok();
        let (prefabs, problems) = project
            .as_ref()
            .map(runity::Prefabs::of)
            .unwrap_or_default();
        self.prefab_dir = project.as_ref().map(|p| p.prefabs());
        self.material_dir = project.as_ref().map(|p| p.materials());
        if let Some(project) = &project {
            // The project's library, loaded if it has been built and made on
            // the first import if not. A library set by hand stays only for
            // scenes that are in no project.
            let directory = project.library();
            self.library = Library::open(&directory).ok().map(|(library, _)| library);
            self.library_dir = Some(directory);
            self.uploaded.clear();
        }
        self.project = project;
        self.prefabs = prefabs;
        let stamps = runity::live::stamps(&path, self.project.as_ref());
        self.on_disk = Some((scene.clone(), stamps));
        self.history.replace(scene);
        self.scene_path = Some(path);
        // Another document's IDs mean nothing here.
        self.selected = None;
        self.also_selected.clear();
        self.drag = None;
        self.respawn();
        Ok(problems
            .into_iter()
            .map(|(path, e)| format!("{}: {e}", path.display()))
            .collect())
    }

    /// Write the scene back: to `path`, or where it was opened from.
    pub fn save_scene(&mut self, path: Option<&Path>) -> EditResult<()> {
        let target = path
            .map(Path::to_path_buf)
            .or_else(|| self.scene_path.clone())
            .ok_or(EditError::NoPath)?;
        self.history
            .scene()
            .save(&target)
            .map_err(|e| EditError::Scene(format!("{e:#}")))?;
        if self.scene_path.as_ref() == Some(&target) {
            let stamps = runity::live::stamps(&target, self.project.as_ref());
            self.on_disk = Some((self.history.scene().clone(), stamps));
        }
        Ok(())
    }

    /// Put everything selected down on what is beneath it — Unity's End
    /// key — as one undoable step. Returns how many found ground.
    ///
    /// "Beneath" is the real shape of everything else: an entity's collider
    /// if it has one, its model if not — so a crate settles on a hill's
    /// slope, not on the box around the hill. What finds nothing below
    /// stays where it is.
    pub fn drop_to_ground(&mut self) -> EditResult<usize> {
        self.refuse_while_playing()?;
        let roots = self.selection_roots();
        if roots.is_empty() {
            return Ok(0);
        }
        // Everything the drop moves, parts of instances included.
        let moving: std::collections::HashSet<EntityId> = self
            .instanced
            .scene
            .flatten()
            .iter()
            .filter(|(d, _)| {
                self.instanced
                    .owner_of(d.id)
                    .is_some_and(|owner| roots.iter().any(|r| self.is_within(owner, *r)))
            })
            .map(|(d, _)| d.id)
            .collect();

        // A world where everything else is solid.
        let mut ground = self.instanced.scene.clone();
        fn solidify(entities: &mut [EntityDesc], moving: &std::collections::HashSet<EntityId>) {
            for e in entities {
                if moving.contains(&e.id) {
                    e.body = runity::Body::None;
                } else {
                    e.body = runity::Body::Static;
                    if e.collider == runity::scene::Collider::None {
                        e.collider = runity::scene::Collider::Model;
                    }
                }
                solidify(&mut e.children, moving);
            }
        }
        solidify(&mut ground.entities, &moving);
        let mut world = hecs::World::new();
        runity::spawn_scene(&ground, &mut world, |_| Some(MeshHandle::TEST));
        runity::physics::attach_scene_collision_meshes(&mut world, &ground, self.library.as_ref());
        let mut physics = runity::PhysicsWorld::new(1.0 / 60.0);
        physics.sync_from_world(&mut world);
        physics.refresh_queries();

        let mut moves: Vec<(EntityId, f32)> = Vec::new();
        for root in &roots {
            let Some((low, high)) = self.world_bounds(*root) else {
                continue;
            };
            // The bottom's corners and middle, from the top down: it rests
            // on the highest thing under any of them.
            let (x0, x1, z0, z1) = (low.x, high.x, low.z, high.z);
            let (xm, zm) = ((x0 + x1) * 0.5, (z0 + z1) * 0.5);
            let inset = |a: f32, b: f32| (a + (b - a) * 0.1, b - (b - a) * 0.1);
            let ((ax, bx), (az, bz)) = (inset(x0, x1), inset(z0, z1));
            let rest = [(xm, zm), (ax, az), (ax, bz), (bx, az), (bx, bz)]
                .into_iter()
                .filter_map(|(x, z)| {
                    physics
                        .cast_ray_with_normal(
                            Vec3::new(x, high.y + 0.01, z),
                            Vec3::NEG_Y,
                            10_000.0,
                            false,
                        )
                        .map(|(point, _, _)| point.y)
                })
                .fold(None, |best: Option<f32>, y| {
                    Some(best.map_or(y, |b| b.max(y)))
                });
            if let Some(y) = rest {
                moves.push((*root, y - low.y));
            }
        }
        if moves.is_empty() {
            return Ok(0);
        }
        let parents: Vec<Option<Mat4>> =
            moves.iter().map(|(id, _)| self.parent_world(*id)).collect();
        let scene = self.history.edit();
        for ((id, lift), parent) in moves.iter().zip(parents) {
            let delta = match parent {
                Some(parent) => parent
                    .inverse()
                    .transform_vector3(Vec3::new(0.0, *lift, 0.0)),
                None => Vec3::new(0.0, *lift, 0.0),
            };
            if let Some(desc) = scene.get_mut(*id) {
                desc.transform.position += delta;
            }
        }
        self.respawn();
        Ok(moves.len())
    }

    /// Whether `id` is `ancestor` or under it in the document.
    fn is_within(&self, id: EntityId, ancestor: EntityId) -> bool {
        self.history
            .scene()
            .get(ancestor)
            .is_some_and(|a| a.flatten().iter().any(|(d, _)| d.id == id))
    }

    /// The world matrix of an entity's parent, if it has one.
    fn parent_world(&self, id: EntityId) -> Option<Mat4> {
        fn find(
            entities: &[EntityDesc],
            id: EntityId,
            parent: Mat4,
            has: bool,
        ) -> Option<Option<Mat4>> {
            for e in entities {
                if e.id == id {
                    return Some(has.then_some(parent));
                }
                if let Some(found) = find(&e.children, id, parent * e.transform.matrix(), true) {
                    return Some(found);
                }
            }
            None
        }
        find(&self.history.scene().entities, id, Mat4::IDENTITY, false).flatten()
    }

    /// The world-space box around a document entity and everything it
    /// brings, from its models' bounds.
    fn world_bounds(&self, id: EntityId) -> Option<(Vec3, Vec3)> {
        let mut low = Vec3::splat(f32::MAX);
        let mut high = Vec3::splat(f32::MIN);
        for (desc, world) in self.instanced.scene.flatten() {
            let owned = self
                .instanced
                .owner_of(desc.id)
                .is_some_and(|owner| self.is_within(owner, id));
            if !owned {
                continue;
            }
            let Some((a, b)) = self.bounds_of(&desc.model) else {
                continue;
            };
            for corner in 0..8u32 {
                let pick = |axis: usize| {
                    if corner & (1 << axis) == 0 {
                        a[axis]
                    } else {
                        b[axis]
                    }
                };
                let p = world.transform_point3(Vec3::new(pick(0), pick(1), pick(2)));
                low = low.min(p);
                high = high.max(p);
            }
        }
        (low.x <= high.x).then_some((low, high))
    }

    // --- the view, moved the way a scene view moves it -----------------

    /// Turn the view around what it looks at: `yaw` about the vertical,
    /// `pitch` up and down, in degrees. Not an edit.
    pub fn orbit(&mut self, yaw: f32, pitch: f32) {
        let offset = self.camera.position - self.camera.target;
        let distance = offset.length().max(1e-3);
        let current_pitch = (offset.y / distance).clamp(-1.0, 1.0).asin();
        let current_yaw = offset.x.atan2(offset.z);
        let pitch = (current_pitch + pitch.to_radians()).clamp(-1.5, 1.5);
        let yaw = current_yaw + yaw.to_radians();
        let offset = Vec3::new(
            pitch.cos() * yaw.sin(),
            pitch.sin(),
            pitch.cos() * yaw.cos(),
        ) * distance;
        self.camera.position = self.camera.target + offset;
    }

    /// Slide the view sideways and up, in metres, keeping its direction.
    pub fn pan(&mut self, right: f32, up: f32) {
        let forward = (self.camera.target - self.camera.position).normalize_or_zero();
        let side = forward.cross(Vec3::Y).normalize_or_zero();
        let lift = side.cross(forward).normalize_or_zero();
        let offset = side * right + lift * up;
        self.camera.position += offset;
        self.camera.target += offset;
    }

    /// Move toward what the view looks at — `factor` below one — or away.
    pub fn zoom(&mut self, factor: f32) {
        let offset = (self.camera.position - self.camera.target) * factor.max(0.01);
        self.camera.position = self.camera.target + offset.clamp_length_min(0.1);
    }

    /// Write an instance's overrides into its prefab — Unity's "Apply" — so
    /// every instance gets them, and clear them from this one. Returns how
    /// many parts changed.
    ///
    /// The prefab file is saved at once, keeping its text where nothing
    /// changed; clearing the instance's overrides is an ordinary undoable
    /// step of the scene. Undoing it puts the overrides back on the
    /// instance, where they now say what the prefab says.
    pub fn apply_overrides(&mut self, instance: EntityId) -> EditResult<usize> {
        self.refuse_while_playing()?;
        let line = self
            .history
            .scene()
            .get(instance)
            .ok_or(EditError::NoEntity(instance))?
            .clone();
        if line.prefab.is_empty() {
            return Err(EditError::Scene(format!(
                "{instance} is not a prefab instance"
            )));
        }
        if line.overrides.is_empty() {
            return Ok(0);
        }
        let directory = self.prefab_dir.clone().ok_or(EditError::NotInProject)?;
        let path = directory.join(format!("{}.prefab", line.prefab));
        let (_, mut prefab) = runity::Prefabs::read(&path).map_err(EditError::Io)?;
        fn find(desc: &mut EntityDesc, id: EntityId) -> Option<&mut EntityDesc> {
            if desc.id == id {
                return Some(desc);
            }
            desc.children.iter_mut().find_map(|c| find(c, id))
        }
        let mut applied = 0;
        for (part, change) in &line.overrides {
            if let Some(target) = prefab.children.iter_mut().find_map(|c| find(c, *part)) {
                change.apply(target);
                applied += 1;
            }
        }
        runity::Prefabs::save(&prefab, &path).map_err(EditError::Io)?;
        self.prefabs.insert(line.prefab.clone(), prefab);
        self.edit_entity(instance)?.overrides.clear();
        self.respawn();
        Ok(applied)
    }

    /// Drop an instance's overrides — Unity's "Revert" — as one undoable
    /// step: it is the prefab again.
    pub fn revert_overrides(&mut self, instance: EntityId) -> EditResult<()> {
        self.edit_entity(instance)?.overrides.clear();
        self.respawn();
        Ok(())
    }

    /// A walkable path between two points of the scene as it stands — can
    /// the player get from the spawn to the exit, and which way — or `None`
    /// when there is none.
    ///
    /// Baked from the scene's static colliders in a world of its own, so
    /// asking changes nothing: not the document, not the world the editor
    /// draws, not a play session.
    pub fn find_path(
        &self,
        from: Vec3,
        to: Vec3,
        settings: runity::navigation::NavSettings,
    ) -> Option<Vec<Vec3>> {
        let scene = &self.instanced.scene;
        let mut world = hecs::World::new();
        runity::spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        runity::physics::attach_scene_collision_meshes(&mut world, scene, self.library.as_ref());
        let mut physics = runity::PhysicsWorld::new(1.0 / 60.0);
        physics.sync_from_world(&mut world);
        physics.refresh_queries();

        // The area: everything solid, and both ends, with room to go round.
        let mut min = from.min(to);
        let mut max = from.max(to);
        for (placed, body) in world
            .query::<(&runity::world::WorldTransform, &runity::world::Physics)>()
            .iter()
        {
            if body.0 != runity::Body::None {
                let (scale, _, at) = placed.0.to_scale_rotation_translation();
                min = min.min(at - scale * 0.5);
                max = max.max(at + scale * 0.5);
            }
        }
        let (min, max) = (min - Vec3::splat(5.0), max + Vec3::splat(5.0));
        // A finer grid than a million cells buys nothing a level check needs.
        let extent = (max.x - min.x).max(max.z - min.z);
        let settings = runity::navigation::NavSettings {
            cell: settings.cell.max(extent / 1000.0),
            ceiling: max.y + 10.0,
            ..settings
        };
        let grid = runity::navigation::NavGrid::bake(
            &physics,
            runity::glam::Vec2::new(min.x, min.z),
            runity::glam::Vec2::new(max.x, max.z),
            settings,
        );
        grid.path(from, to)
    }

    /// Who holds which Git LFS lock in the open project's repository —
    /// what the editor shows next to a binary source someone else is
    /// editing.
    pub fn locks(&self) -> EditResult<Vec<Lock>> {
        let root = self.project.as_ref().ok_or(EditError::NotInProject)?.root();
        history::locks(root).map_err(EditError::Io)
    }

    /// Lock a file, or unlock it, through Git LFS.
    pub fn set_locked(&self, path: impl AsRef<Path>, locked: bool) -> EditResult<()> {
        let path = path.as_ref();
        if locked {
            history::lock(path)
        } else {
            history::unlock(path)
        }
        .map_err(EditError::Io)
    }

    /// The commits that touched the open scene's file, newest first: who,
    /// when, and why.
    pub fn scene_history(&self) -> EditResult<Vec<Revision>> {
        let path = self.scene_path.as_ref().ok_or(EditError::NoPath)?;
        history::log(path).map_err(EditError::Io)
    }

    /// The open scene as it was at `commit`, without touching the document:
    /// to look at, or to render, before deciding.
    pub fn scene_at(&self, commit: &str) -> EditResult<Scene> {
        let path = self.scene_path.as_ref().ok_or(EditError::NoPath)?;
        let text = history::show(path, commit).map_err(EditError::Io)?;
        let mut scene: Scene = runity::ron::from_str(&text)
            .map_err(|e| EditError::Scene(format!("{} at {commit}: {e}", path.display())))?;
        scene.assign_ids();
        Ok(scene)
    }

    /// Put the scene back as it was at `commit`, as one undoable edit —
    /// nothing is saved until the document is.
    pub fn restore_revision(&mut self, commit: &str) -> EditResult<()> {
        self.refuse_while_playing()?;
        let scene = self.scene_at(commit)?;
        *self.history.edit() = scene;
        if self
            .selected
            .is_some_and(|id| self.history.scene().get(id).is_none())
        {
            self.selected = None;
        }
        self.respawn();
        Ok(())
    }

    /// The conflicts in the open scene, when git is in the middle of a merge
    /// that conflicted on it; empty otherwise.
    ///
    /// Git keeps the common ancestor, ours and theirs in the index while a
    /// merge is unsettled, so the editor merges them again itself and gets
    /// the same conflicts `runity merge` reported — with the values, so each
    /// can be shown and settled here instead of in a text editor.
    pub fn merge_conflicts(&mut self) -> EditResult<Vec<runity::merge::Conflict>> {
        let stages = (
            self.scene_at(":1"),
            self.scene_at(":2"),
            self.scene_at(":3"),
        );
        let (Ok(base), Ok(ours), Ok(theirs)) = stages else {
            self.merge = None;
            return Ok(Vec::new());
        };
        let merged = runity::merge::merge_scenes(&base, &ours, &theirs);
        self.merge = Some((theirs, merged.conflicts.clone()));
        Ok(merged.conflicts)
    }

    /// Settle one conflict — by its place in [`Session::merge_conflicts`] —
    /// theirs' way, as one undoable edit. Keeping ours needs nothing: the
    /// merge already kept it.
    pub fn take_theirs(&mut self, conflict: usize) -> EditResult<()> {
        self.refuse_while_playing()?;
        let Some((theirs, conflicts)) = self.merge.clone() else {
            return Err(EditError::Scene(
                "no merge conflicts loaded — call merge_conflicts first".into(),
            ));
        };
        let Some(conflict) = conflicts.get(conflict) else {
            return Err(EditError::Scene(format!(
                "there are {} conflicts, not {}",
                conflicts.len(),
                conflict + 1
            )));
        };
        let mut scene = self.history.scene().clone();
        if !conflict.take_theirs(&mut scene, &theirs) {
            return Err(EditError::Scene(format!(
                "nothing of theirs to take for: {conflict}"
            )));
        }
        *self.history.edit() = scene;
        self.respawn();
        Ok(())
    }

    /// Pick up the scene's file, or a prefab, changed by someone else — a
    /// text editor, `git pull`, an agent.
    ///
    /// Cheap when nothing changed: a few `stat`s, so an editor can call it
    /// several times a second. A changed file becomes an ordinary edit, so
    /// undo takes it back and selection stays on whatever still exists. When
    /// the session has unsaved edits of its own, nothing is overwritten
    /// either way — that is [`SceneReload::Conflict`], and the caller asks.
    /// While playing it waits: the change is picked up after stop.
    pub fn reload_scene(&mut self) -> EditResult<SceneReload> {
        let (Some(path), Some((seen, stamps))) = (&self.scene_path, &self.on_disk) else {
            return Ok(SceneReload::Unchanged);
        };
        if self.play.is_some() {
            return Ok(SceneReload::Unchanged);
        }
        let now = runity::live::stamps(path, self.project.as_ref());
        if &now == stamps {
            return Ok(SceneReload::Unchanged);
        }
        let prefabs_changed = now[1..] != stamps[1..];
        let scene = Scene::load(path).map_err(|e| EditError::Scene(format!("{e:#}")))?;
        let scene_changed = &scene != seen;
        let ours = self.history.scene() != seen;
        if scene_changed && ours {
            return Ok(SceneReload::Conflict);
        }
        if prefabs_changed {
            if let Some(project) = &self.project {
                self.prefabs = runity::Prefabs::of(project).0;
            }
        }
        if scene_changed {
            *self.history.edit() = scene.clone();
            if self
                .selected
                .is_some_and(|id| self.history.scene().get(id).is_none())
            {
                self.selected = None;
            }
        }
        self.on_disk = Some((scene, now));
        if scene_changed || prefabs_changed {
            self.respawn();
            return Ok(SceneReload::Reloaded);
        }
        Ok(SceneReload::Unchanged)
    }

    /// The document as it stands.
    pub fn scene(&self) -> &Scene {
        self.history.scene()
    }

    /// The project the open scene is in, if it is in one.
    pub fn project(&self) -> Option<&runity::Project> {
        self.project.as_ref()
    }

    // --- the scene tree, flattened --------------------------------------

    /// How many entities the open scene has, counting children.
    pub fn entity_count(&self) -> usize {
        self.history.scene().flatten().len()
    }

    /// How many entities are in the world: the scene with its prefab
    /// instances expanded, which is what is drawn and what a click can hit.
    pub fn spawned_count(&self) -> usize {
        self.world.len() as usize
    }

    /// Every entity, in the order the tree shows them: depth first, parents
    /// before their children.
    pub fn entities(&self) -> Vec<EntityId> {
        self.history.scene().ids()
    }

    /// The first entity with this name. For people and tests; anything that
    /// has to find the same entity twice keeps its ID.
    pub fn find(&self, name: &str) -> Option<EntityId> {
        self.history.scene().find(name).map(|e| e.id)
    }

    /// One entity's name.
    pub fn entity_name(&self, id: EntityId) -> Option<String> {
        self.line(id).map(|e| e.name.clone())
    }

    /// An entity's local transform — the one the file holds.
    pub fn transform(&self, id: EntityId) -> Option<runity::Transform> {
        self.line(id).map(|e| e.transform)
    }

    /// Set an entity's local transform, as one undoable step.
    pub fn set_transform(&mut self, id: EntityId, transform: runity::Transform) -> EditResult<()> {
        // Recorded: a value typed into an inspector is one undoable edit. A
        // drag is not, because `gizmo_begin` already took the snapshot that
        // covers the whole gesture.
        self.modify(id, |desc| desc.transform = transform)
    }

    /// Rename an entity, as one undoable step. Names are for people and
    /// need not be unique; an empty one is refused.
    pub fn rename(&mut self, id: EntityId, name: &str) -> EditResult<()> {
        if name.is_empty() {
            return Err(EditError::EmptyName("an entity"));
        }
        self.modify(id, |desc| desc.name = name.to_string())
    }

    /// Change any fields of an entity's line, as one undoable step.
    ///
    /// For callers that set several things at once — an agent writing a
    /// name, a place and a material in one go should make one step, not
    /// three. Its ID and children are the document's, not the caller's, and
    /// are put back if the closure touched them.
    pub fn update(&mut self, id: EntityId, change: impl FnOnce(&mut EntityDesc)) -> EditResult<()> {
        self.modify(id, change)
    }

    /// Change an entity, as one undoable step: a line of the document, or —
    /// for a part a prefab instance brought — an override on that instance,
    /// so the prefab keeps its values and this instance keeps its change.
    /// Its ID and children are not the caller's to change, and are put back.
    fn modify(&mut self, id: EntityId, change: impl FnOnce(&mut EntityDesc)) -> EditResult<()> {
        if self.history.scene().get(id).is_some() {
            let desc = self.edit_entity(id)?;
            let children = std::mem::take(&mut desc.children);
            change(desc);
            desc.id = id;
            desc.children = children;
            // Respawned rather than patched in place: a moved parent moves
            // its children, and keeping two ways to apply that is how they
            // drift.
            self.respawn();
            return Ok(());
        }
        self.refuse_while_playing()?;
        let (instance, part) = *self
            .instanced
            .parts
            .get(&id)
            .ok_or(EditError::NoEntity(id))?;
        if self.history.scene().get(instance).is_none() {
            return Err(EditError::Scene(format!(
                "{id} is part of a prefab inside another prefab; open that prefab to change it"
            )));
        }
        let current = self
            .instanced
            .scene
            .get(id)
            .cloned()
            .ok_or(EditError::NoEntity(id))?;
        let mut edited = current.clone();
        change(&mut edited);
        let delta = runity::scene::Override::between(&current, &edited);
        if delta.is_empty() {
            return Ok(());
        }
        self.edit_entity(instance)?
            .overrides
            .entry(part)
            .or_default()
            .merge(delta);
        self.respawn();
        Ok(())
    }

    /// Set one of the game's components on an entity, as RON text, or take
    /// it off with `None` — one undoable step.
    ///
    /// The text is checked to be RON, not to fit the component: the editor
    /// does not link the game, so whether `(open_angle: "wide")` is a door
    /// is for the game to say when it reads the scene.
    pub fn set_component(&mut self, id: EntityId, name: &str, ron: Option<&str>) -> EditResult<()> {
        if name.is_empty() {
            return Err(EditError::EmptyName("a component"));
        }
        // Parsed before the edit is recorded, so a typo costs no undo step.
        let mut probe = EntityDesc::default();
        if let Some(ron) = ron {
            probe.set_component(name, ron).map_err(EditError::Scene)?;
        }
        let value = probe.components.remove(name);
        self.modify(id, |desc| match value {
            Some(value) => {
                desc.components.insert(name.to_string(), value);
            }
            None => {
                desc.components.remove(name);
            }
        })
    }

    /// Where an entity actually is, in world space.
    ///
    /// Not the same as its transform. The transform is local and belongs to
    /// the file; this is where the thing ends up once its parents — and,
    /// while play is running, the simulation — have had their say. An
    /// inspector that showed only the local one would say a falling crate is
    /// still four metres up.
    pub fn world_position(&self, id: EntityId) -> Option<Vec3> {
        // The entity spawned for that line. For an instance it is the
        // prefab's root, which keeps the instance's ID: the line stands for
        // the whole thing, and the root is where the whole thing is.
        self.world
            .query::<(&runity::SceneId, &runity::world::WorldTransform)>()
            .iter()
            .find(|(scene_id, _)| scene_id.0 == id)
            .map(|(_, placed)| placed.0.w_axis.truncate())
    }

    // --- editing, and taking it back ------------------------------------

    /// Add an entity with a model, under `parent` or at the top. Returns its
    /// ID.
    pub fn add(&mut self, parent: Option<EntityId>, model: &str) -> EditResult<EntityId> {
        let desc = EntityDesc {
            name: "entity".into(),
            model: model.to_string(),
            ..Default::default()
        };
        self.insert(parent, desc)
    }

    /// Add a whole entity — name, model, place, material, children — under
    /// `parent` or at the top, as one undoable step. Returns its ID. IDs it
    /// carries that are taken, and missing ones, are minted.
    pub fn add_entity(
        &mut self,
        parent: Option<EntityId>,
        desc: EntityDesc,
    ) -> EditResult<EntityId> {
        self.insert(parent, desc)
    }

    /// Scatter copies of a model — or instances of a prefab, when `what`
    /// names one — over a disc around `centre`, as one group entity and one
    /// undoable step. Returns the group's ID.
    ///
    /// Laid out by [`runity::edit::scatter`]: the same seed gives the same
    /// layout, so the scene file says exactly what was placed and a
    /// re-scatter with another seed is a readable diff.
    pub fn scatter(
        &mut self,
        parent: Option<EntityId>,
        what: &str,
        centre: Vec3,
        layout: &runity::edit::Scatter,
    ) -> EditResult<EntityId> {
        let prefab = self.prefabs.get(what).is_some();
        if what.is_empty() {
            return Err(EditError::EmptyName("a model or prefab to scatter"));
        }
        let children = runity::edit::scatter(layout)
            .into_iter()
            .map(|transform| EntityDesc {
                name: what.trim_start_matches("builtin:").to_string(),
                model: if prefab {
                    String::new()
                } else {
                    what.to_string()
                },
                prefab: if prefab {
                    what.to_string()
                } else {
                    String::new()
                },
                transform,
                ..Default::default()
            })
            .collect();
        let group = EntityDesc {
            name: format!("{} ×{}", what.trim_start_matches("builtin:"), layout.count),
            transform: runity::Transform {
                position: centre,
                ..Default::default()
            },
            children,
            ..Default::default()
        };
        self.insert(parent, group)
    }

    /// Delete an entity and everything under it.
    pub fn delete(&mut self, id: EntityId) -> EditResult<()> {
        self.refuse_while_playing()?;
        self.require(id)?;
        runity::edit::remove(self.history.edit(), id);
        self.after_structural_change();
        Ok(())
    }

    /// Copy an entity beside itself. Returns the copy's ID.
    pub fn duplicate(&mut self, id: EntityId) -> EditResult<EntityId> {
        self.refuse_while_playing()?;
        self.require(id)?;
        let copy =
            runity::edit::duplicate(self.history.edit(), id).ok_or(EditError::NoEntity(id))?;
        self.respawn();
        Ok(copy)
    }

    /// Move an entity under another, or to the top. Refuses to make
    /// something its own ancestor, and says `false` when it did.
    pub fn reparent(&mut self, id: EntityId, new_parent: Option<EntityId>) -> EditResult<bool> {
        self.refuse_while_playing()?;
        self.require(id)?;
        if let Some(parent) = new_parent {
            self.require(parent)?;
        }
        // Checked on a copy first, so a refused move costs no undo step.
        let mut trial = self.history.scene().clone();
        if !runity::edit::reparent(&mut trial, id, new_parent) {
            return Ok(false);
        }
        *self.history.edit() = trial;
        self.after_structural_change();
        Ok(true)
    }

    /// Step back. `false` when there is nothing to undo.
    pub fn undo(&mut self) -> EditResult<bool> {
        self.refuse_while_playing()?;
        let stepped = self.history.undo();
        if stepped {
            self.after_structural_change();
        }
        Ok(stepped)
    }

    /// Step forward again.
    pub fn redo(&mut self) -> EditResult<bool> {
        self.refuse_while_playing()?;
        let stepped = self.history.redo();
        if stepped {
            self.after_structural_change();
        }
        Ok(stepped)
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    // --- materials ------------------------------------------------------

    /// The material an entity is drawn with.
    ///
    /// Resolved rather than raw: an entity naming `stone` reports the colour
    /// `stone` actually is, so an inspector's swatch shows what is on screen
    /// rather than the word. [`Session::material_name`] tells the two apart.
    pub fn material(&self, id: EntityId) -> Option<Material> {
        self.history
            .scene()
            .get(id)
            .map(|desc| self.resolve_material(desc))
    }

    /// Give an entity a colour of its own.
    ///
    /// This breaks any link to a named material, which is what dragging a
    /// slider on one object means. Pointing it back at the palette is
    /// [`Session::set_material_name`] — a separate call, because the
    /// difference between "this rock is a bit greener" and "this rock is
    /// moss" is a difference the scene file has to keep.
    pub fn set_material(&mut self, id: EntityId, material: Material) -> EditResult<()> {
        // One undoable step, like a value typed into an inspector. A drag
        // along a colour slider that wants to be one step takes its own
        // snapshot the way a gizmo drag does.
        self.modify(id, |desc| desc.material = MaterialRef::Inline(material))
    }

    /// The name of the material an entity points at, or `None` when it
    /// carries its own colour.
    pub fn material_name(&self, id: EntityId) -> Option<String> {
        match self.line(id).map(|desc| &desc.material) {
            Some(MaterialRef::Named(name)) => Some(name.clone()),
            _ => None,
        }
    }

    /// Point an entity at a material by name — a `.rmat` in the library, or
    /// a builtin.
    ///
    /// An unknown name is accepted, and draws grey. Refusing it would mean
    /// an editor could not name a material before importing it, and the
    /// scene format already treats a name nothing answers to as a visible
    /// mistake rather than a failure. An empty name is refused: clearing the
    /// link means giving the entity a colour.
    pub fn set_material_name(&mut self, id: EntityId, name: &str) -> EditResult<()> {
        if name.is_empty() {
            return Err(EditError::EmptyName("a material"));
        }
        self.modify(id, |desc| {
            desc.material = MaterialRef::Named(name.to_string())
        })
    }

    /// Every material the editor can offer, in the order to show them: the
    /// library's palette first, then the builtins it does not shadow.
    pub fn palette(&self) -> Vec<(String, Material)> {
        let mut out: Vec<(String, Material)> = Vec::new();
        if let Some(library) = self.library.as_ref() {
            for name in library.names_of(runity::asset::AssetKind::Material) {
                if let Some(material) = library.material_by_name(name) {
                    out.push((name.to_string(), material));
                }
            }
        }
        for name in runity::material::builtin::NAMES {
            if out.iter().any(|(existing, _)| existing == name) {
                // Shadowed by the project's own. Listing both would offer a
                // name that means one thing in the list and another in the
                // scene.
                continue;
            }
            if let Some(material) = runity::material::builtin::by_name(name) {
                out.push((name.to_string(), material));
            }
        }
        out
    }

    /// Save an entity's colour as a named material, and point it at it.
    ///
    /// The other half of tuning a colour: a value dragged on one object
    /// stays on that object until it is given a name, and a name is what
    /// every other scene can use. Writes a `.rmat` source into `materials/`
    /// beside the scene and imports it, so the thing the editor produced is
    /// the same kind of file a person would have written.
    pub fn save_material(&mut self, id: EntityId, name: &str) -> EditResult<()> {
        self.refuse_while_playing()?;
        if name.is_empty() {
            return Err(EditError::EmptyName("a material"));
        }
        // The project first: outside one there is nowhere for the source to
        // go, and "no library" would send someone looking for the wrong fix.
        let source_dir = self.material_dir.clone().ok_or(EditError::NotInProject)?;
        let library_dir = self.library_dir.clone().ok_or(EditError::NoLibrary)?;
        let material = self.material(id).ok_or(EditError::NoEntity(id))?;

        // Written as sRGB hex, which is what the format is for: the file
        // that comes out is one a person can read and edit, not a dump of
        // the editor's floats.
        let channel = |value: f32| (runity::material::linear_to_srgb(value) * 255.0).round() as u8;
        let text = format!(
            "(color: \"#{:02x}{:02x}{:02x}\"{})\n",
            channel(material.base_color[0]),
            channel(material.base_color[1]),
            channel(material.base_color[2]),
            if material.shading == runity::Shading::Unlit {
                ", unlit: true"
            } else {
                ""
            }
        );
        std::fs::create_dir_all(&source_dir)?;
        let source = source_dir.join(format!("{name}.rmat"));
        std::fs::write(&source, text)?;
        match &self.project {
            Some(project) => {
                runity_import::import_into(project, &source, None)
                    .map_err(|e| EditError::Import(format!("{e:#}")))?;
            }
            None => {
                let settings = runity_import::ImportSettings::for_source(
                    source.to_string_lossy().into_owned(),
                );
                runity_import::import_file(&source, &library_dir, settings)
                    .map_err(|e| EditError::Import(format!("{e:#}")))?;
            }
        }
        self.reopen_library()?;

        self.edit_entity(id)?.material = MaterialRef::Named(name.to_string());
        self.respawn();
        Ok(())
    }

    // --- prefabs --------------------------------------------------------

    /// Use prefabs from somewhere other than `prefabs/` beside the scene.
    ///
    /// Opening a scene already loads the ones beside it, which is the
    /// convention every tool follows. Returns what was skipped and why.
    pub fn set_prefabs(&mut self, directory: impl AsRef<Path>) -> EditResult<Vec<String>> {
        let directory = directory.as_ref().to_path_buf();
        let (prefabs, problems) =
            runity::Prefabs::open(&directory).map_err(|e| EditError::Io(e.to_string()))?;
        self.prefabs = prefabs;
        self.prefab_dir = Some(directory);
        self.respawn();
        Ok(problems
            .into_iter()
            .map(|(path, e)| format!("{}: {e}", path.display()))
            .collect())
    }

    /// What there is to place, sorted so a list does not reshuffle between
    /// openings.
    pub fn prefab_names(&self) -> Vec<String> {
        self.prefabs
            .names()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// What prefab an entity is an instance of, if any.
    ///
    /// The tree needs this to say so: an instance is one row whose insides
    /// belong to a file, and a row that looks like every other row hides the
    /// difference until someone tries to move a stone and moves twelve.
    pub fn entity_prefab(&self, id: EntityId) -> Option<String> {
        self.history
            .scene()
            .get(id)
            .map(|desc| desc.prefab.clone())
            .filter(|name| !name.is_empty())
    }

    /// Place an instance of a prefab, under `parent` or at the top. Returns
    /// its ID.
    pub fn add_instance(&mut self, parent: Option<EntityId>, prefab: &str) -> EditResult<EntityId> {
        if self.prefabs.get(prefab).is_none() {
            // Refused, unlike an unknown material name. A colour that does
            // not resolve shows grey and can be fixed by typing; an instance
            // of nothing is an entity with no model and no way to tell why.
            return Err(EditError::UnknownPrefab(prefab.to_string()));
        }
        let desc = EntityDesc {
            name: prefab.to_string(),
            model: String::new(),
            prefab: prefab.to_string(),
            ..Default::default()
        };
        self.insert(parent, desc)
    }

    /// Save an entity's subtree as a prefab and make it an instance of it.
    ///
    /// The move that turns a thing arranged once into a thing placed many
    /// times, and the reason it is one call rather than "save it, then
    /// retype it as an instance": doing it by hand leaves the scene holding
    /// a copy that drifts from the file the moment either changes.
    pub fn make_prefab(&mut self, id: EntityId, name: &str) -> EditResult<()> {
        self.refuse_while_playing()?;
        if name.is_empty() {
            return Err(EditError::EmptyName("a prefab"));
        }
        let directory = self.prefab_dir.clone().ok_or(EditError::NotInProject)?;

        // Taken from the expanded document, so making a prefab out of
        // something that already contains an instance writes what it stands
        // for rather than a reference the new file's neighbours may not
        // have.
        // The instance root keeps the document's ID, so this is the whole
        // expanded subtree of the line being turned into a prefab.
        let desc = self
            .instanced
            .scene
            .get(id)
            .cloned()
            .ok_or(EditError::NoEntity(id))?;

        std::fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{name}.{}", runity::prefab::EXTENSION));
        runity::Prefabs::save(&desc, &path).map_err(EditError::Io)?;
        self.prefabs.insert(name.to_string(), desc);

        // The entity becomes an instance: its children now live in the
        // file, and leaving a copy of them in the scene is how the two start
        // to drift.
        let entity = self.edit_entity(id)?;
        entity.prefab = name.to_string();
        entity.model = String::new();
        entity.children.clear();
        self.respawn();
        Ok(())
    }

    // --- the library ----------------------------------------------------

    /// Import a source file: drag-and-drop.
    ///
    /// The editor is the one side that links the importer, so a model
    /// dropped on a window becomes an asset without anybody running a
    /// command. The shipped game still links none of it.
    ///
    /// Into the project's library, named relative to the project. A source
    /// from outside the project is imported by absolute path, and the
    /// warning that comes back says a clone elsewhere will not find it —
    /// that is for the editor to show, not to swallow.
    pub fn import(&mut self, source: impl AsRef<Path>) -> EditResult<Vec<String>> {
        let source = source.as_ref();
        let warnings = match &self.project {
            Some(project) => {
                let report = runity_import::import_into(project, source, None)
                    .map_err(|e| EditError::Import(format!("{e:#}")))?;
                report.warning.into_iter().collect()
            }
            // A scene in no project, with a library set by hand: imported
            // there, with its sidecar beside the source as always.
            None => {
                let library_dir = self.library_dir.clone().ok_or(EditError::NoLibrary)?;
                let settings = runity_import::ImportSettings::for_source(
                    source.to_string_lossy().into_owned(),
                );
                runity_import::import_file(source, &library_dir, settings)
                    .map_err(|e| EditError::Import(format!("{e:#}")))?;
                Vec::new()
            }
        };
        self.reopen_library()?;
        Ok(warnings)
    }

    /// Rebuild what changed on disk and re-read it: the hot loop.
    ///
    /// Returns how many assets came back different. Sources are rebuilt
    /// first — a changed `.png` becomes a changed `.rasset` — and then the
    /// library re-reads exactly those, so a colour tweaked in a text file
    /// shows up in the viewport without anything being reopened.
    pub fn reload_assets(&mut self) -> usize {
        // In a project the sources are the truth: whatever changed, moved
        // or appeared in `assets/` and `materials/` is rebuilt first, and the
        // library read again if anything was. Without one there are no
        // sources to look at, only a library to re-read.
        if let Some(project) = &self.project {
            let synced = runity_import::sync(project);
            let changed = synced.iter().filter(|r| r.result.is_ok()).count();
            if changed > 0 {
                // New and moved assets are files the open library has never
                // seen, so it is read again rather than refreshed.
                let _ = self.reopen_library();
            }
            return changed;
        }
        let Some(library) = self.library.as_mut() else {
            return 0;
        };
        let changed = library.reload_changed();
        if !changed.is_empty() {
            // Mesh handles point at what was uploaded from the old bytes.
            self.uploaded.clear();
            self.respawn();
        }
        changed.len()
    }

    // --- the view -------------------------------------------------------

    pub fn camera(&self) -> Camera {
        self.camera
    }

    /// Move the camera. Flying around is not an edit.
    pub fn set_camera(&mut self, eye: Vec3, target: Vec3) {
        self.camera.position = eye;
        self.camera.target = target;
    }

    /// Write where the editor is looking into the scene, as one undoable
    /// step.
    ///
    /// Separate from [`Session::set_camera`] on purpose: a scene that
    /// changed every time someone looked at it from a different angle would
    /// produce a diff on every open. Keeping the viewpoint is a decision, so
    /// it is a call.
    pub fn capture_camera(&mut self) -> EditResult<()> {
        self.refuse_while_playing()?;
        let view = runity::captured_view(&self.camera);
        self.history.edit().view = view;
        Ok(())
    }

    /// Point the camera at the selected entity, close enough to fill the
    /// view. `false` with nothing selected.
    ///
    /// The one editor command nobody notices until it is missing: without
    /// it, selecting something in a list means hunting for it by flying
    /// around, and anything small enough to be hard to see is also too small
    /// to fly to.
    pub fn focus_selected(&mut self) -> bool {
        let Some(id) = self.selected else {
            return false;
        };
        let Some(target) = self.world_position(id) else {
            return false;
        };
        // How big the thing is, so a boulder and a pebble both end up
        // filling the frame rather than one of them being a dot.
        let radius = self.selected_radius(id).max(0.05);
        let half_fov = (self.camera.fov_y_degrees * 0.5).to_radians().max(1e-3);
        // A little further than the geometry needs, so the thing is framed
        // rather than touching the edges.
        let distance = radius / half_fov.sin() * 1.3;
        let back = (self.camera.position - self.camera.target).normalize_or_zero();
        let back = if back.length_squared() < 1e-6 {
            Vec3::new(0.0, 0.4, 1.0).normalize()
        } else {
            back
        };
        self.camera.target = target;
        self.camera.position = target + back * distance;
        true
    }

    /// Draw one frame into the session's image.
    pub fn render(&mut self) {
        let scene = self.history.scene();
        let mut frame = Frame {
            camera: self.camera,
            // From the scene's hour, like every other tool: an editor
            // lighting a scene differently from the render is an editor you
            // cannot trust about anything you are looking at.
            lighting: runity::scene_lighting(&scene.sun),
            fog: FogSettings {
                color: Vec3::from_array(scene.fog.color),
                start: scene.fog.start,
                end: scene.fog.end,
            },
            clear_color: Vec3::from_array(scene.fog.color),
            ..runity::build_frame(
                &self.world,
                self.camera,
                Lighting::default(),
                FogSettings::default(),
            )
        };
        // The gizmo goes in after the scene's own draws and before the frame
        // is submitted, so it is part of the same pass and does not need a
        // second one. It is unlit and drawn last, which is what keeps a
        // handle visible against anything.
        if let Some(origin) = self.selected_origin() {
            let arm = match self.gizmo_arm {
                Some(arm) => arm,
                None => {
                    let cube = builtin::cube(1.0);
                    let arm = self.renderer.upload_mesh_owned(&self.gpu, &cube);
                    self.gizmo_arm = Some(arm);
                    arm
                }
            };
            frame.overlay_draws.extend(gizmo::draws_for(
                self.tool,
                arm,
                &self.camera,
                &self.gizmo_style,
                origin,
                self.drag.map(|d| d.handle),
            ));
        }
        self.renderer.render(&self.gpu, &self.target, &frame);
        self.pixels = self.target.read_rgba(&self.gpu);
    }

    /// The last rendered frame, RGBA8, top row first.
    pub fn frame_pixels(&self) -> &[u8] {
        &self.pixels
    }

    // --- selection and the gizmo ----------------------------------------

    /// The entity under a point in the image.
    ///
    /// Tested against bounding boxes, against the expanded scene, and
    /// answered with a document entity: clicking a stone that came out of a
    /// prefab selects the fire that brought it, because the fire is the
    /// thing the document can move. A triangle-exact pick is better and
    /// much slower, and for a box the difference only shows on thin
    /// diagonal geometry.
    pub fn pick(&self, x: u32, y: u32) -> Option<EntityId> {
        let (near, direction) = self.ray(x, y);
        let mut best: Option<(f32, EntityId)> = None;
        for (desc, world) in self.instanced.scene.flatten() {
            let Some(bounds) = self.bounds_of(&desc.model) else {
                continue;
            };
            let Some(owner) = self.instanced.owner_of(desc.id) else {
                continue;
            };
            if let Some(distance) = ray_box(near, direction, bounds, world) {
                if best.is_none_or(|(closest, _)| distance < closest) {
                    best = Some((distance, owner));
                }
            }
        }
        best.map(|(_, id)| id)
    }

    /// Put the gizmo on an entity, or clear the selection with `None`.
    pub fn select(&mut self, id: Option<EntityId>) -> EditResult<()> {
        if let Some(id) = id {
            self.require(id)?;
        }
        self.selected = id;
        self.also_selected.clear();
        self.drag = None;
        Ok(())
    }

    pub fn selected(&self) -> Option<EntityId> {
        self.selected
    }

    /// Add an entity to the selection — shift-click. The first selected
    /// stays the one the gizmo is on.
    pub fn add_to_selection(&mut self, id: EntityId) -> EditResult<()> {
        self.require(id)?;
        match self.selected {
            None => self.selected = Some(id),
            Some(first) if first == id => {}
            Some(_) => {
                if !self.also_selected.contains(&id) {
                    self.also_selected.push(id);
                }
            }
        }
        Ok(())
    }

    /// Everything selected, the gizmo's first.
    pub fn selection(&self) -> Vec<EntityId> {
        self.selected
            .into_iter()
            .chain(self.also_selected.iter().copied())
            .collect()
    }

    /// The selection without anything whose ancestor is also selected: what
    /// a group operation acts on, so a child is not moved twice or copied
    /// inside its own copied parent.
    fn selection_roots(&self) -> Vec<EntityId> {
        let chosen = self.selection();
        let scene = self.history.scene();
        chosen
            .iter()
            .copied()
            .filter(|id| {
                !chosen.iter().any(|other| {
                    other != id
                        && scene
                            .get(*other)
                            .is_some_and(|desc| desc.flatten().iter().any(|(d, _)| d.id == *id))
                })
            })
            .collect()
    }

    /// Delete everything selected, as one undoable step.
    pub fn delete_selection(&mut self) -> EditResult<usize> {
        self.refuse_while_playing()?;
        let roots = self.selection_roots();
        if roots.is_empty() {
            return Ok(0);
        }
        let scene = self.history.edit();
        for id in &roots {
            runity::edit::remove(scene, *id);
        }
        self.after_structural_change();
        Ok(roots.len())
    }

    /// Copy everything selected beside itself, as one undoable step, and
    /// select the copies.
    pub fn duplicate_selection(&mut self) -> EditResult<Vec<EntityId>> {
        self.refuse_while_playing()?;
        let roots = self.selection_roots();
        if roots.is_empty() {
            return Ok(Vec::new());
        }
        let scene = self.history.edit();
        let copies: Vec<EntityId> = roots
            .iter()
            .filter_map(|id| runity::edit::duplicate(scene, *id))
            .collect();
        self.respawn();
        self.select_all(&copies);
        Ok(copies)
    }

    /// Move everything selected by `offset`, as one undoable step.
    pub fn translate_selection(&mut self, offset: Vec3) -> EditResult<()> {
        self.refuse_while_playing()?;
        let roots = self.selection_roots();
        if roots.is_empty() {
            return Ok(());
        }
        let scene = self.history.edit();
        for id in roots {
            if let Some(desc) = scene.get_mut(id) {
                desc.transform.position += offset;
            }
        }
        self.respawn();
        Ok(())
    }

    /// The selection as text: the entities in the scene's RON, children
    /// included — what a clipboard holds. Paste it into this scene or
    /// another, or hand it to an agent.
    pub fn copy_selection(&self) -> String {
        let scene = self.history.scene();
        let entities: Vec<EntityDesc> = self
            .selection_roots()
            .iter()
            .filter_map(|id| scene.get(*id).cloned())
            .collect();
        let pretty = runity::ron::ser::PrettyConfig::new().depth_limit(3);
        runity::ron::ser::to_string_pretty(&entities, pretty).unwrap_or_default()
    }

    /// Paste entities copied as text — [`Session::copy_selection`], or
    /// written by hand — under `parent` or at the top, as one undoable step.
    /// Everything pasted gets new IDs: a paste is a new thing, even into the
    /// scene it was copied from. The pasted entities become the selection.
    pub fn paste(&mut self, text: &str, parent: Option<EntityId>) -> EditResult<Vec<EntityId>> {
        self.refuse_while_playing()?;
        let mut entities: Vec<EntityDesc> = runity::ron::from_str(text.trim())
            .or_else(|_| runity::ron::from_str::<EntityDesc>(text.trim()).map(|one| vec![one]))
            .map_err(|e| EditError::Scene(format!("not entities in RON: {e}")))?;
        if let Some(parent) = parent {
            self.require(parent)?;
        }
        fn forget(desc: &mut EntityDesc) {
            desc.id = EntityId::default();
            desc.children.iter_mut().for_each(forget);
        }
        entities.iter_mut().for_each(forget);
        let scene = self.history.edit();
        let pasted: Vec<EntityId> = entities
            .into_iter()
            .filter_map(|desc| runity::edit::add(scene, parent, desc))
            .collect();
        self.respawn();
        self.select_all(&pasted);
        Ok(pasted)
    }

    fn select_all(&mut self, ids: &[EntityId]) {
        self.selected = ids.first().copied();
        self.also_selected = ids.iter().skip(1).copied().collect();
        self.drag = None;
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// Choose what the gizmo does.
    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        // A held handle belongs to the tool that was showing when it was
        // grabbed; keeping it across a change would drag a ring that is no
        // longer drawn.
        self.drag = None;
        self.drag_from = None;
    }

    pub fn snap(&self) -> Snap {
        self.snap
    }

    /// Set the grid a drag lands on.
    ///
    /// Applied to the result rather than to the movement, so a drag lands on
    /// the grid instead of on wherever it started plus a whole number of
    /// steps. Anything not a positive number turns that axis off.
    pub fn set_snap(&mut self, snap: Snap) {
        let sane = |v: f32| if v.is_finite() && v > 0.0 { v } else { 0.0 };
        self.snap = Snap {
            meters: sane(snap.meters),
            degrees: sane(snap.degrees),
            scale: sane(snap.scale),
        };
    }

    /// Which handle is under a point, without grabbing it.
    pub fn gizmo_hover(&self, x: u32, y: u32) -> Option<Handle> {
        let origin = self.selected_origin()?;
        let (from, direction) = self.ray(x, y);
        gizmo::hit_for(
            self.tool,
            &self.camera,
            &self.gizmo_style,
            origin,
            from,
            direction,
        )
    }

    /// Grab whatever handle is under a point.
    pub fn gizmo_begin(&mut self, x: u32, y: u32) -> EditResult<Option<Handle>> {
        self.refuse_while_playing()?;
        let Some(origin) = self.selected_origin() else {
            return Ok(None);
        };
        let (from, direction) = self.ray(x, y);
        let Some(handle) = gizmo::hit_for(
            self.tool,
            &self.camera,
            &self.gizmo_style,
            origin,
            from,
            direction,
        ) else {
            return Ok(None);
        };
        // One snapshot for the whole gesture: everything until the next one
        // undoes as a single step, however many frames the drag lasts.
        self.history.snapshot();
        self.drag_from = self.selected.and_then(|id| self.transform(id));
        self.drag = Some(gizmo::begin_for(self.tool, origin, handle, from, direction));
        Ok(Some(handle))
    }

    /// Move the held handle to follow a point. `false` without a grab.
    pub fn gizmo_drag(&mut self, x: u32, y: u32) -> EditResult<bool> {
        self.refuse_while_playing()?;
        let (Some(drag), Some(id)) = (self.drag, self.selected) else {
            return Ok(false);
        };
        let (from, direction) = self.ray(x, y);
        let motion = gizmo::update_for(&drag, from, direction);

        // The gizmo sits at the entity's world position, but what is edited
        // is its local one. The difference is the parent's transform, and
        // applying the move in world space without undoing it drags a child
        // out of its parent by however much the parent is offset.
        let parent = self.parent_matrix(id);
        let started = self.drag_from;
        let snap = self.snap;
        // Untracked: the snapshot for this gesture was taken at
        // `gizmo_begin`.
        let desc = self
            .history
            .scene_mut_untracked()
            .get_mut(id)
            .ok_or(EditError::NoEntity(id))?;
        match motion {
            Motion::Position(moved) => {
                // Snapped in local space, which is the space the file holds
                // and the space a person means: a child snapped in world
                // space lands on a grid its parent is not on.
                let local = parent.inverse().transform_point3(moved);
                desc.transform.position = gizmo::snap_all(local, snap.meters);
            }
            Motion::Rotation(delta) => {
                let Some(started) = started else {
                    return Ok(false);
                };
                // The turn is in world axes and the file holds a local
                // rotation, so the parent's own turn has to come out first —
                // the same correction the move path makes for position.
                let (_, parent_rotation, _) = parent.to_scale_rotation_translation();
                let local = parent_rotation.inverse() * delta * parent_rotation;
                desc.transform.set_rotation(local * started.rotation());
                // Snapped as degrees, which is what the file holds and what
                // an inspector shows; snapping a quaternion is not a thing.
                desc.transform.rotation_deg =
                    gizmo::snap_all(desc.transform.rotation_deg, snap.degrees);
            }
            Motion::Scale(factor) => {
                let Some(started) = started else {
                    return Ok(false);
                };
                desc.transform.scale = gizmo::snap_all(started.scale * factor, snap.scale);
            }
        }
        self.respawn();
        Ok(true)
    }

    /// Let go. Safe without a grab.
    pub fn gizmo_end(&mut self) {
        self.drag = None;
        self.drag_from = None;
    }

    // --- play mode ------------------------------------------------------

    /// Start simulating the scene.
    ///
    /// The document is kept aside and restored when play stops, so a thing
    /// that fell over stays fallen only as long as you are watching it.
    pub fn play(&mut self) {
        if self.play.is_some() {
            return;
        }
        let fixed = runity::TimeSettings::default().fixed_delta;
        let mut physics = runity::PhysicsWorld::new(fixed);
        // Built from the world rather than the scene, so what is simulated
        // is exactly what is drawn — prefabs already expanded, hierarchy
        // already applied.
        runity::physics::attach_scene_collision_meshes(
            &mut self.world,
            &self.instanced.scene,
            self.library.as_ref(),
        );
        physics.sync_from_world(&mut self.world);
        self.play = Some(Play {
            physics,
            clock: runity::Time::new(runity::TimeSettings::default()),
            before: self.history.scene().clone(),
        });
        self.drag = None;
        self.drag_from = None;
    }

    /// Advance the simulation by however long the caller's frame took.
    ///
    /// Returns how many fixed steps were taken — zero on a fast frame,
    /// several on a slow one. Nothing happens unless play has started.
    pub fn step(&mut self, seconds: f32) -> u32 {
        let Some(play) = self.play.as_mut() else {
            return 0;
        };
        let seconds = seconds.max(0.0);
        play.clock.advance(seconds);
        let mut steps = 0;
        while play.clock.next_step().is_some() {
            play.physics.step();
            steps += 1;
        }
        if steps > 0 {
            play.physics.sync_to_world(&mut self.world);
        }
        // Animation runs on the frame rather than the step: a pose
        // interpolates and does not need to be deterministic the way a
        // solver does.
        runity::advance_animations(&mut self.world, seconds);
        steps
    }

    /// Stop simulating and put the scene back as it was. `false` when it
    /// was not playing.
    pub fn stop(&mut self) -> bool {
        let Some(play) = self.play.take() else {
            return false;
        };
        // Untracked: starting and stopping a preview is not something to
        // undo, and putting it on the stack would mean pressing play cost a
        // step of real editing history.
        *self.history.scene_mut_untracked() = play.before;
        self.respawn();
        true
    }

    pub fn is_playing(&self) -> bool {
        self.play.is_some()
    }

    // --- inside ---------------------------------------------------------

    fn refuse_while_playing(&self) -> EditResult<()> {
        if self.play.is_some() {
            Err(EditError::Playing)
        } else {
            Ok(())
        }
    }

    /// That the document has an entity with this ID.
    fn require(&self, id: EntityId) -> EditResult<()> {
        match self.history.scene().get(id) {
            Some(_) => Ok(()),
            None => Err(EditError::NoEntity(id)),
        }
    }

    /// The scene with its prefab instances expanded: what is drawn, and
    /// where the parts of instances are found by their IDs.
    pub fn expanded(&self) -> &Scene {
        &self.instanced.scene
    }

    /// An entity's line: the document's, or for a part a prefab instance
    /// brought, the part as this instance has it.
    fn line(&self, id: EntityId) -> Option<&EntityDesc> {
        self.history
            .scene()
            .get(id)
            .or_else(|| self.instanced.scene.get(id))
    }

    /// The entity with `id`, for an edit that is one undoable step.
    fn edit_entity(&mut self, id: EntityId) -> EditResult<&mut EntityDesc> {
        self.refuse_while_playing()?;
        // Checked before `edit` takes a snapshot, so a call with an ID that
        // is not there does not leave an empty step on the undo stack.
        self.require(id)?;
        self.history
            .edit()
            .get_mut(id)
            .ok_or(EditError::NoEntity(id))
    }

    fn insert(&mut self, parent: Option<EntityId>, desc: EntityDesc) -> EditResult<EntityId> {
        self.refuse_while_playing()?;
        if let Some(parent) = parent {
            self.require(parent)?;
        }
        let id = runity::edit::add(self.history.edit(), parent, desc)
            .expect("the parent was checked a line above");
        self.respawn();
        Ok(id)
    }

    /// After anything that can remove entities: keep the selection if what
    /// it names is still there, which by ID it usually is.
    fn after_structural_change(&mut self) {
        if let Some(id) = self.selected {
            if self.history.scene().get(id).is_none() {
                self.selected = None;
            }
        }
        let scene = self.history.scene();
        self.also_selected.retain(|id| scene.get(*id).is_some());
        if self.selected.is_none() && !self.also_selected.is_empty() {
            self.selected = Some(self.also_selected.remove(0));
        }
        self.drag = None;
        self.drag_from = None;
        self.respawn();
    }

    /// Rebuild the world from the scene.
    fn respawn(&mut self) {
        self.world.clear();
        // Instances expanded first, so everything past this point — the
        // world, the frame, a click — sees a plain tree and knows nothing
        // about prefabs.
        self.instanced = runity::instantiate(self.history.scene(), &self.prefabs);
        let renderer = &mut self.renderer;
        let gpu = &self.gpu;
        let library = self.library.as_ref();
        let uploaded = &mut self.uploaded;
        runity::spawn_scene_with(
            &self.instanced.scene,
            &mut self.world,
            |name| {
                if let Some(found) = uploaded.iter().find(|(n, _)| n == name) {
                    return Some(found.1);
                }
                let handle = if let Some(mesh) = builtin::by_name(name) {
                    renderer.upload_mesh_owned(gpu, &mesh)
                } else {
                    renderer.upload_mesh(gpu, library?.mesh_by_name(name)?)
                };
                uploaded.push((name.to_string(), handle));
                Some(handle)
            },
            |name| library?.material_by_name(name),
        );
    }

    /// Re-read the library from disk, keeping nothing uploaded from before.
    fn reopen_library(&mut self) -> EditResult<()> {
        let directory = self.library_dir.clone().ok_or(EditError::NoLibrary)?;
        let (library, _skipped) =
            Library::open(&directory).map_err(|e| EditError::Io(e.to_string()))?;
        self.library = Some(library);
        self.uploaded.clear();
        self.respawn();
        Ok(())
    }

    /// The material an entity is actually drawn with.
    ///
    /// The one place that knows the resolution order, so the inspector and
    /// the frame cannot disagree about what colour something is.
    fn resolve_material(&self, desc: &EntityDesc) -> Material {
        desc.material_from(|name| self.library.as_ref()?.material_by_name(name))
    }

    /// How big a document entity is, as a radius around it.
    ///
    /// Measured over the expanded subtree, so focusing on a campfire frames
    /// the ring of stones rather than the patch of earth under it.
    fn selected_radius(&self, id: EntityId) -> f32 {
        let Some(centre) = self.world_position(id) else {
            return 0.0;
        };
        let mut radius: f32 = 0.0;
        for (desc, world) in self.instanced.scene.flatten() {
            if self.instanced.owner_of(desc.id) != Some(id) {
                continue;
            }
            let Some((low, high)) = self.bounds_of(&desc.model) else {
                continue;
            };
            for corner in 0..8u32 {
                // Every corner of the box, transformed: the far corner of a
                // rotated box is further away than the box's own extents
                // suggest, and framing by extents alone clips it.
                let pick = |axis: usize| {
                    if corner & (1 << axis) == 0 {
                        low[axis]
                    } else {
                        high[axis]
                    }
                };
                let local = Vec3::new(pick(0), pick(1), pick(2));
                radius = radius.max((world.transform_point3(local) - centre).length());
            }
        }
        radius
    }

    /// The world ray through a pixel.
    fn ray(&self, x: u32, y: u32) -> (Vec3, Vec3) {
        let (width, height) = {
            let (w, h) = self.size();
            (w as f32, h as f32)
        };
        let ndc_x = (x as f32 + 0.5) / width * 2.0 - 1.0;
        let ndc_y = 1.0 - (y as f32 + 0.5) / height * 2.0;
        let inverse = self.camera.view_projection(width / height).inverse();
        let near = inverse.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let far = inverse.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        (near, (far - near).normalize_or_zero())
    }

    /// Where the gizmo sits: the selected entity's world position.
    fn selected_origin(&self) -> Option<Vec3> {
        let id = self.selected?;
        let (_, world) = self.placed(id)?;
        Some(world.w_axis.truncate())
    }

    /// The transform an entity's parents impose on it.
    fn parent_matrix(&self, id: EntityId) -> Mat4 {
        let Some((desc, world)) = self.placed(id) else {
            return Mat4::IDENTITY;
        };
        world * desc.transform.matrix().inverse()
    }

    /// An entity of the document and where its parents put it.
    fn placed(&self, id: EntityId) -> Option<(&EntityDesc, Mat4)> {
        self.history
            .scene()
            .flatten()
            .into_iter()
            .find(|(desc, _)| desc.id == id)
    }

    fn bounds_of(&self, model: &str) -> Option<(Vec3, Vec3)> {
        if let Some(mesh) = builtin::by_name(model) {
            return Some((
                Vec3::from_array(mesh.bounds.min),
                Vec3::from_array(mesh.bounds.max),
            ));
        }
        let mesh = self.library.as_ref()?.mesh_by_name(model)?;
        let (min, max) = (&mesh.bounds.min, &mesh.bounds.max);
        Some((
            Vec3::new(min[0].to_native(), min[1].to_native(), min[2].to_native()),
            Vec3::new(max[0].to_native(), max[1].to_native(), max[2].to_native()),
        ))
    }
}

/// Distance along a ray to a transformed box, if it hits.
///
/// The ray is moved into the box's space rather than the box into the
/// world's: a rotated box is not a box any more, and testing its world-space
/// bounds would pick things the cursor is nowhere near.
fn ray_box(origin: Vec3, direction: Vec3, bounds: (Vec3, Vec3), transform: Mat4) -> Option<f32> {
    let inverse = transform.inverse();
    let local_origin = inverse.transform_point3(origin);
    let local_direction = inverse.transform_vector3(direction);

    let (mut near, mut far) = (f32::NEG_INFINITY, f32::INFINITY);
    for axis in 0..3 {
        let d = local_direction[axis];
        let (lo, hi) = (bounds.0[axis], bounds.1[axis]);
        if d.abs() < 1e-6 {
            // Parallel to this slab: a miss unless it starts inside it.
            if local_origin[axis] < lo || local_origin[axis] > hi {
                return None;
            }
            continue;
        }
        let t0 = (lo - local_origin[axis]) / d;
        let t1 = (hi - local_origin[axis]) / d;
        let (t0, t1) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
        near = near.max(t0);
        far = far.min(t1);
        if near > far {
            return None;
        }
    }
    (far >= 0.0).then(|| near.max(0.0))
}
