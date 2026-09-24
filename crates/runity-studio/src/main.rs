//! The editor. See the crate's documentation for what it is.

use std::path::{Path, PathBuf};

fn main() {
    let bundled = in_app_bundle();
    if bundled {
        adopt_login_path();
    }
    let Some(scene) = scene_to_open(bundled) else {
        return;
    };
    let session = match runity_studio::open(&scene) {
        Ok(session) => session,
        Err(message) => {
            eprintln!("{message}");
            if bundled {
                rfd::MessageDialog::new()
                    .set_title("runity")
                    .set_description(&message)
                    .show();
            }
            std::process::exit(1);
        }
    };
    remember(&scene);
    let title = scene
        .file_name()
        .map(|name| format!("runity — {}", name.to_string_lossy()))
        .unwrap_or_else(|| "runity".to_string());
    runity_studio::window::run(session, title);
}

/// The scene named on the command line; else, from the repository, the
/// reference scene; else the scene opened last; else one the person picks.
/// `None` when they cancel the picker.
fn scene_to_open(bundled: bool) -> Option<PathBuf> {
    if let Some(arg) = std::env::args().nth(1) {
        return Some(PathBuf::from(arg));
    }
    let reference = PathBuf::from(runity_studio::REFERENCE_SCENE);
    if !bundled && reference.is_file() {
        return Some(reference);
    }
    if let Some(last) = last_opened().filter(|p| p.is_file()) {
        return Some(last);
    }
    if !bundled {
        return Some(reference);
    }
    rfd::FileDialog::new()
        .set_title("Open a scene")
        .add_filter("scene", &["ron"])
        .pick_file()
}

/// Started from `Runity.app`: Finder gave it `/` as the directory and a
/// PATH without cargo, git or Blender.
fn in_app_bundle() -> bool {
    std::env::current_exe()
        .map(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"))
        .unwrap_or(false)
}

/// The PATH the person's login shell has, so Play finds cargo the way it
/// does in a terminal. `~/.cargo/bin` is added when the shell did not.
fn adopt_login_path() {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let from_shell = std::process::Command::new(shell)
        .args(["-l", "-c", "printf %s \"$PATH\""])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .filter(|path| !path.trim().is_empty());
    let mut path = from_shell.unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
    if let Ok(home) = std::env::var("HOME") {
        let cargo = format!("{home}/.cargo/bin");
        if !path.split(':').any(|p| p == cargo) {
            path = format!("{cargo}:{path}");
        }
    }
    std::env::set_var("PATH", path);
}

/// Where the last scene opened at start is written, outside any project.
fn memory_file() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(home.join("Library/Application Support/runity/last-scene"))
}

fn last_opened() -> Option<PathBuf> {
    let text = std::fs::read_to_string(memory_file()?).ok()?;
    Some(PathBuf::from(text.trim()))
}

fn remember(scene: &Path) {
    let (Some(file), Ok(scene)) = (memory_file(), scene.canonicalize()) else {
        return;
    };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, scene.to_string_lossy().as_bytes());
}
