//! The seam for shared authority: who owns what, what goes over the wire,
//! and who may change it.
//!
//! DNA, postulate 4 and the accepted decision: shared authority — every
//! object has an owner, and ownership passes. What is here is the part
//! that has to exist before there are many components to retrofit:
//!
//! * [`Owner`] on an entity says which peer's word counts for it.
//! * A component is sent only if it is registered networked
//!   ([`crate::Components::register_networked`]); local is the default.
//! * A [`Snapshot`] is what one peer owns: each entity's transform and its
//!   networked components, addressed by [`EntityId`] — the same stable ID
//!   the scene file uses, so peers that loaded the same scene agree on
//!   which tree is which without a handshake.
//! * [`apply`] takes a snapshot only for entities its sender owns, and an
//!   [`Handover`] only from the current owner. A peer cannot move what is
//!   not its own.
//! * [`Transport`] is the wire, a trait: Steam on desktop, UDP on phones,
//!   [`Loopback`] in tests.
//!
//! What is deliberately not here: how physics bodies owned by different
//! peers push each other, and who wins when two grab one thing at once.
//! That is the DNA's open question 3, and nothing here decides it.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::components::Components;
use crate::id::EntityId;
use crate::scene::Transform;
use crate::world::SceneId;

/// A participant: the host is peer 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PeerId(pub u32);

impl PeerId {
    pub const HOST: PeerId = PeerId(0);
}

/// Which peer's word counts for this entity. An entity without one is the
/// host's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner(pub PeerId);

/// The network identity of an entity the game spawned at run time — one
/// the scene file does not have, so no [`SceneId`] names it. Minted by the
/// peer that spawns it and sent in the [`Spawn`], so every peer calls it
/// the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetId(pub EntityId);

/// A prefab spawned at run time, as its spawner announces it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spawn {
    pub from: PeerId,
    pub id: EntityId,
    pub prefab: String,
    pub transform: Transform,
}

/// A run-time entity gone, said by its owner.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Despawn {
    pub from: PeerId,
    pub id: EntityId,
}

/// One entity, as its owner sends it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityState {
    pub id: EntityId,
    pub transform: Transform,
    /// Networked components by name, as RON.
    pub components: Vec<(String, String)>,
}

/// What one peer owns, at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub from: PeerId,
    pub entities: Vec<EntityState>,
}

/// Ownership passing on, said by the one who has it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Handover {
    pub from: PeerId,
    pub id: EntityId,
    pub to: PeerId,
}

/// What a peer joining mid-game needs besides the scene it loads itself:
/// everything spawned at run time, and who owns what now. Sent by the host,
/// who hears every spawn and handover, and taken only from the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Welcome {
    pub from: PeerId,
    pub spawns: Vec<Spawn>,
    /// Every entity whose owner is not the host, the default.
    pub owners: Vec<(EntityId, PeerId)>,
}

/// What goes over the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    Snapshot(Snapshot),
    Handover(Handover),
    Spawn(Spawn),
    Despawn(Despawn),
    Welcome(Welcome),
}

/// Which prefab a run-time entity was spawned from, so a peer joining later
/// can be told to spawn the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetPrefab(pub String);

impl Message {
    /// RON: readable in a log, and the same text an agent reads in a test.
    /// A compact binary encoding is a transport's business later.
    pub fn encode(&self) -> Vec<u8> {
        ron::to_string(self).unwrap_or_default().into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        ron::from_str(text).map_err(|e| e.to_string())
    }
}

/// Ask for an entity's transform to glide between its owner's snapshots
/// rather than jump to each: what a remote player or a thrown crate wants.
pub fn smooth_this(world: &mut hecs::World, entity: hecs::Entity) {
    let now = world
        .get::<&Transform>(entity)
        .map(|t| *t)
        .unwrap_or_default();
    let _ = world.insert_one(
        entity,
        Remote {
            from: now,
            to: now,
            elapsed: 0.0,
            interval: 0.1,
        },
    );
}

/// Who owns an entity: its [`Owner`], or the host.
pub fn owner_of(world: &hecs::World, entity: hecs::Entity) -> PeerId {
    world.get::<&Owner>(entity).map_or(PeerId::HOST, |o| o.0)
}

/// What `me` owns, as a snapshot to send.
pub fn snapshot(world: &hecs::World, components: &Components, me: PeerId) -> Snapshot {
    let entities = world
        .query::<(hecs::Entity, Option<&SceneId>, Option<&NetId>, &Transform)>()
        .iter()
        .filter(|(entity, _, _, _)| owner_of(world, *entity) == me)
        .filter_map(|(entity, scene, net, transform)| {
            Some(EntityState {
                id: net.map(|n| n.0).or(scene.map(|s| s.0))?,
                transform: *transform,
                components: components.write_networked(world, entity),
            })
        })
        .collect();
    Snapshot { from: me, entities }
}

