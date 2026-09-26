//! The server: a state cache and a router, never a simulator.
//!
//! The dacha simulator's shape, carried over whole. The server knows who
//! is connected, which entity exists and who owns it, and the latest value
//! of each component of each entity — as blobs it never opens, the
//! transform included. It routes what one client says to the others,
//! rules on who drives what, and hands a late joiner everything it holds.
//! It runs no game code, and holds no world: it lives in the host's
//! process on its own thread ([`ServerThread`]), and the host's own game
//! is a client of it like everyone else's, through a loopback. A game on
//! its own is the same — a server with one client — so there is no second
//! path for single player (DNA, postulate 4).
//!
//! What it decides, and nothing else:
//!
//! * **Who is who.** A client is given its [`PeerId`] here, never reused:
//!   a rejoin is a new client, so nothing still pointing at the old one
//!   follows the new. The host's own connection is peer 0.
//! * **Who drives what.** Every entity has an owner. A scene entity
//!   nobody asked for is the host's; a spawned one is its spawner's. A
//!   request for ownership is granted, and the ruling goes to everyone,
//!   the asker included. Anything from a non-owner about an entity is
//!   dropped.
//! * **When a client may hear about the world**: only once it says it
//!   stands in the scene of this epoch (the join gate).
//! * **Where a leaver's things go**: to whoever owns fewest (ties to the
//!   lower id), or with them, if they asked to be despawned with their
//!   owner.
//!
//! Malformed input is expected input: a datagram that does not read is
//! dropped, never a reason to stop.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::link::{Ended, Link, LinkEvent, Mode};
use super::protocol::{self, BlobId, Entry, Record, ToClient, ToServer, PROTOCOL};
use super::PeerId;
use crate::id::EntityId;

/// A connection: which link, and which endpoint on it.
pub type Key = (usize, PeerId);

/// About what fits in a datagram without it being split on the way.
pub const DATAGRAM_BUDGET: usize = 1100;

struct Client {
    key: Key,
    name: String,
    ready: bool,
    /// Others' entries waiting to go to it, the newest of each entity.
    pending: HashMap<EntityId, Pending>,
    /// Bytes it may be sent now, and when that was reckoned.
    allowance: f32,
    reckoned: web_time::Instant,
}

/// An entry waiting for its turn to one client: whose, of which tick,
/// whether partial, and how many flushes it has waited.
struct Pending {
    owner: PeerId,
    tick: u64,
    partial: bool,
    entry: Entry,
    waited: f32,
}

impl Pending {
    /// A newer entry of the same entity over this one: a whole one, or
    /// another owner's, replaces it; a partial one lays its blobs over it,
    /// so a change in the one it replaces is not lost.
    fn take(&mut self, owner: PeerId, tick: u64, partial: bool, entry: Entry) {
        if owner != self.owner || !partial {
            (self.owner, self.partial, self.entry) = (owner, partial, entry);
        } else {
            for (name, bytes) in entry.blobs {
                match self.entry.blobs.iter_mut().find(|(n, _)| *n == name) {
                    Some(slot) => slot.1 = bytes,
                    None => self.entry.blobs.push((name, bytes)),
                }
            }
        }
        self.tick = tick;
    }

    fn size(&self) -> usize {
        self.entry.blobs.iter().map(|(_, b)| b.len() + 3).sum::<usize>() + 12
    }
}

/// Bytes a second of others' snapshots the server sends one client at
/// most: twice what one peer may send ([`super::sync::BUDGET`] at 30
/// ticks). Past it, what has waited longest goes first and the rest waits,
/// the newest of each (docs/netsim.md, «Трафик»); the last words go at
/// once.
pub const CLIENT_BUDGET: f32 = 60.0 * 1024.0;

/// An entity as the server holds it.
struct Held {
    owner: PeerId,
    prefab: Option<String>,
    despawn_with_owner: bool,
    gone: bool,
    blobs: BTreeMap<BlobId, Vec<u8>>,
    /// The newest tick taken from the owner; forgotten when it changes
    /// hands, since two owners' ticks are two clocks.
    tick: Option<u64>,
}

/// What the server has to say to one client this tick.
#[derive(Default)]
struct Outbox {
    reliable: Vec<ToClient>,
    unreliable: Vec<ToClient>,
}

/// The session's server. Call [`Server::tick`] thirty times a second, or
/// hand it to a [`ServerThread`].
pub struct Server {
    links: Vec<Link>,
    host: Key,
    clients: BTreeMap<PeerId, Client>,
    by_key: HashMap<Key, PeerId>,
    next_peer: u32,
    entities: HashMap<EntityId, Held>,
    scene: String,
    epoch: u32,
    session: u64,
    fingerprint: u64,
    outboxes: BTreeMap<PeerId, Outbox>,
    /// What it did that someone may want to read: joins, refusals.
    pub log: Vec<String>,
    /// When the session began: the clock everyone agrees on.
    began: web_time::Instant,
    /// Each entity's short number and when it was given; by number; the
    /// next; and the ones given this tick, to tell everyone.
    shorts: HashMap<EntityId, (u32, web_time::Instant)>,
    by_short: HashMap<u32, EntityId>,
    next_short: u32,
    given: Vec<(EntityId, u32)>,
    /// [`SHORT_AGE`], unless a test says.
    pub short_age: Duration,
    /// [`CLIENT_BUDGET`], unless the game says.
    pub client_budget: f32,
}

