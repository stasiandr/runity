//! The scenes and prefabs in the repository are in the form the engine
//! writes.
//!
//! An entity without an ID still loads — it gets one — but a committed file
//! that relies on that gets new IDs every time it is opened and a different
//! diff every time it is saved. The examples are what people and agents
//! copy from, so they carry their IDs, and this is the test that says so
//! when one is added without.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use runity::{EntityDesc, Scene};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/runity is two levels down")
        .to_path_buf()
}

fn files(directory: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory)
        .unwrap_or_else(|e| panic!("{}: {e}", directory.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some(extension))
        .collect();
    found.sort();
    found
}

/// Every ID under these entities, failing on the first one that is missing
/// or repeated.
fn check(entities: &[EntityDesc], file: &Path, seen: &mut HashSet<runity::EntityId>) {
    for entity in entities {
        assert!(
            !entity.id.is_unassigned(),
            "{}: `{}` has no id — give it one, e.g. id: \"{}\"",
            file.display(),
            entity.name,
            runity::EntityId::fresh()
        );
        assert!(
            seen.insert(entity.id),
            "{}: id {} is used twice (second time on `{}`)",
            file.display(),
            entity.id,
            entity.name
        );
        check(&entity.children, file, seen);
    }
}

#[test]
fn every_example_scene_names_every_entity() {
    let scenes = files(&repository().join("scenes"), "ron");
    assert!(!scenes.is_empty());
    for path in scenes {
        // Parsed raw, not through `Scene::load`, which would quietly fill in
        // what is missing — the thing being checked.
        let text = std::fs::read_to_string(&path).unwrap();
        let scene: Scene =
            ron::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        check(&scene.entities, &path, &mut HashSet::new());
    }
}

#[test]
fn every_example_prefab_names_every_entity() {
    let prefabs = files(&repository().join("scenes/prefabs"), "prefab");
    assert!(!prefabs.is_empty());
    for path in prefabs {
        let text = std::fs::read_to_string(&path).unwrap();
        let root: EntityDesc =
            ron::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        check(std::slice::from_ref(&root), &path, &mut HashSet::new());
    }
}
