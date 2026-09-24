//! How a simulation goes over the network (docs/netsim.md): the mode every
//! simulated part carries in its `net:` field, and what the network module
//! and the simulating modules agree on without knowing each other.

use serde::{Deserialize, Serialize};

use crate::id::EntityId;

/// A simulation's way over the network. `Local` unless the line says:
/// the game turns the network on where it needs it, and pays nothing for
/// what it did not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum NetMode {
    /// Everyone simulates their own; nothing goes over the wire beyond
    /// what it already follows (a transform, the clock, the wind).
    #[default]
    Local,
    /// Everyone simulates their own from what is shared — what it hangs
    /// on, the wind, the session's clock — and a sparse summary pulls it
    /// back toward the owner's.
    Rough,
    /// The owner simulates; the others show its state a moment late and
    /// take it over without a jump when it changes hands.
    Full,
    /// Everyone plays the same event from the same seed: it is sent once,
    /// reliably, and the rest is deterministic.
    Event,
}

impl NetMode {
    pub fn is_local(&self) -> bool {
        *self == NetMode::Local
    }

    /// Whether a peer that does not own the entity simulates it: every
    /// mode but `Full`, whose replicas show the owner's state.
    pub fn simulated_by_replicas(self) -> bool {
        self != NetMode::Full
    }
}

/// Entities that have to be simulated by one peer with this one: a rope
/// and the load on its end, a ragdoll's parts. Whoever takes one takes
/// them all — two machines solving one rope pull it apart. Kept by the
/// module that ties them; the network module only reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tied(pub Vec<EntityId>);

/// Time since the session began, seconds, the same on every peer give or
/// take the clock's error: what a `Rough` simulation's gusts and waves are
/// drawn from, so everyone's flag flaps alike. One entity carries it; the
/// network module writes it, alone it is the game's own clock.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SessionClock(pub f64);

/// The session's clock in `world`, if anything keeps one.
pub fn session_time(world: &hecs::World) -> Option<f64> {
    world.query::<&SessionClock>().iter().next().map(|c| c.0)
}

/// Set the session's clock in `world`.
pub fn set_session_time(world: &mut hecs::World, seconds: f64) {
    if let Some(clock) = world.query_mut::<&mut SessionClock>().into_iter().next() {
        clock.0 = seconds;
        return;
    }
    world.spawn((SessionClock(seconds),));
}

/// How late the owners' states arrive here, in network ticks, one way:
/// what a takeover carries a state forward by. The network module writes
/// it on the entity that keeps the [`SessionClock`]; alone it is 0.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LinkDelay(pub f64);

/// The one-way delay in `world`, network ticks.
pub fn link_delay(world: &hecs::World) -> f64 {
    world.query::<&LinkDelay>().iter().next().map_or(0.0, |d| d.0)
}

/// Set the one-way delay in `world`.
pub fn set_link_delay(world: &mut hecs::World, ticks: f64) {
    let clock = world.query::<(hecs::Entity, &SessionClock)>().iter().next().map(|(e, _)| e);
    let entity = clock.unwrap_or_else(|| world.spawn((SessionClock(0.0),)));
    let _ = world.insert_one(entity, LinkDelay(ticks));
}

/// A player's own body: it belongs to its player for as long as it
/// exists. Gathering what is tied to it stops at it — a rope two players
/// hold is not dragged, with one of them, to the other's machine — and
/// nothing is gathered along with it. The game claims it for its player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pawn;

/// Named like a scene's entity, but not the network's: every peer has its
/// own. A `Local` ragdoll's parts — their ids name each other's joints,
/// and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Unshared;
