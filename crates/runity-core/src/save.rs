//! Saving a game in progress, and loading it back.
//!
//! A world starts from its scene, so a save is what differs from the scene:
//! for every entity the scene has — by the stable id the file gives it —
//! where it is now and the components the game marked as saved
//! ([`Components::register_saved`]); which of the scene's entities are
//! gone; and what was spawned at run time with an id to be known by (see
//! the network module's `announce`). Loading is opening the same scene and putting
//! that back. The file is RON: a save someone can read to see what went
//! wrong, and diff against the one before.
//!
//! What is not marked saved starts from the scene again, which is the
//! useful default: a door's `locked` flag belongs in a save, its hinge's
//! stiffness does not, and a tuning change should reach old saves.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::components::Components;
use crate::id::EntityId;
use crate::world::{addressable, despawn_tree, NetId, NetPrefab};
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
    /// The state its animation graph's controller is in, if it has one:
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
    /// What the editor's tools show while the game runs: the network's
    /// view of each entity, the systems and what they cost. Only in a
    /// report to an editor (`LiveScene::report`), never in a save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Diagnostics>,
}

/// A running game's report beyond where things are: for the editor's
/// network inspector and systems list.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Diagnostics {
    /// Which player this is: 0 the host.
    #[serde(default)]
    pub me: u32,
    /// Each networked entity: its id, owner, whether this peer only holds
    /// a copy, and the newest snapshot it applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub net: Vec<NetLine>,
    /// The game's systems in the order they ran, with what each cost:
    /// median and worst milliseconds over the recent frames.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub systems: Vec<(String, f32, f32)>,
    /// Each animated entity's last transitions, `#update from → to (why)`:
    /// how a character came to be in its state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub animators: Vec<(crate::id::EntityId, Vec<String>)>,
}


/// One networked entity as a peer sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetLine {
    pub id: EntityId,
    pub owner: u32,
    /// A copy moved by its owner's snapshots, not simulated here.
    #[serde(default)]
    pub replica: bool,
    /// The owner's tick of the newest snapshot applied, and how long ago
    /// it came, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick: Option<(u64, f32)>,
}

/// One way two worlds differ, about one entity.
#[derive(Debug, Clone, PartialEq)]
pub enum Difference {
    /// Only in the first, or only in the second.
    OnlyIn(EntityId, usize),
    /// Further apart than the tolerance, in metres.
    Moved(EntityId, f32),
    /// Turned apart, in degrees.
    Turned(EntityId, f32),
    Scaled(EntityId),
    /// A component whose text differs.
    Component(EntityId, String),
}

/// Where two worlds disagree — two players' views of one game, or a game
/// and the scene it started from: the editor's world diff. Positions
/// within `tolerance` metres (and turns within ten times it in degrees)
/// count as the same; networked copies lag by design.
pub fn diff(a: &SaveGame, b: &SaveGame, tolerance: f32) -> Vec<Difference> {
    use std::collections::BTreeMap;
    let index = |g: &SaveGame| -> BTreeMap<EntityId, Saved> {
        g.entities.iter().map(|s| (s.id, s.clone())).collect()
    };
    let (left, right) = (index(a), index(b));
    let mut out = Vec::new();
    for (id, one) in &left {
        let Some(two) = right.get(id) else {
            out.push(Difference::OnlyIn(*id, 0));
            continue;
        };
        let apart = (one.transform.position - two.transform.position).length();
        if apart > tolerance {
            out.push(Difference::Moved(*id, apart));
        }
        let turned = one
            .transform
            .rotation()
            .angle_between(two.transform.rotation())
            .to_degrees();
        if turned > tolerance * 10.0 {
            out.push(Difference::Turned(*id, turned));
        }
        if (one.transform.scale - two.transform.scale).length() > tolerance {
            out.push(Difference::Scaled(*id));
        }
        let components =
            |s: &Saved| -> BTreeMap<String, String> { s.components.iter().cloned().collect() };
        let (c1, c2) = (components(one), components(two));
        for name in c1
            .keys()
            .chain(c2.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            if c1.get(name) != c2.get(name) {
                out.push(Difference::Component(*id, name.clone()));
            }
        }
    }
    for id in right.keys().filter(|id| !left.contains_key(id)) {
        out.push(Difference::OnlyIn(*id, 1));
    }
    out
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
    capture_with(world, components, scene, &|_, _| String::new())
}

