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

/// Time spent in named parts of a frame or a step — "physics",
/// "systems", "frame" — each with its own median and hitches: Unity's
/// Profiler, as numbers. The question every feature answers (DNA,
/// postulate 6) is what it costs in milliseconds; this is where that is
/// read off a running game.
#[derive(Debug, Clone, Default)]
pub struct Profiler {
    spans: Vec<(String, FrameTimes)>,
    capacity: usize,
}

impl Profiler {
    /// Keeping the last `capacity` samples of each span.
    pub fn new(capacity: usize) -> Self {
        Self {
            spans: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    /// Run `work`, and count how long it took under `name`. On the web,
    /// where there is no `Instant`, it runs uncounted: a host there records
    /// what it measured itself ([`Profiler::record`]).
    pub fn time<R>(&mut self, name: &str, work: impl FnOnce() -> R) -> R {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let start = std::time::Instant::now();
            let out = work();
            self.record(name, start.elapsed());
            out
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = name;
            work()
        }
    }

    /// Count a duration measured elsewhere under `name`.
    pub fn record(&mut self, name: &str, took: Duration) {
        let capacity = if self.capacity == 0 {
            600
        } else {
            self.capacity
        };
        match self.spans.iter_mut().find(|(n, _)| n == name) {
            Some((_, times)) => times.record(took),
            None => {
                let mut times = FrameTimes::new(capacity);
                times.record(took);
                self.spans.push((name.to_string(), times));
            }
        }
    }

    /// Every span's summary, costliest median first.
    pub fn report(&self) -> Vec<(String, FrameSummary)> {
        let mut out: Vec<(String, FrameSummary)> = self
            .spans
            .iter()
            .filter_map(|(name, times)| Some((name.clone(), times.summary()?)))
            .collect();
        out.sort_by(|a, b| b.1.median.cmp(&a.1.median).then(a.0.cmp(&b.0)));
        out
    }

    /// The report as lines to draw over the game or print.
    pub fn lines(&self) -> Vec<String> {
        self.report()
            .into_iter()
            .map(|(name, s)| {
                format!(
                    "{name:<12} {:>6.2} ms  p99 {:>6.2}  worst {:>6.2}  hitches {}",
                    s.median.as_secs_f64() * 1e3,
                    s.p99.as_secs_f64() * 1e3,
                    s.worst.as_secs_f64() * 1e3,
                    s.hitches
                )
            })
            .collect()
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

    #[test]
    fn a_profiler_says_which_part_costs_what_costliest_first() {
        let mut profile = Profiler::new(10);
        for i in 0..10 {
            profile.record(
                "physics",
                Duration::from_micros(if i == 3 { 9000 } else { 2000 }),
            );
            profile.record("frame", Duration::from_micros(500));
        }
        let answer = profile.time("systems", || 7);
        assert_eq!(answer, 7, "the work's value comes back");
        let report = profile.report();
        assert_eq!(report[0].0, "physics");
        assert_eq!(report[0].1.median, Duration::from_millis(2));
        assert_eq!(report[0].1.hitches, 1);
        assert_eq!(report.len(), 3);
        assert!(
            profile.lines()[0].starts_with("physics"),
            "{:?}",
            profile.lines()
        );
    }
}
