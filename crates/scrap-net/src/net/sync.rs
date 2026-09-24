//! The world's end of the link: the only thing that turns messages into
//! entities and entities into messages.
//!
//! The dacha simulator's netsync — gather, apply, claim — over a hecs
//! world:
//!
//! * **Marking** keeps [`Owned`] and [`Replica`] in step with [`Owner`],
//!   so a system asks one cheap question.
//! * **Claims** are taken the frame they are asked for: [`Owner`] is
//!   this peer's at once, with [`OwnershipPending`] beside it, and the
//!   request goes out. The server's ruling settles it; a ruling naming
//!   someone else takes it back, and the entity glides to where its real
//!   owner has it.
//! * **Gathering** sends what this peer owns — only what changed since it
//!   was last sent, compared as bytes, so a resting crate costs nothing;
//!   and once it has been still a moment, its last word goes reliably,
//!   because a single lost snapshot would otherwise strand the others for
//!   good. Spawns go out whole. Despawns are *found*: an entity of ours the
//!   world no longer has is one somebody removed.
//! * **Applying** takes from the others only what their owner says, only
//!   newer than what was last taken from that owner, and never about
//!   anything this peer owns.
//! * **Presenting** shows a replica a couple of network ticks in the past,
//!   between the poses that really arrived ([`Presented`]), and bridges a
//!   change of owner rather than restarting.

use std::collections::{HashMap, HashSet, VecDeque};
use web_time::Instant;

use glam::{Quat, Vec3};

use super::protocol::{self, Blob, BlobId, Entry, Record, ToClient, ToServer, TRANSFORM};
use super::{
    addressable, despawn_tree, owner_of, DespawnWithOwner, NetId, NetPrefab, NetTick, Owned, Owner,
    OwnershipPending, PeerId, Replica, RequestOwnership,
};
use crate::components::Components;
use crate::id::EntityId;
use crate::scene::Transform;
use crate::world::SceneId;

/// Snapshots a second: a network tick per fixed step at the default 30 Hz
/// clock.
pub const NET_HZ: f32 = 30.0;

/// One of ours that changed, as it may go this tick.
struct Change {
    id: EntityId,
    /// What goes: all of it (`whole`), or what changed lately.
    entry: Entry,
    whole: bool,
    /// Something it had is gone: it goes in a whole snapshot.
    removed: bool,
    /// All of it, and as bytes.
    blobs: Vec<Blob>,
    bytes: Vec<u8>,
}

/// Unchanged network ticks before an entity's last word is sent. Soon:
/// for what changes a few times a second (a `Rough` summary) the last word
/// is the second copy that carries it over a lost datagram — said later,
/// or said at once reliably, it came late on a lossy link, and a cape
/// drifted a third further from its owner's (docs/netsim.md, «Трафик»).
pub const SETTLE_TICKS: u32 = 2;

/// About how many bytes of entries go in one snapshot message.
const ENTRY_BUDGET: usize = 1000;

/// What became of every snapshot entry that came in — the dacha
/// simulator's `SnapshotTally`, one count per reason, because each is a
/// different bug.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    /// Reached the world.
    pub accepted: u64,
    /// Named something this peer does not have.
    pub unknown: u64,
    /// About something this peer owns: a grant crossing a snapshot.
    pub locally_owned: u64,
    /// From someone who no longer owns it: a handover crossing a snapshot.
    pub foreign_sender: u64,
    /// Older than what was taken from that owner already.
    pub stale: u64,
}

/// What this peer last sent about one of its entities.
struct Baseline {
    /// All of it, as staged.
    bytes: Vec<u8>,
    quiet: u32,
    settled: bool,
    /// Each blob as last sent, and the network tick it last changed.
    blobs: HashMap<BlobId, (Vec<u8>, u64)>,
    /// The network tick it last went whole.
    whole_at: u64,
}

impl Baseline {
    /// `blobs` sent at `tick` over what was sent before (`was`).
    fn after(was: Option<Baseline>, blobs: &[Blob], bytes: Vec<u8>, tick: u64, whole: bool) -> Self {
        let (mut map, whole_at) = was.map_or((HashMap::new(), tick), |b| (b.blobs, b.whole_at));
        map.retain(|n, _| blobs.iter().any(|(m, _)| m == n));
        for (n, b) in blobs {
            if map.get(n).is_none_or(|(old, _)| old != b) {
                map.insert(*n, (b.clone(), tick));
            }
        }
        Self { bytes, quiet: 0, settled: false, blobs: map, whole_at: if whole { tick } else { whole_at } }
    }
}

/// Network ticks between an entity's whole entries while it changes: a
/// blob whose change was lost comes with the next (docs/netsim.md,
/// «Трафик»). Each entity's second falls on its own tick.
pub const REFRESH: u64 = 30;
/// Network ticks a changed blob goes on being sent after it stops
/// changing: two lost datagrams in a row do not lose it.
pub const REPEAT: u64 = 3;

