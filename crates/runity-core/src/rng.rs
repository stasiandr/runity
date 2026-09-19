//! A small deterministic random number generator for simulation.
//!
//! [`Rng`] is seeded from `(world_seed, tick)` rather than drawn from any
//! process-global generator, so the same tick of the same world seed always
//! produces the same draws, on any machine, on any run. That is what a
//! planner needs to break ties reproducibly without reaching for a system
//! RNG.

/// A seeded, deterministic xorshift64* generator.
///
/// `std`-only: no external RNG crate, per the project's no-dependency rule.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Seed deterministically from a world seed and a simulation tick.
    ///
    /// Two seeds or two ticks that differ by a single bit produce unrelated
    /// generator states: `world_seed` and `tick` are folded together through
    /// splitmix64 before they ever reach xorshift, which is poor at spreading
    /// low-entropy seeds on its own.
    pub fn new(world_seed: u64, tick: u64) -> Self {
        let mixed = splitmix64(world_seed ^ splitmix64(tick));
        // xorshift64* has one fixed point, an all-zero state, that it can
        // never leave. splitmix64 makes landing on it astronomically
        // unlikely, but "unlikely" is not "impossible" for a generator
        // millions of ticks are meant to run through.
        Self {
            state: if mixed == 0 { 1 } else { mixed },
        }
    }

    /// The next raw 64-bit output.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// The next raw 32-bit output.
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// A float uniformly distributed over `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / (1u32 << 24) as f32)
    }

    /// An integer uniformly distributed over `[0, bound)`.
    ///
    /// Returns 0 for a bound of 0, since there is no value in the range to
    /// return, and a planner tie-break has to return something.
    pub fn next_below(&mut self, bound: u32) -> u32 {
        if bound == 0 {
            0
        } else {
            self.next_u32() % bound
        }
    }
}

/// Bob Jenkins' / Sebastiano Vigna's splitmix64, used only to mix a seed —
/// not exposed, since [`Rng`] is the sequence callers should draw from.
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_and_tick_produce_the_same_sequence() {
        let mut a = Rng::new(42, 7);
        let mut b = Rng::new(42, 7);
        let sequence_a: Vec<u64> = (0..20).map(|_| a.next_u64()).collect();
        let sequence_b: Vec<u64> = (0..20).map(|_| b.next_u64()).collect();
        assert_eq!(sequence_a, sequence_b);
    }

    #[test]
    fn different_ticks_produce_different_sequences() {
        let mut a = Rng::new(42, 7);
        let mut b = Rng::new(42, 8);
        let sequence_a: Vec<u64> = (0..20).map(|_| a.next_u64()).collect();
        let sequence_b: Vec<u64> = (0..20).map(|_| b.next_u64()).collect();
        assert_ne!(sequence_a, sequence_b);
    }

    #[test]
    fn different_world_seeds_produce_different_sequences() {
        let mut a = Rng::new(1, 100);
        let mut b = Rng::new(2, 100);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn next_f32_stays_in_zero_one() {
        let mut rng = Rng::new(9, 0);
        for _ in 0..1000 {
            let value = rng.next_f32();
            assert!((0.0..1.0).contains(&value), "{value}");
        }
    }

    #[test]
    fn next_below_never_reaches_the_bound() {
        let mut rng = Rng::new(5, 5);
        for _ in 0..1000 {
            assert!(rng.next_below(7) < 7);
        }
        assert_eq!(rng.next_below(0), 0);
    }

    #[test]
    fn a_zero_seed_and_tick_do_not_produce_a_stuck_generator() {
        let mut rng = Rng::new(0, 0);
        let first = rng.next_u64();
        let second = rng.next_u64();
        assert_ne!(first, second);
        assert_ne!(first, 0);
    }
}
