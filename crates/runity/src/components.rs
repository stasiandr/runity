//! The game's components in scene files.
//!
//! A scene line carries `components: { "door": (open_angle: 90.0) }`, and
//! the game says what `"door"` is:
//!
//! ```
//! # use serde::Deserialize;
//! #[derive(Deserialize)]
//! struct Door { open_angle: f32 }
//!
//! let mut components = runity::Components::new();
//! components.register::<Door>("door");
//! ```
//!
//! After the scene spawns, [`Components::apply`] reads each value into its
//! type and inserts it on the entity spawned from that line. That is the
//! whole mechanism: no base class, no reflection, no attributes — a
//! component is a plain struct, a line is a set of them, and the systems
//! that use them are ordinary functions over the world. What Unity does
//! with a MonoBehaviour's serialized fields, an entity store does with a
//! map from name to value.
//!
//! The name is given, not taken from the type, on purpose: renaming a Rust
//! type must not orphan every scene that uses it.
//!
//! On a live reload, [`Components::patch`] writes only the components whose
//! text changed between the two versions of the file, and removes the ones
//! the file stopped naming — the same rule as the engine's own fields, so a
//! door the game swung open stays open until someone edits the door.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use hecs::World;
use ron::value::RawValue;
use serde::de::DeserializeOwned;

use crate::id::EntityId;
use crate::scene::{EntityDesc, Scene};
use crate::world::SceneId;

type Insert = fn(&RawValue, &mut World, hecs::Entity) -> Result<(), String>;
type Remove = fn(&mut World, hecs::Entity);
/// A component's value on an entity, as RON — for the ones that go over the
/// network.
type Write = fn(&World, hecs::Entity) -> Option<String>;

/// The component types a game has, by the names scenes use for them.
#[derive(Default, Clone)]
pub struct Components {
    by_name: BTreeMap<String, (Insert, Remove)>,
    /// The ones marked networked, and how to write each out.
    networked: BTreeMap<String, Write>,
}

/// A component a scene names that could not be put on its entity.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentProblem {
    pub entity: EntityId,
    pub entity_name: String,
    pub component: String,
    pub reason: String,
}

