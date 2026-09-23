//! What crosses the wire between a client and the server.
//!
//! Three planes, as in the dacha simulator: **control** (joining, spawns,
//! ownership, the scene — reliable and ordered), **snapshots** (what an
//! owner's entities are like now — unreliable and latest-wins, except the
//! last word before an entity goes quiet, which is reliable), and **RPC**
//! (a thing that happened once — reliable, broadcast). The server reads
//! control and routes the rest; a component's value is a [`Blob`] it
//! stores and forwards and never opens. The transform is a blob like any
//! other, named [`TRANSFORM`]: the server knows no more about where things
//! are than about whether a torch is lit.
//!
//! Variants are appended, never reordered: the binary encoding is their
//! position. Changing what a message carries is a [`PROTOCOL`] bump.

use serde::{Deserialize, Serialize};

use super::PeerId;
use crate::id::EntityId;

/// Bumped whenever a message changes shape. A client of another version
/// is turned away at the door rather than misread.
pub const PROTOCOL: u32 = 1;

/// The blob name the transform travels under.
pub const TRANSFORM: &str = "transform";

/// One component's value, by name: RON for the game's components,
/// postcard for the transform. Opaque to the server.
pub type Blob = (String, Vec<u8>);

/// One entity in a snapshot: its whole networked state. Whole, always —
/// a component missing from an owner's entry is one it no longer has, and
/// the server says so to everyone ([`ToClient::ComponentsRemoved`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: EntityId,
    pub blobs: Vec<Blob>,
}

/// An entity as the server holds it, for someone joining late.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: EntityId,
    pub owner: PeerId,
    /// Spawned at run time from this prefab; `None` for the scene's own.
    pub prefab: Option<String>,
    /// A scene entity somebody destroyed.
    pub gone: bool,
    pub blobs: Vec<Blob>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToServer {
    /// Let me in: my protocol, my components (a fingerprint of the
    /// networked ones, so two builds that disagree about what goes over
    /// the wire never shake hands), my name.
    JoinRequest {
        protocol: u32,
        fingerprint: u64,
        name: String,
    },
    /// I stand in the scene of this epoch: send me the world.
    Ready { epoch: u32 },
    /// I made this; it is mine.
    Spawn {
        epoch: u32,
        id: EntityId,
        prefab: String,
        despawn_with_owner: bool,
        blobs: Vec<Blob>,
    },
    /// Mine is gone.
    Despawn { epoch: u32, id: EntityId },
    /// I want to drive this.
    OwnershipRequest { epoch: u32, id: EntityId },
    /// What mine are like, at my tick. `settle`: the last word before
    /// they go quiet, sent reliably.
    Snapshot {
        epoch: u32,
        tick: u64,
        settle: bool,
        entries: Vec<Entry>,
    },
    /// Something that happened once, for everyone (or one peer).
    Rpc {
        to: Option<PeerId>,
        kind: String,
        body: Vec<u8>,
    },
    /// The host moves everyone to another scene.
    SetScene { scene: String },
    /// Leaving on purpose.
    Leave,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToClient {
    /// You are in: who you are, who hosts, which scene, which epoch of
    /// it, and which session this is.
    JoinAccepted {
        you: PeerId,
        host: PeerId,
        scene: String,
        epoch: u32,
        session: u64,
    },
    JoinRejected {
        reason: String,
    },
    /// Someone is in the game — replayed for everyone, yourself included,
    /// when you are ready.
    ClientJoined {
        peer: PeerId,
        name: String,
    },
    ClientLeft {
        peer: PeerId,
        clean: bool,
    },
    /// What the server holds, in chunks; the last says `last`.
    WorldState {
        records: Vec<Record>,
        last: bool,
    },
    Spawn {
        owner: PeerId,
        id: EntityId,
        prefab: String,
        blobs: Vec<Blob>,
    },
    Despawn {
        id: EntityId,
    },
    /// Who drives this now — told to everyone, the asker included, so
    /// nobody assumes a grant.
    OwnershipChanged {
        id: EntityId,
        owner: PeerId,
    },
    /// An owner's entry stopped naming these: they are gone. Carries the
    /// owner's tick, so a snapshot sent before it cannot put them back.
    ComponentsRemoved {
        id: EntityId,
        tick: u64,
        names: Vec<String>,
    },
    /// An owner's entities, at the owner's tick.
    Snapshot {
        owner: PeerId,
        tick: u64,
        settle: bool,
        entries: Vec<Entry>,
    },
    Rpc {
        from: PeerId,
        kind: String,
        body: Vec<u8>,
    },
    /// Everyone to another scene: purge, load, say ready again.
    SceneChanged {
        scene: String,
        epoch: u32,
    },
    /// The host is leaving on purpose; the session ends with it.
    SessionEnding,
}

/// Several messages in one datagram.
pub fn encode<T: Serialize>(messages: &[T]) -> Vec<u8> {
    postcard::to_stdvec(messages).unwrap_or_default()
}

/// A datagram's messages. Anything that does not read is dropped whole —
/// malformed input is expected input (an old build, a truncated datagram)
/// and must never take the session down.
pub fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<Vec<T>, String> {
    postcard::from_bytes(bytes).map_err(|e| format!("a datagram that does not read: {e}"))
}

/// Split messages into datagrams of about `budget` bytes: a world state
/// sent in one piece would be one lost datagram from never arriving on a
/// link that fragments. A message bigger than the budget goes alone.
pub fn pack<T: Serialize>(messages: Vec<T>, budget: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut batch: Vec<T> = Vec::new();
    let mut size = 0;
    for message in messages {
        let one = postcard::to_stdvec(&message).map(|b| b.len()).unwrap_or(0);
        if !batch.is_empty() && size + one > budget {
            out.push(encode(&batch));
            batch.clear();
            size = 0;
        }
        size += one;
        batch.push(message);
    }
    if !batch.is_empty() {
        out.push(encode(&batch));
    }
    out
}

/// The networked components' names, hashed (FNV-1a): what two builds must
/// agree on to play together.
pub fn fingerprint<'a>(names: impl IntoIterator<Item = &'a str>) -> u64 {
    let mut names: Vec<&str> = names.into_iter().collect();
    names.sort_unstable();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for name in names {
        for byte in name.bytes().chain([0]) {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_in_packed_datagrams() {
        let messages: Vec<ToClient> = (0..50)
            .map(|i| ToClient::Despawn {
                id: EntityId::from_raw(i + 1),
            })
            .collect();
        let packed = pack(messages.clone(), 64);
        assert!(packed.len() > 1, "split by the budget");
        let back: Vec<ToClient> = packed
            .iter()
            .flat_map(|d| decode::<ToClient>(d).unwrap())
            .collect();
        assert_eq!(back, messages);
        assert!(decode::<ToClient>(&[200, 1, 2]).is_err());
    }

    #[test]
    fn the_fingerprint_is_the_set_not_the_order() {
        assert_eq!(fingerprint(["lit", "hp"]), fingerprint(["hp", "lit"]));
        assert_ne!(fingerprint(["lit"]), fingerprint(["lit", "hp"]));
    }
}