/// The sync state of one client.
pub struct Sync {
    pub me: PeerId,
    pub epoch: u32,
    /// Ticks this peer has sent; the clock its snapshots are stamped with.
    pub tick: u64,
    baselines: HashMap<EntityId, Baseline>,
    /// Spawned ids of ours the server has been told about.
    announced: HashSet<EntityId>,
    /// Newest tick taken per entity, and from whom.
    gates: HashMap<EntityId, (PeerId, u64)>,
    /// While a world state is coming in: what it named.
    welcoming: Option<HashSet<EntityId>>,
    pub tally: Tally,
    /// Half the round trip, in network ticks: how far behind the newest
    /// pose from someone else is by the time it is here. What a takeover
    /// carries it forward by.
    pub one_way_ticks: f64,
    /// Bytes of changes a network tick this peer sends at most
    /// ([`BUDGET`] unless the game says).
    pub budget: usize,
    /// How long each changed thing not yet sent has waited, ticks.
    waiting: HashMap<EntityId, f32>,
    /// The server's short numbers for entities ([`ToClient::Shorts`]),
    /// both ways.
    shorts: HashMap<EntityId, u32>,
    by_short: HashMap<u32, EntityId>,
    /// Bytes of changes sent, all told.
    pub sent_bytes: u64,
}

/// Bytes of changes a network tick a peer sends at most: 30 KB a second
/// at the default 30 ticks (the author's decision of 2026-09-24).
pub const BUDGET: usize = 1000;

/// What a network tick's snapshot costs beyond its entries, bytes: the
/// message's head (epoch, tick, counts) and the datagram's — charged to
/// the budget first, so what goes on the wire keeps within it.
const OVERHEAD: usize = 40;

/// Something the world end noticed that the game may want to know.
#[derive(Debug, Clone, PartialEq)]
pub enum Noticed {
    /// The whole world state is in.
    Welcomed,
    /// A claim was ruled against: someone else drives it.
    ClaimLost(EntityId, PeerId),
    /// What could not be done, in words: a prefab this peer lacks.
    Problem(String),
}

impl Sync {
    pub fn new(me: PeerId, epoch: u32) -> Self {
        Self {
            me,
            epoch,
            tick: 0,
            baselines: HashMap::new(),
            announced: HashSet::new(),
            gates: HashMap::new(),
            welcoming: None,
            tally: Tally::default(),
            one_way_ticks: 0.0,
            budget: BUDGET,
            waiting: HashMap::new(),
            sent_bytes: 0,
            shorts: HashMap::new(),
            by_short: HashMap::new(),
        }
    }

    /// An entry of ours, named short when the server has given a number.
    fn entry(&self, id: EntityId, blobs: Vec<Blob>) -> Entry {
        let entry = Entry::new(id, blobs);
        match self.shorts.get(&id) {
            Some(&short) => entry.shortened(short),
            None => entry,
        }
    }

    /// Keep [`Owned`] and [`Replica`] in step with who owns what.
    pub fn mark(&self, world: &mut hecs::World) {
        let one_way = self.one_way_ticks;
        let mut own = Vec::new();
        let mut replicate = Vec::new();
        for (entity, owned, replica) in world
            .query::<(hecs::Entity, Option<&Owned>, Option<&Replica>)>()
            .with::<&Transform>()
            .iter()
        {
            if world.get::<&SceneId>(entity).is_err() && world.get::<&NetId>(entity).is_err() {
                continue;
            }
            if world.get::<&scrap_core::netsim::Unshared>(entity).is_ok() {
                continue;
            }
            let mine = owner_of(world, entity) == self.me;
            if mine && (owned.is_none() || replica.is_some()) {
                own.push(entity);
            } else if !mine && (owned.is_some() || replica.is_none()) {
                replicate.push(entity);
            }
        }
        for entity in own {
            let _ = world.remove_one::<Replica>(entity);
            // Taken over: from where the old owner has it now, not from
            // the picture of a moment ago — or everything handed over in
            // motion steps back by the delay.
            if let Ok(presented) = world.remove_one::<Presented>(entity) {
                if let Some((pose, velocity, spin)) = presented.takeover(one_way) {
                    if let Ok(mut transform) = world.get::<&mut Transform>(entity) {
                        *transform = pose;
                    }
                    let _ = world.insert_one(entity, super::Takeover { velocity, spin });
                }
            }
            let _ = world.insert_one(entity, Owned);
        }
        for entity in replicate {
            let _ = world.remove_one::<Owned>(entity);
            let _ = world.remove_one::<OwnershipPending>(entity);
            let _ = world.insert_one(entity, Replica);
        }
    }

    /// Take what the game asked to drive: this peer's at once, pending,
    /// and asked for.
    pub fn claims(&mut self, world: &mut hecs::World, out: &mut Vec<ToServer>) {
        let mut asked: Vec<hecs::Entity> = world
            .query::<(hecs::Entity, &RequestOwnership)>()
            .iter()
            .map(|(e, _)| e)
            .collect();
        if !asked.is_empty() {
            asked = Self::together(world, asked);
        }
        for entity in asked {
            let _ = world.remove_one::<RequestOwnership>(entity);
            let Some(id) = super::network_id(world, entity) else {
                continue;
            };
            if owner_of(world, entity) == self.me && world.get::<&OwnershipPending>(entity).is_err()
            {
                continue;
            }
            let _ = world.insert(entity, (Owner(self.me), OwnershipPending));
            // Sent whole as soon as the grant makes it ours.
            self.baselines.remove(&id);
            out.push(ToServer::OwnershipRequest {
                epoch: self.epoch,
                id,
            });
        }
    }

