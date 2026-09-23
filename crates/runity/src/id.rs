//! Stable names for the entities in a scene.
//!
//! DNA, postulate 2: entities are addressed by IDs that survive insertion,
//! deletion and reordering — not by their position in the file, and not by
//! a hash of their contents. A position shifts every entity after it when
//! one is added, so two people editing one scene fight over numbers that
//! mean nothing; a hash changes whenever the thing it names changes, which
//! is exactly when it needs to stay put. An ID is assigned once and then
//! only ever copied.
//!
//! Random rather than counted. Two branches that each add an entity would
//! both mint "the next number", and the merge would have two entities with
//! one name. Sixty-four random bits make that a coincidence nobody will
//! meet, and whatever does collide — a block copy-pasted by hand, a merge
//! gone strange — is re-minted on load rather than trusted.
//!
//! Written as sixteen hex digits, because it has to survive a text editor, a
//! `git diff` and a language model, all of which mangle large integers in
//! their own ways and none of which touch a string.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One entity's identity within a scene or a prefab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct EntityId(u64);

impl EntityId {
    /// Not yet given an identity. What an entity built in code starts as,
    /// and what one read from a file without an `id` becomes until the
    /// scene assigns it one.
    pub const UNASSIGNED: EntityId = EntityId(0);

    /// A new identity, unlike any other in practice.
    ///
    /// Seeded from the standard library's per-process random hasher keys,
    /// a counter and the clock. Not cryptographic, and it does not need to
    /// be: it needs to not repeat across machines and branches, and it
    /// needs no dependency to do that.
    pub fn fresh() -> Self {
        use std::hash::{BuildHasher, Hasher};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            hasher.write_u128(now.as_nanos());
        }
        Self::nonzero(hasher.finish())
    }

    pub const fn from_raw(raw: u64) -> Self {
        EntityId(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub fn is_unassigned(&self) -> bool {
        *self == Self::UNASSIGNED
    }

    /// The identity of `inner` — an entity inside a prefab — as it appears
    /// inside the instance `self`.
    ///
    /// Derived, not stored. Two instances of one prefab hold the same inner
    /// IDs, so the inner ID alone is not unique in an expanded scene; mixed
    /// with the instance's it is, and it stays the same for as long as the
    /// instance and the prefab do — which is what an override or a network
    /// message will need to point at a stone inside a campfire.
    pub fn within(self, inner: EntityId) -> EntityId {
        Self::nonzero(mix(
            self.0 ^ mix(inner.0.wrapping_add(0x9e37_79b9_7f4a_7c15))
        ))
    }

    fn nonzero(raw: u64) -> Self {
        // Zero means unassigned. A generator that could produce it would
        // produce an entity that gets a new identity every time it loads.
        EntityId(if raw == 0 { 1 } else { raw })
    }
}

/// splitmix64's finaliser: every input bit reaches every output bit.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl std::str::FromStr for EntityId {
    type Err = String;

    /// Up to sixteen hex digits. Shorter is accepted, so a person or an agent
    /// writing a scene by hand can say `id: "a1"`; it is written back padded.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits = text.trim();
        if digits.is_empty() || digits.len() > 16 || !digits.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(format!(
                "entity id {text:?} is not up to sixteen hex digits, like \"4f1c0e9a7b3d2e65\""
            ));
        }
        u64::from_str_radix(digits, 16)
            .map(EntityId)
            .map_err(|e| e.to_string())
    }
}

impl Serialize for EntityId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for EntityId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_ids_do_not_repeat() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..10_000 {
            let id = EntityId::fresh();
            assert!(!id.is_unassigned());
            assert!(seen.insert(id), "{id} came up twice");
        }
    }

    #[test]
    fn an_id_is_written_as_sixteen_hex_digits_and_read_back() {
        let id = EntityId::from_raw(0x4f1c_0e9a_7b3d_2e65);
        let text = ron::to_string(&id).unwrap();
        assert_eq!(text, "\"4f1c0e9a7b3d2e65\"");
        assert_eq!(ron::from_str::<EntityId>(&text).unwrap(), id);

        // Short is fine to write by hand, and comes back padded.
        let short: EntityId = ron::from_str("\"a1\"").unwrap();
        assert_eq!(short.to_string(), "00000000000000a1");
    }

    #[test]
    fn a_malformed_id_says_what_an_id_looks_like() {
        let err = ron::from_str::<EntityId>("\"crate\"").unwrap_err();
        assert!(err.to_string().contains("hex"), "{err}");
    }

    #[test]
    fn a_derived_id_depends_on_both_halves_and_on_nothing_else() {
        let (fire_a, fire_b) = (EntityId::from_raw(1), EntityId::from_raw(2));
        let stone = EntityId::from_raw(7);
        // The same stone in two campfires is two different entities...
        assert_ne!(fire_a.within(stone), fire_b.within(stone));
        // ...and the same one every time it is asked.
        assert_eq!(fire_a.within(stone), fire_a.within(stone));
        assert!(!fire_a.within(stone).is_unassigned());
    }
}
