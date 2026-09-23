//! Iteration time, as numbers with budgets.
//!
//! DNA, postulate 1: the time from an edit to seeing it is measured, and a
//! budget is a number a test holds, not a feeling. These are the engine's
//! share of "save the scene → see it in the running game": reading the
//! file, and bringing the world up to date. The GPU upload and the frame
//! come after and are the renderer's budget, not this one's.
//!
//! Measured in a debug build, which is what someone iterating runs, on a
//! scene far bigger than a greybox level. The budgets are set for a slow
//! CI runner; each is the best of several runs, so a runner that stalls
//! once does not fail it, and a real regression — an accidental quadratic,
//! a clone per entity per field — still does.

use std::time::{Duration, Instant};

use runity::{EntityDesc, EntityId, MeshHandle, Scene, Transform};

/// A big level: a thousand props, each with three parts.
const PROPS: usize = 1000;

fn level() -> Scene {
    let mut scene = Scene::default();
    for i in 0..PROPS {
        let part = |j: u64, model: &str| {
            EntityDesc {
                id: EntityId::from_raw((i as u64) * 16 + j + 1),
                name: format!("part {j}"),
                ..EntityDesc::default()
            }
            .with(runity::scene::ModelRef(model.into()))
            .with(runity::scene::MaterialRef::Named("stone".into()))
        };
        let mut prop = part(0, "builtin:cube");
        prop.name = format!("prop {i}");
        prop.transform.position.x = i as f32;
        prop.children = vec![part(1, "builtin:sphere"), part(2, "builtin:cone")];
        scene.entities.push(prop);
    }
    scene
}

/// The best of `runs`, each timed around `work`.
fn best(runs: usize, mut work: impl FnMut()) -> Duration {
    (0..runs)
        .map(|_| {
            let start = Instant::now();
            work();
            start.elapsed()
        })
        .min()
        .unwrap()
}

fn within(what: &str, took: Duration, budget_ms: u64) {
    eprintln!("{what}: {took:.2?} (budget {budget_ms} ms)");
    assert!(
        took <= Duration::from_millis(budget_ms),
        "{what} took {took:.2?}, over its budget of {budget_ms} ms"
    );
}

#[test]
fn reading_a_big_scene_file_is_within_budget() {
    let text =
        ron::ser::to_string_pretty(&level(), ron::ser::PrettyConfig::new().depth_limit(4)).unwrap();
    let path = std::env::temp_dir().join("runity-budget-level.ron");
    std::fs::write(&path, &text).unwrap();
    let took = best(5, || {
        let scene = Scene::load(&path).unwrap();
        assert_eq!(scene.entities.len(), PROPS);
    });
    within(
        &format!("read {} KiB, {} entities", text.len() / 1024, PROPS * 3),
        took,
        150,
    );
}

#[test]
fn a_one_field_edit_to_a_big_scene_reaches_the_world_within_budget() {
    let before = level();
    let mut after = before.clone();
    after.entities[PROPS / 2].transform = Transform {
        position: runity::glam::Vec3::new(0.0, 9.0, 0.0),
        ..Transform::default()
    };
    let mut world = runity::hecs::World::new();
    runity::spawn_scene(&before, &mut world, |_| Some(MeshHandle::TEST));

    // Back and forth, so every run is a real one-field change.
    let mut forward = true;
    let took = best(7, || {
        let (from, to) = if forward {
            (&before, &after)
        } else {
            (&after, &before)
        };
        forward = !forward;
        let patched =
            runity::patch_scene(from, to, &mut world, |_| Some(MeshHandle::TEST), |_| None);
        assert_eq!(patched.updated, 1);
    });
    within(
        &format!("patch one field into {} entities", PROPS * 3),
        took,
        60,
    );
}

#[test]
fn building_a_frame_of_a_big_level_is_within_budget_and_steady() {
    // The CPU's share of a frame: the draw list from the world. The GPU's
    // share depends on the adapter and is measured elsewhere; this one is
    // ours, and a clone per entity or a sort that went quadratic shows here.
    let scene = level();
    let mut world = runity::hecs::World::new();
    runity::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
    let camera = runity::scene_camera(&scene.view());
    let mut times = runity::FrameTimes::new(60);
    for _ in 0..60 {
        let start = Instant::now();
        let frame = runity::build_frame(
            &world,
            camera,
            runity::scene_lighting(&scene.sun()),
            runity::scene_fog(&scene.fog()),
        );
        times.record(start.elapsed());
        assert_eq!(frame.draws.len(), PROPS * 3);
    }
    let summary = times.summary().unwrap();
    eprintln!("build a frame of {} entities: {summary}", PROPS * 3);
    within("the median frame build", summary.median, 5);
}
