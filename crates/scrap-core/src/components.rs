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
//! let mut components = scrap_core::Components::new();
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
/// A simulation's state on an entity, as the bytes it goes over the
/// network in; `None` when it has nothing to say (not simulated here, or
/// not in a mode that sends it).
pub type GatherState = fn(&World, hecs::Entity) -> Option<Vec<u8>>;
/// The owner's state put onto a replica: the sender's peer number and its
/// tick, and the bytes.
pub type TakeState = fn(&mut World, hecs::Entity, u32, u64, &[u8]);

/// The component types a game has, by the names scenes use for them.
#[derive(Default, Clone)]
pub struct Components {
    by_name: BTreeMap<String, (Insert, Remove)>,
    /// The ones marked networked, and how to write each out.
    networked: BTreeMap<String, Write>,
    /// The ones a save game keeps, and how to write each out.
    saved: BTreeMap<String, Write>,
    /// What each looks like, read off its `Deserialize`.
    shapes: BTreeMap<String, fn() -> crate::shape::Shape>,
    /// Simulations' states that go over the network as their own bytes
    /// rather than a component's RON (docs/netsim.md): a rope's particles.
    states: BTreeMap<String, (GatherState, TakeState)>,
    /// [`Components::networked_names`], kept: what a blob's number is the
    /// place in.
    order: Vec<String>,
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
        self.shapes.insert(
            name.to_string(),
            crate::shape::of::<T> as fn() -> crate::shape::Shape,
        );
        self
    }

    /// What each component looks like, by name: its fields and what they
    /// hold — for an editor that does not link the game.
    pub fn shapes(&self) -> BTreeMap<String, crate::shape::Shape> {
        self.shapes
            .iter()
            .map(|(name, shape)| (name.clone(), shape()))
            .collect()
    }

    /// Write [`Components::shapes`] where the editor and `scrap check`
    /// read them: `library/components.ron` in a project
    /// ([`crate::project::SHAPES`]). Built from the game's code, like the
    /// rest of `library/`, so it is not committed.
    pub fn write_shapes(&self, path: impl AsRef<std::path::Path>) -> Result<(), String> {
        let path = path.as_ref();
        let text = ron::ser::to_string_pretty(&self.shapes(), ron::ser::PrettyConfig::new())
            .map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Say that `name` means `T`, and that it is networked: its value on
    /// an entity goes to the other peers in a snapshot
    /// from whoever owns the entity. The mark is the design decision — a
    /// component is local unless it says otherwise — made while there are
    /// still few components to make it for (DNA, postulate 4).
    pub fn register_networked<T>(&mut self, name: &str) -> &mut Self
    where
        T: hecs::Component + DeserializeOwned + serde::Serialize,
    {
        self.register::<T>(name);
        self.networked.insert(name.to_string(), write::<T>);
        self.reorder();
        self
    }

    fn reorder(&mut self) {
        self.order = self.networked_names().map(str::to_string).collect();
    }

    /// A networked component's or state's number on the wire: one past its
    /// place among the names sorted (nought is the transform's).
    pub fn networked_id(&self, name: &str) -> Option<u16> {
        self.order.binary_search_by(|n| n.as_str().cmp(name)).ok().map(|i| i as u16 + 1)
    }

    /// The name a number on the wire stands for.
    pub fn networked_name(&self, id: u16) -> Option<&str> {
        self.order.get((id as usize).checked_sub(1)?).map(String::as_str)
    }

    /// Say that `name` means `T`, and that a save game keeps it: its value
    /// on an entity is written by [`crate::save::capture`] and put back by
    /// [`crate::save::restore`]. What is not marked starts from the scene
    /// again on load — a component is scenery unless it says otherwise.
    pub fn register_saved<T>(&mut self, name: &str) -> &mut Self
    where
        T: hecs::Component + DeserializeOwned + serde::Serialize,
    {
        self.register::<T>(name);
        self.saved.insert(name.to_string(), write::<T>);
        self
    }

    pub fn is_saved(&self, name: &str) -> bool {
        self.saved.contains_key(name)
    }

    /// Every saved component on an entity, as `(name, RON)`.
    pub(crate) fn write_saved(&self, world: &World, entity: hecs::Entity) -> Vec<(String, String)> {
        self.saved
            .iter()
            .filter_map(|(name, write)| write(world, entity).map(|text| (name.clone(), text)))
            .collect()
    }

    pub fn is_networked(&self, name: &str) -> bool {
        self.networked.contains_key(name)
    }

    /// Say that `name` is a simulation's state: written by `gather` on the
    /// owner, taken by `take` on everyone else, as bytes the module chooses
    /// (docs/netsim.md, `NetState`). Not a component: nothing in a scene
    /// names it, and it is not saved.
    pub fn register_state(&mut self, name: &str, gather: GatherState, take: TakeState) -> &mut Self {
        self.states.insert(name.to_string(), (gather, take));
        self.reorder();
        self
    }

    pub fn is_state(&self, name: &str) -> bool {
        self.states.contains_key(name)
    }

    /// Every simulation's state on an entity, as `(name, bytes)`.
    pub fn gather_states(&self, world: &World, entity: hecs::Entity) -> Vec<(String, Vec<u8>)> {
        self.states
            .iter()
            .filter_map(|(name, (gather, _))| gather(world, entity).map(|bytes| (name.clone(), bytes)))
            .collect()
    }

    /// The owner's state `name` onto a replica; false for a name that is
    /// not a state.
    pub fn take_state(&self, name: &str, world: &mut World, entity: hecs::Entity, sender: u32, tick: u64, bytes: &[u8]) -> bool {
        match self.states.get(name) {
            Some((_, take)) => {
                take(world, entity, sender, tick, bytes);
                true
            }
            None => false,
        }
    }

    /// Every networked component on an entity, as `(name, RON)`.
    pub fn write_networked(
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
    pub fn insert_text(
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

    /// Take one component off an entity, by name. Nothing for a name
    /// nobody registered.
    pub fn remove_by_name(&self, name: &str, world: &mut World, entity: hecs::Entity) {
        if let Some((_, remove)) = self.by_name.get(name) {
            remove(world, entity);
        }
    }

    /// The networked components' and states' names, sorted: what two
    /// builds must agree on.
    pub fn networked_names(&self) -> impl Iterator<Item = &str> {
        let mut names: Vec<&str> = self.networked.keys().chain(self.states.keys()).map(String::as_str).collect();
        names.sort_unstable();
        names.into_iter()
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
    /// from it. Call it after the scene is spawned.
    pub fn apply(&self, scene: &Scene, world: &mut World) -> Vec<ComponentProblem> {
        self.patch(&Scene::default(), scene, world)
    }

    /// Bring components from one version of a scene to the next: write the
    /// ones whose text changed, remove the ones no longer named, leave the
    /// rest — and whatever the game did to them — alone. Call it after
    /// the world is patched from the one to the other.
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