/// Snapshots that carry only what changed. A peer usually owns much that
/// stands still — the scene it loaded — and sending all of it ten times a
/// second is most of the bandwidth for none of the news. Every
/// `keyframe`-th snapshot is whole anyway: snapshots travel unreliably, and
/// a peer that lost the one carrying a change gets it back within a
/// keyframe.
pub struct Delta {
    sent: HashMap<EntityId, EntityState>,
    count: u32,
    keyframe: u32,
}

impl Delta {
    pub fn new(keyframe: u32) -> Self {
        Self {
            sent: HashMap::new(),
            count: 0,
            keyframe: keyframe.max(1),
        }
    }

    /// What `me` owns that changed since the last snapshot — or all of it,
    /// on a keyframe.
    pub fn snapshot(
        &mut self,
        world: &hecs::World,
        components: &Components,
        me: PeerId,
    ) -> Snapshot {
        let whole = snapshot(world, components, me);
        let keyframe = self.count % self.keyframe == 0;
        self.count = self.count.wrapping_add(1);
        let entities = whole
            .entities
            .into_iter()
            .filter(|state| keyframe || self.sent.get(&state.id) != Some(state))
            .collect::<Vec<_>>();
        for state in &entities {
            self.sent.insert(state.id, state.clone());
        }
        Snapshot { from: me, entities }
    }
}

/// Every entity a peer can name over the network: the scene's by
/// [`SceneId`], the run-time ones by [`NetId`].
fn addressable(world: &hecs::World) -> HashMap<EntityId, hecs::Entity> {
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

/// Mark an entity the game just spawned as networked: a fresh [`NetId`],
/// owned by `me`, and the [`Spawn`] to send so the others spawn it too.
pub fn announce(
    world: &mut hecs::World,
    entity: hecs::Entity,
    me: PeerId,
    prefab: &str,
) -> Message {
    let id = EntityId::fresh();
    let transform = world
        .get::<&Transform>(entity)
        .map(|t| *t)
        .unwrap_or_default();
    let _ = world.insert(
        entity,
        (NetId(id), Owner(me), NetPrefab(prefab.to_string())),
    );
    Message::Spawn(Spawn {
        from: me,
        id,
        prefab: prefab.to_string(),
        transform,
    })
}

/// What applying a message did, and what it refused.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Applied {
    pub updated: usize,
    /// Refusals in words: a write to something the sender does not own, a
    /// component nobody registered, an entity nobody has.
    pub refused: Vec<String>,
}

/// Take a message from another peer into this world — only what its sender
/// has the right to say. This form refuses a [`Spawn`]; see [`apply_with`].
pub fn apply(world: &mut hecs::World, components: &Components, message: &Message) -> Applied {
    apply_with(world, components, message, |_, _, _| None)
}