/// How long after a short number is given before the server names an
/// entity by it: long enough that the reliable word of it is in on all
/// but a failing link. Until then, and for anyone whose word is late, the
/// entry is whole — or dropped, and the next one comes.
pub const SHORT_AGE: Duration = Duration::from_secs(1);

impl Server {
    /// A server for a game of `scene`, whose components hash to
    /// `fingerprint`, listening on `links`. `host` is the host's own
    /// connection; it becomes peer 0.
    pub fn new(links: Vec<Link>, host: Key, scene: &str, fingerprint: u64) -> Self {
        Self {
            links,
            host,
            clients: BTreeMap::new(),
            by_key: HashMap::new(),
            next_peer: 1,
            entities: HashMap::new(),
            scene: scene.to_string(),
            epoch: 1,
            session: EntityId::fresh().raw(),
            fingerprint,
            outboxes: BTreeMap::new(),
            log: Vec::new(),
            began: web_time::Instant::now(),
            shorts: HashMap::new(),
            by_short: HashMap::new(),
            next_short: 1,
            given: Vec::new(),
            short_age: SHORT_AGE,
            client_budget: CLIENT_BUDGET,
        }
    }

    /// An entity's short number, given now if it has none.
    fn short(&mut self, id: EntityId) -> u32 {
        if let Some((short, _)) = self.shorts.get(&id) {
            return *short;
        }
        let short = self.next_short;
        self.next_short += 1;
        self.shorts.insert(id, (short, web_time::Instant::now()));
        self.by_short.insert(short, id);
        self.given.push((id, short));
        short
    }

    pub fn session(&self) -> u64 {
        self.session
    }

    /// Everyone connected and in, by id.
    pub fn members(&self) -> Vec<PeerId> {
        self.clients.keys().copied().collect()
    }

    /// Who owns an entity, as the server has it.
    pub fn owner(&self, id: EntityId) -> Option<PeerId> {
        self.entities.get(&id).map(|e| e.owner)
    }

    /// One tick: take in what arrived, act on it, send what it made.
    pub fn tick(&mut self) {
        for index in 0..self.links.len() {
            for event in self.links[index].poll() {
                match event {
                    LinkEvent::Connected(_) => {}
                    LinkEvent::Data(endpoint, bytes, _) => {
                        match protocol::decode::<ToServer>(&bytes) {
                            Ok(messages) => {
                                for message in messages {
                                    self.dispatch((index, endpoint), message);
                                }
                            }
                            Err(e) => self.log.push(format!("from {endpoint:?}: {e}")),
                        }
                    }
                    LinkEvent::Disconnected(endpoint, ended) => {
                        if let Some(peer) = self.by_key.get(&(index, endpoint)).copied() {
                            self.leave(peer, ended == Ended::Clean);
                        }
                    }
                }
            }
        }
        self.flush();
    }

    /// Tell everyone the session is over, and send it.
    pub fn end(&mut self) {
        let everyone: Vec<PeerId> = self.clients.keys().copied().collect();
        for peer in everyone {
            self.reliable(peer, ToClient::SessionEnding);
        }
        self.flush();
    }

    fn reliable(&mut self, to: PeerId, message: ToClient) {
        self.outboxes.entry(to).or_default().reliable.push(message);
    }

    fn broadcast(&mut self, message: ToClient, except: Option<PeerId>, mode: Mode) {
        let ready: Vec<PeerId> = self
            .clients
            .iter()
            .filter(|(p, c)| c.ready && Some(**p) != except)
            .map(|(p, _)| *p)
            .collect();
        for peer in ready {
            let outbox = self.outboxes.entry(peer).or_default();
            match mode {
                Mode::Reliable => outbox.reliable.push(message.clone()),
                Mode::Unreliable => outbox.unreliable.push(message.clone()),
            }
        }
    }

    fn flush(&mut self) {
        if !self.given.is_empty() {
            let pairs = std::mem::take(&mut self.given);
            self.broadcast(ToClient::Shorts { pairs }, None, Mode::Reliable);
        }
        self.send_pending();
        for (peer, outbox) in std::mem::take(&mut self.outboxes) {
            let Some(client) = self.clients.get(&peer) else {
                continue;
            };
            let (index, endpoint) = client.key;
            for (messages, mode) in [
                (outbox.reliable, Mode::Reliable),
                (outbox.unreliable, Mode::Unreliable),
            ] {
                for datagram in protocol::pack(messages, DATAGRAM_BUDGET) {
                    self.links[index].send(endpoint, datagram, mode);
                }
            }
        }
    }

