//! Streaming: a region a hundred metres off comes into the world as the
//! camera walks up to it — its three crates spawned, their mesh uploaded —
//! and goes again, mesh and all, once the camera walks away.

use scrap::glam::Vec3;
use scrap::streaming::{StreamEvent, Streamed, Streamer};
use scrap::{Gpu, OffscreenTarget, Prefabs, Renderer, Scene};

#[test]
fn a_region_comes_in_as_the_camera_nears_it_and_goes_as_it_leaves() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let dir = std::env::temp_dir().join(format!("scrap-stream-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("yard.ron"),
        r#"(
    entities: [
        (id: "a100000000000001", name: "crate 1", model: "builtin:cube", transform: (position: (99.0, 0.5, 0.0))),
        (id: "a100000000000002", name: "crate 2", model: "builtin:cube", transform: (position: (100.0, 0.5, 0.0))),
        (id: "a100000000000003", name: "crate 3", model: "builtin:cube", transform: (position: (101.0, 0.5, 0.0))),
    ],
)"#,
    )
    .unwrap();
    let world_scene: Scene = scrap::ron::from_str(
        r#"(
    entities: [
        (id: "b100000000000001", name: "yard", transform: (position: (100.0, 0.0, 0.0)), stream: (scene: "yard", radius: 20.0, keep: 5.0)),
    ],
)"#,
    )
    .unwrap();
    let target = OffscreenTarget::new(&gpu, 32, 32);
    let mut renderer = Renderer::new(&gpu, &target);
    let mut world = scrap::hecs::World::new();
    scrap::spawn_scene(&world_scene, &mut world, |_| None);
    scrap::world::apply_hierarchy(&mut world);
    let streamed = |world: &scrap::hecs::World| world.query::<&Streamed>().iter().count();
    let prefabs = Prefabs::default();
    let mut streamer = Streamer::new();
    let meshes = renderer.mesh_count();

    // Far: nothing comes in.
    let events = streamer.update(&mut world, Vec3::ZERO, &dir, &prefabs, None, &gpu, &mut renderer);
    assert!(events.is_empty() && streamed(&world) == 0);
    // Near: the yard, its crates, their one mesh.
    let events = streamer.update(&mut world, Vec3::new(85.0, 1.0, 0.0), &dir, &prefabs, None, &gpu, &mut renderer);
    assert_eq!(events, vec![StreamEvent::In("yard".into(), 3)]);
    assert_eq!(streamed(&world), 3);
    assert_eq!(renderer.mesh_count(), meshes + 1);
    // Just past the radius, inside `keep`: it stays.
    let events = streamer.update(&mut world, Vec3::new(78.0, 1.0, 0.0), &dir, &prefabs, None, &gpu, &mut renderer);
    assert!(events.is_empty() && streamed(&world) == 3);
    // Away: gone, and its mesh with it.
    let events = streamer.update(&mut world, Vec3::ZERO, &dir, &prefabs, None, &gpu, &mut renderer);
    assert_eq!(events, vec![StreamEvent::Out("yard".into())]);
    assert_eq!(streamed(&world), 0);
    assert_eq!(renderer.mesh_count(), meshes);
    assert_eq!(streamer.loaded(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}
