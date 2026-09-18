//! Deterministic pseudo-random numbers.
//!
//! A world that plays itself is only debuggable if it can be replayed, so
//! nothing here reads a clock or the operating system: a generator is fully
//! described by its seed and its stream, and the same pair always produces the
//! same sequence on every machine.
//!
//! **Streams** are the important part. Give terrain generation, villager
//! decisions and combat their own named streams and they stop interfering:
//! adding one die roll to combat no longer changes where the forests are.
//!
//! The generator is PCG32 (O'Neill, 2014): a 64-bit LCG whose output is
//! scrambled by an xorshift and a variable rotate. Small, fast, and it passes
//! the statistical tests that a bare LCG fails badly.

use crate::vec::{Vec2, Vec3};

const MULTIPLIER: u64 = 6_364_136_223_846_793_005;
const DEFAULT_STREAM: u64 = 1_442_695_040_888_963_407;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    state: u64,
    /// Must be odd; it selects which of the 2^63 streams this generator walks.
    increment: u64,
}

impl Default for Rng {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Rng {
    /// A generator for this seed, on the default stream.
    pub fn new(seed: u64) -> Self {
        Self::with_stream(seed, DEFAULT_STREAM)
    }

    /// A generator for this seed on a chosen stream. Two streams with the same
    /// seed produce unrelated sequences.
    pub fn with_stream(seed: u64, stream: u64) -> Self {
        let mut rng = Self {
            state: 0,
            increment: (stream << 1) | 1,
        };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(seed);
        rng.next_u32();
        rng
    }

    /// A generator on the stream named by a string — the readable way to keep
    /// subsystems from disturbing each other.
    ///
    /// ```
    /// use runity_math::Rng;
    /// let terrain = Rng::named(1234, "terrain");
    /// let villagers = Rng::named(1234, "villagers");
    /// assert_ne!(terrain, villagers);
    /// ```
    pub fn named(seed: u64, name: &str) -> Self {
        Self::with_stream(seed, hash_str(name))
    }

    /// A child generator, derived deterministically from this one's state.
    ///
    /// Use it to give a new entity its own generator without threading the
    /// parent's through everything it does.
    pub fn fork(&mut self) -> Rng {
        let seed = self.next_u64();
        let stream = self.next_u64();
        Self::with_stream(seed, stream)
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(MULTIPLIER).wrapping_add(self.increment);
        // Xorshift the high bits down, then rotate by a count taken from the
        // very top — that variable rotation is what breaks up the LCG's
        // regularity in the low bits.
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rotation = (old >> 59) as u32;
        xorshifted.rotate_right(rotation)
    }

    pub fn next_u64(&mut self) -> u64 {
        ((self.next_u32() as u64) << 32) | self.next_u32() as u64
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        // 24 bits is exactly the mantissa of an f32; taking the high bits keeps
        // the best-distributed part of the word.
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in `[low, high)`.
    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next_f32()
    }

    /// Uniform integer in `[low, high)`. Returns `low` for an empty range.
    pub fn range_i32(&mut self, low: i32, high: i32) -> i32 {
        if high <= low {
            return low;
        }
        let span = (high - low) as u32;
        low + self.below(span) as i32
    }

    /// Uniform in `[0, bound)`, without the modulo bias.
    pub fn below(&mut self, bound: u32) -> u32 {
        if bound == 0 {
            return 0;
        }
        // Discard the values that would make the last, partial block of
        // `bound` values more likely than the others.
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let value = self.next_u32();
            if value >= threshold {
                return value % bound;
            }
        }
    }

