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
//! An instance changes its own name, placement, material, physics and
//! components on its line, can have children of its own, and changes parts
//! of the prefab through `overrides`, keyed by each part's id in the prefab
//! file — so moving a stone in the file does not orphan the override.
//!
//! A **variant** is a prefab file whose root is itself an instance: a
//! campfire with mossy stones is `(name: "mossy campfire", prefab:
//! "campfire", overrides: {…})`. Unity needs a separate asset kind for
//! this; here it is the same line a scene would write, in its own file.
//! The base still reaches every variant where the variant said nothing, and
//! a scene overrides a variant's parts by the same ids as the base's.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::id::EntityId;
use crate::scene::{Body, Collider, EntityDesc, MaterialRef, Scene};

/// What a prefab file is called.
pub const EXTENSION: &str = "prefab";

/// Prefabs a scene can name, by file stem.
#[derive(Debug, Clone, Default)]
pub struct Prefabs {
    by_name: HashMap<String, EntityDesc>,
    /// Each prefab's ID, from its sidecar, and back: what a link to one
    /// holds (docs/refs.md).
    ids: HashMap<crate::AssetId, String>,
    id_of: HashMap<String, crate::AssetId>,
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
                    if let Some(id) = crate::asset::sidecar_id(crate::asset::sidecar_of(&path)) {
                        prefabs.ids.insert(id, name.clone());
                        prefabs.id_of.insert(name.clone(), id);
                    }
                    prefabs.insert(name, desc);
                }
                Err(e) => problems.push((path, e)),
            }
        }
        Ok((prefabs, problems))
    }

    /// A project's prefabs: everything in its `prefabs/`.
    ///
    /// Found through the project rather than next to whichever scene is
    /// open, so the headless render, the walk-around and the editor all see
    /// the same set, and a scene in `scenes/caves/` finds the same campfire
    /// as one in `scenes/`. No folder means no prefabs, which is not an
    /// error: most projects start with none.
    pub fn of(project: &crate::Project) -> (Self, Vec<(PathBuf, String)>) {
        Self::open(project.prefabs()).unwrap_or_else(|_| (Self::new(), Vec::new()))
    }

    /// Read one prefab file, returning its name and what is in it.
    pub fn read(path: impl AsRef<Path>) -> Result<(String, EntityDesc), String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut desc: EntityDesc =
            ron::from_str(&text).map_err(|e| format!("{}:{e}", path.display()))?;
        // The same rule as a scene: an entity without an ID gets one, and a
        // repeated one is re-minted, so every part of an instance has an
        // identity to be scoped — derived from its place when missing, so
        // an instance's parts keep their IDs across reloads of the file.
        crate::scene::derive_ids(std::slice::from_mut(&mut desc), &mut HashSet::new());
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
        crate::ron_text::write_preserving(path.as_ref(), desc, pretty).map_err(|e| e.to_string())
    }

    pub fn insert(&mut self, name: impl Into<String>, mut desc: EntityDesc) {
        crate::scene::assign_ids(std::slice::from_mut(&mut desc), &mut HashSet::new());
        self.by_name.insert(name.into(), desc);
    }

    pub fn get(&self, name: &str) -> Option<&EntityDesc> {
        self.by_name.get(name)
    }

    /// Follow a link to a prefab: by its ID, then by its name. The name
    /// found is the prefab's name now.
    pub fn find(&self, link: &crate::AssetLink) -> Option<(&str, &EntityDesc)> {
        let name = link
            .id
            .and_then(|id| self.ids.get(&id))
            .map(String::as_str)
            .unwrap_or(link.as_str());
        let (name, desc) = self.by_name.get_key_value(name)?;
        Some((name.as_str(), desc))
    }

    /// A prefab's ID, when its sidecar has been written.
    pub fn id_of(&self, name: &str) -> Option<crate::AssetId> {
        self.id_of.get(name).copied()
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
    /// For every entity in `scene`, the entity of the *document* it belongs
    /// to.
    ///
    /// A document entity belongs to itself. Anything that came out of a
    /// prefab belongs to the instance that brought it in, because that is
    /// the thing the document can select, move and delete. Without this an
    /// editor could tell you a stone was clicked and have nothing to do
    /// about it.
    pub owner: HashMap<EntityId, EntityId>,
    /// For every entity that came out of a prefab file: the instance its ID
    /// is scoped to, and its ID in the prefab file. What an override of that
    /// part is keyed by.
    pub parts: HashMap<EntityId, (EntityId, EntityId)>,
    /// Instances whose prefab could not be expanded, and why. Reported
    /// rather than logged: the editor wants to show this next to the entity.
    pub problems: Vec<Problem>,
}

