//! A game played together: the one object a game keeps for it.
//!
//! [`Party`] is this peer's client of the session's server
//! ([`crate::net::server`]), and, on the host, the server's keeper too:
//! the server runs in the host's process and the host's own game talks to
//! it through a loopback, like everyone else's does over the network. A
//! game on its own is the same thing with nobody else connected
//! ([`Party::alone`]) — there is no offline mode, so single player goes
//! the path four friends take (DNA, postulate 4; the dacha simulator).
//!
//! Call [`Party::update`] every frame. It keeps the connection, joins in
//! two steps — accepted, then *ready* once this peer stands in the
//! session's scene, and only then the world — sends what this peer owns
//! thirty times a second (what changed, and a last word when it stops),
//! takes what the others own, and shows their things a moment in the
//! past, smoothly.
//!
//! **Ownership, from the game's side.** Everything has an owner; the
//! scene's things start as the host's, and what a peer spawns is its own.
//! To drive something, ask: [`Party::claim`] (or put
//! [`crate::net::RequestOwnership`] on it). It is yours at once — this
//! frame, on an assumption, [`crate::net::OwnershipPending`] beside it —
//! and the server's ruling settles it. Ask *before* the contact, not on
//! it: a thing somebody else owns is kinematic here, and a push against it
//! moves nothing until it is yours. When two ask at once, the server's
//! last grant is everyone's answer, and the other one's copy glides to
//! where the winner has it ([`Event::ClaimLost`]).
//!
//! **When the host goes**, the session goes with it: no host migration,
//! as the dacha simulator decided after building one — a level is the
//! host's world. A guest is told ([`Event::HostLost`]), keeps its world on
//! screen, and redials every few seconds until the host is back
//! ([`Event::HostBack`]) or the game gives up ([`Party::give_up`], which
//! makes it the host of its own copy).
//!
//! **Started from the editor or the command line**: `RUNITY_NET` is
//! `host:ADDRESS` or `join:ADDRESS`, `RUNITY_PLAYER` the name to join
//! with, and `RUNITY_LINK` how bad to make the link (`poor`, `awful`,
//! `latency=80,loss=3`); [`Party::from_env`] reads them. Without
//! `RUNITY_NET` the game plays alone.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::components::Components;
use crate::id::EntityId;
use crate::net::link::{Link, LinkEvent, Mode};
use crate::net::protocol::{self, ToClient, ToServer, PROTOCOL};
use crate::net::server::{Server, ServerThread, DATAGRAM_BUDGET};
use crate::net::sync::{present, Noticed, Sync, Tally, NET_HZ};
use crate::net::wire::{Conditions, Laggy, Loopback, Transport, Udp};
use crate::net::{
    announce, despawn_tree, network_id, owner_of, NetId, NetTick, Owned, Owner, OwnershipPending,
    PeerId, Replica, RequestOwnership,
};
use crate::scene::Transform;
use crate::world::SceneId;

/// Names the session to join: `host:127.0.0.1:47800` to host it there,
/// `join:127.0.0.1:47800` to join it. Unset, the game plays alone.
pub const NET_VAR: &str = "RUNITY_NET";

/// The name a player joins with: "Player 2".
pub const PLAYER_VAR: &str = "RUNITY_PLAYER";

/// How bad to make this player's link, for trying the game on one:
/// `poor`, `awful`, or `latency=80,jitter=10,loss=3,dup=1`.
pub const LINK_VAR: &str = "RUNITY_LINK";

// Where each player's window opens: the launcher sets it, the desktop
// shell reads it, and neither is this module, so the core names it.
pub use runity_core::project::WINDOW_VAR;

/// How long a replica may go without word from its owner, not having been
/// told it came to rest, before [`Event::Silent`] says so.
const SILENCE: Duration = Duration::from_secs(3);

/// Where player `n` of `count` (from 0) opens its window, as
/// [`WINDOW_VAR`] wants it: half the game's size when several play, two to
/// a row, so four fit where one would.
pub fn tile(n: u32, count: u32, (width, height): (u32, u32)) -> String {
    let (w, h) = if count > 1 {
        (width / 2, height / 2)
    } else {
        (width, height)
    };
    let (column, row) = (n % 2, n / 2);
    let x = 40 + column * (w + 16);
    let y = 60 + row * (h + 48);
    format!("{x},{y},{w},{h}")
}

