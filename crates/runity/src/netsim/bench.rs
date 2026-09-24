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

/// Fixed steps a second: the game's (`time::Settings::fixed_delta`). The
/// party sends at its own rate, a network tick every other step.
pub const HZ: f32 = 60.0;

/// Steps in `seconds`.
pub fn ticks(seconds: f32) -> usize {
    (seconds * HZ).round() as usize
}

/// One participant: its party, its world, its physics.
pub struct Peer {
    pub party: Party,
    pub world: World,
    pub physics: PhysicsWorld,
    /// Problems the party reported.
    pub problems: Vec<String>,
}

/// What a peer's models are drawn with: `(peer, model)` to a handle.
pub type Meshes<'a> = &'a mut dyn FnMut(usize, &crate::AssetLink) -> Option<MeshHandle>;

impl Peer {
    fn new(party: Party, scene: &Scene, index: usize, meshes: Meshes) -> Self {
        let mut world = World::new();
        crate::spawn_scene(scene, &mut world, |name| meshes(index, name));
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
        self.step_with(components, |_| {});
    }

    /// [`Peer::step`], with what the game does before the physics: a
    /// player pulling on their own body.
    pub fn step_with(&mut self, components: &Components, game: impl FnOnce(&mut Peer)) {
        let dt = 1.0 / HZ;
        for event in self.party.update(&mut self.world, components, dt, |_, _, _| None) {
            if let Event::Problem(problem) = event {
                self.problems.push(problem);
            }
        }
        crate::world::apply_hierarchy(&mut self.world);
        super::claim_approaching(&mut self.world, &self.physics);
        game(self);
        #[cfg(feature = "character")]
        crate::character::step(&mut self.world, &mut self.physics, dt);
        self.physics.run(&mut self.world);
        crate::world::apply_hierarchy(&mut self.world);
        #[cfg(feature = "soft")]
        {
            super::anchor_ropes(&mut self.world, &mut self.physics);
            crate::soft::step(&mut self.world, dt);
            super::pull_bodies(&mut self.world, &mut self.physics);
        }
        #[cfg(feature = "destruction")]
        crate::destruction::step(&mut self.world, &mut self.physics, dt);
    }
}

/// Counts what goes through a transport: all of it, and by the kind of
/// the link's frame (its first byte: unreliable, reliable, ack, ping,
/// goodbye), bytes and datagrams.
struct Metered<T> {
    inner: T,
    sent: Arc<AtomicU64>,
    kinds: Arc<Kinds>,
}

/// Bytes and datagrams by the link's frame kind.
#[derive(Default)]
pub struct Kinds {
    pub bytes: [AtomicU64; 8],
    pub datagrams: [AtomicU64; 8],
    /// Every datagram, while a test keeps them (`Some`).
    pub kept: std::sync::Mutex<Option<Vec<Vec<u8>>>>,
}

impl Kinds {
    /// `(bytes, datagrams)` by kind.
    pub fn read(&self) -> [(u64, u64); 8] {
        std::array::from_fn(|i| (self.bytes[i].load(Ordering::Relaxed), self.datagrams[i].load(Ordering::Relaxed)))
    }
}

impl<T: Transport> Transport for Metered<T> {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>) {
        self.sent.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        let kind = (bytes.first().copied().unwrap_or(7) as usize).min(7);
        self.kinds.bytes[kind].fetch_add(bytes.len() as u64, Ordering::Relaxed);
        self.kinds.datagrams[kind].fetch_add(1, Ordering::Relaxed);
        if let Ok(mut kept) = self.kinds.kept.lock() {
            if let Some(kept) = kept.as_mut() {
                kept.push(bytes.clone());
            }
        }
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
    /// The same by the link's frame kind: the guests', the server's.
    pub sent_kinds: Arc<Kinds>,
    pub served_kinds: Arc<Kinds>,
    networked: bool,
    clock: Instant,
    /// Ticks played.
    pub tick: usize,
    /// Ends of the network kept for peers who join late, with the link
    /// and seed they come in over.
    late: Vec<(Box<dyn Transport + Send>, Scene)>,
}

impl Session {
    /// `peers` in a session over `link` (the host's own is perfect: it is
    /// in the server's process).
    pub fn new(scene: &Scene, peers: usize, link: Conditions, seed: u64) -> Self {
        Self::with_late(scene, peers, 0, link, seed)
    }

