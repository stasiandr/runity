//! What crosses the wire between a client and the server.
//!
//! Three planes, as in the dacha simulator: **control** (joining, spawns,
//! ownership, the scene — reliable and ordered), **snapshots** (what an
//! owner's entities are like now — unreliable and latest-wins, except the
//! last word before an entity goes quiet, which is reliable), and **RPC**
//! (a thing that happened once — reliable, broadcast). The server reads
//! control and routes the rest; a component's value is a [`Blob`] it
//! stores and forwards and never opens. The transform is a blob like any
//! other, numbered [`TRANSFORM`]: the server knows no more about where
//! things are than about whether a torch is lit. A blob goes by a number,
//! not its name: the networked names sorted, counted from one — both ends
//! have the same list, the fingerprint made sure at the door.
//!
//! Variants are appended, never reordered: the binary encoding is their
//! position. Changing what a message carries is a [`PROTOCOL`] bump.

use serde::{Deserialize, Serialize};

use super::PeerId;
use crate::id::EntityId;

/// Bumped whenever a message changes shape. A client of another version
/// is turned away at the door rather than misread.
pub const PROTOCOL: u32 = 3;

/// A blob's number: [`TRANSFORM`], or one past the component's place among
/// the networked names sorted (`Components::networked_id`).
pub type BlobId = u16;

/// The number the transform travels under.
pub const TRANSFORM: BlobId = 0;

/// One component's value, by number: RON for the game's components,
/// [`encode_transform`] for the transform, a simulation's own bytes for a
/// state. Opaque to the server.
pub type Blob = (BlobId, Vec<u8>);

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
    /// What time is it on the server? `sent`: my clock, seconds, for
    /// the answer to bring back.
    Clock { sent: f64 },
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
        names: Vec<BlobId>,
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
    /// The server's clock, seconds since the session began, answering a
    /// [`ToServer::Clock`] sent at `sent` on the asker's.
    Clock {
        sent: f64,
        server: f64,
    },
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

/// Of a metre, the step positions go in: a millimetre.
pub const POSITION_STEP: f32 = 1.0 / 1000.0;

