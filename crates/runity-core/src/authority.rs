//! Who is allowed to change what, and in what order it happens.
//!
//! The world in this engine is meant to keep developing on its own and to be
//! shared, which means two machines can want to change the same thing at the
//! same time. Fully authoritative servers solve that by letting only the
//! server decide, at the cost of a delay on everything a player does. Shared
//! authority instead hands out ownership: a player owns their own character
//! and whatever they are working on, and the server owns everything else,
//! including the settlement's slow simulation.
//!
//! Two rules make that safe enough to build on:
//!
//! * **Ownership is a lease, not a gift.** It expires. A client that crashes
//!   mid-swing does not take the tree it was felling out of the world with
//!   it — the lease runs out and the server takes over.
//! * **Conflicts resolve the same way everywhere.** Two claims on the same
//!   tick are settled by peer id, not by which packet happened to arrive
//!   first. Arrival order differs between machines; peer ids do not.
//!
//! What this module deliberately does not do is decide whether a command is
//! *reasonable* — whether that villager really had the wood, whether the
//! player could reach that far. That is the game's rules, and a network layer
//! guessing at them would be both wrong and in the way.

use std::collections::HashMap;

use runity_serialize::{serializable, Deserialize, Reader, Result, Serialize, Writer};

use crate::world::Entity;

/// Who a peer is, in a way both sides agree on.
///
/// Zero is the server: it owns whatever nobody else has claimed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId(pub u32);

impl PeerId {
    /// The server, which owns the world by default.
    pub const SERVER: PeerId = PeerId(0);

    /// Whether this is the server.
    pub fn is_server(self) -> bool {
        self == PeerId::SERVER
    }
}

serializable!(PeerId(0));

/// One peer's hold on one entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    /// Who holds it.
    pub owner: PeerId,
    /// The tick the claim was made on, which is how ties are broken.
    pub claimed: u64,
    /// The tick it lapses on, unless renewed.
    pub expires: u64,
}

serializable!(Lease {
    owner,
    claimed,
    expires
});

/// Who owns what.
#[derive(Debug, Default, Clone)]
pub struct Authority {
    leases: HashMap<Entity, Lease>,
    /// How long a lease lasts without being renewed.
    pub duration: u64,
}

impl Authority {
    /// An empty map where leases last `duration` ticks.
    ///
    /// A few seconds' worth is the usual choice: long enough to survive a
    /// hiccup, short enough that a crashed client does not hold a tree for a
    /// minute.
    pub fn new(duration: u64) -> Self {
        Self {
            leases: HashMap::new(),
            duration: duration.max(1),
        }
    }

    /// Who owns an entity right now, as of `tick`.
    ///
    /// An expired lease is nobody's, so the answer is the server.
    pub fn owner(&self, entity: Entity, tick: u64) -> PeerId {
        match self.leases.get(&entity) {
            Some(lease) if lease.expires > tick => lease.owner,
            _ => PeerId::SERVER,
        }
    }

    /// The lease itself, expired or not.
    pub fn lease(&self, entity: Entity) -> Option<Lease> {
        self.leases.get(&entity).copied()
    }

    /// Whether a peer may change an entity.
    ///
    /// The server may always; a client may only what it holds.
    pub fn may_change(&self, peer: PeerId, entity: Entity, tick: u64) -> bool {
        peer.is_server() || self.owner(entity, tick) == peer
    }

    /// Ask for ownership.
    ///
    /// Granted when the entity is free, when its lease has lapsed, or when
    /// the asker already holds it. A live lease held by somebody else is not
    /// taken away — except when both claims are for the same tick, where the
    /// lower peer id wins, so that two machines resolving the same pair of
    /// claims reach the same answer whichever packet arrived first.
    pub fn claim(&mut self, entity: Entity, peer: PeerId, tick: u64) -> bool {
        let granted = match self.leases.get(&entity) {
            None => true,
            Some(lease) if lease.expires <= tick => true,
            Some(lease) if lease.owner == peer => true,
            Some(lease) if lease.claimed == tick => peer < lease.owner,
            Some(_) => false,
        };
        if granted {
            self.leases.insert(
                entity,
                Lease {
                    owner: peer,
                    claimed: tick,
                    expires: tick + self.duration,
                },
            );
        }
        granted
    }

