//! The world streamed: regions put into the world as the camera comes near
//! them and taken out as it leaves (`stream: (scene: "oasis", radius:
//! 80.0)` on an entity; the field is [`scrap_core::stream::Stream`]).
//!
//! [`Streamer::update`] each frame, with where the camera is: a region
//! within its radius is loaded — its scene read, its prefabs expanded, its
//! entities spawned with every module's dressers, its models uploaded
//! (from the library, which a game can open lazily with
//! [`crate::Library::open_lazy`] so its bytes are read off disk only now,
//! and give back with [`crate::Library::release`] once they are on the
//! GPU) — and one past its radius
//! and `keep` is taken out: its entities despawned, and each model no
//! other loaded region uses released from the GPU
//! ([`Renderer::release_mesh`]). What a region's entities are is
//! remembered by the region; a streamed scene can hold streams of its own.
//!
//! What it does not do yet: load on another thread (a region comes in on
//! the frame it is asked for, as a scene does), or release a region's
//! pictures (textures are few and small beside meshes, and shared).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use glam::Vec3;
use hecs::World;

use crate::render::MeshHandle;
use crate::world::WorldTransform;
use crate::{Gpu, Library, Prefabs, Renderer, Scene};

/// A `stream` field on its entity.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamPart(pub scrap_core::stream::Stream);

/// On an entity a region put into the world: which stream's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Streamed(pub hecs::Entity);

/// A line's `stream`, on its entity.
pub struct StreamDress;

impl crate::world::Dress for StreamDress {
    fn parts(&self) -> &[&'static str] {
        &["stream"]
    }

    fn dress(
        &mut self,
        line: &crate::scene::EntityDesc,
        entity: hecs::Entity,
        world: &mut World,
        changed: crate::world::Changed,
        _missing: &mut Vec<crate::world::Unresolved>,
    ) {
        if !changed.has("stream") {
            return;
        }
        match line.part::<scrap_core::stream::Stream>() {
            Some(stream) => {
                let _ = world.insert_one(entity, StreamPart(stream));
            }
            None => {
                scrap_core::world::take_off::<StreamPart>(world, entity);
            }
        }
    }
}

/// What a frame's update did.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// A region came in: its scene, and how many entities.
    In(String, usize),
    /// A region went out.
    Out(String),
    /// A region's scene could not be put in: why.
    Failed(String, String),
}

struct Region {
    scene: String,
    entities: Vec<hecs::Entity>,
    models: Vec<String>,
}

/// The regions in the world now, and the models they have uploaded.
#[derive(Default)]
pub struct Streamer {
    regions: HashMap<hecs::Entity, Region>,
    /// Each model uploaded for regions: its handle and how many loaded
    /// regions use it.
    models: HashMap<String, (MeshHandle, usize)>,
}