/// A transform in about a dozen bytes (docs/netsim.md, «Трафик»): a flag
/// byte, the position in millimetres as three zigzag varints, the turn as
/// the smallest three in six bytes (fifteen bits each, some thousandths of
/// a degree), and the scale whole only when it is not one. What a body at
/// rest trembles by in the last bits of an f32 does not show in these
/// bytes, so a resting thing reads as unchanged and is not sent.
pub fn encode_transform(t: &crate::scene::Transform) -> Vec<u8> {
    let scaled = (t.scale - glam::Vec3::ONE).abs().max_element() > 1e-6;
    let mut out = Vec::with_capacity(16);
    out.push(scaled as u8);
    for v in t.position.to_array() {
        let q = (v / POSITION_STEP).round().clamp(i32::MIN as f32, i32::MAX as f32) as i32;
        varint(&mut out, ((q << 1) ^ (q >> 31)) as u32);
    }
    out.extend_from_slice(&pack_turn(t.rotation()).to_le_bytes()[..6]);
    if scaled {
        for v in t.scale.to_array() {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// [`encode_transform`]'s bytes back; `None` if they do not read.
pub fn decode_transform(bytes: &[u8]) -> Option<crate::scene::Transform> {
    let (&flags, mut rest) = bytes.split_first()?;
    let mut position = [0.0f32; 3];
    for v in &mut position {
        let (z, after) = read_varint(rest)?;
        rest = after;
        let q = ((z >> 1) as i32) ^ -((z & 1) as i32);
        *v = q as f32 * POSITION_STEP;
    }
    let mut turn = [0u8; 8];
    turn[..6].copy_from_slice(rest.get(..6)?);
    rest = &rest[6..];
    let mut scale = glam::Vec3::ONE;
    if flags & 1 != 0 {
        let mut s = [0.0f32; 3];
        for v in &mut s {
            *v = f32::from_le_bytes(rest.get(..4)?.try_into().ok()?);
            rest = &rest[4..];
        }
        scale = glam::Vec3::from_array(s);
    }
    let mut t = crate::scene::Transform { position: glam::Vec3::from_array(position), scale, ..Default::default() };
    t.set_rotation(unpack_turn(u64::from_le_bytes(turn)));
    Some(t)
}

fn varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn read_varint(bytes: &[u8]) -> Option<(u32, &[u8])> {
    let mut v = 0u32;
    for (i, &b) in bytes.iter().enumerate().take(5) {
        v |= ((b & 0x7f) as u32) << (7 * i);
        if b & 0x80 == 0 {
            return Some((v, &bytes[i + 1..]));
        }
    }
    None
}

/// Bits a kept component of a turn goes in.
const TURN_BITS: u32 = 15;

/// A unit quaternion in 47 bits: the largest component dropped (its index
/// in two bits, its sign made positive), the other three in
/// [`TURN_BITS`] each over ±1/√2.
fn pack_turn(q: glam::Quat) -> u64 {
    let q = q.normalize();
    let a = q.to_array();
    let (largest, _) = a.iter().enumerate().fold((0, 0.0f32), |(i, m), (j, v)| if v.abs() > m { (j, v.abs()) } else { (i, m) });
    let sign = if a[largest] < 0.0 { -1.0 } else { 1.0 };
    let most = ((1u64 << TURN_BITS) - 1) as f32;
    let mut out = largest as u64;
    let mut shift = 2;
    for (j, v) in a.iter().enumerate() {
        if j != largest {
            let x = (v * sign * std::f32::consts::SQRT_2 * 0.5 + 0.5).clamp(0.0, 1.0);
            out |= ((x * most).round() as u64) << shift;
            shift += TURN_BITS;
        }
    }
    out
}

fn unpack_turn(bits: u64) -> glam::Quat {
    let largest = (bits & 3) as usize;
    let mask = (1u64 << TURN_BITS) - 1;
    let most = mask as f32;
    let mut a = [0.0f32; 4];
    let mut shift = 2;
    let mut sum = 0.0;
    for (j, slot) in a.iter_mut().enumerate() {
        if j != largest {
            let x = ((bits >> shift) & mask) as f32 / most;
            *slot = (x - 0.5) * 2.0 / std::f32::consts::SQRT_2;
            sum += *slot * *slot;
            shift += TURN_BITS;
        }
    }
    a[largest] = (1.0 - sum).max(0.0).sqrt();
    glam::Quat::from_array(a).normalize()
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
    fn a_transform_goes_in_a_dozen_bytes_and_comes_back_close() {
        use crate::scene::Transform;
        use glam::{Quat, Vec3};
        let mut t = Transform { position: Vec3::new(-3.2104, 0.3, 12.5), ..Default::default() };
        t.set_rotation(Quat::from_euler(glam::EulerRot::YXZ, 1.1, -0.4, 2.9));
        let bytes = encode_transform(&t);
        assert!(bytes.len() <= 14, "{} bytes", bytes.len());
        let back = decode_transform(&bytes).unwrap();
        assert!((back.position - t.position).abs().max_element() <= POSITION_STEP * 0.5 + 1e-5);
        assert!(back.rotation().angle_between(t.rotation()) < 1e-3, "{}", back.rotation().angle_between(t.rotation()));
        assert_eq!(back.scale, Vec3::ONE);
        let scaled = Transform { scale: Vec3::new(2.0, 1.0, 0.5), ..t };
        assert_eq!(decode_transform(&encode_transform(&scaled)).unwrap().scale, scaled.scale);
        assert_eq!(decode_transform(&bytes[..5]), None);
        // What a body at rest trembles by is not in the bytes.
        let trembling = Transform { position: t.position + Vec3::splat(1e-6), ..t };
        assert_eq!(encode_transform(&trembling), bytes);
    }

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