/// [`apply`], with `spawn` to put a prefab into the world at a transform —
/// [`crate::LiveScene::spawn_prefab`], usually — for a peer's [`Spawn`].
pub fn apply_with(
    world: &mut hecs::World,
    components: &Components,
    message: &Message,
    mut spawn: impl FnMut(&mut hecs::World, &str, Transform) -> Option<hecs::Entity>,
) -> Applied {
    let by_id = addressable(world);
    let mut out = Applied::default();
    match message {
        Message::Snapshot(snapshot) => {
            for state in &snapshot.entities {
                let Some(&entity) = by_id.get(&state.id) else {
                    out.refused
                        .push(format!("{}: no such entity here", state.id));
                    continue;
                };
                let owner = owner_of(world, entity);
                if owner != snapshot.from {
                    out.refused.push(format!(
                        "{}: peer {} sent it, but peer {} owns it",
                        state.id, snapshot.from.0, owner.0
                    ));
                    continue;
                }
                // Smoothed if the game asked for it on this entity: from
                // where it is now to where its owner says, over the time a
                // snapshot takes. Otherwise it is simply there.
                let smoothed = world.get::<&Remote>(entity).ok().map(|r| *r);
                match smoothed {
                    Some(remote) => {
                        let now = world
                            .get::<&Transform>(entity)
                            .map(|t| *t)
                            .unwrap_or_default();
                        let interval = if remote.elapsed > 0.0 {
                            remote.elapsed
                        } else {
                            remote.interval
                        };
                        let _ = world.insert_one(
                            entity,
                            Remote {
                                from: now,
                                to: state.transform,
                                elapsed: 0.0,
                                interval: interval.clamp(1.0 / 120.0, 1.0),
                            },
                        );
                    }
                    None => {
                        let _ = world.insert_one(entity, state.transform);
                    }
                }
                for (name, text) in &state.components {
                    if !components.is_networked(name) {
                        out.refused.push(format!(
                            "{}: `{name}` is not a networked component here",
                            state.id
                        ));
                        continue;
                    }
                    if let Err(e) = components.insert_text(name, text, world, entity) {
                        out.refused.push(format!("{}: `{name}`: {e}", state.id));
                    }
                }
                out.updated += 1;
            }
            crate::world::apply_hierarchy(world);
        }
        Message::Handover(handover) => match by_id.get(&handover.id) {
            Some(&entity) if owner_of(world, entity) == handover.from => {
                let _ = world.insert_one(entity, Owner(handover.to));
                out.updated += 1;
            }
            Some(&entity) => out.refused.push(format!(
                "{}: peer {} handed it over, but peer {} owns it",
                handover.id,
                handover.from.0,
                owner_of(world, entity).0
            )),
            None => out
                .refused
                .push(format!("{}: no such entity here", handover.id)),
        },
        Message::Spawn(announced) => {
            if by_id.contains_key(&announced.id) {
                out.refused.push(format!("{}: already here", announced.id));
            } else {
                match spawn(world, &announced.prefab, announced.transform) {
                    Some(entity) => {
                        let _ = world.insert(
                            entity,
                            (
                                NetId(announced.id),
                                Owner(announced.from),
                                NetPrefab(announced.prefab.clone()),
                            ),
                        );
                        out.updated += 1;
                    }
                    None => out.refused.push(format!(
                        "{}: no prefab `{}` to spawn here",
                        announced.id, announced.prefab
                    )),
                }
            }
        }
        Message::Welcome(welcome) => {
            if welcome.from != PeerId::HOST {
                out.refused.push(format!(
                    "a welcome from peer {}: only the host knows the whole game",
                    welcome.from.0
                ));
                return out;
            }
            for announced in &welcome.spawns {
                if by_id.contains_key(&announced.id) {
                    continue;
                }
                match spawn(world, &announced.prefab, announced.transform) {
                    Some(entity) => {
                        let _ = world.insert(
                            entity,
                            (
                                NetId(announced.id),
                                Owner(announced.from),
                                NetPrefab(announced.prefab.clone()),
                            ),
                        );
                        out.updated += 1;
                    }
                    None => out.refused.push(format!(
                        "{}: no prefab `{}` to spawn here",
                        announced.id, announced.prefab
                    )),
                }
            }
            let by_id = addressable(world);
            for (id, owner) in &welcome.owners {
                match by_id.get(id) {
                    Some(&entity) => {
                        let _ = world.insert_one(entity, Owner(*owner));
                        out.updated += 1;
                    }
                    None => out.refused.push(format!("{id}: no such entity here")),
                }
            }
        }
        Message::Despawn(gone) => match by_id.get(&gone.id) {
            Some(&entity) if world.get::<&NetId>(entity).is_err() => out.refused.push(format!(
                "{}: the scene's, not something spawned to despawn",
                gone.id
            )),
            Some(&entity) if owner_of(world, entity) == gone.from => {
                despawn_tree(world, entity);
                out.updated += 1;
            }
            Some(&entity) => out.refused.push(format!(
                "{}: peer {} despawned it, but peer {} owns it",
                gone.id,
                gone.from.0,
                owner_of(world, entity).0
            )),
            None => out
                .refused
                .push(format!("{}: no such entity here", gone.id)),
        },
    }
    out
}

/// Everything a peer joining now needs, from the host's world: a [`Spawn`]
/// for each run-time entity, as its owner, and who owns what where it is
/// not the host. Send it to the newcomer, who loads the scene itself and
/// then takes this with [`apply_with`]; from there, each owner's snapshots
/// reach it like anyone else's.
pub fn welcome(world: &hecs::World) -> Message {
    let mut spawns: Vec<Spawn> = world
        .query::<(hecs::Entity, &NetId, &NetPrefab, &Transform)>()
        .iter()
        .map(|(entity, id, prefab, transform)| Spawn {
            from: owner_of(world, entity),
            id: id.0,
            prefab: prefab.0.clone(),
            transform: *transform,
        })
        .collect();
    spawns.sort_by_key(|s| s.id);
    let mut owners: Vec<(EntityId, PeerId)> = addressable(world)
        .into_iter()
        .map(|(id, entity)| (id, owner_of(world, entity)))
        .filter(|(_, owner)| *owner != PeerId::HOST)
        .collect();
    owners.sort();
    Message::Welcome(Welcome {
        from: PeerId::HOST,
        spawns,
        owners,
    })
}

