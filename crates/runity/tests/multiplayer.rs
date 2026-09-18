//! A server and two clients, over real sockets, replicating a world.
//!
//! This is where the pieces have to agree with each other: change detection
//! decides what to send, the serializer turns it into bytes, the connection
//! gets those bytes across a loopback socket, and the replica puts them back
//! into a world whose entity handles are its own.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use runity::net::{Host, HostEvent, UdpTransport};
use runity::prelude::*;

const DT: f32 = 1.0 / 60.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Position(Vec3);
#[derive(Debug, Clone, Copy, PartialEq)]
struct Carrying(u32);

serializable!(Position(0));
serializable!(Carrying(0));

fn registry() -> Replication {
    let mut registry = Replication::new();
    registry.register::<Position>(1);
    registry.register::<Carrying>(2);
    registry
}

/// The authoritative side: a world, and one acknowledged tick per client.
struct Server {
    world: World,
    registry: Replication,
    host: Host<UdpTransport>,
    address: SocketAddr,
    acked: Vec<(SocketAddr, u64)>,
    /// When set, clients are told only about entities within this radius of
    /// the origin.
    interest: Option<f32>,
}

impl Server {
    fn new() -> Self {
        let transport =
            UdpTransport::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).unwrap();
        let address = transport.local_address().unwrap();
        Server {
            world: World::new(),
            registry: registry(),
            host: Host::new(transport, 8),
            address,
            acked: Vec::new(),
            interest: None,
        }
    }

    /// Tell each client what it has missed.
    ///
    /// Called *after* the tick's systems have run: a change is stamped with
    /// the tick that was current when it happened, so a snapshot taken before
    /// the systems run describes the tick before.
    fn broadcast(&mut self) {
        let peers: Vec<SocketAddr> = self.host.peers().map(|(address, _)| address).collect();
        for peer in peers {
            let since = self
                .acked
                .iter()
                .find(|(address, _)| *address == peer)
                .map(|(_, tick)| *tick)
                .unwrap_or(0);
            let snapshot = match self.interest {
                None => self.registry.snapshot_all(&self.world, since),
                Some(radius) => {
                    let interest = Nearby {
                        centre: Vec3::ZERO,
                        radius,
                        position: |world: &World, entity: Entity| {
                            world.get::<Position>(entity).map(|position| position.0)
                        },
                    };
                    self.registry.snapshot(&self.world, since, &interest)
                }
            };
            self.host.send_reliable(peer, &snapshot);
        }
        // Everything every client has seen can be forgotten.
        let oldest = self.acked.iter().map(|(_, tick)| *tick).min().unwrap_or(0);
        self.world.forget_despawns_before(oldest);
    }

    /// Read acknowledgements and connections.
    fn pump(&mut self) {
        for event in self.host.update(DT).unwrap() {
            match event {
                HostEvent::Connected(address) => self.acked.push((address, 0)),
                HostEvent::Disconnected(address) => self.acked.retain(|(peer, _)| *peer != address),
                HostEvent::Message { from, bytes } => {
                    // The only thing a client says here is "I have this tick".
                    if let Ok(tick) = runity::serialize::from_bytes::<u64>(&bytes) {
                        if let Some(entry) = self.acked.iter_mut().find(|(peer, _)| *peer == from) {
                            entry.1 = entry.1.max(tick);
                        }
                    }
                }
            }
        }
    }
}

/// A client: its own world, a replica, and a connection.
struct Client {
    world: World,
    registry: Replication,
    replica: Replica,
    host: Host<UdpTransport>,
    server: SocketAddr,
}

impl Client {
    fn new(server: SocketAddr) -> Self {
        let transport =
            UdpTransport::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).unwrap();
        let mut host = Host::new(transport, 1);
        host.connect(server);
        Client {
            world: World::new(),
            registry: registry(),
            replica: Replica::new(),
            host,
            server,
        }
    }

    fn pump(&mut self) {
        let mut applied = false;
        for event in self.host.update(DT).unwrap() {
            if let HostEvent::Message { bytes, .. } = event {
                self.replica
                    .apply(&mut self.world, &self.registry, &bytes)
                    .expect("a valid snapshot");
                applied = true;
            }
        }
        if applied {
            let ack = runity::serialize::to_bytes(&self.replica.tick());
            self.host.send_reliable(self.server, &ack);
        }
    }

    fn position_of(&self, remote: Entity) -> Option<Vec3> {
        let local = self.replica.local(remote)?;
        self.world.get::<Position>(local).map(|position| position.0)
    }
}

