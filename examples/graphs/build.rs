//! Finds the game's components and systems. Written by `scrap new`; not
//! edited.
//!
//! Code lies by feature: `src/cooking/pot.rs`, `src/cooking/cook.rs`. A file
//! in a folder under `src/` that declares the struct of its name
//! (`pot.rs`, `pub struct Pot`) is a component, and its file name is the
//! name scenes use for it. One with `pub fn run(` is a system; `step` in
//! main.rs runs them in order. The rest are the game's own modules. A
//! folder with a `mod.rs` is an ordinary module main.rs declares, and so
//! are the files right in `src/`: neither is looked into.

use std::fmt::Write;
use std::path::{Path, PathBuf};

fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let src = Path::new(&root).join("src");
    let (components, systems) = find(&src);

    let mut text = String::new();
    for (name, path) in &components {
        let _ = writeln!(text, "#[path = {path:?}]
pub mod {name};
pub use {name}::{};", type_name(name));
    }
    text.push_str("
/// Every component above, by its file name.
pub fn register(components: &mut scrap::Components) {
    let _ = &components;
    // The engine's simulations' network states (docs/netsim.md).
    scrap::netsim::register(components);
");
    for (name, path) in &components {
        // A component that can be written down is kept: by a save game,
        // and across a hot patch.
        let how = if serializable(path) { "register_saved" } else { "register" };
        let _ = writeln!(text, "    components.{how}::<{}>({name:?});", type_name(name));
        // One whose file says `pub const NETWORKED: bool = true;` goes to
        // the other players from whoever owns the entity.
        if networked(path) {
            let _ = writeln!(text, "    let _ = {name}::NETWORKED;
    components.register_networked::<{}>({name:?});", type_name(name));
        }
    }
    text.push_str("}
");
    std::fs::write(Path::new(&out).join("components.rs"), text).unwrap();

    let mut text = String::new();
    for (name, path) in &systems {
        let _ = writeln!(text, "#[path = {path:?}]
pub mod {name};");
    }
    std::fs::write(Path::new(&out).join("systems.rs"), text).unwrap();
}

/// Whether a component's file derives `Serialize` (not only `Deserialize`).
fn serializable(path: &str) -> bool {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.match_indices("Serialize").any(|(at, _)| !text[..at].ends_with("De"))
}

/// Whether a component's file marks it networked.
fn networked(path: &str) -> bool {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .contains("pub const NETWORKED: bool = true;")
}

/// The components and the systems under `src`, as `(module name, absolute
/// path)`, sorted. Two of either with one name is an error: a scene could
/// not tell the components apart, and `step` the systems.
fn find(src: &Path) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let mut components = Vec::new();
    let mut systems = Vec::new();
    let mut folders: Vec<PathBuf> = Vec::new();
    println!("cargo::rerun-if-changed={}", src.display());
    for entry in std::fs::read_dir(src).into_iter().flatten().flatten() {
        if entry.path().is_dir() {
            folders.push(entry.path());
        }
    }
    while let Some(folder) = folders.pop() {
        println!("cargo::rerun-if-changed={}", folder.display());
        if folder.join("mod.rs").is_file() {
            continue;
        }
        // A project laid out before: the folder says which.
        let only = folder.file_name().and_then(|n| n.to_str()).filter(|n| *n == "components" || *n == "systems").map(str::to_string);
        for path in std::fs::read_dir(&folder).into_iter().flatten().flatten().map(|e| e.path()) {
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let component = match only.as_deref() {
                Some(which) => which == "components",
                None => declares(&name, &text),
            };
            let system = match only.as_deref() {
                Some(which) => which == "systems",
                None => !component && text.contains("pub fn run("),
            };
            if !(component || system) {
                continue;
            }
            if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
                panic!("{}: a component or system file is named in snake_case, like front_door.rs", path.display());
            }
            let found = (name, path.to_string_lossy().replace('\\', "/"));
            if component { components.push(found) } else { systems.push(found) }
        }
    }
    for list in [&mut components, &mut systems] {
        list.sort();
        for pair in list.windows(2) {
            if pair[0].0 == pair[1].0 {
                panic!("two files are called {}.rs: {} and {} — a scene or `step` could not tell them apart; rename one", pair[0].0, pair[0].1, pair[1].1);
            }
        }
    }
    (components, systems)
}

/// Whether `name.rs` declares the struct of its name.
fn declares(name: &str, text: &str) -> bool {
    let decl = format!("pub struct {}", type_name(name));
    text.match_indices(&decl).any(|(at, _)| !text[at + decl.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_'))
}

/// `front_door` is `FrontDoor`.
fn type_name(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
        })
        .collect()
}
