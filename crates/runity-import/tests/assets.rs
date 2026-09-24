//! Renaming an asset keeps every scene that used it working.
//!
//! On a copy of the example project, with its hand-written scenes: the
//! file and its sidecar move, the asset keeps its ID, each line that named
//! the old stem names the new one, and nothing else in the text changes.
//! And the renames that would quietly change what a scene means are
//! refused before anything moves.

#[allow(unused_imports)]
use runity::prelude::*;
use std::path::{Path, PathBuf};

use runity::refs::AssetRef;
use runity::scene::MaterialRef;
use runity::{Library, Prefabs, Project, Scene};
use runity_import::assets::{rename, usages, usages_of};
use runity_import::{sidecar_for, sync, ImportSettings};

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "library" {
            continue;
        }
        if path.is_dir() {
            copy_tree(&path, &to.join(&name));
        } else {
            std::fs::copy(&path, to.join(&name)).unwrap();
        }
    }
}

/// The example project, copied, with its library built.
fn valley(name: &str) -> (Project, PathBuf) {
    let root = std::env::temp_dir().join(format!("runity-rename-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/valley"),
        &root,
    );
    let project = Project::open(&root).unwrap();
    let built = sync(&project);
    assert!(built.iter().all(|r| r.result.is_ok()), "{built:?}");
    (project, root)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// Lines of `after` not in `before`, for files of equal length.
fn changed(before: &str, after: &str) -> Vec<String> {
    let (a, b): (Vec<&str>, Vec<&str>) = (before.lines().collect(), after.lines().collect());
    assert_eq!(a.len(), b.len(), "line count changed:\n{after}");
    a.iter()
        .zip(&b)
        .filter(|(x, y)| x != y)
        .map(|(_, y)| y.to_string())
        .collect()
}

#[test]
fn renaming_a_material_renames_it_in_every_line_that_used_it() {
    let (project, root) = valley("material");
    let scenes = [
        "scenes/first-light.ron",
        "scenes/camp.ron",
        "prefabs/campfire.prefab",
    ];
    let before: Vec<String> = scenes.iter().map(|s| read(&root.join(s))).collect();
    let id = ImportSettings::load(root.join("materials/stone.rmat.rimport"))
        .unwrap()
        .id;
    let used = usages(&project, Path::new("materials/stone.rmat")).unwrap();
    let total = before
        .iter()
        .map(|t| t.matches("\"stone\"").count())
        .sum::<usize>();
    assert_eq!(used.len(), total, "{used:#?}");

    let done = rename(
        &project,
        Path::new("materials/stone.rmat"),
        Path::new("materials/granite.rmat"),
    )
    .unwrap();

    assert_eq!(
        done.reference,
        Some((
            AssetRef::Material("stone".into()),
            AssetRef::Material("granite".into())
        ))
    );
    assert_eq!(done.rewritten.iter().map(|(_, n)| n).sum::<usize>(), total);
    for (scene, before) in scenes.iter().zip(&before) {
        let after = read(&root.join(scene));
        let lines = changed(before, &after);
        assert_eq!(
            lines.len(),
            before.lines().filter(|l| l.contains("\"stone\"")).count(),
            "{scene}: only the lines that named stone"
        );
        assert!(
            lines.iter().all(|l| l.contains("\"granite\"")),
            "{lines:#?}"
        );
        assert!(!after.contains("\"stone\""), "{after}");
    }

    // The file and its sidecar moved; the asset kept its ID, under its new name.
    assert!(!root.join("materials/stone.rmat").exists());
    assert!(!root.join("materials/stone.rmat.rimport").exists());
    let sidecar = ImportSettings::load(sidecar_for(&root.join("materials/granite.rmat"))).unwrap();
    assert_eq!(sidecar.source, "materials/granite.rmat");
    assert_eq!(sidecar.id, id);
    assert!(
        done.synced.iter().all(|r| r.result.is_ok()),
        "{:?}",
        done.synced
    );
    let (library, _) = Library::open(project.library()).unwrap();
    assert!(library.material_by_name("granite").is_some());
    assert!(
        library.material_by_name("stone").is_none(),
        "no second copy"
    );

    let camp = Scene::load(root.join("scenes/camp.ron")).unwrap();
    assert!(camp
        .flatten()
        .iter()
        .any(|(e, _)| e.material_ref() == MaterialRef::Named("granite".into())));
    assert!(
        usages(&project, Path::new("materials/granite.rmat"))
            .unwrap()
            .len()
            == total,
        "and the new name is found where the old one was"
    );
}

#[test]
fn renaming_a_prefab_renames_its_instances() {
    let (project, root) = valley("prefab");
    let before = read(&root.join("scenes/camp.ron"));
    let done = rename(
        &project,
        Path::new("prefabs/campfire.prefab"),
        Path::new("prefabs/hearth.prefab"),
    )
    .unwrap();
    assert_eq!(
        done.rewritten,
        [(
            "scenes/camp.ron".to_string(),
            before.matches("prefab: \"campfire\"").count()
        )]
    );
    let after = read(&root.join("scenes/camp.ron"));
    assert!(changed(&before, &after)
        .iter()
        .all(|l| l.contains("prefab: \"hearth\"")));
    let (prefabs, problems) = Prefabs::of(&project);
    assert!(problems.is_empty());
    assert!(prefabs.get("hearth").is_some());
    let camp = Scene::load(root.join("scenes/camp.ron")).unwrap();
    let instanced = runity::prefab::instantiate(&camp, &prefabs);
    assert!(instanced.problems.is_empty(), "{:?}", instanced.problems);
}

#[test]
fn renaming_a_model_renames_the_lines_that_draw_it() {
    let (project, root) = valley("model");
    let scene = root.join("scenes/first-light.ron");
    let text = read(&scene).replace(
        "name: \"tree near\",   model: \"builtin:cone\"",
        "name: \"tree near\",   model: \"axe\"",
    );
    std::fs::write(&scene, &text).unwrap();
    rename(
        &project,
        Path::new("assets/models/axe.obj"),
        Path::new("assets/tools/hatchet.obj"),
    )
    .unwrap();
    let after = read(&scene);
    assert_eq!(changed(&text, &after).len(), 1);
    assert!(after.contains("model: \"hatchet\""));
    let (library, _) = Library::open(project.library()).unwrap();
    assert!(library.mesh_by_name("hatchet").is_some());
    assert!(root.join("assets/tools/hatchet.obj.rimport").is_file());
}

#[test]
fn moving_a_material_between_folders_changes_no_scene() {
    let (project, root) = valley("move");
    let before = read(&root.join("scenes/camp.ron"));
    let done = rename(
        &project,
        Path::new("materials/stone.rmat"),
        Path::new("materials/rock/stone.rmat"),
    )
    .unwrap();
    assert_eq!(done.reference, None);
    assert!(done.rewritten.is_empty());
    assert_eq!(read(&root.join("scenes/camp.ron")), before);
    let (library, _) = Library::open(project.library()).unwrap();
    assert!(library.material_by_name("stone").is_some());
}

#[test]
fn a_rename_that_would_change_what_a_line_means_is_refused_and_moves_nothing() {
    let (project, root) = valley("refused");

    // Onto another material's name.
    let e = rename(
        &project,
        Path::new("materials/stone.rmat"),
        Path::new("materials/rock/moss.rmat"),
    )
    .unwrap_err();
    assert!(
        format!("{e:#}").contains("materials/moss.rmat is already called `moss`"),
        "{e:#}"
    );

    // Onto a name lines already use, with no file behind it yet.
    let scene = root.join("scenes/first-light.ron");
    let text = read(&scene).replacen("material: \"needle\"", "material: \"sand\"", 1);
    std::fs::write(&scene, &text).unwrap();
    let e = rename(
        &project,
        Path::new("materials/stone.rmat"),
        Path::new("materials/sand.rmat"),
    )
    .unwrap_err();
    assert!(
        format!("{e:#}").contains("material `sand` is already named by 1 line(s)"),
        "{e:#}"
    );

    // Into another kind of folder.
    let e = rename(
        &project,
        Path::new("materials/stone.rmat"),
        Path::new("assets/stone.rmat"),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("not a rename"), "{e:#}");

    assert!(root.join("materials/stone.rmat").is_file(), "nothing moved");
    assert_eq!(read(&scene), text, "nothing rewritten");
    assert_eq!(
        usages_of(&project, &AssetRef::Material("stone".into()))
            .unwrap()
            .len(),
        usages(&project, Path::new("materials/stone.rmat"))
            .unwrap()
            .len()
    );
}

#[test]
fn moving_a_heightmap_points_its_terrain_at_the_new_place() {
    let (project, root) = valley("heightmap");
    std::fs::create_dir_all(root.join("assets/terrain")).unwrap();
    image::GrayImage::from_fn(8, 8, |x, _| image::Luma([(x * 30) as u8]))
        .save(root.join("assets/terrain/ramp.png"))
        .unwrap();
    let terrain = root.join("assets/terrain/field.rterrain");
    std::fs::write(
        &terrain,
        "(\n    // painted by hand\n    size: (30.0, 30.0), resolution: 8, height: 6.0,\n    heightmap: \"ramp.png\",\n)\n",
    )
    .unwrap();
    assert!(sync(&project).iter().all(|r| r.result.is_ok()));

    let done = rename(
        &project,
        Path::new("assets/terrain/ramp.png"),
        Path::new("assets/paint/valley.png"),
    )
    .unwrap();
    assert_eq!(
        read(&terrain),
        "(\n    // painted by hand\n    size: (30.0, 30.0), resolution: 8, height: 6.0,\n    heightmap: \"../paint/valley.png\",\n)\n"
    );
    assert!(
        done.synced.iter().all(|r| r.result.is_ok()),
        "{:?}",
        done.synced
    );

    // And the terrain moving keeps finding its image.
    rename(
        &project,
        Path::new("assets/terrain/field.rterrain"),
        Path::new("assets/paint/field.rterrain"),
    )
    .unwrap();
    assert!(read(&root.join("assets/paint/field.rterrain")).contains("heightmap: \"valley.png\""));
    let (library, _) = Library::open(project.library()).unwrap();
    assert!(library.mesh_by_name("field").is_some());
}

#[test]
fn the_project_lists_every_asset_with_how_much_it_is_used() {
    let (project, _) = valley("list");
    let entries = runity_import::assets::list(&project).unwrap();
    let stone = entries
        .iter()
        .find(|e| e.file == "materials/stone.rmat")
        .unwrap();
    assert_eq!(stone.kind, "material");
    assert_eq!(stone.name, "stone");
    assert!(stone.built && stone.id.is_some());
    assert_eq!(
        stone.uses,
        usages(&project, Path::new("materials/stone.rmat"))
            .unwrap()
            .len()
    );
    let campfire = entries
        .iter()
        .find(|e| e.file == "prefabs/campfire.prefab")
        .unwrap();
    assert_eq!((campfire.kind, campfire.uses), ("prefab", 3));
    let axe = entries
        .iter()
        .find(|e| e.file == "assets/models/axe.obj")
        .unwrap();
    assert_eq!((axe.kind, axe.uses), ("model", 0));
    assert!(entries.iter().all(|e| !e.file.ends_with(".rimport")));
    let mut sorted = entries.clone();
    sorted.sort_by(|a, b| a.file.cmp(&b.file));
    assert_eq!(sorted, entries, "sorted by file");
}

#[test]
fn deleting_what_is_still_used_is_refused_with_the_lines_and_the_rest_goes() {
    let (project, root) = valley("delete");
    let e = runity_import::assets::delete(&project, Path::new("materials/stone.rmat")).unwrap_err();
    let said = format!("{e:#}");
    assert!(
        said.contains("materials/stone.rmat is still used"),
        "{said}"
    );
    assert!(said.contains("scenes/camp.ron: `kettle`"), "{said}");
    assert!(root.join("materials/stone.rmat").is_file());

    let built =
        runity_import::built_for(&root.join("assets/models/axe.obj"), &project.library()).unwrap();
    assert!(built.is_file());
    runity_import::assets::delete(&project, Path::new("assets/models/axe.obj")).unwrap();
    assert!(!root.join("assets/models/axe.obj").exists());
    assert!(!root.join("assets/models/axe.obj.rimport").exists());
    assert!(!built.exists(), "and its built asset");
    assert!(sync(&project).is_empty(), "nothing left for a sync to find");
}

#[test]
fn a_duplicate_is_a_new_asset_with_the_same_settings() {
    let (project, root) = valley("duplicate");
    let original = ImportSettings::load(root.join("assets/models/axe.obj.rimport")).unwrap();
    let done = runity_import::assets::duplicate(
        &project,
        Path::new("assets/models/axe.obj"),
        Path::new("assets/models/axe_old.obj"),
    )
    .unwrap();
    assert!(done.iter().all(|r| r.result.is_ok()), "{done:?}");
    let copy = ImportSettings::load(root.join("assets/models/axe_old.obj.rimport")).unwrap();
    assert_eq!(copy.source, "assets/models/axe_old.obj");
    assert_eq!(copy.scale, original.scale);
    assert_ne!(copy.asset_id(), original.asset_id(), "an asset of its own");
    assert_eq!(copy.hash, original.hash, "the same bytes");
    let (library, _) = Library::open(project.library()).unwrap();
    assert!(library.mesh_by_name("axe_old").is_some());
    assert!(library.mesh_by_name("axe").is_some());

    // A copy under the same name elsewhere would be two `axe`s.
    let e = runity_import::assets::duplicate(
        &project,
        Path::new("assets/models/axe.obj"),
        Path::new("assets/tools/axe.obj"),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("already called `axe`"), "{e:#}");
}

#[test]
fn two_sources_with_one_name_in_two_folders_are_two_assets() {
    let root = std::env::temp_dir().join(format!("runity-same-name-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = runity::Project::create(&root, "same-name").unwrap();
    let square = "v 0 0 0\nv 1 0 0\nv 1 0 1\nf 1 2 3\n";
    let bigger = "v 0 0 0\nv 4 0 0\nv 4 0 4\nf 1 2 3\n";
    for (folder, text) in [("rocks", square), ("cliffs", bigger)] {
        let dir = root.join("assets").join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rock.obj"), text).unwrap();
    }
    runity_import::sync(&project);
    let a =
        runity_import::built_for(&root.join("assets/rocks/rock.obj"), &project.library()).unwrap();
    let b =
        runity_import::built_for(&root.join("assets/cliffs/rock.obj"), &project.library()).unwrap();
    assert_ne!(a, b, "two files in the library");
    assert!(a.is_file() && b.is_file(), "neither wrote over the other");

    // A library from before assets were named by ID is rebuilt by ID.
    std::fs::write(project.library().join("rock.obj.rasset"), b"old").unwrap();
    runity_import::sync(&project);
    assert!(!project.library().join("rock.obj.rasset").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn prefabs_scenes_graphs_and_screens_get_an_id_that_follows_a_move() {
    let root = std::env::temp_dir().join(format!("runity-identify-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = runity::Project::create(&root, "identify").unwrap();
    std::fs::write(root.join("prefabs/door.prefab"), "(name: \"door\")").unwrap();
    std::fs::create_dir_all(root.join("animators")).unwrap();
    std::fs::write(
        root.join("animators/hero.ron"),
        "(start: \"idle\", states: {}, transitions: [])",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("ui")).unwrap();
    std::fs::write(root.join("ui/menu.ron"), "(elements: [])").unwrap();

    let made = runity_import::identify(&project);
    assert!(made.len() >= 3, "{made:?}");
    let id_of = |path: &std::path::Path| {
        runity_import::ImportSettings::load(runity_import::sidecar_for(path))
            .unwrap()
            .asset_id()
    };
    let door = id_of(&root.join("prefabs/door.prefab"));
    let sidecar = std::fs::read_to_string(root.join("prefabs/door.prefab.rimport")).unwrap();
    assert!(
        !sidecar.contains("hash"),
        "no hash to change with every save: {sidecar}"
    );
    for scene in std::fs::read_dir(project.scenes()).unwrap().flatten() {
        let path = scene.path();
        if path.extension().is_some_and(|e| e == "ron") {
            assert!(
                runity_import::sidecar_for(&path).is_file(),
                "{}",
                path.display()
            );
        }
    }
    assert!(
        runity_import::identify(&project).is_empty(),
        "the second time, nothing"
    );

    // Moved to another folder in Finder, without its sidecar: the sidecar
    // follows, and the ID is the same.
    std::fs::create_dir_all(root.join("prefabs/props")).unwrap();
    std::fs::rename(
        root.join("prefabs/door.prefab"),
        root.join("prefabs/props/door.prefab"),
    )
    .unwrap();
    let moved = runity_import::identify(&project);
    assert!(
        matches!(moved[0].change, runity_import::Change::Moved { .. }),
        "{moved:?}"
    );
    assert_eq!(id_of(&root.join("prefabs/props/door.prefab")), door);
    assert!(!root.join("prefabs/door.prefab.rimport").exists());
    let _ = std::fs::remove_dir_all(&root);
}
