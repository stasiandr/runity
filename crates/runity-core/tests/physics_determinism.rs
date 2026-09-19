//! The same recorded input, at two frame rates, tick for tick.
//!
//! Determinism here is not a promise in a doc comment, it is a consequence of
//! where the simulation lives: physics runs only in `fixed_update`, is handed
//! its own `fixed_delta` when the world is built, and never sees a frame delta
//! at all. If that is true, then running a recording at 60 frames per second
//! and at 120 has to produce the *same numbers on the same ticks* — not close,
//! not within a tolerance, the same bits. If some frame-rate-dependent term
//! ever creeps in, this is the test that fails.
//!
//! The scene is deliberately not a body on a plane: a slope past the limit, a
//! wall, two bodies pushing each other and a tree coming down between them, so
//! every pass of [`runity_core::physics::step`] contributes to the numbers
//! being compared.

use runity_core::physics::{
    Blocker, Body, Command, FallingTree, Heightfield, PendingInput, PhysicsWorld, Tuning,
};
use runity_core::{App, Engine, Entity, Game, Transform};
use runity_math::{Vec2, Vec3};
use runity_platform::{HeadlessWindow, WindowConfig};
use std::cell::RefCell;
use std::io;
use std::rc::Rc;

const FIXED_DELTA: f32 = 1.0 / 20.0;

/// One entry of the recording: what to do, and when.
///
/// The time is a wall-clock instant in the simulated run, exactly as a real
/// recording of a player would store it — not a tick index, which would make
/// the test circular by construction.
struct Entry {
    at: f32,
    what: Action,
}

enum Action {
    /// Both walkers are told where to go.
    Walk(Vec3, Vec3),
    Jump,
    /// The axe lands and the tree starts over.
    Fell(Vec2),
}

/// The recording. The instants sit well away from any tick boundary — a
/// 1/20 s tick lands on multiples of 0.05 — so the tiny difference between a
/// float accumulated 60 times a second and 120 cannot move a command into a
/// different tick and make the comparison meaningless.
fn recording() -> Vec<Entry> {
    vec![
        Entry {
            at: 0.033,
            what: Action::Walk(Vec3::new(1.0, 0.0, 0.3), Vec3::new(-0.4, 0.0, 1.0)),
        },
        // A jump on a frame that is nowhere near a tick.
        Entry {
            at: 0.283,
            what: Action::Jump,
        },
        Entry {
            at: 0.533,
            what: Action::Fell(Vec2::new(0.0, 1.0)),
        },
        Entry {
            at: 0.783,
            what: Action::Walk(Vec3::new(-1.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.2)),
        },
        Entry {
            at: 1.283,
            what: Action::Jump,
        },
        Entry {
            at: 1.533,
            what: Action::Walk(Vec3::ZERO, Vec3::new(-1.0, 0.0, -1.0)),
        },
    ]
}

/// Ground that rises along +X steeply enough to be unwalkable in places.
fn slope() -> Heightfield {
    let (cols, rows, cell) = (21usize, 21usize, 4.0f32);
    let origin = Vec2::splat(-40.0);
    let mut heights = Vec::with_capacity(cols * rows);
    for row in 0..rows {
        for col in 0..cols {
            let x = origin.x + col as f32 * cell;
            // A slope in X with a ripple in Z, so the normal is never axis
            // aligned and the slope test has something to say.
            heights.push(x * 0.35 + ((row % 3) as f32) * 0.4);
        }
    }
    Heightfield::new(origin, cell, cols, rows, heights)
}

struct Replay {
    physics: Option<PhysicsWorld>,
    walkers: Vec<Entity>,
    tree: Option<Entity>,
    script: Vec<Entry>,
    /// How far into the recording playback has got.
    cursor: usize,
    /// Every body's position, at the end of every tick.
    log: Rc<RefCell<Vec<Vec<Vec3>>>>,
}

impl Game for Replay {
    fn start(&mut self, engine: &mut Engine) -> io::Result<()> {
        engine.time.fixed_delta = FIXED_DELTA;
        let mut physics = PhysicsWorld::new(slope(), Tuning::default(), FIXED_DELTA);

        for (x, z) in [(-2.0f32, 0.0f32), (-1.6, 0.4)] {
            let y = physics.ground.height_at(x, z);
            let walker = engine.world.spawn();
            engine.world.insert(walker, Body::new(Vec3::new(x, y, z)));
            engine.world.insert(walker, PendingInput::new());
            self.walkers.push(walker);
        }

        // A wall to slide along, and a step to walk over.
        for (position, blocker) in [
            (
                Vec3::new(1.0, physics.ground.height_at(1.0, 0.0), 6.0),
                Blocker::Box {
                    half: Vec2::new(0.3, 6.0),
                    facing: Vec2::new(0.0, 1.0),
                    top: 2.5,
                },
            ),
            (
                Vec3::new(-3.0, physics.ground.height_at(-3.0, 3.0), 3.0),
                Blocker::Cylinder {
                    radius: 1.2,
                    top: 0.3,
                },
            ),
        ] {
            let entity = engine.world.spawn();
            engine
                .world
                .insert(entity, Transform::from_position(position));
            engine.world.insert(entity, blocker);
        }

        // And a tree, to be felled across them mid-recording.
        let base = Vec3::new(-1.0, physics.ground.height_at(-1.0, 2.0), 2.0);
        let tree = engine.world.spawn();
        engine.world.insert(tree, Transform::from_position(base));
        engine.world.insert(
            tree,
            Blocker::Cylinder {
                radius: 0.35,
                top: 6.0,
            },
        );
        engine.world.insert(tree, FallingTree::new(6.0, 0.35));
        self.tree = Some(tree);

        physics.mark_statics_dirty();
        self.physics = Some(physics);
        Ok(())
    }

