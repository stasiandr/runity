//! `scrap migrate-layout`: a project laid out before 2026-09-26 — a
//! folder per kind at the root — moved to the layout of docs/layout.md.
//!
//! Mechanical, so it can be trusted: every file keeps its folder under the
//! new place (`prefabs/food/tomato.prefab` becomes
//! `content/<name>/prefabs/food/tomato.prefab`), a `.ron` whose old folder
//! said its kind gets the extension that says it now
//! (`scenes/main.ron` → `content/<name>/maps/main.scene.ron`), and a
//! sidecar moves with its file, its `source` rewritten. Grouping by feature
//! is a person's to do after — a tomato's prefab and material moved beside
//! each other — and links by ID keep every scene working while they do.
//!
//! What it cannot move is a path written in the game's code
//! (`data_file(.., "scenes/main.ron")`): those it lists.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use scrap::layout::Kind;
use scrap::Project;

/// What a migration did.
#[derive(Debug, Default)]
pub struct Migrated {
    /// Every file moved, from and to, project-relative.
    pub moved: Vec<(String, String)>,
    /// Lines of the game's code that name a path that moved: `file:line:
    /// text`.
    pub code: Vec<String>,
}

/// Where each old folder's files go, under `content/<name>/`, and the kind
/// a plain `.ron` there was.
fn destination(folder: &str) -> Option<(&'static str, Option<Kind>)> {
    Some(match folder {
        "scenes" => ("maps", Some(Kind::Scene)),
        "ui" => ("ui/screens", Some(Kind::Screen)),
        "animators" => ("animators", Some(Kind::Animator)),
        "clips" => ("clips", Some(Kind::Clip)),
        "dialogues" => ("dialogues", Some(Kind::Dialogue)),
        "quests" => ("quests", Some(Kind::Quest)),
        "prefabs" => ("prefabs", None),
        "materials" => ("materials", None),
        "shaders" => ("shaders", None),
        "configs" => ("configs", None),
        "assets" => ("", None),
        _ => return None,
    })
}

