//! Jobs: work over a slice split across the machine's cores, in order —
//! the render's preparing of every draw, a system's pass over thousands
//! of things.
//!
//! [`map`] cuts the slice into as many runs as there are workers (no run
//! shorter than `least`), maps each run on a thread of its own and gives
//! back the results in the slice's order. Below `least` items — or with
//! `RUNITY_JOBS=1` — it runs on the calling thread and costs nothing more
//! than the loop it replaces. Threads are the standard library's scoped
//! ones, made for the call: a frame's few calls cost tens of microseconds,
//! and borrowing what the caller has needs nothing `'static`.

use std::sync::OnceLock;

/// Threads a call may use: the machine's cores, or `RUNITY_JOBS`.
pub fn workers() -> usize {
    static WORKERS: OnceLock<usize> = OnceLock::new();
    *WORKERS.get_or_init(|| {
        std::env::var("RUNITY_JOBS")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
            .max(1)
    })
}

/// `f` over every item, spread over the workers in runs of at least
/// `least`; the results in order.
pub fn map<T: Sync, R: Send>(items: &[T], least: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = workers();
    if workers <= 1 || items.len() < least.max(1) * 2 {
        return items.iter().map(&f).collect();
    }
    let run = items.len().div_ceil(workers).max(least.max(1));
    let f = &f;
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(run)
            .map(|chunk| scope.spawn(move || chunk.iter().map(f).collect::<Vec<R>>()))
            .collect();
        let mut out = Vec::with_capacity(items.len());
        for handle in handles {
            out.extend(handle.join().expect("a job panicked"));
        }
        out
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_map_keeps_the_order_whether_split_or_not() {
        let items: Vec<u64> = (0..10_000).collect();
        let squares = super::map(&items, 64, |x| x * x);
        assert_eq!(squares.len(), items.len());
        assert!(squares.iter().enumerate().all(|(i, s)| *s == (i as u64) * (i as u64)));
        // Too few to split: the same, on this thread.
        let few = super::map(&items[..10], 64, |x| x + 1);
        assert_eq!(few, (1..11).collect::<Vec<u64>>());
        assert!(super::workers() >= 1);
    }
}
