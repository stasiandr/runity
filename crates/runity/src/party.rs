//! A game played together: who is in it, and keeping their worlds in step.
//!
//! [`crate::net`] has the pieces — owners, snapshots, handovers, spawns, a
//! welcome for whoever comes late, what happens to a leaver's things.
//! [`Party`] is the one object a game keeps that puts them together, once
//! a frame: it lets peers in, tells everyone who is here, sends what this
//! peer owns, takes what the others own, notices who went quiet, and makes
//! the others' things glide rather than jump.
//!
//! DNA, postulate 4: a game on its own is a party of one ([`Party::alone`]),
//! and the same code runs it — so a single-player run already goes through
//! the path four friends take.
//!
//! **The shape of it is a star.** Guests talk to the host; the host passes
//! on what each guest says to the others. It works over every
//! [`Transport`] — plain [`crate::net::Udp`] included, where a guest knows
//! nobody's address but the host's — at the cost of a second hop between
//! two guests: two friends at 80 ms from the host see each other at about
//! 160 ms. The things in one's own hands are not delayed at all (shared
//! authority), which is what that trade is for.
//!
//! **What it does not do yet.** When the host goes, the guests are told
//! ([`Event::HostLost`]) and the game is over for them: moving the host
//! would need guests who can reach each other, which a star does not give.
//! Two peers grabbing one thing at once, and bodies of different owners
//! pushing each other, are the DNA's open question 3; a party only carries
//! what owners say.
//!
//! **Started from the editor.** `RUNITY_NET` says `host:ADDRESS` or
//! `join:ADDRESS` and `RUNITY_PEER` which peer this is; [`Party::from_env`]
//! reads them, and a game without them plays alone. The editor's play with
//! several players (Unity's Multiplayer Play Mode) and `runity run
//! --players N` set them for each window they open.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::components::Components;
use crate::id::EntityId;
use crate::net::{
    announce, owner_of, peer_left, smooth, smooth_this, welcome, Delta, Handover, Loopback,
    Message, NetId, Owner, PeerId, Reliable, Remote, Transport, Udp,
};
use crate::scene::Transform;
use crate::world::SceneId;

/// Names the session to join: `host:127.0.0.1:47800` to be the host there,
/// `join:127.0.0.1:47800` to join it. Unset, the game plays alone.
pub const NET_VAR: &str = "RUNITY_NET";

/// Which peer this process is, a number; the host is 0. A guest without
/// one picks one at random.
pub const PEER_VAR: &str = "RUNITY_PEER";

/// Where a window opens and how big, as `x,y,width,height` in logical
/// pixels: set for each player's window when several play from the editor
/// (`runity run --players N`), so they lie side by side rather than on top
/// of each other. The desktop shell reads it; it overrides the size the
/// game asked for.
pub const WINDOW_VAR: &str = "RUNITY_WINDOW";

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
    Ok(std::net::UdpSocket::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// What happened in the party this frame, for the game to show or act on.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Someone is in the game now.
    Joined(PeerId),
    /// Someone left or went quiet; what they owned is `inherited` by the
    /// lowest peer still here.
    Left {
        peer: PeerId,
        inherited: Vec<EntityId>,
    },
    /// A guest was let in: it has what was spawned before it came, and
    /// who owns what.
    Welcomed,
    /// The host is gone, or never answered. The game is over for a guest.
    HostLost,
    /// A message that said something its sender had no right to, in words.
    Refused(String),
}

/// Nowhere to send: the transport of a party of one.
struct Nobody;

impl Transport for Nobody {
    fn send(&mut self, _: PeerId, _: Vec<u8>) {}
    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        Vec::new()
    }
}

/// This peer's place in a game played together. Keep one, and call
/// [`Party::update`] every frame.
pub struct Party {
    me: PeerId,
    link: Reliable<Box<dyn Transport + Send>>,
    /// Everyone else in the game, and when they were last heard from. A
    /// guest knows the host from the start, and the rest from the roster.
    heard: BTreeMap<PeerId, Instant>,
    delta: Delta,
    /// Seconds since the last snapshot went out.
    since: f32,
    welcomed: bool,
    host_lost: bool,
    /// Started to play by itself, with no one to hear it.
    alone: bool,
    /// How often this peer says what it owns.
    pub send_every: f32,
    /// How long a peer may be silent before it counts as gone.
    pub timeout: Duration,
}

