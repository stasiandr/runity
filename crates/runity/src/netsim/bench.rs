//! The simulations' bench (docs/netsim.md): a scene played alone, then by
//! several peers in one process over a link as bad as asked, every peer
//! running the whole fixed step — network, physics, soft things — and
//! what each of them shows recorded, so a test can hold the session
//! against the solo run and the peers against each other.
//!
//! The network module's own bench (`runity::bench`) does this for bodies
//! alone; this one runs the facade's step, where the modules meet.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hecs::World;

use crate::components::Components;
use crate::net::wire::{Conditions, Laggy, Loopback, Transport};
use crate::net::PeerId;
use crate::party::{Event, Party};
use crate::physics::PhysicsWorld;
use crate::render::MeshHandle;
use crate::scene::Scene;
use crate::EntityId;

/// Fixed steps a second: a network tick each.
pub const HZ: f32 = 30.0;

/// One participant: its party, its world, its physics.
pub struct Peer {
    pub party: Party,
    pub world: World,
    pub physics: PhysicsWorld,
    /// Problems the party reported.
    pub problems: Vec<String>,
}

impl Peer {
    fn new(party: Party, scene: &Scene) -> Self {
        let mut world = World::new();
        crate::spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        crate::world::apply_hierarchy(&mut world);
        let mut physics = PhysicsWorld::new(1.0 / HZ);
        physics.sync_from_world(&mut world);
        Self { party, world, physics, problems: Vec::new() }
    }

    /// The entity a scene line or a spawn names.
    pub fn entity(&self, id: u64) -> Option<hecs::Entity> {
        crate::net::addressable(&self.world).get(&EntityId::from_raw(id)).copied()
    }

    /// One fixed step, the order a game's tick has it: what the network
    /// says, the physics, then the soft things held by the bodies, and
    /// what they did back to the bodies for the next step.
    pub fn step(&mut self, components: &Components) {
        let dt = 1.0 / HZ;
        for event in self.party.update(&mut self.world, components, dt, |_, _, _| None) {
            if let Event::Problem(problem) = event {
                self.problems.push(problem);
            }
        }
        crate::world::apply_hierarchy(&mut self.world);
        self.physics.run(&mut self.world);
        crate::world::apply_hierarchy(&mut self.world);
        #[cfg(feature = "soft")]
        {
            super::anchor_ropes(&mut self.world, &mut self.physics);
            crate::soft::step(&mut self.world, dt);
            super::pull_bodies(&mut self.world, &mut self.physics);
        }
    }
}

/// Counts what goes through a transport.
struct Metered<T> {
    inner: T,
    sent: Arc<AtomicU64>,
}

impl<T: Transport> Transport for Metered<T> {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>) {
        self.sent.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        self.inner.send(to, bytes);
    }

    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        self.inner.receive()
    }
}

/// Everyone in one process: the host first, then the guests over the
/// link.
pub struct Session {
    pub peers: Vec<Peer>,
    pub components: Components,
    /// Bytes the guests sent, and the server sent them.
    pub sent: Arc<AtomicU64>,
    pub served: Arc<AtomicU64>,
    networked: bool,
    clock: Instant,
    /// Ticks played.
    pub tick: usize,
}

impl Session {
    /// `peers` in a session over `link` (the host's own is perfect: it is
    /// in the server's process).
    pub fn new(scene: &Scene, peers: usize, link: Conditions, seed: u64) -> Self {
        let mut components = Components::new();
        super::register(&mut components);
        let sent = Arc::new(AtomicU64::new(0));
        let served = Arc::new(AtomicU64::new(0));
        if peers <= 1 {
            let party = Party::alone("netsim", &components);
            return Self {
                peers: vec![Peer::new(party, scene)],
                components,
                sent,
                served,
                networked: false,
                clock: Instant::now(),
                tick: 0,
            };
        }
        let mut ends = Loopback::network(peers as u32).into_iter();
        let listener = ends.next().expect("the host's end");
        let host = Party::host(
            "netsim",
            "host",
            &components,
            vec![Box::new(Metered { inner: listener, sent: served.clone() })],
            false,
        );
        let mut all = vec![Peer::new(host, scene)];
        for (i, end) in ends.enumerate() {
            let lagged: Box<dyn Transport + Send> = if link == Conditions::GOOD {
                Box::new(Metered { inner: end, sent: sent.clone() })
            } else {
                Box::new(Metered { inner: Laggy::new(end, link, seed * 101 + i as u64), sent: sent.clone() })
            };
            all.push(Peer::new(Party::join(lagged, "netsim", &format!("guest{}", i + 1), &components), scene));
        }
        Self { peers: all, components, sent, served, networked: true, clock: Instant::now(), tick: 0 }
    }

    /// One tick for everyone, at the pace of real time when there is a
    /// link (its delays are the clock's).
    pub fn step(&mut self) {
        for peer in &mut self.peers {
            peer.step(&self.components);
        }
        self.tick += 1;
        if self.networked {
            self.clock += Duration::from_secs_f32(1.0 / HZ);
            let now = Instant::now();
            if self.clock > now {
                std::thread::sleep(self.clock - now);
            } else if now - self.clock > Duration::from_millis(200) {
                // Fell far behind (a slow machine): time is what it is.
                self.clock = now;
            }
        }
    }

    /// Ticks until everyone is in, at most eight seconds' worth; whether
    /// they all are.
    pub fn join(&mut self) -> bool {
        for _ in 0..(8.0 * HZ) as usize {
            self.step();
            if self.peers.iter().all(|p| p.party.welcomed()) {
                return true;
            }
        }
        false
    }

    /// Peer `peer` asks to drive entity `id`.
    pub fn claim(&mut self, peer: usize, id: u64) {
        let p = &mut self.peers[peer];
        if let Some(entity) = p.entity(id) {
            p.party.claim(&mut p.world, entity);
        }
    }

    /// Which peer simulates entity `id`, as each peer believes.
    pub fn owners(&self, id: u64) -> Vec<bool> {
        self.peers
            .iter()
            .map(|p| p.entity(id).is_some_and(|e| p.world.get::<&crate::net::Owned>(e).is_ok()))
            .collect()
    }

    /// Every problem any party reported.
    pub fn problems(&self) -> Vec<String> {
        self.peers.iter().flat_map(|p| p.problems.iter().cloned()).collect()
    }
}
