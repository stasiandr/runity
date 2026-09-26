//! Where things are in a project: wherever a person put them
//! (docs/layout.md).
//!
//! What a file is, its extension says — `.scene.ron`, `.prefab`, `.scrmat`
//! — and not the folder it lies in. So a tomato's model, material and
//! prefab can lie together in `content/kitchen/food/tomato/`, and every
//! tool still finds each of them: they walk the project once and sort what
//! they find by kind.
//!
//! RON is the text of many kinds, so those have a double extension:
//! `main.scene.ron`, `hud.screen.ron`, `cook.animator.ron`. The name a
//! scene or a line of code uses is the file's name without it: `main`.
//!
//! A project laid out before this — `scenes/*.ron`, `ui/*.ron`,
//! `animators/*.ron` — still reads: a plain `.ron` under one of those
//! folders at the root is taken for what the folder said. Until
//! 2026-10-31 (DNA, postulate 7: a second way comes with the date the first
//! one goes); `scrap migrate-layout` renames such a project's files.
//!
//! What the walk skips: folders that start with a dot, and at the root
//! what is derived or is code — `library/`, `target/`, `build/`, `src/`.

use std::path::{Path, PathBuf};

/// What a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A level, or any tree of entities placed in the world.
    Scene,
    /// One entity subtree a scene places by name.
    Prefab,
    /// A `.scrmat` source.
    Material,
    /// A material's own shader: WGSL, a shader graph, an effect graph.
    Shader,
    /// A screen of the game's UI.
    Screen,
    /// Which animation plays when: an Animator Controller.
    Animator,
    /// A clip that moves things, not bones.
    Clip,
    /// A conversation.
    Dialogue,
    /// A quest: its steps and the flags that move it.
    Quest,
    /// The cases `scrap check` plays against a graph or a dialogue beside
    /// them.
    Cases,
}

impl Kind {
    /// Every kind, in the order a listing shows them.
    pub const ALL: [Kind; 10] = [
        Kind::Scene,
        Kind::Prefab,
        Kind::Material,
        Kind::Shader,
        Kind::Screen,
        Kind::Animator,
        Kind::Clip,
        Kind::Dialogue,
        Kind::Quest,
        Kind::Cases,
    ];

    /// The extension a new file of this kind gets: `.scene.ron`.
    pub fn extension(self) -> &'static str {
        match self {
            Kind::Scene => ".scene.ron",
            Kind::Prefab => ".prefab",
            Kind::Material => ".scrmat",
            Kind::Shader => ".wgsl",
            Kind::Screen => ".screen.ron",
            Kind::Animator => ".animator.ron",
            Kind::Clip => ".clip.ron",
            Kind::Dialogue => ".dialogue.ron",
            Kind::Quest => ".quest.ron",
            Kind::Cases => ".cases.ron",
        }
    }

    /// The folder at the root a project laid out before kinds had
    /// extensions kept this kind in, where there was one for a plain
    /// `.ron`.
    pub fn legacy_folder(self) -> Option<&'static str> {
        match self {
            Kind::Scene => Some("scenes"),
            Kind::Screen => Some("ui"),
            Kind::Animator => Some("animators"),
            Kind::Clip => Some("clips"),
            Kind::Dialogue => Some("dialogues"),
            Kind::Quest => Some("quests"),
            _ => None,
        }
    }

    /// The word for it, as a listing or a message says it.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Scene => "scene",
            Kind::Prefab => "prefab",
            Kind::Material => "material",
            Kind::Shader => "shader",
            Kind::Screen => "screen",
            Kind::Animator => "animator",
            Kind::Clip => "clip",
            Kind::Dialogue => "dialogue",
            Kind::Quest => "quest",
            Kind::Cases => "cases",
        }
    }
}

/// The extensions that say a kind, the longest first so `.cases.ron` is
/// not taken for something else.
const SUFFIXES: [(&str, Kind); 14] = [
    (".scene.ron", Kind::Scene),
    (".screen.ron", Kind::Screen),
    (".animator.ron", Kind::Animator),
    (".dialogue.ron", Kind::Dialogue),
    (".quest.ron", Kind::Quest),
    (".cases.ron", Kind::Cases),
    (".subgraph.ron", Kind::Shader),
    (".graph.ron", Kind::Shader),
    (".clip.ron", Kind::Clip),
    (".vfx.ron", Kind::Shader),
    (".post.ron", Kind::Shader),
    (".prefab", Kind::Prefab),
    (".scrmat", Kind::Material),
    (".wgsl", Kind::Shader),
];

