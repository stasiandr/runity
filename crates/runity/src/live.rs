//! A scene that keeps up with its files while the game runs.
//!
//! The first postulate asks that scenes and assets reload without a restart.
//! [`LiveScene`] is that, for a game: it owns what a scene needs from its
//! project — prefabs, the built library, meshes uploaded — and on each
//! [`LiveScene::reload`] looks at the files and brings the world up to date.
//!
//! * A changed scene or prefab is read again and [patched](crate::patch_scene)
//!   into the world line by line: only what the file changed is written, so
//!   the game's state survives someone moving a tree.
//! * A changed asset in the library is re-read and re-uploaded, and every
//!   entity drawing the old mesh draws the new one. A new asset — imported
//!   while the game runs — is picked up, and entities that were waiting for
//!   it get it.
//! * A file that does not parse is reported, and the world keeps the last
//!   version that did. Saving half a line should not empty the valley.
//!
//! The library is the importer's output, not the importer: a game does not
//! link the importers (third postulate). Something else — the editor, or
//! `runity sync` — turns a changed `.png` into a changed `.rasset`,
//! and this picks the `.rasset` up.
//!
//! Polling, not a watcher, for the reason the library polls: the game
//! already has a loop, and a watcher is a thread, a channel and debouncing.
//! The caller decides how often; a few times a second is plenty.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use hecs::World;

use crate::asset::AssetKind;
use crate::gpu::Gpu;
use crate::library::{Library, Reloaded};
use crate::render::{MeshHandle, Renderer};
use crate::scene::{MaterialRef, Scene};
use crate::world::{Model, Patched, SceneId, Unresolved};
use crate::{builtin, ComponentProblem, Components, Prefabs, Project};

/// A scene file, its project, and what has been uploaded for it.
pub struct LiveScene {
    path: PathBuf,
    project: Option<Project>,
    library: Option<Library>,
    /// What the world was last brought to: the file with its prefab
    /// instances expanded.
    current: Scene,
    /// The project's prefabs, for the scene and for spawning at run time.
    prefabs: Prefabs,
    stamps: Stamps,
    meshes: HashMap<String, MeshHandle>,
    since_poll: f32,
    components: Components,
}

/// A prefab spawned at run time.
#[derive(Debug)]
pub struct Instance {
    pub root: hecs::Entity,
    /// What could not be resolved: models, components.
    pub problems: Vec<String>,
}

/// What [`LiveScene::spawn`] could not do.
#[derive(Debug, Default)]
pub struct Spawned {
    /// Lines whose model nothing answers to.
    pub missing: Vec<Unresolved>,
    /// Components the game does not register, or whose values do not fit.
    pub components: Vec<ComponentProblem>,
}

impl Spawned {
    /// What went wrong, a line each.
    pub fn lines(&self) -> Vec<String> {
        self.missing
            .iter()
            .map(|m| format!("{}: no model named {}", m.entity_name, m.model))
            .chain(self.components.iter().map(ToString::to_string))
            .collect()
    }
}

/// What one [`LiveScene::reload`] found and did.
#[derive(Debug, Default)]
pub struct Reload {
    /// Set when the scene or a prefab changed and was read again.
    pub patched: Option<Patched>,
    /// Assets re-read or added from the library.
    pub assets: Vec<Reloaded>,
    /// What could not be read, in words. The world keeps the last good
    /// version of whatever this names.
    pub problems: Vec<String>,
}

impl Reload {
    pub fn is_empty(&self) -> bool {
        self.patched.as_ref().is_none_or(Patched::is_empty)
            && self.assets.is_empty()
            && self.problems.is_empty()
    }

    /// What happened, a line each, for a terminal or a log.
    pub fn lines(&self) -> Vec<String> {
        let mut out = self.problems.clone();
        if let Some(patched) = self.patched.as_ref().filter(|p| !p.is_empty()) {
            out.push(format!(
                "scene: {} spawned, {} changed, {} removed",
                patched.spawned, patched.updated, patched.despawned
            ));
            out.extend(
                patched
                    .missing
                    .iter()
                    .map(|m| format!("{}: no model named {}", m.entity_name, m.model)),
            );
        }
        out.extend(
            self.assets
                .iter()
                .map(|asset| format!("reloaded {}", asset.path.display())),
        );
        out
    }
}

/// How often [`LiveScene::poll`] looks at the files: as fast as anyone
/// saves one, and slow enough that the `stat`s cost nothing.
pub const POLL_SECONDS: f32 = 0.25;