impl Party {
    /// A snapshot every 50 ms: twenty a second, a keyframe every ten.
    pub const SEND_EVERY: f32 = 0.05;
    /// Three seconds of silence is gone: long enough for a hitch, short
    /// enough that a friend's crash does not freeze their things for long.
    pub const TIMEOUT: Duration = Duration::from_secs(3);
    /// How long a guest waits for the host to let it in at all: a host
    /// still opening its window, loading its scene, is not gone.
    pub const JOIN_TIMEOUT: Duration = Duration::from_secs(30);

    fn new(me: PeerId, transport: Box<dyn Transport + Send>) -> Self {
        Self {
            me,
            link: Reliable::new(transport),
            heard: BTreeMap::new(),
            delta: Delta::new(10),
            since: 0.0,
            welcomed: me == PeerId::HOST,
            host_lost: false,
            alone: false,
            send_every: Self::SEND_EVERY,
            timeout: Self::TIMEOUT,
        }
    }

    /// A game on its own: the host of nobody.
    pub fn alone() -> Self {
        let mut party = Self::new(PeerId::HOST, Box::new(Nobody));
        party.alone = true;
        party
    }

    /// Whether this is a game on its own rather than one others can join.
    pub fn is_alone(&self) -> bool {
        self.alone
    }

    /// Host a game over `transport`: peer 0, the one guests say hello to.
    pub fn host(transport: impl Transport + Send + 'static) -> Self {
        Self::new(PeerId::HOST, Box::new(transport))
    }

    /// Join the game whose host `transport` reaches, as `me`. The hello
    /// goes out now and again until the host answers.
    pub fn join(transport: impl Transport + Send + 'static, me: PeerId) -> Self {
        let mut party = Self::new(me, Box::new(transport));
        party.heard.insert(PeerId::HOST, Instant::now());
        party.link.send_message(PeerId::HOST, &Message::Hello(me));
        party
    }

    /// The party `RUNITY_NET` and `RUNITY_PEER` describe, or one of one
    /// when there is no `RUNITY_NET`.
    pub fn from_env() -> Result<Self, String> {
        let net = std::env::var(NET_VAR).unwrap_or_default();
        let peer = std::env::var(PEER_VAR).ok();
        Self::from_words(&net, peer.as_deref())
    }

    /// [`Party::from_env`] given the two values.
    pub fn from_words(net: &str, peer: Option<&str>) -> Result<Self, String> {
        let net = net.trim();
        if net.is_empty() {
            return Ok(Self::alone());
        }
        let (role, address) = net.split_once(':').ok_or_else(|| {
            format!("{NET_VAR}=`{net}`: want host:ADDRESS or join:ADDRESS")
        })?;
        let io = |e: std::io::Error| format!("{NET_VAR}=`{net}`: {e}");
        match role {
            "host" => Ok(Self::host(Udp::bind(address, PeerId::HOST).map_err(io)?)),
            "join" => {
                let host: std::net::SocketAddr = address
                    .parse()
                    .map_err(|e| format!("{NET_VAR}=`{net}`: `{address}` is not an address: {e}"))?;
                let me = match peer {
                    Some(text) => PeerId(text.trim().parse().map_err(|_| {
                        format!("{PEER_VAR}=`{text}`: want a number, 1 or more")
                    })?),
                    None => PeerId(EntityId::fresh().raw() as u32 | 1),
                };
                if me == PeerId::HOST {
                    return Err(format!("{PEER_VAR}=0 is the host's; a guest is 1 or more"));
                }
                let any = if host.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
                let mut udp = Udp::bind(any, me).map_err(io)?;
                udp.connect(PeerId::HOST, host);
                Ok(Self::join(udp, me))
            }
            other => Err(format!(
                "{NET_VAR}=`{net}`: `{other}` is not host or join"
            )),
        }
    }

    /// Four peers in one process, for tests: a host and three guests who
    /// have said hello.
    pub fn loopback(count: u32) -> Vec<Self> {
        Loopback::network(count)
            .into_iter()
            .enumerate()
            .map(|(i, end)| {
                if i == 0 {
                    Self::host(end)
                } else {
                    Self::join(end, PeerId(i as u32))
                }
            })
            .collect()
    }

