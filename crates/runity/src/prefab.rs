//! Prefabs: a thing built once and placed many times.
//!
//! A campfire is a fire, a ring of stones and an ember, arranged just so.
//! Placing twelve of them by copying the subtree into twelve scenes means
//! twelve places to fix when the ring turns out to be too wide — which is
//! how a scene file becomes something nobody edits by hand again.
//!
//! So a prefab is one entity subtree in its own file, and a scene points at
//! it by name. Editing the prefab changes every instance, everywhere, with
//! no step in between.
//!
//! Text, not an asset. A prefab is edited by people and by agents exactly
//! like a scene is, and `git` has to show what changed in it — the same
//! reason scenes are RON while meshes are binary. It is authored input, not
//! compiled output.
//!
//! **What this does not do yet:** overriding something deep inside an
//! instance — "this campfire's third stone is turned a bit" — which is the
//! part of Unity's prefab system that is genuinely hard, and the part that
//! goes wrong quietly when it is built in a hurry. An instance overrides its
//! own name, placement, material and physics, and can have children of its
//! own; anything deeper is a change to the prefab. When that turns out to be
//! too little, the missing piece is an override list on the instance, keyed
//! by a path through the prefab — and it should be designed then, against a
//! real scene that needs it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::scene::{Body, Collider, EntityDesc, MaterialRef, Scene};

/// What a prefab file is called.
pub const EXTENSION: &str = "prefab";

/// Prefabs a scene can name, by file stem.
#[derive(Debug, Clone, Default)]
pub struct Prefabs {
    by_name: HashMap<String, EntityDesc>,
}

impl Prefabs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read every `.prefab` in a directory.
    ///
    /// A file that does not parse is reported and skipped, like a bad asset
    /// in a library: one broken prefab should cost one missing thing, not
    /// every scene that happens to sit beside it.
    pub fn open(directory: impl AsRef<Path>) -> std::io::Result<(Self, Vec<(PathBuf, String)>)> {
        let mut prefabs = Self::new();
        let mut problems = Vec::new();
        for entry in std::fs::read_dir(directory.as_ref())? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
                continue;
            }
            match Self::read(&path) {
                Ok((name, desc)) => {
                    prefabs.insert(name, desc);
                }
                Err(e) => problems.push((path, e)),
            }
        }
        Ok((prefabs, problems))
    }

    /// Open the prefabs that belong to a scene: the `prefabs/` directory
    /// beside it.
    ///
    /// A convention rather than a path in the scene file, because every tool
    /// has to find the same ones — the headless render, the walk-around and
    /// the editor — and a path written in one scene is a path the next scene
    /// gets wrong. No such directory means no prefabs, which is not an
    /// error: most scenes have none.
    pub fn beside(scene: impl AsRef<Path>) -> (Self, Vec<(PathBuf, String)>) {
        let Some(directory) = scene.as_ref().parent().map(|d| d.join("prefabs")) else {
            return (Self::new(), Vec::new());
        };
        Self::open(directory).unwrap_or_else(|_| (Self::new(), Vec::new()))
    }

    /// Read one prefab file, returning its name and what is in it.
    pub fn read(path: impl AsRef<Path>) -> Result<(String, EntityDesc), String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let desc: EntityDesc =
            ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok((name, desc))
    }

    /// Write one out, pretty-printed the way a scene is.
    pub fn save(desc: &EntityDesc, path: impl AsRef<Path>) -> Result<(), String> {
        // Struct names off, for the reason scenes have them off: with them
        // on, an untagged material cannot be read back, and a prefab that
        // saves cleanly and will not reopen is the worst kind of bug.
        let pretty = ron::ser::PrettyConfig::new().depth_limit(4);
        let text = ron::ser::to_string_pretty(desc, pretty).map_err(|e| e.to_string())?;
        std::fs::write(path.as_ref(), text).map_err(|e| e.to_string())
    }

    pub fn insert(&mut self, name: impl Into<String>, desc: EntityDesc) {
        self.by_name.insert(name.into(), desc);
    }

    pub fn get(&self, name: &str) -> Option<&EntityDesc> {
        self.by_name.get(name)
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Every prefab's name, sorted, so an editor's list does not reshuffle
    /// itself between openings.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.by_name.keys().map(|k| k.as_str()).collect();
        names.sort_unstable();
        names
    }
}