    fn dispatch(&mut self, key: Key, message: ToServer) {
        let Some(peer) = self.by_key.get(&key).copied() else {
            if let ToServer::JoinRequest {
                protocol,
                fingerprint,
                name,
            } = message
            {
                self.join(key, protocol, fingerprint, name);
            }
            return;
        };
        let ready = self.clients.get(&peer).is_some_and(|c| c.ready);
        match message {
            ToServer::JoinRequest { .. } => {}
            ToServer::Ready { epoch } => self.ready(peer, epoch),
            ToServer::Leave => self.leave(peer, true),
            ToServer::Clock { sent } => {
                let server = self.began.elapsed().as_secs_f64();
                self.outboxes
                    .entry(peer)
                    .or_default()
                    .unreliable
                    .push(ToClient::Clock { sent, server });
            }
            ToServer::SetScene { scene } => {
                if peer == PeerId::HOST {
                    self.set_scene(scene);
                } else {
                    self.log
                        .push(format!("peer {} may not change the scene", peer.0));
                }
            }
            ToServer::Rpc { to, kind, body } => {
                let message = ToClient::Rpc {
                    from: peer,
                    kind,
                    body,
                };
                match to {
                    Some(target) if self.clients.contains_key(&target) => {
                        self.reliable(target, message)
                    }
                    Some(_) => {}
                    None => self.broadcast(message, Some(peer), Mode::Reliable),
                }
            }
            // Everything else is about entities, and waits for the gate:
            // a client not standing in this epoch's scene cannot be
            // describing anything in it.
            _ if !ready => {}
            ToServer::Spawn { epoch, .. }
            | ToServer::Despawn { epoch, .. }
            | ToServer::OwnershipRequest { epoch, .. }
            | ToServer::Snapshot { epoch, .. }
                if epoch != self.epoch => {}
            ToServer::Spawn {
                id,
                prefab,
                despawn_with_owner,
                blobs,
                ..
            } => {
                if self.entities.contains_key(&id) {
                    self.log
                        .push(format!("{id}: spawned twice; the second is dropped"));
                    return;
                }
                self.short(id);
                self.entities.insert(
                    id,
                    Held {
                        owner: peer,
                        prefab: Some(prefab.clone()),
                        despawn_with_owner,
                        gone: false,
                        blobs: blobs.iter().cloned().collect(),
                        tick: None,
                    },
                );
                self.broadcast(
                    ToClient::Spawn {
                        owner: peer,
                        id,
                        prefab,
                        blobs,
                    },
                    Some(peer),
                    Mode::Reliable,
                );
            }
            ToServer::Despawn { id, .. } => {
                let Some(held) = self.held(id) else { return };
                if held.owner != peer {
                    self.log.push(format!(
                        "{id}: peer {} despawned it, but does not own it",
                        peer.0
                    ));
                    return;
                }
                if held.prefab.is_some() {
                    self.entities.remove(&id);
                } else {
                    // A scene entity destroyed: joiners load it with the
                    // scene, so they must be told it is gone.
                    held.gone = true;
                    held.blobs.clear();
                }
                self.broadcast(ToClient::Despawn { id }, Some(peer), Mode::Reliable);
            }
            ToServer::OwnershipRequest { id, .. } => {
                let Some(held) = self.held(id) else { return };
                if held.gone {
                    return;
                }
                if held.owner == peer {
                    // Already theirs — but they are waiting for a ruling.
                    self.reliable(peer, ToClient::OwnershipChanged { id, owner: peer });
                    return;
                }
                held.owner = peer;
                held.tick = None;
                self.broadcast(
                    ToClient::OwnershipChanged { id, owner: peer },
                    None,
                    Mode::Reliable,
                );
            }
            ToServer::Snapshot {
                tick,
                settle,
                partial,
                entries,
                ..
            } => self.snapshot(peer, tick, settle, partial, entries),
        }
    }

    /// The server's record of an entity, made on first mention for one of
    /// the scene's: nobody announces a level, and an entity nobody has
    /// touched has no record at all.
    fn held(&mut self, id: EntityId) -> Option<&mut Held> {
        if !self.entities.contains_key(&id) {
            self.short(id);
        }
        Some(self.entities.entry(id).or_insert(Held {
            owner: PeerId::HOST,
            prefab: None,
            despawn_with_owner: false,
            gone: false,
            blobs: BTreeMap::new(),
            tick: None,
        }))
    }

    fn join(&mut self, key: Key, version: u32, fingerprint: u64, name: String) {
        let refuse = |reason: String| ToClient::JoinRejected { reason };
        let refusal = if version != PROTOCOL {
            Some(format!(
                "protocol {version}, but this game speaks {PROTOCOL}"
            ))
        } else if fingerprint != self.fingerprint {
            Some("a different build: its networked components differ from the host's".to_string())
        } else {
            None
        };
        if let Some(reason) = refusal {
            self.log.push(format!("{key:?} turned away: {reason}"));
            let datagram = protocol::encode(&[refuse(reason)]);
            self.links[key.0].send(key.1, datagram, Mode::Reliable);
            return;
        }
        let peer = if key == self.host {
            PeerId::HOST
        } else {
            let peer = PeerId(self.next_peer);
            self.next_peer += 1;
            peer
        };
        self.by_key.insert(key, peer);
        let name = if name.is_empty() {
            format!("Player {}", peer.0 + 1)
        } else {
            name
        };
        self.log.push(format!("peer {} ({name}) joined", peer.0));
        self.clients.insert(
            peer,
            Client {
                key,
                name,
                ready: false,
                pending: HashMap::new(),
                // A tenth of a second's worth to begin with.
                allowance: self.client_budget * 0.1,
                reckoned: web_time::Instant::now(),
            },
        );
        let accepted = ToClient::JoinAccepted {
            you: peer,
            host: PeerId::HOST,
            scene: self.scene.clone(),
            epoch: self.epoch,
            session: self.session,
        };
        self.reliable(peer, accepted);
    }