    /// Which peer this is.
    pub fn me(&self) -> PeerId {
        self.me
    }

    pub fn is_host(&self) -> bool {
        self.me == PeerId::HOST
    }

    /// Everyone in the game as this peer knows it, itself included, lowest
    /// first.
    pub fn peers(&self) -> Vec<PeerId> {
        let mut out: Vec<PeerId> = self.heard.keys().copied().collect();
        out.push(self.me);
        out.sort();
        out.dedup();
        out
    }

    /// Whether this peer has what it needs to play along: the host always,
    /// a guest once the host's welcome is in.
    pub fn welcomed(&self) -> bool {
        self.welcomed
    }

    /// How a window says who it is: "Player 1 (host)", "Player 3".
    pub fn name(&self) -> String {
        let number = self.me.0.wrapping_add(1);
        if self.is_host() {
            format!("Player {number} (host)")
        } else {
            format!("Player {number}")
        }
    }

    /// Say something to everyone else: to all guests from the host, to the
    /// host (who passes it on) from a guest.
    pub fn send(&mut self, message: &Message) {
        if self.is_host() {
            for &peer in self.heard.keys() {
                self.link.send_message(peer, message);
            }
        } else {
            self.link.send_message(PeerId::HOST, message);
        }
    }

    /// An entity the game just spawned from `prefab`, made this peer's and
    /// spawned for everyone else too.
    pub fn spawn(&mut self, world: &mut hecs::World, entity: hecs::Entity, prefab: &str) {
        let message = announce(world, entity, self.me, prefab);
        self.send(&message);
    }

    /// Give something this peer owns to `to`. `false` when it is not this
    /// peer's to give, or nobody can name it.
    pub fn hand_over(&mut self, world: &mut hecs::World, entity: hecs::Entity, to: PeerId) -> bool {
        if owner_of(world, entity) != self.me {
            return false;
        }
        let Some(id) = network_id(world, entity) else {
            return false;
        };
        let _ = world.insert_one(entity, Owner(to));
        self.send(&Message::Handover(Handover {
            from: self.me,
            id,
            to,
        }));
        true
    }

    /// Remove something this peer spawned, for everyone. `false` when it is
    /// not this peer's, or was not spawned over the network.
    pub fn despawn(&mut self, world: &mut hecs::World, entity: hecs::Entity) -> bool {
        let Ok(id) = world.get::<&NetId>(entity).map(|n| n.0) else {
            return false;
        };
        if owner_of(world, entity) != self.me {
            return false;
        }
        crate::net::despawn_tree(world, entity);
        self.send(&Message::Despawn(crate::net::Despawn { from: self.me, id }));
        true
    }

    /// One frame of the party: let in who says hello, take what the others
    /// sent, pass it on (the host), notice who went quiet, say what this
    /// peer owns when it is time, and move the others' things along.
    /// `spawn` puts a prefab into the world for a peer's spawn —
    /// [`crate::LiveScene::spawn_prefab`], usually.
    pub fn update(
        &mut self,
        world: &mut hecs::World,
        components: &Components,
        delta: f32,
        mut spawn: impl FnMut(&mut hecs::World, &str, Transform) -> Option<hecs::Entity>,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        let now = Instant::now();
        for (sender, message) in self.link.pump() {
            if self.is_host() {
                self.hear_as_host(world, components, sender, message, now, &mut spawn, &mut events);
            } else {
                self.hear_as_guest(world, components, sender, message, now, &mut spawn, &mut events);
            }
        }
        self.notice_silence(world, now, &mut events);
        self.mark_remote(world);
        self.since += delta;
        if self.since >= self.send_every {
            self.since = 0.0;
            if self.welcomed && !(self.is_host() && self.heard.is_empty()) {
                let snapshot = self.delta.snapshot(world, components, self.me);
                self.send(&Message::Snapshot(snapshot));
            }
        }
        smooth(world, delta);
        events
    }

