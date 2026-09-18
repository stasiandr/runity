use std::time::Instant;

/// Frame timing.
#[derive(Debug, Clone)]
pub struct Time {
    start: Instant,
    last_frame: Instant,
    delta: f32,
    elapsed: f32,
    frame: u64,
    /// Upper bound on a single frame's delta, so a stall (a debugger breakpoint,
    /// a dragged window) cannot make simulation explode.
    pub max_delta: f32,
    /// Timestep used for [`crate::Game::fixed_update`].
    pub fixed_delta: f32,
    accumulator: f32,
    /// How long [`Time::average_fps`] averages over before it refreshes.
    pub fps_window: f32,
    window_elapsed: f32,
    window_frames: u32,
    average_fps: f32,
}

impl Default for Time {
    fn default() -> Self {
        Self::new()
    }
}

impl Time {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last_frame: now,
            delta: 0.0,
            elapsed: 0.0,
            frame: 0,
            max_delta: 0.25,
            fixed_delta: 1.0 / 60.0,
            accumulator: 0.0,
            fps_window: 0.5,
            window_elapsed: 0.0,
            window_frames: 0,
            average_fps: 0.0,
        }
    }

    /// Seconds since the previous frame.
    #[inline]
    pub fn delta(&self) -> f32 {
        self.delta
    }

    /// Seconds since the engine started.
    #[inline]
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }

    #[inline]
    pub fn frame(&self) -> u64 {
        self.frame
    }

    #[inline]
    pub fn fps(&self) -> f32 {
        if self.delta > 0.0 {
            1.0 / self.delta
        } else {
            0.0
        }
    }

    /// Frames per second averaged over the last [`Time::fps_window`] seconds.
    ///
    /// [`Time::fps`] is one frame's reciprocal delta, which jitters far too
    /// much to put in a window title. This value only changes when a window's
    /// worth of frames has gone by — twice a second at the default 0.5 s — so
    /// the number a player reads stays still long enough to read.
    ///
    /// Before the first window fills there is nothing to average, so the
    /// instantaneous rate stands in.
    #[inline]
    pub fn average_fps(&self) -> f32 {
        if self.average_fps > 0.0 {
            self.average_fps
        } else {
            self.fps()
        }
    }

    /// Advance to the next frame using the wall clock.
    pub fn tick(&mut self) {
        let now = Instant::now();
        let delta = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.advance(delta);
        self.elapsed = now.duration_since(self.start).as_secs_f32();
    }

    /// Advance by an explicit delta — used by tests and by headless runs that
    /// should not depend on how fast the machine is.
    pub fn advance(&mut self, delta: f32) {
        self.delta = delta.clamp(0.0, self.max_delta);
        self.elapsed += self.delta;
        self.frame += 1;
        self.accumulator += self.delta;

        self.window_elapsed += self.delta;
        self.window_frames += 1;
        if self.fps_window > 0.0 && self.window_elapsed >= self.fps_window {
            self.average_fps = self.window_frames as f32 / self.window_elapsed;
            self.window_elapsed = 0.0;
            self.window_frames = 0;
        }
    }

    /// Take one fixed step if enough time has accumulated.
    ///
    /// Call in a `while` loop; it drains the accumulator one step at a time.
    pub fn next_fixed_step(&mut self) -> bool {
        if self.fixed_delta > 0.0 && self.accumulator >= self.fixed_delta {
            self.accumulator -= self.fixed_delta;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_is_clamped_to_max_delta() {
        let mut t = Time::new();
        t.max_delta = 0.1;
        t.advance(5.0);
        assert_eq!(t.delta(), 0.1);
    }

    #[test]
    fn fixed_steps_drain_the_accumulator() {
        let mut t = Time::new();
        t.fixed_delta = 0.01;
        t.advance(0.035);
        let mut steps = 0;
        while t.next_fixed_step() {
            steps += 1;
        }
        assert_eq!(
            steps, 3,
            "0.035s at a 10ms step is three ticks with 5ms left over"
        );

        t.advance(0.005);
        assert!(
            t.next_fixed_step(),
            "the leftover plus the new frame is one more step"
        );
        assert!(!t.next_fixed_step());
    }

    #[test]
    fn average_fps_refreshes_once_per_window_and_smooths_a_stutter() {
        let mut t = Time::new();
        assert_eq!(t.fps_window, 0.5);

        // Half a second of 60 Hz frames is exactly one window.
        for _ in 0..30 {
            t.advance(1.0 / 60.0);
        }
        assert!((t.average_fps() - 60.0).abs() < 0.01, "{}", t.average_fps());

        // One long frame drags the instantaneous rate down hard, but the
        // average holds the previous window's value until the next one fills.
        t.advance(0.2);
        assert!((t.fps() - 5.0).abs() < 0.01);
        assert!((t.average_fps() - 60.0).abs() < 0.01, "not refreshed yet");

        for _ in 0..18 {
            t.advance(1.0 / 60.0);
        }
        // 19 frames over 0.2 + 18/60 s.
        let expected = 19.0 / (0.2 + 18.0 / 60.0);
        assert!(
            (t.average_fps() - expected).abs() < 0.01,
            "{} vs {expected}",
            t.average_fps()
        );
    }

    #[test]
    fn average_fps_falls_back_to_the_instantaneous_rate_at_startup() {
        let mut t = Time::new();
        t.advance(1.0 / 50.0);
        assert!(
            (t.average_fps() - 50.0).abs() < 0.01,
            "the first window has not filled, so there is nothing to average"
        );
    }

    #[test]
    fn frames_and_elapsed_time_accumulate() {
        let mut t = Time::new();
        t.advance(0.016);
        t.advance(0.016);
        assert_eq!(t.frame(), 2);
        assert!((t.elapsed() - 0.032).abs() < 1e-6);
        assert!((t.fps() - 62.5).abs() < 0.1);
    }
}