/// A peer has left: what it owned passes to one who stayed, so the game
/// goes on with its crates and its torches rather than freezing them where
/// they were. Every remaining peer calls this with the same arguments and
/// gets the same answer without a message: the new owner is the lowest
/// peer still here — the host, while the host is. When the host is the one
/// who left, what nobody owned explicitly (the host's by default) goes the
/// same way. Returns what changed hands.
pub fn peer_left(world: &mut hecs::World, gone: PeerId, remaining: &[PeerId]) -> Vec<EntityId> {
    let Some(&heir) = remaining.iter().filter(|p| **p != gone).min() else {
        return Vec::new();
    };
    let orphans: Vec<(EntityId, hecs::Entity)> = addressable(world)
        .into_iter()
        .filter(|(_, entity)| owner_of(world, *entity) == gone)
        .collect();
    let mut moved: Vec<EntityId> = orphans.iter().map(|(id, _)| *id).collect();
    for (_, entity) in orphans {
        let _ = world.insert_one(entity, Owner(heir));
    }
    moved.sort();
    moved
}

/// An entity and everything parented to it.
fn despawn_tree(world: &mut hecs::World, root: hecs::Entity) {
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

/// The wire: send to one peer, take what arrived.
pub trait Transport {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>);
    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)>;
}

/// Peers over UDP: each datagram is the sender's [`PeerId`] and a
/// [`Message`]. Unreliable and unordered, which is what snapshots want — a
/// late one is superseded by the next. Handovers and spawns want delivery
/// guarantees this does not give yet; a reliable channel is the next step.
pub struct Udp {
    socket: std::net::UdpSocket,
    me: PeerId,
    peers: HashMap<PeerId, std::net::SocketAddr>,
}

impl Udp {
    /// Bind to `address` (`"0.0.0.0:7777"`, or port 0 for any) as `me`.
    pub fn bind(address: &str, me: PeerId) -> std::io::Result<Self> {
        let socket = std::net::UdpSocket::bind(address)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            me,
            peers: HashMap::new(),
        })
    }

    pub fn local_address(&self) -> std::io::Result<std::net::SocketAddr> {
        self.socket.local_addr()
    }

    /// Where a peer is.
    pub fn connect(&mut self, peer: PeerId, address: std::net::SocketAddr) {
        self.peers.insert(peer, address);
    }
}

impl Transport for Udp {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>) {
        let Some(address) = self.peers.get(&to) else {
            return;
        };
        let mut datagram = self.me.0.to_le_bytes().to_vec();
        datagram.extend_from_slice(&bytes);
        let _ = self.socket.send_to(&datagram, address);
    }

    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        let mut out = Vec::new();
        let mut buffer = vec![0u8; 65_536];
        while let Ok((length, address)) = self.socket.recv_from(&mut buffer) {
            if length < 4 {
                continue;
            }
            let from = PeerId(u32::from_le_bytes([
                buffer[0], buffer[1], buffer[2], buffer[3],
            ]));
            // Learn where a peer is from what it sends: a client need not be
            // configured on the host.
            self.peers.entry(from).or_insert(address);
            out.push((from, buffer[4..length].to_vec()));
        }
        out
    }
}

/// Delivery for what must arrive — handovers, spawns, despawns — over any
/// transport. Each reliable message carries a sequence number and is sent
/// again every [`Reliable::RESEND`] until the peer acknowledges it; a
/// receiver acknowledges everything and passes each sequence on once.
/// Snapshots go through as they are: a lost one is superseded by the next,
/// and resending it would only deliver the past.
pub struct Reliable<T: Transport> {
    inner: T,
    next: u64,
    /// Unacknowledged: to whom, sequence, the framed bytes, and when last sent.
    waiting: Vec<(PeerId, u64, Vec<u8>, std::time::Instant)>,
    /// What each peer has already delivered, so a resend is not a repeat.
    seen: HashMap<PeerId, std::collections::HashSet<u64>>,
}

impl<T: Transport> Reliable<T> {
    /// How long before an unacknowledged message is sent again.
    pub const RESEND: std::time::Duration = std::time::Duration::from_millis(100);

    pub fn new(inner: T) -> Self {
        Self {
            inner,
            next: 1,
            waiting: Vec::new(),
            seen: HashMap::new(),
        }
    }

    /// Send a message the way its kind wants: snapshots as they are,
    /// everything else until acknowledged.
    pub fn send_message(&mut self, to: PeerId, message: &Message) {
        let body = message.encode();
        if matches!(message, Message::Snapshot(_)) {
            let mut framed = vec![0u8];
            framed.extend_from_slice(&body);
            self.inner.send(to, framed);
            return;
        }
        let sequence = self.next;
        self.next += 1;
        let mut framed = vec![1u8];
        framed.extend_from_slice(&sequence.to_le_bytes());
        framed.extend_from_slice(&body);
        self.inner.send(to, framed.clone());
        self.waiting
            .push((to, sequence, framed, std::time::Instant::now()));
    }

