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
    fn frames_and_elapsed_time_accumulate() {
        let mut t = Time::new();
        t.advance(0.016);
        t.advance(0.016);
        assert_eq!(t.frame(), 2);
        assert!((t.elapsed() - 0.032).abs() < 1e-6);
        assert!((t.fps() - 62.5).abs() < 0.1);
    }
}
