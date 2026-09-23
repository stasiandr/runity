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

/// What goes over the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    Snapshot(Snapshot),
    Handover(Handover),
    Spawn(Spawn),
    Despawn(Despawn),
}

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
    let _ = world.insert(entity, (NetId(id), Owner(me)));
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
                let _ = world.insert_one(entity, state.transform);
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
                        let _ = world.insert(entity, (NetId(announced.id), Owner(announced.from)));
                        out.updated += 1;
                    }
                    None => out.refused.push(format!(
                        "{}: no prefab `{}` to spawn here",
                        announced.id, announced.prefab
                    )),
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
}