    /// Through the gate: the roster, then the world, and from now on
    /// everything.
    fn ready(&mut self, peer: PeerId, epoch: u32) {
        if epoch != self.epoch {
            return;
        }
        let Some(client) = self.clients.get_mut(&peer) else {
            return;
        };
        if client.ready {
            return;
        }
        client.ready = true;
        let name = client.name.clone();
        let roster: Vec<ToClient> = self
            .clients
            .iter()
            .filter(|(_, c)| c.ready)
            .map(|(p, c)| ToClient::ClientJoined {
                peer: *p,
                name: c.name.clone(),
            })
            .collect();
        for message in roster {
            self.reliable(peer, message);
        }
        self.broadcast(
            ToClient::ClientJoined { peer, name },
            Some(peer),
            Mode::Reliable,
        );
        // Every short number so far, before the world that may use them.
        let mut pairs: Vec<(EntityId, u32)> = self.shorts.iter().map(|(id, (short, _))| (*id, *short)).collect();
        pairs.sort_by_key(|p| p.1);
        for pairs in pairs.chunks(64) {
            self.reliable(peer, ToClient::Shorts { pairs: pairs.to_vec() });
        }
        let mut records: Vec<Record> = self
            .entities
            .iter()
            .map(|(id, held)| Record {
                id: *id,
                owner: held.owner,
                prefab: held.prefab.clone(),
                gone: held.gone,
                blobs: held
                    .blobs
                    .iter()
                    .map(|(n, b)| (n.clone(), b.clone()))
                    .collect(),
            })
            .collect();
        records.sort_by_key(|r| r.id);
        // A few records to a message, so no message outgrows a datagram
        // by much.
        let mut chunks: Vec<Vec<Record>> = Vec::new();
        let mut size = 0;
        for record in records {
            let one = postcard::to_stdvec(&record).map(|b| b.len()).unwrap_or(0);
            if chunks.is_empty() || size + one > DATAGRAM_BUDGET {
                chunks.push(Vec::new());
                size = 0;
            }
            size += one;
            chunks.last_mut().expect("just pushed").push(record);
        }
        if chunks.is_empty() {
            chunks.push(Vec::new());
        }
        let count = chunks.len();
        for (i, records) in chunks.into_iter().enumerate() {
            self.reliable(
                peer,
                ToClient::WorldState {
                    records,
                    last: i + 1 == count,
                },
            );
        }
    }

    fn snapshot(&mut self, peer: PeerId, tick: u64, settle: bool, partial: bool, entries: Vec<Entry>) {
        let mut forward = Vec::new();
        let mut removed = Vec::new();
        for mut entry in entries {
            if entry.short != 0 {
                let Some(&id) = self.by_short.get(&entry.short) else {
                    continue;
                };
                entry.id = id;
                entry.short = 0;
            }
            let known = self.entities.contains_key(&entry.id);
            // A scene entity first heard of in a snapshot is registered as
            // the host's; anyone else's word about it is not yet worth
            // anything.
            if !known && peer != PeerId::HOST {
                continue;
            }
            let Some(held) = self.held(entry.id) else {
                continue;
            };
            if held.owner != peer || held.gone {
                continue;
            }
            if held.tick.is_some_and(|t| t >= tick) {
                continue;
            }
            held.tick = Some(tick);
            // A whole entry names all it has; a partial one only adds.
            let names: Vec<BlobId> = held
                .blobs
                .keys()
                .filter(|n| !partial && !entry.blobs.iter().any(|(m, _)| m == *n))
                .cloned()
                .collect();
            for name in &names {
                held.blobs.remove(name);
            }
            for (name, bytes) in &entry.blobs {
                held.blobs.insert(*name, bytes.clone());
            }
            if !names.is_empty() {
                removed.push(ToClient::ComponentsRemoved {
                    id: entry.id,
                    tick,
                    names,
                });
            }
            forward.push(entry);
        }
        for message in removed {
            self.broadcast(message, Some(peer), Mode::Reliable);
        }
        if forward.is_empty() {
            return;
        }
        let others: Vec<PeerId> = self.clients.iter().filter(|(p, c)| c.ready && **p != peer).map(|(p, _)| *p).collect();
        if settle {
            // The last word goes at once, reliably, and what waited of the
            // same is stale.
            for other in &others {
                if let Some(client) = self.clients.get_mut(other) {
                    for entry in &forward {
                        if client.pending.get(&entry.id).is_some_and(|p| p.owner == peer && p.tick <= tick) {
                            client.pending.remove(&entry.id);
                        }
                    }
                }
            }
            let forward = forward.into_iter().map(|entry| self.shortened(entry)).collect();
            self.broadcast(ToClient::Snapshot { owner: peer, tick, settle, partial, entries: forward }, Some(peer), Mode::Reliable);
            return;
        }
        for other in others {
            let Some(client) = self.clients.get_mut(&other) else { continue };
            for entry in &forward {
                match client.pending.get_mut(&entry.id) {
                    Some(waiting) => waiting.take(peer, tick, partial, entry.clone()),
                    None => {
                        client.pending.insert(entry.id, Pending { owner: peer, tick, partial, entry: entry.clone(), waited: 0.0 });
                    }
                }
            }
        }
    }

    /// Named short once everyone has had time to hear the number.
    fn shortened(&self, entry: Entry) -> Entry {
        match self.shorts.get(&entry.id) {
            Some(&(short, given)) if given.elapsed() >= self.short_age => entry.shortened(short),
            _ => entry,
        }
    }

