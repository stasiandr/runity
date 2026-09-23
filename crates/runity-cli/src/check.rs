//! `runity check`: what in a project does not resolve, said so it can be
//! fixed.
//!
//! The engine is forgiving at run time on purpose — a model nobody has is
//! an entity with nothing drawn, a material typo is grey — because a scene
//! that refuses to open is worse than one with a visible mistake. The price
//! is that a mistake can sit unnoticed. This is where it gets noticed: on
//! the command line, in CI, and by an agent that has just written a scene
//! and wants to know whether it holds together (DNA, postulate 5: errors an
//! agent can act on).
//!
//! Every finding names the file, the entity (by name and ID), what is wrong,
//! and what would fix it, closest name included.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use runity::scene::MaterialRef;
use runity::{EntityDesc, EntityId, Project, Scene};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Something that will not be what the file says: a model not drawn, a
    /// colour gone grey, an instance of nothing.
    Error,
    /// Something that works today and will cost later: a sidecar that does
    /// not match its source, an entity without an ID.
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    /// Project-relative, forward slashes.
    pub file: String,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{severity}: {}: {}", self.file, self.message)
    }
}

/// Every name a scene can use, and where each comes from.
struct Names {
    /// Model name → the source files that would build an asset of that name.
    models: HashMap<String, Vec<String>>,
    materials: HashMap<String, Vec<String>>,
    prefabs: HashSet<String>,
    /// The game's components, from `src/components/`; `None` when the
    /// project has no such folder and only its code knows.
    components: Option<Vec<String>>,
    /// The collision layers, from `layers.ron`.
    layers: runity::layers::Layers,
}

const MODEL_SOURCES: [&str; 4] = ["gltf", "glb", "obj", "rterrain"];

/// Look the whole project over.
pub fn check(project: &Project) -> Vec<Finding> {
    let mut out = Vec::new();
    let names = names(project, &mut out);

    let prefab_dir = project.prefabs();
    for path in files(&prefab_dir, "prefab") {
        let file = relative(project, &path);
        match runity::Prefabs::read(&path) {
            Ok(_) => {
                // Read raw as well: `read` fills missing IDs in, which is
                // what is being checked.
                if let Some(desc) = parse::<EntityDesc>(&path, &file, &mut out) {
                    check_entities(std::slice::from_ref(&desc), &file, &names, &mut out);
                }
            }
            Err(e) => out.push(error(&file, e)),
        }
    }

    for path in files(&project.scenes(), "ron") {
        let file = relative(project, &path);
        if let Some(scene) = parse::<Scene>(&path, &file, &mut out) {
            check_entities(&scene.entities, &file, &names, &mut out);
        }
    }

    expansion(project, &mut out);

    let input = project.root().join(runity::project::INPUT);
    if input.is_file() {
        if let Err(e) = runity::Actions::load(&input) {
            out.push(error(runity::project::INPUT, e));
        }
    }

    let mut keys: Vec<String> = Vec::new();
    for path in files(&project.root().join(runity::project::UI), "ron") {
        let file = relative(project, &path);
        if let Some(layout) = parse::<runity::screen::Layout>(&path, &file, &mut out) {
            for problem in layout.problems() {
                out.push(error(&file, problem));
            }
            keys.extend(runity::screen::Screen::keys(&layout));
        }
    }
    let strings = project.root().join(runity::strings::DIR);
    let mut tables = Vec::new();
    for (language, path) in runity::strings::tables(&strings) {
        let file = relative(project, &path);
        if let Some(table) = parse::<runity::strings::Table>(&path, &file, &mut out) {
            tables.push((language, table));
        }
    }
    if tables.is_empty() && !keys.is_empty() {
        out.push(error(
            runity::strings::DIR,
            format!(
                "screens ask for `@{}` and there is no language in strings/ to say it",
                keys[0]
            ),
        ));
    }
    for missing in runity::strings::missing(&tables, &keys) {
        let (language, rest) = missing.split_once(": ").unwrap_or(("", &missing));
        out.push(error(
            &format!("{}/{language}.ron", runity::strings::DIR),
            rest,
        ));
    }

    // The game starts on a scene and speaks a language that are there.
    let game = &project.manifest().game;
    let scenes = project.scene_names();
    if !scenes.contains(&game.start_scene) {
        out.push(error(
            runity::project::FILE,
            format!(
                "start_scene `{}` is not in scenes/{}",
                game.start_scene,
                suggest(&game.start_scene, scenes.iter().map(String::as_str))
            ),
        ));
    }
    let languages: Vec<String> = tables.iter().map(|(l, _)| l.clone()).collect();
    if !languages.is_empty() && !languages.contains(&game.language) {
        out.push(error(
            runity::project::FILE,
            format!(
                "language `{}` has no strings/{}.ron{}",
                game.language,
                game.language,
                suggest(&game.language, languages.iter().map(String::as_str))
            ),
        ));
    }
    if game.steps_per_second == 0 || game.width == 0 || game.height == 0 {
        out.push(error(
            runity::project::FILE,
            "steps_per_second, width and height are more than 0",
        ));
    }

    for path in files(&project.root().join(runity::project::TUNING), "ron") {
        let file = relative(project, &path);
        let _: Option<ron::Value> = parse(&path, &file, &mut out);
    }

    check_sidecars(project, &mut out);
    out.sort_by(|a, b| (a.severity, &a.file).cmp(&(b.severity, &b.file)));
    out
}