    /// What has to be driven by one peer with these: the rest of their
    /// groups, and whatever a joint holds to them, both halves and on
    /// along a chain — two machines solving one rope pull it apart.
    fn together(world: &hecs::World, asked: Vec<hecs::Entity>) -> Vec<hecs::Entity> {
        use std::collections::{HashMap, HashSet};
        let mut by_id: HashMap<EntityId, hecs::Entity> = HashMap::new();
        let mut links: HashMap<hecs::Entity, Vec<hecs::Entity>> = HashMap::new();
        for (entity, id) in world.query::<(hecs::Entity, &SceneId)>().iter() {
            by_id.insert(id.0, entity);
        }
        for (entity, joint) in world
            .query::<(hecs::Entity, &crate::world::Jointed)>()
            .iter()
        {
            if let Some(other) = joint.0.to().and_then(|to| by_id.get(&to)) {
                links.entry(entity).or_default().push(*other);
                links.entry(*other).or_default().push(entity);
            }
        }
        // What a simulation ties together: a rope and its load.
        let spawned = addressable(world);
        for (entity, tied) in world.query::<(hecs::Entity, &scrap_core::netsim::Tied)>().iter() {
            for id in &tied.0 {
                if let Some(other) = by_id.get(id).or_else(|| spawned.get(id)) {
                    links.entry(entity).or_default().push(*other);
                    links.entry(*other).or_default().push(entity);
                }
            }
        }
        // A player's body is its player's: nothing gathers it along — a
        // rope two players hold does not drag one of them to the other's
        // machine. What it ties to itself (its ragdoll's parts) goes with
        // it; what it is merely tied to by something else does not.
        let pawn = |e: hecs::Entity| world.get::<&scrap_core::netsim::Pawn>(e).is_ok();
        let own: HashMap<hecs::Entity, Vec<hecs::Entity>> = world
            .query::<(hecs::Entity, &scrap_core::netsim::Tied)>()
            .with::<&scrap_core::netsim::Pawn>()
            .iter()
            .map(|(e, tied)| (e, tied.0.iter().filter_map(|id| by_id.get(id).or_else(|| spawned.get(id)).copied()).collect()))
            .collect();
        for (entity, near) in links.iter_mut() {
            near.retain(|e| !pawn(*e));
            if pawn(*entity) {
                let mine = own.get(entity).cloned().unwrap_or_default();
                near.retain(|e| mine.contains(e));
            }
        }
        let mut groups: HashMap<super::NetGroup, Vec<hecs::Entity>> = HashMap::new();
        for (entity, group) in world.query::<(hecs::Entity, &super::NetGroup)>().iter() {
            groups.entry(*group).or_default().push(entity);
        }
        let mut seen: HashSet<hecs::Entity> = asked.iter().copied().collect();
        let mut out = asked.clone();
        let mut next = asked;
        while let Some(entity) = next.pop() {
            let mut near: Vec<hecs::Entity> = links.get(&entity).cloned().unwrap_or_default();
            if let Ok(group) = world.get::<&super::NetGroup>(entity) {
                near.extend(groups.get(&*group).into_iter().flatten().copied());
            }
            for other in near {
                if seen.insert(other) {
                    out.push(other);
                    next.push(other);
                }
            }
        }
        out
    }

    /// What this peer owns, as blobs.
    fn stage(world: &hecs::World, components: &Components, entity: hecs::Entity) -> Vec<Blob> {
        let transform = world
            .get::<&Transform>(entity)
            .map(|t| *t)
            .unwrap_or_default();
        let mut blobs = vec![(TRANSFORM, protocol::encode_transform(&transform))];
        for (name, text) in components.write_networked(world, entity) {
            if let Some(id) = components.networked_id(&name) {
                blobs.push((id, text.into_bytes()));
            }
        }
        for (name, bytes) in components.gather_states(world, entity) {
            if let Some(id) = components.networked_id(&name) {
                blobs.push((id, bytes));
            }
        }
        blobs
    }

