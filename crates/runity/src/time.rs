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

use std::time::{Duration, Instant};

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
    elapsed: f32,
    frame: u64,
    step: u64,
    accumulator: f32,
    steps_this_frame: u32,
    last: Option<Instant>,
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

    /// Advance by the wall clock. Called once per frame by the shell.
    pub fn tick(&mut self) {
        let now = Instant::now();
        let raw = match self.last {
            Some(last) => now.duration_since(last).as_secs_f32(),
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
        self.delta = capped * self.settings.scale;
        self.elapsed += self.delta;
        self.frame += 1;
        self.accumulator += self.delta;
        self.steps_this_frame = 0;
    }

    /// Seconds the last frame took, after scaling.
    pub fn delta(&self) -> f32 {
        self.delta
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
    /// # let mut time = runity::Time::default();
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

#[cfg(test)]
mod tests {
    use super::*;

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