    /// Each client's waiting entries within its allowance, the longest
    /// waited first (Fiedler's priority accumulator, as the peers' own
    /// budget): what does not fit gains a point and waits.
    fn send_pending(&mut self) {
        let now = web_time::Instant::now();
        let budget = self.client_budget;
        let mut out: Vec<(PeerId, ToClient)> = Vec::new();
        for (&peer, client) in &mut self.clients {
            let dt = now.duration_since(client.reckoned).as_secs_f32();
            client.reckoned = now;
            // At most a tenth of a second's worth saved up.
            client.allowance = (client.allowance + budget * dt).min(budget * 0.1);
            if client.pending.is_empty() {
                continue;
            }
            let mut order: Vec<(EntityId, f32)> = client.pending.iter().map(|(id, p)| (*id, p.waited)).collect();
            order.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            let mut going: Vec<Pending> = Vec::new();
            for (id, _) in order {
                let size = client.pending[&id].size();
                if client.allowance < size as f32 && !(going.is_empty() && client.allowance > 0.0) {
                    continue;
                }
                client.allowance -= size as f32;
                going.push(client.pending.remove(&id).expect("listed"));
            }
            for waiting in client.pending.values_mut() {
                waiting.waited += 1.0;
            }
            // One message for each owner's tick.
            going.sort_by_key(|p| (p.owner, p.tick, p.partial));
            let mut messages: Vec<(PeerId, u64, bool, Vec<Entry>)> = Vec::new();
            for p in going {
                match messages.last_mut() {
                    Some((o, t, partial, entries)) if *o == p.owner && *t == p.tick && *partial == p.partial => entries.push(p.entry),
                    _ => messages.push((p.owner, p.tick, p.partial, vec![p.entry])),
                }
            }
            for (owner, tick, partial, entries) in messages {
                out.push((peer, ToClient::Snapshot { owner, tick, settle: false, partial, entries }));
            }
        }
        for (peer, message) in out {
            let message = match message {
                ToClient::Snapshot { owner, tick, settle, partial, entries } => ToClient::Snapshot {
                    owner,
                    tick,
                    settle,
                    partial,
                    entries: entries.into_iter().map(|e| self.shortened(e)).collect(),
                },
                other => other,
            };
            self.outboxes.entry(peer).or_default().unreliable.push(message);
        }
    }

    fn set_scene(&mut self, scene: String) {
        self.epoch += 1;
        self.scene = scene.clone();
        self.entities.clear();
        for client in self.clients.values_mut() {
            client.ready = false;
            client.pending.clear();
        }
        let everyone: Vec<PeerId> = self.clients.keys().copied().collect();
        for peer in everyone {
            self.reliable(
                peer,
                ToClient::SceneChanged {
                    scene: scene.clone(),
                    epoch: self.epoch,
                },
            );
        }
    }

    /// A client gone: everyone told, its things to whoever owns fewest —
    /// or with it, when they were to die with their owner.
    fn leave(&mut self, peer: PeerId, clean: bool) {
        let Some(client) = self.clients.remove(&peer) else {
            return;
        };
        self.by_key.remove(&client.key);
        self.outboxes.remove(&peer);
        self.log.push(format!(
            "peer {} left{}",
            peer.0,
            if clean { "" } else { ", silent" }
        ));
        self.broadcast(ToClient::ClientLeft { peer, clean }, None, Mode::Reliable);
        let orphans: Vec<EntityId> = {
            let mut ids: Vec<EntityId> = self
                .entities
                .iter()
                .filter(|(_, h)| h.owner == peer)
                .map(|(id, _)| *id)
                .collect();
            ids.sort();
            ids
        };
        for id in orphans {
            let dies = self.entities.get(&id).is_some_and(|h| h.despawn_with_owner);
            if dies {
                self.entities.remove(&id);
                self.broadcast(ToClient::Despawn { id }, None, Mode::Reliable);
                continue;
            }
            let Some(heir) = self.heir() else {
                // Nobody left to simulate it; it waits, the host's, for
                // the next one in.
                if let Some(held) = self.entities.get_mut(&id) {
                    held.owner = PeerId::HOST;
                    held.tick = None;
                }
                continue;
            };
            if let Some(held) = self.entities.get_mut(&id) {
                held.owner = heir;
                held.tick = None;
            }
            self.broadcast(
                ToClient::OwnershipChanged { id, owner: heir },
                None,
                Mode::Reliable,
            );
        }
    }

    /// Who inherits: the ready member owning fewest, ties to the lower id
    /// — so the same events always give the same answer.
    fn heir(&self) -> Option<PeerId> {
        self.clients
            .iter()
            .filter(|(_, c)| c.ready)
            .map(|(p, _)| {
                let owned = self.entities.values().filter(|h| h.owner == *p).count();
                (owned, *p)
            })
            .min()
            .map(|(_, p)| p)
    }
}

