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

use std::path::{Path, PathBuf};

use runity::gizmo::{self, Drag, GizmoStyle, Handle, Motion, Tool};
use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, FogSettings, Frame, Lighting, MeshHandle};
use runity::scene::MaterialRef;
use runity::{builtin, EntityDesc, Gpu, Library, Material, OffscreenTarget, Renderer, Scene};

pub use error::EditError;

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
    library: Option<Library>,
    /// Where that library sits, so the editor can import into it.
    library_dir: Option<PathBuf>,
    /// Where `.rmat` sources go: `materials/` beside the scene, by the same
    /// convention prefabs follow.
    material_dir: Option<PathBuf>,
    /// Prefabs a scene's instances name. Loaded from `prefabs/` beside the
    /// scene when one is opened, so the editor finds the same ones the
    /// headless render does.
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
    /// Which entity the gizmo is on, by flattened index.
    selected: Option<usize>,
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
        // Prefabs come from `prefabs/` beside the scene, by the same
        // convention the headless render uses. An editor that had to be told
        // where they are would be an editor that shows a different scene
        // than the one CI renders.
        let (prefabs, problems) = runity::Prefabs::beside(&path);
        self.prefab_dir = path.parent().map(|d| d.join("prefabs"));
        self.material_dir = path.parent().map(|d| d.join("materials"));
        self.prefabs = prefabs;
        self.history.replace(scene);
        self.scene_path = Some(path);
        self.selected = None;
        self.drag = None;
        self.respawn();
        Ok(problems
            .into_iter()
            .map(|(path, e)| format!("{}: {e}", path.display()))
            .collect())
    }

    /// Write the scene back: to `path`, or where it was opened from.
    pub fn save_scene(&self, path: Option<&Path>) -> EditResult<()> {
        let target = path
            .map(Path::to_path_buf)
            .or_else(|| self.scene_path.clone())
            .ok_or(EditError::NoPath)?;
        self.history
            .scene()
            .save(&target)
            .map_err(|e| EditError::Scene(format!("{e:#}")))
    }

    /// The document as it stands.
    pub fn scene(&self) -> &Scene {
        self.history.scene()
    }

    // --- the scene tree, flattened --------------------------------------

    /// How many entities the open scene has, counting children.
    pub fn entity_count(&self) -> usize {
        self.history.scene().flatten().len()
    }

    /// One entity's name.
    pub fn entity_name(&self, index: usize) -> Option<String> {
        let flat = self.history.scene().flatten();
        flat.get(index).map(|(desc, _)| desc.name.clone())
    }

    /// An entity's local transform — the one the file holds.
    pub fn transform(&self, index: usize) -> Option<runity::Transform> {
        let flat = self.history.scene().flatten();
        flat.get(index).map(|(desc, _)| desc.transform)
    }

    /// Set an entity's local transform, as one undoable step.
    pub fn set_transform(&mut self, index: usize, transform: runity::Transform) -> EditResult<()> {
        // Recorded: a value typed into an inspector is one undoable edit. A
        // drag is not, because `gizmo_begin` already took the snapshot that
        // covers the whole gesture.
        let desc = self.edit_entity(index)?;
        desc.transform = transform;
        // Respawned rather than patched in place: a moved parent moves its
        // children, and keeping two ways to apply that is how they drift.
        self.respawn();
        Ok(())
    }

    /// Where an entity actually is, in world space.
    ///
    /// Not the same as its transform. The transform is local and belongs to
    /// the file; this is where the thing ends up once its parents — and,
    /// while play is running, the simulation — have had their say. An
    /// inspector that showed only the local one would say a falling crate is
    /// still four metres up.
    pub fn world_position(&self, index: usize) -> Option<Vec3> {
        let mut best: Option<(usize, Vec3)> = None;
        for (scene_index, placed) in self
            .world
            .query::<(&runity::SceneIndex, &runity::world::WorldTransform)>()
            .iter()
        {
            // The first spawned entity belonging to that row is the answer:
            // for an instance that is the prefab's root, which is the thing
            // the row stands for.
            let row = self
                .instanced
                .source
                .get(scene_index.0)
                .copied()
                .unwrap_or(scene_index.0);
            if row != index {
                continue;
            }
            if best.is_none_or(|(first, _)| scene_index.0 < first) {
                best = Some((scene_index.0, placed.0.w_axis.truncate()));
            }
        }
        best.map(|(_, position)| position)
    }

    // --- editing, and taking it back ------------------------------------

    /// Add an entity with a model, under `parent` or at the top. Returns its
    /// index.
    pub fn add(&mut self, parent: Option<usize>, model: &str) -> EditResult<usize> {
        let desc = EntityDesc {
            name: "entity".into(),
            model: model.to_string(),
            ..Default::default()
        };
        self.insert(parent, desc)
    }

    /// Delete an entity and everything under it.
    pub fn delete(&mut self, index: usize) -> EditResult<()> {
        self.refuse_while_playing()?;
        runity::edit::remove(self.history.edit(), index).ok_or(EditError::NoEntity(index))?;
        // The selection is an index into a list that just changed shape.
        // Keeping it would point the gizmo at whatever slid into the gap.
        self.selected = None;
        self.drag = None;
        self.respawn();
        Ok(())
    }

    /// Copy an entity beside itself. Returns the copy's index.
    pub fn duplicate(&mut self, index: usize) -> EditResult<usize> {
        self.refuse_while_playing()?;
        let copy = runity::edit::duplicate(self.history.edit(), index)
            .ok_or(EditError::NoEntity(index))?;
        self.respawn();
        Ok(copy)
    }

    /// Move an entity under another, or to the top. Refuses to make
    /// something its own ancestor, and says `false` when it did.
    pub fn reparent(&mut self, index: usize, new_parent: Option<usize>) -> EditResult<bool> {
        self.refuse_while_playing()?;
        let moved = runity::edit::reparent(self.history.edit(), index, new_parent);
        if moved {
            self.selected = None;
            self.respawn();
        }
        Ok(moved)
    }

    /// Step back. `false` when there is nothing to undo.
    pub fn undo(&mut self) -> EditResult<bool> {
        self.refuse_while_playing()?;
        let stepped = self.history.undo();
        if stepped {
            self.selected = None;
            self.drag = None;
            self.respawn();
        }
        Ok(stepped)
    }

    /// Step forward again.
    pub fn redo(&mut self) -> EditResult<bool> {
        self.refuse_while_playing()?;
        let stepped = self.history.redo();
        if stepped {
            self.selected = None;
            self.drag = None;
            self.respawn();
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
    pub fn material(&self, index: usize) -> Option<Material> {
        let flat = self.history.scene().flatten();
        flat.get(index).map(|(desc, _)| self.resolve_material(desc))
    }

    /// Give an entity a colour of its own.
    ///
    /// This breaks any link to a named material, which is what dragging a
    /// slider on one object means. Pointing it back at the palette is
    /// [`Session::set_material_name`] — a separate call, because the
    /// difference between "this rock is a bit greener" and "this rock is
    /// moss" is a difference the scene file has to keep.
    pub fn set_material(&mut self, index: usize, material: Material) -> EditResult<()> {
        // One undoable step, like a value typed into an inspector. A drag
        // along a colour slider that wants to be one step takes its own
        // snapshot the way a gizmo drag does.
        self.edit_entity(index)?.material = MaterialRef::Inline(material);
        self.respawn();
        Ok(())
    }

    /// The name of the material an entity points at, or `None` when it
    /// carries its own colour.
    pub fn material_name(&self, index: usize) -> Option<String> {
        let flat = self.history.scene().flatten();
        match flat.get(index).map(|(desc, _)| &desc.material) {
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
    pub fn set_material_name(&mut self, index: usize, name: &str) -> EditResult<()> {
        if name.is_empty() {
            return Err(EditError::EmptyName("a material"));
        }
        self.edit_entity(index)?.material = MaterialRef::Named(name.to_string());
        self.respawn();
        Ok(())
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
    pub fn save_material(&mut self, index: usize, name: &str) -> EditResult<()> {
        self.refuse_while_playing()?;
        if name.is_empty() {
            return Err(EditError::EmptyName("a material"));
        }
        let library_dir = self.library_dir.clone().ok_or(EditError::NoLibrary)?;
        let source_dir = self
            .material_dir
            .clone()
            .ok_or(EditError::NoSceneDirectory)?;
        let material = self.material(index).ok_or(EditError::NoEntity(index))?;

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
        let settings =
            runity_import::ImportSettings::for_source(source.to_string_lossy().into_owned());
        runity_import::import_file(&source, &library_dir, settings)
            .map_err(|e| EditError::Import(format!("{e:#}")))?;
        self.reopen_library()?;

        self.edit_entity(index)?.material = MaterialRef::Named(name.to_string());
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
    pub fn entity_prefab(&self, index: usize) -> Option<String> {
        let flat = self.history.scene().flatten();
        flat.get(index)
            .map(|(desc, _)| desc.prefab.clone())
            .filter(|name| !name.is_empty())
    }

    /// Place an instance of a prefab, under `parent` or at the top. Returns
    /// its index.
    pub fn add_instance(&mut self, parent: Option<usize>, prefab: &str) -> EditResult<usize> {
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
    pub fn make_prefab(&mut self, index: usize, name: &str) -> EditResult<()> {
        self.refuse_while_playing()?;
        if name.is_empty() {
            return Err(EditError::EmptyName("a prefab"));
        }
        let directory = self.prefab_dir.clone().ok_or(EditError::NoSceneDirectory)?;

        // Taken from the expanded document, so making a prefab out of
        // something that already contains an instance writes what it stands
        // for rather than a reference the new file's neighbours may not
        // have.
        let desc = self
            .instanced
            .scene
            .flatten()
            .iter()
            .zip(&self.instanced.source)
            .find(|(_, source)| **source == index)
            .map(|((desc, _), _)| (*desc).clone())
            .ok_or(EditError::NoEntity(index))?;

        std::fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{name}.{}", runity::prefab::EXTENSION));
        runity::Prefabs::save(&desc, &path).map_err(EditError::Io)?;
        self.prefabs.insert(name.to_string(), desc);

        // The entity becomes an instance: its children now live in the
        // file, and leaving a copy of them in the scene is how the two start
        // to drift.
        let entity = self.edit_entity(index)?;
        entity.prefab = name.to_string();
        entity.model = String::new();
        entity.children.clear();
        self.respawn();
        Ok(())
    }

    // --- the library ----------------------------------------------------

    /// Import a source file into the open library: drag-and-drop.
    ///
    /// The editor is the one side that links the importer, so a model
    /// dropped on a window becomes an asset without anybody running a
    /// command. The shipped game still links none of it.
    pub fn import(&mut self, source: impl AsRef<Path>) -> EditResult<()> {
        let source = source.as_ref();
        let library_dir = self.library_dir.clone().ok_or(EditError::NoLibrary)?;
        // The sidecar records the path as given. An absolute path from here
        // is a known departure from the DNA's decision on source paths —
        // relative to the project, plus a content hash — which lands with
        // the project layout, where "the project" first means something.
        let settings =
            runity_import::ImportSettings::for_source(source.to_string_lossy().into_owned());
        runity_import::import_file(source, &library_dir, settings)
            .map_err(|e| EditError::Import(format!("{e:#}")))?;
        self.reopen_library()
    }

    /// Rebuild what changed on disk and re-read it: the hot loop.
    ///
    /// Returns how many assets came back different. Sources are rebuilt
    /// first — a changed `.png` becomes a changed `.rasset` — and then the
    /// library re-reads exactly those, so a colour tweaked in a text file
    /// shows up in the viewport without anything being reopened.
    pub fn reload_assets(&mut self) -> usize {
        let Some(library_dir) = self.library_dir.clone() else {
            return 0;
        };
        // Sidecars written by this editor hold absolute paths, which ignore
        // the root; ones written by the command line are relative to the
        // project, and the scene's directory is the closest thing to it the
        // editor knows.
        let root = self
            .scene_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| library_dir.clone());
        runity_import::reimport_changed(&library_dir, &root);

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
        let Some(index) = self.selected else {
            return false;
        };
        let Some(target) = self.world_position(index) else {
            return false;
        };
        // How big the thing is, so a boulder and a pebble both end up
        // filling the frame rather than one of them being a dot.
        let radius = self.selected_radius(index).max(0.05);
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
    /// answered with a document index: clicking a stone that came out of a
    /// prefab selects the fire that brought it, because the fire is the
    /// thing the document can move. A triangle-exact pick is better and
    /// much slower, and for a box the difference only shows on thin
    /// diagonal geometry.
    pub fn pick(&self, x: u32, y: u32) -> Option<usize> {
        let (near, direction) = self.ray(x, y);
        let mut best: Option<(f32, usize)> = None;
        for (index, (desc, world)) in self.instanced.scene.flatten().iter().enumerate() {
            let Some(bounds) = self.bounds_of(&desc.model) else {
                continue;
            };
            if let Some(distance) = ray_box(near, direction, bounds, *world) {
                let owner = self.instanced.source.get(index).copied().unwrap_or(index);
                if best.is_none_or(|(closest, _)| distance < closest) {
                    best = Some((distance, owner));
                }
            }
        }
        best.map(|(_, index)| index)
    }

    /// Put the gizmo on an entity, or clear the selection with `None`.
    pub fn select(&mut self, index: Option<usize>) -> EditResult<()> {
        match index {
            None => {
                self.selected = None;
                self.drag = None;
            }
            Some(index) if index >= self.entity_count() => {
                return Err(EditError::NoEntity(index));
            }
            Some(index) => self.selected = Some(index),
        }
        Ok(())
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
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
        self.drag_from = self.selected.and_then(|index| self.transform(index));
        self.drag = Some(gizmo::begin_for(self.tool, origin, handle, from, direction));
        Ok(Some(handle))
    }

    /// Move the held handle to follow a point. `false` without a grab.
    pub fn gizmo_drag(&mut self, x: u32, y: u32) -> EditResult<bool> {
        self.refuse_while_playing()?;
        let (Some(drag), Some(index)) = (self.drag, self.selected) else {
            return Ok(false);
        };
        let (from, direction) = self.ray(x, y);
        let motion = gizmo::update_for(&drag, from, direction);

        // The gizmo sits at the entity's world position, but what is edited
        // is its local one. The difference is the parent's transform, and
        // applying the move in world space without undoing it drags a child
        // out of its parent by however much the parent is offset.
        let parent = self.parent_matrix(index);
        let started = self.drag_from;
        let snap = self.snap;
        // Untracked: the snapshot for this gesture was taken at
        // `gizmo_begin`.
        let desc = runity::edit::nth_mut(self.history.scene_mut_untracked(), index)
            .ok_or(EditError::NoEntity(index))?;
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

    /// The entity at `index`, for an edit that is one undoable step.
    fn edit_entity(&mut self, index: usize) -> EditResult<&mut EntityDesc> {
        self.refuse_while_playing()?;
        if index >= self.entity_count() {
            // Checked before `edit` takes a snapshot, so a call with a bad
            // index does not leave an empty step on the undo stack.
            return Err(EditError::NoEntity(index));
        }
        runity::edit::nth_mut(self.history.edit(), index).ok_or(EditError::NoEntity(index))
    }

    fn insert(&mut self, parent: Option<usize>, desc: EntityDesc) -> EditResult<usize> {
        self.refuse_while_playing()?;
        if let Some(parent) = parent.filter(|p| *p >= self.entity_count()) {
            return Err(EditError::NoEntity(parent));
        }
        let index = runity::edit::add(self.history.edit(), parent, desc)
            .ok_or(EditError::NoEntity(parent.unwrap_or(0)))?;
        self.respawn();
        Ok(index)
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

    /// How big the thing at a document index is, as a radius around it.
    ///
    /// Measured over the expanded subtree, so focusing on a campfire frames
    /// the ring of stones rather than the patch of earth under it.
    fn selected_radius(&self, index: usize) -> f32 {
        let Some(centre) = self.world_position(index) else {
            return 0.0;
        };
        let mut radius: f32 = 0.0;
        for (i, (desc, world)) in self.instanced.scene.flatten().iter().enumerate() {
            if self.instanced.source.get(i).copied().unwrap_or(i) != index {
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
        let index = self.selected?;
        let flat = self.history.scene().flatten();
        let (_, world) = flat.get(index)?;
        Some(world.w_axis.truncate())
    }

    /// The transform an entity's parents impose on it.
    fn parent_matrix(&self, index: usize) -> Mat4 {
        let flat = self.history.scene().flatten();
        let Some((desc, world)) = flat.get(index) else {
            return Mat4::IDENTITY;
        };
        *world * desc.transform.matrix().inverse()
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
