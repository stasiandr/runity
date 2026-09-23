//! Networking, as the dacha simulator does it: shared authority, with a
//! server that caches and routes and rules on ownership but simulates
//! nothing.
//!
//! DNA, postulate 4, and the decision of 2026-09-23 that settled open
//! question 3 the way `~/personal/dacha-simulator/Docs/05-networking.md`
//! does. The layers, bottom up:
//!
//! * [`wire`] — datagrams to endpoints ([`Transport`]: UDP, a relay,
//!   Steam, a loopback), and [`Laggy`] to make any of them bad on purpose.
//! * [`link`] — connections over a wire: reliable-ordered and unreliable
//!   planes, pings, and knowing a goodbye from a silence.
//! * [`protocol`] — what a client and the server say to each other.
//! * [`server`] — the state cache and router, on its own thread in the
//!   host's process.
//! * [`sync`] — the world's end: what this peer owns goes out, what the
//!   others own comes in, claims are taken optimistically, and the others'
//!   things are shown a moment in the past, smoothly ([`sync::Presented`]).
//! * [`crate::party`] puts them together into the one object a game keeps.
//!
//! **Ownership is data, and changing it is a message.** Every networked
//! entity has an [`Owner`]. The peer that owns it has [`Owned`] on it and
//! simulates it; everyone else has [`Replica`] on it, and is told. The
//! convention from the dacha simulator, from day one and even alone: a
//! system that *simulates* filters `.with::<&Owned>()`, and anything that
//! only shows runs on everything. In a game alone everything is owned, so
//! the convention is free — and no system is rewritten for multiplayer.
//! Physics follows it by itself: a body that is a [`Replica`] is
//! kinematic, moved to where its owner says, one solver per body.
//!
//! **Which entities are networked:** the scene's, by the [`SceneId`] every
//! peer loaded from the same file, and whatever the game spawned through
//! [`crate::party::Party::spawn`], by a fresh [`NetId`]. Anything else the
//! game made is its own business, on its own machine.

pub mod link;
pub mod protocol;
pub mod server;
pub mod sync;
pub mod wire;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::id::EntityId;
use crate::world::SceneId;

pub use link::{Ended, Link, LinkEvent, Mode};
pub use wire::{Conditions, Laggy, Loopback, Nobody, Transport, Udp};

/// A participant, as the server numbers them: the host is 0, and every
/// client after it is the next number — never reused, so a rejoin is a new
/// peer. On a [`Transport`] it names an endpoint instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PeerId(pub u32);

impl PeerId {
    pub const HOST: PeerId = PeerId(0);
}

/// Which peer drives this entity. One without is the host's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner(pub PeerId);

/// This peer simulates it: what simulating systems filter on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owned;

/// Someone else simulates it; this peer shows what they say. A body with
/// this is kinematic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replica;

/// "I want to drive this": put it on an entity, and the next frame takes
/// it — optimistically, at once — and asks the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestOwnership;

/// Driven on an assumption: asked for, not yet granted. Work that can be
/// taken back reads [`Owned`]; work that cannot — despawning, spending —
/// waits for [`Owned`] without this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnershipPending;

/// Dies with its owner rather than passing to someone else: a player's
/// pawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DespawnWithOwner;

/// The network identity of an entity the game spawned at run time — one
/// the scene file does not have, so no [`SceneId`] names it. Minted by the
/// spawner, so the entity is live the same frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetId(pub EntityId);

/// Which prefab a run-time entity was spawned from, so a peer joining
/// later can spawn the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetPrefab(pub String);

/// The newest snapshot applied to a replica: whose, at what tick of
/// theirs, when it came, and whether it was the owner's last word before
/// going quiet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetTick {
    pub sender: PeerId,
    pub tick: u64,
    pub settled: bool,
    pub at: std::time::Instant,
}

/// Who owns an entity: its [`Owner`], or the host.
pub fn owner_of(world: &hecs::World, entity: hecs::Entity) -> PeerId {
    world.get::<&Owner>(entity).map_or(PeerId::HOST, |o| o.0)
}

/// Every entity the network can name: the scene's by [`SceneId`], the
/// run-time ones by [`NetId`].
pub fn addressable(world: &hecs::World) -> HashMap<EntityId, hecs::Entity> {
    let mut out: HashMap<EntityId, hecs::Entity> = world
        .query::<(hecs::Entity, &SceneId)>()
        .iter()
        .map(|(entity, id)| (id.0, entity))
        .collect();
    out.extend(
        world
            .query::<(hecs::Entity, &NetId)>()
            .iter()
            .map(|(entity, id)| (id.0, entity)),
    );
    out
}

/// How the network names an entity.
pub fn network_id(world: &hecs::World, entity: hecs::Entity) -> Option<EntityId> {
    world
        .get::<&NetId>(entity)
        .map(|n| n.0)
        .ok()
        .or_else(|| world.get::<&SceneId>(entity).map(|s| s.0).ok())
}

/// Mark an entity the game just spawned as networked and `me`'s: a fresh
/// [`NetId`], the prefab it came from, and the owner. The next frame
/// announces it. Returns its id.
pub fn announce(
    world: &mut hecs::World,
    entity: hecs::Entity,
    me: PeerId,
    prefab: &str,
) -> EntityId {
    let id = EntityId::fresh();
    let _ = world.insert(
        entity,
        (NetId(id), Owner(me), Owned, NetPrefab(prefab.to_string())),
    );
    id
}

/// An entity and everything parented to it.
pub(crate) fn despawn_tree(world: &mut hecs::World, root: hecs::Entity) {
    let mut doomed = vec![root];
    let mut i = 0;
    while i < doomed.len() {
        let parent = doomed[i];
        doomed.extend(
            world
                .query::<(hecs::Entity, &crate::world::Parent)>()
                .iter()
                .filter(|(_, p)| p.0 == parent)
                .map(|(e, _)| e),
        );
        i += 1;
    }
    for entity in doomed {
        let _ = world.despawn(entity);
    }
}