impl LiveScene {
    /// Read a scene, with the prefabs and library of the project it is in.
    ///
    /// A scene outside any project opens too, with builtins only. What could
    /// not be read on the way — a prefab that does not parse, an instance of
    /// one that does not exist — comes back as the second value.
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<(Self, Vec<String>)> {
        let path = path.as_ref().to_path_buf();
        let project = Project::find(&path).ok();
        let (current, prefabs, mut problems) = read(&path, project.as_ref())?;
        let library = match project.as_ref().map(Project::library) {
            Some(directory) if directory.is_dir() => {
                let (library, skipped) = Library::open(&directory)?;
                problems.extend(
                    skipped
                        .into_iter()
                        .map(|(path, e)| format!("{}: {e}", path.display())),
                );
                Some(library)
            }
            _ => None,
        };
        let stamps = stamps(&path, project.as_ref());
        Ok((
            Self {
                path,
                project,
                library,
                current,
                prefabs,
                stamps,
                meshes: HashMap::new(),
                since_poll: 0.0,
                components: Components::new(),
            },
            problems,
        ))
    }

    /// The scene as the world has it, prefabs expanded.
    pub fn scene(&self) -> &Scene {
        &self.current
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }

    pub fn library(&self) -> Option<&Library> {
        self.library.as_ref()
    }

    /// The game's component types, which scene lines name. Set before
    /// [`LiveScene::spawn`].
    pub fn with_components(mut self, components: Components) -> Self {
        self.components = components;
        self
    }

    /// Put the scene into a world, uploading the meshes it names and
    /// giving each entity the game's components its line carries.
    pub fn spawn(&mut self, world: &mut World, gpu: &Gpu, renderer: &mut Renderer) -> Spawned {
        let library = self.library.as_ref();
        let missing = crate::spawn_scene_with(
            &self.current,
            world,
            resolver(&mut self.meshes, library, gpu, renderer),
            |name| library?.material_by_name(name),
        );
        let components = self.components.apply(&self.current, world);
        #[cfg(feature = "physics")]
        crate::physics::attach_scene_collision_meshes(world, &self.current, self.library.as_ref());
        Spawned {
            missing,
            components,
        }
    }

    /// Start the world again under new code, keeping what a save keeps —
    /// what a hot patch calls, Unity's domain reload.
    ///
    /// A patch can change a component's fields, and the world still holds
    /// values laid out the old way: new code reading them reads garbage
    /// (subsecond leaves this to the framework, and says so). So the world
    /// is written down with the component types it was built with (the
    /// current [`Components`] still point at the old code), dropped — not
    /// cleared: hecs remembers each type's old layout in its archetypes —
    /// spawned afresh from the scene with `components`, registered by the
    /// new code, and the save put back: every entity's transform and the
    /// saved components, by name, as text into the new types, a new field
    /// taking its `#[serde(default)]`. What a save does not keep starts
    /// from the scene, as a load does.
    pub fn reinstance(
        &mut self,
        world: &mut World,
        components: Components,
        gpu: &Gpu,
        renderer: &mut Renderer,
    ) -> crate::save::Restored {
        let save = crate::save::capture(world, &self.components, &self.current);
        *world = World::new();
        self.components = components;
        let spawned = self.spawn(world, gpu, renderer);
        let components = self.components.clone();
        let mut restored = crate::save::restore(world, &components, &save, |world, prefab, at| {
            self.spawn_prefab(prefab, at, None, world, gpu, renderer)
                .ok()
                .map(|instance| instance.root)
        });
        restored.problems.extend(spawned.lines());
        restored
    }