    #[allow(clippy::too_many_arguments)]
    fn hear_as_host(
        &mut self,
        world: &mut hecs::World,
        components: &Components,
        sender: PeerId,
        message: Message,
        now: Instant,
        spawn: &mut impl FnMut(&mut hecs::World, &str, Transform) -> Option<hecs::Entity>,
        events: &mut Vec<Event>,
    ) {
        match message {
            Message::Hello(peer) => {
                if peer != sender || peer == PeerId::HOST {
                    return;
                }
                if self.heard.insert(peer, now).is_none() {
                    self.link.send_message(peer, &welcome(world));
                    self.send_roster();
                    events.push(Event::Joined(peer));
                }
            }
            // A guest that has not said hello is not in the game.
            _ if !self.heard.contains_key(&sender) => {}
            Message::Bye(peer) if peer == sender => {
                self.leave(world, peer, events);
            }
            Message::Roster(_) | Message::Welcome(_) | Message::Bye(_) => {
                self.heard.insert(sender, now);
            }
            message => {
                self.heard.insert(sender, now);
                // Everyone else hears what one guest says through the host.
                for &peer in self.heard.keys() {
                    if peer != sender {
                        self.link.send_message(peer, &message);
                    }
                }
                take(world, components, &message, spawn, events);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn hear_as_guest(
        &mut self,
        world: &mut hecs::World,
        components: &Components,
        sender: PeerId,
        message: Message,
        now: Instant,
        spawn: &mut impl FnMut(&mut hecs::World, &str, Transform) -> Option<hecs::Entity>,
        events: &mut Vec<Event>,
    ) {
        // A star: everything comes through the host.
        if sender != PeerId::HOST || self.host_lost {
            return;
        }
        self.heard.insert(PeerId::HOST, now);
        match message {
            Message::Welcome(_) => {
                take(world, components, &message, spawn, events);
                if !self.welcomed {
                    self.welcomed = true;
                    events.push(Event::Welcomed);
                }
            }
            Message::Roster(peers) => self.take_roster(world, &peers, now, events),
            Message::Bye(peer) if peer == PeerId::HOST => {
                self.host_lost = true;
                events.push(Event::HostLost);
            }
            Message::Hello(_) | Message::Bye(_) => {}
            // Before the welcome, a snapshot may name things this peer does
            // not have yet; the next keyframe after it will do.
            Message::Snapshot(_) if !self.welcomed => {}
            message => take(world, components, &message, spawn, events),
        }
    }

    /// The host's word on who is here: newcomers joined, and whoever is
    /// missing left — so every guest passes a leaver's things on the same
    /// way the host did, without being told where they went.
    fn take_roster(
        &mut self,
        world: &mut hecs::World,
        peers: &[PeerId],
        now: Instant,
        events: &mut Vec<Event>,
    ) {
        for &peer in peers {
            if peer != self.me && !self.heard.contains_key(&peer) {
                self.heard.insert(peer, now);
                events.push(Event::Joined(peer));
            }
        }
        let gone: Vec<PeerId> = self
            .heard
            .keys()
            .copied()
            .filter(|p| !peers.contains(p))
            .collect();
        for peer in gone {
            self.heard.remove(&peer);
            let inherited = peer_left(world, peer, peers);
            events.push(Event::Left { peer, inherited });
        }
    }

    fn send_roster(&mut self) {
        let roster = Message::Roster(self.peers());
        for &peer in self.heard.keys() {
            self.link.send_message(peer, &roster);
        }
    }

    /// The host hears from every guest many times a second; one silent
    /// for [`Party::timeout`] is gone. A guest only listens for the host.
    fn notice_silence(&mut self, world: &mut hecs::World, now: Instant, events: &mut Vec<Event>) {
        if self.is_host() {
            let quiet: Vec<PeerId> = self
                .heard
                .iter()
                .filter(|(_, at)| now.duration_since(**at) > self.timeout)
                .map(|(p, _)| *p)
                .collect();
            for peer in quiet {
                self.leave(world, peer, events);
            }
        } else if !self.host_lost
            && self.heard.get(&PeerId::HOST).is_some_and(|at| {
                let patience = if self.welcomed {
                    self.timeout
                } else {
                    Self::JOIN_TIMEOUT.max(self.timeout)
                };
                now.duration_since(*at) > patience
            })
        {
            self.host_lost = true;
            events.push(Event::HostLost);
        }
    }

    /// The host lets a guest go: its things to the lowest peer here, and
    /// the new roster to everyone left.
    fn leave(&mut self, world: &mut hecs::World, peer: PeerId, events: &mut Vec<Event>) {
        if self.heard.remove(&peer).is_none() {
            return;
        }
        let inherited = peer_left(world, peer, &self.peers());
        self.send_roster();
        events.push(Event::Left { peer, inherited });
    }

    /// The others' things glide between their snapshots; this peer's own
    /// move only as this peer moves them — including what was just handed
    /// to it, which must stop gliding towards where its old owner left it.
    fn mark_remote(&mut self, world: &mut hecs::World) {
        if self.heard.is_empty() {
            return;
        }
        let mut glide = Vec::new();
        let mut hold = Vec::new();
        for (entity, remote, scene, net) in world
            .query::<(hecs::Entity, Option<&Remote>, Option<&SceneId>, Option<&NetId>)>()
            .with::<&Transform>()
            .iter()
        {
            if scene.is_none() && net.is_none() {
                continue;
            }
            let mine = owner_of(world, entity) == self.me;
            match (mine, remote.is_some()) {
                (false, false) => glide.push(entity),
                (true, true) => hold.push(entity),
                _ => {}
            }
        }
        for entity in glide {
            smooth_this(world, entity);
        }
        for entity in hold {
            let _ = world.remove_one::<Remote>(entity);
        }
    }
}

impl Drop for Party {
    /// Leaving says so, so the others need not wait out the silence.
    fn drop(&mut self) {
        if self.is_host() && self.heard.is_empty() {
            return;
        }
        let bye = Message::Bye(self.me);
        self.send(&bye);
    }
}

/// How the network names an entity: its [`NetId`] if spawned at run time,
/// its scene's ID otherwise.
fn network_id(world: &hecs::World, entity: hecs::Entity) -> Option<EntityId> {
    world
        .get::<&NetId>(entity)
        .map(|n| n.0)
        .ok()
        .or_else(|| world.get::<&SceneId>(entity).map(|s| s.0).ok())
}

/// Apply what a peer said, and say what was refused.
fn take(
    world: &mut hecs::World,
    components: &Components,
    message: &Message,
    spawn: &mut impl FnMut(&mut hecs::World, &str, Transform) -> Option<hecs::Entity>,
    events: &mut Vec<Event>,
) {
    let applied = crate::net::apply_with(world, components, message, |w, prefab, at| spawn(w, prefab, at));
    events.extend(applied.refused.into_iter().map(Event::Refused));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Transform;
    use glam::Vec3;

    /// The same scene loaded by every peer: a crate and a door.
    fn scene_world() -> hecs::World {
        let mut world = hecs::World::new();
        for n in [1u64, 2] {
            world.spawn((SceneId(EntityId::from_raw(n)), Transform::default()));
        }
        world
    }

    fn find(world: &hecs::World, id: u64) -> hecs::Entity {
        world
            .query::<(hecs::Entity, &SceneId)>()
            .iter()
            .find(|(_, s)| s.0 == EntityId::from_raw(id))
            .map(|(e, _)| e)
            .unwrap()
    }

    fn at(world: &hecs::World, entity: hecs::Entity) -> Vec3 {
        world.get::<&Transform>(entity).unwrap().position
    }

    /// Every peer runs a frame of 50 ms, the host first.
    fn frame(
        parties: &mut [Party],
        worlds: &mut [hecs::World],
        components: &Components,
    ) -> Vec<Vec<Event>> {
        parties
            .iter_mut()
            .zip(worlds.iter_mut())
            .map(|(party, world)| party.update(world, components, 0.05, |_, _, _| None))
            .collect()
    }

    fn frames(n: usize, parties: &mut [Party], worlds: &mut [hecs::World], components: &Components) -> Vec<Event> {
        let mut all = Vec::new();
        for _ in 0..n {
            for events in frame(parties, worlds, components) {
                all.extend(events);
            }
        }
        all
    }

    #[test]
    fn players_windows_lie_side_by_side_at_half_the_size() {
        assert_eq!(tile(0, 1, (1280, 720)), "40,60,1280,720");
        assert_eq!(tile(0, 4, (1280, 720)), "40,60,640,360");
        assert_eq!(tile(1, 4, (1280, 720)), "696,60,640,360");
        assert_eq!(tile(2, 4, (1280, 720)), "40,468,640,360");
        assert_eq!(tile(3, 4, (1280, 720)), "696,468,640,360");
    }

    #[test]
    fn alone_is_a_party_of_one_that_says_nothing() {
        let mut party = Party::from_words("", None).unwrap();
        let mut world = scene_world();
        let events = party.update(&mut world, &Components::new(), 1.0, |_, _, _| None);
        assert!(events.is_empty());
        assert!(party.is_host() && party.welcomed() && party.is_alone());
        assert_eq!(party.peers(), [PeerId::HOST]);
        assert_eq!(party.name(), "Player 1 (host)");
    }

    #[test]
    fn four_friends_come_in_and_all_know_each_other() {
        let components = Components::new();
        let mut parties = Party::loopback(4);
        let mut worlds: Vec<_> = (0..4).map(|_| scene_world()).collect();
        let events = frames(3, &mut parties, &mut worlds, &components);
        for party in &parties {
            assert_eq!(party.peers(), (0..4).map(PeerId).collect::<Vec<_>>());
            assert!(party.welcomed());
        }
        assert_eq!(
            events.iter().filter(|e| **e == Event::Welcomed).count(),
            3,
            "{events:?}"
        );
        assert_eq!(parties[2].name(), "Player 3");
    }

    #[test]
    fn what_the_host_moves_every_guest_sees_and_a_guest_cannot_move_it() {
        let components = Components::new();
        let mut parties = Party::loopback(3);
        let mut worlds: Vec<_> = (0..3).map(|_| scene_world()).collect();
        frames(3, &mut parties, &mut worlds, &components);

        let host_crate = find(&worlds[0], 1);
        worlds[0].get::<&mut Transform>(host_crate).unwrap().position = Vec3::new(4.0, 0.0, 0.0);
        // A guest's own system pushes the host's crate somewhere else; the
        // host's word wins.
        let guest_crate = find(&worlds[2], 1);
        worlds[2].get::<&mut Transform>(guest_crate).unwrap().position = Vec3::new(-9.0, 0.0, 0.0);
        frames(10, &mut parties, &mut worlds, &components);
        for world in &worlds {
            assert!(at(world, find(world, 1)).distance(Vec3::new(4.0, 0.0, 0.0)) < 1e-3);
        }
    }

    #[test]
    fn a_guest_picks_the_crate_up_and_the_others_see_it_through_the_host() {
        let components = Components::new();
        let mut parties = Party::loopback(3);
        let mut worlds: Vec<_> = (0..3).map(|_| scene_world()).collect();
        frames(3, &mut parties, &mut worlds, &components);

        // The host hands the crate to guest 1, who carries it off.
        let host_crate = find(&worlds[0], 1);
        assert!(parties[0].hand_over(&mut worlds[0], host_crate, PeerId(1)));
        assert!(!parties[0].hand_over(&mut worlds[0], host_crate, PeerId(2)), "no longer the host's");
        frames(3, &mut parties, &mut worlds, &components);
        let carried = find(&worlds[1], 1);
        assert_eq!(owner_of(&worlds[1], carried), PeerId(1));
        worlds[1].get::<&mut Transform>(carried).unwrap().position = Vec3::new(0.0, 2.0, 3.0);
        frames(10, &mut parties, &mut worlds, &components);
        // In the carrier's hands it stays where they put it, and guest 2
        // sees it there, though guest 1 never spoke to guest 2.
        for world in &worlds {
            let e = find(world, 1);
            assert_eq!(owner_of(world, e), PeerId(1));
            assert!(at(world, e).distance(Vec3::new(0.0, 2.0, 3.0)) < 1e-3, "{:?}", at(world, e));
        }
    }

    #[test]
    fn a_friend_who_goes_quiet_leaves_their_things_to_the_host() {
        let components = Components::new();
        let mut parties = Party::loopback(3);
        let mut worlds: Vec<_> = (0..3).map(|_| scene_world()).collect();
        for party in &mut parties {
            party.timeout = Duration::from_millis(100);
        }
        frames(3, &mut parties, &mut worlds, &components);
        let door = find(&worlds[0], 2);
        assert!(parties[0].hand_over(&mut worlds[0], door, PeerId(2)));
        frames(3, &mut parties, &mut worlds, &components);

        // Guest 2's game stops answering (a crash: no bye).
        let crashed = parties.pop().unwrap();
        std::mem::forget(crashed);
        worlds.pop();
        // What it said last is taken in; then nothing more comes.
        frames(1, &mut parties, &mut worlds, &components);
        std::thread::sleep(Duration::from_millis(150));
        let events = frames(3, &mut parties, &mut worlds, &components);
        let left = Event::Left {
            peer: PeerId(2),
            inherited: vec![EntityId::from_raw(2)],
        };
        assert_eq!(events.iter().filter(|e| **e == left).count(), 2, "{events:?}");
        for (party, world) in parties.iter().zip(&worlds) {
            assert_eq!(party.peers(), [PeerId(0), PeerId(1)]);
            assert_eq!(owner_of(world, find(world, 2)), PeerId::HOST);
        }
    }

    #[test]
    fn leaving_on_purpose_is_noticed_at_once() {
        let components = Components::new();
        let mut parties = Party::loopback(3);
        let mut worlds: Vec<_> = (0..3).map(|_| scene_world()).collect();
        frames(3, &mut parties, &mut worlds, &components);
        drop(parties.pop());
        worlds.pop();
        let events = frames(2, &mut parties, &mut worlds, &components);
        assert!(events.contains(&Event::Left { peer: PeerId(2), inherited: vec![] }), "{events:?}");
        assert_eq!(parties[1].peers(), [PeerId(0), PeerId(1)]);
    }

    #[test]
    fn the_host_leaving_ends_the_game_for_the_guests() {
        let components = Components::new();
        let mut parties = Party::loopback(2);
        let mut worlds: Vec<_> = (0..2).map(|_| scene_world()).collect();
        frames(3, &mut parties, &mut worlds, &components);
        let host = parties.remove(0);
        worlds.remove(0);
        drop(host);
        let events = frames(2, &mut parties, &mut worlds, &components);
        assert_eq!(events, [Event::HostLost]);
    }

    #[test]
    fn late_joiners_get_what_was_spawned_before_them() {
        let components = Components::new();
        let mut ends = Loopback::network(2);
        let mut host = Party::host(ends.remove(0));
        let mut host_world = scene_world();
        let torch = host_world.spawn((Transform::default(),));
        host.spawn(&mut host_world, torch, "torch");
        host.update(&mut host_world, &components, 0.05, |_, _, _| None);

        let mut guest = Party::join(ends.remove(0), PeerId(1));
        let mut guest_world = scene_world();
        let mut spawned = Vec::new();
        for _ in 0..3 {
            host.update(&mut host_world, &components, 0.05, |_, _, _| None);
            guest.update(&mut guest_world, &components, 0.05, |w, prefab, t| {
                spawned.push(prefab.to_string());
                Some(w.spawn((t,)))
            });
        }
        assert_eq!(spawned, ["torch"]);
        assert!(guest.welcomed());
    }

    #[test]
    fn over_real_sockets_too() {
        let components = Components::new();
        // A free port, as the editor picks one for the host.
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let mut host = Party::from_words(&format!("host:127.0.0.1:{port}"), None).unwrap();
        let mut guest = Party::from_words(&format!("join:127.0.0.1:{port}"), Some("1")).unwrap();
        let (mut hw, mut gw) = (scene_world(), scene_world());
        let e = find(&hw, 1);
        hw.get::<&mut Transform>(e).unwrap().position = Vec3::X;
        for _ in 0..40 {
            host.update(&mut hw, &components, 0.05, |_, _, _| None);
            guest.update(&mut gw, &components, 0.05, |_, _, _| None);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(host.peers(), [PeerId(0), PeerId(1)]);
        assert!(at(&gw, find(&gw, 1)).distance(Vec3::X) < 1e-3);
    }

    #[test]
    fn the_words_that_start_a_party_are_checked() {
        for (net, peer, says) in [
            ("somewhere", None, "host:ADDRESS"),
            ("visit:1.2.3.4:5", None, "not host or join"),
            ("join:nowhere", Some("1"), "not an address"),
            ("join:127.0.0.1:9", Some("x"), "want a number"),
            ("join:127.0.0.1:9", Some("0"), "the host's"),
        ] {
            let Err(e) = Party::from_words(net, peer) else {
                panic!("{net} should not start a party");
            };
            assert!(e.contains(says), "{e}");
        }
    }
}