    /// Everything this peer has to say this network tick: spawns, then
    /// snapshots, then despawns — a thing replaced by another is shown
    /// twice for a moment rather than not at all. `reliable` and
    /// `unreliable` are the two planes.
    pub fn gather(
        &mut self,
        world: &hecs::World,
        components: &Components,
        reliable: &mut Vec<ToServer>,
        unreliable: &mut Vec<ToServer>,
    ) {
        self.tick += 1;
        let here = addressable(world);
        let mut changed = Vec::new();
        let mut settling = Vec::new();
        let mut ids: Vec<(&EntityId, &hecs::Entity)> = here.iter().collect();
        ids.sort();
        for (&id, &entity) in ids {
            if owner_of(world, entity) != self.me {
                continue;
            }
            let blobs = Self::stage(world, components, entity);
            let bytes = postcard::to_stdvec(&blobs).unwrap_or_default();
            // Spawned by us and not yet told: announced whole.
            if let Ok(prefab) = world.get::<&NetPrefab>(entity) {
                if !self.announced.contains(&id) {
                    reliable.push(ToServer::Spawn {
                        epoch: self.epoch,
                        id,
                        prefab: prefab.0.clone(),
                        despawn_with_owner: world.get::<&DespawnWithOwner>(entity).is_ok(),
                        blobs: blobs.clone(),
                    });
                    self.announced.insert(id);
                    let base = Baseline::after(None, &blobs, bytes, self.tick, true);
                    self.baselines.insert(id, base);
                    continue;
                }
            }
            let tick = self.tick;
            let said_reliably = match self.baselines.get_mut(&id) {
                Some(base) if base.bytes == bytes => {
                    base.quiet += 1;
                    let settles = base.quiet >= SETTLE_TICKS && !base.settled;
                    base.settled |= settles;
                    if settles {
                        base.whole_at = tick;
                    }
                    Some(settles)
                }
                _ => None,
            };
            match said_reliably {
                Some(true) => settling.push(self.entry(id, blobs)),
                Some(false) => {}
                None => {
                    // Only what changed lately, unless its second is up or
                    // nothing was sent before; and a blob gone is told by a
                    // whole entry (the server takes removals only from one).
                    let (sent, removed) = match self.baselines.get(&id) {
                        Some(base) => {
                            let removed = base.blobs.keys().any(|n| !blobs.iter().any(|(m, _)| m == n));
                            if removed || tick >= base.whole_at + REFRESH {
                                (blobs.clone(), removed)
                            } else {
                                let lately = |(n, b): &&Blob| base.blobs.get(n).is_none_or(|(old, at)| old != b || tick < at + REPEAT);
                                (blobs.iter().filter(lately).cloned().collect(), false)
                            }
                        }
                        None => (blobs.clone(), false),
                    };
                    let whole = sent.len() == blobs.len();
                    changed.push(Change { id, entry: self.entry(id, sent), whole, removed, blobs, bytes });
                }
            }
        }
        // What changed goes by how long it has waited, within the budget
        // (docs/netsim.md, bad links: Fiedler's priority accumulator):
        // each thing waiting gains a point a tick, the most waited go
        // first, and what does not fit waits — still changed, so it goes
        // next time with more points. Nothing waits for ever.
        for change in &changed {
            *self.waiting.entry(change.id).or_insert(0.0) += 1.0;
        }
        changed.sort_by(|a, b| {
            let (pa, pb) = (self.waiting.get(&a.id).copied().unwrap_or(0.0), self.waiting.get(&b.id).copied().unwrap_or(0.0));
            pb.total_cmp(&pa).then(a.id.cmp(&b.id))
        });
        let mut spent = OVERHEAD;
        // One snapshot a tick: partial if any entry leaves something out —
        // a whole entry in it lacks nothing, so it removes nothing either
        // — and a whole one for what lost a blob.
        let (mut removing, mut rest, mut all_whole) = (Vec::new(), Vec::new(), true);
        for Change { id, entry, whole: is_whole, removed, blobs, bytes } in changed {
            let size = entry.blobs.iter().map(|(_, b)| b.len() + 3).sum::<usize>() + 12;
            if spent > OVERHEAD && spent + size > self.budget {
                continue;
            }
            spent += size;
            self.waiting.remove(&id);
            let was = self.baselines.remove(&id);
            self.baselines.insert(id, Baseline::after(was, &blobs, bytes, self.tick, is_whole));
            if removed {
                removing.push(entry);
            } else {
                all_whole &= is_whole;
                rest.push(entry);
            }
        }
        if !removing.is_empty() || !rest.is_empty() {
            self.sent_bytes += spent as u64;
        }
        for (entries, is_partial) in [(removing, false), (rest, !all_whole)] {
            for entries in chunks(entries) {
                unreliable.push(ToServer::Snapshot {
                    epoch: self.epoch,
                    tick: self.tick,
                    settle: false,
                    partial: is_partial,
                    entries,
                });
            }
        }
        for entries in chunks(settling) {
            reliable.push(ToServer::Snapshot {
                epoch: self.epoch,
                tick: self.tick,
                settle: true,
                partial: false,
                entries,
            });
        }
        // Found, not reported: ours, announced, and no longer in the world.
        let gone: Vec<EntityId> = self
            .announced
            .iter()
            .filter(|id| !here.contains_key(id))
            .copied()
            .collect();
        for id in gone {
            self.announced.remove(&id);
            self.baselines.remove(&id);
            reliable.push(ToServer::Despawn {
                epoch: self.epoch,
                id,
            });
        }
    }

    /// Everything of ours as it stands is what the others already have:
    /// the scene they loaded. Nothing is sent until it changes.
    pub fn seed(&mut self, world: &hecs::World, components: &Components) {
        for (id, entity) in addressable(world) {
            if owner_of(world, entity) != self.me || world.get::<&NetPrefab>(entity).is_ok() {
                continue;
            }
            let blobs = Self::stage(world, components, entity);
            let bytes = postcard::to_stdvec(&blobs).unwrap_or_default();
            let mut base = Baseline::after(None, &blobs, bytes, self.tick, true);
            base.quiet = SETTLE_TICKS;
            base.settled = true;
            // Each entity's second of whole entries falls on its own tick.
            base.whole_at = self.tick.saturating_sub(id.raw() % REFRESH);
            self.baselines.insert(id, base);
        }
    }

