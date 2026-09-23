//! `runity run [--hot] [--scene NAME]`: the game, from its project, in one
//! command, on `scenes/main.ron` or the scene named.
//!
//! Plain, it is `cargo run` in the project: scenes, prefabs, assets,
//! shaders and numbers already reload while it runs. With `--hot` the game
//! runs under `dx serve --hotpatch`, and a saved change to the game's own
//! Rust — a system, `step`, `frame` — is compiled and patched into the
//! running process through `subsecond`, keeping its state (DNA, postulate
//! 1). `dx` is the Dioxus CLI; when it is not installed, `--hot` says how
//! to get it rather than falling back quietly to something slower.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Result};
use runity::Project;

/// Where an executable of this name is on the PATH.
pub fn on_path(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(&exe))
            .find(|path| path.is_file())
    })
}

/// The command that runs the game: `cargo run` in the project, or `dx
/// serve --hotpatch` with `hot`, where `dx` is the one found (if any).
pub fn command(project: &Project, hot: bool, release: bool, dx: Option<&Path>) -> Result<Command> {
    if !project.root().join("Cargo.toml").is_file() {
        bail!(
            "{} has no game crate (Cargo.toml) to run — `runity new` makes one",
            project.root().display()
        );
    }
    let mut command = if hot {
        let Some(dx) = dx else {
            bail!(
                "--hot runs the game under dx, which is not on the PATH: \
                 `cargo install dioxus-cli`, then `runity run --hot` again — \
                 or `runity run` without it, where everything but the game's \
                 own code still reloads"
            );
        };
        if release {
            bail!("--hot patches a debug build; leave out --release");
        }
        let mut command = Command::new(dx);
        command.args(["serve", "--hotpatch"]);
        command
    } else {
        let mut command = Command::new("cargo");
        command.arg("run");
        if release {
            command.arg("--release");
        }
        command
    };
    command.current_dir(project.root());
    Ok(command)
}

/// What the game reads to know which scene to open: `main` without it.
pub const SCENE_VAR: &str = "RUNITY_SCENE";

/// Check that the project has `scenes/NAME.ron`, for `--scene NAME`, and
/// say which ones it has when not.
pub fn scene(project: &Project, name: &str) -> Result<String> {
    let names = project.scene_names();
    if names.iter().any(|n| n == name) {
        return Ok(name.to_string());
    }
    let near = runity::spelling::closest(name, names.iter().map(String::as_str))
        .map(|n| format!(" — did you mean `{n}`?"))
        .unwrap_or_default();
    bail!(
        "no scenes/{name}.ron{near} (there are: {})",
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> Project {
        let root = std::env::temp_dir().join(format!("runity-run-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        Project::create(&root, name).unwrap()
    }

    #[test]
    fn a_scene_to_play_is_one_the_project_has() {
        let project = project("scene");
        assert_eq!(scene(&project, "main").unwrap(), "main");
        let e = scene(&project, "mian").unwrap_err().to_string();
        assert!(e.contains("did you mean `main`?"), "{e}");
    }

    fn args(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn plain_is_cargo_run_and_hot_is_dx_with_the_hotpatch() {
        let project = project("commands");
        let plain = command(&project, false, false, None).unwrap();
        assert_eq!(plain.get_program(), "cargo");
        assert_eq!(args(&plain), ["run"]);
        assert_eq!(plain.get_current_dir(), Some(project.root()));
        assert_eq!(
            args(&command(&project, false, true, None).unwrap()),
            ["run", "--release"]
        );

        let dx = Path::new("/opt/bin/dx");
        let hot = command(&project, true, false, Some(dx)).unwrap();
        assert_eq!(hot.get_program(), dx.as_os_str());
        assert_eq!(args(&hot), ["serve", "--hotpatch"]);

        let e = command(&project, true, false, None)
            .unwrap_err()
            .to_string();
        assert!(e.contains("cargo install dioxus-cli"), "{e}");
        assert!(command(&project, true, true, Some(dx)).is_err());
    }
}
