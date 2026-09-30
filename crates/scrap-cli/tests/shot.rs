//! `scrap frame-debug --seconds`: a scene played before its picture is
//! taken, as the game's fixed steps play it — a character in its clip's
//! pose rather than its bind pose.

use scrap::Project;

#[test]
fn a_scene_played_a_moment_shows_its_skinned_model_in_its_clips_pose() {
    let root = std::env::temp_dir().join("scrap-shot-played");
    let _ = std::fs::remove_dir_all(&root);
    let project = Project::create(&root, "played").unwrap();
    let assets = project.assets();
    std::fs::create_dir_all(&assets).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scrap-import/tests/fixtures/skinned_banner.gltf");
    std::fs::copy(&fixture, assets.join("banner.gltf")).unwrap();
    // Beside the model, not in `animators/`: found by its extension.
    std::fs::write(
        assets.join("wave.animator.ron"),
        r#"(start: "furl", states: { "furl": (clip: "furl") })"#,
    )
    .unwrap();
    scrap_import::sync(&project);
    let scene = project.scenes().join("flag.scene.ron");
    std::fs::create_dir_all(scene.parent().unwrap()).unwrap();
    std::fs::write(
        &scene,
        r#"(view: (position: (0.0, 1.0, 4.0), target: (0.0, 1.0, 0.0)), entities: [
            (id: "b1", name: "banner", model: "banner", animator: "wave"),
            (id: "b2", name: "mute", model: "banner", animator: "wav"),
        ])"#,
    )
    .unwrap();

    let Ok(mut shot) = scrap::shot::Shot::open(&scene, 96, 64, None) else {
        eprintln!("skipping: no adapter");
        return;
    };
    shot.draw(2);
    let still = shot.pixels();
    let said = shot.play(0.4);
    shot.draw(2);
    assert_ne!(shot.pixels(), still, "the banner furls");
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].contains("`wav`"), "an animator that is not there is said: {said:?}");
    assert!(shot.problems.contains(&said[0]));
}
