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

use scrap::{EntityDesc, Scene};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/scrap is two levels down")
        .to_path_buf()
}

/// Every ID under these entities, failing on the first one that is missing
/// or repeated.
fn check(entities: &[EntityDesc], file: &Path, seen: &mut HashSet<scrap::EntityId>) {
    for entity in entities {
        assert!(
            !entity.id.is_unassigned(),
            "{}: `{}` has no id — give it one, e.g. id: \"{}\"",
            file.display(),
            entity.name,
            scrap::EntityId::fresh()
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
    let scenes = scrap::layout::files(
        repository().join("examples/valley"),
        scrap::layout::Kind::Scene,
    );
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
    let prefabs = scrap::layout::files(
        repository().join("examples/valley"),
        scrap::layout::Kind::Prefab,
    );
    assert!(!prefabs.is_empty());
    for path in prefabs {
        let text = std::fs::read_to_string(&path).unwrap();
        let root: EntityDesc =
            ron::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        check(std::slice::from_ref(&root), &path, &mut HashSet::new());
    }
}

#[test]
fn the_example_is_a_project_in_the_standard_layout() {
    // It is what people and agents copy from, so it has to look like what
    // the generator makes — or the first copy teaches the wrong layout.
    let root = repository().join("examples/valley");
    let project = scrap::Project::open(&root).expect("examples/valley is a project");
    assert_eq!(project.name(), "valley");
    // Unreal's scheme (docs/layout.md): the project's own folder in
    // content/, its levels in maps/, and no folder per kind at the root.
    assert!(!project.is_legacy(), "examples/valley has content/");
    assert!(
        project.scenes().is_dir(),
        "{} is missing",
        project.scenes().display()
    );
    for old in [
        "scenes",
        "prefabs",
        "materials",
        "assets",
        "shaders",
        "configs",
    ] {
        assert!(!root.join(old).exists(), "{old}/ is the old layout");
    }
    assert!(root.join("content/developers").is_dir());
    assert!(
        std::fs::read_to_string(root.join(".gitignore"))
            .unwrap()
            .contains("/library/"),
        "the built library is derived and must not be committed"
    );
}
