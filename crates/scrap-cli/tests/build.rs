//! What `scrap build` lays out: the game and its data, and no sources.

use scrap::Project;

#[test]
fn a_build_carries_the_data_the_game_reads_and_leaves_the_sources_home() {
    let root = std::env::temp_dir().join("scrap-build-package");
    let _ = std::fs::remove_dir_all(&root);
    let project = Project::create(&root, "shipping").unwrap();
    std::fs::write(
        project.assets().join("rock.obj"),
        "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
    )
    .unwrap();
    scrap_import::sync(&project);
    // Stands in for the compiled game: packaging does not care what it is.
    let exe = root.join("fake-game");
    std::fs::write(&exe, "binary").unwrap();

    let out = root.join("build");
    let shipped = scrap_cli::build::package(&project, &exe, &out).unwrap();
    assert_eq!(shipped, out.join("fake-game"));
    for present in [
        "data/scrap.ron",
        "data/input.ron",
        "data/tuning/world.ron",
        "data/scenes/main.ron",
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
        "data/assets",
        "data/materials",
        "data/src",
        "data/Cargo.toml",
    ] {
        assert!(!out.join(absent).exists(), "{absent} stays home");
    }
    assert!(
        Project::find(out.join("data/scenes/main.ron")).is_ok(),
        "the shipped scene finds its project, so its library"
    );

    // A second build replaces the first: a scene deleted from the project
    // does not linger in the next build.
    std::fs::write(out.join("data/scenes/old.ron"), "(entities: [])").unwrap();
    scrap_cli::build::package(&project, &exe, &out).unwrap();
    assert!(!out.join("data/scenes/old.ron").exists());
}
