//! The race between a frame and a tick, run through the real main loop.
//!
//! [`runity_core::Input`] clears its press and release edges at the top of
//! every frame, and the fixed tick is slower than the frame rate — 20 Hz
//! against 60. So a `Space` that goes down during a frame which runs no fixed
//! step has its edge wiped before any tick can see it, and two jumps in three
//! never happen.
//!
//! [`PendingInput`] is the latch that fixes it, and this file is the test that
//! the latch is actually needed: the same press, on the same frame, through the
//! same loop, is driven into two games that differ only in where they read the
//! key. The one that reads it in `update` jumps; the one that reads it in
//! `fixed_update` does not.

use runity_core::physics::{Body, Command, Heightfield, PendingInput, PhysicsWorld, Tuning};
use runity_core::{App, Engine, Entity, Game};
use runity_math::{Vec2, Vec3};
use runity_platform::{Event, Key, Window, WindowConfig};
use std::cell::RefCell;
use std::io;
use std::rc::Rc;

/// The valley's numbers: 20 Hz physics under 60 Hz frames.
const FIXED_DELTA: f32 = 1.0 / 20.0;
const FRAME_DELTA: f32 = 1.0 / 60.0;

/// A window that presses a key on one chosen frame and releases it on the
/// next — a single press edge, at a frame of the test's choosing.
struct Tapper {
    press_on: u64,
    frames: u64,
}

impl Window for Tapper {
    fn size(&self) -> (u32, u32) {
        (8, 8)
    }

    fn poll_events(&mut self) -> io::Result<Vec<Event>> {
        // `poll_events` runs at the top of a frame, before `Time::advance`
        // numbers it, so this counter and `Time::frame` agree.
        self.frames += 1;
        Ok(if self.frames == self.press_on {
            vec![Event::KeyDown(Key::Space)]
        } else if self.frames == self.press_on + 1 {
            vec![Event::KeyUp(Key::Space)]
        } else {
            Vec::new()
        })
    }

    fn present(&mut self, _pixels: &[u32], _width: u32, _height: u32) -> io::Result<()> {
        Ok(())
    }

    fn set_title(&mut self, _title: &str) -> io::Result<()> {
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "tapper"
    }
}

/// Where a game reads the jump key.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadsThePressIn {
    /// Every frame, latching into [`PendingInput`] — what the design asks for.
    Update,
    /// Only on a tick, straight off [`runity_core::Input`] — the bug this
    /// whole mechanism exists to prevent.
    FixedUpdate,
}

/// What the run is asked about afterwards.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Findings {
    /// Frame the press was visible on.
    saw_press_on_frame: Option<u64>,
    /// How many ticks had already run when the key went down.
    ticks_at_press: u64,
    /// Which tick actually took the jump.
    jumped_on_tick: Option<u64>,
    ticks: u64,
}

struct Jumper {
    reads: ReadsThePressIn,
    physics: Option<PhysicsWorld>,
    walker: Option<Entity>,
    found: Rc<RefCell<Findings>>,
}

impl Game for Jumper {
    fn start(&mut self, engine: &mut Engine) -> io::Result<()> {
        engine.time.fixed_delta = FIXED_DELTA;
        let ground = Heightfield::new(Vec2::splat(-20.0), 4.0, 11, 11, vec![0.0; 121]);
        self.physics = Some(PhysicsWorld::new(ground, Tuning::default(), FIXED_DELTA));

        let walker = engine.world.spawn();
        engine.world.insert(walker, Body::new(Vec3::ZERO));
        engine.world.insert(walker, PendingInput::new());
        self.walker = Some(walker);
        Ok(())
    }

    fn update(&mut self, engine: &mut Engine) {
        if !engine.input.key_pressed(Key::Space) {
            return;
        }
        let mut found = self.found.borrow_mut();
        found.saw_press_on_frame = Some(engine.time.frame());
        found.ticks_at_press = found.ticks;
        drop(found);

        if self.reads == ReadsThePressIn::Update {
            self.pending(engine).push(Command::Jump);
        }
    }

