//! Two clocks, because a frame and a simulation step are not the same thing.
//!
//! The frame clock is however long the last frame took — what animation and
//! camera movement use, so they stay smooth on any machine. The simulation
//! clock runs in whole steps of a fixed size, so that the same inputs give
//! the same result on two machines, which is what save files, networking and
//! tests all quietly depend on.
//!
//! Mixing them is the classic bug: physics stepped by a variable delta gives
//! a different answer at 144 Hz than at 60, and a replay stops replaying.
//!
//! Game code turns the clock from the world, where its systems are: slow
//! motion ([`set_scale`]) and a hit-stop ([`hit_stop`], the whole world
//! still for a few hundredths of a second, as a heavy blow lands). What a
//! system asked is carried to the clock by whoever owns the loop
//! ([`sync`]), which also leaves the frame's unscaled delta in the world
//! ([`unscaled_delta`]) for what must not slow with it — a camera's blend,
//! its shake (docs/feel.md).

use std::time::Duration;

/// The longest real interval one frame may report, in seconds.
///
/// A machine that stalled, a laptop that slept, or a debugger that stopped
/// the process all produce a delta that is true and useless. Clamping is
/// applied to the raw interval, before [`TimeSettings::scale`], because what
/// is being bounded is real elapsed time.
pub const MAX_FRAME_DELTA: f32 = 0.25;

/// How the simulation clock behaves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeSettings {
    /// Seconds per simulation step. 1/60 by default.
    pub fixed_delta: f32,
    /// Most steps one frame may run to catch up.
    ///
    /// Without a cap, a frame that took a second asks for sixty steps, which
    /// takes longer than a second, which asks for more — the spiral of death.
    /// The cap makes a slow machine run the world slowly, which is bad, while
    /// the alternative is a machine that never draws again.
    pub max_steps_per_frame: u32,
    /// Multiplies the frame delta: 0.0 pauses, 0.5 is slow motion.
    pub scale: f32,
}

impl Default for TimeSettings {
    fn default() -> Self {
        Self {
            fixed_delta: 1.0 / 60.0,
            max_steps_per_frame: 8,
            scale: 1.0,
        }
    }
}

/// The clocks.
#[derive(Debug, Clone)]
pub struct Time {
    settings: TimeSettings,
    delta: f32,
    unscaled_delta: f32,
    /// Real seconds of hit-stop still to come: the world is still for
    /// them, whatever the scale.
    frozen: f32,
    elapsed: f32,
    frame: u64,
    step: u64,
    accumulator: f32,
    steps_this_frame: u32,
    /// The host's clock at the last tick, seconds.
    last: Option<f64>,
}

impl Default for Time {
    fn default() -> Self {
        Self::new(TimeSettings::default())
    }
}

impl Time {
    pub fn new(settings: TimeSettings) -> Self {
        Self {
            settings,
            delta: 0.0,
            unscaled_delta: 0.0,
            frozen: 0.0,
            elapsed: 0.0,
            frame: 0,
            step: 0,
            accumulator: 0.0,
            steps_this_frame: 0,
            last: None,
        }
    }

    pub fn settings(&self) -> TimeSettings {
        self.settings
    }

    pub fn settings_mut(&mut self) -> &mut TimeSettings {
        &mut self.settings
    }