/// A UDP port on this machine nobody is using, for a host to listen on.
pub fn free_port() -> std::io::Result<u16> {
    Ok(std::net::UdpSocket::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

/// What happened in the party, for the game to show or act on.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The session plays another scene than this game has loaded: load
    /// it, then [`Party::ready_in`]. Nothing about the world arrives
    /// until then.
    SceneRequired { scene: String },
    /// In the game: the world as the server holds it has arrived.
    Welcomed,
    /// Someone is in the game.
    Joined { peer: PeerId, name: String },
    /// Someone left — `clean` when they said so, rather than went quiet.
    Left {
        peer: PeerId,
        name: String,
        clean: bool,
    },
    /// The host is gone (`quit`: on purpose). The world stays; the party
    /// redials.
    HostLost { quit: bool },
    /// The host answered again, and the world is the session's again.
    HostBack,
    /// Turned away at the door, and why.
    Rejected(String),
    /// A claim was ruled against: `owner` drives it.
    ClaimLost { id: EntityId, owner: PeerId },
    /// Something that happened once, published by `from` ([`Party::publish`];
    /// decode it with [`Event::decode`]).
    Message {
        from: PeerId,
        kind: String,
        body: Vec<u8>,
    },
    /// Its owner stopped talking about it without saying it came to rest.
    Silent { id: EntityId, owner: PeerId },
    /// What could not be done here, in words.
    Problem(String),
}

impl Event {
    /// A [`Event::Message`]'s body as `T`, if it is of `kind`.
    pub fn decode<T: DeserializeOwned>(&self, kind: &str) -> Option<T> {
        match self {
            Event::Message { kind: k, body, .. } if k == kind => postcard::from_bytes(body).ok(),
            _ => None,
        }
    }
}

enum Hosting {
    Inline(Box<Server>),
    Thread(ServerThread),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// Dialled; waiting for the server to answer.
    Dialling,
    /// Accepted; loading the session's scene.
    Loading,
    /// Ready: the world is on its way, or here.
    Playing,
    /// The host is gone; redialling at `retry`.
    Lost {
        retry: Instant,
    },
    Rejected,
}

/// This peer's place in a game played together. Keep one, and call
/// [`Party::update`] every frame.
pub struct Party {
    name: String,
    /// The scene this game has loaded.
    scene: String,
    fingerprint: u64,
    link: Link,
    hosting: Option<Hosting>,
    alone: bool,
    stage: Stage,
    sync: Sync,
    welcomed: bool,
    was_lost: bool,
    session: Option<u64>,
    roster: BTreeMap<PeerId, String>,
    since_send: f32,
    since_watch: f32,
    silent: std::collections::HashSet<EntityId>,
    outbox: Vec<ToServer>,
    local: Vec<Event>,
    /// This process's clock, and what it has learned of the server's.
    clock: Clock,
}

/// The server's clock as seen from here: each answer to a question sent
/// at `sent` and heard at `now` says the server read `server` about half
/// way between, give or take half the round trip. Of the last few, the
/// quickest round trip is trusted — a slow one was held up somewhere, one
/// way or the other, and says less.
#[derive(Debug)]
struct Clock {
    began: Instant,
    since_ask: f32,
    /// (round trip, offset to add to ours), newest last.
    samples: std::collections::VecDeque<(f64, f64)>,
}

impl Clock {
    fn new() -> Self {
        Self {
            began: Instant::now(),
            // Asks as soon as it can.
            since_ask: f32::MAX,
            samples: Default::default(),
        }
    }

    fn now(&self) -> f64 {
        self.began.elapsed().as_secs_f64()
    }

    fn heard(&mut self, sent: f64, server: f64) {
        let now = self.now();
        let trip = (now - sent).max(0.0);
        self.samples.push_back((trip, server + trip * 0.5 - now));
        while self.samples.len() > 8 {
            self.samples.pop_front();
        }
    }

    fn best(&self) -> Option<(f64, f64)> {
        self.samples
            .iter()
            .copied()
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }
}

/// The server's end of a client's link.
const SERVER: PeerId = PeerId(0);

impl Party {
    fn client(
        mut link: Link,
        scene: &str,
        name: &str,
        components: &Components,
        hosting: Option<Hosting>,
    ) -> Self {
        link.dial(SERVER);
        Self {
            name: name.to_string(),
            scene: scene.to_string(),
            fingerprint: protocol::fingerprint(components.networked_names()),
            link,
            hosting,
            alone: false,
            stage: Stage::Dialling,
            sync: Sync::new(PeerId::HOST, 0),
            welcomed: false,
            was_lost: false,
            session: None,
            roster: BTreeMap::new(),
            since_send: 0.0,
            since_watch: 0.0,
            silent: Default::default(),
            outbox: Vec::new(),
            local: Vec::new(),
            clock: Clock::new(),
        }
    }

    /// Host a game of `scene`, reachable over `external` transports (none:
    /// only this process). `threaded`: the server on a thread of its own,
    /// as a game runs it; unthreaded, it ticks inside [`Party::update`],
    /// which is what a test wants.
    pub fn host(
        scene: &str,
        name: &str,
        components: &Components,
        external: Vec<Box<dyn Transport + Send>>,
        threaded: bool,
    ) -> Self {
        let mut ends = Loopback::network(2);
        let client_end = ends.pop().expect("two ends");
        let server_end = ends.pop().expect("two ends");
        let mut links = vec![Link::accepting(server_end)];
        links.extend(external.into_iter().map(Link::accepting));
        let fingerprint = protocol::fingerprint(components.networked_names());
        let server = Server::new(links, (0, client_end.me()), scene, fingerprint);
        let hosting = if threaded {
            Hosting::Thread(ServerThread::start(server, NET_HZ))
        } else {
            Hosting::Inline(Box::new(server))
        };
        Self::client(
            Link::dialling(client_end),
            scene,
            name,
            components,
            Some(hosting),
        )
    }

    /// A game on its own: the host of a session nobody else can reach.
    pub fn alone(scene: &str, components: &Components) -> Self {
        let mut party = Self::host(scene, "", components, Vec::new(), false);
        party.alone = true;
        party
    }

    /// Join the session whose server `transport` reaches at endpoint 0.
    pub fn join(
        transport: impl Transport + Send + 'static,
        scene: &str,
        name: &str,
        components: &Components,
    ) -> Self {
        Self::client(Link::dialling(transport), scene, name, components, None)
    }

    /// The party `RUNITY_NET`, `RUNITY_PLAYER` and `RUNITY_LINK` describe,
    /// for a game that has `scene` loaded.
    pub fn from_env(scene: &str, components: &Components) -> Result<Self, String> {
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        Self::from_words(
            &var(NET_VAR),
            &var(PLAYER_VAR),
            &var(LINK_VAR),
            scene,
            components,
        )
    }

    /// [`Party::from_env`], given the values.
    pub fn from_words(
        net: &str,
        player: &str,
        link: &str,
        scene: &str,
        components: &Components,
    ) -> Result<Self, String> {
        let net = net.trim();
        if net.is_empty() {
            return Ok(Self::alone(scene, components));
        }
        let conditions = Conditions::parse(link).map_err(|e| format!("{LINK_VAR}: {e}"))?;
        let (role, address) = net
            .split_once(':')
            .ok_or_else(|| format!("{NET_VAR}=`{net}`: want host:ADDRESS or join:ADDRESS"))?;
        let io = |e: std::io::Error| format!("{NET_VAR}=`{net}`: {e}");
        let seed = EntityId::fresh().raw();
        let bad = |t: Udp| -> Box<dyn Transport + Send> {
            if conditions == Conditions::GOOD {
                Box::new(t)
            } else {
                Box::new(Laggy::new(t, conditions, seed))
            }
        };
        match role {
            "host" => {
                let udp = Udp::bind(address, SERVER).map_err(io)?;
                Ok(Self::host(scene, player, components, vec![bad(udp)], true))
            }
            "join" => {
                let host: std::net::SocketAddr = address.parse().map_err(|e| {
                    format!("{NET_VAR}=`{net}`: `{address}` is not an address: {e}")
                })?;
                let any = if host.is_ipv4() {
                    "0.0.0.0:0"
                } else {
                    "[::]:0"
                };
                // The endpoint this socket introduces itself as: anything
                // but the server's 0.
                let me = PeerId((seed as u32) | 1);
                let mut udp = Udp::bind(any, me).map_err(io)?;
                udp.connect(SERVER, host);
                Ok(Self::join(bad(udp), scene, player, components))
            }
            other => Err(format!("{NET_VAR}=`{net}`: `{other}` is not host or join")),
        }
    }

    /// The session's clock, seconds since it began, the same on every
    /// machine give or take a few milliseconds: when a timed thing
    /// started, a countdown everyone sees end together. Our own clock
    /// until the server has answered (and alone).
    pub fn server_time(&self) -> f64 {
        self.clock.now() + self.clock.best().map_or(0.0, |(_, offset)| offset)
    }

    /// How long a message takes there and back, seconds, at best of the
    /// last few; `None` before the first answer, or alone.
    pub fn round_trip(&self) -> Option<f64> {
        self.clock.best().map(|(trip, _)| trip)
    }

    /// Which peer this is, as the server numbered it (the host is 0).
    pub fn me(&self) -> PeerId {
        self.sync.me
    }

    pub fn is_host(&self) -> bool {
        self.hosting.is_some()
    }

    /// A game on its own, that nobody can join.
    pub fn is_alone(&self) -> bool {
        self.alone
    }

    /// In the game, with the world the session has.
    pub fn welcomed(&self) -> bool {
        self.welcomed
    }

    /// Everyone in the game, this peer included: id and name.
    pub fn roster(&self) -> Vec<(PeerId, String)> {
        self.roster.iter().map(|(p, n)| (*p, n.clone())).collect()
    }

    /// How a window says who it is: the name joined with, or "Player N".
    pub fn name(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        format!("Player {}", self.me().0 + 1)
    }

    /// What became of the snapshots that came in.
    pub fn tally(&self) -> Tally {
        self.sync.tally
    }

    /// Ask to drive `entity`: this peer's at once, the server's ruling to
    /// follow.
    /// Make these one thing for ownership — a player and both hands: a
    /// claim on any of them is a claim on all, so one peer drives the lot.
    pub fn group(&self, world: &mut hecs::World, members: &[hecs::Entity]) {
        let group = crate::net::NetGroup(EntityId::fresh());
        for &entity in members {
            let _ = world.insert_one(entity, group);
        }
    }

    pub fn claim(&self, world: &mut hecs::World, entity: hecs::Entity) {
        let _ = world.insert_one(entity, RequestOwnership);
    }

    /// An entity the game just spawned from `prefab`: made this peer's, and
    /// spawned for everyone at the next network tick. Put
    /// [`crate::net::DespawnWithOwner`] on it first for something that
    /// should leave with this player.
    pub fn spawn(&self, world: &mut hecs::World, entity: hecs::Entity, prefab: &str) -> EntityId {
        announce(world, entity, self.me(), prefab)
    }

    /// Remove something for everyone. Only what this peer owns for sure —
    /// not on an assumption, since a removal cannot be taken back.
    pub fn despawn(&mut self, world: &mut hecs::World, entity: hecs::Entity) -> bool {
        let sure =
            world.get::<&Owned>(entity).is_ok() && world.get::<&OwnershipPending>(entity).is_err();
        let Some(id) = network_id(world, entity) else {
            return false;
        };
        if !sure {
            return false;
        }
        // A spawned one is found gone by the next gather; a scene one has
        // to be said, since a scene unloading looks the same.
        if world.get::<&NetId>(entity).is_err() {
            self.outbox.push(ToServer::Despawn {
                epoch: self.sync.epoch,
                id,
            });
        }
        despawn_tree(world, entity);
        true
    }

    /// Tell everyone something that happened once — here at once, the
    /// others a round trip later. Comes back as [`Event::Message`] on every
    /// peer, this one included.
    pub fn publish<T: Serialize>(&mut self, kind: &str, value: &T) {
        let body = postcard::to_stdvec(value).unwrap_or_default();
        self.local.push(Event::Message {
            from: self.me(),
            kind: kind.to_string(),
            body: body.clone(),
        });
        self.outbox.push(ToServer::Rpc {
            to: None,
            kind: kind.to_string(),
            body,
        });
    }

    /// Move everyone to another scene (the host only): every peer gets
    /// [`Event::SceneRequired`], this one too.
    pub fn set_scene(&mut self, scene: &str) {
        if self.is_host() {
            self.outbox.push(ToServer::SetScene {
                scene: scene.to_string(),
            });
        }
    }

    /// This game now stands in `scene`: let the world in.
    pub fn ready_in(&mut self, scene: &str) {
        self.scene = scene.to_string();
        if self.stage == Stage::Loading {
            self.stage = Stage::Playing;
            self.outbox.push(ToServer::Ready {
                epoch: self.sync.epoch,
            });
        }
    }

    /// Stop waiting for a host that is gone: become the host of this copy
    /// of the world, alone.
    pub fn give_up(&mut self, components: &Components, world: &mut hecs::World) {
        if !matches!(self.stage, Stage::Lost { .. }) {
            return;
        }
        let old = self.me();
        // What was ours stays ours, and the scene is the new host's — which
        // is us. Anyone else's spawned things are ours to keep too: they
        // are in our copy of the world, and no one else will drive them.
        for (_, entity) in crate::net::addressable(world) {
            if world.get::<&SceneId>(entity).is_ok() || owner_of(world, entity) == old {
                let _ = world.remove_one::<Owner>(entity);
            } else {
                let _ = world.insert_one(entity, Owner(PeerId::HOST));
            }
        }
        *self = Party::alone(&self.scene, components);
    }

    fn send(&mut self, messages: Vec<ToServer>, mode: Mode) {
        for datagram in protocol::pack(messages, DATAGRAM_BUDGET) {
            self.link.send(SERVER, datagram, mode);
        }
    }

    /// One frame of the party. `spawn` puts a prefab into the world for
    /// someone else's spawn — [`crate::LiveScene::spawn_prefab`], usually.
    pub fn update(
        &mut self,
        world: &mut hecs::World,
        components: &Components,
        delta: f32,
        mut spawn: impl FnMut(&mut hecs::World, &str, Transform) -> Option<hecs::Entity>,
    ) -> Vec<Event> {
        let mut events = std::mem::take(&mut self.local);
        if let Some(Hosting::Inline(server)) = &mut self.hosting {
            server.tick();
            for line in server.log.drain(..) {
                if !line.contains("joined") && !line.contains(" left") {
                    events.push(Event::Problem(format!("server: {line}")));
                }
            }
        }
        for event in self.link.poll() {
            match event {
                LinkEvent::Connected(_) => {
                    let join = ToServer::JoinRequest {
                        protocol: PROTOCOL,
                        fingerprint: self.fingerprint,
                        name: self.name.clone(),
                    };
                    self.send(vec![join], Mode::Reliable);
                }
                LinkEvent::Data(_, bytes, _) => match protocol::decode::<ToClient>(&bytes) {
                    Ok(messages) => {
                        for message in messages {
                            self.hear(world, components, message, &mut spawn, &mut events);
                        }
                    }
                    Err(e) => events.push(Event::Problem(e)),
                },
                LinkEvent::Disconnected(_, _) => self.lose(false, &mut events),
            }
        }
        if let Stage::Lost { retry } = self.stage {
            if Instant::now() >= retry {
                self.link.dial(SERVER);
                self.stage = Stage::Lost {
                    retry: Instant::now() + Duration::from_secs(3),
                };
            }
        }
        let playing = self.stage == Stage::Playing && self.welcomed;
        if playing {
            crate::net::claim_nearby(world, self.me());
            // Taken before marking, so what was asked for is driven this
            // very frame.
            let mut claims = std::mem::take(&mut self.outbox);
            self.sync.claims(world, &mut claims);
            self.send(claims, Mode::Reliable);
        }
        self.sync.one_way_ticks = self
            .round_trip()
            .map_or(0.0, |trip| trip * 0.5 * crate::net::sync::NET_HZ as f64);
        // The session's clock and the delay, for what simulates by them
        // (docs/netsim.md): gusts drawn from the same time everywhere,
        // takeovers carried forward by the delay.
        runity_core::netsim::set_session_time(world, self.server_time());
        runity_core::netsim::set_link_delay(world, self.sync.one_way_ticks);
        self.sync.mark(world);
        if matches!(self.stage, Stage::Loading | Stage::Playing) {
            // What time the server says, once a second.
            self.clock.since_ask += delta;
            if self.clock.since_ask >= 1.0 {
                self.clock.since_ask = 0.0;
                let sent = self.clock.now();
                self.send(vec![ToServer::Clock { sent }], Mode::Unreliable);
            }
        }
        if playing {
            self.since_send += delta;
            let period = 1.0 / NET_HZ;
            if self.since_send >= period {
                self.since_send = (self.since_send - period).min(period);
                let (mut reliable, mut unreliable) = (Vec::new(), Vec::new());
                self.sync
                    .gather(world, components, &mut reliable, &mut unreliable);
                self.send(reliable, Mode::Reliable);
                self.send(unreliable, Mode::Unreliable);
            }
        } else {
            // Ready, a scene change and messages go out before the gate
            // opens; anything about entities waits.
            let (early, later): (Vec<_>, Vec<_>) =
                std::mem::take(&mut self.outbox).into_iter().partition(|m| {
                    matches!(
                        m,
                        ToServer::Ready { .. } | ToServer::SetScene { .. } | ToServer::Rpc { .. }
                    )
                });
            self.send(early, Mode::Reliable);
            self.outbox = later;
        }
        present(world, delta);
        self.watch(world, delta, &mut events);
        events
    }

    fn lose(&mut self, quit: bool, events: &mut Vec<Event>) {
        if matches!(self.stage, Stage::Lost { .. } | Stage::Rejected) || self.hosting.is_some() {
            return;
        }
        self.stage = Stage::Lost {
            retry: Instant::now() + Duration::from_secs(1),
        };
        self.welcomed = false;
        self.was_lost = true;
        events.push(Event::HostLost { quit });
    }

    fn hear(
        &mut self,
        world: &mut hecs::World,
        components: &Components,
        message: ToClient,
        spawn: &mut dyn FnMut(&mut hecs::World, &str, Transform) -> Option<hecs::Entity>,
        events: &mut Vec<Event>,
    ) {
        match message {
            ToClient::JoinAccepted {
                you,
                scene,
                epoch,
                session,
                ..
            } => {
                let old = self.sync.me;
                // Another session, or ours come back: what we spawned and
                // own is ours under our new number, the scene's goes back
                // to its default until the world state says, and everyone
                // else's spawned things wait to be confirmed.
                for (_, entity) in crate::net::addressable(world) {
                    let ours = self.session.is_some() && owner_of(world, entity) == old;
                    if ours && world.get::<&NetId>(entity).is_ok() {
                        let _ = world.insert_one(entity, Owner(you));
                    } else if world.get::<&SceneId>(entity).is_ok() {
                        let _ = world.remove_one::<Owner>(entity);
                    }
                }
                self.sync = Sync::new(you, epoch);
                self.session = Some(session);
                self.roster.clear();
                self.stage = Stage::Loading;
                if scene == self.scene {
                    self.ready_in(&scene);
                } else {
                    events.push(Event::SceneRequired { scene });
                }
            }
            ToClient::JoinRejected { reason } => {
                self.stage = Stage::Rejected;
                events.push(Event::Rejected(reason));
            }
            ToClient::ClientJoined { peer, name } => {
                self.roster.insert(peer, name.clone());
                if peer != self.me() {
                    events.push(Event::Joined { peer, name });
                }
            }
            ToClient::ClientLeft { peer, clean } => {
                let name = self.roster.remove(&peer).unwrap_or_default();
                events.push(Event::Left { peer, name, clean });
            }
            ToClient::SceneChanged { scene, epoch } => {
                self.sync = Sync::new(self.me(), epoch);
                self.welcomed = false;
                self.stage = Stage::Loading;
                events.push(Event::SceneRequired { scene });
            }
            ToClient::SessionEnding => self.lose(true, events),
            ToClient::Clock { sent, server } => self.clock.heard(sent, server),
            ToClient::Rpc { from, kind, body } => events.push(Event::Message { from, kind, body }),
            other => {
                for noticed in self.sync.apply(world, components, other, spawn) {
                    match noticed {
                        Noticed::Welcomed => {
                            self.welcomed = true;
                            events.push(Event::Welcomed);
                            if std::mem::take(&mut self.was_lost) {
                                events.push(Event::HostBack);
                            }
                        }
                        Noticed::ClaimLost(id, owner) => {
                            events.push(Event::ClaimLost { id, owner })
                        }
                        Noticed::Problem(p) => events.push(Event::Problem(p)),
                    }
                }
            }
        }
    }

    /// Once a second: a replica still moving when its owner went quiet is
    /// said once, by id and owner — the dacha simulator's watchdog.
    fn watch(&mut self, world: &hecs::World, delta: f32, events: &mut Vec<Event>) {
        self.since_watch += delta;
        if self.since_watch < 1.0 {
            return;
        }
        self.since_watch = 0.0;
        let now = Instant::now();
        for (entity, tick) in world
            .query::<(hecs::Entity, &NetTick)>()
            .with::<&Replica>()
            .iter()
        {
            let Some(id) = network_id(world, entity) else {
                continue;
            };
            let quiet = !tick.settled && now.duration_since(tick.at) > SILENCE;
            if quiet && self.silent.insert(id) {
                events.push(Event::Silent {
                    id,
                    owner: tick.sender,
                });
            } else if !quiet {
                self.silent.remove(&id);
            }
        }
    }
}

impl Drop for Party {
    /// Leaving says so. A host's server tells everyone the session is over.
    fn drop(&mut self) {
        match self.hosting.take() {
            Some(Hosting::Inline(mut server)) => server.end(),
            Some(Hosting::Thread(thread)) => drop(thread),
            None => {
                self.send(vec![ToServer::Leave], Mode::Reliable);
                self.link.close(SERVER);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::DespawnWithOwner;
    use glam::Vec3;
    use serde::Deserialize;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Lit(bool);

    fn components() -> Components {
        let mut c = Components::new();
        c.register_networked::<Lit>("lit");
        c
    }

    /// The same scene loaded by every peer: a crate (1), a door (2) and a
    /// torch (3), the torch unlit.
    fn scene_world() -> hecs::World {
        let mut world = hecs::World::new();
        for n in [1u64, 2] {
            world.spawn((SceneId(EntityId::from_raw(n)), Transform::default()));
        }
        world.spawn((
            SceneId(EntityId::from_raw(3)),
            Transform::default(),
            Lit(false),
        ));
        world
    }

    fn find(world: &hecs::World, id: u64) -> hecs::Entity {
        crate::net::addressable(world)[&EntityId::from_raw(id)]
    }

    fn at(world: &hecs::World, entity: hecs::Entity) -> Vec3 {
        world.get::<&Transform>(entity).unwrap().position
    }

    fn put(world: &mut hecs::World, id: u64, position: Vec3) {
        let e = find(world, id);
        world.get::<&mut Transform>(e).unwrap().position = position;
    }

    /// A host and guests over loopback, the server ticking inside the
    /// host's update.
    struct Game {
        parties: Vec<Party>,
        worlds: Vec<hecs::World>,
        components: Components,
    }

    impl Game {
        fn new(guests: u32, link: Conditions) -> Self {
            let components = components();
            let mut ends = Loopback::network(guests + 1).into_iter();
            let listener = ends.next().unwrap();
            let mut parties = vec![Party::host(
                "main",
                "host",
                &components,
                vec![Box::new(listener)],
                false,
            )];
            for (i, end) in ends.enumerate() {
                let end: Box<dyn Transport + Send> = if link == Conditions::GOOD {
                    Box::new(end)
                } else {
                    Box::new(Laggy::new(end, link, 11 + i as u64))
                };
                parties.push(Party::join(
                    end,
                    "main",
                    &format!("guest{}", i + 1),
                    &components,
                ));
            }
            let worlds = (0..=guests).map(|_| scene_world()).collect();
            let mut game = Self {
                parties,
                worlds,
                components,
            };
            game.frames(10);
            game
        }

        /// Every peer runs a frame of a thirtieth of a second, the host
        /// first, and a little real time passes, as it would.
        fn frame(&mut self) -> Vec<Vec<Event>> {
            let components = &self.components;
            let out = self
                .parties
                .iter_mut()
                .zip(self.worlds.iter_mut())
                .map(|(party, world)| {
                    party.update(world, components, 1.0 / NET_HZ, |w, _, t| {
                        Some(w.spawn((t,)))
                    })
                })
                .collect();
            std::thread::sleep(Duration::from_millis(2));
            out
        }

        fn frames(&mut self, n: usize) -> Vec<Event> {
            let mut all = Vec::new();
            for _ in 0..n {
                for events in self.frame() {
                    all.extend(events);
                }
            }
            all
        }

        /// Frames until `done`, or fail after `limit`.
        fn until(&mut self, limit: usize, done: impl Fn(&Game) -> bool) -> Vec<Event> {
            let mut all = Vec::new();
            for _ in 0..limit {
                if done(self) {
                    return all;
                }
                for events in self.frame() {
                    all.extend(events);
                }
            }
            let tallies: Vec<Tally> = self.parties.iter().map(|p| p.tally()).collect();
            assert!(done(self), "not after {limit} frames; tallies {tallies:?}");
            all
        }
    }

    #[test]
    fn alone_is_a_session_of_one_that_owns_everything() {
        let components = components();
        let mut party = Party::from_words("", "", "", "main", &components).unwrap();
        let mut world = scene_world();
        for _ in 0..5 {
            party.update(&mut world, &components, 1.0 / NET_HZ, |_, _, _| None);
        }
        assert!(party.is_host() && party.is_alone() && party.welcomed());
        assert_eq!(party.me(), PeerId::HOST);
        assert!(
            world.get::<&Owned>(find(&world, 1)).is_ok(),
            "the host owns the scene"
        );
    }

    #[test]
    fn four_friends_come_in_and_know_each_other_by_name() {
        let game = Game::new(3, Conditions::GOOD);
        for party in &game.parties {
            assert!(party.welcomed());
            let names: Vec<String> = party.roster().into_iter().map(|(_, n)| n).collect();
            assert_eq!(names, ["host", "guest1", "guest2", "guest3"]);
        }
        assert_eq!(game.parties[2].me(), PeerId(2));
        assert!(game.worlds[0]
            .get::<&Owned>(find(&game.worlds[0], 1))
            .is_ok());
        assert!(game.worlds[1]
            .get::<&Replica>(find(&game.worlds[1], 1))
            .is_ok());
    }

    #[test]
    fn what_the_host_moves_every_guest_sees_and_a_guest_cannot_move_it() {
        let mut game = Game::new(2, Conditions::GOOD);
        put(&mut game.worlds[0], 1, Vec3::new(4.0, 0.0, 0.0));
        // A guest's own system pushes the host's crate; the host's word
        // wins, and the guest's write never leaves the guest.
        put(&mut game.worlds[2], 1, Vec3::new(-9.0, 0.0, 0.0));
        game.until(60, |g| {
            g.worlds
                .iter()
                .all(|w| at(w, find(w, 1)).distance(Vec3::X * 4.0) < 1e-3)
        });
    }

    #[test]
    fn a_networked_component_travels_with_its_owner() {
        let mut game = Game::new(1, Conditions::GOOD);
        let torch = find(&game.worlds[0], 3);
        *game.worlds[0].get::<&mut Lit>(torch).unwrap() = Lit(true);
        game.until(30, |g| {
            *g.worlds[1].get::<&Lit>(find(&g.worlds[1], 3)).unwrap() == Lit(true)
        });
    }

    #[test]
    fn a_claim_is_had_at_once_and_everyone_ends_up_seeing_the_claimant() {
        let mut game = Game::new(2, Conditions::GOOD);
        let crate_ = find(&game.worlds[1], 1);
        game.parties[1].claim(&mut game.worlds[1], crate_);
        game.frame();
        // Driven this frame, on an assumption.
        assert!(game.worlds[1].get::<&Owned>(crate_).is_ok());
        game.until(30, |g| {
            g.worlds
                .iter()
                .all(|w| owner_of(w, find(w, 1)) == PeerId(1))
                && g.worlds[1]
                    .get::<&OwnershipPending>(find(&g.worlds[1], 1))
                    .is_err()
        });
        assert!(
            game.worlds[0]
                .get::<&Replica>(find(&game.worlds[0], 1))
                .is_ok(),
            "the host now shows it"
        );
        let target = Vec3::new(0.0, 2.0, 3.0);
        put(&mut game.worlds[1], 1, target);
        game.until(60, |g| {
            g.worlds
                .iter()
                .all(|w| at(w, find(w, 1)).distance(target) < 1e-3)
        });
    }

    #[test]
    fn a_claim_on_one_of_a_group_takes_the_whole_group() {
        let mut game = Game::new(1, Conditions::GOOD);
        let (crate_, door) = (find(&game.worlds[1], 1), find(&game.worlds[1], 2));
        game.parties[1].group(&mut game.worlds[1], &[crate_, door]);
        game.parties[1].claim(&mut game.worlds[1], crate_);
        game.frames(20);
        for world in &game.worlds {
            assert_eq!(owner_of(world, find(world, 1)), PeerId(1));
            assert_eq!(
                owner_of(world, find(world, 2)),
                PeerId(1),
                "the other one too"
            );
        }
    }

    #[test]
    fn a_claim_on_one_half_of_a_joint_takes_the_other() {
        let mut game = Game::new(1, Conditions::GOOD);
        for world in &mut game.worlds {
            let (crate_, door) = (find(world, 1), find(world, 2));
            let to = world.get::<&SceneId>(door).unwrap().0;
            let _ = world.insert_one(
                crate_,
                crate::world::Jointed(crate::scene::Joint::Fixed { to }),
            );
        }
        let door = find(&game.worlds[1], 2);
        game.parties[1].claim(&mut game.worlds[1], door);
        game.frames(20);
        for world in &game.worlds {
            assert_eq!(owner_of(world, find(world, 2)), PeerId(1));
            assert_eq!(
                owner_of(world, find(world, 1)),
                PeerId(1),
                "held by the joint"
            );
        }
    }

    #[test]
    fn a_player_coming_near_a_loose_body_claims_it() {
        use crate::world::{Physics, WorldTransform};
        let mut game = Game::new(1, Conditions::GOOD);
        let at = |x: f32| WorldTransform(glam::Mat4::from_translation(Vec3::new(x, 0.0, 0.0)));
        for world in &mut game.worlds {
            let crate_ = find(world, 1);
            let _ = world.insert(crate_, (Physics(crate::scene::Body::Dynamic), at(5.0)));
            put(world, 1, Vec3::new(5.0, 0.0, 0.0));
        }
        let player = game.worlds[1].spawn((
            at(0.0),
            crate::net::Owner(PeerId(1)),
            crate::net::ClaimNear { radius: 2.0 },
        ));
        game.frames(10);
        assert_eq!(
            owner_of(&game.worlds[1], find(&game.worlds[1], 1)),
            PeerId::HOST,
            "too far"
        );
        let _ = game.worlds[1].insert_one(player, at(4.0));
        game.frames(20);
        for world in &game.worlds {
            assert_eq!(
                owner_of(world, find(world, 1)),
                PeerId(1),
                "came near: the guest's"
            );
        }
    }

    #[test]
    fn two_grabbing_one_thing_get_one_answer_and_the_loser_is_told() {
        let mut game = Game::new(2, Conditions::GOOD);
        for i in [1, 2] {
            let crate_ = find(&game.worlds[i], 1);
            game.parties[i].claim(&mut game.worlds[i], crate_);
        }
        let events = game.frames(20);
        let owners: Vec<PeerId> = game
            .worlds
            .iter()
            .map(|w| owner_of(w, find(w, 1)))
            .collect();
        assert!(
            owners.iter().all(|o| *o == owners[0]) && owners[0] != PeerId::HOST,
            "{owners:?}"
        );
        let lost = events
            .iter()
            .filter(|e| matches!(e, Event::ClaimLost { .. }))
            .count();
        assert_eq!(lost, 1, "{events:?}");
    }

    #[test]
    fn a_guests_spawn_reaches_everyone_and_a_late_joiner_and_leaves_with_them() {
        let components = components();
        let mut ends = Loopback::network(3).into_iter();
        let listener = ends.next().unwrap();
        let mut game = Game {
            parties: vec![
                Party::host("main", "host", &components, vec![Box::new(listener)], false),
                Party::join(ends.next().unwrap(), "main", "early", &components),
            ],
            worlds: vec![scene_world(), scene_world()],
            components,
        };
        game.frames(10);
        let pawn = game.worlds[1].spawn((Transform::default(), DespawnWithOwner));
        let crate_ = game.worlds[1].spawn((Transform::default(),));
        let pawn_id = game.parties[1].spawn(&mut game.worlds[1], pawn, "pawn");
        let crate_id = game.parties[1].spawn(&mut game.worlds[1], crate_, "crate");
        game.frames(10);
        assert!(crate::net::addressable(&game.worlds[0]).contains_key(&pawn_id));

        let late = Party::join(ends.next().unwrap(), "main", "late", &game.components);
        game.parties.push(late);
        game.worlds.push(scene_world());
        game.frames(10);
        let named = crate::net::addressable(&game.worlds[2]);
        assert!(
            named.contains_key(&pawn_id) && named.contains_key(&crate_id),
            "the late one has both"
        );
        // The host drives two things by now; the late one nothing.
        put(&mut game.worlds[0], 1, Vec3::X);
        put(&mut game.worlds[0], 2, Vec3::Y);
        game.frames(5);

        // The early guest leaves: the pawn with them, the crate to whoever
        // owns fewest — the late one, who owns nothing.
        drop(game.parties.remove(1));
        game.worlds.remove(1);
        let events = game.frames(10);
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Left { name, clean: true, .. } if name == "early")));
        for world in &game.worlds {
            let named = crate::net::addressable(world);
            assert!(
                !named.contains_key(&pawn_id),
                "the pawn went with its owner"
            );
            assert_eq!(owner_of(world, named[&crate_id]), PeerId(2));
        }
    }

    #[test]
    fn a_message_is_heard_by_everyone_the_sender_first() {
        let mut game = Game::new(2, Conditions::GOOD);
        game.parties[2].publish("bell", &3u8);
        let first = game.frame();
        assert_eq!(first[2][0].decode::<u8>("bell"), Some(3), "here at once");
        let later = game.frames(3);
        assert_eq!(
            later
                .iter()
                .filter(|e| e.decode::<u8>("bell") == Some(3))
                .count(),
            2
        );
    }

    #[test]
    fn the_host_leaving_is_said_and_the_world_stays() {
        let mut game = Game::new(1, Conditions::GOOD);
        put(&mut game.worlds[0], 1, Vec3::Y);
        game.frames(20);
        drop(game.parties.remove(0));
        game.worlds.remove(0);
        let events = game.frames(3);
        assert!(
            events.contains(&Event::HostLost { quit: true }),
            "{events:?}"
        );
        assert!(
            at(&game.worlds[0], find(&game.worlds[0], 1)).distance(Vec3::Y) < 1e-3,
            "still on screen"
        );
        let components = game.components.clone();
        game.parties[0].give_up(&components, &mut game.worlds[0]);
        game.frames(3);
        assert!(game.parties[0].is_host());
        assert!(
            game.worlds[0]
                .get::<&Owned>(find(&game.worlds[0], 1))
                .is_ok(),
            "ours now"
        );
    }

    #[test]
    fn another_build_is_turned_away() {
        let components = components();
        let mut ends = Loopback::network(2).into_iter();
        let mut host = Party::host(
            "main",
            "",
            &components,
            vec![Box::new(ends.next().unwrap())],
            false,
        );
        let mut other = Components::new();
        other.register_networked::<Lit>("lamp");
        let mut guest = Party::join(ends.next().unwrap(), "main", "", &other);
        let (mut hw, mut gw) = (scene_world(), scene_world());
        let mut events = Vec::new();
        for _ in 0..10 {
            host.update(&mut hw, &components, 0.03, |_, _, _| None);
            events.extend(guest.update(&mut gw, &other, 0.03, |_, _, _| None));
        }
        assert!(
            matches!(&events[..], [Event::Rejected(r)] if r.contains("different build")),
            "{events:?}"
        );
    }

    #[test]
    fn everyone_goes_where_the_host_goes() {
        let mut game = Game::new(1, Conditions::GOOD);
        game.parties[0].set_scene("cave");
        let events = game.frames(5);
        let asked = events
            .iter()
            .filter(|e| matches!(e, Event::SceneRequired { scene } if scene == "cave"))
            .count();
        assert_eq!(asked, 2, "{events:?}");
        for (party, world) in game.parties.iter_mut().zip(game.worlds.iter_mut()) {
            *world = scene_world();
            party.ready_in("cave");
        }
        game.frames(10);
        assert!(game.parties.iter().all(|p| p.welcomed()));
    }

    #[test]
    fn everyone_reads_the_same_session_clock_over_a_slow_link() {
        let slow = Conditions {
            latency: Duration::from_millis(30),
            ..Conditions::GOOD
        };
        let mut game = Game::new(1, slow);
        game.until(3000, |g| g.parties.iter().all(|p| p.round_trip().is_some()));
        let trip = game.parties[1].round_trip().unwrap();
        assert!(trip >= 0.025, "the delay shows in the round trip: {trip}");
        let apart = (game.parties[0].server_time() - game.parties[1].server_time()).abs();
        assert!(
            apart < 0.03,
            "the host's and the guest's clocks: {apart} s apart"
        );
    }

    #[test]
    fn everything_converges_over_an_awful_link() {
        let mut game = Game::new(2, Conditions::AWFUL);
        game.until(3000, |g| g.parties.iter().all(|p| p.welcomed()));
        put(&mut game.worlds[0], 1, Vec3::new(1.0, 2.0, 3.0));
        let door = find(&game.worlds[2], 2);
        game.parties[2].claim(&mut game.worlds[2], door);
        game.frames(3);
        put(&mut game.worlds[2], 2, Vec3::new(-5.0, 0.0, 0.0));
        game.until(3000, |g| {
            g.worlds.iter().all(|w| {
                at(w, find(w, 1)).distance(Vec3::new(1.0, 2.0, 3.0)) < 1e-3
                    && at(w, find(w, 2)).distance(Vec3::new(-5.0, 0.0, 0.0)) < 1e-3
            })
        });
    }

    #[test]
    fn over_real_sockets_too() {
        let components = components();
        let port = free_port().unwrap();
        let mut host = Party::from_words(
            &format!("host:127.0.0.1:{port}"),
            "host",
            "",
            "main",
            &components,
        )
        .unwrap();
        let mut guest = Party::from_words(
            &format!("join:127.0.0.1:{port}"),
            "guest",
            "",
            "main",
            &components,
        )
        .unwrap();
        let (mut hw, mut gw) = (scene_world(), scene_world());
        for i in 0..300 {
            if i == 20 {
                put(&mut hw, 1, Vec3::X);
            }
            host.update(&mut hw, &components, 1.0 / NET_HZ, |_, _, _| None);
            guest.update(&mut gw, &components, 1.0 / NET_HZ, |_, _, _| None);
            if guest.welcomed() && at(&gw, find(&gw, 1)).distance(Vec3::X) < 1e-3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(host.roster().len(), 2);
        assert!(at(&gw, find(&gw, 1)).distance(Vec3::X) < 1e-3);
    }

    #[test]
    fn the_words_that_start_a_party_are_checked() {
        let components = components();
        for (net, link, says) in [
            ("somewhere", "", "host:ADDRESS"),
            ("visit:1.2.3.4:5", "", "not host or join"),
            ("join:nowhere", "", "not an address"),
            ("join:127.0.0.1:9", "bad", "RUNITY_LINK"),
        ] {
            let Err(e) = Party::from_words(net, "", link, "main", &components) else {
                panic!("{net} should not start a party");
            };
            assert!(e.contains(says), "{e}");
        }
    }

    #[test]
    fn players_windows_lie_side_by_side_at_half_the_size() {
        assert_eq!(tile(0, 1, (1280, 720)), "40,60,1280,720");
        assert_eq!(tile(1, 4, (1280, 720)), "696,60,640,360");
        assert_eq!(tile(3, 4, (1280, 720)), "696,468,640,360");
    }
}
