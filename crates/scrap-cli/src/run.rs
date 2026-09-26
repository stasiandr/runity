//! `scrap run [--hot] [--scene NAME]`: the game, from its project, in one
//! command, on the start scene or the scene named.
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
use scrap::Project;

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
            "{} has no game crate (Cargo.toml) to run — `scrap new` makes one",
            project.root().display()
        );
    }
    let mut command = if hot {
        let Some(dx) = dx else {
            bail!(
                "--hot runs the game under dx, which is not on the PATH: \
                 `cargo install dioxus-cli`, then `scrap run --hot` again — \
                 or `scrap run` without it, where everything but the game's \
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

/// `--players N`: the game built once, then started N times on this
/// machine, playing together — the host first, the others joining it, each
/// in its own window laid out beside the others, with its own player
/// folder, and its lines marked with whose they are. Unity's Multiplayer
/// Play Mode, from a terminal. Ends when the host does; ending it ends all.
pub fn players(
    project: &Project,
    count: u32,
    release: bool,
    scene: Option<&str>,
    link: &str,
) -> Result<bool> {
    if !(2..=4).contains(&count) {
        bail!("--players wants 2 to 4; one player is plain `scrap run`");
    }
    scrap::net::Conditions::parse(link).map_err(|e| anyhow::anyhow!("--link: {e}"))?;
    let mut build = Command::new("cargo");
    build.arg("build").current_dir(project.root());
    if release {
        build.arg("--release");
    }
    if !build.status()?.success() {
        return Ok(false);
    }
    let exe = crate::build::find_executable(project, release)?;
    let size = scrap::project::GameSettings::load(&project.root().to_string_lossy())
        .map(|(_, s)| (s.width, s.height))
        .unwrap_or((1280, 720));
    let address = format!("127.0.0.1:{}", scrap::party::free_port()?);
    println!("{count} players: player 1 hosts on {address}");
    let mut children = Vec::new();
    for peer in 0..count {
        let mut command = player_command(&exe, project, peer, count, &address, size);
        if let Some(scene) = scene {
            command.env(SCENE_VAR, scene);
        }
        if peer > 0 && !link.is_empty() {
            command.env(scrap::party::LINK_VAR, link);
        }
        let mut child = command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let label = format!("player {}", peer + 1);
        for stream in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let label = label.clone();
            std::thread::spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(stream)
                    .lines()
                    .map_while(Result::ok)
                {
                    println!("{label}: {line}");
                }
            });
        }
        children.push(child);
    }
    // The host's end is the game's.
    let host = children[0].wait()?;
    for child in &mut children[1..] {
        let _ = child.kill();
        let _ = child.wait();
    }
    Ok(host.success())
}

/// Player `peer` (0 hosts) of `count`, from the built game at `exe`.
pub fn player_command(
    exe: &Path,
    project: &Project,
    peer: u32,
    count: u32,
    address: &str,
    size: (u32, u32),
) -> Command {
    use scrap::party;
    let mut command = Command::new(exe);
    command
        .current_dir(project.root())
        .env(party::PLAYER_VAR, format!("Player {}", peer + 1))
        .env(party::WINDOW_VAR, party::tile(peer, count, size));
    if peer == 0 {
        command.env(party::NET_VAR, format!("host:{address}"));
    } else {
        command.env(party::NET_VAR, format!("join:{address}")).env(
            scrap::player_prefs::USER_DIR_VAR,
            project.root().join(format!(".scrap/players/{}", peer + 1)),
        );
    }
    command
}

/// What the game reads to know which scene to open: `main` without it.
pub const SCENE_VAR: &str = "SCRAP_SCENE";

/// Check that the project has a scene called NAME, wherever it lies, for
/// `--scene NAME`, and say which ones it has when not.
pub fn scene(project: &Project, name: &str) -> Result<String> {
    let names = project.scene_names();
    if names.iter().any(|n| n == name) {
        return Ok(name.to_string());
    }
    let near = scrap::spelling::closest(name, names.iter().map(String::as_str))
        .map(|n| format!(" — did you mean `{n}`?"))
        .unwrap_or_default();
    bail!(
        "no scene called `{name}`{near} (there are: {})",
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> Project {
        let root = std::env::temp_dir().join(format!("scrap-run-{name}"));
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
    fn each_player_is_told_who_it_is_and_where_the_host_is() {
        let project = project("players");
        let exe = Path::new("/game/bin");
        let env = |c: &Command, k: &str| {
            c.get_envs()
                .find(|(key, _)| *key == k)
                .and_then(|(_, v)| v)
                .map(|v| v.to_string_lossy().into_owned())
        };
        let host = player_command(exe, &project, 0, 3, "127.0.0.1:4000", (1280, 720));
        assert_eq!(
            env(&host, "SCRAP_NET").as_deref(),
            Some("host:127.0.0.1:4000")
        );
        assert_eq!(
            env(&host, "SCRAP_WINDOW").as_deref(),
            Some("40,60,640,360")
        );
        assert_eq!(
            env(&host, "SCRAP_USER_DIR"),
            None,
            "the host is the person's own"
        );
        let third = player_command(exe, &project, 2, 3, "127.0.0.1:4000", (1280, 720));
        assert_eq!(
            env(&third, "SCRAP_NET").as_deref(),
            Some("join:127.0.0.1:4000")
        );
        assert_eq!(env(&third, "SCRAP_PLAYER").as_deref(), Some("Player 3"));
        assert!(env(&third, "SCRAP_USER_DIR")
            .unwrap()
            .ends_with(".scrap/players/3"));
        assert!(players(&project, 7, false, None, "").is_err());
        assert!(players(&project, 2, false, None, "ping=3").is_err());
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
