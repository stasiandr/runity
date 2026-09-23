//! `runity build`: a folder to ship.
//!
//! Unity's File → Build, for a project laid out the standard way: bring the
//! library up to date, compile the game, and lay out
//!
//! ```text
//! <out>/
//!   <game>            the executable
//!   data/
//!     runity.ron      so the game finds its project
//!     input.ron       the bindings
//!     scenes/  prefabs/
//!     library/        the built assets, and nothing they were built from
//! ```
//!
//! Sources — `assets/`, `materials/`, the `.rimport` sidecars — stay home:
//! a player needs the `.rasset`s, not the `.png`s they came from. The game
//! finds `data/` through [`runity::project::data_file`].

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use runity::project::{
    ANIMATORS, DATA, FILE, INPUT, LIBRARY, PREFABS, SCENES, SHADERS, TUNING, UI,
};
use runity::Project;

pub struct Built {
    pub folder: PathBuf,
    pub executable: PathBuf,
    /// Sources the library could not be brought up to date with.
    pub stale: Vec<String>,
}

/// Build `project` into `out`: `release` for a shippable build, or a debug
/// one, which is quicker and what a smoke test wants.
pub fn build(project: &Project, out: &Path, release: bool) -> Result<Built> {
    let stale: Vec<String> = runity_import::sync(project)
        .into_iter()
        .filter_map(|r| {
            r.result
                .err()
                .map(|e| format!("{}: {e}", r.source.display()))
        })
        .collect();

    let manifest = project.root().join("Cargo.toml");
    if !manifest.is_file() {
        bail!("{} has no game crate to build", project.root().display());
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .args(["build", "--message-format", "short", "--manifest-path"])
        .arg(&manifest);
    if release {
        command.arg("--release");
    }
    let output = command.output().context("running cargo")?;
    if !output.status.success() {
        bail!(
            "the game did not build:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let executable = find_executable(project, release)?;
    let executable = package(project, &executable, out)?;
    Ok(Built {
        folder: out.to_path_buf(),
        executable,
        stale,
    })
}

/// Where cargo put the game: its target folder, which `CARGO_TARGET_DIR`
/// may have moved.
pub fn find_executable(project: &Project, release: bool) -> Result<PathBuf> {
    let name = crate_name(project)?;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| project.root().join("target"));
    let path = target
        .join(if release { "release" } else { "debug" })
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    anyhow::ensure!(path.is_file(), "no executable at {}", path.display());
    Ok(path)
}

fn crate_name(project: &Project) -> Result<String> {
    let text = std::fs::read_to_string(project.root().join("Cargo.toml"))?;
    text.lines()
        .find_map(|line| {
            let rest = line.trim().strip_prefix("name")?.trim().strip_prefix('=')?;
            Some(rest.trim().trim_matches('"').to_string())
        })
        .context("Cargo.toml has no package name")
}

/// Lay a built game out in `out`: the executable, and `data/` with what it
/// reads at run time. What was there before is replaced, so a file deleted
/// from the project does not linger in the build. Returns where the
/// executable went.
pub fn package(project: &Project, executable: &Path, out: &Path) -> Result<PathBuf> {
    if out.exists() {
        std::fs::remove_dir_all(out).with_context(|| format!("clearing {}", out.display()))?;
    }
    let data = out.join(DATA);
    std::fs::create_dir_all(&data)?;
    std::fs::copy(project.root().join(FILE), data.join(FILE))?;
    for file in [INPUT, runity::layers::FILE] {
        if project.root().join(file).is_file() {
            std::fs::copy(project.root().join(file), data.join(file))?;
        }
    }
    for dir in [
        SCENES,
        PREFABS,
        LIBRARY,
        TUNING,
        UI,
        ANIMATORS,
        SHADERS,
        runity::strings::DIR,
    ] {
        copy_tree(&project.root().join(dir), &data.join(dir))?;
    }
    let shipped = out.join(
        executable
            .file_name()
            .context("the executable has no name")?,
    );
    std::fs::copy(executable, &shipped)?;
    Ok(shipped)
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    let Ok(entries) = std::fs::read_dir(from) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            copy_tree(&path, &to.join(&name))?;
        } else {
            std::fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}