/// Move a project laid out before to the layout of docs/layout.md. Refuses
/// one that already has `content/`.
pub fn migrate(project: &Project) -> Result<Migrated> {
    let root = project.root();
    if !project.is_legacy() {
        bail!(
            "{} already has content/: nothing to migrate",
            root.display()
        );
    }
    let own = Path::new(scrap::project::CONTENT).join(scrap::project::crate_name(project.name()));
    let mut plan: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (old, new) in [
        (scrap::project::legacy::INPUT, scrap::project::INPUT),
        (scrap::layers::LEGACY_FILE, scrap::layers::FILE),
    ] {
        if root.join(old).is_file() {
            plan.push((root.join(old), root.join(new)));
        }
    }
    for file in files(&root.join(scrap::strings::LEGACY_DIR)) {
        let rest = file.strip_prefix(root.join(scrap::strings::LEGACY_DIR))?;
        plan.push((file.clone(), root.join(scrap::strings::DIR).join(rest)));
    }
    for folder in [
        "scenes",
        "ui",
        "animators",
        "clips",
        "dialogues",
        "quests",
        "prefabs",
        "materials",
        "shaders",
        "configs",
        "assets",
    ] {
        let (to, kind) = destination(folder).expect("every folder has a place");
        let from = root.join(folder);
        for file in files(&from) {
            let rest = file.strip_prefix(&from)?.to_path_buf();
            let mut target = root.join(&own).join(to).join(&rest);
            let mut name = rest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let orphan = name.ends_with(".scrimport") && !file.with_extension("").exists();
            if name.ends_with(".scrimport") && !orphan {
                // Moves with its file, below.
                continue;
            }
            // A sidecar whose file is gone goes where its file would have.
            if orphan {
                name = name.trim_end_matches(".scrimport").to_string();
                target.set_file_name(&name);
            }
            if let Some(kind) = kind {
                let plain = name.ends_with(".ron") && scrap::layout::kind_of(&name).is_none();
                if plain {
                    let stem = name.strip_suffix(".ron").unwrap_or(&name);
                    target.set_file_name(format!("{stem}{}", kind.extension()));
                }
            }
            if orphan {
                let sidecar = scrap_import::sidecar_for(&target);
                let text = std::fs::read_to_string(&file)?;
                let text = text.replacen(
                    &format!("{:?}", shown(root, &file.with_extension(""))),
                    &format!("{:?}", shown(root, &target)),
                    1,
                );
                std::fs::create_dir_all(sidecar.parent().unwrap_or(root))?;
                std::fs::write(&sidecar, text)?;
                std::fs::remove_file(&file)?;
                continue;
            }
            plan.push((file, target));
        }
    }

    let mut out = Migrated::default();
    for (_, to) in &plan {
        if to.exists() {
            bail!(
                "{} is already there; move it aside and run again",
                shown(root, to)
            );
        }
    }
    for (from, to) in plan {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&from, &to).with_context(|| format!("moving {}", from.display()))?;
        let sidecar = scrap_import::sidecar_for(&from);
        if sidecar.is_file() {
            let moved = scrap_import::sidecar_for(&to);
            let text = std::fs::read_to_string(&sidecar)?;
            let text = text.replacen(
                &format!("{:?}", shown(root, &from)),
                &format!("{:?}", shown(root, &to)),
                1,
            );
            std::fs::write(&moved, text)?;
            std::fs::remove_file(&sidecar)?;
        }
        out.moved.push((shown(root, &from), shown(root, &to)));
    }
    // What is left of the old folders: empty, or only `.gitkeep`.
    for folder in [
        "scenes",
        "ui",
        "animators",
        "clips",
        "dialogues",
        "quests",
        "prefabs",
        "materials",
        "shaders",
        "configs",
        "assets",
        "strings",
    ] {
        remove_if_empty(&root.join(folder));
    }
    std::fs::create_dir_all(
        root.join(scrap::project::CONTENT)
            .join(scrap::layout::DEVELOPERS),
    )?;
    let keep = root
        .join(scrap::project::CONTENT)
        .join(scrap::layout::DEVELOPERS)
        .join(".gitkeep");
    if !keep.exists() {
        std::fs::write(keep, "")?;
    }
    attributes(root)?;
    out.code = code_paths(root, &out.moved);
    Ok(out)
}

/// `.gitattributes`: the merge lines by extension, in place of the ones by
/// folder.
fn attributes(root: &Path) -> Result<()> {
    let path = root.join(".gitattributes");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let old = [
        "scenes/**/*.ron merge=scrap",
        "prefabs/**/*.prefab merge=scrap",
        "configs/**/*.ron merge=scrap",
    ];
    if !old
        .iter()
        .any(|line| text.lines().any(|l| l.trim() == *line))
    {
        return Ok(());
    }
    let mut lines: Vec<String> = Vec::new();
    let mut put = false;
    for line in text.lines() {
        if old.contains(&line.trim()) {
            if !put {
                lines.extend(
                    [
                        "*.scene.ron merge=scrap",
                        "*.prefab merge=scrap",
                        "*.ron merge=scrap",
                    ]
                    .map(String::from),
                );
                put = true;
            }
            continue;
        }
        lines.push(line.to_string());
    }
    std::fs::write(&path, lines.join("\n") + "\n")?;
    Ok(())
}