    /// Extend a lease the peer already holds. Returns false if it does not.
    pub fn renew(&mut self, entity: Entity, peer: PeerId, tick: u64) -> bool {
        match self.leases.get_mut(&entity) {
            Some(lease) if lease.owner == peer && lease.expires > tick => {
                lease.expires = tick + self.duration;
                true
            }
            _ => false,
        }
    }

    /// Give an entity up.
    pub fn release(&mut self, entity: Entity, peer: PeerId) -> bool {
        match self.leases.get(&entity) {
            Some(lease) if lease.owner == peer => {
                self.leases.remove(&entity);
                true
            }
            _ => false,
        }
    }

    /// Take everything a peer holds — for a client that has disconnected.
    pub fn revoke_all(&mut self, peer: PeerId) -> usize {
        let before = self.leases.len();
        self.leases.retain(|_, lease| lease.owner != peer);
        before - self.leases.len()
    }

    /// Drop leases that lapsed before `tick`, so the map does not grow.
    pub fn expire(&mut self, tick: u64) -> usize {
        let before = self.leases.len();
        self.leases.retain(|_, lease| lease.expires > tick);
        before - self.leases.len()
    }

    /// Everything a peer currently holds.
    pub fn held_by(&self, peer: PeerId, tick: u64) -> Vec<Entity> {
        let mut held: Vec<Entity> = self
            .leases
            .iter()
            .filter(|(_, lease)| lease.owner == peer && lease.expires > tick)
            .map(|(entity, _)| *entity)
            .collect();
        // Sorted, because a HashMap's order is not the same twice and anything
        // derived from this list would inherit that.
        held.sort();
        held
    }

    /// How many leases exist, live or lapsed.
    pub fn len(&self) -> usize {
        self.leases.len()
    }

    /// Whether nothing is claimed.
    pub fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }
}

/// Something a peer asked to have happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command<T> {
    /// Who asked.
    pub issuer: PeerId,
    /// Their own counter, so a resent command is recognised.
    pub sequence: u32,
    /// The tick they issued it on.
    pub tick: u64,
    /// What they asked for.
    pub payload: T,
}

impl<T: Serialize> Serialize for Command<T> {
    fn serialize(&self, writer: &mut Writer) {
        writer
            .write(&self.issuer)
            .write(&self.sequence)
            .write(&self.tick)
            .write(&self.payload);
    }
}

impl<T: Deserialize> Deserialize for Command<T> {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        Ok(Command {
            issuer: reader.read()?,
            sequence: reader.read()?,
            tick: reader.read()?,
            payload: reader.read()?,
        })
    }
}

/// Commands waiting to be applied, in an order both sides agree on.
///
/// Arrival order is not that order: packets from two clients race, and the
/// race is decided differently on every machine. Sorting by tick, then peer,
/// then sequence gives one answer everywhere — which is the difference
/// between a shared world and two diverging ones.
#[derive(Debug)]
pub struct CommandQueue<T> {
    pending: Vec<Command<T>>,
    /// The newest sequence accepted from each peer, for rejecting repeats.
    seen: HashMap<PeerId, u32>,
    /// How far ahead of the current tick a command may claim to be.
    pub future_tolerance: u64,
    accepted: u64,
    rejected: u64,
}