/// What only expanding prefabs finds: an override for a part the prefab no
/// longer has, and a variant that is, some levels down, a variant of
/// itself. A missing prefab is not repeated here; the line naming it was
/// already reported.
///
/// Prefab files first, each placed once on its own, so a variant's stale
/// override is reported against the variant — and then not again for every
/// scene that places it.
fn expansion(project: &Project, out: &mut Vec<Finding>) {
    let (prefabs, _) = runity::Prefabs::of(project);
    let mut said: HashSet<String> = HashSet::new();
    let mut report = |file: &str, done: runity::Instanced, out: &mut Vec<Finding>| {
        for problem in done.problems {
            if problem.reason == "no prefab by that name" {
                continue;
            }
            let message = format!(
                "`{}` (an instance of `{}`): {}",
                problem.entity_name, problem.prefab, problem.reason
            );
            if said.insert(message.clone()) {
                out.push(error(file, message));
            }
        }
    };
    for path in files(&project.prefabs(), "prefab") {
        let file = relative(project, &path);
        let Some(name) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let alone = Scene {
            entities: vec![EntityDesc {
                name: name.clone(),
                prefab: name,
                ..EntityDesc::default()
            }],
            ..Scene::default()
        };
        report(&file, runity::instantiate(&alone, &prefabs), out);
    }
    for path in files(&project.scenes(), "ron") {
        let file = relative(project, &path);
        if let Ok(scene) = Scene::load(&path) {
            report(&file, runity::instantiate(&scene, &prefabs), out);
        }
    }
}

fn names(project: &Project, out: &mut Vec<Finding>) -> Names {
    let mut models: HashMap<String, Vec<String>> = HashMap::new();
    let mut materials: HashMap<String, Vec<String>> = HashMap::new();
    for root in [project.assets(), project.materials()] {
        runity_import::walk(&root, &mut |path| {
            let (Some(stem), Some(extension)) = (
                path.file_stem().map(|s| s.to_string_lossy().into_owned()),
                path.extension().map(|e| e.to_string_lossy().to_lowercase()),
            ) else {
                return;
            };
            let file = relative(project, path);
            if MODEL_SOURCES.contains(&extension.as_str()) {
                models.entry(stem).or_default().push(file);
            } else if extension == "rmat" {
                materials.entry(stem).or_default().push(file);
            }
        });
    }
    // The library knows an asset by its source's file name, so two sources
    // with one name in different folders are one name with two meanings,
    // and whichever was built last wins.
    for (kind, map) in [("model", &models), ("material", &materials)] {
        let mut clashes: Vec<_> = map.iter().filter(|(_, files)| files.len() > 1).collect();
        clashes.sort();
        for (name, files) in clashes {
            let mut files = files.clone();
            files.sort();
            out.push(error(
                &files[0],
                format!(
                    "{kind} name `{name}` is used by {} — scenes cannot tell them apart; rename all but one",
                    files.join(", ")
                ),
            ));
        }
    }
    let prefabs = files(&project.prefabs(), "prefab")
        .iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    Names {
        models,
        materials,
        prefabs,
        components: project.component_names(),
        layers: match runity::layers::Layers::of(project) {
            Ok(layers) => layers,
            Err(e) => {
                out.push(error(runity::layers::FILE, e));
                runity::layers::Layers::default()
            }
        },
    }
}

