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

/// The host's randomness, mixed into fresh identities ([`EntityId::seed`]).
static SEED: AtomicU64 = AtomicU64::new(0);

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
        hasher.write_u64(SEED.load(Ordering::Relaxed));
        // No clock on the web (wasm32 panics asking): there the host's seed
        // stands in for it.
        #[cfg(not(target_arch = "wasm32"))]
        if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            hasher.write_u128(now.as_nanos());
        }
        Self::nonzero(hasher.finish())
    }

    /// Mix the host's randomness into every fresh identity from now on:
    /// where the process has no clock or random keys of its own — the web,
    /// with `crypto.getRandomValues` — so two players' fresh IDs differ.
    pub fn seed(random: u64) {
        SEED.fetch_xor(random, Ordering::Relaxed);
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

/// Hex text in a file a person reads; the number itself in a binary
/// format (the wire), where the text would be seventeen bytes for a
/// number of at most ten.
impl Serialize for EntityId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.collect_str(self)
        } else {
            serializer.serialize_u64(self.0)
        }
    }
}

/// What an [`EntityId`] read from text says it expects: how
/// [`crate::shape`] knows a field holds one — a link to an entity.
pub const ID_EXPECTING: &str = "an entity id: up to sixteen hex digits";

impl<'de> Deserialize<'de> for EntityId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Text;
        impl serde::de::Visitor<'_> for Text {
            type Value = EntityId;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(ID_EXPECTING)
            }
            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<EntityId, E> {
                text.parse().map_err(E::custom)
            }
        }
        if deserializer.is_human_readable() {
            deserializer.deserialize_str(Text)
        } else {
            u64::deserialize(deserializer).map(EntityId)
        }
    }
}

/// A link from a game's component to another entity of the scene: Unity's
/// object field. A door names its switch, a spawner its spawn point.
///
/// It holds the entity's stable ID, so it survives the target being
/// renamed, moved in the hierarchy or reordered. In a scene it reads
/// `EntityRef("4f1c0e9a7b3d2e65")`; `EntityRef("")` links nothing. The
/// editor shows a field of this type as a picker, and `check` names a link
/// whose entity is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EntityRef(pub Option<EntityId>);

impl EntityRef {
    /// The name its type goes by in a file, and what an editor recognises.
    pub const NAME: &'static str = "EntityRef";

    pub fn to(id: EntityId) -> Self {
        EntityRef(Some(id))
    }

    /// The entity it links to in a running world: the one spawned from the
    /// scene line with that ID, or — in something the game spawned from a
    /// prefab — from that prefab's line.
    pub fn get(&self, world: &hecs::World) -> Option<hecs::Entity> {
        let id = self.0?;
        world
            .query::<(hecs::Entity, &crate::world::SceneId)>()
            .iter()
            .find(|(_, scene)| scene.0 == id)
            .map(|(entity, _)| entity)
            .or_else(|| {
                world
                    .query::<(hecs::Entity, &crate::world::SpawnedId)>()
                    .iter()
                    .find(|(_, spawned)| spawned.0 == id)
                    .map(|(entity, _)| entity)
            })
    }

    /// Every entity ID linked in a value's RON text: what `check` looks
    /// for among the scene's entities.
    pub fn find_in(text: &str) -> Vec<EntityId> {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find("EntityRef(\"") {
            rest = &rest[at + "EntityRef(\"".len()..];
            let Some(end) = rest.find('"') else { break };
            if let Ok(id) = rest[..end].parse::<EntityId>() {
                out.push(id);
            }
            rest = &rest[end..];
        }
        out
    }
}

impl Serialize for EntityRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let text = self.0.map(|id| id.to_string()).unwrap_or_default();
        serializer.serialize_newtype_struct(Self::NAME, &text)
    }
}

impl<'de> Deserialize<'de> for EntityRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Link;
        impl<'de> serde::de::Visitor<'de> for Link {
            type Value = EntityRef;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "EntityRef(\"entity id\")")
            }
            fn visit_newtype_struct<D: Deserializer<'de>>(
                self,
                deserializer: D,
            ) -> Result<EntityRef, D::Error> {
                let text = String::deserialize(deserializer)?;
                self.visit_str(&text)
            }
            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<EntityRef, E> {
                if text.trim().is_empty() {
                    return Ok(EntityRef(None));
                }
                text.parse()
                    .map(|id| EntityRef(Some(id)))
                    .map_err(E::custom)
            }
        }
        deserializer.deserialize_newtype_struct(Self::NAME, Link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_reads_and_writes_as_its_name_and_an_id() {
        let id: EntityId = "4f1c".parse().unwrap();
        let link: EntityRef = ron::from_str(r#"EntityRef("4f1c")"#).unwrap();
        assert_eq!(link, EntityRef::to(id));
        let none: EntityRef = ron::from_str(r#"EntityRef("")"#).unwrap();
        assert_eq!(none, EntityRef(None));
        let back: EntityRef = ron::from_str(&ron::to_string(&link).unwrap()).unwrap();
        assert_eq!(back, link);
        assert_eq!(
            EntityRef::find_in(r#"(target: EntityRef("4f1c"), other: EntityRef(""))"#),
            vec![id]
        );
    }

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
