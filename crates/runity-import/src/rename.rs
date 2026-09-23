//! Renaming or moving an asset source without breaking what names it.
//!
//! In Unity a rename in the Project window keeps every reference, because
//! scenes hold GUIDs. Scenes here hold names a person reads — `stone`,
//! `campfire`, `pine_large` (DNA, postulate 2) — so a rename is an
//! operation: the file moves, its `.rimport` follows with its settings and
//! its asset ID, and every scene and prefab line that named the old stem is
//! rewritten to the new one, keeping the rest of each file's text as it was.
//! The diff is the move plus one changed field per line that pointed at the
//! file, which is also the answer to "who used this?".
//!
//! Two people at once: one renames `stone` to `granite` while the other
//! adds a rock with `material: "stone"`. Git merges both cleanly and the new
//! rock names a material that is gone — which `runity check` reports as an
//! error with the file and the entity, before anyone sees a grey rock in
//! the game. A rename that would make a name mean
//! something else — onto a name another file has, or one lines already use
//! (a builtin a project has not shadowed yet) — is refused rather than done.

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use runity::refs::{self, AssetRef, Use};
use runity::{Prefabs, Project, Scene};

use crate::{asset_for, importable, sidecar_for, sync, terrain, walk, ImportSettings, Reimported};

/// What a rename did.
#[derive(Debug)]
pub struct Renamed {
    /// Project-relative, with forward slashes.
    pub from: String,
    pub to: String,
    /// The name scenes used, and the one they use now. `None` when the
    /// stem did not change (a move between folders), or when scenes do not
    /// name this kind of file (a texture or a sound is named from code).
    pub reference: Option<(AssetRef, AssetRef)>,
    /// Every file rewritten, project-relative, with how many references in
    /// it changed. Terrains whose heightmap moved are here too.
    pub rewritten: Vec<(String, usize)>,
    /// What bringing the library up to date afterwards did.
    pub synced: Vec<Reimported>,
}

/// One place in the project that names an asset.
#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    /// The scene or prefab, project-relative.
    pub file: String,
    pub at: Use,
}

impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: `{}` ({}) {}",
            self.file, self.at.name, self.at.entity, self.at.field
        )
    }
}

/// What a source file is, as far as names in scenes go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Model,
    Material,
    Prefab,
    /// Imported, but not named by scenes: a texture, a sound, a heightmap.
    Other,
}

const MODEL_EXTENSIONS: [&str; 4] = ["gltf", "glb", "obj", "rterrain"];

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn kind(project: &Project, path: &Path) -> Result<Kind> {
    let ext = extension(path);
    let inside = |dir: PathBuf| normalize(path).starts_with(normalize(&dir));
    if inside(project.prefabs()) && ext == "prefab" {
        ensure!(
            normalize(path.parent().unwrap_or(Path::new(""))) == normalize(&project.prefabs()),
            "{}: prefabs sit directly in prefabs/, which is where scenes look for them",
            shown(project, path)
        );
        return Ok(Kind::Prefab);
    }
    if inside(project.materials()) && ext == "rmat" {
        return Ok(Kind::Material);
    }
    if inside(project.assets()) && importable(path) {
        return Ok(if MODEL_EXTENSIONS.contains(&ext.as_str()) {
            Kind::Model
        } else {
            Kind::Other
        });
    }
    bail!(
        "{} is not an asset source: renaming works on models, textures and sounds in assets/, .rmat in materials/ and .prefab in prefabs/",
        shown(project, path)
    )
}

/// The name scenes use for a file, if they name it at all.
pub fn reference(project: &Project, file: &Path) -> Result<Option<AssetRef>> {
    let file = absolute(project, file);
    Ok(match kind(project, &file)? {
        Kind::Model => Some(AssetRef::Model(stem(&file))),
        Kind::Material => Some(AssetRef::Material(stem(&file))),
        Kind::Prefab => Some(AssetRef::Prefab(stem(&file))),
        Kind::Other => None,
    })
}

/// Every scene and prefab line that names `what`, in file order.
///
/// A scene or prefab that does not load is an error: saying "nothing uses
/// this" about a project half of which could not be read would be a lie.
pub fn usages_of(project: &Project, what: &AssetRef) -> Result<Vec<Usage>> {
    let documents = Documents::read(project)?;
    let mut out = Vec::new();
    for (path, scene) in &documents.scenes {
        for at in refs::uses_in_scene(scene, what) {
            out.push(Usage {
                file: shown(project, path),
                at,
            });
        }
    }
    for (path, prefab) in &documents.prefabs {
        for at in refs::uses(std::slice::from_ref(prefab), what) {
            out.push(Usage {
                file: shown(project, path),
                at,
            });
        }
    }
    Ok(out)
}