    /// Advance by the wall clock. Called once per frame by the shell. Not on
    /// the web, where there is no `Instant`: there the host says the time
    /// ([`Time::tick_at`]).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn tick(&mut self) {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let start = *START.get_or_init(std::time::Instant::now);
        self.tick_at(start.elapsed().as_secs_f64());
    }

    /// Advance to `now`, seconds on the host's own clock (DNA, "Не
    /// закрывать веб": the loop's clock without `std::time::Instant`) — a
    /// browser's `performance.now() / 1000`, a recorded run's timestamps.
    /// Only the difference between ticks counts.
    pub fn tick_at(&mut self, now: f64) {
        let raw = match self.last {
            Some(last) => (now - last).max(0.0) as f32,
            // The first frame has no previous instant to measure against.
            // Reporting one fixed step is closer to the truth than zero, and
            // keeps anything that divides by delta out of trouble.
            None => self.settings.fixed_delta,
        };
        self.last = Some(now);
        self.advance(raw);
    }

    /// Advance by a delta given directly — for tests, for recorded runs, and
    /// for headless work where wall-clock time is the wrong clock entirely.
    pub fn advance(&mut self, raw_delta: f32) {
        let capped = raw_delta.clamp(0.0, MAX_FRAME_DELTA);
        self.unscaled_delta = capped;
        // A hit-stop eats real time first: the part of the frame it covers
        // is no time at all for the world, the rest runs as scaled.
        let still = self.frozen.min(capped);
        self.frozen -= still;
        self.delta = (capped - still) * self.settings.scale;
        self.elapsed += self.delta;
        self.frame += 1;
        self.accumulator += self.delta;
        self.steps_this_frame = 0;
    }

    /// Seconds the last frame took, after scaling.
    pub fn delta(&self) -> f32 {
        self.delta
    }

    /// Seconds the last frame took in the real world: before the scale and
    /// a hit-stop, after the clamp. For what must not slow with the world —
    /// a camera's blend and shake, a pause menu's animation.
    pub fn unscaled_delta(&self) -> f32 {
        self.unscaled_delta
    }

    /// Set how fast the world runs from now on: 1 is real time, 0.2 slow
    /// motion, 0 paused. Negative is 0.
    pub fn set_scale(&mut self, scale: f32) {
        self.settings.scale = scale.max(0.0);
    }

    /// Hold the world still for `seconds` of real time, from the next
    /// frame: no steps, no frame time, whatever the scale; then it goes on
    /// at the scale it had. A second hit-stop while one lasts makes it last
    /// as long as the longer of the two, not their sum.
    pub fn hit_stop(&mut self, seconds: f32) {
        self.frozen = self.frozen.max(seconds.max(0.0));
    }

    /// Real seconds of hit-stop left.
    pub fn frozen(&self) -> f32 {
        self.frozen
    }

    /// Do what the game asked of the clock ([`TimeAsk`]).
    pub fn ask(&mut self, asked: TimeAsk) {
        if let Some(scale) = asked.scale {
            self.set_scale(scale);
        }
        if asked.hit_stop > 0.0 {
            self.hit_stop(asked.hit_stop);
        }
    }

    /// Seconds since the clock started, after scaling.
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// How many simulation steps have run since the clock started.
    pub fn step(&self) -> u64 {
        self.step
    }

    /// Take one pending simulation step, if one is due.
    ///
    /// The shape is a `while let` rather than a count, so that the caller
    /// cannot take the number and then forget to run them:
    ///
    /// ```
    /// # let mut time = scrap_core::Time::default();
    /// # time.advance(0.05);
    /// while time.next_step().is_some() {
    ///     // step the world by `time.settings().fixed_delta`
    /// }
    /// ```
    pub fn next_step(&mut self) -> Option<f32> {
        let fixed = self.settings.fixed_delta;
        if fixed <= 0.0 || self.accumulator < fixed {
            // Whatever is left over stays in the accumulator for next frame,
            // which is what keeps the simulation from drifting against the
            // wall clock.
            return None;
        }
        if self.steps_this_frame >= self.settings.max_steps_per_frame {
            // Behind by more than the cap: drop the debt rather than trying
            // to pay it, or the next frame inherits it and grows.
            self.accumulator = 0.0;
            return None;
        }
        self.accumulator -= fixed;
        self.step += 1;
        self.steps_this_frame += 1;
        Some(fixed)
    }

    /// How far the current frame sits between the last simulation step and
    /// the next, in `0..1`.
    ///
    /// Rendering at the last step's positions makes movement stutter at any
    /// frame rate that is not an exact multiple of the step rate. Blending by
    /// this is what removes it.
    pub fn interpolation(&self) -> f32 {
        if self.settings.fixed_delta <= 0.0 {
            return 0.0;
        }
        (self.accumulator / self.settings.fixed_delta).clamp(0.0, 1.0)
    }

    pub fn elapsed_duration(&self) -> Duration {
        Duration::from_secs_f32(self.elapsed.max(0.0))
    }
}

/// What game code asked of the clock since the loop last looked: a new
/// scale, a hit-stop. Asked in the world ([`set_scale`], [`hit_stop`]) or
/// on the shell's context, done by [`Time::ask`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TimeAsk {
    /// The scale from now on, if asked; the last asked wins.
    pub scale: Option<f32>,
    /// Real seconds to hold the world still; the longest asked wins.
    pub hit_stop: f32,
}

impl TimeAsk {
    /// Both asks as one: `later`'s scale if it has one, the longer stop.
    pub fn and(self, later: TimeAsk) -> TimeAsk {
        TimeAsk {
            scale: later.scale.or(self.scale),
            hit_stop: self.hit_stop.max(later.hit_stop),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.scale.is_none() && self.hit_stop <= 0.0
    }
}

/// The clock as the world's systems see it: one entity carries it, the
/// loop writes it ([`sync`]). What the systems asked waits in it until
/// then. Not saved and not sent: a peer's clock is its own (docs/feel.md,
/// «Сеть»).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WorldClock {
    /// The last frame's real seconds ([`Time::unscaled_delta`]).
    pub unscaled_delta: f32,
    /// How fast the world runs ([`TimeSettings::scale`]).
    pub scale: f32,
    /// Real seconds of hit-stop left.
    pub frozen: f32,
    /// Asked by systems since the loop last looked.
    pub asked: TimeAsk,
}