    /// Take one message from the server into the world. `spawn` puts a
    /// prefab into it — [`crate::LiveScene::spawn_prefab`], usually.
    pub fn apply(
        &mut self,
        world: &mut hecs::World,
        components: &Components,
        message: ToClient,
        spawn: &mut dyn FnMut(&mut hecs::World, &str, Transform) -> Option<hecs::Entity>,
    ) -> Vec<Noticed> {
        let mut noticed = Vec::new();
        match message {
            ToClient::Spawn {
                owner,
                id,
                prefab,
                blobs,
            } => {
                if let Some(problem) =
                    self.spawn_one(world, components, id, owner, &prefab, &blobs, spawn)
                {
                    noticed.push(Noticed::Problem(problem));
                }
            }
            ToClient::Despawn { id } => {
                if let Some(&entity) = addressable(world).get(&id) {
                    if owner_of(world, entity) != self.me {
                        despawn_tree(world, entity);
                    }
                }
            }
            ToClient::OwnershipChanged { id, owner } => {
                let Some(&entity) = addressable(world).get(&id) else {
                    return noticed;
                };
                let was_pending = world.get::<&OwnershipPending>(entity).is_ok();
                let _ = world.remove_one::<OwnershipPending>(entity);
                let _ = world.insert_one(entity, Owner(owner));
                // A new owner is a new clock.
                self.gates.remove(&id);
                if owner == self.me {
                    self.baselines.remove(&id);
                    if world.get::<&NetPrefab>(entity).is_ok() {
                        self.announced.insert(id);
                    }
                } else {
                    self.announced.remove(&id);
                    self.baselines.remove(&id);
                    // Where we last had it is where the picture starts, so
                    // the new owner's stream is joined, not jumped to.
                    if world.get::<&Presented>(entity).is_err() {
                        let here = world
                            .get::<&Transform>(entity)
                            .map(|t| *t)
                            .unwrap_or_default();
                        let mut presented = Presented::default();
                        presented.push(self.me, self.tick.saturating_sub(1), here);
                        presented.push(self.me, self.tick, here);
                        presented.render = self.tick as f64;
                        let _ = world.insert_one(entity, presented);
                    }
                    if was_pending {
                        noticed.push(Noticed::ClaimLost(id, owner));
                    }
                }
                self.mark(world);
            }
            ToClient::ComponentsRemoved { id, tick, names } => {
                let Some(&entity) = addressable(world).get(&id) else {
                    return noticed;
                };
                if owner_of(world, entity) == self.me {
                    return noticed;
                }
                let owner = owner_of(world, entity);
                let gate = self.gates.entry(id).or_insert((owner, 0));
                gate.1 = gate.1.max(tick);
                for name in names.into_iter().filter_map(|id| components.networked_name(id)) {
                    components.remove_by_name(name, world, entity);
                }
            }
            ToClient::Shorts { pairs } => {
                for (id, short) in pairs {
                    self.shorts.insert(id, short);
                    self.by_short.insert(short, id);
                }
            }
            ToClient::Snapshot {
                owner,
                tick,
                settle,
                entries,
                ..
            } => {
                for entry in entries {
                    self.take_entry(world, components, owner, tick, settle, entry);
                }
            }
            ToClient::WorldState { records, last } => {
                let named = self.welcoming.get_or_insert_with(HashSet::new);
                named.extend(records.iter().map(|r| r.id));
                for record in records {
                    if let Some(problem) = self.take_record(world, components, record, spawn) {
                        noticed.push(Noticed::Problem(problem));
                    }
                }
                if last {
                    let named = self.welcoming.take().unwrap_or_default();
                    // Spawned things of someone else's the server does not
                    // have are left over from a session that is gone.
                    let stale: Vec<hecs::Entity> = world
                        .query::<(hecs::Entity, &NetId)>()
                        .iter()
                        .filter(|(e, id)| !named.contains(&id.0) && owner_of(world, *e) != self.me)
                        .map(|(e, _)| e)
                        .collect();
                    for entity in stale {
                        despawn_tree(world, entity);
                    }
                    self.mark(world);
                    self.seed(world, components);
                    noticed.push(Noticed::Welcomed);
                }
            }
            _ => {}
        }
        noticed
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_one(
        &mut self,
        world: &mut hecs::World,
        components: &Components,
        id: EntityId,
        owner: PeerId,
        prefab: &str,
        blobs: &[Blob],
        spawn: &mut dyn FnMut(&mut hecs::World, &str, Transform) -> Option<hecs::Entity>,
    ) -> Option<String> {
        if addressable(world).contains_key(&id) {
            return None;
        }
        let transform = transform_of(blobs).unwrap_or_default();
        let Some(entity) = spawn(world, prefab, transform) else {
            return Some(format!("{id}: no prefab `{prefab}` to spawn here"));
        };
        let _ = world.insert(
            entity,
            (NetId(id), NetPrefab(prefab.to_string()), Owner(owner)),
        );
        let _ = world.insert_one(entity, transform);
        write_components(world, components, entity, blobs, owner, 0);
        if owner == self.me {
            self.announced.insert(id);
        } else {
            let _ = world.insert_one(entity, Replica);
        }
        None
    }

    fn take_record(
        &mut self,
        world: &mut hecs::World,
        components: &Components,
        record: Record,
        spawn: &mut dyn FnMut(&mut hecs::World, &str, Transform) -> Option<hecs::Entity>,
    ) -> Option<String> {
        let here = addressable(world);
        if record.gone {
            if let Some(&entity) = here.get(&record.id) {
                despawn_tree(world, entity);
            }
            return None;
        }
        match here.get(&record.id) {
            None => {
                // A scene entity this peer does not have is one it has not
                // loaded; only a spawned one can be made from the record.
                let prefab = record.prefab.as_ref()?;
                return self.spawn_one(
                    world,
                    components,
                    record.id,
                    record.owner,
                    prefab,
                    &record.blobs,
                    spawn,
                );
            }
            Some(&entity) => {
                let _ = world.insert_one(entity, Owner(record.owner));
                if record.owner != self.me {
                    if let Some(transform) = transform_of(&record.blobs) {
                        let _ = world.insert_one(entity, transform);
                        let _ = world.remove_one::<Presented>(entity);
                    }
                    write_components(world, components, entity, &record.blobs, record.owner, 0);
                }
            }
        }
        None
    }

    fn take_entry(
        &mut self,
        world: &mut hecs::World,
        components: &Components,
        sender: PeerId,
        tick: u64,
        settle: bool,
        mut entry: Entry,
    ) {
        if entry.short != 0 {
            // A number not yet heard of: its word is on the way, and the
            // next entry will do.
            let Some(&id) = self.by_short.get(&entry.short) else {
                self.tally.unknown += 1;
                return;
            };
            entry.id = id;
        }
        let Some(&entity) = addressable(world).get(&entry.id) else {
            self.tally.unknown += 1;
            return;
        };
        let owner = owner_of(world, entity);
        if owner == self.me {
            self.tally.locally_owned += 1;
            return;
        }
        if owner != sender {
            self.tally.foreign_sender += 1;
            return;
        }
        match self.gates.get(&entry.id) {
            Some(&(from, newest)) if from == sender && newest >= tick => {
                self.tally.stale += 1;
                return;
            }
            _ => {}
        }
        self.gates.insert(entry.id, (sender, tick));
        self.tally.accepted += 1;
        if let Some(transform) = transform_of(&entry.blobs) {
            let buffered = world.get::<&Presented>(entity).is_ok();
            if buffered {
                if let Ok(mut presented) = world.get::<&mut Presented>(entity) {
                    presented.push(sender, tick, transform);
                }
            } else {
                let mut presented = Presented::default();
                presented.push(sender, tick, transform);
                let _ = world.insert_one(entity, presented);
            }
        }
        write_components(world, components, entity, &entry.blobs, sender, tick);
        let _ = world.insert_one(
            entity,
            NetTick {
                sender,
                tick,
                settled: settle,
                at: Instant::now(),
            },
        );
    }
}

fn transform_of(blobs: &[Blob]) -> Option<Transform> {
    blobs
        .iter()
        .find(|(name, _)| *name == TRANSFORM)
        .and_then(|(_, bytes)| protocol::decode_transform(bytes))
}

fn write_components(
    world: &mut hecs::World,
    components: &Components,
    entity: hecs::Entity,
    blobs: &[Blob],
    sender: PeerId,
    tick: u64,
) {
    for (id, bytes) in blobs {
        let Some(name) = components.networked_name(*id) else {
            continue;
        };
        if components.take_state(name, world, entity, sender.0, tick, bytes) {
            continue;
        }
        if !components.is_networked(name) {
            continue;
        }
        if let Ok(text) = std::str::from_utf8(bytes) {
            let _ = components.insert_text(name, text, world, entity);
        }
    }
}

/// Entries into groups of about [`ENTRY_BUDGET`] bytes.
fn chunks(entries: Vec<Entry>) -> Vec<Vec<Entry>> {
    let mut out: Vec<Vec<Entry>> = Vec::new();
    let mut size = 0;
    for entry in entries {
        let one = entry
            .blobs
            .iter()
            .map(|(_, b)| b.len() + 4)
            .sum::<usize>()
            + 12;
        if out.is_empty() || size + one > ENTRY_BUDGET {
            out.push(Vec::new());
            size = 0;
        }
        size += one;
        out.last_mut().expect("just pushed").push(entry);
    }
    out
}

/// How far behind a replica is shown, in network ticks: far enough that
/// the next pose is usually in hand already.
pub const DELAY: f64 = 2.0;
/// The most a replica is shown behind, however bad the link: past this
/// it is late enough to feel.
pub const MOST_DELAY: f64 = 6.0;

/// How far behind to show, network ticks, for a link whose samples come
/// `spread` ticks off their beat on the mean: two ticks, and twice the
/// spread more — so the next sample is nearly always in hand even when
/// the link stutters or drops one (docs/netsim.md, bad links).
pub fn delay_for(spread: f64) -> f64 {
    (DELAY + 2.0 * spread).clamp(DELAY, MOST_DELAY)
}
/// A pair of samples further apart than this in speed is a teleport, not
/// motion, and is jumped rather than slid through the air.
pub const SNAP_SPEED: f32 = 35.0;
/// Seconds over which a change of owner is bent into the new owner's
/// stream.
pub const HANDOVER_BLEND: f32 = 0.5;

/// The most change of speed a takeover carries forward, metres a second
/// a second: a little more than gravity.
pub const TAKEOVER_ACCEL: f32 = 12.0;

/// A replica's recent poses as its owners sent them, and the clock it is
/// shown by — the dacha simulator's interpolation buffer. The transform on
/// the entity is the *presented* pose, so the collider, the picking ray
/// and the pixels agree.
#[derive(Debug, Clone, Default)]
pub struct Presented {
    /// (tick on this buffer's timeline, pose), oldest first.
    samples: VecDeque<(f64, Transform)>,
    sender: Option<PeerId>,
    /// Added to the current sender's ticks to put them on the timeline.
    offset: f64,
    /// Where the clock stands, in timeline ticks.
    render: f64,
    /// When the first sample of this sender came, how late the earliest
    /// came, and how far off their beat samples come on the mean, ticks:
    /// what the delay is chosen by.
    origin: Option<Instant>,
    base: Option<f64>,
    spread: f64,
    /// A handover being hidden: the gap at the join, seconds since the
    /// picture reached it, the join's tick and the old stream's newest —
    /// between the two the gap grows in as the picture slides from one
    /// owner's pose to the other's, so it never jumps.
    blend: Option<(Vec3, Quat, f32, f64, f64)>,
}

impl Presented {
    /// An arrival, against the beat it was sent on: how late it came
    /// beyond the earliest any came (the link's own delay aside) is how
    /// far off the beat the link is.
    fn hear(&mut self, tick: f64) {
        let now = Instant::now();
        let origin = *self.origin.get_or_insert(now);
        let late = now.duration_since(origin).as_secs_f64() * NET_HZ as f64 - tick;
        // The earliest, let rise a little each time so a link that got
        // slower for good is not held against it for ever.
        let base = self.base.map_or(late, |b: f64| (b + 0.02).min(late));
        self.base = Some(base);
        self.spread += ((late - base).min(8.0) - self.spread) * 0.05;
    }