/// Every place that names `file`. Empty for a file scenes do not name.
pub fn usages(project: &Project, file: &Path) -> Result<Vec<Usage>> {
    match reference(project, file)? {
        Some(what) => usages_of(project, &what),
        None => Ok(Vec::new()),
    }
}

/// Move `from` to `to` — both relative to the project root, or absolute —
/// and point everything that named it at the new name.
///
/// Everything is checked before anything is touched: every scene and
/// prefab must load, the target must be free, and the new name must not
/// already mean something. Then the file and its sidecar move, the
/// references are rewritten, and the library is synced.
pub fn rename(project: &Project, from: &Path, to: &Path) -> Result<Renamed> {
    let from = absolute(project, from);
    let to = absolute(project, to);
    ensure!(
        from.is_file(),
        "{} is not there to rename",
        shown(project, &from)
    );
    ensure!(
        !to.exists(),
        "{} is already there; renaming onto it would lose one of the two",
        shown(project, &to)
    );
    let kind_from = kind(project, &from)?;
    let kind_to =
        kind(project, &to).with_context(|| format!("{} as the new name", shown(project, &to)))?;
    ensure!(
        kind_from == kind_to && extension(&from) == extension(&to),
        "{} to {}: a different extension or folder kind makes it another asset, not a rename",
        shown(project, &from),
        shown(project, &to)
    );

    let documents = Documents::read(project)?;
    let (old, new) = (stem(&from), stem(&to));
    let reference = match kind_from {
        Kind::Model if old != new => Some(AssetRef::Model(old.clone())),
        Kind::Material if old != new => Some(AssetRef::Material(old.clone())),
        Kind::Prefab if old != new => Some(AssetRef::Prefab(old.clone())),
        _ => None,
    };
    if let Some(what) = &reference {
        let target = what.renamed(new.clone());
        if let Some(other) = same_stem(project, kind_from, &new, &from) {
            bail!(
                "{} is already called `{new}`; two {}s with one name would be one too many",
                shown(project, &other),
                match kind_from {
                    Kind::Model => "model",
                    Kind::Material => "material",
                    _ => "prefab",
                }
            );
        }
        let taken = documents.uses(project, &target);
        if let Some(first) = taken.first() {
            bail!(
                "{} is already named by {} line(s) — first {first} — and they would start meaning this file",
                target,
                taken.len()
            );
        }
    }

    // Terrains whose heightmap this is, or this terrain's own heightmap if
    // it changes folder: the path is relative to the terrain, so either
    // move changes the text.
    let mut heightmaps: Vec<(PathBuf, String)> = Vec::new();
    let from_dir = normalize(from.parent().unwrap_or(Path::new("")));
    let to_dir = normalize(to.parent().unwrap_or(Path::new("")));
    if kind_from == Kind::Other {
        walk(&project.assets(), &mut |path| {
            if extension(path) != "rterrain" {
                return;
            }
            let Some(name) = terrain::heightmap(path) else {
                return;
            };
            let dir = path.parent().unwrap_or(Path::new(""));
            if normalize(&dir.join(&name)) == normalize(&from) {
                heightmaps.push((path.to_path_buf(), relative_between(dir, &to)));
            }
        });
    } else if extension(&from) == "rterrain" && from_dir != to_dir {
        if let Some(name) = terrain::heightmap(&from) {
            let image = normalize(&from_dir.join(name));
            heightmaps.push((to.clone(), relative_between(&to_dir, &image)));
        }
    }

    // The file, and its sidecar with it.
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&from, &to).with_context(|| {
        format!(
            "moving {} to {}",
            shown(project, &from),
            shown(project, &to)
        )
    })?;
    let old_sidecar = sidecar_for(&from);
    if old_sidecar.is_file() {
        let mut settings = ImportSettings::load(&old_sidecar)?;
        settings.source = shown(project, &to);
        settings.save(sidecar_for(&to))?;
        std::fs::remove_file(&old_sidecar)?;
        // Built under the old file name; left, it would be a second copy of
        // the same ID in the library.
        let _ = std::fs::remove_file(asset_for(&from, &project.library()));
    }

    let mut rewritten = Vec::new();
    if let Some(what) = &reference {
        for (path, mut scene) in documents.scenes {
            let count = refs::rewrite_scene(&mut scene, what, &new);
            if count > 0 {
                scene.save(&path)?;
                rewritten.push((shown(project, &path), count));
            }
        }
        for (path, mut prefab) in documents.prefabs {
            let count = refs::rewrite(std::slice::from_mut(&mut prefab), what, &new);
            if count > 0 {
                Prefabs::save(&prefab, &path).map_err(anyhow::Error::msg)?;
                rewritten.push((shown(project, &path), count));
            }
        }
    }
    for (path, heightmap) in heightmaps {
        terrain::set_heightmap(&path, &heightmap)?;
        rewritten.push((shown(project, &path), 1));
    }

    Ok(Renamed {
        from: shown(project, &from),
        to: shown(project, &to),
        reference: reference.map(|what| {
            let now = what.renamed(new);
            (what, now)
        }),
        rewritten,
        synced: sync(project),
    })
}