/// A scene with every instance replaced by what it stands for.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Instanced {
    /// The expanded scene: a plain tree, with no instances left in it.
    /// Everything downstream — spawning, culling, rendering — sees only
    /// this, and so knows nothing about prefabs at all.
    pub scene: Scene,
    /// For each entity in `scene.flatten()`, which entity of the *source*
    /// scene's `flatten()` it belongs to.
    ///
    /// Anything that came out of a prefab points at the instance that
    /// brought it in, because that is the thing the document can select,
    /// move and delete. Without this an editor could tell you a stone was
    /// clicked and have nothing to do about it.
    pub source: Vec<usize>,
    /// Instances whose prefab could not be expanded, and why. Reported
    /// rather than logged: the editor wants to show this next to the entity.
    pub problems: Vec<Problem>,
}

/// An instance that could not be expanded.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub entity_name: String,
    pub prefab: String,
    pub reason: String,
}

/// How deep prefabs may nest before we call it a loop.
///
/// A prefab that contains itself is a file that describes an infinite scene,
/// and the honest answer is to stop and say so. Eight is well past anything
/// a person nests on purpose.
const MAX_DEPTH: usize = 8;

/// Replace every prefab instance in a scene with what it names.
///
/// The source scene is untouched: the document keeps its references, so
/// saving it writes the references back rather than the expansion. Nothing
/// downstream has to know a prefab existed.
pub fn instantiate(scene: &Scene, prefabs: &Prefabs) -> Instanced {
    let mut out = Instanced {
        scene: Scene {
            view: scene.view,
            sun: scene.sun,
            fog: scene.fog,
            entities: Vec::new(),
        },
        source: Vec::new(),
        problems: Vec::new(),
    };
    // The document's flatten index, advanced in the same pre-order flatten
    // uses, so that what comes out lines up with what an editor lists.
    let mut document_index = 0usize;
    let mut entities = Vec::new();
    for desc in &scene.entities {
        entities.push(expand(
            desc,
            prefabs,
            &mut document_index,
            0,
            &mut out.source,
            &mut out.problems,
        ));
    }
    out.scene.entities = entities;
    out
}

/// Expand one entity, appending a source index for it and everything under
/// it in the order `flatten` will produce.
fn expand(
    desc: &EntityDesc,
    prefabs: &Prefabs,
    document_index: &mut usize,
    depth: usize,
    source: &mut Vec<usize>,
    problems: &mut Vec<Problem>,
) -> EntityDesc {
    let own_index = *document_index;
    *document_index += 1;
    source.push(own_index);

    let mut expanded = match resolve(desc, prefabs, depth, problems) {
        Some(from_prefab) => from_prefab,
        None => EntityDesc {
            children: Vec::new(),
            ..desc.clone()
        },
    };

    // Whatever the prefab brought with it belongs to the instance: it has no
    // entry in the document, so it cannot be selected or moved on its own.
    let inherited = std::mem::take(&mut expanded.children);
    let mut children = Vec::with_capacity(inherited.len() + desc.children.len());
    for child in &inherited {
        children.push(claim(child, own_index, source));
    }
    // The instance's own children do have document entries, and keep them.
    for child in &desc.children {
        children.push(expand(
            child,
            prefabs,
            document_index,
            depth,
            source,
            problems,
        ));
    }
    expanded.children = children;
    expanded.prefab = String::new();
    expanded
}

/// Take a subtree that came out of a prefab and mark all of it as belonging
/// to one instance.
fn claim(desc: &EntityDesc, owner: usize, source: &mut Vec<usize>) -> EntityDesc {
    source.push(owner);
    EntityDesc {
        children: desc
            .children
            .iter()
            .map(|child| claim(child, owner, source))
            .collect(),
        prefab: String::new(),
        ..desc.clone()
    }
}