    /// How far behind it is shown now, network ticks.
    pub fn delay(&self) -> f64 {
        delay_for(self.spread)
    }

    /// A pose from `sender`, at its tick.
    pub fn push(&mut self, sender: PeerId, tick: u64, pose: Transform) {
        let tick = tick as f64;
        match self.sender {
            None => {
                self.offset = 0.0;
                self.render = tick - self.delay();
            }
            Some(old) if old != sender => {
                // A new clock: its beat is measured afresh.
                self.origin = None;
                self.base = None;
                // A new owner is a new clock: their first sample goes a
                // delay ahead of where the picture stands, and the gap
                // between where the old stream was heading and where the
                // new owner has it is hidden over a moment.
                let newest = self.samples.back().map_or(self.render, |(t, _)| *t);
                let join = (self.render + self.delay()).ceil().max(newest + 1.0);
                self.offset = join - tick;
                let heading = self.pose_at(join);
                self.blend = Some((
                    heading.position - pose.position,
                    heading.rotation() * pose.rotation().inverse(),
                    0.0,
                    join,
                    newest,
                ));
            }
            _ => {}
        }
        self.sender = Some(sender);
        self.hear(tick);
        let at = tick + self.offset;
        if let Some((newest, last)) = self.samples.back() {
            if at <= *newest {
                return;
            }
            let seconds = ((at - newest) / NET_HZ as f64) as f32;
            if last.position.distance(pose.position) / seconds.max(1e-3) > SNAP_SPEED {
                self.samples.clear();
                self.render = at - self.delay();
                self.blend = None;
            }
        }
        self.samples.push_back((at, pose));
        while self.samples.len() > 8 {
            self.samples.pop_front();
        }
    }