    /// Spawn a prefab at run time — Unity's `Instantiate` — at `transform`,
    /// optionally under `parent`. Returns its root entity.
    ///
    /// The entities are the game's, not the file's: they carry no
    /// [`crate::SceneId`], so a reload of the scene neither patches nor
    /// removes them. The prefab's parts, its models and the game's
    /// components on it are all there; what could not be resolved comes back
    /// with the root, in words.
    pub fn spawn_prefab(
        &mut self,
        name: &str,
        transform: crate::Transform,
        parent: Option<hecs::Entity>,
        world: &mut World,
        gpu: &Gpu,
        renderer: &mut Renderer,
    ) -> Result<Instance, String> {
        if self.prefabs.get(name).is_none() {
            let names = self.prefabs.names();
            let hint = crate::spelling::closest(name, names.iter().copied())
                .map(|n| format!(" — did you mean `{n}`?"))
                .unwrap_or_default();
            return Err(format!("no prefab named `{name}` in prefabs/{hint}"));
        }
        let instance = crate::EntityDesc {
            id: crate::EntityId::fresh(),
            name: name.to_string(),
            prefab: name.to_string(),
            transform,
            ..crate::EntityDesc::default()
        };
        let expanded = crate::instantiate(
            &Scene {
                entities: vec![instance],
                ..Scene::default()
            },
            &self.prefabs,
        );
        let library = self.library.as_ref();
        let (spawned, missing) = crate::world::spawn_owned(
            &expanded.scene.entities[0],
            parent,
            world,
            resolver(&mut self.meshes, library, gpu, renderer),
            |name| library?.material_by_name(name),
        );
        let mut problems: Vec<String> = missing
            .iter()
            .map(|m| format!("{}: no model named {}", m.entity_name, m.model))
            .collect();
        #[cfg(feature = "physics")]
        crate::physics::attach_collision_meshes(world, spawned.iter().copied(), library);
        for (entity, desc) in &spawned {
            problems.extend(
                self.components
                    .insert_all(desc, *entity, world)
                    .iter()
                    .map(ToString::to_string),
            );
        }
        Ok(Instance {
            root: spawned[0].0,
            problems,
        })
    }

    /// Take this scene out of a world: every entity its lines spawned, and
    /// whatever hangs off them. Other scenes in the world, and what the game
    /// spawned on its own, stay. With [`LiveScene::open`] and
    /// [`LiveScene::spawn`] it is how a game changes level, or streams one
    /// area out while another comes in: each scene its own `LiveScene`,
    /// side by side in one world, each reloading only its own lines.
    pub fn unload(&self, world: &mut World) -> usize {
        crate::patch_scene(&self.current, &Scene::default(), world, |_| None, |_| None).despawned
    }

    /// [`LiveScene::reload`], at most every [`POLL_SECONDS`]: call it every
    /// frame with the frame's delta.
    pub fn poll(
        &mut self,
        delta: f32,
        world: &mut World,
        gpu: &Gpu,
        renderer: &mut Renderer,
    ) -> Reload {
        self.since_poll += delta;
        if self.since_poll < POLL_SECONDS {
            return Reload::default();
        }
        self.since_poll = 0.0;
        self.reload(world, gpu, renderer)
    }

    /// Look at the files and bring the world up to date with them.
    pub fn reload(&mut self, world: &mut World, gpu: &Gpu, renderer: &mut Renderer) -> Reload {
        let mut out = Reload::default();
        self.reload_library(world, gpu, renderer, &mut out);

        let now = stamps(&self.path, self.project.as_ref());
        if now != self.stamps {
            self.stamps = now;
            match read(&self.path, self.project.as_ref()) {
                Ok((scene, prefabs, problems)) => {
                    self.prefabs = prefabs;
                    let library = self.library.as_ref();
                    out.patched = Some(crate::patch_scene(
                        &self.current,
                        &scene,
                        world,
                        resolver(&mut self.meshes, library, gpu, renderer),
                        |name| library?.material_by_name(name),
                    ));
                    out.problems.extend(problems);
                    out.problems.extend(
                        self.components
                            .patch(&self.current, &scene, world)
                            .iter()
                            .map(ToString::to_string),
                    );
                    self.current = scene;
                    #[cfg(feature = "physics")]
                    crate::physics::attach_scene_collision_meshes(
                        world,
                        &self.current,
                        self.library.as_ref(),
                    );
                }
                Err(e) => out.problems.push(format!("{e:#}")),
            }
        }
        out
    }