    /// True with the given probability.
    pub fn chance(&mut self, probability: f32) -> bool {
        self.next_f32() < probability
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            return None;
        }
        items.get(self.below(items.len() as u32) as usize)
    }

    /// Fisher-Yates, which is uniform over permutations.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            items.swap(i, j);
        }
    }

    /// A direction, uniform over the sphere.
    ///
    /// Sampling z uniformly and then the angle is what makes it uniform over
    /// *area*; picking two angles instead would crowd the poles.
    pub fn unit_vector(&mut self) -> Vec3 {
        let z = self.range(-1.0, 1.0);
        let angle = self.range(0.0, core::f32::consts::TAU);
        let radius = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(radius * angle.cos(), radius * angle.sin(), z)
    }

    /// A direction in the XZ plane, uniform over the circle.
    pub fn unit_vector_xz(&mut self) -> Vec3 {
        let angle = self.range(0.0, core::f32::consts::TAU);
        Vec3::new(angle.cos(), 0.0, angle.sin())
    }

    /// A point uniformly inside the unit disc.
    pub fn in_unit_disc(&mut self) -> Vec2 {
        // The square root is what keeps the samples from bunching at the middle.
        let radius = self.next_f32().sqrt();
        let angle = self.range(0.0, core::f32::consts::TAU);
        Vec2::new(radius * angle.cos(), radius * angle.sin())
    }

    /// Approximately normal, mean 0 and standard deviation 1.
    ///
    /// The sum of twelve uniforms, which is close enough for spreading trees
    /// around and far cheaper than a Box-Muller transform.
    pub fn normal(&mut self) -> f32 {
        let mut sum = 0.0;
        for _ in 0..12 {
            sum += self.next_f32();
        }
        sum - 6.0
    }
}

