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

use scrap::physics::PhysicsWorld;
use scrap::render::{Camera, FogSettings, Lighting};
use scrap::scene::{Body, Collider, MaterialRef};
use scrap::{EntityDesc, EntityId, MeshHandle, Scene};

const SIZES: [usize; 4] = [10, 100, 1_000, 10_000];
/// The most the time may grow for ten times the entities.
const MOST: f64 = 30.0;

/// `n` crates on a grid, a tenth of them falling onto a floor.
fn scene(n: usize) -> Scene {
    let side = (n as f32).sqrt().ceil() as usize;
    let mut scene = Scene::default();
    scene.entities.push(
        EntityDesc {
            id: EntityId::from_raw(1),
            name: "floor".into(),
            transform: scrap::Transform {
                scale: glam::Vec3::new(side as f32 * 2.0 + 4.0, 1.0, side as f32 * 2.0 + 4.0),
                ..Default::default()
            },
            ..EntityDesc::default()
        }
        .with(scrap::scene::ModelRef("builtin:plane".into()))
        .with(Body::Static)
        .with(Collider::Box {
            half: glam::Vec3::new(0.5, 0.01, 0.5),
            center: glam::Vec3::ZERO,
        }),
    );
    for i in 0..n - 1 {
        let falling = i % 10 == 0;
        scene.entities.push(
            EntityDesc {
                id: EntityId::from_raw(i as u64 + 2),
                name: format!("crate {i}"),
                transform: scrap::Transform {
                    position: glam::Vec3::new(
                        (i % side) as f32 * 2.0 - side as f32,
                        if falling { 3.0 } else { 0.5 },
                        (i / side) as f32 * 2.0 - side as f32,
                    ),
                    ..Default::default()
                },
                ..EntityDesc::default()
            }
            .with(scrap::scene::ModelRef("builtin:cube".into()))
            .with(MaterialRef::Named("bark".into()))
            .with(if falling { Body::Dynamic } else { Body::Static })
            .with(Collider::Box {
                half: glam::Vec3::splat(0.5),
                center: glam::Vec3::ZERO,
            }),
        );
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
            scrap::spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        })
    });
}

#[test]
fn building_a_frame_grows_linearly() {
    curve("frame", |scene| {
        let mut world = hecs::World::new();
        scrap::spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        best(1, || {
            let frame = scrap::build_frame(
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
        scrap::spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        let mut after = scene.clone();
        after.entities.last_mut().unwrap().transform.position.y += 1.0;
        best(1, || {
            let done = scrap::patch_scene(
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
        scrap::spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        // Bodies built and falling: the steady step, not the first build.
        for _ in 0..3 {
            physics.run(&mut world);
            scrap::world::apply_hierarchy(&mut world);
        }
        best(1, || {
            physics.run(&mut world);
            scrap::world::apply_hierarchy(&mut world);
        })
    });
}

#[test]
fn a_rope_step_grows_linearly() {
    // A rope strung from every tenth crate to the next along, lying on the
    // crates between: ropes and colliders grow together, which is where a
    // rope trying every collider would go quadratic.
    curve("rope step", |scene| {
        let mut scene = scene.clone();
        for line in scene.entities.iter_mut().skip(1).step_by(10) {
            line.set_part(&scrap::soft::Rope {
                to: glam::Vec3::new(2.0, 0.5, 0.0),
                segments: 12,
                ..Default::default()
            });
        }
        let mut world = hecs::World::new();
        scrap::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        scrap::world::apply_hierarchy(&mut world);
        for _ in 0..5 {
            scrap::soft::step(&mut world, 1.0 / 60.0);
        }
        best(1, || scrap::soft::step(&mut world, 1.0 / 60.0))
    });
}

#[test]
fn a_cloth_step_grows_linearly() {
    // A flag of 8×8 on every tenth crate, lying against the crates about
    // it as the wind takes it.
    curve("cloth step", |scene| {
        let mut scene = scene.clone();
        for line in scene.entities.iter_mut().skip(1).step_by(10) {
            line.set_part(&scrap::soft::Cloth {
                size: [1.0, 1.0],
                cells: [8, 8],
                pinned: scrap::soft::Pinned::Left,
                ..Default::default()
            });
        }
        let mut world = hecs::World::new();
        scrap::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        scrap::world::apply_hierarchy(&mut world);
        for _ in 0..5 {
            scrap::soft::step(&mut world, 1.0 / 60.0);
        }
        best(1, || scrap::soft::step(&mut world, 1.0 / 60.0))
    });
}

/// A table of `n` records, a tenth of them on a base and each linking to
/// the one before it — what a catalog of items grows into.
fn table(n: usize) -> String {
    let mut text = String::from("{\n");
    for i in 0..n {
        let base = if i % 10 == 5 {
            format!("base: \"record {}\", ", i - 1)
        } else {
            String::new()
        };
        let before = i.saturating_sub(1);
        text.push_str(&format!(
            "    \"record {i}\": (id: \"{:x}\", {base}weight: {i}.5, tags: [Hard, Warm], after: \"record {before}\"),\n",
            i + 1
        ));
    }
    text.push('}');
    text
}

#[derive(serde::Deserialize)]
#[allow(dead_code)]
enum Tag {
    Hard,
    Warm,
}

#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct Item {
    weight: f32,
    tags: Vec<Tag>,
    after: scrap::Link<Item>,
}

impl scrap::Record for Item {}

#[test]
fn reading_and_checking_a_table_grows_linearly() {
    let texts: Vec<String> = SIZES.iter().map(|&n| table(n)).collect();
    let shapes = vec![scrap::table::TableShape {
        path: "configs/items.ron".into(),
        record: "Item".into(),
        shape: scrap::shape::of::<Item>(),
    }];
    for (what, work) in [
        (
            "read a table",
            &(|text: &str| {
                let table = scrap::Table::<Item>::from_text(text).unwrap();
                assert!(!table.is_empty());
            }) as &dyn Fn(&str),
        ),
        ("check its links", &|text: &str| {
            let read = |_: &str| vec![("configs/items.ron".to_string(), text.to_string())];
            assert!(scrap::table::link_problems(&shapes, &read).is_empty());
        }),
    ] {
        let times: Vec<Duration> = texts.iter().map(|t| best(3, || work(t))).collect();
        let line: Vec<String> = SIZES
            .iter()
            .zip(&times)
            .map(|(n, t)| format!("{n}: {t:.2?}"))
            .collect();
        eprintln!("{what}: {}", line.join(", "));
        for (i, pair) in times.windows(2).enumerate() {
            if pair[1] < Duration::from_micros(500) {
                continue;
            }
            let ratio = pair[1].as_secs_f64() / pair[0].as_secs_f64().max(1e-6);
            assert!(
                ratio <= MOST,
                "{what}: {} to {} records took {ratio:.0}× longer ({:.2?} → {:.2?})",
                SIZES[i],
                SIZES[i + 1],
                pair[0],
                pair[1]
            );
        }
    }
}