impl<T> Default for CommandQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> CommandQueue<T> {
    /// An empty queue.
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            seen: HashMap::new(),
            future_tolerance: 30,
            accepted: 0,
            rejected: 0,
        }
    }

    /// Take a command, unless it is a repeat or claims an impossible tick.
    ///
    /// A command from the future is refused rather than trusted: its tick is
    /// what decides the order everything happens in, so a client that claims
    /// to be a thousand ticks ahead would otherwise reorder the world around
    /// itself.
    ///
    /// A peer's own commands are assumed to arrive in order, which is what the
    /// reliable channel they travel on guarantees; a sequence that goes
    /// backwards is therefore a resend, not a late arrival.
    pub fn accept(&mut self, command: Command<T>, now: u64) -> bool {
        if command.tick > now + self.future_tolerance {
            self.rejected += 1;
            return false;
        }
        let newest = self.seen.get(&command.issuer).copied();
        if let Some(newest) = newest {
            // Sequence numbers only go forward; a repeat is a resend the
            // network layer already delivered once.
            if command.sequence <= newest {
                self.rejected += 1;
                return false;
            }
        }
        self.seen.insert(command.issuer, command.sequence);
        self.pending.push(command);
        self.accepted += 1;
        true
    }

    /// How many commands are waiting.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// How many commands have been accepted and refused.
    pub fn counts(&self) -> (u64, u64) {
        (self.accepted, self.rejected)
    }

    /// Take everything due by `tick`, in the order it must be applied.
    pub fn drain(&mut self, tick: u64) -> Vec<Command<T>> {
        // Partition rather than filter, so what is not due stays queued.
        let mut due = Vec::new();
        let mut later = Vec::new();
        for command in self.pending.drain(..) {
            if command.tick <= tick {
                due.push(command);
            } else {
                later.push(command);
            }
        }
        self.pending = later;
        due.sort_by(|a, b| {
            a.tick
                .cmp(&b.tick)
                .then(a.issuer.cmp(&b.issuer))
                .then(a.sequence.cmp(&b.sequence))
        });
        due
    }

    /// Forget a peer that has gone, so its sequence numbers do not block a
    /// later connection reusing the id.
    pub fn forget(&mut self, peer: PeerId) {
        self.seen.remove(&peer);
        self.pending.retain(|command| command.issuer != peer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use runity_serialize::{from_bytes, to_bytes};

    const ALICE: PeerId = PeerId(1);
    const BOB: PeerId = PeerId(2);

    fn entity() -> Entity {
        World::new().spawn()
    }

    #[test]
    fn unclaimed_things_belong_to_the_server() {
        let authority = Authority::new(100);
        let tree = entity();
        assert_eq!(authority.owner(tree, 0), PeerId::SERVER);
        assert!(authority.may_change(PeerId::SERVER, tree, 0));
        assert!(!authority.may_change(ALICE, tree, 0));
        assert!(authority.is_empty());
    }

    #[test]
    fn a_claim_grants_ownership_and_the_server_keeps_its_own() {
        let mut authority = Authority::new(100);
        let tree = entity();

        assert!(authority.claim(tree, ALICE, 10));
        assert_eq!(authority.owner(tree, 10), ALICE);
        assert!(authority.may_change(ALICE, tree, 10));
        assert!(!authority.may_change(BOB, tree, 10));
        // The server is never locked out of its own world.
        assert!(authority.may_change(PeerId::SERVER, tree, 10));
    }

    #[test]
    fn a_second_claim_on_a_held_thing_is_refused() {
        let mut authority = Authority::new(100);
        let tree = entity();
        assert!(authority.claim(tree, ALICE, 10));
        assert!(!authority.claim(tree, BOB, 11), "Alice is still holding it");
        assert_eq!(authority.owner(tree, 11), ALICE);

        // Alice renewing her own claim is fine.
        assert!(authority.claim(tree, ALICE, 12));
        assert!(authority.renew(tree, ALICE, 13));
        assert!(
            !authority.renew(tree, BOB, 13),
            "and Bob cannot renew what he does not hold"
        );
    }

    #[test]
    fn a_tie_is_broken_the_same_way_on_every_machine() {
        // Two clients claim the same tree on the same tick. Arrival order
        // differs between machines; the answer must not.
        let settle = |first: PeerId, second: PeerId| {
            let mut authority = Authority::new(100);
            let tree = entity();
            authority.claim(tree, first, 50);
            authority.claim(tree, second, 50);
            authority.owner(tree, 50)
        };
        assert_eq!(settle(ALICE, BOB), ALICE);
        assert_eq!(
            settle(BOB, ALICE),
            ALICE,
            "the lower peer id wins, whoever asked first"
        );
    }

    #[test]
    fn a_lapsed_lease_falls_back_to_the_server() {
        // The case that matters: a client crashes mid-swing. The tree must
        // not stay locked to a machine that is never coming back.
        let mut authority = Authority::new(10);
        let tree = entity();
        authority.claim(tree, ALICE, 0);

        assert_eq!(authority.owner(tree, 9), ALICE);
        assert_eq!(
            authority.owner(tree, 10),
            PeerId::SERVER,
            "the lease has run out"
        );
        assert!(!authority.may_change(ALICE, tree, 10));
        assert!(
            authority.claim(tree, BOB, 10),
            "and somebody else may take it up"
        );
    }

    #[test]
    fn a_lease_can_be_given_up_and_taken_away() {
        let mut authority = Authority::new(100);
        let tree = entity();
        authority.claim(tree, ALICE, 0);

        assert!(!authority.release(tree, BOB), "not his to release");
        assert!(authority.release(tree, ALICE));
        assert_eq!(authority.owner(tree, 0), PeerId::SERVER);

        // And everything at once, for a client that has disconnected.
        let mut world = World::new();
        let held: Vec<Entity> = (0..5).map(|_| world.spawn()).collect();
        for entity in &held {
            authority.claim(*entity, BOB, 0);
        }
        assert_eq!(authority.held_by(BOB, 0).len(), 5);
        assert_eq!(authority.revoke_all(BOB), 5);
        assert!(authority.held_by(BOB, 0).is_empty());
    }

    #[test]
    fn expired_leases_are_swept_up() {
        let mut authority = Authority::new(5);
        let mut world = World::new();
        for _ in 0..10 {
            authority.claim(world.spawn(), ALICE, 0);
        }
        assert_eq!(authority.len(), 10);
        assert_eq!(authority.expire(3), 0, "nothing has lapsed yet");
        assert_eq!(authority.expire(20), 10);
        assert!(authority.is_empty());
    }

    #[test]
    fn what_a_peer_holds_comes_back_in_a_stable_order() {
        let mut authority = Authority::new(100);
        let mut world = World::new();
        let entities: Vec<Entity> = (0..20).map(|_| world.spawn()).collect();
        for entity in &entities {
            authority.claim(*entity, ALICE, 0);
        }
        let first = authority.held_by(ALICE, 0);
        for _ in 0..5 {
            assert_eq!(
                authority.held_by(ALICE, 0),
                first,
                "a HashMap's order is not an order"
            );
        }
        assert_eq!(first.len(), 20);
    }

    #[test]
    fn a_lease_survives_a_save() {
        let lease = Lease {
            owner: ALICE,
            claimed: 12,
            expires: 112,
        };
        assert_eq!(from_bytes::<Lease>(&to_bytes(&lease)).unwrap(), lease);
        assert_eq!(from_bytes::<PeerId>(&to_bytes(&BOB)).unwrap(), BOB);
    }

    // ------------------------------------------------------------ commands

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Chop(u32);

    runity_serialize::serializable!(Chop(0));

    fn command(issuer: PeerId, sequence: u32, tick: u64) -> Command<Chop> {
        Command {
            issuer,
            sequence,
            tick,
            payload: Chop(sequence),
        }
    }

    #[test]
    fn commands_come_out_in_an_order_both_machines_agree_on() {
        // The same commands, offered in two different arrival orders.
        let apply = |order: [(PeerId, u32, u64); 4]| {
            let mut queue = CommandQueue::new();
            for (issuer, sequence, tick) in order {
                queue.accept(command(issuer, sequence, tick), 100);
            }
            queue
                .drain(100)
                .into_iter()
                .map(|command| (command.issuer, command.sequence, command.tick))
                .collect::<Vec<_>>()
        };

        // One peer's own commands keep their order — the reliable channel
        // guarantees that — but the two peers race with each other.
        let first = apply([(ALICE, 1, 10), (BOB, 1, 10), (ALICE, 2, 11), (BOB, 2, 11)]);
        let second = apply([(BOB, 1, 10), (BOB, 2, 11), (ALICE, 1, 10), (ALICE, 2, 11)]);
        assert_eq!(first, second);
        // Tick first, then peer, then sequence.
        assert_eq!(
            first,
            vec![(ALICE, 1, 10), (BOB, 1, 10), (ALICE, 2, 11), (BOB, 2, 11)]
        );
    }

    #[test]
    fn a_resent_command_is_not_applied_twice() {
        let mut queue = CommandQueue::new();
        assert!(queue.accept(command(ALICE, 1, 5), 10));
        assert!(
            !queue.accept(command(ALICE, 1, 5), 10),
            "the same one again"
        );
        assert!(!queue.accept(command(ALICE, 0, 5), 10), "and an older one");
        assert!(queue.accept(command(ALICE, 2, 6), 10));
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.counts(), (2, 2));
    }

    #[test]
    fn a_command_from_the_future_is_refused() {
        // Its tick decides when everything happens, so a client claiming to
        // be a thousand ticks ahead would reorder the world around itself.
        let mut queue: CommandQueue<Chop> = CommandQueue::new();
        assert!(queue.accept(command(ALICE, 1, 100), 100));
        assert!(
            queue.accept(command(ALICE, 2, 120), 100),
            "a little ahead is normal"
        );
        assert!(!queue.accept(command(ALICE, 3, 5_000), 100));
        assert_eq!(queue.counts().1, 1);
    }

    #[test]
    fn commands_for_later_ticks_wait_their_turn() {
        let mut queue = CommandQueue::new();
        queue.accept(command(ALICE, 1, 10), 20);
        queue.accept(command(ALICE, 2, 30), 20);

        let due = queue.drain(20);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].tick, 10);
        assert_eq!(queue.len(), 1, "the later one is still waiting");

        let due = queue.drain(30);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].tick, 30);
        assert!(queue.is_empty());
    }

    #[test]
    fn forgetting_a_peer_clears_its_history_and_its_queue() {
        let mut queue = CommandQueue::new();
        queue.accept(command(ALICE, 5, 10), 20);
        queue.accept(command(BOB, 5, 10), 20);
        queue.forget(ALICE);

        assert_eq!(queue.len(), 1, "Alice's pending command went with her");
        // And a new connection reusing her id is not blocked by her old
        // sequence numbers.
        assert!(queue.accept(command(ALICE, 1, 11), 20));
    }

    #[test]
    fn a_command_survives_the_wire() {
        let command = command(BOB, 77, 1234);
        let bytes = to_bytes(&command);
        assert_eq!(from_bytes::<Command<Chop>>(&bytes).unwrap(), command);
    }

    #[test]
    fn authority_and_commands_work_together() {
        // The shape a server's tick actually takes: take commands, check who
        // owns what, apply the ones that are allowed.
        let mut world = World::new();
        let tree = world.spawn();
        let mut authority = Authority::new(50);
        let mut queue = CommandQueue::new();
        authority.claim(tree, ALICE, 0);

        queue.accept(command(ALICE, 1, 1), 1);
        queue.accept(command(BOB, 1, 1), 1);

        let mut applied = Vec::new();
        for command in queue.drain(1) {
            if authority.may_change(command.issuer, tree, 1) {
                applied.push(command.issuer);
            }
        }
        assert_eq!(
            applied,
            vec![ALICE],
            "Bob's command is refused, not silently applied"
        );
    }
}