    fn fixed_update(&mut self, engine: &mut Engine) {
        if self.reads == ReadsThePressIn::FixedUpdate && engine.input.key_pressed(Key::Space) {
            self.pending(engine).push(Command::Jump);
        }

        self.physics
            .as_mut()
            .expect("start ran")
            .step(&mut engine.world);

        let mut found = self.found.borrow_mut();
        found.ticks += 1;
        let walker = self.walker.expect("start ran");
        if engine.world.get::<Body>(walker).expect("a body").jumped {
            found.jumped_on_tick = Some(found.ticks);
        }
    }
}

impl Jumper {
    fn pending<'a>(&self, engine: &'a mut Engine) -> &'a mut PendingInput {
        engine
            .world
            .get_mut::<PendingInput>(self.walker.expect("start ran"))
            .expect("the walker has one")
    }
}

/// Run a jumper for `frames` frames with the key tapped on `press_on`.
fn run(reads: ReadsThePressIn, press_on: u64, frames: u64) -> Findings {
    let found = Rc::new(RefCell::new(Findings::default()));
    let config = WindowConfig::new("race", 8, 8);
    App::new(config)
        .with_max_frames(frames)
        .with_frame_delta(FRAME_DELTA)
        .with_target_fps(None)
        .run_with_window(
            Box::new(Tapper {
                press_on,
                frames: 0,
            }),
            Jumper {
                reads,
                physics: None,
                walker: None,
                found: Rc::clone(&found),
            },
        )
        .expect("a run against a window in memory cannot fail");
    // Bound to a local first: the `Ref` is a temporary that would otherwise
    // still be alive when `found` is dropped at the end of the block.
    let findings = *found.borrow();
    findings
}

#[test]
fn a_jump_pressed_on_a_frame_with_no_tick_is_still_consumed_by_the_next_tick() {
    // Frame 1 at 1/60 has put 1/60 s into a 1/20 s accumulator: no fixed step
    // runs, and by the top of frame 2 the press edge is gone.
    let latched = run(ReadsThePressIn::Update, 1, 30);
    assert_eq!(
        latched.saw_press_on_frame,
        Some(1),
        "the press has to land on the frame the test aimed at"
    );
    assert_eq!(
        latched.ticks_at_press, 0,
        "and that frame must be one that ran no tick — otherwise there is no \
         race here to reproduce"
    );
    assert_eq!(
        latched.jumped_on_tick,
        Some(1),
        "the very next tick to run is the one that takes the jump"
    );
}

#[test]
fn reading_the_key_only_on_a_tick_loses_the_same_press() {
    // The same window, the same frame, the same loop — and no jump, because
    // `begin_frame` cleared the edge two frames before the first tick ran.
    // This is what makes the test above a test and not a tautology.
    let straight = run(ReadsThePressIn::FixedUpdate, 1, 30);
    assert_eq!(straight.saw_press_on_frame, Some(1));
    assert_eq!(straight.ticks_at_press, 0);
    assert_eq!(
        straight.jumped_on_tick, None,
        "this is the bug PendingInput exists to prevent"
    );
}

#[test]
fn a_jump_pressed_on_a_frame_that_does_tick_is_taken_once_and_only_once() {
    // The latch must not turn one press into two jumps when the press happens
    // to land on a ticking frame. Frame 3 is the first tick at 60 Hz under a
    // 20 Hz step, so this is the case the latch could double.
    let latched = run(ReadsThePressIn::Update, 3, 30);
    assert_eq!(latched.saw_press_on_frame, Some(3));
    assert_eq!(
        latched.ticks_at_press, 0,
        "frame 3 ticks after `update` ran"
    );
    assert_eq!(
        latched.jumped_on_tick,
        Some(1),
        "the tick on that very frame takes it"
    );
    // `jumped` is a one-tick flag and this records the latest tick that set
    // it, so a second take-off anywhere later would show up as a later number.
    assert_eq!(latched.ticks, 10, "thirty frames at 60 Hz is ten ticks");
}