    /// Resend what is overdue, take in what arrived, and hand back the
    /// messages — each reliable one exactly once. Call it every frame.
    pub fn pump(&mut self) -> Vec<(PeerId, Message)> {
        let now = std::time::Instant::now();
        for (to, _, framed, sent) in &mut self.waiting {
            if now.duration_since(*sent) >= Self::RESEND {
                self.inner.send(*to, framed.clone());
                *sent = now;
            }
        }
        let mut out = Vec::new();
        for (from, bytes) in self.inner.receive() {
            match bytes.first() {
                Some(0) => {
                    if let Ok(message) = Message::decode(&bytes[1..]) {
                        out.push((from, message));
                    }
                }
                Some(1) if bytes.len() >= 9 => {
                    let sequence = u64::from_le_bytes(bytes[1..9].try_into().expect("eight bytes"));
                    let mut ack = vec![2u8];
                    ack.extend_from_slice(&sequence.to_le_bytes());
                    self.inner.send(from, ack);
                    if self.seen.entry(from).or_default().insert(sequence) {
                        if let Ok(message) = Message::decode(&bytes[9..]) {
                            out.push((from, message));
                        }
                    }
                }
                Some(2) if bytes.len() >= 9 => {
                    let sequence = u64::from_le_bytes(bytes[1..9].try_into().expect("eight bytes"));
                    self.waiting
                        .retain(|(to, s, _, _)| !(*to == from && *s == sequence));
                }
                _ => {}
            }
        }
        out
    }

    /// How many reliable messages are still waiting for an acknowledgement.
    pub fn unacknowledged(&self) -> usize {
        self.waiting.len()
    }
}

/// Where a remote-owned entity is heading, and from where: snapshots arrive
/// a few times a second, and an entity that jumped to each would stutter.
/// [`apply`] sets it for entities owned by others; [`smooth`] moves them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Remote {
    from: Transform,
    to: Transform,
    elapsed: f32,
    interval: f32,
}

/// Move every [`Remote`] entity along, from where it was to where its
/// owner last said, over the time between snapshots. Call it every frame.
pub fn smooth(world: &mut hecs::World, delta: f32) {
    for (transform, remote) in world.query_mut::<(&mut Transform, &mut Remote)>() {
        remote.elapsed += delta;
        let t = (remote.elapsed / remote.interval.max(1e-3)).min(1.0);
        transform.position = remote.from.position.lerp(remote.to.position, t);
        transform.rotation_deg = remote.from.rotation_deg.lerp(remote.to.rotation_deg, t);
        transform.scale = remote.from.scale.lerp(remote.to.scale, t);
    }
    crate::world::apply_hierarchy(world);
}

/// What each peer has waiting: who sent it, and the bytes.
type Inboxes = HashMap<PeerId, VecDeque<(PeerId, Vec<u8>)>>;

/// Peers in one process, for tests and for a game running host and client
/// side by side.
#[derive(Clone)]
pub struct Loopback {
    me: PeerId,
    queues: std::sync::Arc<std::sync::Mutex<Inboxes>>,
}

impl Loopback {
    /// A network of `count` peers, 0 to `count - 1`, one end each.
    pub fn network(count: u32) -> Vec<Loopback> {
        let queues = std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
        (0..count)
            .map(|i| Loopback {
                me: PeerId(i),
                queues: queues.clone(),
            })
            .collect()
    }
}

