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


use serde::{Deserialize, Serialize};

use crate::id::EntityId;

pub use crate::world::{addressable, despawn_tree, network_id, NetId, NetPrefab};
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

// Who simulates an entity is the core's to say (every simulating module
// filters on it); which peer, and how it changes hands, is this module's.
pub use crate::world_core::{Owned, Replica};

// The speed a body had with its old owner is the physics module's.
pub use crate::bodies::Takeover;

/// "I want to drive this": put it on an entity, and the next frame takes
/// it — optimistically, at once — and asks the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestOwnership;

/// Claims what comes near: every loose body (a dynamic one) within
/// `radius` metres of this entity, when it is nearer this than any other
/// peer's claimer by half a metre — the dacha simulator's claim on
/// approach, so what a player reaches for is theirs before the hand
/// arrives, and two players side by side do not pull it back and forth.
/// Put it on the player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClaimNear {
    pub radius: f32,
}

/// How much nearer a claimer must be than any other to take a body.
pub const CLAIM_MARGIN: f32 = 0.5;

/// Ask for every loose body a claimer of `me` has come nearest: see
/// [`ClaimNear`].
pub fn claim_nearby(world: &mut hecs::World, me: PeerId) {
    use crate::world::WorldTransform;
    let claimers: Vec<(PeerId, glam::Vec3, f32, Option<NetGroup>)> = world
        .query::<(hecs::Entity, &ClaimNear, &WorldTransform)>()
        .iter()
        .map(|(e, near, placed)| {
            (
                owner_of(world, e),
                placed.0.w_axis.truncate(),
                near.radius,
                world.get::<&NetGroup>(e).ok().map(|g| *g),
            )
        })
        .collect();
    if !claimers.iter().any(|(owner, ..)| *owner == me) {
        return;
    }
    let mut wanted = Vec::new();
    for (entity, physics, placed) in world
        .query::<(hecs::Entity, &crate::world::Physics, &WorldTransform)>()
        .without::<(&ClaimNear, &OwnershipPending, &RequestOwnership)>()
        .iter()
    {
        if physics.0 != crate::scene::Body::Dynamic || owner_of(world, entity) == me {
            continue;
        }
        // A claimer's own parts (a hand) are its group's business.
        let group = world.get::<&NetGroup>(entity).ok().map(|g| *g);
        if group.is_some() && claimers.iter().any(|c| c.3 == group) {
            continue;
        }
        let at = placed.0.w_axis.truncate();
        let nearest = |mine: bool| {
            claimers
                .iter()
                .filter(|(owner, ..)| (*owner == me) == mine)
                .filter(|(_, p, r, _)| !mine || p.distance(at) <= *r)
                .map(|(_, p, ..)| p.distance(at))
                .min_by(f32::total_cmp)
        };
        let Some(mine) = nearest(true) else {
            continue;
        };
        if nearest(false).is_none_or(|theirs| mine + CLAIM_MARGIN < theirs) {
            wanted.push(entity);
        }
    }
    for entity in wanted {
        let _ = world.insert_one(entity, RequestOwnership);
    }
}

/// Owned together: a claim on one of a group's entities is a claim on
/// every one — the player and both hands, never one hand on another
/// machine. See [`crate::party::Party::group`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetGroup(pub EntityId);

/// Driven on an assumption: asked for, not yet granted. Work that can be
/// taken back reads [`Owned`]; work that cannot — despawning, spending —
/// waits for [`Owned`] without this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnershipPending;

/// Dies with its owner rather than passing to someone else: a player's
/// pawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DespawnWithOwner;



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



/// The network's view of every networked entity, for a report.
pub fn net_lines(world: &hecs::World) -> Vec<crate::save_core::NetLine> {
    let mut out: Vec<crate::save_core::NetLine> = world
        .query::<(
            hecs::Entity,
            &NetId,
            Option<&crate::net::Replica>,
            Option<&crate::net::NetTick>,
        )>()
        .iter()
        .map(|(entity, id, replica, tick)| crate::save_core::NetLine {
            id: id.0,
            owner: crate::net::owner_of(world, entity).0,
            replica: replica.is_some(),
            tick: tick.map(|t| (t.tick, t.at.elapsed().as_secs_f32() * 1000.0)),
        })
        .collect();
    out.sort_by_key(|l| l.id);
    out
}

