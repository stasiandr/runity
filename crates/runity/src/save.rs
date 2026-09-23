//! Saving a game in progress, and loading it back.
//!
//! A world starts from its scene, so a save is what differs from the scene:
//! for every entity the scene has — by the stable id the file gives it —
//! where it is now and the components the game marked as saved
//! ([`Components::register_saved`]); which of the scene's entities are
//! gone; and what was spawned at run time with an id to be known by (see
//! [`crate::net::announce`]). Loading is opening the same scene and putting
//! that back. The file is RON: a save someone can read to see what went
//! wrong, and diff against the one before.
//!
//! What is not marked saved starts from the scene again, which is the
//! useful default: a door's `locked` flag belongs in a save, its hinge's
//! stiffness does not, and a tuning change should reach old saves.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::components::Components;
use crate::id::EntityId;
use crate::net::{addressable, despawn_tree, NetId, NetPrefab};
use crate::scene::{Scene, Transform};
use crate::world::SceneId;

/// One entity, as saved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub id: EntityId,
    pub transform: Transform,
    /// Saved components by name, as RON.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<(String, String)>,
    /// For an entity spawned at run time: the prefab it came from, to be
    /// spawned again on load. Empty for the scene's own.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prefab: String,
    /// The state its [`crate::animgraph::Controller`] is in, if it has one:
    /// what the editor's Animator lights up while the game runs. A load
    /// does not put it back — the graph finds its state again from the
    /// parameters.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub animator: String,
}

/// A game in progress.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SaveGame {
    pub entities: Vec<Saved>,
    /// The scene's entities that are no longer there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gone: Vec<EntityId>,
}

/// What a load did, and what it could not.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Restored {
    pub restored: usize,
    pub spawned: usize,
    pub removed: usize,
    /// In words: a saved entity the scene no longer has, a component not
    /// registered any more, a prefab that is gone.
    pub problems: Vec<String>,
}

/// Write down a world that started from `scene`.
pub fn capture(world: &hecs::World, components: &Components, scene: &Scene) -> SaveGame {
    let mut entities: Vec<Saved> = Vec::new();
    for (entity, id, transform) in world.query::<(hecs::Entity, &SceneId, &Transform)>().iter() {
        entities.push(Saved {
            id: id.0,
            transform: *transform,
            components: components.write_saved(world, entity),
            prefab: String::new(),
            animator: animator_state(world, entity),
        });
    }
    for (entity, id, prefab, transform) in world
        .query::<(hecs::Entity, &NetId, &NetPrefab, &Transform)>()
        .iter()
    {
        entities.push(Saved {
            id: id.0,
            transform: *transform,
            components: components.write_saved(world, entity),
            prefab: prefab.0.clone(),
            animator: animator_state(world, entity),
        });
    }
    entities.sort_by_key(|s| s.id);
    let here = addressable(world);
    let mut gone: Vec<EntityId> = scene
        .flatten()
        .iter()
        .map(|(desc, _)| desc.id)
        .filter(|id| !here.contains_key(id))
        .collect();
    gone.sort();
    SaveGame { entities, gone }
}

fn animator_state(world: &hecs::World, entity: hecs::Entity) -> String {
    world
        .get::<&crate::animgraph::Controller>(entity)
        .ok()
        .and_then(|c| c.state().map(str::to_string))
        .unwrap_or_default()
}

/// Put a save back into a world freshly spawned from the same scene.
/// `spawn` puts a prefab into the world at a transform — usually
/// [`crate::LiveScene::spawn_prefab`] — for what was spawned at run time.
pub fn restore(
    world: &mut hecs::World,
    components: &Components,
    save: &SaveGame,
    mut spawn: impl FnMut(&mut hecs::World, &str, Transform) -> Option<hecs::Entity>,
) -> Restored {
    let mut out = Restored::default();
    let by_id = addressable(world);
    for id in &save.gone {
        if let Some(&entity) = by_id.get(id) {
            despawn_tree(world, entity);
            out.removed += 1;
        }
    }
    for saved in &save.entities {
        let entity = match by_id.get(&saved.id) {
            Some(&entity) if world.contains(entity) => entity,
            _ if !saved.prefab.is_empty() => match spawn(world, &saved.prefab, saved.transform) {
                Some(entity) => {
                    let _ =
                        world.insert(entity, (NetId(saved.id), NetPrefab(saved.prefab.clone())));
                    out.spawned += 1;
                    entity
                }
                None => {
                    out.problems.push(format!(
                        "{}: no prefab `{}` to spawn it from",
                        saved.id, saved.prefab
                    ));
                    continue;
                }
            },
            _ => {
                out.problems.push(format!(
                    "{}: the scene has no such entity any more",
                    saved.id
                ));
                continue;
            }
        };
        let _ = world.insert_one(entity, saved.transform);
        for (name, text) in &saved.components {
            if let Err(e) = components.insert_text(name, text, world, entity) {
                out.problems.push(format!("{}: `{name}`: {e}", saved.id));
            }
        }
        out.restored += 1;
    }
    crate::world::apply_hierarchy(world);
    out
}

