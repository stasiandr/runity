//! The link's clock: the wall's, or a simulated one a test steps by hand.
//!
//! [`super::Laggy`]'s delays, [`super::Link`]'s resends and timeouts, and
//! everything a session times by — the server's clock that everyone agrees
//! on, each client's allowance, how late a replica's ticks arrive and how
//! long one has been silent, a lost party's retry — read [`now`]. A test that plays two peers in one thread calls [`simulate`]
//! and then [`advance`]s the clock by each frame's length: a 40 ms
//! latency is then 40 ms of the frames' time, however fast or slow the
//! machine runs them — the same run on every machine, no sleeps, no
//! flakes (Not Divinity's `online_the_relic_vote_needs_both` failed one
//! run in four on a loaded machine: frames played faster than the wall's
//! latency held a press for many ticks).

use std::cell::Cell;
use web_time::{Duration, Instant};

thread_local! {
    static SIM: Cell<Option<(Instant, Duration)>> = const { Cell::new(None) };
}

/// Now: the wall's, or this thread's simulated clock.
pub fn now() -> Instant {
    SIM.with(|s| match s.get() {
        Some((base, at)) => base + at,
        None => Instant::now(),
    })
}

/// This thread's link clock from now on simulated, starting at the wall's
/// now and standing still until [`advance`]d.
pub fn simulate() {
    SIM.with(|s| s.set(Some((Instant::now(), Duration::ZERO))));
}

/// The wall's clock again on this thread.
pub fn real() {
    SIM.with(|s| s.set(None));
}

/// The simulated clock moved on by `d` (nothing when it is the wall's).
pub fn advance(d: Duration) {
    SIM.with(|s| {
        if let Some((base, at)) = s.get() {
            s.set(Some((base, at + d)));
        }
    });
}

/// How long since `t`, by [`now`] (nothing when `t` is ahead of it).
pub fn since(t: Instant) -> Duration {
    now().saturating_duration_since(t)
}

/// Whether this thread's link clock is simulated.
pub fn simulated() -> bool {
    SIM.with(|s| s.get().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_simulated_clock_moves_only_when_told() {
        simulate();
        let a = now();
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(now(), a, "standing still");
        advance(Duration::from_millis(40));
        assert_eq!(now() - a, Duration::from_millis(40));
        real();
        assert!(!simulated());
    }
}