fn clock_mut(world: &mut hecs::World) -> hecs::RefMut<'_, WorldClock> {
    let found = world
        .query::<(hecs::Entity, &WorldClock)>()
        .iter()
        .next()
        .map(|(e, _)| e);
    let entity = found.unwrap_or_else(|| {
        world.spawn((WorldClock {
            scale: 1.0,
            ..Default::default()
        },))
    });
    world
        .get::<&mut WorldClock>(entity)
        .expect("just found or made")
}

/// The world's clock, if the loop has written one.
pub fn clock(world: &hecs::World) -> Option<WorldClock> {
    world.query::<&WorldClock>().iter().next().copied()
}

/// Ask the loop for slow motion from game code: 1 is real time, 0.2 slow
/// motion, 0 paused — from the next frame, until asked again.
pub fn set_scale(world: &mut hecs::World, scale: f32) {
    clock_mut(world).asked.scale = Some(scale);
}

/// Ask the loop for a hit-stop from game code: the whole world still for
/// `seconds` of real time, from the next frame (Feel's Freeze Frame;
/// 0.05–0.15 s for a blow). A camera's shake and blend keep moving.
pub fn hit_stop(world: &mut hecs::World, seconds: f32) {
    let mut clock = clock_mut(world);
    clock.asked.hit_stop = clock.asked.hit_stop.max(seconds);
}

/// The last frame's real seconds, as the loop left them in the world;
/// `None` when nothing writes the world's clock.
pub fn unscaled_delta(world: &hecs::World) -> Option<f32> {
    clock(world).map(|c| c.unscaled_delta)
}

