//! A simulation at rest stops stepping: sand that has settled, a pool
//! gone still. Stepping it costs as much as when it moved and changes
//! nothing on the screen; a real-time frame cannot spare that.
//!
//! It falls asleep once its fastest particle has stayed under
//! [`REST_SPEED`] for [`SLEEP_AFTER`] steps, and wakes the moment anything
//! could move it: what is solid round it changes (a crate set down, a sled
//! through the snow), or its particles were moved from outside (a cloth
//! falling on the water). Whether it sleeps follows from its state alone,
//! so a step stays reproducible.

use glam::Vec3;

/// Metres a second under which a particle counts as still.
pub const REST_SPEED: f32 = 0.02;
/// Steps in a row with every particle still before it sleeps.
pub const SLEEP_AFTER: u32 = 30;

/// A simulation's rest: how long it has been still, and what its
/// particles were when it fell asleep.
#[derive(Debug, Clone, Default)]
pub struct Rest {
    still: u32,
    /// The sum of its particles' coordinates when it fell asleep.
    at: Option<f64>,
}

impl Rest {
    /// Whether this step can be skipped. `changed`: what is solid round it
    /// is not what it was last step.
    pub fn asleep(&mut self, positions: &[Vec3], changed: bool) -> bool {
        if changed {
            self.wake();
            return false;
        }
        if self.still < SLEEP_AFTER {
            return false;
        }
        let sum = positions.iter().map(|p| p.x as f64 + p.y as f64 + p.z as f64).sum::<f64>();
        match self.at {
            None => {
                self.at = Some(sum);
                true
            }
            Some(was) if was == sum => true,
            // Moved from outside while it slept.
            Some(_) => {
                self.wake();
                false
            }
        }
    }

    /// A step was taken; `fastest` is its fastest particle's speed.
    pub fn stepped(&mut self, fastest: f32) {
        self.at = None;
        if fastest < REST_SPEED {
            self.still = self.still.saturating_add(1);
        } else {
            self.still = 0;
        }
    }

    pub fn wake(&mut self) {
        self.still = 0;
        self.at = None;
    }

    /// Asleep now.
    pub fn sleeping(&self) -> bool {
        self.at.is_some()
    }
}

/// The fastest of `speeds`.
pub fn fastest(speeds: impl Iterator<Item = Vec3>) -> f32 {
    speeds.map(|v| v.length_squared()).fold(0.0f32, f32::max).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn still_long_enough_it_sleeps_and_a_nudge_or_a_change_wakes_it() {
        let mut rest = Rest::default();
        let at = vec![Vec3::ZERO, Vec3::ONE];
        for _ in 0..SLEEP_AFTER {
            assert!(!rest.asleep(&at, false));
            rest.stepped(0.001);
        }
        assert!(rest.asleep(&at, false) && rest.sleeping());
        assert!(rest.asleep(&at, false));
        // Pushed from outside: awake.
        assert!(!rest.asleep(&[Vec3::ZERO, Vec3::new(1.0, 1.1, 1.0)], false));
        for _ in 0..SLEEP_AFTER {
            rest.stepped(0.001);
        }
        assert!(rest.asleep(&at, false));
        // Something solid came near: awake.
        assert!(!rest.asleep(&at, true));
        // Moving: never asleep.
        for _ in 0..100 {
            rest.stepped(1.0);
        }
        assert!(!rest.asleep(&at, false));
    }
}