impl fmt::Display for ComponentProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` ({}): component `{}` — {}",
            self.entity_name, self.entity, self.component, self.reason
        )
    }
}

fn insert<T: hecs::Component + DeserializeOwned>(
    value: &RawValue,
    world: &mut World,
    entity: hecs::Entity,
) -> Result<(), String> {
    let value: T = value.into_rust().map_err(|e| e.to_string())?;
    world
        .insert_one(entity, value)
        .map_err(|_| "the entity is gone".to_string())
}

fn write<T: hecs::Component + serde::Serialize>(
    world: &World,
    entity: hecs::Entity,
) -> Option<String> {
    let value = world.get::<&T>(entity).ok()?;
    ron::to_string(&*value).ok()
}

fn remove<T: hecs::Component>(world: &mut World, entity: hecs::Entity) {
    let _ = world.remove_one::<T>(entity);
}

impl Components {
    pub fn new() -> Self {
        Self::default()
    }

    /// Say that `name` in a scene means `T`.
    pub fn register<T: hecs::Component + DeserializeOwned>(&mut self, name: &str) -> &mut Self {
        self.by_name
            .insert(name.to_string(), (insert::<T>, remove::<T>));
        self
    }

    /// Say that `name` means `T`, and that it is networked: its value on
    /// an entity goes to the other peers in a [`crate::net::Snapshot`]
    /// from whoever owns the entity. The mark is the design decision — a
    /// component is local unless it says otherwise — made while there are
    /// still few components to make it for (DNA, postulate 4).
    pub fn register_networked<T>(&mut self, name: &str) -> &mut Self
    where
        T: hecs::Component + DeserializeOwned + serde::Serialize,
    {
        self.register::<T>(name);
        self.networked.insert(name.to_string(), write::<T>);
        self
    }

    pub fn is_networked(&self, name: &str) -> bool {
        self.networked.contains_key(name)
    }

    /// Every networked component on an entity, as `(name, RON)`.
    pub(crate) fn write_networked(
        &self,
        world: &World,
        entity: hecs::Entity,
    ) -> Vec<(String, String)> {
        self.networked
            .iter()
            .filter_map(|(name, write)| write(world, entity).map(|text| (name.clone(), text)))
            .collect()
    }

    /// Put one component's RON onto an entity, by name.
    pub(crate) fn insert_text(
        &self,
        name: &str,
        text: &str,
        world: &mut World,
        entity: hecs::Entity,
    ) -> Result<(), String> {
        let (insert, _) = self
            .by_name
            .get(name)
            .ok_or_else(|| format!("no component `{name}` registered"))?;
        let value = RawValue::from_ron(text).map_err(|e| e.to_string())?;
        insert(value, world, entity)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Put the components one line carries on one entity: for entities the
    /// game spawns itself, which no scene reload will patch.
    pub fn insert_all(
        &self,
        desc: &EntityDesc,
        entity: hecs::Entity,
        world: &mut World,
    ) -> Vec<ComponentProblem> {
        let mut problems = Vec::new();
        for (name, value) in &desc.components {
            self.insert_one(desc, name, value, entity, world, &mut problems);
        }
        problems
    }

    fn insert_one(
        &self,
        desc: &EntityDesc,
        name: &str,
        value: &RawValue,
        entity: hecs::Entity,
        world: &mut World,
        problems: &mut Vec<ComponentProblem>,
    ) {
        let problem = |reason: String| ComponentProblem {
            entity: desc.id,
            entity_name: desc.name.clone(),
            component: name.to_string(),
            reason,
        };
        match self.by_name.get(name) {
            Some((insert, _)) => {
                if let Err(e) = insert(value, world, entity) {
                    problems.push(problem(format!("{e}; the entity keeps what it had")));
                }
            }
            None => {
                let hint = crate::spelling::closest(name, self.names())
                    .map(|n| format!(" — did you mean `{n}`?"))
                    .unwrap_or_default();
                problems.push(problem(format!(
                    "the game registers no component by this name{hint}"
                )));
            }
        }
    }

    /// Put every component of every line of `scene` on the entity spawned
    /// from it. Call it after [`crate::spawn_scene_with`] on the same scene.
    pub fn apply(&self, scene: &Scene, world: &mut World) -> Vec<ComponentProblem> {
        self.patch(&Scene::default(), scene, world)
    }

    /// Bring components from one version of a scene to the next: write the
    /// ones whose text changed, remove the ones no longer named, leave the
    /// rest — and whatever the game did to them — alone. Call it after
    /// [`crate::patch_scene`] on the same two versions.
    pub fn patch(&self, before: &Scene, after: &Scene, world: &mut World) -> Vec<ComponentProblem> {
        let mut old: HashMap<EntityId, &EntityDesc> = HashMap::new();
        for (desc, _) in before.flatten() {
            old.insert(desc.id, desc);
        }
        let live: HashMap<EntityId, hecs::Entity> = world
            .query::<(hecs::Entity, &SceneId)>()
            .iter()
            .map(|(entity, id)| (id.0, entity))
            .collect();

        let mut problems = Vec::new();
        for (desc, _) in after.flatten() {
            let Some(&entity) = live.get(&desc.id) else {
                continue;
            };
            let was = old.get(&desc.id).map(|d| &d.components);
            for (name, value) in &desc.components {
                if was.and_then(|w| w.get(name)) == Some(value) {
                    continue;
                }
                self.insert_one(desc, name, value, entity, world, &mut problems);
            }
            if let Some(was) = was {
                for name in was.keys().filter(|n| !desc.components.contains_key(*n)) {
                    if let Some((_, remove)) = self.by_name.get(name) {
                        remove(world, entity);
                    }
                }
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::MeshHandle;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Door {
        open_angle: f32,
        #[serde(default)]
        locked: bool,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Loot(String);

    fn components() -> Components {
        let mut components = Components::new();
        components.register::<Door>("door").register::<Loot>("loot");
        components
    }

    fn scene(text: &str) -> Scene {
        let mut scene: Scene = ron::from_str(text).unwrap();
        scene.assign_ids();
        scene
    }

    fn spawned(scene: &Scene) -> (World, hecs::Entity) {
        let mut world = World::new();
        crate::spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        let entity = world.query::<hecs::Entity>().iter().next().unwrap();
        (world, entity)
    }

    const GATE: &str = r#"(entities: [(id: "a1", name: "gate", model: "m",
        components: { "door": (open_angle: 90.0), "loot": ("chest") })])"#;

    #[test]
    fn a_scene_line_brings_the_games_components() {
        let gate = scene(GATE);
        let (mut world, entity) = spawned(&gate);
        assert!(components().apply(&gate, &mut world).is_empty());
        assert_eq!(
            *world.get::<&Door>(entity).unwrap(),
            Door {
                open_angle: 90.0,
                locked: false
            }
        );
        assert_eq!(world.get::<&Loot>(entity).unwrap().0, "chest");
    }

    #[test]
    fn a_reload_writes_changed_components_and_keeps_the_rest() {
        let before = scene(GATE);
        let (mut world, entity) = spawned(&before);
        let components = components();
        components.apply(&before, &mut world);
        // The game swings the door; someone edits the loot table.
        world.get::<&mut Door>(entity).unwrap().open_angle = 12.0;
        let after = scene(&GATE.replace("(\"chest\")", "(\"barrel\")"));
        assert!(components.patch(&before, &after, &mut world).is_empty());
        assert_eq!(world.get::<&Door>(entity).unwrap().open_angle, 12.0);
        assert_eq!(world.get::<&Loot>(entity).unwrap().0, "barrel");

        // And a component taken out of the file is taken off the entity.
        let bare = scene(
            &GATE
                .replace(r#", "loot": ("barrel")"#, "")
                .replace(r#", "loot": ("chest")"#, ""),
        );
        components.patch(&after, &bare, &mut world);
        assert!(world.get::<&Loot>(entity).is_err());
        assert!(world.get::<&Door>(entity).is_ok());
    }

    #[test]
    fn a_name_nobody_registered_and_a_value_that_does_not_fit_are_said() {
        let odd = scene(
            r#"(entities: [(id: "a1", name: "gate", model: "m",
                components: { "dor": (open_angle: 1.0), "loot": (42) })])"#,
        );
        let (mut world, _) = spawned(&odd);
        let problems = components().apply(&odd, &mut world);
        assert_eq!(problems.len(), 2, "{problems:?}");
        let text: Vec<String> = problems.iter().map(ToString::to_string).collect();
        assert!(text[0].contains("did you mean `door`?"), "{text:?}");
        assert!(
            text[1].starts_with("`gate` (00000000000000a1): component `loot`"),
            "{text:?}"
        );
    }

    #[test]
    fn component_text_survives_a_save_byte_for_byte() {
        let text = "(\n    entities: [\n        (\n            id: \"00000000000000a1\",\n            name: \"gate\",\n            model: \"m\",\n            components: {\n                \"door\": (open_angle: 90.0,   locked: true),\n            },\n        ),\n    ],\n)\n";
        let dir = std::env::temp_dir().join("runity-components-save");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("scene.ron");
        std::fs::write(&path, text).unwrap();
        let loaded = Scene::load(&path).unwrap();
        assert_eq!(
            loaded.entities[0].components["door"].get_ron(),
            "(open_angle: 90.0,   locked: true)",
            "as written, spacing and all"
        );
        loaded.save(&path).unwrap();
        let once = std::fs::read_to_string(&path).unwrap();
        Scene::load(&path).unwrap().save(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), once, "stable");
        assert!(
            once.contains("(open_angle: 90.0,   locked: true)"),
            "{once}"
        );
    }
}