impl Transport for Loopback {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>) {
        if let Ok(mut queues) = self.queues.lock() {
            queues.entry(to).or_default().push_back((self.me, bytes));
        }
    }

    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        self.queues
            .lock()
            .map(|mut queues| queues.entry(self.me).or_default().drain(..).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::MeshHandle;
    use crate::scene::Scene;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Lit(bool);

    /// A secret no peer should ever receive.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Secret(u32);

    fn components() -> Components {
        let mut c = Components::new();
        c.register_networked::<Lit>("lit")
            .register::<Secret>("secret");
        c
    }

    const CAMP: &str = r#"(entities: [
        (id: "a1", name: "fire", model: "m", components: { "lit": (false), "secret": (7) }),
        (id: "b2", name: "crate", model: "m"),
    ])"#;

    /// A peer that loaded the scene.
    fn peer() -> (hecs::World, Components) {
        let mut scene: Scene = ron::from_str(CAMP).unwrap();
        scene.assign_ids();
        let mut world = hecs::World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        let components = components();
        components.apply(&scene, &mut world);
        (world, components)
    }

    fn entity(world: &hecs::World, id: &str) -> hecs::Entity {
        let id: EntityId = id.parse().unwrap();
        world
            .query::<(hecs::Entity, &SceneId)>()
            .iter()
            .find(|(_, s)| s.0 == id)
            .map(|(e, _)| e)
            .unwrap()
    }

    #[test]
    fn what_the_owner_changes_reaches_the_others_and_only_the_networked_part() {
        let (host, components) = peer();
        let (mut client, _) = peer();
        let mut wire = Loopback::network(2);

        // The host lights the fire and moves it.
        let fire = entity(&host, "a1");
        *host.get::<&mut Lit>(fire).unwrap() = Lit(true);
        host.get::<&mut Transform>(fire).unwrap().position.x = 4.0;
        host.get::<&mut Secret>(fire).unwrap().0 = 99;
        let message = Message::Snapshot(snapshot(&host, &components, PeerId::HOST));
        wire[0].send(PeerId(1), message.encode());

        for (_, bytes) in wire[1].receive() {
            let done = apply(&mut client, &components, &Message::decode(&bytes).unwrap());
            assert!(done.refused.is_empty(), "{:?}", done.refused);
        }
        let fire = entity(&client, "a1");
        assert_eq!(*client.get::<&Lit>(fire).unwrap(), Lit(true));
        assert_eq!(client.get::<&Transform>(fire).unwrap().position.x, 4.0);
        assert_eq!(
            client.get::<&Secret>(fire).unwrap().0,
            7,
            "local stays local"
        );
    }

    #[test]
    fn a_peer_cannot_move_what_it_does_not_own_until_it_is_handed_over() {
        let (mut host, components) = peer();
        let (client, _) = peer();

        // The client tries to move the host's crate: refused.
        let crate_ = entity(&client, "b2");
        client.get::<&mut Transform>(crate_).unwrap().position.y = 9.0;
        let mut attempt = snapshot(&client, &components, PeerId::HOST);
        attempt.from = PeerId(1);
        let id: EntityId = "b2".parse().unwrap();
        attempt.entities.retain(|e| e.id == id);
        let done = apply(&mut host, &components, &Message::Snapshot(attempt.clone()));
        assert!(
            done.refused.iter().any(|r| r.contains("peer 0 owns it")),
            "{:?}",
            done.refused
        );
        assert_eq!(
            host.get::<&Transform>(entity(&host, "b2"))
                .unwrap()
                .position
                .y,
            0.0
        );

        // A handover from someone who does not own it: refused too.
        let forged = Message::Handover(Handover {
            from: PeerId(1),
            id,
            to: PeerId(1),
        });
        assert_eq!(apply(&mut host, &components, &forged).updated, 0);

        // The host hands it over; now the client's word counts.
        let handover = Message::Handover(Handover {
            from: PeerId::HOST,
            id,
            to: PeerId(1),
        });
        apply(&mut host, &components, &handover);
        let done = apply(&mut host, &components, &Message::Snapshot(attempt));
        assert!(done.refused.is_empty(), "{:?}", done.refused);
        assert_eq!(
            host.get::<&Transform>(entity(&host, "b2"))
                .unwrap()
                .position
                .y,
            9.0
        );
    }

    #[test]
    fn a_spawned_prefab_appears_on_the_other_peer_and_its_owner_can_take_it_away() {
        let (mut host, components) = peer();
        let (mut client, _) = peer();

        // The host spawns a torch at run time and announces it.
        let torch = host.spawn((Transform {
            position: glam::Vec3::new(2.0, 0.0, 0.0),
            ..Transform::default()
        },));
        let spawn = announce(&mut host, torch, PeerId::HOST, "torch");
        let mut spawned_here = None;
        let done = apply_with(&mut client, &components, &spawn, |world, prefab, t| {
            let e = (prefab == "torch").then(|| world.spawn((t,)));
            spawned_here = e;
            e
        });
        assert_eq!(done.updated, 1, "{:?}", done.refused);
        let copy = spawned_here.unwrap();
        assert_eq!(client.get::<&Transform>(copy).unwrap().position.x, 2.0);
        assert_eq!(owner_of(&client, copy), PeerId::HOST);

        // It moves in the host's snapshots like anything else it owns.
        host.get::<&mut Transform>(torch).unwrap().position.x = 5.0;
        let moved = Message::Snapshot(snapshot(&host, &components, PeerId::HOST));
        apply(&mut client, &components, &moved);
        assert_eq!(client.get::<&Transform>(copy).unwrap().position.x, 5.0);

        // Only its owner despawns it, and a scene entity is not despawnable.
        let Message::Spawn(Spawn { id, .. }) = spawn else {
            unreachable!()
        };
        let forged = Message::Despawn(Despawn {
            from: PeerId(1),
            id,
        });
        assert_eq!(apply(&mut client, &components, &forged).updated, 0);
        let scene_crate: EntityId = "b2".parse().unwrap();
        let scene = Message::Despawn(Despawn {
            from: PeerId::HOST,
            id: scene_crate,
        });
        assert_eq!(apply(&mut client, &components, &scene).updated, 0);
        let real = Message::Despawn(Despawn {
            from: PeerId::HOST,
            id,
        });
        apply(&mut client, &components, &real);
        assert!(!client.contains(copy));
    }

    fn wait(transport: &mut impl Transport) -> Vec<(PeerId, Vec<u8>)> {
        for _ in 0..200 {
            let got = transport.receive();
            if !got.is_empty() {
                return got;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Vec::new()
    }

    #[test]
    fn peers_talk_over_udp() {
        let (Ok(mut host), Ok(mut client)) = (
            Udp::bind("127.0.0.1:0", PeerId::HOST),
            Udp::bind("127.0.0.1:0", PeerId(1)),
        ) else {
            eprintln!("skipping: no loopback sockets");
            return;
        };
        client.connect(PeerId::HOST, host.local_address().unwrap());
        let hello = Message::Handover(Handover {
            from: PeerId(1),
            id: EntityId::from_raw(7),
            to: PeerId::HOST,
        });
        client.send(PeerId::HOST, hello.encode());
        let got = wait(&mut host);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, PeerId(1));
        assert_eq!(Message::decode(&got[0].1).unwrap(), hello);

        // The host learned where the client is from the datagram.
        host.send(PeerId(1), hello.encode());
        assert_eq!(wait(&mut client).len(), 1);
    }

    /// A loopback that loses every other datagram.
    struct Lossy {
        inner: Loopback,
        drop: bool,
    }

    impl Transport for Lossy {
        fn send(&mut self, to: PeerId, bytes: Vec<u8>) {
            self.drop = !self.drop;
            if !self.drop {
                self.inner.send(to, bytes);
            }
        }
        fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
            self.inner.receive()
        }
    }

    #[test]
    fn a_handover_arrives_once_over_a_line_that_loses_half_of_everything() {
        let mut ends = Loopback::network(2).into_iter();
        let mut host = Reliable::new(Lossy {
            inner: ends.next().unwrap(),
            drop: false,
        });
        let mut client = Reliable::new(Lossy {
            inner: ends.next().unwrap(),
            drop: false,
        });
        let handover = Message::Handover(Handover {
            from: PeerId::HOST,
            id: EntityId::from_raw(5),
            to: PeerId(1),
        });
        host.send_message(PeerId(1), &handover);
        let mut delivered = Vec::new();
        for _ in 0..40 {
            delivered.extend(client.pump());
            host.pump();
            std::thread::sleep(Reliable::<Loopback>::RESEND / 4);
        }
        assert_eq!(delivered.len(), 1, "exactly once: {delivered:?}");
        assert_eq!(delivered[0].1, handover);
        assert_eq!(host.unacknowledged(), 0, "and the host knows it arrived");
    }

    #[test]
    fn a_smoothed_remote_entity_glides_to_where_its_owner_says() {
        let (host, components) = peer();
        let (mut client, _) = peer();
        let fire = entity(&client, "a1");
        smooth_this(&mut client, fire);
        let mut moved = snapshot(&host, &components, PeerId::HOST);
        for state in &mut moved.entities {
            state.transform.position.x = 10.0;
        }
        apply(&mut client, &components, &Message::Snapshot(moved));
        assert_eq!(
            client.get::<&Transform>(fire).unwrap().position.x,
            0.0,
            "not a jump"
        );
        smooth(&mut client, 0.05);
        let halfway = client.get::<&Transform>(fire).unwrap().position.x;
        assert!((halfway - 5.0).abs() < 0.01, "{halfway}");
        smooth(&mut client, 0.2);
        assert_eq!(client.get::<&Transform>(fire).unwrap().position.x, 10.0);
    }

    #[test]
    fn a_delta_sends_what_moved_and_everything_on_a_keyframe() {
        let (host, components) = peer();
        let mut delta = Delta::new(10);
        let first = delta.snapshot(&host, &components, PeerId::HOST);
        assert_eq!(first.entities.len(), 2, "the first is whole");
        assert!(
            delta
                .snapshot(&host, &components, PeerId::HOST)
                .entities
                .is_empty(),
            "nothing moved"
        );

        let fire = entity(&host, "a1");
        host.get::<&mut Transform>(fire).unwrap().position.x = 1.0;
        let moved = delta.snapshot(&host, &components, PeerId::HOST);
        assert_eq!(moved.entities.len(), 1);
        assert_eq!(moved.entities[0].id, "a1".parse().unwrap());
        // Snapshots 0 (whole), 1 and 2 were sent; 3 to 9 carry nothing new,
        // and 10 is the next keyframe.
        for _ in 3..10 {
            delta.snapshot(&host, &components, PeerId::HOST);
        }
        assert_eq!(
            delta
                .snapshot(&host, &components, PeerId::HOST)
                .entities
                .len(),
            2,
            "keyframe"
        );
    }

    #[test]
    fn a_peer_that_leaves_leaves_its_things_to_the_lowest_one_left() {
        // Peer 2 had taken the crate and spawned a torch.
        let mut worlds: Vec<hecs::World> = (0..3).map(|_| peer().0).collect();
        let torch_id = {
            let world = &mut worlds[2];
            let torch = world.spawn((Transform::default(),));
            let Message::Spawn(spawn) = announce(world, torch, PeerId(2), "torch") else {
                unreachable!()
            };
            spawn.id
        };
        for world in &mut worlds {
            let crate_ = entity(world, "b2");
            world.insert_one(crate_, Owner(PeerId(2))).unwrap();
        }
        let torch_at_1 = worlds[1].spawn((Transform::default(), NetId(torch_id), Owner(PeerId(2))));

        // Peer 2 is gone: 0 and 1 each work out the same heir, the host.
        for world in &mut worlds[..2] {
            let moved = peer_left(world, PeerId(2), &[PeerId(0), PeerId(1)]);
            assert_eq!(moved.len(), 1 + usize::from(world.contains(torch_at_1)));
            assert_eq!(owner_of(world, entity(world, "b2")), PeerId::HOST);
        }
        assert_eq!(owner_of(&worlds[1], torch_at_1), PeerId::HOST);

        // Then the host goes: everything the host held — explicitly or by
        // default — passes to peer 1, the lowest left.
        let moved = peer_left(&mut worlds[1], PeerId::HOST, &[PeerId(1)]);
        assert_eq!(moved.len(), 3, "fire, crate and torch");
        assert_eq!(owner_of(&worlds[1], entity(&worlds[1], "a1")), PeerId(1));
        assert!(
            peer_left(&mut worlds[1], PeerId(1), &[PeerId(1)]).is_empty(),
            "nobody left to inherit"
        );
    }

    #[test]
    fn a_peer_joining_mid_game_is_told_what_was_spawned_and_who_owns_what() {
        let (mut host, components) = peer();
        // Peer 1 took the crate, and the host spawned a torch.
        let crate_ = entity(&host, "b2");
        host.insert_one(crate_, Owner(PeerId(1))).unwrap();
        let torch = host.spawn((Transform {
            position: glam::Vec3::new(3.0, 0.0, 0.0),
            ..Transform::default()
        },));
        announce(&mut host, torch, PeerId::HOST, "torch");

        let (mut late, _) = peer();
        let hello = Message::decode(&welcome(&host).encode()).unwrap();
        let spawn = |world: &mut hecs::World, prefab: &str, t: Transform| {
            (prefab == "torch").then(|| world.spawn((t,)))
        };
        let done = apply_with(&mut late, &components, &hello, spawn);
        assert!(done.refused.is_empty(), "{:?}", done.refused);
        let torch_here = late
            .query::<(hecs::Entity, &NetPrefab)>()
            .iter()
            .map(|(e, _)| e)
            .next()
            .expect("the torch was spawned here too");
        assert_eq!(late.get::<&Transform>(torch_here).unwrap().position.x, 3.0);
        assert_eq!(owner_of(&late, entity(&late, "b2")), PeerId(1));

        // So peer 1's word about the crate counts here now.
        let (mut one, _) = peer();
        let crate_at_one = entity(&one, "b2");
        one.insert_one(crate_at_one, Owner(PeerId(1))).unwrap();
        one.get::<&mut Transform>(crate_at_one).unwrap().position.z = 6.0;
        let moved = Message::Snapshot(snapshot(&one, &components, PeerId(1)));
        assert_eq!(apply(&mut late, &components, &moved).updated, 1);
        assert_eq!(
            late.get::<&Transform>(entity(&late, "b2"))
                .unwrap()
                .position
                .z,
            6.0
        );

        // A welcome from anyone but the host is refused.
        let Message::Welcome(mut forged) = welcome(&host) else {
            unreachable!()
        };
        forged.from = PeerId(1);
        let done = apply(&mut late, &components, &Message::Welcome(forged));
        assert_eq!(done.updated, 0);
        assert!(
            done.refused[0].contains("only the host"),
            "{:?}",
            done.refused
        );
    }
}
