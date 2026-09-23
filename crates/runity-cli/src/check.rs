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

    let input = project.root().join(runity::project::INPUT);
    if input.is_file() {
        if let Err(e) = runity::Actions::load(&input) {
            out.push(error(runity::project::INPUT, e));
        }
    }

    check_sidecars(project, &mut out);
    out.sort_by(|a, b| (a.severity, &a.file).cmp(&(b.severity, &b.file)));
    out
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
    }
}

fn check_entities(entities: &[EntityDesc], file: &str, names: &Names, out: &mut Vec<Finding>) {
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