/// FNV-1a, used to turn a stream name into a stream number.
fn hash_str(name: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn different_seeds_and_streams_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());

        let mut terrain = Rng::named(7, "terrain");
        let mut villagers = Rng::named(7, "villagers");
        let first: Vec<u32> = (0..8).map(|_| terrain.next_u32()).collect();
        let second: Vec<u32> = (0..8).map(|_| villagers.next_u32()).collect();
        assert_ne!(first, second, "named streams must not overlap");
    }

    #[test]
    fn a_stream_is_unaffected_by_draws_from_another() {
        // The property that makes streams worth having: adding a die roll to
        // one system cannot move the forests.
        let expected: Vec<u32> = {
            let mut terrain = Rng::named(99, "terrain");
            (0..5).map(|_| terrain.next_u32()).collect()
        };

        let mut terrain = Rng::named(99, "terrain");
        let mut combat = Rng::named(99, "combat");
        let mut actual = Vec::new();
        for _ in 0..5 {
            combat.next_u32();
            combat.next_u32();
            actual.push(terrain.next_u32());
        }
        assert_eq!(expected, actual);
    }

    #[test]
    fn floats_stay_in_range_and_spread_out() {
        let mut rng = Rng::new(3);
        let mut buckets = [0u32; 10];
        const SAMPLES: u32 = 100_000;
        let mut sum = 0.0f64;
        for _ in 0..SAMPLES {
            let value = rng.next_f32();
            assert!((0.0..1.0).contains(&value), "{value}");
            sum += value as f64;
            buckets[(value * 10.0) as usize] += 1;
        }
        let mean = sum / SAMPLES as f64;
        assert!((mean - 0.5).abs() < 0.01, "mean {mean}");
        // Every tenth should hold about a tenth of the samples.
        for (index, count) in buckets.iter().enumerate() {
            let share = *count as f64 / SAMPLES as f64;
            assert!((share - 0.1).abs() < 0.01, "bucket {index} holds {share}");
        }
    }

    #[test]
    fn bounded_integers_are_unbiased_and_in_range() {
        let mut rng = Rng::new(11);
        let mut counts = [0u32; 3];
        for _ in 0..30_000 {
            let value = rng.below(3);
            assert!(value < 3);
            counts[value as usize] += 1;
        }
        for count in counts {
            assert!(
                (count as f64 / 30_000.0 - 1.0 / 3.0).abs() < 0.02,
                "{counts:?}"
            );
        }

        assert_eq!(rng.below(0), 0, "an empty range is not a panic");
        assert_eq!(rng.below(1), 0);
        for _ in 0..100 {
            let value = rng.range_i32(-5, 5);
            assert!((-5..5).contains(&value), "{value}");
        }
        assert_eq!(rng.range_i32(4, 4), 4, "an empty integer range collapses");
    }

    #[test]
    fn chance_matches_its_probability() {
        let mut rng = Rng::new(5);
        let hits = (0..20_000).filter(|_| rng.chance(0.25)).count();
        assert!((hits as f64 / 20_000.0 - 0.25).abs() < 0.02, "{hits}");
        assert!(!rng.chance(0.0));
        assert!(rng.chance(1.0));
    }

    #[test]
    fn shuffling_produces_a_permutation() {
        let mut rng = Rng::new(17);
        let mut items: Vec<u32> = (0..64).collect();
        rng.shuffle(&mut items);
        assert_ne!(items, (0..64).collect::<Vec<_>>(), "it actually moved");
        items.sort_unstable();
        assert_eq!(
            items,
            (0..64).collect::<Vec<_>>(),
            "nothing lost or duplicated"
        );

        // Empty and single-element slices are not a special case.
        rng.shuffle(&mut [] as &mut [u32]);
        rng.shuffle(&mut [1]);
    }

    #[test]
    fn picking_from_an_empty_slice_is_none() {
        let mut rng = Rng::new(1);
        let empty: [u32; 0] = [];
        assert!(rng.pick(&empty).is_none());
        assert_eq!(rng.pick(&[7]), Some(&7));
    }

    #[test]
    fn unit_vectors_are_unit_length_and_cover_the_sphere() {
        let mut rng = Rng::new(23);
        let mut mean = Vec3::ZERO;
        const SAMPLES: usize = 20_000;
        for _ in 0..SAMPLES {
            let v = rng.unit_vector();
            assert!((v.length() - 1.0).abs() < 1e-5, "{v:?}");
            mean += v;
        }
        // Uniform over the sphere means the average points nowhere.
        assert!(
            (mean.length() / SAMPLES as f32) < 0.02,
            "{:?}",
            mean * (1.0 / SAMPLES as f32)
        );

        let flat = rng.unit_vector_xz();
        assert_eq!(flat.y, 0.0);
        assert!((flat.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn disc_samples_fill_the_disc_evenly() {
        let mut rng = Rng::new(31);
        let mut inner = 0;
        const SAMPLES: usize = 20_000;
        for _ in 0..SAMPLES {
            let p = rng.in_unit_disc();
            let radius = p.length();
            assert!(radius <= 1.0 + 1e-6, "{p:?}");
            // Half the area is inside radius 1/sqrt(2).
            if radius < core::f32::consts::FRAC_1_SQRT_2 {
                inner += 1;
            }
        }
        let share = inner as f64 / SAMPLES as f64;
        assert!((share - 0.5).abs() < 0.02, "{share}");
    }

    #[test]
    fn a_forked_generator_is_independent_but_reproducible() {
        let mut parent = Rng::new(77);
        let mut child = parent.fork();
        let from_child: Vec<u32> = (0..4).map(|_| child.next_u32()).collect();

        let mut same_parent = Rng::new(77);
        let mut same_child = same_parent.fork();
        let again: Vec<u32> = (0..4).map(|_| same_child.next_u32()).collect();
        assert_eq!(from_child, again, "forking is deterministic");

        let from_parent: Vec<u32> = (0..4).map(|_| parent.next_u32()).collect();
        assert_ne!(from_child, from_parent);
    }

    #[test]
    fn the_normal_distribution_is_centered_and_spread() {
        let mut rng = Rng::new(13);
        const SAMPLES: usize = 50_000;
        let values: Vec<f32> = (0..SAMPLES).map(|_| rng.normal()).collect();
        let mean = values.iter().sum::<f32>() / SAMPLES as f32;
        let variance = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / SAMPLES as f32;
        assert!(mean.abs() < 0.02, "mean {mean}");
        assert!((variance - 1.0).abs() < 0.05, "variance {variance}");
    }
}
