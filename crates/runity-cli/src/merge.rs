//! `runity merge`: the git merge driver for scenes and prefabs.
//!
//! Git hands a driver three files — the common ancestor, ours, theirs — and
//! takes the result from where ours was. This merges them as scenes: by
//! entity and field (see [`runity::merge`]), written over ours with ours'
//! text kept wherever the merge did not change it. A conflict leaves ours
//! in place, is described in words on the terminal, and makes the driver
//! fail, so git marks the file conflicted — and the file still loads.
//!
//! A file that is not a scene or a prefab, or does not parse, goes to git's
//! own line merge, conflict markers and all: nothing is worse off for the
//! driver being on.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use runity::merge::{merge_prefabs, merge_scenes, Merged};
use runity::{Prefabs, Scene};

/// What the merge came to.
pub struct Outcome {
    /// Conflicts in words; empty when the merge is clean.
    pub conflicts: Vec<String>,
    /// Whether git's line merge did it instead.
    pub by_lines: bool,
}

/// Merge `theirs` into `ours` with `base` as the ancestor, writing the
/// result over `ours`. `path` is the file's name in the repository, which
/// says whether it is a scene or a prefab; git's temporary files do not.
pub fn merge_files(
    base: &Path,
    ours: &Path,
    theirs: &Path,
    path: Option<&Path>,
) -> Result<Outcome> {
    let named = path.unwrap_or(ours);
    let prefab = named.extension().and_then(|e| e.to_str()) == Some("prefab");
    let merged = if prefab {
        read_prefab(base)
            .zip(read_prefab(ours))
            .zip(read_prefab(theirs))
            .map(|((b, o), t)| {
                let merged = merge_prefabs(&b, &o, &t);
                (merged, true)
            })
    } else {
        Scene::load(base)
            .ok()
            .zip(Scene::load(ours).ok())
            .zip(Scene::load(theirs).ok())
            .map(|((b, o), t)| (merge_scenes(&b, &o, &t), false))
    };
    let Some((Merged { scene, conflicts }, prefab)) = merged else {
        return by_lines(base, ours, theirs);
    };
    if prefab {
        let root = scene
            .entities
            .into_iter()
            .next()
            .context("the merged prefab has no root")?;
        Prefabs::save(&root, ours).map_err(anyhow::Error::msg)?;
    } else {
        scene.save(ours)?;
    }
    Ok(Outcome {
        conflicts: conflicts.iter().map(ToString::to_string).collect(),
        by_lines: false,
    })
}

fn read_prefab(path: &Path) -> Option<runity::EntityDesc> {
    Prefabs::read(path).ok().map(|(_, desc)| desc)
}

fn by_lines(base: &Path, ours: &Path, theirs: &Path) -> Result<Outcome> {
    let status = Command::new("git")
        .args(["merge-file", "-L", "ours", "-L", "base", "-L", "theirs"])
        .arg(ours)
        .arg(base)
        .arg(theirs)
        .status()
        .context("running git merge-file")?;
    Ok(Outcome {
        conflicts: if status.success() {
            Vec::new()
        } else {
            vec!["not a scene that reads; merged by lines, with conflict markers".into()]
        },
        by_lines: true,
    })
}

/// Turn the driver on in the clone `root` is in: git keeps drivers in its
/// local config, which is not committed, so each clone does this once.
/// The `.gitattributes` lines that route scenes and prefabs to it are
/// added if the project does not have them yet.
pub fn git_setup(root: &Path) -> Result<Vec<String>> {
    let mut done = Vec::new();
    let git = |args: &[&str]| -> Result<()> {
        let status = Command::new("git")
            .current_dir(root)
            .args(args)
            .status()
            .context("running git")?;
        anyhow::ensure!(status.success(), "git {} failed", args.join(" "));
        Ok(())
    };
    git(&[
        "config",
        "merge.runity.name",
        "runity: scenes merged by entity and field",
    ])?;
    git(&["config", "merge.runity.driver", "runity merge %O %A %B %P"])?;
    done.push("merge.runity driver set in this clone's git config".to_string());

    let attributes = root.join(".gitattributes");
    let text = std::fs::read_to_string(&attributes).unwrap_or_default();
    if !text.contains("merge=runity") {
        let mut text = text;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(runity::project::MERGE_ATTRIBUTES);
        std::fs::write(&attributes, text)?;
        done.push(".gitattributes routes scenes and prefabs to it".to_string());
    }
    Ok(done)
}