impl Instanced {
    /// The document entity an expanded one belongs to.
    pub fn owner_of(&self, id: EntityId) -> Option<EntityId> {
        self.owner.get(&id).copied()
    }
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
///
/// Every entity a prefab brings gets the ID [`EntityId::within`] gives it —
/// its ID in the prefab file, scoped to the instance — so the same stone in
/// two campfires is two entities, and each keeps its identity for as long as
/// the instance and the prefab do.
pub fn instantiate(scene: &Scene, prefabs: &Prefabs) -> Instanced {
    let mut problems = Vec::new();
    let mut parts = HashMap::new();
    let entities = scene
        .entities
        .iter()
        .map(|desc| expand(desc, None, prefabs, 0, &mut problems, &mut parts))
        .collect();
    let expanded = Scene {
        view: scene.view,
        sun: scene.sun,
        fog: scene.fog,
        sky: scene.sky,
        post: scene.post,
        ambient_occlusion: scene.ambient_occlusion,
        ray_tracing: scene.ray_tracing,
        virtual_shadows: scene.virtual_shadows,
        volumetric_fog: scene.volumetric_fog,
        wind: scene.wind,
        weather: scene.weather,
        screen_space_reflections: scene.screen_space_reflections,
        entities,
    };

    // Who owns what, read off the result: a document entity owns itself,
    // and anything else belongs to its nearest ancestor that is in the
    // document. Derived IDs are not document IDs, so this lands every part
    // of a prefab on the instance that brought it.
    let document: HashSet<EntityId> = scene.ids().into_iter().collect();
    let mut owner = HashMap::new();
    fn walk(
        entities: &[EntityDesc],
        current: Option<EntityId>,
        document: &HashSet<EntityId>,
        owner: &mut HashMap<EntityId, EntityId>,
    ) {
        for entity in entities {
            let mine = if document.contains(&entity.id) {
                Some(entity.id)
            } else {
                current
            };
            if let Some(mine) = mine {
                owner.insert(entity.id, mine);
            }
            walk(&entity.children, mine, document, owner);
        }
    }
    walk(&expanded.entities, None, &document, &mut owner);

    Instanced {
        scene: expanded,
        owner,
        parts,
        problems,
    }
}

fn find_mut(entities: &mut [EntityDesc], id: EntityId) -> Option<&mut EntityDesc> {
    for entity in entities {
        if entity.id == id {
            return Some(entity);
        }
        if let Some(found) = find_mut(&mut entity.children, id) {
            return Some(found);
        }
    }
    None
}

/// Expand one entity and everything under it.
///
/// `scope` is `None` for an entity of the document, which keeps its own ID,
/// and the instance's ID for an entity out of a prefab file, whose ID is
/// scoped to it.
fn expand(
    desc: &EntityDesc,
    scope: Option<EntityId>,
    prefabs: &Prefabs,
    depth: usize,
    problems: &mut Vec<Problem>,
    parts: &mut HashMap<EntityId, (EntityId, EntityId)>,
) -> EntityDesc {
    let id = match scope {
        None => desc.id,
        Some(instance) => {
            let id = instance.within(desc.id);
            parts.insert(id, (instance, desc.id));
            id
        }
    };
    // What this line itself says — its components' links, its joint — is
    // written in the file that holds it, so it takes that file's scope:
    // here, before its prefab (if it is an instance) is expanded, so what
    // the prefab brings keeps the scope of its own instance and is not
    // scoped again. A door's hinge in the prefab is this door's hinge in
    // the scene, as Unity rewrites a prefab's references per instance, and
    // a door in a shed in a yard is still that door's.
    let local = match scope {
        Some(instance) => scoped_links(desc, instance),
        None => desc.clone(),
    };
    // An instance becomes what its prefab holds, children and all; anything
    // else is itself, with its children still to come.
    let mut expanded =
        resolve(&local, id, prefabs, depth, problems, parts).unwrap_or_else(|| EntityDesc {
            children: Vec::new(),
            ..local.clone()
        });
    expanded.id = id;
    expanded.prefab = Default::default();
    // Its own children come after whatever the prefab brought, in the same
    // scope as itself: a kettle put beside a campfire in the scene is the
    // scene's, not the campfire's.
    for child in &desc.children {
        expanded
            .children
            .push(expand(child, scope, prefabs, depth, problems, parts));
    }
    // What a spline carries grows here, like a prefab's parts: the file
    // keeps the spline and the spacing, everything downstream sees copies.
    if let (Some(spline), Some(along)) = (&expanded.spline, &expanded.along) {
        let grown = along.grow(id, spline);
        expanded.children.extend(grown);
    }
    expanded
}

/// Point every link to `from` in `desc` and under it at `to` instead.
fn relink(desc: &mut EntityDesc, from: EntityId, to: EntityId) {
    if desc.joint.to() == Some(from) {
        desc.joint = desc.joint.with_to(to);
    }
    let (was, now) = (
        format!("EntityRef(\"{from}\")"),
        format!("EntityRef(\"{to}\")"),
    );
    for value in desc.components.values_mut() {
        if value.get_ron().contains(&was) {
            let text = value.get_ron().replace(&was, &now);
            if let Ok(raw) = ron::value::RawValue::from_boxed_ron(text.into_boxed_str()) {
                *value = raw;
            }
        }
    }
    for child in &mut desc.children {
        relink(child, from, to);
    }
}

/// A line with the links it writes — its components' `EntityRef`s and its
/// joint's other end — put in `instance`'s scope; its children as they are.
fn scoped_links(desc: &EntityDesc, instance: EntityId) -> EntityDesc {
    let mut out = desc.clone();
    if let Some(to) = out.joint.to() {
        if !to.is_unassigned() {
            out.joint = out.joint.with_to(instance.within(to));
        }
    }
    for value in out.components.values_mut() {
        let text = value.get_ron();
        let links = crate::EntityRef::find_in(text);
        if links.is_empty() {
            continue;
        }
        let mut scoped = text.to_string();
        for id in links {
            scoped = scoped.replace(
                &format!("EntityRef(\"{id}\")"),
                &format!("EntityRef(\"{}\")", instance.within(id)),
            );
        }
        if let Ok(raw) = ron::value::RawValue::from_boxed_ron(scoped.into_boxed_str()) {
            *value = raw;
        }
    }
    out
}

/// What an instance stands for, with the instance's own overrides on top.
///
/// Returns `None` for an entity that is not an instance, and for one whose
/// prefab is missing — a scene with a broken reference still opens, with a
/// hole where the thing should be, which is more useful than no scene.
fn resolve(
    desc: &EntityDesc,
    id: EntityId,
    prefabs: &Prefabs,
    depth: usize,
    problems: &mut Vec<Problem>,
    parts: &mut HashMap<EntityId, (EntityId, EntityId)>,
) -> Option<EntityDesc> {
    if desc.prefab.is_empty() {
        return None;
    }
    if depth >= MAX_DEPTH {
        problems.push(Problem {
            entity_name: desc.name.clone(),
            prefab: desc.prefab.to_string(),
            reason: format!("nested more than {MAX_DEPTH} deep — a prefab containing itself?"),
        });
        return None;
    }
    let Some((_, template)) = prefabs.find(&desc.prefab) else {
        problems.push(Problem {
            entity_name: desc.name.clone(),
            prefab: desc.prefab.to_string(),
            reason: "no prefab by that name".into(),
        });
        return None;
    };

    // Everything in the prefab is scoped to this instance. A prefab that is
    // itself built out of prefabs is expanded on the way, so a shelter made
    // of walls is one thing to place.
    let mut root =
        if template.prefab.is_empty() {
            let mut root = expand(template, Some(id), prefabs, depth + 1, problems, parts);
            // The root is the instance itself, not a part of it — and so is
            // what a link to the prefab's root names.
            parts.remove(&id.within(template.id));
            relink(&mut root, id.within(template.id), id);
            root
        } else {
            // A variant: a prefab whose root is an instance of another — Unity's
            // prefab variant, which here is nothing but a prefab file written
            // the way a scene line is. Its root *is* this instance, so the base
            // is expanded straight into this instance's scope: a scene override
            // names a base part by the same id the variant's own overrides do,
            // and a variant of a variant is no deeper to address.
            let mut root = resolve(template, id, prefabs, depth + 1, problems, parts)
                .unwrap_or_else(|| EntityDesc {
                    children: Vec::new(),
                    ..template.clone()
                });
            root.prefab = Default::default();
            for child in &template.children {
                root.children
                    .push(expand(child, Some(id), prefabs, depth + 1, problems, parts));
            }
            root
        };

    // The instance *is* the prefab's root, so it keeps the instance's ID.
    // Its own overrides: name and placement always, because that is what
    // placing a thing is. The rest only when the scene said something: a
    // default here means "unspecified", not "plain grey".
    root.id = id;
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
    // The instance's own joint names something in the scene, as written.
    if !desc.joint.is_none() {
        root.joint = desc.joint;
    }
    // Components one by one: an instance that says `"door": (locked:
    // true)` changes the door and keeps the prefab's other components.
    for (name, value) in &desc.components {
        root.components.insert(name.clone(), value.clone());
    }
    // Overrides of the prefab's parts, each found by its id in the prefab
    // file — scoped to this instance, as every part is.
    for (part, change) in &desc.overrides {
        let scoped = id.within(*part);
        match find_mut(&mut root.children, scoped) {
            Some(target) => change.apply(target),
            None => problems.push(Problem {
                entity_name: desc.name.clone(),
                prefab: desc.prefab.to_string(),
                reason: format!(
                    "an override for part {part}, which the prefab does not have (any more?)"
                ),
            }),
        }
    }
    Some(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Transform;
    use glam::Vec3;

    /// A scene as `Scene::load` would hand it over: parsed, with IDs.
    fn parse(text: &str) -> Scene {
        let mut scene: Scene = ron::from_str(text).unwrap();
        scene.assign_ids();
        scene
    }

    fn campfire() -> EntityDesc {
        let mut desc: EntityDesc = ron::from_str(
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
        .unwrap();
        crate::scene::assign_ids(std::slice::from_mut(&mut desc), &mut HashSet::new());
        desc
    }

    fn with_campfire() -> Prefabs {
        let mut prefabs = Prefabs::new();
        prefabs.insert("campfire", campfire());
        prefabs
    }

    fn scene_with_two_fires() -> Scene {
        parse(
            r#"(entities: [
                (name: "ground", model: "builtin:plane"),
                (name: "north fire", model: "", prefab: "campfire",
                 transform: (position: (0.0, 0.0, -10.0))),
                (name: "south fire", model: "", prefab: "campfire",
                 transform: (position: (0.0, 0.0, 10.0))),
            ])"#,
        )
    }

    /// Which document entity owns each expanded one, by name, in tree order.
    fn owners(document: &Scene, done: &Instanced) -> Vec<String> {
        done.scene
            .flatten()
            .iter()
            .map(|(e, _)| {
                let owner = done.owner_of(e.id).expect("every entity has an owner");
                document.get(owner).unwrap().name.clone()
            })
            .collect()
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
        let scene = scene_with_two_fires();
        let done = instantiate(&scene, &with_campfire());
        assert_eq!(
            owners(&scene, &done),
            [
                "ground",
                "north fire",
                "north fire",
                "north fire",
                "south fire",
                "south fire",
                "south fire"
            ]
        );
    }

    #[test]
    fn the_same_stone_in_two_fires_is_two_entities_and_keeps_being_them() {
        // Each part of an instance has an identity of its own — what an
        // override or a network message will point at — and it does not
        // change between two expansions of the same scene.
        let scene = scene_with_two_fires();
        let prefabs = with_campfire();
        let stones = |done: &Instanced| -> Vec<EntityId> {
            done.scene
                .flatten()
                .iter()
                .filter(|(e, _)| e.name == "stone")
                .map(|(e, _)| e.id)
                .collect()
        };
        let first = stones(&instantiate(&scene, &prefabs));
        assert_eq!(first.len(), 2);
        assert_ne!(first[0], first[1], "two stones, two identities");
        assert_eq!(first, stones(&instantiate(&scene, &prefabs)), "and stable");

        // The instance itself keeps the document's ID: it *is* that line.
        let north = scene.find("north fire").unwrap().id;
        let done = instantiate(&scene, &prefabs);
        assert_eq!(done.scene.find("north fire").unwrap().id, north);
    }

    #[test]
    fn an_instance_places_and_recolours_without_touching_the_prefab() {
        let scene = parse(
            r#"(entities: [(
                name: "cold fire", model: "", prefab: "campfire",
                material: "stone", body: Static,
                transform: (position: (3.0, 0.0, 0.0), scale: (2.0, 2.0, 2.0)),
            )])"#,
        );
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
        let scene = parse(
            r#"(entities: [(
                name: "fire", model: "", prefab: "campfire",
                children: [(name: "kettle", model: "builtin:sphere")],
            )])"#,
        );
        let done = instantiate(&scene, &with_campfire());
        let names: Vec<&str> = done
            .scene
            .flatten()
            .iter()
            .map(|(e, _)| e.name.as_str())
            .collect();
        assert_eq!(names, ["fire", "ember", "stone", "kettle"]);
        assert_eq!(
            owners(&scene, &done),
            ["fire", "fire", "fire", "kettle"],
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
        let scene = parse(r#"(entities: [(name: "camp", model: "", prefab: "camp")])"#);
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert_eq!(done.scene.flatten().len(), 4, "camp, fire, ember, stone");
        assert_eq!(
            owners(&scene, &done),
            ["camp"; 4],
            "all of it belongs to the one instance in the document"
        );
        let ids = done.scene.ids();
        assert_eq!(
            ids.iter().collect::<HashSet<_>>().len(),
            ids.len(),
            "and nesting did not give two parts one identity"
        );
    }

    #[test]
    fn a_link_to_a_part_holds_in_a_prefab_inside_a_prefab() {
        // A door whose component names its hinge, the door in a shed, the
        // shed in a scene and a shed's lamp naming the door: each link is
        // to a part that is there, whatever the depth.
        let mut prefabs = Prefabs::new();
        prefabs.insert(
            "door",
            ron::from_str(
                r#"(id: "00000000000000d0", name: "door", model: "", components: {
                    "door": (hinge: EntityRef("00000000000000d1")),
                }, children: [(id: "00000000000000d1", name: "hinge", model: "", components: {
                    "hinge": (door: EntityRef("00000000000000d0")),
                })])"#,
            )
            .unwrap(),
        );
        prefabs.insert(
            "shed",
            ron::from_str(
                r#"(id: "00000000000000e0", name: "shed", model: "", children: [
                    (id: "00000000000000e1", name: "door", model: "", prefab: "door"),
                    (id: "00000000000000e2", name: "lamp", model: "", components: {
                        "lamp": (watches: EntityRef("00000000000000e1")),
                    }),
                ])"#,
            )
            .unwrap(),
        );
        let scene = parse(
            r#"(entities: [(id: "00000000000000f0", name: "shed", model: "", prefab: "shed")])"#,
        );
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        let ids: HashSet<String> = done.scene.ids().iter().map(|i| i.to_string()).collect();
        let mut links = Vec::new();
        for (desc, _) in done.scene.flatten() {
            for value in desc.components.values() {
                links.extend(crate::EntityRef::find_in(value.get_ron()));
            }
        }
        assert_eq!(
            links.len(),
            3,
            "the hinge names its door, the prefab's root, too"
        );
        for link in links {
            assert!(ids.contains(&link.to_string()), "{link} is a part: {ids:?}");
        }
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
        let scene = parse(r#"(entities: [(name: "s", model: "", prefab: "snake")])"#);
        let done = instantiate(&scene, &prefabs);
        assert!(!done.problems.is_empty(), "it should complain");
        assert!(done.problems[0].reason.contains("itself"));
        assert!(done.scene.flatten().len() < 64, "and it stopped");
        assert_eq!(owners(&scene, &done), vec!["s"; done.scene.flatten().len()]);
    }

    #[test]
    fn a_missing_prefab_leaves_a_hole_rather_than_refusing_the_scene() {
        let scene = parse(
            r#"(entities: [
                (name: "ground", model: "builtin:plane"),
                (name: "ghost", model: "", prefab: "not_here"),
            ])"#,
        );
        let done = instantiate(&scene, &Prefabs::new());
        assert_eq!(done.problems.len(), 1);
        assert_eq!(done.problems[0].entity_name, "ghost");
        assert_eq!(done.scene.flatten().len(), 2, "the scene still opens");
        assert_eq!(owners(&scene, &done), ["ground", "ghost"]);
    }

    #[test]
    fn a_prefab_round_trips_through_a_file() {
        let dir = std::env::temp_dir().join("runity-prefab-file");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("campfire.{EXTENSION}"));
        let fire = campfire();
        Prefabs::save(&fire, &path).unwrap();

        let (prefabs, problems) = Prefabs::open(&dir).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(prefabs.names(), vec!["campfire"]);
        assert_eq!(prefabs.get("campfire"), Some(&fire), "IDs and all");
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
        let mut scene = Scene {
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
        scene.assign_ids();
        let done = instantiate(&scene, &Prefabs::new());
        assert_eq!(done.scene, scene);
        assert_eq!(owners(&scene, &done), ["rock"]);
    }

    #[test]
    fn an_override_changes_one_part_of_one_instance_and_the_prefab_still_reaches_the_rest() {
        let mut prefabs = Prefabs::new();
        prefabs.insert(
            "fire",
            ron::from_str(
                r#"(id: "c1", name: "fire", model: "m", children: [
                    (id: "c2", name: "ember", model: "m", material: "ember"),
                    (id: "c3", name: "stone", model: "m"),
                ])"#,
            )
            .unwrap(),
        );
        let scene = parse(
            r#"(entities: [
                (id: "a1", name: "one", prefab: "fire",
                 overrides: { "00000000000000c2": (material: "moss", name: "cold ember") }),
                (id: "a2", name: "two", prefab: "fire",
                 overrides: { "00000000000000ff": (name: "ghost") }),
            ])"#,
        );
        let out = instantiate(&scene, &prefabs);
        let one: EntityId = "a1".parse().unwrap();
        let ember = out.scene.get(one.within("c2".parse().unwrap())).unwrap();
        assert_eq!(ember.name, "cold ember");
        assert_eq!(ember.material, MaterialRef::Named("moss".into()));
        assert_eq!(
            ember.model, "m",
            "what it did not say comes from the prefab"
        );
        let two: EntityId = "a2".parse().unwrap();
        assert_eq!(
            out.scene
                .get(two.within("c2".parse().unwrap()))
                .unwrap()
                .material,
            MaterialRef::Named("ember".into()),
            "the other instance is the prefab's"
        );
        assert_eq!(out.problems.len(), 1, "{:?}", out.problems);
        assert!(out.problems[0].reason.contains("does not have"));
        assert_eq!(
            out.parts.get(&one.within("c3".parse().unwrap())),
            Some(&(one, "c3".parse().unwrap())),
            "and every part knows where it came from"
        );
    }

    /// A campfire with mossy stones, a kettle, and no fire of its own
    /// making: the variant's file is a scene line. With the base stone's id.
    fn with_variant() -> (Prefabs, EntityId) {
        let base = campfire();
        let stone = base.children[1].id;
        let mut prefabs = Prefabs::new();
        prefabs.insert("campfire", base);
        let text = format!(
            r#"(
                name: "camp kitchen",
                prefab: "campfire",
                overrides: {{ "{stone}": (material: "moss") }},
                children: [(name: "kettle", model: "builtin:sphere", transform: (position: (0.0, 0.5, 0.0)))],
            )"#
        );
        let mut variant: EntityDesc = ron::from_str(&text).unwrap();
        crate::scene::assign_ids(std::slice::from_mut(&mut variant), &mut HashSet::new());
        prefabs.insert("kitchen", variant);
        (prefabs, stone)
    }

    #[test]
    fn a_variant_is_its_base_with_its_own_changes() {
        let (prefabs, _) = with_variant();
        let scene = parse(
            r#"(entities: [(name: "west", model: "", prefab: "kitchen",
                transform: (position: (5.0, 0.0, 0.0)))])"#,
        );
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        let names: Vec<&str> = done
            .scene
            .flatten()
            .iter()
            .map(|(e, _)| e.name.as_str())
            .collect();
        assert_eq!(names, ["west", "ember", "stone", "kettle"]);
        assert_eq!(
            done.scene.find("stone").unwrap().material,
            MaterialRef::Named("moss".into()),
            "the variant's override"
        );
        assert_eq!(
            done.scene.find("ember").unwrap().material,
            MaterialRef::Named("ember".into()),
            "the base, where the variant said nothing"
        );
        let west = scene.entities[0].id;
        assert!(done
            .scene
            .flatten()
            .iter()
            .all(|(e, _)| done.owner_of(e.id) == Some(west)));
    }

    #[test]
    fn a_scene_overrides_a_variant_by_the_base_part_ids() {
        let (prefabs, stone) = with_variant();
        let scene = parse(&format!(
            r#"(entities: [(name: "west", model: "", prefab: "kitchen",
                overrides: {{ "{stone}": (material: "bark") }})])"#
        ));
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert_eq!(
            done.scene.find("stone").unwrap().material,
            MaterialRef::Named("bark".into()),
            "the instance beats the variant, as the variant beats the base"
        );
        let id = done.scene.find("stone").unwrap().id;
        assert_eq!(
            done.parts.get(&id),
            Some(&(scene.entities[0].id, stone)),
            "addressed as a part of the instance, by its id in the base"
        );
    }

    #[test]
    fn a_variant_of_a_variant_and_a_variant_of_itself() {
        let (mut prefabs, _) = with_variant();
        let mut grand: EntityDesc =
            ron::from_str(r#"(name: "big kitchen", prefab: "kitchen", material: "stone")"#)
                .unwrap();
        crate::scene::assign_ids(std::slice::from_mut(&mut grand), &mut HashSet::new());
        prefabs.insert("big kitchen", grand);
        let scene = parse(r#"(entities: [(name: "camp", model: "", prefab: "big kitchen")])"#);
        let done = instantiate(&scene, &prefabs);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert_eq!(done.scene.flatten().len(), 4);
        assert_eq!(
            done.scene.find("stone").unwrap().material,
            MaterialRef::Named("moss".into())
        );

        let mut prefabs = Prefabs::new();
        let mut ouroboros: EntityDesc = ron::from_str(r#"(name: "loop", prefab: "loop")"#).unwrap();
        crate::scene::assign_ids(std::slice::from_mut(&mut ouroboros), &mut HashSet::new());
        prefabs.insert("loop", ouroboros);
        let scene = parse(r#"(entities: [(name: "x", model: "", prefab: "loop")])"#);
        let done = instantiate(&scene, &prefabs);
        assert!(
            done.problems[0]
                .reason
                .contains("a prefab containing itself?"),
            "{:?}",
            done.problems
        );
    }

    #[test]
    fn a_joint_inside_a_prefab_holds_the_parts_of_its_own_instance() {
        let mut prefabs = Prefabs::new();
        let lamp: EntityDesc = ron::from_str(
            r#"(id: "00000000000000c1", name: "lamp post", model: "builtin:cube", body: Static,
                children: [(id: "00000000000000c2", name: "lamp", model: "builtin:sphere", body: Dynamic,
                    joint: Ball(to: "00000000000000c1", anchor: (0.0, 0.5, 0.0)))])"#,
        )
        .unwrap();
        prefabs.insert("lamp", lamp);
        let scene = parse(
            r#"(entities: [
                (id: "00000000000000a1", name: "west", prefab: "lamp"),
                (id: "00000000000000a2", name: "east", prefab: "lamp"),
            ])"#,
        );
        let done = instantiate(&scene, &prefabs);
        let (west, east): (EntityId, EntityId) = ("a1".parse().unwrap(), "a2".parse().unwrap());
        let part: EntityId = "c2".parse().unwrap();
        let joint_of = |id: EntityId| done.scene.get(id).unwrap().joint.to().unwrap();
        // The lamp hangs from its own post — the instance's root, which
        // keeps the instance's id — not from the file's.
        assert_eq!(joint_of(west.within(part)), west);
        assert_eq!(joint_of(east.within(part)), east);
        assert!(
            done.scene.get(west).is_some(),
            "and that is an entity there"
        );
    }
}
