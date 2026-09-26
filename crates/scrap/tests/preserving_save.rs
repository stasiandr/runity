//! Saving a hand-written scene changes only what was changed.
//!
//! The example scenes are written by hand: comments explaining each object,
//! trees lined up in columns on one line each. The editor saving one of
//! them must not turn that into the serializer's layout. DNA, postulate 2:
//! saving without a change is a zero diff, and a change is a diff of that
//! change — which these tests check against the real files, line by line.

use std::path::{Path, PathBuf};

use scrap::{EntityDesc, Prefabs, Scene};

fn example(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/valley")
        .join(relative)
}

/// A copy of an example file to save over.
fn copy(relative: &str, name: &str) -> (PathBuf, String) {
    let text = std::fs::read_to_string(example(relative)).unwrap();
    let dir = std::env::temp_dir().join(format!("scrap-preserve-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(Path::new(relative).file_name().unwrap());
    std::fs::write(&path, &text).unwrap();
    (path, text)
}

/// The lines that differ, as (before, after), for files of equal length.
fn changed_lines(before: &str, after: &str) -> Vec<(String, String)> {
    let (a, b): (Vec<&str>, Vec<&str>) = (before.lines().collect(), after.lines().collect());
    assert_eq!(a.len(), b.len(), "line count changed:\n{after}");
    a.iter()
        .zip(&b)
        .filter(|(x, y)| x != y)
        .map(|(x, y)| (x.to_string(), y.to_string()))
        .collect()
}

#[test]
fn saving_a_hand_written_scene_unchanged_writes_nothing_new() {
    for scene in [
        "content/valley/maps/first-light.scene.ron",
        "content/valley/maps/camp.scene.ron",
    ] {
        let (path, original) = copy(scene, "unchanged");
        Scene::load(&path).unwrap().save(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original, "{scene}");
    }
}

#[test]
fn moving_one_tree_changes_its_line_and_no_other() {
    let (path, original) = copy("content/valley/maps/first-light.scene.ron", "move");
    let mut scene = Scene::load(&path).unwrap();
    let id = scene.find("tree mid").unwrap().id;
    scene.get_mut(id).unwrap().transform.position.x = 2.5;
    scene.save(&path).unwrap();

    let saved = std::fs::read_to_string(&path).unwrap();
    let changed = changed_lines(&original, &saved);
    assert_eq!(changed.len(), 1, "{changed:#?}");
    assert!(changed[0].0.contains("\"tree mid\""), "{changed:#?}");
    assert!(changed[0].1.contains("2.5"), "{changed:#?}");
    assert!(saved.contains("// The ground."), "comments stay");
    assert_eq!(
        Scene::load(&path).unwrap(),
        scene,
        "and it reads back as saved"
    );
}

#[test]
fn a_new_entity_is_added_and_a_removed_one_goes_leaving_the_rest() {
    let (path, original) = copy("content/valley/maps/first-light.scene.ron", "add-remove");
    let mut scene = Scene::load(&path).unwrap();
    let far = scene.find("tree far").unwrap().id;
    scrap::edit::remove(&mut scene, far);
    scrap::edit::add(
        &mut scene,
        None,
        EntityDesc {
            name: "stump".into(),
            ..EntityDesc::default()
        }
        .with(scrap::scene::ModelRef("builtin:cone".into())),
    );
    scene.save(&path).unwrap();

    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("\"tree far\""));
    assert!(saved.contains("\"stump\""));
    // Every other line of the original is still there, in order.
    let kept: Vec<&str> = original
        .lines()
        .filter(|line| !line.contains("\"tree far\""))
        .collect();
    let mut rest = saved.lines();
    for line in kept {
        assert!(
            rest.any(|l| l == line),
            "lost or reordered: {line:?}\n{saved}"
        );
    }
    assert_eq!(Scene::load(&path).unwrap(), scene);
}

#[test]
fn a_first_save_writes_missing_ids_into_the_lines_as_they_are() {
    let dir = std::env::temp_dir().join("scrap-preserve-ids");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.ron");
    let original = "(\n    entities: [\n        // a tree\n        (name: \"tree\", model: \"builtin:cone\"),\n        (name: \"rock\", model: \"builtin:sphere\", material: \"stone\"),\n    ],\n)\n";
    std::fs::write(&path, original).unwrap();

    let scene = Scene::load(&path).unwrap();
    scene.save(&path).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    let ids = scene.ids();
    assert_eq!(
        saved,
        format!(
            "(\n    entities: [\n        // a tree\n        (id: \"{}\", name: \"tree\", model: \"builtin:cone\"),\n        (id: \"{}\", name: \"rock\", model: \"builtin:sphere\", material: \"stone\"),\n    ],\n)\n",
            ids[0], ids[1]
        )
    );
}

#[test]
fn a_prefab_keeps_its_comments_through_a_save() {
    let (path, original) = copy("content/valley/camp/campfire/campfire.prefab", "prefab");
    let (_, desc) = Prefabs::read(&path).unwrap();
    Prefabs::save(&desc, &path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
}