/// What an instance stands for, with the instance's own overrides on top.
///
/// Returns `None` for an entity that is not an instance, and for one whose
/// prefab is missing — a scene with a broken reference still opens, with a
/// hole where the thing should be, which is more useful than no scene.
fn resolve(
    desc: &EntityDesc,
    prefabs: &Prefabs,
    depth: usize,
    problems: &mut Vec<Problem>,
) -> Option<EntityDesc> {
    if desc.prefab.is_empty() {
        return None;
    }
    if depth >= MAX_DEPTH {
        problems.push(Problem {
            entity_name: desc.name.clone(),
            prefab: desc.prefab.clone(),
            reason: format!("nested more than {MAX_DEPTH} deep — a prefab containing itself?"),
        });
        return None;
    }
    let Some(template) = prefabs.get(&desc.prefab) else {
        problems.push(Problem {
            entity_name: desc.name.clone(),
            prefab: desc.prefab.clone(),
            reason: "no prefab by that name".into(),
        });
        return None;
    };

    // A prefab that is itself built out of prefabs: expanded here, so that a
    // shelter made of walls is one thing to place.
    let mut nested_source = Vec::new();
    let mut index = 0usize;
    let mut root = expand(
        template,
        prefabs,
        &mut index,
        depth + 1,
        &mut nested_source,
        problems,
    );

    // The instance's own overrides. Name and placement always, because that
    // is what placing a thing *is*. The rest only when the scene said
    // something: a default here means "unspecified", not "plain grey".
    root.name = desc.name.clone();
    root.transform = desc.transform;
    if desc.material != MaterialRef::default() {
        root.material = desc.material.clone();
    }
    if desc.body != Body::default() {
        root.body = desc.body;
    }
    if desc.collider != Collider::default() {
        root.collider = desc.collider;
    }
    Some(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Transform;
    use glam::Vec3;

    fn campfire() -> EntityDesc {
        ron::from_str(
            r#"(
                name: "campfire",
                model: "builtin:plane",
                material: "earth",
                children: [
                    (name: "ember", model: "builtin:sphere", material: "ember",
                     transform: (position: (0.0, 0.2, 0.0))),
                    (name: "stone", model: "builtin:cube", material: "stone",
                     transform: (position: (0.8, 0.0, 0.0))),
                ],
            )"#,
        )
        .unwrap()
    }

    fn with_campfire() -> Prefabs {
        let mut prefabs = Prefabs::new();
        prefabs.insert("campfire", campfire());
        prefabs
    }

    fn scene_with_two_fires() -> Scene {
        ron::from_str(
            r#"(entities: [
                (name: "ground", model: "builtin:plane"),
                (name: "north fire", model: "", prefab: "campfire",
                 transform: (position: (0.0, 0.0, -10.0))),
                (name: "south fire", model: "", prefab: "campfire",
                 transform: (position: (0.0, 0.0, 10.0))),
            ])"#,
        )
        .unwrap()
    }

    #[test]
    fn an_instance_becomes_the_whole_thing_it_names() {
        let done = instantiate(&scene_with_two_fires(), &with_campfire());
        assert!(done.problems.is_empty(), "{:?}", done.problems);

        let flat = done.scene.flatten();
        assert_eq!(flat.len(), 7, "ground plus two fires of three each");
        let names: Vec<&str> = flat.iter().map(|(e, _)| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "ground",
                "north fire",
                "ember",
                "stone",
                "south fire",
                "ember",
                "stone"
            ]
        );

        // Placed by the instance, not by the prefab: the ember of the north
        // fire is ten metres away from the south one's.
        let embers: Vec<f32> = flat
            .iter()
            .filter(|(e, _)| e.name == "ember")
            .map(|(_, world)| world.w_axis.z)
            .collect();
        assert_eq!(embers, vec![-10.0, 10.0]);

        // And the prefab's own look came along.
        let ember = done.scene.find("ember").unwrap();
        assert_eq!(ember.material(), crate::material::builtin::EMBER);
    }

    #[test]
    fn everything_a_prefab_brought_belongs_to_the_instance_that_brought_it() {
        // The editor has to be able to answer "what did I just click on"
        // with something the document can move. A stone that came out of a
        // prefab has no entry of its own, so the answer is the fire.
        let done = instantiate(&scene_with_two_fires(), &with_campfire());
        assert_eq!(done.source, vec![0, 1, 1, 1, 2, 2, 2]);
    }

    #[test]
    fn an_instance_places_and_recolours_without_touching_the_prefab() {
        let scene: Scene = ron::from_str(
            r#"(entities: [(
                name: "cold fire", model: "", prefab: "campfire",
                material: "stone", body: Static,
                transform: (position: (3.0, 0.0, 0.0), scale: (2.0, 2.0, 2.0)),
            )])"#,
        )
        .unwrap();
        let prefabs = with_campfire();
        let done = instantiate(&scene, &prefabs);
        let root = &done.scene.entities[0];

        assert_eq!(root.model, "builtin:plane", "the prefab says what it is");
        assert_eq!(root.material(), crate::material::builtin::STONE);
        assert_eq!(root.body, Body::Static);
        assert_eq!(root.transform.scale, Vec3::splat(2.0));
        assert_eq!(root.children.len(), 2, "and it still brought its children");

        // The prefab itself is untouched, so the next instance is not
        // wearing the last one's overrides.
        assert_eq!(prefabs.get("campfire").unwrap().material(), {
            crate::material::builtin::EARTH
        });
    }

    #[test]
    fn an_instance_may_have_children_of_its_own() {
        // Those do have entries in the document, so they stay selectable —
        // which is the difference between "part of the prefab" and "put
        // there beside it".
        let scene: Scene = ron::from_str(
            r#"(entities: [(
                name: "fire", model: "", prefab: "campfire",
                children: [(name: "kettle", model: "builtin:sphere")],
            )])"#,
        )
        .unwrap();
        let done = instantiate(&scene, &with_campfire());
        let names: Vec<&str> = done
            .scene
            .flatten()
            .iter()
            .map(|(e, _)| e.name.as_str())
            .collect();
        assert_eq!(names, ["fire", "ember", "stone", "kettle"]);
        assert_eq!(
            done.source,
            vec![0, 0, 0, 1],
            "the kettle is its own entry; the prefab's parts are the fire's"
        );
    }

    #[test]
    fn a_prefab_can_be_built_out_of_prefabs() {
        let mut prefabs = with_campfire();
        prefabs.insert(
            "camp",
            ron::from_str(
                r#"(name: "camp", model: "builtin:plane", children: [
                    (name: "fire", model: "", prefab: "campfire"),
                ])"#,
            )
            .unwrap(),
        );
        let scene: Scene =
            ron::from_str(r#"(entities: [(name: "camp", model: "", prefab: "camp")])"#).unwrap();
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert_eq!(done.scene.flatten().len(), 4, "camp, fire, ember, stone");
        assert!(
            done.source.iter().all(|i| *i == 0),
            "all of it belongs to the one instance in the document"
        );
    }

    #[test]
    fn a_prefab_that_contains_itself_stops_and_says_so() {
        // A file describing an infinite scene. Stopping with a message beats
        // filling memory, and beats silently drawing one level of it.
        let mut prefabs = Prefabs::new();
        prefabs.insert(
            "snake",
            ron::from_str(
                r#"(name: "snake", model: "builtin:cube", children: [
                    (name: "tail", model: "", prefab: "snake"),
                ])"#,
            )
            .unwrap(),
        );
        let scene: Scene =
            ron::from_str(r#"(entities: [(name: "s", model: "", prefab: "snake")])"#).unwrap();
        let done = instantiate(&scene, &prefabs);
        assert!(!done.problems.is_empty(), "it should complain");
        assert!(done.problems[0].reason.contains("itself"));
        assert!(done.scene.flatten().len() < 64, "and it stopped");
        assert_eq!(done.source.len(), done.scene.flatten().len());
    }

    #[test]
    fn a_missing_prefab_leaves_a_hole_rather_than_refusing_the_scene() {
        let scene: Scene = ron::from_str(
            r#"(entities: [
                (name: "ground", model: "builtin:plane"),
                (name: "ghost", model: "", prefab: "not_here"),
            ])"#,
        )
        .unwrap();
        let done = instantiate(&scene, &Prefabs::new());
        assert_eq!(done.problems.len(), 1);
        assert_eq!(done.problems[0].entity_name, "ghost");
        assert_eq!(done.scene.flatten().len(), 2, "the scene still opens");
        assert_eq!(done.source, vec![0, 1]);
    }

    #[test]
    fn a_prefab_round_trips_through_a_file() {
        let dir = std::env::temp_dir().join("runity-prefab-file");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("campfire.{EXTENSION}"));
        Prefabs::save(&campfire(), &path).unwrap();

        let (prefabs, problems) = Prefabs::open(&dir).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(prefabs.names(), vec!["campfire"]);
        assert_eq!(prefabs.get("campfire"), Some(&campfire()));
    }

    #[test]
    fn a_prefab_that_does_not_parse_costs_one_prefab_and_not_the_directory() {
        let dir = std::env::temp_dir().join("runity-prefab-broken");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Prefabs::save(&campfire(), dir.join(format!("good.{EXTENSION}"))).unwrap();
        std::fs::write(dir.join(format!("bad.{EXTENSION}")), "(name: ").unwrap();

        let (prefabs, problems) = Prefabs::open(&dir).unwrap();
        assert_eq!(problems.len(), 1);
        assert_eq!(prefabs.names(), vec!["good"]);
    }

    #[test]
    fn a_scene_with_no_prefabs_is_the_same_scene() {
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "rock".into(),
                model: "builtin:sphere".into(),
                transform: Transform {
                    position: Vec3::new(1.0, 0.0, 0.0),
                    ..Transform::default()
                },
                ..EntityDesc::default()
            }],
            ..Scene::default()
        };
        let done = instantiate(&scene, &Prefabs::new());
        assert_eq!(done.scene, scene);
        assert_eq!(done.source, vec![0]);
    }
}