/// A [`Server`] on a thread of its own, ticking at `hz`. The thread is a
/// wall as much as a speed-up: the server holds no world and no game
/// object, and running it over there is what turns that from a rule into
/// something that cannot compile. Dropping it ends the session — everyone
/// is told — and joins the thread.
pub struct ServerThread {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ServerThread {
    pub fn start(mut server: Server, hz: f32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let period = Duration::from_secs_f32(1.0 / hz.max(1.0));
        let thread = std::thread::Builder::new()
            .name("scrap-server".into())
            .spawn(move || {
                while !flag.load(Ordering::Acquire) {
                    let started = web_time::Instant::now();
                    server.tick();
                    for line in server.log.drain(..) {
                        eprintln!("server: {line}");
                    }
                    if let Some(rest) = period.checked_sub(started.elapsed()) {
                        std::thread::sleep(rest);
                    }
                }
                server.end();
            })
            .expect("a thread for the server");
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for ServerThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocol::Blob;
    use crate::net::wire::Loopback;

    /// A component's number in these tests.
    const HP: BlobId = 1;

    /// A server on endpoint 0 of a loopback, and clients 1.. as raw links,
    /// so a test speaks the protocol itself.
    struct Rig {
        server: Server,
        clients: Vec<Link>,
    }

    impl Rig {
        fn new(count: u32) -> Self {
            let mut ends = Loopback::network(count + 1).into_iter();
            let server_end = ends.next().unwrap();
            let clients: Vec<Link> = ends
                .map(|end| {
                    let mut link = Link::dialling(end);
                    link.dial(PeerId(0));
                    link
                })
                .collect();
            let mut server = Server::new(vec![Link::accepting(server_end)], (0, PeerId(1)), "main", 7);
            // No budget but where a test sets one: these run faster than
            // any clock would refill it.
            server.client_budget = 1e12;
            Self { server, clients }
        }

        fn say(&mut self, client: usize, messages: Vec<ToServer>) {
            self.clients[client].send(PeerId(0), protocol::encode(&messages), Mode::Reliable);
        }

        /// Run the server, and gather what each client heard.
        fn run(&mut self) -> Vec<Vec<ToClient>> {
            self.server.tick();
            self.clients
                .iter_mut()
                .map(|link| {
                    link.poll()
                        .into_iter()
                        .filter_map(|e| match e {
                            LinkEvent::Data(_, bytes, _) => {
                                protocol::decode::<ToClient>(&bytes).ok()
                            }
                            _ => None,
                        })
                        .flatten()
                        .collect()
                })
                .collect()
        }

        /// Everyone in and ready.
        fn all_in(count: u32) -> Self {
            let mut rig = Self::new(count);
            for i in 0..count as usize {
                rig.say(i, vec![join(&format!("p{i}"))]);
            }
            rig.run();
            for i in 0..count as usize {
                rig.say(i, vec![ToServer::Ready { epoch: 1 }]);
            }
            rig.run();
            rig
        }
    }

    fn join(name: &str) -> ToServer {
        ToServer::JoinRequest {
            protocol: PROTOCOL,
            fingerprint: 7,
            name: name.into(),
        }
    }

    fn id(n: u64) -> EntityId {
        EntityId::from_raw(n)
    }

    fn snapshot(tick: u64, n: u64, value: u8) -> ToServer {
        ToServer::Snapshot {
            epoch: 1,
            tick,
            settle: false,
            partial: false,
            entries: vec![Entry::new(id(n), vec![(HP, vec![value])])],
        }
    }

    #[test]
    fn the_host_is_peer_0_and_others_count_up_and_the_world_waits_for_ready() {
        let mut rig = Rig::new(2);
        rig.say(1, vec![join("guest")]);
        rig.say(0, vec![join("host")]);
        let heard = rig.run();
        assert!(matches!(
            heard[0][0],
            ToClient::JoinAccepted {
                you: PeerId(0),
                epoch: 1,
                ..
            }
        ));
        assert!(matches!(
            heard[1][0],
            ToClient::JoinAccepted { you: PeerId(1), .. }
        ));
        // The host plays before the guest is ready: the guest hears
        // nothing of it until then.
        rig.say(0, vec![ToServer::Ready { epoch: 1 }, snapshot(1, 5, 1)]);
        let heard = rig.run();
        assert!(heard[1].is_empty(), "{:?}", heard[1]);
        rig.say(1, vec![ToServer::Ready { epoch: 1 }]);
        let heard = rig.run();
        let names: Vec<_> = heard[1]
            .iter()
            .filter_map(|m| match m {
                ToClient::ClientJoined { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["host", "guest"], "the roster, itself included");
        let world = heard[1].iter().find_map(|m| match m {
            ToClient::WorldState {
                records,
                last: true,
            } => Some(records.clone()),
            _ => None,
        });
        let world = world.expect("the world after the roster");
        assert_eq!(world[0].blobs, [(HP, vec![1])]);
        assert_eq!(world[0].owner, PeerId::HOST);
    }

    #[test]
    fn another_build_is_turned_away() {
        let mut rig = Rig::new(1);
        rig.say(
            0,
            vec![ToServer::JoinRequest {
                protocol: PROTOCOL,
                fingerprint: 8,
                name: "x".into(),
            }],
        );
        let heard = rig.run();
        assert!(
            matches!(&heard[0][0], ToClient::JoinRejected { reason } if reason.contains("different build"))
        );
        assert!(rig.server.members().is_empty());
    }

    #[test]
    fn a_claim_is_granted_and_told_to_everyone_and_the_old_owner_is_no_longer_heard() {
        let mut rig = Rig::all_in(3);
        rig.say(
            2,
            vec![ToServer::OwnershipRequest {
                epoch: 1,
                id: id(9),
            }],
        );
        let heard = rig.run();
        for client in &heard {
            assert!(
                client.contains(&ToClient::OwnershipChanged {
                    id: id(9),
                    owner: PeerId(2)
                }),
                "{client:?}"
            );
        }
        // The host still talking about it is dropped; the new owner's word
        // goes through, even at a lower tick — a new owner is a new clock.
        rig.say(0, vec![snapshot(500, 9, 1)]);
        rig.say(2, vec![snapshot(3, 9, 2)]);
        let heard = rig.run();
        let snaps: Vec<_> = heard[1]
            .iter()
            .filter_map(|m| match m {
                ToClient::Snapshot { owner, tick, .. } => Some((*owner, *tick)),
                _ => None,
            })
            .collect();
        assert_eq!(snaps, [(PeerId(2), 3)]);
    }

    fn many(tick: u64, count: u64, size: usize) -> ToServer {
        ToServer::Snapshot {
            epoch: 1,
            tick,
            settle: false,
            partial: false,
            entries: (0..count).map(|n| Entry::new(id(100 + n), vec![(HP, vec![tick as u8; size])])).collect(),
        }
    }

    fn heard_entries(heard: &[ToClient]) -> Vec<EntityId> {
        heard
            .iter()
            .flat_map(|m| match m {
                ToClient::Snapshot { entries, .. } => entries.iter().map(|e| e.id).collect(),
                _ => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn over_a_clients_budget_the_longest_waited_go_first_and_none_is_left_behind() {
        let mut rig = Rig::all_in(2);
        // 30 KB a second, flushed every 10 ms: about 300 bytes a flush,
        // two or three entries of the ten that change every time.
        rig.server.client_budget = 30_000.0;
        let mut last_heard: HashMap<EntityId, usize> = HashMap::new();
        let mut most_waited = 0;
        let mut bytes = 0usize;
        let flushes = 40;
        let began = std::time::Instant::now();
        for tick in 1..=flushes as u64 {
            std::thread::sleep(Duration::from_millis(10));
            rig.say(0, vec![many(tick, 10, 100)]);
            let heard = rig.run();
            bytes += heard[1].iter().map(|m| postcard::to_stdvec(m).unwrap().len()).sum::<usize>();
            for e in heard_entries(&heard[1]) {
                if let Some(last) = last_heard.insert(e, tick as usize) {
                    most_waited = most_waited.max(tick as usize - last);
                }
            }
        }
        assert_eq!(last_heard.len(), 10, "every one went");
        assert!(most_waited <= 6, "none waited more than six flushes: {most_waited}");
        let seconds = began.elapsed().as_secs_f32();
        assert!((bytes as f32 / seconds) < 40_000.0, "{} bytes a second", bytes as f32 / seconds);
    }

    #[test]
    fn partial_entries_that_waited_together_are_laid_over_each_other() {
        let mut rig = Rig::all_in(2);
        rig.server.client_budget = 0.0;
        rig.say(0, vec![snapshot(1, 4, 1)]);
        rig.run();
        const MANA: BlobId = 2;
        let partial = |tick: u64, blobs: Vec<Blob>| ToServer::Snapshot { epoch: 1, tick, settle: false, partial: true, entries: vec![Entry::new(id(4), blobs)] };
        rig.say(0, vec![partial(2, vec![(MANA, vec![5])])]);
        rig.run();
        rig.say(0, vec![partial(3, vec![(HP, vec![9])])]);
        rig.run();
        rig.server.client_budget = 1e9;
        std::thread::sleep(Duration::from_millis(5));
        let heard = rig.run();
        let entries: Vec<&Entry> = heard[1]
            .iter()
            .flat_map(|m| match m {
                ToClient::Snapshot { entries, tick: 3, .. } => entries.iter().collect(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(entries.len(), 1, "{heard:?}");
        let mut blobs = entries[0].blobs.clone();
        blobs.sort();
        assert_eq!(blobs, [(HP, vec![9]), (MANA, vec![5])], "the newer health and the older mana");
    }

    #[test]
    fn a_partial_snapshot_adds_and_removes_nothing() {
        let mut rig = Rig::all_in(2);
        rig.say(0, vec![snapshot(1, 4, 1)]);
        rig.run();
        let partial = |tick: u64, blobs: Vec<Blob>, partial: bool| ToServer::Snapshot {
            epoch: 1,
            tick,
            settle: false,
            partial,
            entries: vec![Entry::new(id(4), blobs)],
        };
        const MANA: BlobId = 2;
        rig.say(0, vec![partial(2, vec![(MANA, vec![7])], true)]);
        let heard = rig.run();
        assert!(!heard[1].iter().any(|m| matches!(m, ToClient::ComponentsRemoved { .. })), "{heard:?}");
        assert!(heard[1].iter().any(|m| matches!(m, ToClient::Snapshot { partial: true, .. })), "passed on as partial");
        assert_eq!(rig.server.entities[&id(4)].blobs.len(), 2, "both held");
        // A whole one without health removes it.
        rig.say(0, vec![partial(3, vec![(MANA, vec![7])], false)]);
        let heard = rig.run();
        assert!(heard[1].contains(&ToClient::ComponentsRemoved { id: id(4), tick: 3, names: vec![HP] }));
    }

    #[test]
    fn a_stale_snapshot_is_dropped_and_a_missing_component_is_a_removed_one() {
        let mut rig = Rig::all_in(2);
        rig.say(0, vec![snapshot(5, 4, 1)]);
        rig.run();
        rig.say(0, vec![snapshot(4, 4, 9)]);
        assert!(rig.run()[1].is_empty(), "older than what is held");
        rig.say(
            0,
            vec![ToServer::Snapshot {
                epoch: 1,
                tick: 6,
                settle: true,
                partial: false,
                entries: vec![Entry::new(id(4), vec![])],
            }],
        );
        let heard = rig.run();
        assert!(heard[1].contains(&ToClient::ComponentsRemoved {
            id: id(4),
            tick: 6,
            names: vec![HP]
        }));
    }

    #[test]
    fn an_entity_gets_a_short_number_told_to_all_and_is_named_by_it_both_ways() {
        let mut rig = Rig::all_in(2);
        rig.server.short_age = Duration::ZERO;
        rig.say(0, vec![snapshot(1, 4, 1)]);
        let heard = rig.run();
        let told = ToClient::Shorts { pairs: vec![(id(4), 1)] };
        assert!(heard[0].contains(&told) && heard[1].contains(&told), "{heard:?}");
        let forwarded = |heard: &[ToClient]| {
            heard.iter().find_map(|m| match m {
                ToClient::Snapshot { entries, .. } => Some(entries.clone()),
                _ => None,
            })
        };
        assert_eq!(forwarded(&heard[1]), Some(vec![Entry::new(id(4), vec![(HP, vec![1])]).shortened(1)]));
        // The owner names it short too; an unknown number is dropped.
        rig.say(
            0,
            vec![ToServer::Snapshot {
                epoch: 1,
                tick: 2,
                settle: false,
                partial: false,
                entries: vec![Entry::new(id(4), vec![(HP, vec![2])]).shortened(1), Entry::new(id(9), vec![]).shortened(77)],
            }],
        );
        let heard = rig.run();
        assert_eq!(forwarded(&heard[1]), Some(vec![Entry::new(id(4), vec![(HP, vec![2])]).shortened(1)]));
        assert_eq!(rig.server.owner(id(9)), None);
    }

    #[test]
    fn someone_coming_in_hears_every_short_number_before_the_world() {
        let mut rig = Rig::new(2);
        rig.say(0, vec![join("host")]);
        rig.run();
        rig.say(0, vec![ToServer::Ready { epoch: 1 }]);
        rig.run();
        rig.say(0, vec![snapshot(1, 4, 1), snapshot(1, 5, 1)]);
        rig.run();
        rig.say(1, vec![join("late")]);
        rig.run();
        rig.say(1, vec![ToServer::Ready { epoch: 1 }]);
        let heard = rig.run().remove(1);
        let shorts = heard.iter().position(|m| *m == ToClient::Shorts { pairs: vec![(id(4), 1), (id(5), 2)] });
        let world = heard.iter().position(|m| matches!(m, ToClient::WorldState { .. }));
        assert!(shorts.is_some() && shorts < world, "{heard:?}");
    }

    #[test]
    fn a_leavers_things_go_to_who_owns_fewest_or_die_with_them() {
        let mut rig = Rig::all_in(3);
        rig.say(
            2,
            vec![
                ToServer::Spawn {
                    epoch: 1,
                    id: id(20),
                    prefab: "crate".into(),
                    despawn_with_owner: false,
                    blobs: vec![],
                },
                ToServer::Spawn {
                    epoch: 1,
                    id: id(21),
                    prefab: "pawn".into(),
                    despawn_with_owner: true,
                    blobs: vec![],
                },
            ],
        );
        // The host already drives two things; peer 1 nothing.
        rig.say(0, vec![snapshot(1, 30, 0), snapshot(2, 31, 0)]);
        rig.run();
        rig.say(2, vec![ToServer::Leave]);
        let heard = rig.run();
        assert!(heard[0].contains(&ToClient::ClientLeft {
            peer: PeerId(2),
            clean: true
        }));
        assert!(heard[0].contains(&ToClient::Despawn { id: id(21) }));
        assert!(heard[0].contains(&ToClient::OwnershipChanged {
            id: id(20),
            owner: PeerId(1)
        }));
        assert_eq!(rig.server.owner(id(20)), Some(PeerId(1)));
    }

    #[test]
    fn only_the_host_moves_everyone_and_the_old_epoch_is_dropped() {
        let mut rig = Rig::all_in(2);
        rig.say(
            1,
            vec![ToServer::SetScene {
                scene: "cave".into(),
            }],
        );
        rig.run();
        assert!(rig
            .server
            .log
            .iter()
            .any(|l| l.contains("may not change the scene")));
        rig.say(
            0,
            vec![ToServer::SetScene {
                scene: "cave".into(),
            }],
        );
        let heard = rig.run();
        assert!(heard[1].contains(&ToClient::SceneChanged {
            scene: "cave".into(),
            epoch: 2
        }));
        let mut moved = snapshot(9, 4, 1);
        if let ToServer::Snapshot { epoch, .. } = &mut moved {
            *epoch = 2;
        }
        rig.say(0, vec![ToServer::Ready { epoch: 2 }, moved]);
        rig.say(1, vec![ToServer::Ready { epoch: 2 }]);
        rig.run();
        // A straggler from the old scene names an id the new one reuses.
        rig.say(
            1,
            vec![ToServer::OwnershipRequest {
                epoch: 1,
                id: id(4),
            }],
        );
        rig.run();
        assert_eq!(rig.server.owner(id(4)), Some(PeerId::HOST));
    }

    #[test]
    fn a_message_for_everyone_goes_to_everyone_else() {
        let mut rig = Rig::all_in(3);
        rig.say(
            1,
            vec![ToServer::Rpc {
                to: None,
                kind: "bell".into(),
                body: vec![1],
            }],
        );
        let heard = rig.run();
        let bell = ToClient::Rpc {
            from: PeerId(1),
            kind: "bell".into(),
            body: vec![1],
        };
        assert!(heard[0].contains(&bell) && heard[2].contains(&bell));
        assert!(!heard[1].contains(&bell), "not back to the sender");
    }
}