/// [`capture`], with each entity's state as `state` tells it: the graph
/// state its animator is in, for the editor's report, when the animation
/// module is there to say.
pub fn capture_with(
    world: &hecs::World,
    components: &Components,
    scene: &Scene,
    state: &dyn Fn(&hecs::World, hecs::Entity) -> String,
) -> SaveGame {
    let mut entities: Vec<Saved> = Vec::new();
    for (entity, id, transform) in world.query::<(hecs::Entity, &SceneId, &Transform)>().iter() {
        entities.push(Saved {
            id: id.0,
            transform: *transform,
            components: components.write_saved(world, entity),
            prefab: String::new(),
            animator: state(world, entity),
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
            animator: state(world, entity),
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
    SaveGame {
        entities,
        gone,
        diagnostics: None,
    }
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

    /// Save as `<name>.ron` in `dir`, keeping the ones before it — Dacha
    /// Simulator's rotation: the last `keep` saves as `<name>.1.ron`,
    /// `<name>.2.ron`…, and `<name>.restore.ron`, the newest save before
    /// this one that read back whole. A save that is cut short (the power
    /// goes) leaves the ones before it, and [`SaveGame::read_newest`] finds
    /// the newest that reads.
    pub fn write_rotating(
        &self,
        dir: impl AsRef<Path>,
        name: &str,
        keep: usize,
    ) -> Result<PathBuf, String> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let at = |n: usize| {
            if n == 0 {
                dir.join(format!("{name}.ron"))
            } else {
                dir.join(format!("{name}.{n}.ron"))
            }
        };
        let current = at(0);
        if SaveGame::read(&current).is_ok() {
            let _ = std::fs::copy(&current, dir.join(format!("{name}.restore.ron")));
        }
        for n in (0..keep).rev() {
            let from = at(n);
            if from.is_file() {
                if n + 1 > keep {
                    let _ = std::fs::remove_file(&from);
                } else {
                    let _ = std::fs::rename(&from, at(n + 1));
                }
            }
        }
        self.write(&current)?;
        Ok(current)
    }

    /// The newest save of a rotation that reads: `<name>.ron`, then
    /// `<name>.1.ron`…, then the restore copy.
    pub fn read_newest(dir: impl AsRef<Path>, name: &str) -> Option<(PathBuf, SaveGame)> {
        let dir = dir.as_ref();
        let mut candidates = vec![dir.join(format!("{name}.ron"))];
        candidates.extend((1..=16).map(|n| dir.join(format!("{name}.{n}.ron"))));
        candidates.push(dir.join(format!("{name}.restore.ron")));
        candidates
            .into_iter()
            .find_map(|p| SaveGame::read(&p).ok().map(|s| (p, s)))
    }

    pub fn read(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}:{e}", path.display()))
    }
}

#[cfg(test)]
mod rotation_tests {
    use super::*;

    #[test]
    fn saves_rotate_three_deep_and_a_broken_one_falls_back() {
        let dir = std::env::temp_dir().join(format!("runity-rotation-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save = |n: u64| SaveGame {
            gone: vec![EntityId::from_raw(n)],
            ..Default::default()
        };
        for n in 1..=5 {
            save(n).write_rotating(&dir, "slot", 3).unwrap();
        }
        let gone = |p: &str| SaveGame::read(dir.join(p)).unwrap().gone[0].raw();
        assert_eq!(gone("slot.ron"), 5);
        assert_eq!(gone("slot.1.ron"), 4);
        assert_eq!(gone("slot.3.ron"), 2);
        assert!(!dir.join("slot.4.ron").exists(), "three kept");
        assert_eq!(gone("slot.restore.ron"), 4, "the last good one before");

        // Cut short: the newest that reads is the one before.
        std::fs::write(dir.join("slot.ron"), "(entities: [(id: ").unwrap();
        let (path, newest) = SaveGame::read_newest(&dir, "slot").unwrap();
        assert!(path.ends_with("slot.1.ron"));
        assert_eq!(newest.gone[0].raw(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod diff_tests {
    use super::*;

    fn saved(id: u64, x: f32, door: &str) -> Saved {
        Saved {
            id: EntityId::from_raw(id),
            transform: Transform {
                position: glam::Vec3::new(x, 0.0, 0.0),
                ..Default::default()
            },
            components: vec![("door".into(), door.into())],
            prefab: String::new(),
            animator: String::new(),
        }
    }

    #[test]
    fn two_worlds_differ_where_they_do_and_not_within_the_tolerance() {
        let a = SaveGame {
            entities: vec![
                saved(1, 0.0, "(open: true)"),
                saved(2, 5.0, "()"),
                saved(3, 0.0, "()"),
            ],
            ..Default::default()
        };
        let b = SaveGame {
            entities: vec![
                saved(1, 0.01, "(open: false)"),
                saved(2, 7.0, "()"),
                saved(4, 0.0, "()"),
            ],
            ..Default::default()
        };
        let d = diff(&a, &b, 0.05);
        assert!(d.contains(&Difference::Component(EntityId::from_raw(1), "door".into())));
        assert!(!d
            .iter()
            .any(|x| matches!(x, Difference::Moved(id, _) if *id == EntityId::from_raw(1))));
        assert!(d.contains(&Difference::Moved(EntityId::from_raw(2), 2.0)));
        assert!(d.contains(&Difference::OnlyIn(EntityId::from_raw(3), 0)));
        assert!(d.contains(&Difference::OnlyIn(EntityId::from_raw(4), 1)));
    }
}