/// Folders at the root the walk does not go into: derived, or code.
pub const SKIPPED: [&str; 4] = ["library", "target", "build", "src"];

/// Where a person tries things out (Unreal's `Developers/`): read by the
/// editor like the rest, never shipped by `scrap build`.
pub const DEVELOPERS: &str = "developers";

/// What the file at `relative` — a path from the project's root, forward
/// slashes — is. `None` for a source (a model, a texture), data (a config
/// table) or anything else.
pub fn kind_of(relative: &str) -> Option<Kind> {
    let relative = relative.replace('\\', "/");
    let name = relative.rsplit('/').next().unwrap_or(&relative);
    if let Some((_, kind)) = SUFFIXES
        .iter()
        .find(|(suffix, _)| name.ends_with(suffix) && name.len() > suffix.len())
    {
        return Some(*kind);
    }
    if !name.ends_with(".ron") {
        return None;
    }
    let first = relative.split('/').next().unwrap_or_default();
    if first == relative {
        return None;
    }
    Kind::ALL
        .into_iter()
        .find(|kind| kind.legacy_folder() == Some(first))
}

/// The name a file goes by — what a scene, a link or a line of code calls
/// it: its file name without the extension that says its kind.
/// `maps/main.scene.ron` is `main`, `tomato.prefab` is `tomato`,
/// `scenes/cave.ron` is `cave`.
pub fn name_of(path: impl AsRef<Path>) -> String {
    let name = path
        .as_ref()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    for (suffix, _) in SUFFIXES {
        if let Some(stem) = name.strip_suffix(suffix) {
            if !stem.is_empty() {
                return stem.to_string();
            }
        }
    }
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => name,
    }
}

