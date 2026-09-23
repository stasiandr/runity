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
        .query::<(hecs::Entity, &SceneId, &Transform)>()
        .iter()
        .filter(|(entity, _, _)| owner_of(world, *entity) == me)
        .map(|(entity, id, transform)| EntityState {
            id: id.0,
            transform: *transform,
            components: components.write_networked(world, entity),
        })
        .collect();
    Snapshot { from: me, entities }
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
/// has the right to say.
pub fn apply(world: &mut hecs::World, components: &Components, message: &Message) -> Applied {
    let by_id: HashMap<EntityId, hecs::Entity> = world
        .query::<(hecs::Entity, &SceneId)>()
        .iter()
        .map(|(entity, id)| (id.0, entity))
        .collect();
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
    }
    out
}

/// The wire: send to one peer, take what arrived.
pub trait Transport {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>);
    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)>;
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
}
