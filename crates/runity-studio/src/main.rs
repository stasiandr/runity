//! The editor. See the crate's documentation for what it is.

use std::path::PathBuf;

fn main() {
    let scene = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(runity_studio::REFERENCE_SCENE));
    let session = match runity_studio::open(&scene) {
        Ok(session) => session,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };
    let title = scene
        .file_name()
        .map(|name| format!("runity — {}", name.to_string_lossy()))
        .unwrap_or_else(|| "runity".to_string());
    runity_studio::window::run(session, title);
}
