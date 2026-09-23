//! Frame times, as the numbers a budget is written in.
//!
//! DNA, postulate 6: budgets are numbers held by tests, and both the median
//! and the hitches count — a player complains about the stutter, not the
//! average. [`FrameTimes`] keeps the last few hundred frames and says the
//! median, the 99th percentile, the worst, and how many frames took more
//! than twice the median.

use std::collections::VecDeque;
use std::time::Duration;

/// The last `capacity` frame times.
#[derive(Debug, Clone)]
pub struct FrameTimes {
    times: VecDeque<Duration>,
    capacity: usize,
}

/// What the recorded frames came to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSummary {
    pub frames: usize,
    pub median: Duration,
    pub p99: Duration,
    pub worst: Duration,
    /// Frames longer than twice the median: what a player feels.
    pub hitches: usize,
}

impl Default for FrameTimes {
    fn default() -> Self {
        Self::new(600)
    }
}

impl FrameTimes {
    pub fn new(capacity: usize) -> Self {
        Self {
            times: VecDeque::with_capacity(capacity.max(1)),
            capacity: capacity.max(1),
        }
    }

    pub fn record(&mut self, frame: Duration) {
        if self.times.len() == self.capacity {
            self.times.pop_front();
        }
        self.times.push_back(frame);
    }

    pub fn summary(&self) -> Option<FrameSummary> {
        if self.times.is_empty() {
            return None;
        }
        let mut sorted: Vec<Duration> = self.times.iter().copied().collect();
        sorted.sort();
        let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
        let median = at(0.5);
        Some(FrameSummary {
            frames: sorted.len(),
            median,
            p99: at(0.99),
            worst: *sorted.last().expect("not empty"),
            hitches: sorted.iter().filter(|t| **t > median * 2).count(),
        })
    }
}

impl std::fmt::Display for FrameSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} frames: median {:.2} ms, p99 {:.2} ms, worst {:.2} ms, {} hitches",
            self.frames,
            self.median.as_secs_f64() * 1e3,
            self.p99.as_secs_f64() * 1e3,
            self.worst.as_secs_f64() * 1e3,
            self.hitches
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stutter_is_counted_even_when_the_median_is_fine() {
        let mut times = FrameTimes::new(100);
        for i in 0..100 {
            let ms = if i % 25 == 0 { 40 } else { 16 };
            times.record(Duration::from_millis(ms));
        }
        let summary = times.summary().unwrap();
        assert_eq!(summary.median, Duration::from_millis(16));
        assert_eq!(summary.hitches, 4);
        assert_eq!(summary.worst, Duration::from_millis(40));
        // Only the last `capacity` frames count.
        for _ in 0..100 {
            times.record(Duration::from_millis(16));
        }
        assert_eq!(times.summary().unwrap().hitches, 0);
    }
}