    /// Where the sender has it now, and how fast it goes: the newest pose,
    /// carried forward over the ticks since it was sent — `one_way` on the
    /// way here and one more — by its speed and how that was changing
    /// (gravity, mostly), from the last three. `None` with fewer than two.
    pub fn takeover(&self, one_way: f64) -> Option<(Transform, Vec3, Vec3)> {
        let n = self.samples.len();
        if n < 2 {
            return None;
        }
        let (newest_at, newest) = self.samples[n - 1];
        let (before_at, before) = self.samples[n - 2];
        let ticks = (newest_at - before_at).max(1e-6);
        let mut per_tick = (newest.position - before.position) / ticks as f32;
        // Speeding up or slowing down, per tick per tick, from a third.
        let mut accel = Vec3::ZERO;
        if n >= 3 {
            let (older_at, older) = self.samples[n - 3];
            let earlier_ticks = (before_at - older_at).max(1e-6);
            let earlier = (before.position - older.position) / earlier_ticks as f32;
            accel = (per_tick - earlier) / (0.5 * (ticks + earlier_ticks)) as f32;
            // Falling is carried on; a knock — landing, a hit — is not:
            // it is over by the time the next tick starts, and carrying it
            // forward throws the body.
            let most = TAKEOVER_ACCEL / (NET_HZ * NET_HZ);
            accel = accel.clamp_length_max(most);
            // The speed at the newest, not half a tick before it.
            per_tick += accel * (0.5 * ticks) as f32;
        }
        let velocity = per_tick * NET_HZ;
        let lead = ((self.render + self.delay() - newest_at).max(0.0) + 1.0 + one_way.max(0.0)) as f32;
        // The turn between the last two, the short way round.
        let mut turn = newest.rotation() * before.rotation().inverse();
        if turn.w < 0.0 {
            turn = -turn;
        }
        let (axis, angle) = turn.to_axis_angle();
        let spin_per_tick = axis * angle / ticks as f32;
        let mut pose = newest;
        pose.position += per_tick * lead + accel * (0.5 * lead * lead);
        let velocity = velocity + accel * NET_HZ * lead;
        let ahead = spin_per_tick * lead;
        if ahead.length() > 1e-6 {
            pose.set_rotation(Quat::from_scaled_axis(ahead) * newest.rotation());
        }
        Some((pose, velocity, spin_per_tick * NET_HZ))
    }

