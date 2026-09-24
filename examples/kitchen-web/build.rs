//! Finds the game's components and systems. Written by `runity new`; not
//! edited. Adding a component is adding a file to src/components/, and the
//! file's name is the name scenes use for it.

use std::fmt::Write;
use std::path::Path;

fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let components = files(&Path::new(&root).join("src/components"));
    let systems = files(&Path::new(&root).join("src/systems"));

    let mut text = String::new();
    for (name, path) in &components {
        let _ = writeln!(
            text,
            "#[path = {path:?}]
pub mod {name};
pub use {name}::{};",
            type_name(name)
        );
    }
    text.push_str(
        "
/// Every component above, by its file name.
pub fn register(components: &mut runity::Components) {
    let _ = &components;
",
    );
    for (name, path) in &components {
        // A component that can be written down is kept: by a save game,
        // and across a hot patch.
        let how = if serializable(path) {
            "register_saved"
        } else {
            "register"
        };
        let _ = writeln!(
            text,
            "    components.{how}::<{}>({name:?});",
            type_name(name)
        );
        // One whose file says `pub const NETWORKED: bool = true;` goes to
        // the other players from whoever owns the entity.
        if networked(path) {
            let _ = writeln!(
                text,
                "    let _ = {name}::NETWORKED;
    components.register_networked::<{}>({name:?});",
                type_name(name)
            );
        }
    }
    text.push_str(
        "}
",
    );
    std::fs::write(Path::new(&out).join("components.rs"), text).unwrap();

    let mut text = String::new();
    for (name, path) in &systems {
        let _ = writeln!(
            text,
            "#[path = {path:?}]
pub mod {name};"
        );
    }
    std::fs::write(Path::new(&out).join("systems.rs"), text).unwrap();
}

/// Whether a component's file derives `Serialize` (not only `Deserialize`).
fn serializable(path: &str) -> bool {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.match_indices("Serialize")
        .any(|(at, _)| !text[..at].ends_with("De"))
}

/// Whether a component's file marks it networked.
fn networked(path: &str) -> bool {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .contains("pub const NETWORKED: bool = true;")
}

/// `(module name, absolute path)` for every .rs file in a folder, sorted.
fn files(dir: &Path) -> Vec<(String, String)> {
    println!("cargo::rerun-if-changed={}", dir.display());
    let mut found: Vec<(String, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "rs"))
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            if !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                panic!(
                    "{}: a component or system file is named in snake_case, like front_door.rs",
                    path.display()
                );
            }
            (name, path.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    found.sort();
    found
}

/// `front_door` is `FrontDoor`.
fn type_name(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}
