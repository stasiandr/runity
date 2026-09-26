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
use crate::scene::{EntityDesc, Scene};

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

    /// Read every `.prefab` under a directory, however deep
    /// ([`crate::layout::files`]): a project's root, or any folder in it.
    ///
    /// A file that does not parse is reported and skipped, like a bad asset
    /// in a library: one broken prefab should cost one missing thing, not
    /// every scene that happens to sit beside it.
    pub fn open(directory: impl AsRef<Path>) -> std::io::Result<(Self, Vec<(PathBuf, String)>)> {
        let mut prefabs = Self::new();
        let mut problems = Vec::new();
        if !crate::files::is_dir(directory.as_ref()) {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{}: no such folder", directory.as_ref().display())));
        }
        let paths = crate::layout::files(directory.as_ref(), crate::layout::Kind::Prefab);
        // Read and parsed on every core: a game's thousand prefabs are
        // megabytes of text, and each stands alone.
        let read = crate::jobs::map(&paths, 8, |path| {
            Self::read(path).map(|(name, desc)| (name, desc, crate::asset::sidecar_id(crate::asset::sidecar_of(path))))
        });
        for (path, read) in paths.into_iter().zip(read) {
            match read {
                Ok((name, desc, id)) => {
                    if let Some(id) = id {
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

    /// Read one hand-written prefab file again, as [`Prefabs::open`] reads
    /// each: what changed on disk, without reading all the others.
    pub fn reread(&mut self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        let (name, desc) = Self::read(path)?;
        if let Some(id) = crate::asset::sidecar_id(crate::asset::sidecar_of(path)) {
            self.ids.insert(id, name.clone());
            self.id_of.insert(name.clone(), id);
        }
        self.insert(name, desc);
        Ok(())
    }

    /// Every prefab under `folder` of a game's [`crate::data::Data`] — `""`
    /// for all of it: what [`Prefabs::open`] reads from a directory, read
    /// through the seam.
    pub fn open_from(data: &dyn crate::data::Data, folder: &str) -> (Self, Vec<(String, String)>) {
        let mut prefabs = Self::new();
        let mut problems = Vec::new();
        for path in data.walk(folder).unwrap_or_default() {
            if !path.ends_with(&format!(".{EXTENSION}")) {
                continue;
            }
            let top = path.strip_prefix(folder).unwrap_or(&path).trim_start_matches('/');
            let first = top.split('/').next().unwrap_or_default();
            if top.contains('/') && (first.starts_with('.') || crate::layout::SKIPPED.contains(&first)) {
                continue;
            }
            let read = data
                .read_text(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| {
                    ron::from_str::<EntityDesc>(&text).map_err(|e| format!("{path}:{e}"))
                });
            match read {
                Ok(mut desc) => {
                    crate::scene::derive_ids(std::slice::from_mut(&mut desc), &mut HashSet::new());
                    let name = crate::data::stem(&path);
                    let sidecar = data
                        .read_text(&format!("{path}.scrimport"))
                        .ok()
                        .and_then(|text| crate::asset::sidecar_id_of(&text));
                    if let Some(id) = sidecar {
                        prefabs.ids.insert(id, name.clone());
                        prefabs.id_of.insert(name.clone(), id);
                    }
                    prefabs.insert(name, desc);
                }
                Err(e) => problems.push((path, e)),
            }
        }
        (prefabs, problems)
    }

    /// A project's prefabs: every `.prefab` in it, wherever it lies
    /// (docs/layout.md).
    ///
    /// Found through the project rather than next to whichever scene is
    /// open, so the headless render, the walk-around and the editor all see
    /// the same set, and a scene in `maps/caves/` finds the same campfire
    /// as one in `maps/`. None is not an error: most projects start with
    /// none.
    ///
    /// And the prefabs imports built into `library/`: a glTF or `.blend`
    /// scene is a tree the importer writes there (docs/blender.md), found
    /// by the same names as a hand-written one.
    pub fn of(project: &crate::Project) -> (Self, Vec<(PathBuf, String)>) {
        let (mut prefabs, mut problems) =
            Self::open(project.root()).unwrap_or_else(|_| (Self::new(), Vec::new()));
        problems.extend(prefabs.add_imported(project.library()));
        (prefabs, problems)
    }

    /// Add the prefabs an import built into a library: `<asset id>.prefab`,
    /// named by their root. A hand-written prefab of the same name wins —
    /// it is the one somebody chose to write.
    pub fn add_imported(&mut self, library: impl AsRef<Path>) -> Vec<(PathBuf, String)> {
        let mut problems = Vec::new();
        let Ok(entries) = crate::files::read_dir(library.as_ref()) else {
            return problems;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(EXTENSION))
            .collect();
        paths.sort();
        paths.retain(|path| path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<crate::AssetId>().ok()).is_some());
        let read = crate::jobs::map(&paths, 4, |path| Self::read(path));
        for (path, read) in paths.into_iter().zip(read) {
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<crate::AssetId>().ok());
            let Some(id) = id else { continue };
            match read {
                Ok((_, desc)) => {
                    let name = desc.name.clone();
                    if self.by_name.contains_key(&name) {
                        continue;
                    }
                    self.ids.insert(id, name.clone());
                    self.id_of.insert(name.clone(), id);
                    self.insert(name, desc);
                }
                Err(e) => problems.push((path, e)),
            }
        }
        problems
    }

    /// Read one prefab file, returning its name and what is in it.
    pub fn read(path: impl AsRef<Path>) -> Result<(String, EntityDesc), String> {
        let path = path.as_ref();
        let text =
            crate::files::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
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

    /// Every prefab's tree, to change in memory: what a live preview from
    /// another program does (docs/blender.md) until the file is saved.
    pub fn trees_mut(&mut self) -> impl Iterator<Item = &mut EntityDesc> {
        self.by_name.values_mut()
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
    instantiate_with(scene, prefabs, |_| {})
}

/// [`instantiate`], with what the modules grow on an expanded scene —
/// copies set along a spline — added before who owns what is read off it:
/// what grows under a line is that line's.
pub fn instantiate_with(
    scene: &Scene,
    prefabs: &Prefabs,
    grow: impl Fn(&mut [EntityDesc]),
) -> Instanced {
    let mut problems = Vec::new();
    let mut parts = HashMap::new();
    let mut entities: Vec<EntityDesc> = scene
        .entities
        .iter()
        .map(|desc| expand(desc, None, prefabs, 0, &mut problems, &mut parts))
        .collect();
    nested_links(&mut entities, &parts);
    grow(&mut entities);
    let expanded = Scene {
        parts: scene.parts.clone(),
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

/// A part's id as its prefab alone would give it: the nested instance it
/// is in, within the id that instance's prefab alone gives it — and so on
/// down, `shelf.within(screen.within(part))`.
fn local_key(
    parts: &HashMap<EntityId, (EntityId, EntityId)>,
    instance: EntityId,
    id: EntityId,
) -> Option<EntityId> {
    let mut chain = Vec::new();
    let mut at = id;
    while at != instance {
        let (within, own) = *parts.get(&at)?;
        chain.push(own);
        at = within;
    }
    let mut out = chain.first().copied()?;
    for outer in &chain[1..] {
        out = outer.within(out);
    }
    Some(out)
}

fn find_by_key<'a>(
    entities: &'a mut [EntityDesc],
    parts: &HashMap<EntityId, (EntityId, EntityId)>,
    instance: EntityId,
    key: EntityId,
) -> Option<&'a mut EntityDesc> {
    for entity in entities {
        if local_key(parts, instance, entity.id) == Some(key) {
            return Some(entity);
        }
        if let Some(found) = find_by_key(&mut entity.children, parts, instance, key) {
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
        let grown = expand(child, scope, prefabs, depth, problems, parts);
        // One the scene hung on a part of the prefab goes under that part.
        let under = child
            .in_part
            .filter(|_| !desc.prefab.is_empty())
            .and_then(|key| {
                let scoped = id.within(key);
                // Looked for twice: the borrow checker cannot yet see that the
                // first look's borrow ends when it finds nothing.
                if find_mut(&mut expanded.children, scoped).is_some() {
                    find_mut(&mut expanded.children, scoped)
                } else {
                    find_by_key(&mut expanded.children, parts, id, key)
                }
            });
        match under {
            Some(part) => part.children.push(grown),
            None => {
                if let Some(key) = child
                    .in_part
                    .filter(|k| *k != id && !desc.prefab.is_empty())
                {
                    let root_key = prefabs.find(&desc.prefab).map(|(_, t)| t.id);
                    if Some(key) != root_key {
                        problems.push(Problem {
                            entity_name: child.name.clone(),
                            prefab: desc.prefab.to_string(),
                            reason: format!(
                                "hung on part {key}, which the prefab does not have (any more?): under the instance instead"
                            ),
                        });
                    }
                }
                expanded.children.push(grown);
            }
        }
    }
    expanded
}

/// The entities a module's field names: every entity ID written in its
/// text, a quoted sixteen hex digits — a joint's `to: "5f1c09aa3e7b2d10"`.
/// (An asset's ID is thirty-two, and is not one.) The core scopes and
/// follows these as it does a component's `EntityRef`, without knowing
/// which module's field it is.
pub fn links_in(text: &str) -> Vec<EntityId> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 18 <= bytes.len() {
        if bytes[i] == b'"'
            && bytes[i + 17] == b'"'
            && bytes[i + 1..i + 17].iter().all(u8::is_ascii_hexdigit)
            && (i + 18 == bytes.len() || !bytes[i + 18].is_ascii_hexdigit())
        {
            if let Ok(id) = text[i + 1..i + 17].parse::<EntityId>() {
                out.push(id);
            }
            i += 18;
        } else {
            i += 1;
        }
    }
    out
}

/// Links to a part of a prefab inside a prefab pointed at that part.
///
/// A file names such a part by the key [`local_key`] gives it —
/// `screen.within(label)` for a label in the screen placed in a machine —
/// and scoping that link to the machine's instance makes
/// `machine.within(screen.within(label))`; but the label itself, expanded,
/// is `machine.within(screen).within(label)`: the screen's instance first,
/// then the label in it. `within` does not compose, so every link that
/// names a nested part is put right here, once, after expansion — for the
/// links a scene writes as for those a prefab does, at any depth. A link
/// to a part of the instance's own prefab is already right and is left.
fn nested_links(entities: &mut [EntityDesc], parts: &HashMap<EntityId, (EntityId, EntityId)>) {
    let mut named: HashMap<EntityId, EntityId> = HashMap::new();
    for &id in parts.keys() {
        let Some(&(mut instance, own)) = parts.get(&id) else {
            continue;
        };
        let mut key = own;
        // Each instance further up names it by its own key for it.
        while let Some(&(outer, outer_own)) = parts.get(&instance) {
            key = outer_own.within(key);
            instance = outer;
            let written = instance.within(key);
            if written != id {
                named.insert(written, id);
            }
        }
    }
    if named.is_empty() {
        return;
    }
    fn walk(entities: &mut [EntityDesc], named: &HashMap<EntityId, EntityId>) {
        for desc in entities {
            map_part_links(&mut desc.parts, |id| named.get(&id).copied().unwrap_or(id));
            for value in desc.components.values_mut() {
                let text = value.get_ron();
                let links = crate::EntityRef::find_in(text);
                if !links.iter().any(|l| named.contains_key(l)) {
                    continue;
                }
                let mut out = text.to_string();
                for id in links {
                    if let Some(to) = named.get(&id) {
                        out = out.replace(&format!("EntityRef(\"{id}\")"), &format!("EntityRef(\"{to}\")"));
                    }
                }
                if let Ok(raw) = ron::value::RawValue::from_boxed_ron(out.into_boxed_str()) {
                    *value = raw;
                }
            }
            walk(&mut desc.children, named);
        }
    }
    walk(entities, &named);
}

/// A line's module fields with every entity they name put through `map`.
fn map_part_links(parts: &mut crate::parts::Parts, map: impl Fn(EntityId) -> EntityId) {
    let named: Vec<(String, String)> = parts
        .iter()
        .filter(|(_, text)| !links_in(text).is_empty())
        .map(|(name, text)| {
            let mut out = text.to_string();
            for id in links_in(text) {
                out = out.replace(&format!("\"{id}\""), &format!("\"{}\"", map(id)));
            }
            (name.to_string(), out)
        })
        .collect();
    for (name, text) in named {
        let _ = parts.set_raw(&name, &text);
    }
}

/// Point every link to `from` in `desc` and under it at `to` instead.
fn relink(desc: &mut EntityDesc, from: EntityId, to: EntityId) {
    map_part_links(&mut desc.parts, |id| if id == from { to } else { id });
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

/// A line with the links it writes — its components' `EntityRef`s and the
/// entities its module fields name, a joint's other end — put in
/// `instance`'s scope; its children as they are.
fn scoped_links(desc: &EntityDesc, instance: EntityId) -> EntityDesc {
    let mut out = desc.clone();
    map_part_links(&mut out.parts, |id| {
        if id.is_unassigned() {
            id
        } else {
            instance.within(id)
        }
    });
    // Its components, and those its overrides give its prefab's parts:
    // both written in this file.
    let overridden = out
        .overrides
        .values_mut()
        .flat_map(|o| o.components.values_mut());
    for value in out.components.values_mut().chain(overridden) {
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
    // Every module field the instance's line says — a material, a body, a
    // joint naming something in the scene, a light — on top of the root's.
    // Not its model: an instance's look is its prefab's.
    for (name, text) in desc.parts.iter() {
        if name != "model" {
            let _ = root.parts.set_raw(name, text);
        }
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
        // The prefab's root is this instance itself.
        if *part == template.id {
            change.apply(&mut root);
            root.id = id;
            continue;
        }
        // A part of a prefab inside the prefab is named by the id it has
        // when the prefab is expanded alone: its instance's within its own.
        let found = match find_mut(&mut root.children, scoped) {
            Some(target) => Some(target),
            None => find_by_key(&mut root.children, parts, id, *part),
        };
        match found {
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
