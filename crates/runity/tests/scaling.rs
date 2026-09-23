//! How the engine's per-frame and per-edit work grows with the number of
//! things in a scene: a curve, not a point.
//!
//! DNA, postulate 6: scale is checked at 10 / 100 / 1 000 / 10 000 entities,
//! to see "it went quadratic" before a player does. A budget at one size
//! (see `iteration_budget.rs`) can hide a quadratic that only bites at the
//! next size up; the ratio between two sizes cannot. Each step here is ten
//! times more entities, so linear work takes about ten times longer and a
//! quadratic about a hundred: the test allows thirty, which a noisy debug
//! runner stays well inside and an accidental `O(n²)` does not.
//!
//! Timings are the best of several runs, printed for whoever reads the log.

use std::time::{Duration, Instant};

use runity::physics::PhysicsWorld;
use runity::render::{Camera, FogSettings, Lighting};
use runity::scene::{Body, Collider, MaterialRef};
use runity::{EntityDesc, EntityId, MeshHandle, Scene};

const SIZES: [usize; 4] = [10, 100, 1_000, 10_000];
/// The most the time may grow for ten times the entities.
const MOST: f64 = 30.0;

/// `n` crates on a grid, a tenth of them falling onto a floor.
fn scene(n: usize) -> Scene {
    let side = (n as f32).sqrt().ceil() as usize;
    let mut scene = Scene::default();
    scene.entities.push(EntityDesc {
        id: EntityId::from_raw(1),
        name: "floor".into(),
        model: "builtin:plane".into(),
        body: Body::Static,
        collider: Collider::Box {
            half: glam::Vec3::new(0.5, 0.01, 0.5),
        },
        transform: runity::Transform {
            scale: glam::Vec3::new(side as f32 * 2.0 + 4.0, 1.0, side as f32 * 2.0 + 4.0),
            ..Default::default()
        },
        ..EntityDesc::default()
    });
    for i in 0..n - 1 {
        let falling = i % 10 == 0;
        scene.entities.push(EntityDesc {
            id: EntityId::from_raw(i as u64 + 2),
            name: format!("crate {i}"),
            model: "builtin:cube".into(),
            material: MaterialRef::Named("bark".into()),
            body: if falling { Body::Dynamic } else { Body::Static },
            collider: Collider::Box {
                half: glam::Vec3::splat(0.5),
            },
            transform: runity::Transform {
                position: glam::Vec3::new(
                    (i % side) as f32 * 2.0 - side as f32,
                    if falling { 3.0 } else { 0.5 },
                    (i / side) as f32 * 2.0 - side as f32,
                ),
                ..Default::default()
            },
            ..EntityDesc::default()
        });
    }
    scene
}

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

/// Time `work` at every size and check each step grows at most [`MOST`]
/// times. `work` gets a scene of that size and returns how long the part
/// being measured took, so setup is not counted.
fn curve(what: &str, mut work: impl FnMut(&Scene) -> Duration) {
    let times: Vec<Duration> = SIZES
        .iter()
        .map(|&n| {
            let scene = scene(n);
            (0..3).map(|_| work(&scene)).min().unwrap()
        })
        .collect();
    let line: Vec<String> = SIZES
        .iter()
        .zip(&times)
        .map(|(n, t)| format!("{n}: {t:.2?}"))
        .collect();
    eprintln!("{what}: {}", line.join(", "));
    // Only the steps where the larger one takes long enough to measure: at
    // ten entities everything is a few microseconds of noise.
    for (i, pair) in times.windows(2).enumerate() {
        if pair[1] < Duration::from_micros(500) {
            continue;
        }
        let ratio = pair[1].as_secs_f64() / pair[0].as_secs_f64().max(1e-6);
        assert!(
            ratio <= MOST,
            "{what}: {} to {} entities took {ratio:.0}× longer ({:.2?} → {:.2?}) — something grew faster than linearly",
            SIZES[i],
            SIZES[i + 1],
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn spawning_a_scene_grows_linearly() {
    curve("spawn", |scene| {
        best(1, || {
            let mut world = hecs::World::new();
            runity::spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        })
    });
}

#[test]
fn building_a_frame_grows_linearly() {
    curve("frame", |scene| {
        let mut world = hecs::World::new();
        runity::spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        best(1, || {
            let frame = runity::build_frame(
                &world,
                Camera::default(),
                Lighting::default(),
                FogSettings::default(),
            );
            assert!(!frame.draws.is_empty());
        })
    });
}

#[test]
fn a_one_line_reload_grows_linearly() {
    // The hot path of postulate 1: one crate moved in the file, the world
    // brought up to date.
    curve("reload one line", |scene| {
        let mut world = hecs::World::new();
        runity::spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        let mut after = scene.clone();
        after.entities.last_mut().unwrap().transform.position.y += 1.0;
        best(1, || {
            let done = runity::patch_scene(
                scene,
                &after,
                &mut world,
                |_| Some(MeshHandle::TEST),
                |_| None,
            );
            assert!(done.updated <= 1);
        })
    });
}

#[test]
fn a_physics_step_grows_linearly() {
    curve("physics step", |scene| {
        let mut world = hecs::World::new();
        runity::spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        // Bodies built and falling: the steady step, not the first build.
        for _ in 0..3 {
            physics.run(&mut world);
            runity::world::apply_hierarchy(&mut world);
        }
        best(1, || {
            physics.run(&mut world);
            runity::world::apply_hierarchy(&mut world);
        })
    });
}