/// Run everything for a while.
fn run(
    server: &mut Server,
    clients: &mut [&mut Client],
    seconds: f32,
    mut each_tick: impl FnMut(&mut Server),
) {
    let steps = (seconds / DT).round() as usize;
    for step in 0..steps {
        server.pump();
        for client in clients.iter_mut() {
            client.pump();
        }
        // Ten world ticks a second against a 60 Hz pump: advance, run the
        // "systems", then send what they changed.
        if step % 6 == 0 {
            server.world.advance_tick();
            each_tick(server);
            server.broadcast();
        }
    }
}

#[test]
fn two_clients_see_the_world_the_server_holds() {
    let mut server = Server::new();
    let mut first = Client::new(server.address);
    let mut second = Client::new(server.address);

    let villager = server.world.spawn();
    server.world.insert(villager, Position(vec3(1.0, 0.0, 1.0)));
    server.world.insert(villager, Carrying(0));

    run(&mut server, &mut [&mut first, &mut second], 1.0, |_| {});
    assert_eq!(server.host.peer_count(), 2);
    assert_eq!(first.position_of(villager), Some(vec3(1.0, 0.0, 1.0)));
    assert_eq!(second.position_of(villager), Some(vec3(1.0, 0.0, 1.0)));

    // The world moves on, and both copies follow.
    run(&mut server, &mut [&mut first, &mut second], 2.0, |server| {
        let mut position = server.world.get_mut::<Position>(villager).unwrap();
        position.0.x += 0.5;
    });

    let expected = server.world.get::<Position>(villager).unwrap().0;
    assert_eq!(first.position_of(villager), Some(expected));
    assert_eq!(second.position_of(villager), Some(expected));
    assert!(
        expected.x > 5.0,
        "the villager should have walked a way: {expected:?}"
    );
}

#[test]
fn a_still_world_costs_almost_nothing_to_replicate() {
    let mut server = Server::new();
    let mut client = Client::new(server.address);

    for index in 0..200 {
        let entity = server.world.spawn();
        server
            .world
            .insert(entity, Position(vec3(index as f32, 0.0, 0.0)));
        server.world.insert(entity, Carrying(index));
    }

    run(&mut server, &mut [&mut client], 1.0, |_| {});
    assert_eq!(
        client.world.count::<Position>(),
        200,
        "the whole world arrives once"
    );

    // Now nothing happens for two seconds.
    let before = server.host.peers().next().unwrap().1.stats().packets_sent;
    run(&mut server, &mut [&mut client], 2.0, |_| {});
    let after = server.host.peers().next().unwrap().1.stats().packets_sent;

    // Keep-alives still flow, but an unchanging world adds no snapshots worth
    // speaking of: 200 entities would be several packets per tick.
    let packets = after - before;
    assert!(
        packets < 60,
        "an idle world should not cost {packets} packets in two seconds"
    );
}

#[test]
fn a_despawn_reaches_a_client_that_is_several_ticks_behind() {
    let mut server = Server::new();
    let mut client = Client::new(server.address);

    let doomed = server.world.spawn();
    server.world.insert(doomed, Position(Vec3::ZERO));
    run(&mut server, &mut [&mut client], 0.6, |_| {});
    let local = client
        .replica
        .local(doomed)
        .expect("it should have arrived");
    assert!(client.world.is_alive(local));

    // The server ticks several times before the client hears anything.
    for _ in 0..5 {
        server.world.advance_tick();
    }
    server.world.despawn(doomed);
    for _ in 0..5 {
        server.world.advance_tick();
    }

    run(&mut server, &mut [&mut client], 1.0, |_| {});
    assert!(
        !client.world.is_alive(local),
        "the client's copy has to go too"
    );
    assert!(client.replica.local(doomed).is_none());
}

#[test]
fn interest_keeps_the_far_half_of_the_world_off_the_wire() {
    let mut server = Server::new();
    let mut client = Client::new(server.address);

    let near = server.world.spawn();
    let far = server.world.spawn();
    server.world.insert(near, Position(vec3(5.0, 0.0, 0.0)));
    server.world.insert(far, Position(vec3(900.0, 0.0, 0.0)));

    server.interest = Some(100.0);
    run(&mut server, &mut [&mut client], 1.0, |_| {});

    assert!(client.replica.local(near).is_some(), "the near one arrives");
    assert!(
        client.position_of(far).is_none(),
        "and the far one is not on the wire at all, so it cannot be read out of the client"
    );
}