impl SaveGame {
    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        let pretty = ron::ser::PrettyConfig::new().depth_limit(3);
        let text = ron::ser::to_string_pretty(self, pretty).map_err(|e| e.to_string())? + "\n";
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        // Whole or not at all: a game that stops halfway through a save
        // keeps the one before, and a reader never sees half a file.
        let part = path.with_extension("part");
        std::fs::write(&part, text).map_err(|e| format!("{}: {e}", part.display()))?;
        std::fs::rename(&part, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn read(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}:{e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::MeshHandle;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Locked(bool);

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Stiffness(f32);

    const KEEP: &str = r#"(entities: [
        (id: "a1", name: "door", model: "m", components: { "locked": (true), "stiffness": (3.0) }),
        (id: "b2", name: "crate", model: "m"),
        (id: "c3", name: "barrel", model: "m"),
    ])"#;

    fn components() -> Components {
        let mut c = Components::new();
        c.register_saved::<Locked>("locked")
            .register::<Stiffness>("stiffness");
        c
    }

    fn start() -> (hecs::World, Scene) {
        let mut scene: Scene = ron::from_str(KEEP).unwrap();
        scene.assign_ids();
        let mut world = hecs::World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        components().apply(&scene, &mut world);
        (world, scene)
    }

    fn entity(world: &hecs::World, id: &str) -> Option<hecs::Entity> {
        addressable(world).get(&id.parse().unwrap()).copied()
    }

    #[test]
    fn a_saved_game_loads_back_into_a_fresh_world_from_the_same_scene() {
        let components = components();
        let (mut world, scene) = start();
        // Play: unlock the door, push the crate, smash the barrel, drop a
        // torch, and change something that is not saved.
        let door = entity(&world, "a1").unwrap();
        world.get::<&mut Locked>(door).unwrap().0 = false;
        world.get::<&mut Stiffness>(door).unwrap().0 = 99.0;
        let crate_ = entity(&world, "b2").unwrap();
        world.get::<&mut Transform>(crate_).unwrap().position.x = 4.0;
        let barrel = entity(&world, "c3").unwrap();
        despawn_tree(&mut world, barrel);
        let torch = world.spawn((Transform {
            position: glam::Vec3::new(1.0, 2.0, 3.0),
            ..Transform::default()
        },));
        crate::net::announce(&mut world, torch, crate::net::PeerId::HOST, "torch");

        let path = std::env::temp_dir().join("runity-save/slot1.ron");
        capture(&world, &components, &scene).write(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("\"locked\"") && !text.contains("stiffness"),
            "{text}"
        );

        let (mut loaded, _) = start();
        let save = SaveGame::read(&path).unwrap();
        let done = restore(&mut loaded, &components, &save, |w, prefab, t| {
            (prefab == "torch").then(|| w.spawn((t,)))
        });
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert_eq!((done.spawned, done.removed), (1, 1));
        let door = entity(&loaded, "a1").unwrap();
        assert_eq!(*loaded.get::<&Locked>(door).unwrap(), Locked(false));
        assert_eq!(
            *loaded.get::<&Stiffness>(door).unwrap(),
            Stiffness(3.0),
            "not saved: from the scene"
        );
        let crate_ = entity(&loaded, "b2").unwrap();
        assert_eq!(loaded.get::<&Transform>(crate_).unwrap().position.x, 4.0);
        assert!(entity(&loaded, "c3").is_none(), "the barrel stays smashed");
        let torch = loaded
            .query::<(&NetPrefab, &Transform)>()
            .iter()
            .map(|(_, t)| t.position)
            .next()
            .expect("the torch is back");
        assert_eq!(torch, glam::Vec3::new(1.0, 2.0, 3.0));
        // And saving the loaded world says the same.
        assert_eq!(capture(&loaded, &components, &scene), save);
    }
}