    /// The pose at a timeline tick, between the samples around it; held
    /// at the ends — never guessed past the newest.
    fn pose_at(&self, at: f64) -> Transform {
        let Some((first_at, first)) = self.samples.front() else {
            return Transform::default();
        };
        if at <= *first_at {
            return *first;
        }
        for pair in self.samples.iter().collect::<Vec<_>>().windows(2) {
            let ((a_at, a), (b_at, b)) = (pair[0], pair[1]);
            if at <= *b_at {
                let t = ((at - a_at) / (b_at - a_at).max(1e-6)) as f32;
                let mut out = Transform {
                    position: a.position.lerp(b.position, t),
                    scale: a.scale.lerp(b.scale, t),
                    ..*a
                };
                out.set_rotation(a.rotation().slerp(b.rotation(), t));
                return out;
            }
        }
        self.samples.back().map(|(_, p)| *p).unwrap_or_default()
    }

    /// Move the clock on by `seconds`, steering it towards a delay behind
    /// the newest pose, and say what to show.
    pub fn advance(&mut self, seconds: f32) -> Option<Transform> {
        let newest = self.samples.back()?.0;
        let target = newest - self.delay();
        let ticks = seconds as f64 * NET_HZ as f64;
        self.render += ticks;
        let off = target - self.render;
        if off.abs() > 4.0 {
            self.render = target;
        } else {
            // Steered, not assigned: assigning re-quantises the picture to
            // when datagrams happened to arrive. A tenth of the way a tick.
            self.render += off * (0.1 * ticks).min(1.0);
        }
        self.render = self.render.min(newest);
        while self.samples.len() > 2 && self.samples[1].0 < self.render - 1.0 {
            self.samples.pop_front();
        }
        let mut pose = self.pose_at(self.render);
        if let Some((gap, turn, age, join, from)) = &mut self.blend {
            let weight = if self.render >= *join {
                *age += seconds;
                let x = (*age / HANDOVER_BLEND).clamp(0.0, 1.0);
                1.0 - x * x * (3.0 - 2.0 * x)
            } else {
                ((self.render - *from) / (*join - *from).max(1e-6)).clamp(0.0, 1.0) as f32
            };
            pose.position += *gap * weight;
            pose.set_rotation(Quat::IDENTITY.slerp(*turn, weight) * pose.rotation());
            if self.render >= *join && *age >= HANDOVER_BLEND {
                self.blend = None;
            }
        }
        Some(pose)
    }
}

/// Show every replica where its buffer says, this frame. Call it every
/// frame with the frame's time.
pub fn present(world: &mut hecs::World, seconds: f32) {
    for (transform, presented) in world
        .query_mut::<(&mut Transform, &mut Presented)>()
        .with::<&Replica>()
    {
        if let Some(pose) = presented.advance(seconds) {
            *transform = pose;
        }
    }
    crate::world::apply_hierarchy(world);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32) -> Transform {
        Transform {
            position: Vec3::new(x, 0.0, 0.0),
            ..Default::default()
        }
    }

    /// Poses arriving one a tick, as they do, and the clock run between.
    fn stream(
        p: &mut Presented,
        sender: u32,
        ticks: std::ops::RangeInclusive<u64>,
        x: impl Fn(u64) -> f32,
    ) -> f32 {
        let mut shown = 0.0;
        for tick in ticks {
            p.push(PeerId(sender), tick, at(x(tick)));
            shown = p.advance(1.0 / NET_HZ).unwrap().position.x;
        }
        shown
    }

    #[test]
    fn a_replica_is_shown_between_what_arrived_a_moment_behind() {
        let mut p = Presented::default();
        let shown = stream(&mut p, 1, 1..=60, |t| t as f32);
        // Two ticks behind the newest.
        assert!((shown - (60.0 - DELAY as f32)).abs() < 0.3, "{shown}");
        // A stream that stopped is held at its last pose, not guessed past.
        for _ in 0..100 {
            p.advance(1.0 / NET_HZ);
        }
        assert!((p.advance(0.0).unwrap().position.x - 60.0).abs() < 1e-3);
    }

    #[test]
    fn a_new_owner_is_bent_into_the_old_stream_not_jumped_to() {
        let mut p = Presented::default();
        let before = stream(&mut p, 1, 1..=30, |t| t as f32);
        // The new owner took it from an older picture — two metres behind —
        // and its clock reads 900.
        let first = stream(&mut p, 2, 900..=900, |_| 28.0);
        assert!(first > before - 0.5, "no jump back: {before} then {first}");
        let settled = stream(&mut p, 2, 901..=960, |t| 28.0 + (t - 900) as f32);
        let truth = 28.0 + 60.0 - DELAY as f32;
        assert!(
            (settled - truth).abs() < 0.3,
            "on the new owner's truth: {settled} vs {truth}"
        );
    }

    #[test]
    fn a_teleport_is_jumped() {
        let mut p = Presented::default();
        p.push(PeerId(1), 1, at(0.0));
        p.push(PeerId(1), 2, at(0.0));
        p.push(PeerId(1), 3, at(50.0));
        for _ in 0..10 {
            p.advance(1.0 / NET_HZ);
        }
        assert!((p.advance(0.0).unwrap().position.x - 50.0).abs() < 1e-3);
    }
}