    /// [`Session::new`], each peer's models drawn with `meshes`: for film.
    pub fn filmed(scene: &Scene, peers: usize, link: Conditions, seed: u64, meshes: Meshes) -> Self {
        Self::networked(scene, peers.max(2), 0, link, seed, meshes)
    }

    /// [`Session::new`], with `late` more peers who come in later
    /// ([`Session::join_late`]), over the same link.
    pub fn with_late(scene: &Scene, peers: usize, late: usize, link: Conditions, seed: u64) -> Self {
        if peers <= 1 && late == 0 {
            return Self::alone(scene);
        }
        Self::networked(scene, peers.max(1), late, link, seed, &mut |_, _| Some(MeshHandle::TEST))
    }

    fn alone(scene: &Scene) -> Self {
        let mut components = Components::new();
        super::register(&mut components);
        let sent = Arc::new(AtomicU64::new(0));
        let served = Arc::new(AtomicU64::new(0));
        {
            let party = Party::alone("netsim", &components);
            return Self {
                peers: vec![Peer::new(party, scene, 0, &mut |_, _| Some(MeshHandle::TEST))],
                components,
                sent,
                served,
                sent_kinds: Default::default(),
                served_kinds: Default::default(),
                networked: false,
                clock: Instant::now(),
                tick: 0,
                late: Vec::new(),
            };
        }
    }

    fn networked(scene: &Scene, peers: usize, late: usize, link: Conditions, seed: u64, meshes: Meshes) -> Self {
        let mut components = Components::new();
        super::register(&mut components);
        let sent = Arc::new(AtomicU64::new(0));
        let served = Arc::new(AtomicU64::new(0));
        let sent_kinds: Arc<Kinds> = Default::default();
        let served_kinds: Arc<Kinds> = Default::default();
        let mut ends = Loopback::network((peers + late) as u32).into_iter();
        let listener = ends.next().expect("the host's end");
        let host = Party::host(
            "netsim",
            "host",
            &components,
            vec![Box::new(Metered { inner: listener, sent: served.clone(), kinds: served_kinds.clone() })],
            false,
        );
        let mut all = vec![Peer::new(host, scene, 0, meshes)];
        let mut waiting = Vec::new();
        for (i, end) in ends.enumerate() {
            let lagged: Box<dyn Transport + Send> = if link == Conditions::GOOD {
                Box::new(Metered { inner: end, sent: sent.clone(), kinds: sent_kinds.clone() })
            } else {
                Box::new(Metered { inner: Laggy::new(end, link, seed * 101 + i as u64), sent: sent.clone(), kinds: sent_kinds.clone() })
            };
            if i + 1 < peers {
                all.push(Peer::new(Party::join(lagged, "netsim", &format!("guest{}", i + 1), &components), scene, i + 1, meshes));
            } else {
                waiting.push((lagged, scene.clone()));
            }
        }
        Self { peers: all, components, sent, served, sent_kinds, served_kinds, networked: true, clock: Instant::now(), tick: 0, late: waiting }
    }

    /// The next late peer comes in: it loads the scene afresh and joins
    /// the session as it stands. Ticks until it is in; whether it is.
    pub fn join_late(&mut self) -> bool {
        let Some((end, scene)) = self.late.pop() else { return false };
        let name = format!("late{}", self.peers.len());
        let index = self.peers.len();
        self.peers.push(Peer::new(Party::join(end, "netsim", &name, &self.components), &scene, index, &mut |_, _| Some(MeshHandle::TEST)));
        for _ in 0..ticks(8.0) {
            self.step();
            if self.peers.last().is_some_and(|p| p.party.welcomed()) {
                return true;
            }
        }
        false
    }

    /// One tick for everyone, at the pace of real time when there is a
    /// link (its delays are the clock's).
    pub fn step(&mut self) {
        self.step_with(|_, _| {});
    }

    /// [`Session::step`], with what each peer's game does before its
    /// physics, by the peer's number.
    pub fn step_with(&mut self, mut game: impl FnMut(usize, &mut Peer)) {
        for (i, peer) in self.peers.iter_mut().enumerate() {
            peer.step_with(&self.components, |p| game(i, p));
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