/// Lines of the game's code under `src/` that name an old path in a
/// string: `"scenes/`, `"ui/hud.ron"`, `"input.ron"`.
fn code_paths(root: &Path, moved: &[(String, String)]) -> Vec<String> {
    let mut folders: Vec<String> = moved
        .iter()
        .filter_map(|(from, _)| from.split_once('/').map(|(first, _)| format!("\"{first}/")))
        .collect();
    folders.extend([
        "\"input.ron\"".to_string(),
        "\"layers.ron\"".to_string(),
        "\"strings\"".to_string(),
        "\"ui\"".to_string(),
        "\"prefabs\"".to_string(),
        "\"shaders\"".to_string(),
    ]);
    folders.sort();
    folders.dedup();
    let mut out = Vec::new();
    for file in files(&root.join(scrap::project::SRC)) {
        if file.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        for (n, line) in text.lines().enumerate() {
            if folders.iter().any(|f| line.contains(f.as_str())) {
                out.push(format!("{}:{}: {}", shown(root, &file), n + 1, line.trim()));
            }
        }
    }
    out
}

/// Every file under `dir`, sorted, but `.gitkeep`.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else if entry.file_name() != ".gitkeep" {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn remove_if_empty(dir: &Path) {
    if !dir.is_dir() {
        return;
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.path().is_dir() {
            remove_if_empty(&entry.path());
        }
    }
    let _ = std::fs::remove_file(dir.join(".gitkeep"));
    let _ = std::fs::remove_dir(dir);
}

fn shown(root: &Path, path: &Path) -> String {
    scrap::layout::relative(root, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_project_moves_to_config_and_content_with_its_sidecars() {
        let root = std::env::temp_dir().join(format!("scrap-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |p: &str, t: &str| {
            let path = root.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        write("scrap.ron", "(name: \"Old Game\")");
        write("input.ron", "()");
        write("scenes/main.ron", "(entities: [])");
        write("scenes/main.ron.scrimport", "ImportSettings(\n    source: \"scenes/main.ron\",\n    id: Some(\"00000000000000000000000000000001\"),\n)\n");
        write("prefabs/.gitkeep", "");
        write("prefabs/rock.prefab", "(name: \"rock\")");
        write("animators/door.ron", "()");
        write("animators/door.cases.ron", "()");
        write("assets/trees/pine.obj", "v 0 0 0");
        write("strings/en.ron", "{}");
        write(
            "src/main.rs",
            "fn main() { let _ = \"scenes/main.ron\"; }\n",
        );
        write(
            ".gitattributes",
            "* text=auto eol=lf\nscenes/**/*.ron merge=scrap\nprefabs/**/*.prefab merge=scrap\n",
        );
        let project = Project::open(&root).unwrap();
        let done = migrate(&project).unwrap();
        for file in [
            "config/input.ron",
            "content/old_game/maps/main.scene.ron",
            "content/old_game/maps/main.scene.ron.scrimport",
            "content/old_game/prefabs/rock.prefab",
            "content/old_game/animators/door.animator.ron",
            "content/old_game/animators/door.cases.ron",
            "content/old_game/trees/pine.obj",
            "content/localization/en.ron",
            "content/developers/.gitkeep",
        ] {
            assert!(root.join(file).is_file(), "{file}");
        }
        assert!(!root.join("scenes").exists() && !root.join("prefabs").exists());
        let sidecar =
            std::fs::read_to_string(root.join("content/old_game/maps/main.scene.ron.scrimport"))
                .unwrap();
        assert!(
            sidecar.contains("\"content/old_game/maps/main.scene.ron\""),
            "{sidecar}"
        );
        let attributes = std::fs::read_to_string(root.join(".gitattributes")).unwrap();
        assert!(
            attributes.contains("*.scene.ron merge=scrap") && !attributes.contains("scenes/**"),
            "{attributes}"
        );
        assert_eq!(done.code.len(), 1, "{:?}", done.code);
        let project = Project::open(&root).unwrap();
        assert!(!project.is_legacy());
        assert_eq!(project.scene_names(), ["main"]);
        assert_eq!(project.input_file(), root.join("config/input.ron"));
        assert!(migrate(&project).is_err(), "twice is refused");
        let _ = std::fs::remove_dir_all(&root);
    }
}