/// The loop's half: leave `time` in the world for its systems, and take
/// what they asked of it — for the loop to do with [`Time::ask`], or with
/// the shell's `Context::ask_time`. Once a frame, before the frame's
/// systems; and after the fixed steps, whose asks it also takes.
pub fn sync(world: &mut hecs::World, time: &Time) -> TimeAsk {
    let mut clock = clock_mut(world);
    clock.unscaled_delta = time.unscaled_delta();
    clock.scale = time.settings().scale;
    clock.frozen = time.frozen();
    std::mem::take(&mut clock.asked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_motion_slows_the_world_and_not_the_real_clock() {
        let mut time = Time::new(TimeSettings {
            fixed_delta: 0.05,
            ..Default::default()
        });
        time.set_scale(0.25);
        time.advance(0.2);
        assert!((time.delta() - 0.05).abs() < 1e-6);
        assert_eq!(
            time.unscaled_delta(),
            0.2,
            "the real frame is still a fifth of a second"
        );
        assert_eq!(steps(&mut time), 1);
        time.set_scale(-3.0);
        assert_eq!(time.settings().scale, 0.0, "backwards is stopped");
    }

    #[test]
    fn a_hit_stop_holds_the_world_for_real_seconds_then_lets_it_go_at_its_scale() {
        // Powers of two, so the sums are exact.
        let mut time = Time::new(TimeSettings {
            fixed_delta: 1.0 / 64.0,
            max_steps_per_frame: 100,
            scale: 0.5,
        });
        time.hit_stop(0.125);
        time.hit_stop(0.0625);
        assert_eq!(time.frozen(), 0.125, "the longer stop, not the sum");
        let mut world_seconds = 0.0;
        let mut stepped = Vec::new();
        for _ in 0..4 {
            time.advance(0.0625);
            world_seconds += time.delta();
            stepped.push(steps(&mut time));
            assert_eq!(time.unscaled_delta(), 0.0625, "real time goes on");
        }
        // A quarter of a real second: an eighth still, an eighth at half speed.
        assert_eq!(world_seconds, 0.0625);
        assert_eq!(stepped, [0, 0, 2, 2], "no steps while still");
        assert_eq!(time.frozen(), 0.0);
        assert_eq!(time.settings().scale, 0.5, "the scale it had");
    }

    #[test]
    fn game_code_asks_the_clock_through_the_world() {
        let mut world = hecs::World::new();
        let mut time = Time::default();
        assert_eq!(unscaled_delta(&world), None, "nothing writes the clock yet");
        set_scale(&mut world, 0.3);
        hit_stop(&mut world, 0.08);
        hit_stop(&mut world, 0.02);
        time.advance(0.1);
        let asked = sync(&mut world, &time);
        assert_eq!(
            asked,
            TimeAsk {
                scale: Some(0.3),
                hit_stop: 0.08
            }
        );
        assert_eq!(unscaled_delta(&world), Some(0.1));
        time.ask(asked);
        assert_eq!(sync(&mut world, &time), TimeAsk::default(), "taken once");
        assert_eq!(time.settings().scale, 0.3);
        assert_eq!(time.frozen(), 0.08);
        time.advance(0.05);
        assert_eq!(time.delta(), 0.0, "still");
        assert_eq!(world.len(), 1, "one entity keeps the clock");
        let later = TimeAsk {
            scale: Some(1.0),
            hit_stop: 0.0,
        };
        assert_eq!(
            asked.and(later),
            TimeAsk {
                scale: Some(1.0),
                hit_stop: 0.08
            }
        );
    }

    #[test]
    fn the_host_says_the_time_and_only_the_difference_counts() {
        let mut time = Time::default();
        time.tick_at(1000.0);
        assert!((time.delta() - 1.0 / 60.0).abs() < 1e-6, "the first frame is one step");
        time.tick_at(1000.1);
        assert!((time.delta() - 0.1).abs() < 1e-4);
        time.tick_at(999.0);
        assert_eq!(time.delta(), 0.0, "a clock going back is no time");
    }

    fn steps(time: &mut Time) -> u32 {
        let mut n = 0;
        while time.next_step().is_some() {
            n += 1;
        }
        n
    }

    #[test]
    fn a_frame_runs_as_many_whole_steps_as_it_paid_for() {
        let mut time = Time::new(TimeSettings {
            fixed_delta: 0.1,
            ..Default::default()
        });
        time.advance(0.25);
        assert_eq!(steps(&mut time), 2, "0.25 buys two whole steps of 0.1");
        assert!(
            (time.interpolation() - 0.5).abs() < 1e-5,
            "and leaves half a step over"
        );
    }

    #[test]
    fn the_leftover_carries_instead_of_being_thrown_away() {
        // Three frames of 0.075 at a step of 0.1 must come to two steps, not
        // zero. Dropping the remainder each frame is how a simulation drifts
        // slower than the clock without anything looking wrong.
        let mut time = Time::new(TimeSettings {
            fixed_delta: 0.1,
            ..Default::default()
        });
        let mut total = 0;
        for _ in 0..3 {
            time.advance(0.075);
            total += steps(&mut time);
        }
        assert_eq!(total, 2);
    }

    #[test]
    fn a_stalled_frame_cannot_spiral() {
        // A frame that took a whole second asks for sixty steps; running
        // them takes longer than a second, which asks for more.
        let mut time = Time::new(TimeSettings {
            fixed_delta: 1.0 / 60.0,
            max_steps_per_frame: 4,
            scale: 1.0,
        });
        time.advance(1.0);
        assert_eq!(steps(&mut time), 4, "capped");

        // And the debt does not follow us into the next frame.
        time.advance(1.0 / 60.0);
        assert_eq!(steps(&mut time), 1);
    }

    #[test]
    fn scale_slows_the_world_without_touching_the_step_size() {
        let mut time = Time::new(TimeSettings {
            fixed_delta: 0.05,
            scale: 0.5,
            ..Default::default()
        });
        // Under MAX_FRAME_DELTA on purpose: the clamp applies to the raw
        // interval, so a test that crosses it is measuring the clamp rather
        // than the scale.
        time.advance(0.2);
        assert_eq!(steps(&mut time), 2, "half of 0.2 is two steps of 0.05");
        assert_eq!(
            time.settings().fixed_delta,
            0.05,
            "the step itself must not change, or physics changes with it"
        );
        assert!((time.delta() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn pausing_stops_the_world_but_not_the_frames() {
        let mut time = Time::new(TimeSettings {
            fixed_delta: 0.1,
            scale: 0.0,
            ..Default::default()
        });
        time.advance(0.5);
        assert_eq!(steps(&mut time), 0);
        assert_eq!(time.frame(), 1, "frames still happen; the world does not");
        assert_eq!(time.elapsed(), 0.0);
    }

    #[test]
    fn the_raw_interval_is_clamped_before_it_is_scaled() {
        // Which way round this happens is worth pinning: clamping after
        // scaling would let slow motion smuggle a stall through.
        let mut time = Time::new(TimeSettings {
            fixed_delta: 0.05,
            scale: 1.0,
            max_steps_per_frame: 1000,
        });
        time.advance(10.0);
        assert_eq!(
            steps(&mut time),
            (MAX_FRAME_DELTA / 0.05) as u32,
            "ten seconds is worth a quarter of a second"
        );
    }

    #[test]
    fn a_debugger_pause_does_not_hand_the_world_an_hour() {
        let mut time = Time::new(TimeSettings {
            fixed_delta: 1.0 / 60.0,
            max_steps_per_frame: 1000,
            scale: 1.0,
        });
        time.advance(3600.0);
        assert!(
            steps(&mut time) <= 15,
            "a frame is clamped to a quarter second before the cap is reached"
        );
    }
}