/// Every scene and prefab in a project, read.
struct Documents {
    scenes: Vec<(PathBuf, Scene)>,
    prefabs: Vec<(PathBuf, runity::EntityDesc)>,
}

impl Documents {
    fn read(project: &Project) -> Result<Self> {
        let mut scene_paths = Vec::new();
        walk(&project.scenes(), &mut |path| {
            if extension(path) == "ron" {
                scene_paths.push(path.to_path_buf());
            }
        });
        scene_paths.sort();
        let mut prefab_paths = Vec::new();
        if let Ok(entries) = std::fs::read_dir(project.prefabs()) {
            for entry in entries.flatten() {
                if extension(&entry.path()) == "prefab" {
                    prefab_paths.push(entry.path());
                }
            }
        }
        prefab_paths.sort();

        let mut scenes = Vec::new();
        for path in scene_paths {
            let scene = Scene::load(&path).with_context(|| {
                format!(
                    "{} does not load, so what it names cannot be known; fix it first",
                    shown(project, &path)
                )
            })?;
            scenes.push((path, scene));
        }
        let mut prefabs = Vec::new();
        for path in prefab_paths {
            let (_, desc) = Prefabs::read(&path).map_err(|e| {
                anyhow::anyhow!(
                    "{e}\n{} does not load, so what it names cannot be known; fix it first",
                    shown(project, &path)
                )
            })?;
            prefabs.push((path, desc));
        }
        Ok(Self { scenes, prefabs })
    }

    fn uses(&self, project: &Project, what: &AssetRef) -> Vec<Usage> {
        let scenes = self.scenes.iter().flat_map(|(path, scene)| {
            refs::uses_in_scene(scene, what)
                .into_iter()
                .map(move |at| (path, at))
        });
        let prefabs = self.prefabs.iter().flat_map(|(path, prefab)| {
            refs::uses(std::slice::from_ref(prefab), what)
                .into_iter()
                .map(move |at| (path, at))
        });
        scenes
            .chain(prefabs)
            .map(|(path, at)| Usage {
                file: shown(project, path),
                at,
            })
            .collect()
    }
}

/// Another source of the same kind already called `name`.
fn same_stem(project: &Project, kind: Kind, name: &str, except: &Path) -> Option<PathBuf> {
    let root = match kind {
        Kind::Model => project.assets(),
        Kind::Material => project.materials(),
        _ => project.prefabs(),
    };
    let mut found = None;
    walk(&root, &mut |path| {
        let matches = match kind {
            Kind::Model => MODEL_EXTENSIONS.contains(&extension(path).as_str()),
            Kind::Material => extension(path) == "rmat",
            _ => extension(path) == "prefab",
        };
        if found.is_none() && matches && stem(path) == name && normalize(path) != normalize(except)
        {
            found = Some(path.to_path_buf());
        }
    });
    found
}

fn absolute(project: &Project, path: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project.root().join(path)
    };
    normalize(&path)
}

/// How a project names a path; the path as given when outside it.
fn shown(project: &Project, path: &Path) -> String {
    project
        .relative(path)
        .unwrap_or_else(|| path.display().to_string())
}

/// `a/./b/../c` as `a/c`, without asking the file system: the target of a
/// rename does not exist yet.
fn normalize(path: &Path) -> PathBuf {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `target` as a path from `dir`, with forward slashes: what a terrain
/// writes for its heightmap.
fn relative_between(dir: &Path, target: &Path) -> String {
    let (dir, target) = (normalize(dir), normalize(target));
    let a: Vec<_> = dir.components().collect();
    let b: Vec<_> = target.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = vec!["..".to_string(); a.len() - common];
    parts.extend(
        b[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_between_folders_climbs_and_descends() {
        assert_eq!(
            relative_between(
                Path::new("/p/assets/terrain"),
                Path::new("/p/assets/terrain/hills.png")
            ),
            "hills.png"
        );
        assert_eq!(
            relative_between(
                Path::new("/p/assets/terrain"),
                Path::new("/p/assets/paint/hills.png")
            ),
            "../paint/hills.png"
        );
        assert_eq!(
            normalize(Path::new("/p/a/./b/../c")),
            PathBuf::from("/p/a/c")
        );
    }
}