impl Streamer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Regions in the world now.
    pub fn loaded(&self) -> usize {
        self.regions.len()
    }

    /// Put in what the camera at `eye` has come near, take out what it has
    /// left: scenes found under `scenes` (a project's root: wherever they lie), prefabs from `prefabs`, models from the
    /// builtins and `library`.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        world: &mut World,
        eye: Vec3,
        scenes: &Path,
        prefabs: &Prefabs,
        library: Option<&Library>,
        gpu: &Gpu,
        renderer: &mut Renderer,
    ) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        let streams: Vec<(hecs::Entity, scrap_core::stream::Stream, Vec3)> = world
            .query::<(hecs::Entity, &StreamPart, &WorldTransform)>()
            .iter()
            .map(|(e, s, t)| (e, s.0.clone(), t.0.w_axis.truncate()))
            .collect();
        // Out: past its reach, or its stream gone from the world.
        let live: HashSet<hecs::Entity> = streams.iter().map(|(e, _, _)| *e).collect();
        let leaving: Vec<hecs::Entity> = self
            .regions
            .keys()
            .copied()
            .filter(|owner| {
                !live.contains(owner)
                    || streams
                        .iter()
                        .find(|(e, _, _)| e == owner)
                        .is_some_and(|(_, s, at)| eye.distance(*at) > s.radius + s.keep.max(0.0))
            })
            .collect();
        for owner in leaving {
            if let Some(region) = self.regions.remove(&owner) {
                self.unload(region, world, gpu, renderer, &mut events);
            }
        }
        // In: within its radius and not in yet.
        for (owner, stream, at) in streams {
            if self.regions.contains_key(&owner) || eye.distance(at) > stream.radius {
                continue;
            }
            let path = crate::layout::find(scenes, crate::layout::Kind::Scene, &stream.scene)
                .unwrap_or_else(|| scenes.join(format!("{}.ron", stream.scene)));
            match self.load(owner, &stream.scene, &path, prefabs, library, world, gpu, renderer) {
                Ok(count) => events.push(StreamEvent::In(stream.scene.clone(), count)),
                Err(reason) => {
                    // Marked in, so a missing file is not read again every
                    // frame; it is tried again once the camera has left.
                    self.regions.insert(
                        owner,
                        Region {
                            scene: stream.scene.clone(),
                            entities: Vec::new(),
                            models: Vec::new(),
                        },
                    );
                    events.push(StreamEvent::Failed(stream.scene.clone(), reason));
                }
            }
        }
        events
    }

    #[allow(clippy::too_many_arguments)]
    fn load(
        &mut self,
        owner: hecs::Entity,
        name: &str,
        path: &Path,
        prefabs: &Prefabs,
        library: Option<&Library>,
        world: &mut World,
        gpu: &Gpu,
        renderer: &mut Renderer,
    ) -> Result<usize, String> {
        let document = Scene::load(path).map_err(|e| e.to_string())?;
        let scene = crate::instantiate(&document, prefabs).scene;
        let before: HashSet<hecs::Entity> = world.iter().map(|e| e.entity()).collect();
        let mut used: Vec<String> = Vec::new();
        let models = &mut self.models;
        crate::spawn_scene_with(
            &scene,
            world,
            |link| {
                let name: &str = link;
                if !used.iter().any(|u| u == name) {
                    used.push(name.to_string());
                }
                if let Some((handle, _)) = models.get(name) {
                    return Some(*handle);
                }
                let handle = if let Some(mesh) = crate::builtin::by_name(name) {
                    renderer.upload_mesh_owned(gpu, &mesh)
                } else {
                    let mesh = crate::mesh_asset::MeshLibrary::mesh_by_name(library?, name)?;
                    renderer.upload_mesh(gpu, mesh)
                };
                models.insert(name.to_string(), (handle, 0));
                Some(handle)
            },
            |link| library.and_then(|l| crate::material::MaterialLibrary::material_by_name(l, link)),
        );
        let entities: Vec<hecs::Entity> = world
            .iter()
            .map(|e| e.entity())
            .filter(|e| !before.contains(e))
            .collect();
        for &e in &entities {
            let _ = world.insert_one(e, Streamed(owner));
        }
        // Only the models it resolved: a name missing is nothing to count.
        used.retain(|name| self.models.contains_key(name));
        for name in &used {
            if let Some(entry) = self.models.get_mut(name) {
                entry.1 += 1;
            }
        }
        crate::terrain::upload_terrains(world, gpu, renderer);
        let _ = crate::world::upload_material_maps(world, library, gpu, renderer);
        let count = entities.len();
        self.regions.insert(
            owner,
            Region {
                scene: name.to_string(),
                entities,
                models: used,
            },
        );
        Ok(count)
    }

    fn unload(&mut self, region: Region, world: &mut World, gpu: &Gpu, renderer: &mut Renderer, events: &mut Vec<StreamEvent>) {
        for entity in region.entities {
            let _ = world.despawn(entity);
        }
        for name in region.models {
            let gone = match self.models.get_mut(&name) {
                Some(entry) => {
                    entry.1 = entry.1.saturating_sub(1);
                    entry.1 == 0
                }
                None => false,
            };
            if gone {
                if let Some((handle, _)) = self.models.remove(&name) {
                    renderer.release_mesh(gpu, handle);
                }
            }
        }
        events.push(StreamEvent::Out(region.scene));
    }
}