#[test]
fn a_reconnecting_client_starts_over_cleanly() {
    let mut server = Server::new();
    let mut client = Client::new(server.address);

    let entity = server.world.spawn();
    server.world.insert(entity, Carrying(3));
    run(&mut server, &mut [&mut client], 0.8, |_| {});
    assert!(client.replica.local(entity).is_some());

    client.host.disconnect(server.address).unwrap();
    run(&mut server, &mut [&mut client], 0.5, |_| {});
    assert_eq!(server.host.peer_count(), 0);

    // A fresh client, and a fresh replica: the server's handles mean nothing
    // to a world that has been rebuilt.
    let mut returning = Client::new(server.address);
    run(&mut server, &mut [&mut returning], 1.0, |_| {});
    assert_eq!(server.host.peer_count(), 1);
    assert_eq!(
        returning
            .world
            .get::<Carrying>(returning.replica.local(entity).unwrap()),
        Some(&Carrying(3))
    );
}

/// Intent, not keystrokes.
///
/// The claim the action layer makes is that what a player wanted can be sent
/// to another machine and replayed there to the same result. This drives a
/// character controller on two independent physics worlds — one from the
/// local actions, one from the bytes those actions serialized to — and
/// requires that they end in the same place, exactly.
#[test]
fn a_remote_machine_replays_an_intent_to_the_same_position() {
    use runity::physics::{Character, CharacterSettings};
    use runity::platform::{Event, Key};
    use runity::serialize::{from_bytes, to_bytes};

    const WALK: Action = Action::named("walk");
    const STRAFE: Action = Action::named("strafe");
    const JUMP: Action = Action::named("jump");

    let mut set = ActionSet::new();
    set.button(JUMP);
    set.axis(WALK).axis(STRAFE);

    let mut bindings = Bindings::new();
    bindings
        .key(JUMP, Key::Space)
        .key_axis(WALK, Key::S, Key::W)
        .key_axis(STRAFE, Key::A, Key::D);
    let mut actions = Actions::new(bindings);
    let mut input = Input::new();

    // Two worlds that never talk to each other except through the wire.
    let mut here = PhysicsWorld::new();
    let mut there = PhysicsWorld::new();
    here.add(RigidBody::fixed(Shape::ground()));
    there.add(RigidBody::fixed(Shape::ground()));
    let settings = CharacterSettings::default();
    let mut local = Character::new(settings, vec3(0.0, 0.2, 0.0));
    let mut remote = Character::new(settings, vec3(0.0, 0.2, 0.0));

    // A scripted sixty ticks of held keys, changing partway through.
    let dt = 1.0 / 60.0;
    for tick in 0..60 {
        input.begin_frame();
        let events: &[Event] = match tick {
            0 => &[Event::KeyDown(Key::W)],
            20 => &[Event::KeyDown(Key::D), Event::KeyDown(Key::Space)],
            40 => &[Event::KeyUp(Key::W)],
            _ => &[],
        };
        for event in events {
            input.handle(event);
        }
        actions.update(&input);

        let intent = actions.intent(&set);
        // Everything below the line is what the other machine has: bytes.
        let bytes = to_bytes(&intent);
        let received: Intent = from_bytes(&bytes).expect("an intent off the wire");

        step_from_intent(&mut local, &mut here, &intent, &set, dt);
        step_from_intent(&mut remote, &mut there, &received, &set, dt);
    }

    assert_eq!(
        local.position, remote.position,
        "the same intent must produce the same position, bit for bit"
    );
    assert!(
        local.position.z < -0.5,
        "and it should actually have walked: {:?}",
        local.position
    );
}

/// Move a character the way a game would, reading only the intent.
fn step_from_intent(
    character: &mut runity::physics::Character,
    world: &mut PhysicsWorld,
    intent: &Intent,
    set: &ActionSet,
    dt: f32,
) {
    const WALK: Action = Action::named("walk");
    const STRAFE: Action = Action::named("strafe");
    const JUMP: Action = Action::named("jump");

    if intent.down(set, JUMP) {
        character.jump();
    }
    let wish = vec3(intent.value(set, STRAFE), 0.0, -intent.value(set, WALK));
    character.step(world, wish * 3.0, dt);
}