/// Every file of the project at `root`, by full path, sorted — what every
/// tool sorts by kind. Works over mounted files too
/// ([`crate::files::mount`]), so a game on the web finds its data the same
/// way.
pub fn walk(root: impl AsRef<Path>) -> Vec<PathBuf> {
    let root = root.as_ref();
    let mut out = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = crate::files::list(&folder) else {
            continue;
        };
        for path in entries {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.starts_with('.') {
                continue;
            }
            if crate::files::is_dir(&path) && !crate::files::is_file(&path) {
                if folder == root && SKIPPED.contains(&name.as_str()) {
                    continue;
                }
                folders.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// A path relative to `root`, forward slashes; the path itself when it is
/// not under it.
pub fn relative(root: &Path, path: &Path) -> String {
    let inside = path.strip_prefix(root).unwrap_or(path);
    inside
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Every file of a kind under `root`, sorted by path.
pub fn files(root: impl AsRef<Path>, kind: Kind) -> Vec<PathBuf> {
    let root = root.as_ref();
    walk(root)
        .into_iter()
        .filter(|path| kind_of(&relative(root, path)) == Some(kind))
        .collect()
}

/// The cases `scrap check` plays against a graph or a dialogue: beside it,
/// `<name>.cases.ron`.
pub fn cases_of(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    path.with_file_name(format!("{}.cases.ron", name_of(path)))
}

/// The name a file of a kind goes by in a project: [`name_of`] — or, in
/// the kind's old folder, its path there (`dialogues/harbour/captain.ron`
/// is `harbour/captain`), as a project laid out before named it.
pub fn name_in(root: impl AsRef<Path>, kind: Kind, path: impl AsRef<Path>) -> String {
    let relative = relative(root.as_ref(), path.as_ref());
    if let Some(folder) = kind.legacy_folder() {
        if let Some(rest) = relative.strip_prefix(&format!("{folder}/")) {
            if rest.ends_with(".ron") && kind_of(&relative) == Some(kind) {
                let name = name_of(path.as_ref());
                let dir = rest
                    .rsplit_once('/')
                    .map(|(dir, _)| format!("{dir}/"))
                    .unwrap_or_default();
                return format!("{dir}{name}");
            }
        }
    }
    name_of(path)
}

/// Every file under `root` with a kind, with it.
pub fn index(root: impl AsRef<Path>) -> Vec<(PathBuf, Kind)> {
    let root = root.as_ref();
    walk(root)
        .into_iter()
        .filter_map(|path| {
            let kind = kind_of(&relative(root, &path))?;
            Some((path, kind))
        })
        .collect()
}

/// The file of a kind called `name` under `root`: the first by path, when
/// two are — `scrap check` names that. `name` may also be a path from the
/// root, with or without the extension (`content/game/maps/cave`).
pub fn find(root: impl AsRef<Path>, kind: Kind, name: &str) -> Option<PathBuf> {
    let root = root.as_ref();
    let name = name.replace('\\', "/");
    if name.contains('/') {
        let direct = root.join(&name);
        if crate::files::is_file(&direct) && kind_of(&name) == Some(kind) {
            return Some(direct);
        }
        let with = root.join(format!("{name}{}", kind.extension()));
        if crate::files::is_file(&with) {
            return Some(with);
        }
        let legacy = root.join(format!("{name}.ron"));
        if crate::files::is_file(&legacy) && kind_of(&format!("{name}.ron")) == Some(kind) {
            return Some(legacy);
        }
        // The end of a path: `caves/deep` is `maps/caves/deep.scene.ron`, or
        // `scenes/caves/deep.ron` of a project laid out before.
        let tail = format!("/{name}");
        return files(root, kind).into_iter().find(|path| {
            let full = relative(root, path);
            let bare = match path.parent() {
                Some(parent) => format!("{}/{}", relative(root, parent), name_of(path)),
                None => name_of(path),
            };
            full.ends_with(&tail) || bare == name || bare.ends_with(&tail)
        });
    }
    files(root, kind)
        .into_iter()
        .find(|path| name_of(path) == name)
}

/// Every name a kind has more than one file for, with the files: what a
/// link by name alone cannot tell apart.
pub fn repeated(root: impl AsRef<Path>, kind: Kind) -> Vec<(String, Vec<PathBuf>)> {
    let mut by_name: std::collections::BTreeMap<String, Vec<PathBuf>> = Default::default();
    for path in files(root, kind) {
        by_name.entry(name_of(&path)).or_default().push(path);
    }
    by_name
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .collect()
}

/// The game's data — tables and tuned numbers, plain `.ron` — under
/// `root`, wherever it lies, sorted: every `.ron` with no kind, but not the
/// project's own settings (the files at the root, `config/`), its words
/// (`content/localization/`, the old `strings/`), or the cases a graph
/// keeps.
pub fn data_files(root: impl AsRef<Path>) -> Vec<PathBuf> {
    let root = root.as_ref();
    walk(root)
        .into_iter()
        .filter(|path| {
            let relative = relative(root, path);
            relative.ends_with(".ron")
                && relative.contains('/')
                && kind_of(&relative).is_none()
                && !relative.starts_with("config/")
                && !relative.starts_with("content/localization/")
                && !relative.starts_with("strings/")
        })
        .collect()
}

/// The name a data file goes by — what the game registers its shape under
/// (`register_tuning::<World>("world")`): its file name without `.ron`; in
/// the old `configs/`, its path there (`enemies/goblins`).
pub fn data_name(root: impl AsRef<Path>, path: impl AsRef<Path>) -> String {
    let relative = relative(root.as_ref(), path.as_ref());
    match relative.strip_prefix("configs/") {
        Some(rest) => rest.strip_suffix(".ron").unwrap_or(rest).to_string(),
        None => name_of(path),
    }
}

/// Whether a project-relative path is in a `developers/` folder: a
/// sandbox, not the game.
pub fn in_developers(relative: &str) -> bool {
    relative
        .replace('\\', "/")
        .split('/')
        .any(|part| part == DEVELOPERS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extension_says_the_kind_and_the_folder_does_not() {
        assert_eq!(
            kind_of("content/game/maps/main.scene.ron"),
            Some(Kind::Scene)
        );
        assert_eq!(
            kind_of("content/game/food/tomato/tomato.prefab"),
            Some(Kind::Prefab)
        );
        assert_eq!(kind_of("anywhere/stone.scrmat"), Some(Kind::Material));
        assert_eq!(
            kind_of("content/game/ui/screens/hud.screen.ron"),
            Some(Kind::Screen)
        );
        assert_eq!(
            kind_of("content/game/cook/cook.animator.ron"),
            Some(Kind::Animator)
        );
        assert_eq!(
            kind_of("content/game/cook/cook.cases.ron"),
            Some(Kind::Cases)
        );
        assert_eq!(kind_of("content/game/water/water.wgsl"), Some(Kind::Shader));
        assert_eq!(kind_of("fx/lava.graph.ron"), Some(Kind::Shader));
        assert_eq!(kind_of("fx/sparks.vfx.ron"), Some(Kind::Shader));
        assert_eq!(kind_of("fx/film.post.ron"), Some(Kind::Shader));
        // Data: a config table is a plain `.ron` wherever it lies.
        assert_eq!(kind_of("content/game/core/world.ron"), None);
        assert_eq!(kind_of("content/game/food/tomato/tomato.png"), None);
    }

    #[test]
    fn a_plain_ron_in_an_old_folder_is_what_the_folder_said() {
        assert_eq!(kind_of("scenes/main.ron"), Some(Kind::Scene));
        assert_eq!(kind_of("scenes/caves/deep.ron"), Some(Kind::Scene));
        assert_eq!(kind_of("ui/hud.ron"), Some(Kind::Screen));
        assert_eq!(kind_of("animators/door.ron"), Some(Kind::Animator));
        assert_eq!(kind_of("animators/door.cases.ron"), Some(Kind::Cases));
        assert_eq!(kind_of("clips/bell.ron"), Some(Kind::Clip));
        assert_eq!(kind_of("dialogues/chef.ron"), Some(Kind::Dialogue));
        assert_eq!(kind_of("configs/world.ron"), None);
        // Only at the root: a feature's `ui/` is not the old one.
        assert_eq!(kind_of("content/game/ui/hud.ron"), None);
    }

    #[test]
    fn a_name_is_the_file_name_without_what_says_its_kind() {
        assert_eq!(name_of("content/game/maps/main.scene.ron"), "main");
        assert_eq!(name_of("scenes/cave.ron"), "cave");
        assert_eq!(name_of("tomato.prefab"), "tomato");
        assert_eq!(name_of("lava.graph.ron"), "lava");
        assert_eq!(name_of("door.cases.ron"), "door");
        assert_eq!(name_of("rock.glb"), "rock");
    }

    #[test]
    fn the_walk_finds_a_kind_anywhere_and_skips_what_is_derived_or_code() {
        let root = Path::new("/nowhere/scrap-layout-test");
        crate::files::mount(
            root,
            [
                ("content/game/maps/main.scene.ron", "()"),
                ("content/game/food/tomato/tomato.prefab", "()"),
                ("content/game/food/soup/soup.prefab", "()"),
                ("scenes/old.ron", "()"),
                ("library/ab.prefab", "()"),
                ("src/lib.prefab", "()"),
                (".scrap/live/main.scene.ron", "()"),
            ]
            .map(|(p, t)| (p.to_string(), t.as_bytes().to_vec())),
        );
        let prefabs: Vec<String> = files(root, Kind::Prefab).iter().map(name_of).collect();
        assert_eq!(prefabs, ["soup", "tomato"]);
        let scenes: Vec<String> = files(root, Kind::Scene)
            .iter()
            .map(|p| relative(root, p))
            .collect();
        assert_eq!(
            scenes,
            ["content/game/maps/main.scene.ron", "scenes/old.ron"]
        );
        assert_eq!(
            find(root, Kind::Scene, "old").unwrap(),
            root.join("scenes/old.ron")
        );
        assert_eq!(
            find(root, Kind::Scene, "content/game/maps/main").unwrap(),
            root.join("content/game/maps/main.scene.ron")
        );
        assert!(find(root, Kind::Scene, "nothing").is_none());
        // The end of a path, the extension left out.
        assert_eq!(
            find(root, Kind::Prefab, "tomato/tomato").unwrap(),
            root.join("content/game/food/tomato/tomato.prefab")
        );
        assert_eq!(
            find(root, Kind::Scene, "maps/main").unwrap(),
            root.join("content/game/maps/main.scene.ron")
        );
    }

    #[test]
    fn developers_is_a_sandbox() {
        assert!(in_developers("content/developers/ann/test.scene.ron"));
        assert!(!in_developers("content/game/maps/main.scene.ron"));
    }
}
