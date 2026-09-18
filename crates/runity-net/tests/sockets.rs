//! The socket layer, over loopback: a server, two clients, and a handshake.
//!
//! These are the only tests here that touch the operating system. Everything
//! about *reliability* is tested against a simulated link instead, because a
//! real socket cannot be asked to lose a third of its packets on cue.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use runity_net::{Host, HostEvent, UdpTransport};

const DT: f32 = 1.0 / 60.0;

fn host(max_peers: usize) -> (Host<UdpTransport>, SocketAddr) {
    let transport =
        UdpTransport::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).expect("bind");
    let address = transport
        .local_address()
        .expect("a bound socket has an address");
    (Host::new(transport, max_peers), address)
}

/// Run both ends for a while, collecting everything that happened.
fn pump(
    server: &mut Host<UdpTransport>,
    clients: &mut [&mut Host<UdpTransport>],
    seconds: f32,
) -> (Vec<HostEvent<SocketAddr>>, Vec<Vec<HostEvent<SocketAddr>>>) {
    let mut server_events = Vec::new();
    let mut client_events: Vec<Vec<HostEvent<SocketAddr>>> =
        clients.iter().map(|_| Vec::new()).collect();
    let steps = (seconds / DT).round() as usize;

    for _ in 0..steps {
        server_events.extend(server.update(DT).expect("the server keeps running"));
        for (index, client) in clients.iter_mut().enumerate() {
            client_events[index].extend(client.update(DT).expect("the client keeps running"));
        }
    }
    (server_events, client_events)
}

fn messages(events: &[HostEvent<SocketAddr>]) -> Vec<Vec<u8>> {
    events
        .iter()
        .filter_map(|event| match event {
            HostEvent::Message { bytes, .. } => Some(bytes.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_client_connects_to_a_server_and_they_talk() {
    let (mut server, server_address) = host(4);
    let (mut client, _) = host(1);
    client.connect(server_address);

    let (server_events, client_events) = pump(&mut server, &mut [&mut client], 1.0);

    assert!(
        server_events
            .iter()
            .any(|event| matches!(event, HostEvent::Connected(_))),
        "the server should have noticed the client"
    );
    assert!(
        client_events[0]
            .iter()
            .any(|event| matches!(event, HostEvent::Connected(_))),
        "and the client should have been let in"
    );
    assert_eq!(server.peer_count(), 1);
    assert!(client.is_connected(server_address));

    // Both directions, reliable and unreliable.
    let peer = server.peers().next().expect("one peer").0;
    server.send_reliable(peer, b"welcome to the valley");
    client.send_reliable(server_address, b"glad to be here");
    client.send_unreliable(server_address, b"and here is where I am");

    let (server_events, client_events) = pump(&mut server, &mut [&mut client], 1.0);
    assert!(messages(&client_events[0]).contains(&b"welcome to the valley".to_vec()));
    let received = messages(&server_events);
    assert!(received.contains(&b"glad to be here".to_vec()));
    assert!(received.contains(&b"and here is where I am".to_vec()));
}

#[test]
fn a_server_holds_several_clients_and_broadcasts_to_all_of_them() {
    let (mut server, server_address) = host(4);
    let (mut first, _) = host(1);
    let (mut second, _) = host(1);
    first.connect(server_address);
    second.connect(server_address);

    pump(&mut server, &mut [&mut first, &mut second], 1.0);
    assert_eq!(server.peer_count(), 2);

    server.broadcast_reliable(b"the granary is finished");
    let (_, client_events) = pump(&mut server, &mut [&mut first, &mut second], 1.0);

    for events in &client_events {
        assert!(
            messages(events).contains(&b"the granary is finished".to_vec()),
            "every client should hear a broadcast"
        );
    }
}

#[test]
fn a_full_server_ignores_further_clients() {
    let (mut server, server_address) = host(1);
    let (mut first, _) = host(1);
    let (mut second, _) = host(1);
    first.connect(server_address);
    pump(&mut server, &mut [&mut first], 0.5);
    assert_eq!(server.peer_count(), 1);

    second.connect(server_address);
    pump(&mut server, &mut [&mut second], 0.5);
    assert_eq!(
        server.peer_count(),
        1,
        "the second client should not get in"
    );
    assert!(!second.is_connected(server_address));
}

#[test]
fn a_goodbye_is_noticed_at_once() {
    let (mut server, server_address) = host(4);
    let (mut client, _) = host(1);
    client.connect(server_address);
    pump(&mut server, &mut [&mut client], 0.5);
    assert_eq!(server.peer_count(), 1);

    client
        .disconnect(server_address)
        .expect("saying goodbye should work");
    let (server_events, _) = pump(&mut server, &mut [&mut client], 0.3);

    assert!(
        server_events
            .iter()
            .any(|event| matches!(event, HostEvent::Disconnected(_))),
        "the server should not have to wait five seconds to find out"
    );
    assert_eq!(server.peer_count(), 0);
}

#[test]
fn connecting_to_nobody_gives_up_rather_than_hanging() {
    let (mut client, _) = host(1);
    // Port 1 on loopback: nothing is listening, and nothing will be.
    let nowhere = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1);
    client.connect(nowhere);

    let mut events = Vec::new();
    for _ in 0..(7.0 / DT) as usize {
        events.extend(
            client
                .update(DT)
                .expect("failing to connect is not an io error"),
        );
    }
    assert!(
        events
            .iter()
            .any(|event| matches!(event, HostEvent::Disconnected(_))),
        "the attempt should time out"
    );
    assert!(!client.is_connected(nowhere));
}

#[test]
fn a_large_message_crosses_a_real_socket() {
    let (mut server, server_address) = host(4);
    let (mut client, _) = host(1);
    client.connect(server_address);
    pump(&mut server, &mut [&mut client], 0.5);

    // Bigger than a datagram, so it has to be split and put back together.
    let snapshot: Vec<u8> = (0..30_000u32).map(|i| (i % 251) as u8).collect();
    let peer = server.peers().next().expect("one peer").0;
    server.send_reliable(peer, &snapshot);

    let (_, client_events) = pump(&mut server, &mut [&mut client], 3.0);
    assert_eq!(messages(&client_events[0]), vec![snapshot]);
}