fn check_entities(entities: &[EntityDesc], file: &str, names: &Names, out: &mut Vec<Finding>) {
    // Every id in the file, for joints to be checked against: a joint names
    // a body in the same file.
    let mut all: HashSet<EntityId> = HashSet::new();
    let mut walk: Vec<&EntityDesc> = entities.iter().collect();
    while let Some(e) = walk.pop() {
        all.insert(e.id);
        walk.extend(e.children.iter());
    }
    let mut seen: HashSet<EntityId> = HashSet::new();
    let mut unnamed = 0;
    let mut stack: Vec<&EntityDesc> = entities.iter().rev().collect();
    while let Some(entity) = stack.pop() {
        stack.extend(entity.children.iter().rev());
        let who = if entity.id.is_unassigned() {
            unnamed += 1;
            format!("`{}`", entity.name)
        } else {
            if !seen.insert(entity.id) {
                out.push(error(
                    file,
                    format!(
                        "`{}`: id {} is used twice — a copied block? delete the id line of the copy and it gets a new one",
                        entity.name, entity.id
                    ),
                ));
            }
            format!("`{}` ({})", entity.name, entity.id)
        };

        if !entity.prefab.is_empty() {
            if !names.prefabs.contains(&entity.prefab) {
                out.push(error(
                    file,
                    format!(
                        "{who}: no prefab named `{}` in prefabs/{}",
                        entity.prefab,
                        suggest(&entity.prefab, names.prefabs.iter().map(String::as_str))
                    ),
                ));
            }
        } else if !entity.model.is_empty() {
            check_model(&entity.model, &who, file, names, out);
        }
        if let MaterialRef::Named(name) = &entity.material {
            check_material(name, &who, file, names, out);
        }
        let layers = std::iter::once(&entity.layer)
            .chain(entity.overrides.values().filter_map(|o| o.layer.as_ref()));
        for layer in layers.filter(|l| !l.is_empty()) {
            if names.layers.index(layer).is_none() {
                out.push(error(
                    file,
                    format!(
                        "{who}: no layer `{layer}` in layers.ron{}",
                        suggest(layer, names.layers.layers.iter().map(String::as_str))
                    ),
                ));
            }
        }
        if let Some(to) = entity.joint.to().filter(|to| !to.is_unassigned()) {
            if !all.contains(&to) {
                out.push(error(
                    file,
                    format!("{who}: its joint hangs from {to}, which is not in this file — the body it holds on to has to be"),
                ));
            }
        }
        if let Some(known) = &names.components {
            let used = entity
                .components
                .keys()
                .chain(entity.overrides.values().flat_map(|o| o.components.keys()));
            for component in used {
                if !known.contains(component) {
                    out.push(error(
                        file,
                        format!(
                            "{who}: no component `{component}` — a component is a file in src/components/ named what scenes call it{}",
                            suggest(component, known.iter().map(String::as_str))
                        ),
                    ));
                }
            }
        }
    }
    if unnamed > 0 {
        out.push(Finding {
            severity: Severity::Warning,
            file: file.to_string(),
            message: format!(
                "{unnamed} entities have no id — they get one on load, and it is written on the next save from the editor"
            ),
        });
    }
}

