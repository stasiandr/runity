//! What a tick costs, at the size the valley is and at two sizes it is not.
//!
//! Run it with output:
//!
//! ```text
//! cargo test -p runity-core --test physics_cost -- --nocapture
//! ```
//!
//! There is no benchmark crate here and there is not going to be one — this
//! engine has no third-party dependencies at all. `Instant` and a loop measure
//! a 20 Hz tick perfectly well.
//!
//! Only the twelve-body case asserts, and it asserts loosely. The numbers below
//! come out of an ordinary `cargo test`, which is a debug build; the figure
//! that matters for the game is the release one, and pinning a release figure
//! in an assert on shared CI hardware buys a flaky test rather than a fast
//! engine. The release target — **under 10 µs per tick at twelve bodies**, and
//! about 8 µs when this was written — is recorded here in prose rather than in
//! an assert. The assert only catches the kind of regression that changes the
//! shape of the cost rather than its constant: a body-body pass gone quadratic
//! in the statics, say, or the static grid being rebuilt every tick instead of
//! on a dirty flag.

use runity_core::physics::{
    Blocker, Body, Command, Heightfield, PendingInput, PhysicsWorld, Tuning,
};
use runity_core::{Transform, World};
use runity_math::{Vec2, Vec3};
use std::time::Instant;

const FIXED_DELTA: f32 = 1.0 / 20.0;

/// Ground with a gentle roll, so the slope and support passes do real work
/// rather than short-circuiting on a plane.
fn ground(half_extent: f32) -> Heightfield {
    let cell = 4.0f32;
    let cols = ((half_extent * 2.0 / cell).ceil() as usize) + 1;
    let origin = Vec2::splat(-half_extent);
    let mut heights = Vec::with_capacity(cols * cols);
    for row in 0..cols {
        for col in 0..cols {
            // Deterministic and varied, without a single call to `sin`.
            let ripple = ((row * 7 + col * 5) % 11) as f32 * 0.12;
            heights.push(ripple + ((col * 3 + row * 2) % 7) as f32 * 0.08);
        }
    }
    Heightfield::new(origin, cell, cols, cols, heights)
}

/// A world with `bodies` walkers and a proportional scattering of statics.
///
/// The bodies are packed close enough to actually meet each other — a
/// benchmark of bodies that never touch measures the wrong pass — and each one
/// is walking, so no tick is a no-op.
fn populate(bodies: usize) -> (PhysicsWorld, World) {
    // Roughly one body per four square metres, whatever the count.
    let side = (bodies as f32).sqrt().ceil();
    let half_extent = (side * 2.0).max(12.0);
    let mut physics = PhysicsWorld::new(ground(half_extent + 8.0), Tuning::default(), FIXED_DELTA);
    let mut world = World::new();

    let columns = side as usize;
    for index in 0..bodies {
        let x = -half_extent + (index % columns) as f32 * 2.0;
        let z = -half_extent + (index / columns) as f32 * 2.0;
        let y = physics.ground.height_at(x, z);
        let entity = world.spawn();
        world.insert(entity, Body::new(Vec3::new(x, y, z)));
        let mut pending = PendingInput::new();
        // Fanned out, so the body-body pass sees approaches and not a column.
        pending.push(Command::Move {
            direction: Vec3::new(
                ((index % 5) as f32 - 2.0) * 0.5,
                0.0,
                (index % 3) as f32 - 1.0,
            ),
            speed_scale: 1.0,
        });
        world.insert(entity, pending);
    }

    // One static per body, over the same ground: trees to walk round and logs
    // to walk over, in the proportion a forest has.
    for index in 0..bodies {
        let x = -half_extent + ((index * 7) % columns.max(1)) as f32 * 2.0 + 1.0;
        let z = -half_extent + ((index * 3) / columns.max(1)) as f32 * 2.0 + 1.0;
        let entity = world.spawn();
        world.insert(
            entity,
            Transform::from_position(Vec3::new(x, physics.ground.height_at(x, z), z)),
        );
        world.insert(
            entity,
            if index % 3 == 0 {
                Blocker::Box {
                    half: Vec2::new(3.0, 0.35),
                    facing: Vec2::new(1.0, 0.0),
                    top: 0.3,
                }
            } else {
                Blocker::Cylinder {
                    radius: 0.35,
                    top: 6.0,
                }
            },
        );
    }
    physics.mark_statics_dirty();
    (physics, world)
}

/// Microseconds per tick, averaged over `ticks` of them after a warm-up.
fn microseconds_per_tick(bodies: usize, ticks: u32) -> f64 {
    let (mut physics, mut world) = populate(bodies);
    // The first tick rebuilds the static grid and touches every page for the
    // first time; averaging that in would measure startup, not steady state.
    for _ in 0..5 {
        physics.step(&mut world);
    }
    let started = Instant::now();
    for _ in 0..ticks {
        physics.step(&mut world);
    }
    started.elapsed().as_secs_f64() * 1e6 / ticks as f64
}

#[test]
fn a_tick_costs_what_the_design_says_it_does() {
    // Fewer ticks as the body count grows: 800 bodies is 319,600 pairs, and
    // the point is the per-tick figure, not a long run.
    let measurements: Vec<(usize, f64)> = [(12usize, 400u32), (100, 200), (800, 40)]
        .into_iter()
        .map(|(bodies, ticks)| (bodies, microseconds_per_tick(bodies, ticks)))
        .collect();

    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    eprintln!("physics::step, microseconds per tick ({build} build):");
    for (bodies, cost) in &measurements {
        eprintln!("  {bodies:>4} bodies: {cost:>9.2} us/tick");
    }

    let twelve = measurements[0].1;
    // The valley's own size. The release figure this stands in for is under
    // 10 us; this bound is loose enough that only a change in the shape of the
    // cost trips it, and tight enough that such a change does.
    assert!(
        twelve < 500.0,
        "twelve bodies cost {twelve:.2} us per tick, which is far more than \
         even a debug build of this should need"
    );
}
