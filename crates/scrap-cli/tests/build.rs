//! What `scrap build` lays out: the game and its data, and no sources.

use scrap::Project;

/// `std::fs::write`, the folders on the way made first: a new project has
/// only the folders its layout needs (docs/layout.md).
#[allow(dead_code)]
fn write_all(path: impl AsRef<std::path::Path>, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

#[test]
fn a_build_carries_the_data_the_game_reads_and_leaves_the_sources_home() {
    let root = std::env::temp_dir().join("scrap-build-package");
    let _ = std::fs::remove_dir_all(&root);
    let project = Project::create(&root, "shipping").unwrap();
    write_all(
        project.assets().join("rock.obj"),
        "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
    )
    .unwrap();
    scrap_import::sync(&project);
    // Stands in for the compiled game: packaging does not care what it is.
    let exe = root.join("fake-game");
    write_all(&exe, "binary").unwrap();

    let out = root.join("build");
    let shipped = scrap_cli::build::package(&project, &exe, &out).unwrap();
    assert_eq!(shipped, out.join("fake-game"));
    for present in [
        "data/scrap.ron",
        "data/config/input.ron",
        "data/content/shipping/core/world.ron",
        "data/content/shipping/maps/main.scene.ron",
    ] {
        assert!(out.join(present).is_file(), "{present} is shipped");
    }
    let built =
        scrap_import::built_for(&project.assets().join("rock.obj"), &project.library()).unwrap();
    let built = out.join("data/library").join(built.file_name().unwrap());
    assert!(
        built.is_file(),
        "the rock's asset is shipped, named by its ID"
    );
    for absent in [
        "data/content/shipping/rock.obj",
        "data/materials",
        "data/src",
        "data/Cargo.toml",
    ] {
        assert!(!out.join(absent).exists(), "{absent} stays home");
    }
    assert!(
        Project::find(out.join("data/content/shipping/maps/main.scene.ron")).is_ok(),
        "the shipped scene finds its project, so its library"
    );

    // A second build replaces the first: a scene deleted from the project
    // does not linger in the next build.
    write_all(
        out.join("data/content/shipping/maps/old.scene.ron"),
        "(entities: [])",
    )
    .unwrap();
    scrap_cli::build::package(&project, &exe, &out).unwrap();
    assert!(!out
        .join("data/content/shipping/maps/old.scene.ron")
        .exists());

    // A sandbox is not the game: developers/ stays home.
    write_all(
        root.join("content/developers/ann/try.scene.ron"),
        "(entities: [])",
    )
    .unwrap();
    scrap_cli::build::package(&project, &exe, &out).unwrap();
    assert!(!out.join("data/content/developers").exists());
}