fn check_model(model: &str, who: &str, file: &str, names: &Names, out: &mut Vec<Finding>) {
    if model.starts_with("builtin:") {
        if runity::builtin::by_name(model).is_none() {
            out.push(error(
                file,
                format!(
                    "{who}: no builtin model `{model}`{}",
                    suggest(model, runity::builtin::NAMES.iter().copied())
                ),
            ));
        }
        return;
    }
    if !names.models.contains_key(model) {
        let known = names
            .models
            .keys()
            .map(String::as_str)
            .chain(runity::builtin::NAMES.iter().copied());
        out.push(error(
            file,
            format!(
                "{who}: no model named `{model}` — a model is a .gltf, .glb, .obj or .rterrain in assets/, named by its file name without the extension{}",
                suggest(model, known)
            ),
        ));
    }
}

fn check_material(name: &str, who: &str, file: &str, names: &Names, out: &mut Vec<Finding>) {
    let builtins = runity::material::builtin::NAMES.iter().copied();
    let found = match name.strip_prefix("builtin:") {
        Some(builtin) => runity::material::builtin::by_name(builtin).is_some(),
        None => {
            names.materials.contains_key(name) || runity::material::builtin::by_name(name).is_some()
        }
    };
    if !found {
        let known = names.materials.keys().map(String::as_str).chain(builtins);
        out.push(error(
            file,
            format!(
                "{who}: no material named `{name}`, so it draws grey — add materials/{name}.rmat or use an existing one{}",
                suggest(name, known)
            ),
        ));
    }
}

/// Every source should have a sidecar that describes it: otherwise the
/// next `runity sync` writes one, and a clone that did not run it builds
/// something else.
fn check_sidecars(project: &Project, out: &mut Vec<Finding>) {
    for root in [project.assets(), project.materials()] {
        runity_import::walk(&root, &mut |source| {
            if !runity_import::importable(source) {
                return;
            }
            let file = relative(project, source);
            let sidecar = runity_import::sidecar_for(source);
            let stale = match runity_import::ImportSettings::load(&sidecar) {
                Err(_) => Some("has no .rimport"),
                Ok(settings) if settings.source != file => {
                    Some("has a .rimport naming another file")
                }
                Ok(settings)
                    if runity_import::content_hash(source).ok().as_ref()
                        != Some(&settings.hash) =>
                {
                    Some("changed since its .rimport was written")
                }
                Ok(_) => None,
            };
            if let Some(why) = stale {
                out.push(Finding {
                    severity: Severity::Warning,
                    file,
                    message: format!("{why} — run `runity sync` and commit the .rimport"),
                });
            }
        });
    }
}

fn parse<T: serde::de::DeserializeOwned>(
    path: &Path,
    file: &str,
    out: &mut Vec<Finding>,
) -> Option<T> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            out.push(error(file, e));
            return None;
        }
    };
    match ron::from_str(&text) {
        Ok(value) => Some(value),
        Err(e) => {
            // ron says where: line, column, and what it expected.
            out.push(error(file, e));
            None
        }
    }
}

fn error(file: &str, message: impl fmt::Display) -> Finding {
    Finding {
        severity: Severity::Error,
        file: file.to_string(),
        message: message.to_string(),
    }
}

fn relative(project: &Project, path: &Path) -> String {
    project
        .relative(path)
        .unwrap_or_else(|| path.display().to_string())
}

/// Files with this extension anywhere under `root`, sorted.
fn files(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    runity_import::walk(root, &mut |path| {
        if path.extension().and_then(|e| e.to_str()) == Some(extension) {
            found.push(path.to_path_buf());
        }
    });
    found.sort();
    found
}

/// ` — did you mean `x`?` for the closest known name, or nothing when none
/// is close enough to be a typo.
fn suggest<'a>(wanted: &str, known: impl Iterator<Item = &'a str>) -> String {
    runity::spelling::closest(wanted, known)
        .map(|name| format!(" — did you mean `{name}`?"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typo_is_suggested_and_a_stranger_is_not() {
        let known = ["rock", "pine_large", "campfire"];
        assert_eq!(
            suggest("rok", known.iter().copied()),
            " — did you mean `rock`?"
        );
        assert_eq!(
            suggest("pine_lrage", known.iter().copied()),
            " — did you mean `pine_large`?"
        );
        assert_eq!(suggest("helicopter", known.iter().copied()), "");
    }
}
