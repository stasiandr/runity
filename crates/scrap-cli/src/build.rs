//! `scrap build`: a folder to ship.
//!
//! Unity's File → Build, for a project laid out the standard way: bring the
//! library up to date, compile the game, and lay out
//!
//! ```text
//! <out>/
//!   <game>            the executable
//!   data/
//!     scrap.ron      so the game finds its project
//!     input.ron       the bindings
//!     scenes/  prefabs/
//!     library/        the built assets, and nothing they were built from
//! ```
//!
//! Sources — `assets/`, `materials/`, the `.scrimport` sidecars — stay home:
//! a player needs the `.scrasset`s, not the `.png`s they came from. The game
//! finds `data/` through [`scrap::project::data_file`].
//!
//! The library is cooked for the platform on the way ([`Platform`]): its
//! textures in the GPU blocks that platform samples — BC7 for a desktop,
//! ASTC for a phone — each encoded once and kept in the project's
//! `.scrap/cook/` (`scrap_import::cook`).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use scrap::project::{
    ANIMATORS, CONFIGS, DATA, FILE, INPUT, LIBRARY, PREFABS, SCENES, SHADERS, UI,
};
use scrap::Project;

/// What a build's textures are cooked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// As the library has them: RGBA8.
    Raw,
    /// Windows, macOS, Linux: BC7.
    Desktop,
    /// iOS, Android: ASTC 4x4.
    Mobile,
}

impl Platform {
    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "none" | "raw" => Platform::Raw,
            "desktop" => Platform::Desktop,
            "mobile" | "ios" | "android" => Platform::Mobile,
            other => bail!("no platform {other}: desktop, mobile or none"),
        })
    }

    fn coding(self) -> Option<scrap::asset::TextureCoding> {
        match self {
            Platform::Raw => None,
            Platform::Desktop => Some(scrap_import::cook::Platform::Desktop.coding()),
            Platform::Mobile => Some(scrap_import::cook::Platform::Mobile.coding()),
        }
    }
}

/// `project`'s library into `out`, its textures cooked into `coding`'s
/// blocks, what was encoded before taken from the project's cache.
pub fn cook(project: &Project, out: &Path, coding: scrap::asset::TextureCoding, zstd: bool) -> Result<scrap_import::cook::Cooked> {
    let cache = project.root().join(".scrap").join("cook");
    scrap_import::cook::cook_library(&project.root().join(LIBRARY), out, coding, zstd, &cache)
}

/// How a game is compiled (DNA, "Два профиля сборки игры"): for speed,
/// the default — what a player plays — or for size, when the download is
/// what matters (the web, a jam's upload limit); or debug, quick to make,
/// what a smoke test wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Debug,
    /// Cargo's release: everything optimised for speed.
    Speed,
    /// `small`: release optimised for size — `opt-level = "z"`, the whole
    /// program at link time, one codegen unit, symbols stripped, panics
    /// that abort (the crash report is still written: the hook runs
    /// first). Given to cargo on the command line, so a game's Cargo.toml
    /// needs nothing for it and may still say its own `[profile.small]`.
    Size,
}

impl Profile {
    /// Where cargo puts what it builds with this profile.
    fn folder(self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Speed => "release",
            Profile::Size => "small",
        }
    }

    /// What `cargo build` is given for it.
    pub fn cargo_args(self) -> Vec<String> {
        match self {
            Profile::Debug => Vec::new(),
            Profile::Speed => vec!["--release".into()],
            Profile::Size => {
                let mut out: Vec<String> = [
                    "profile.small.inherits=\"release\"",
                    "profile.small.opt-level=\"z\"",
                    "profile.small.lto=true",
                    "profile.small.codegen-units=1",
                    "profile.small.strip=true",
                    "profile.small.panic=\"abort\"",
                ]
                .iter()
                .flat_map(|c| ["--config".to_string(), c.to_string()])
                .collect();
                out.extend(["--profile".into(), "small".into()]);
                out
            }
        }
    }
}

pub struct Built {
    pub folder: PathBuf,
    pub executable: PathBuf,
    /// Sources the library could not be brought up to date with.
    pub stale: Vec<String>,
}

/// Build `project` into `out`: `release` for a shippable build (for
/// speed), or a debug one, which is quicker and what a smoke test wants.
pub fn build(project: &Project, out: &Path, release: bool) -> Result<Built> {
    build_with(project, out, if release { Profile::Speed } else { Profile::Debug })
}

/// Build `project` into `out` with a profile.
pub fn build_with(project: &Project, out: &Path, profile: Profile) -> Result<Built> {
    build_for(project, out, profile, Platform::Desktop)
}

/// Build `project` into `out` with a profile, its textures cooked for a
/// platform.
pub fn build_for(project: &Project, out: &Path, profile: Profile, platform: Platform) -> Result<Built> {
    let stale: Vec<String> = scrap_import::sync(project)
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
    command.args(profile.cargo_args());
    let output = command.output().context("running cargo")?;
    if !output.status.success() {
        bail!(
            "the game did not build:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let executable = find_in(project, profile.folder())?;
    let executable = package_for(project, &executable, out, platform)?;
    Ok(Built {
        folder: out.to_path_buf(),
        executable,
        stale,
    })
}

/// Where cargo put the game: its target folder, which `CARGO_TARGET_DIR`
/// may have moved.
pub fn find_executable(project: &Project, release: bool) -> Result<PathBuf> {
    find_in(project, if release { "release" } else { "debug" })
}

fn find_in(project: &Project, folder: &str) -> Result<PathBuf> {
    let name = crate_name(project)?;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| project.root().join("target"));
    let path = target
        .join(folder)
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
    package_for(project, executable, out, Platform::Raw)
}

/// [`package`], the library cooked for `platform`.
pub fn package_for(project: &Project, executable: &Path, out: &Path, platform: Platform) -> Result<PathBuf> {
    if out.exists() {
        std::fs::remove_dir_all(out).with_context(|| format!("clearing {}", out.display()))?;
    }
    let data = out.join(DATA);
    std::fs::create_dir_all(&data)?;
    std::fs::copy(project.root().join(FILE), data.join(FILE))?;
    for file in [INPUT, scrap::layers::FILE] {
        if project.root().join(file).is_file() {
            std::fs::copy(project.root().join(file), data.join(file))?;
        }
    }
    match platform.coding() {
        Some(coding) => {
            // A phone's storage and download are what zstd saves; a
            // desktop's load is quicker without it.
            cook(project, &data.join(LIBRARY), coding, platform == Platform::Mobile)?;
        }
        None => copy_tree(&project.root().join(LIBRARY), &data.join(LIBRARY))?,
    }
    for dir in [
        SCENES,
        PREFABS,
        CONFIGS,
        UI,
        ANIMATORS,
        SHADERS,
        scrap::strings::DIR,
        scrap::dialogue::DIR,
        scrap::motion::DIR,
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

#[cfg(test)]
mod profile_tests {
    use super::Profile;

    #[test]
    fn the_size_profile_is_given_to_cargo_whole() {
        let args = Profile::Size.cargo_args();
        assert!(args.windows(2).any(|w| w == ["--profile", "small"]));
        assert!(args.iter().any(|a| a == "profile.small.opt-level=\"z\""));
        assert_eq!(Profile::Speed.cargo_args(), ["--release"]);
        assert!(Profile::Debug.cargo_args().is_empty());
    }
}