    fn update(&mut self, engine: &mut Engine) {
        let now = engine.time.elapsed();
        while self.cursor < self.script.len() && self.script[self.cursor].at <= now {
            match self.script[self.cursor].what {
                Action::Walk(a, b) => {
                    for (walker, direction) in self.walkers.iter().zip([a, b]) {
                        engine
                            .world
                            .get_mut::<PendingInput>(*walker)
                            .expect("a walker has one")
                            .push(Command::Move {
                                direction,
                                speed_scale: 1.0,
                            });
                    }
                }
                Action::Jump => {
                    for walker in &self.walkers {
                        engine
                            .world
                            .get_mut::<PendingInput>(*walker)
                            .expect("a walker has one")
                            .push(Command::Jump);
                    }
                }
                Action::Fell(direction) => {
                    engine
                        .world
                        .get_mut::<FallingTree>(self.tree.expect("planted"))
                        .expect("a tree")
                        .topple(direction);
                }
            }
            self.cursor += 1;
        }
    }

    fn fixed_update(&mut self, engine: &mut Engine) {
        self.physics
            .as_mut()
            .expect("start ran")
            .step(&mut engine.world);
        let positions = self
            .walkers
            .iter()
            .map(|w| engine.world.get::<Body>(*w).expect("a body").position)
            .collect();
        self.log.borrow_mut().push(positions);
    }
}

/// Play the recording back over `seconds` of simulated time at `frame_delta`,
/// and return every body's position at the end of every tick.
fn replay(frame_delta: f32, seconds: f32) -> Vec<Vec<Vec3>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    let frames = (seconds / frame_delta).round() as u64;
    let config = WindowConfig::new("replay", 8, 8);
    App::new(config.clone())
        .with_max_frames(frames)
        .with_frame_delta(frame_delta)
        .with_target_fps(None)
        .run_with_window(
            Box::new(HeadlessWindow::new(&config)),
            Replay {
                physics: None,
                walkers: Vec::new(),
                tree: None,
                script: recording(),
                cursor: 0,
                log: Rc::clone(&log),
            },
        )
        .expect("a run against a window in memory cannot fail");
    Rc::try_unwrap(log)
        .expect("the game is dropped by now")
        .into_inner()
}

#[test]
fn the_same_recording_at_sixty_and_at_a_hundred_and_twenty_frames_agrees_bit_for_bit() {
    const SECONDS: f32 = 2.5;
    let at_60 = replay(1.0 / 60.0, SECONDS);
    let at_120 = replay(1.0 / 120.0, SECONDS);

    // 2.5 s of a 20 Hz tick is fifty of them, however the frames were sliced.
    assert_eq!(at_60.len(), 50, "ticks at 60 fps");
    assert_eq!(at_120.len(), 50, "ticks at 120 fps");

    for (tick, (a, b)) in at_60.iter().zip(&at_120).enumerate() {
        assert_eq!(a.len(), b.len(), "tick {tick}: the same bodies");
        for (body, (p, q)) in a.iter().zip(b).enumerate() {
            // Bit-for-bit, not within a tolerance: `assert_eq` on `f32` is
            // exactly the comparison this test is about.
            assert_eq!(
                (p.x.to_bits(), p.y.to_bits(), p.z.to_bits()),
                (q.x.to_bits(), q.y.to_bits(), q.z.to_bits()),
                "tick {tick}, body {body}: 60 fps put it at {p:?} and 120 fps \
                 at {q:?} — something in physics is reading the frame delta"
            );
        }
    }
}

#[test]
fn the_recording_actually_moves_the_bodies_around() {
    // A determinism test over a simulation that never does anything passes for
    // the wrong reason. This is the guard on that: the recording has to have
    // shifted both bodies, lifted them off the ground and set them down.
    let log = replay(1.0 / 60.0, 2.5);
    let first = &log[0];
    let last = log.last().expect("fifty ticks");
    for (body, (start, end)) in first.iter().zip(last).enumerate() {
        let travelled = (*end - *start).length();
        assert!(
            travelled > 1.0,
            "body {body} only moved {travelled} m over the whole recording"
        );
    }

    // And the jumps really left the ground: some tick has a body higher than
    // any ground under it at the start.
    let highest = log
        .iter()
        .flat_map(|tick| tick.iter().map(|p| p.y))
        .fold(f32::NEG_INFINITY, f32::max);
    let lowest = log
        .iter()
        .flat_map(|tick| tick.iter().map(|p| p.y))
        .fold(f32::INFINITY, f32::min);
    assert!(
        highest - lowest > 0.5,
        "the bodies never changed height: {lowest} to {highest}"
    );
}
