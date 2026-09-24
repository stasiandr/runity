//! A fast hash for maps keyed by small integers: the cells of a spatial
//! grid, entities. The standard library's SipHash resists an attacker
//! choosing keys, which a grid of particles never is, and costs a tenth of
//! a simulation's step in hashing alone; this is the Firefox/rustc one (a
//! multiply and a rotate a word).

use std::hash::{BuildHasherDefault, Hasher};

/// The hasher: each word folded in by a rotate, an xor and a multiply.
#[derive(Debug, Clone, Copy, Default)]
pub struct FastHasher(u64);

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FastHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().expect("eight bytes")));
        }
        for &b in chunks.remainder() {
            self.add(b as u64);
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }

    #[inline]
    fn write_i32(&mut self, i: i32) {
        self.add(i as u32 as u64);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    #[inline]
    fn write_i64(&mut self, i: i64) {
        self.add(i as u64);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

/// A map with [`FastHasher`]: `FastMap::default()`.
pub type FastMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FastHasher>>;

/// A set with [`FastHasher`].
pub type FastSet<K> = std::collections::HashSet<K, BuildHasherDefault<FastHasher>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbouring_cells_spread_over_the_table() {
        let mut map: FastMap<(i32, i32, i32), u32> = FastMap::default();
        for x in -8..8 {
            for y in -8..8 {
                for z in -8..8 {
                    map.insert((x, y, z), 1);
                }
            }
        }
        assert_eq!(map.len(), 16 * 16 * 16);
        assert_eq!(map.get(&(-3, 4, 7)), Some(&1));
        // Distinct low bits for the cells of a small block: no pile-up.
        let bucket = |k: (i32, i32, i32)| {
            let mut h = FastHasher::default();
            std::hash::Hash::hash(&k, &mut h);
            h.finish() >> 57
        };
        let buckets: std::collections::HashSet<u64> =
            (0..4).flat_map(|x| (0..4).map(move |y| bucket((x, y, 0)))).collect();
        assert!(buckets.len() >= 10, "{buckets:?}");
    }
}