    fn reload_library(
        &mut self,
        world: &mut World,
        gpu: &Gpu,
        renderer: &mut Renderer,
        out: &mut Reload,
    ) {
        let Some(directory) = self.project.as_ref().map(Project::library) else {
            return;
        };
        let library = self.library.get_or_insert_with(Library::new);
        out.assets.extend(library.reload_changed());
        out.assets.extend(library.add_new(&directory));
        if out.assets.is_empty() {
            return;
        }

        // Every name whose meaning just changed.
        let touched: HashSet<String> = out
            .assets
            .iter()
            .filter_map(|asset| library.name(asset.id).map(str::to_string))
            .collect();

        // A mesh already on the GPU is uploaded again, and whoever drew the
        // old one draws the new one. Handles are not reused: the old buffer
        // stays until the renderer goes, which is a development build's
        // trade, not a shipped game's.
        let mut swapped: HashMap<MeshHandle, MeshHandle> = HashMap::new();
        for asset in out.assets.iter().filter(|a| a.kind == AssetKind::Mesh) {
            let Some(name) = library.name(asset.id) else {
                continue;
            };
            if builtin::by_name(name).is_some() {
                continue;
            }
            let (Some(old), Some(mesh)) = (self.meshes.get(name).copied(), library.mesh(asset.id))
            else {
                continue;
            };
            let new = renderer.upload_mesh(gpu, mesh);
            self.meshes.insert(name.to_string(), new);
            swapped.insert(old, new);
        }
        for model in world.query_mut::<&mut Model>() {
            if let Some(new) = swapped.get(&model.0) {
                model.0 = *new;
            }
        }

        // A material that changed, or a mesh that was missing and now is
        // not: the entities naming it are dressed again. Only those — an
        // entity the game recoloured keeps its colour unless its own
        // material is the one that changed.
        let library = self.library.as_ref();
        let wanting: Vec<(hecs::Entity, crate::EntityDesc)> = world
            .query::<(hecs::Entity, &SceneId, Option<&Model>)>()
            .iter()
            .filter_map(|(entity, id, model)| {
                let desc = self.current.get(id.0)?;
                let material =
                    matches!(&desc.material, MaterialRef::Named(n) if touched.contains(n));
                let arrived = model.is_none() && touched.contains(&desc.model);
                (material || arrived).then(|| (entity, desc.clone()))
            })
            .collect();
        let mut resolve = resolver(&mut self.meshes, library, gpu, renderer);
        let mut ignored = Vec::new();
        for (entity, desc) in wanting {
            crate::world::dress(
                &desc,
                entity,
                world,
                &mut resolve,
                &|name| library?.material_by_name(name),
                &mut ignored,
            );
        }
        // A model that changed shape changes what it collides as.
        #[cfg(feature = "physics")]
        crate::physics::attach_scene_collision_meshes(world, &self.current, library);
    }
}

/// Read a scene and expand its prefab instances.
fn read(path: &Path, project: Option<&Project>) -> anyhow::Result<(Scene, Prefabs, Vec<String>)> {
    let document = Scene::load(path)?;
    let (prefabs, skipped) = project.map(Prefabs::of).unwrap_or_default();
    let mut problems: Vec<String> = skipped
        .into_iter()
        .map(|(path, e)| format!("skipped {}: {e}", path.display()))
        .collect();
    let instanced = crate::instantiate(&document, &prefabs);
    problems.extend(
        instanced
            .problems
            .iter()
            .map(|p| format!("{}: prefab {} — {}", p.entity_name, p.prefab, p.reason)),
    );
    Ok((instanced.scene, prefabs, problems))
}

/// When each file a scene is read from last changed.
pub type Stamps = Vec<(PathBuf, Option<SystemTime>)>;

/// When each file the scene is read from last changed: the scene and every
/// prefab. A prefab added or removed changes the list, which counts too.
///
/// Compare two of these to know whether a scene needs reading again.
pub fn stamps(path: &Path, project: Option<&Project>) -> Stamps {
    let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut out = vec![(path.to_path_buf(), modified(path))];
    if let Some(entries) = project.and_then(|p| std::fs::read_dir(p.prefabs()).ok()) {
        let mut prefabs: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("prefab"))
            .collect();
        prefabs.sort();
        out.extend(prefabs.into_iter().map(|path| {
            let stamp = modified(&path);
            (path, stamp)
        }));
    }
    out
}

/// Turn a model name into an uploaded mesh: a builtin first, then the
/// library, each uploaded once.
fn resolver<'a>(
    meshes: &'a mut HashMap<String, MeshHandle>,
    library: Option<&'a Library>,
    gpu: &'a Gpu,
    renderer: &'a mut Renderer,
) -> impl FnMut(&str) -> Option<MeshHandle> + 'a {
    move |name| {
        if let Some(handle) = meshes.get(name) {
            return Some(*handle);
        }
        let handle = match builtin::by_name(name) {
            Some(mesh) => renderer.upload_mesh_owned(gpu, &mesh),
            None => renderer.upload_mesh(gpu, library?.mesh_by_name(name)?),
        };
        meshes.insert(name.to_string(), handle);
        Some(handle)
    }
}
