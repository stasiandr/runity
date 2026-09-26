//! Jobs: work over a slice split across the machine's cores, in order —
//! the render's preparing of every draw, a simulation's pass over its
//! particles, a system's pass over thousands of things.
//!
//! One pool of threads, made the first time it is wanted and kept (rayon's
//! global one, sized to the cores or `SCRAP_JOBS`): a call costs a few
//! microseconds, so a simulation can split every one of its substeps. Work
//! is cut into runs of at least `least` items. Below `least` items — or
//! with `SCRAP_JOBS=1`, or on the web, where the core runs on one thread
//! (DNA, postulate 7) — it runs on the calling thread and costs nothing
//! more than the loop it replaces.
//!
//! What runs here must not depend on the order the runs finish in: each
//! item's work reads what it is given and writes only its own result, so
//! the outcome is the same on one core or twelve — a step stays
//! reproducible (DNA, postulate 6).

// On the web every call is its plain loop: how to split is not asked.
#![cfg_attr(target_arch = "wasm32", allow(unused_variables, dead_code))]

use std::sync::OnceLock;

/// Threads a call may use: the machine's cores, or `SCRAP_JOBS`.
pub fn workers() -> usize {
    static WORKERS: OnceLock<usize> = OnceLock::new();
    *WORKERS.get_or_init(|| {
        let n = std::env::var("SCRAP_JOBS")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
            .max(1);
        #[cfg(not(target_arch = "wasm32"))]
        if n > 1 {
            // Already built (another library asked first): its size stands.
            let _ = rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .thread_name(|i| format!("scrap-job-{i}"))
                .build_global();
        }
        n
    })
}

fn split(len: usize, least: usize) -> bool {
    workers() > 1 && len >= least.max(1) * 2
}

/// `f` over every item, spread over the workers in runs of at least
/// `least`; the results in order.
pub fn map<T: Sync, R: Send>(items: &[T], least: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    #[cfg(not(target_arch = "wasm32"))]
    if split(items.len(), least) {
        use rayon::prelude::*;
        let f = &f;
        return items.par_iter().with_min_len(least.max(1)).map(f).collect();
    }
    items.iter().map(f).collect()
}

/// `f` over every index below `len`, spread over the workers in runs of
/// at least `least`; the results in order.
pub fn map_range<R: Send>(len: usize, least: usize, f: impl Fn(usize) -> R + Sync) -> Vec<R> {
    #[cfg(not(target_arch = "wasm32"))]
    if split(len, least) {
        use rayon::prelude::*;
        let f = &f;
        return (0..len).into_par_iter().with_min_len(least.max(1)).map(f).collect();
    }
    (0..len).map(f).collect()
}

/// `f` on every item in place, spread over the workers in runs of at least
/// `least`.
pub fn for_each_mut<T: Send>(items: &mut [T], least: usize, f: impl Fn(&mut T) + Sync) {
    #[cfg(not(target_arch = "wasm32"))]
    if split(items.len(), least) {
        use rayon::prelude::*;
        let f = &f;
        items.par_iter_mut().with_min_len(least.max(1)).for_each(f);
        return;
    }
    items.iter_mut().for_each(f);
}

/// `f` on every item in place with its index, spread over the workers.
pub fn for_each_indexed_mut<T: Send>(items: &mut [T], least: usize, f: impl Fn(usize, &mut T) + Sync) {
    #[cfg(not(target_arch = "wasm32"))]
    if split(items.len(), least) {
        use rayon::prelude::*;
        let f = &f;
        items
            .par_iter_mut()
            .with_min_len(least.max(1))
            .enumerate()
            .for_each(|(i, item)| f(i, item));
        return;
    }
    items.iter_mut().enumerate().for_each(|(i, item)| f(i, item));
}

/// `f` on each run of `size` items (the last may be shorter), in place,
/// with the index of the run's first: a grid's rows or slices, split
/// across the workers.
pub fn for_each_chunk_mut<T: Send>(items: &mut [T], size: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let size = size.max(1);
    #[cfg(not(target_arch = "wasm32"))]
    if workers() > 1 && items.len() > size {
        use rayon::prelude::*;
        let f = &f;
        items
            .par_chunks_mut(size)
            .enumerate()
            .for_each(|(i, chunk)| f(i * size, chunk));
        return;
    }
    for (i, chunk) in items.chunks_mut(size).enumerate() {
        f(i * size, chunk);
    }
}

/// `a` and `b` side by side on the workers, both results: two unlike
/// jobs that do not wait on each other — pipelines compiled from one
/// shader for two parts of the renderer. One after the other on the web.
pub fn join<A: Send, B: Send>(a: impl FnOnce() -> A + Send, b: impl FnOnce() -> B + Send) -> (A, B) {
    #[cfg(not(target_arch = "wasm32"))]
    if workers() > 1 {
        return rayon::join(a, b);
    }
    (a(), b())
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
        assert_eq!(super::map_range(10_000, 64, |i| i * 2)[4_999], 9_998);
        assert_eq!(few, (1..11).collect::<Vec<u64>>());
        assert!(super::workers() >= 1);
    }

    #[test]
    fn in_place_passes_touch_every_item_once() {
        let mut items: Vec<u64> = (0..10_000).collect();
        super::for_each_mut(&mut items, 64, |x| *x *= 2);
        assert!(items.iter().enumerate().all(|(i, x)| *x == 2 * i as u64));
        super::for_each_indexed_mut(&mut items, 64, |i, x| *x -= i as u64);
        assert!(items.iter().enumerate().all(|(i, x)| *x == i as u64));
        super::for_each_chunk_mut(&mut items, 100, |first, run| {
            for (k, x) in run.iter_mut().enumerate() {
                assert_eq!(*x, (first + k) as u64);
                *x = first as u64;
            }
        });
        assert_eq!(items[250], 200);
    }
}
