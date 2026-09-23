//! Connections over a wire: a reliable, ordered plane and an unreliable
//! one, and knowing when the other end is there.
//!
//! Over a [`Transport`] that promises nothing, a [`Link`] keeps one
//! connection per endpoint. What is sent [`Mode::Reliable`] arrives once,
//! in the order sent — control messages lean on that: a spawn before the
//! snapshots of the thing, a grant after the spawn. What is sent
//! [`Mode::Unreliable`] arrives or not, in any order: snapshots, where a
//! lost one is superseded by the next. A connection is up from the first
//! datagram heard, kept up by a ping when there is nothing else to say,
//! and down after [`Link::timeout`] of silence — or at once when the other
//! end says goodbye, which is how "they left" and "we stopped hearing
//! them" stay two different sentences (the dacha simulator's
//! `DisconnectKind`).
//!
//! Frames: `0 payload` unreliable, `1 seq payload` reliable, `2 seq` its
//! acknowledgement, `3` a ping, `4` goodbye.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use super::wire::Transport;
use super::PeerId;

/// How a message travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Reliable,
    Unreliable,
}

/// How a connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The other end said goodbye.
    Clean,
    /// It went quiet.
    Silent,
}

/// What happened on a link since it was last polled.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEvent {
    Connected(PeerId),
    /// A message, and how it came.
    Data(PeerId, Vec<u8>, Mode),
    Disconnected(PeerId, Ended),
}

const UNRELIABLE: u8 = 0;
const RELIABLE: u8 = 1;
const ACK: u8 = 2;
const PING: u8 = 3;
const BYE: u8 = 4;

struct Connection {
    /// Heard from at all yet.
    up: bool,
    heard: Instant,
    sent: Instant,
    next_send: u64,
    unacked: BTreeMap<u64, (Vec<u8>, Instant)>,
    next_expected: u64,
    held: BTreeMap<u64, Vec<u8>>,
}

impl Connection {
    fn new(now: Instant) -> Self {
        Self {
            up: false,
            heard: now,
            sent: now,
            next_send: 1,
            unacked: BTreeMap::new(),
            next_expected: 1,
            held: BTreeMap::new(),
        }
    }
}

/// Connections to every endpoint of one wire.
pub struct Link {
    wire: Box<dyn Transport + Send>,
    connections: HashMap<PeerId, Connection>,
    /// Whether an endpoint nobody dialled may connect: a server's link.
    accepting: bool,
    /// Silence after which a connection is down.
    pub timeout: Duration,
    /// How long a dial waits for its first answer.
    pub dial_timeout: Duration,
}

impl Link {
    /// Resend what is not acknowledged after this long.
    pub const RESEND: Duration = Duration::from_millis(100);
    /// Say something at least this often, so silence means gone.
    pub const PING: Duration = Duration::from_millis(250);

    /// A link that lets anyone connect: a server's.
    pub fn accepting(wire: impl Transport + Send + 'static) -> Self {
        Self::new(Box::new(wire), true)
    }

    /// A link that talks only to whom it dials: a client's.
    pub fn dialling(wire: impl Transport + Send + 'static) -> Self {
        Self::new(Box::new(wire), false)
    }

    fn new(wire: Box<dyn Transport + Send>, accepting: bool) -> Self {
        Self {
            wire,
            connections: HashMap::new(),
            accepting,
            timeout: Duration::from_secs(3),
            dial_timeout: Duration::from_secs(30),
        }
    }

    /// Start talking to `peer`; [`LinkEvent::Connected`] when it answers,
    /// [`LinkEvent::Disconnected`] if it never does. Dialling again starts
    /// over.
    pub fn dial(&mut self, peer: PeerId) {
        let now = Instant::now();
        self.connections.insert(peer, Connection::new(now));
        self.wire.send(peer, vec![PING]);
    }

    pub fn is_connected(&self, peer: PeerId) -> bool {
        self.connections.get(&peer).is_some_and(|c| c.up)
    }

    /// Send to a connected (or dialled) endpoint. Nothing happens for one
    /// the link does not know.
    pub fn send(&mut self, to: PeerId, payload: Vec<u8>, mode: Mode) {
        let Some(connection) = self.connections.get_mut(&to) else {
            return;
        };
        let now = Instant::now();
        connection.sent = now;
        match mode {
            Mode::Unreliable => {
                let mut frame = Vec::with_capacity(payload.len() + 1);
                frame.push(UNRELIABLE);
                frame.extend_from_slice(&payload);
                self.wire.send(to, frame);
            }
            Mode::Reliable => {
                let seq = connection.next_send;
                connection.next_send += 1;
                let mut frame = Vec::with_capacity(payload.len() + 9);
                frame.push(RELIABLE);
                frame.extend_from_slice(&seq.to_le_bytes());
                frame.extend_from_slice(&payload);
                connection.unacked.insert(seq, (frame.clone(), now));
                self.wire.send(to, frame);
            }
        }
    }

    /// Say goodbye and forget the endpoint. The goodbye is sent a few
    /// times: it is not acknowledged, and it is the only thing that makes
    /// the other side's event [`Ended::Clean`].
    pub fn close(&mut self, peer: PeerId) {
        if self.connections.remove(&peer).is_some() {
            for _ in 0..3 {
                self.wire.send(peer, vec![BYE]);
            }
        }
    }

    /// Close every connection.
    pub fn close_all(&mut self) {
        let peers: Vec<PeerId> = self.connections.keys().copied().collect();
        for peer in peers {
            self.close(peer);
        }
    }

    /// Resend what is overdue, ping what is quiet, take in what arrived,
    /// and say what happened — every reliable message once, in order.
    pub fn poll(&mut self) -> Vec<LinkEvent> {
        let now = Instant::now();
        let mut events = Vec::new();
        for (peer, connection) in &mut self.connections {
            for (frame, sent) in connection.unacked.values_mut() {
                if now.duration_since(*sent) >= Self::RESEND {
                    self.wire.send(*peer, frame.clone());
                    *sent = now;
                    connection.sent = now;
                }
            }
            if now.duration_since(connection.sent) >= Self::PING {
                self.wire.send(*peer, vec![PING]);
                connection.sent = now;
            }
        }
        for (from, frame) in self.wire.receive() {
            let Some(&kind) = frame.first() else {
                continue;
            };
            if !self.connections.contains_key(&from) {
                // A goodbye from nobody we know, or a stranger at a door
                // that does not open.
                if kind == BYE || !self.accepting {
                    continue;
                }
                self.connections.insert(from, Connection::new(now));
                // Answered at once, so the dialler knows it is through
                // without waiting for the first ping.
                self.wire.send(from, vec![PING]);
            }
            let Some(connection) = self.connections.get_mut(&from) else {
                continue;
            };
            connection.heard = now;
            if !connection.up && kind != BYE {
                connection.up = true;
                events.push(LinkEvent::Connected(from));
            }
            match kind {
                UNRELIABLE => {
                    events.push(LinkEvent::Data(from, frame[1..].to_vec(), Mode::Unreliable))
                }
                RELIABLE if frame.len() >= 9 => {
                    let seq = u64::from_le_bytes(frame[1..9].try_into().expect("eight bytes"));
                    let mut ack = vec![ACK];
                    ack.extend_from_slice(&seq.to_le_bytes());
                    self.wire.send(from, ack);
                    if seq >= connection.next_expected {
                        connection.held.insert(seq, frame[9..].to_vec());
                    }
                    while let Some(payload) = connection.held.remove(&connection.next_expected) {
                        events.push(LinkEvent::Data(from, payload, Mode::Reliable));
                        connection.next_expected += 1;
                    }
                }
                ACK if frame.len() >= 9 => {
                    let seq = u64::from_le_bytes(frame[1..9].try_into().expect("eight bytes"));
                    connection.unacked.remove(&seq);
                }
                BYE => {
                    self.connections.remove(&from);
                    events.push(LinkEvent::Disconnected(from, Ended::Clean));
                }
                _ => {}
            }
        }
        let quiet: Vec<PeerId> = self
            .connections
            .iter()
            .filter(|(_, c)| {
                let patience = if c.up {
                    self.timeout
                } else {
                    self.dial_timeout
                };
                now.duration_since(c.heard) > patience
            })
            .map(|(p, _)| *p)
            .collect();
        for peer in quiet {
            self.connections.remove(&peer);
            events.push(LinkEvent::Disconnected(peer, Ended::Silent));
        }
        events
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.close_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::wire::{Conditions, Laggy, Loopback};

    fn pair(conditions: Conditions) -> (Link, Link) {
        let mut ends = Loopback::network(2);
        let client = ends.pop().unwrap();
        let server = ends.pop().unwrap();
        let server = Link::accepting(server);
        let mut client = Link::dialling(Laggy::new(client, conditions, 3));
        client.dial(PeerId(0));
        (server, client)
    }

    fn data(events: &[LinkEvent]) -> Vec<(Vec<u8>, Mode)> {
        events
            .iter()
            .filter_map(|e| match e {
                LinkEvent::Data(_, bytes, mode) => Some((bytes.clone(), *mode)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_dial_is_answered_and_both_ends_say_so() {
        let (mut server, mut client) = pair(Conditions::GOOD);
        assert_eq!(server.poll(), [LinkEvent::Connected(PeerId(1))]);
        server.send(PeerId(1), b"hi".to_vec(), Mode::Reliable);
        let events = client.poll();
        assert_eq!(events[0], LinkEvent::Connected(PeerId(0)));
        assert_eq!(data(&events), [(b"hi".to_vec(), Mode::Reliable)]);
    }

    #[test]
    fn reliable_arrives_once_and_in_order_over_an_awful_link() {
        let (mut server, mut client) = pair(Conditions::AWFUL);
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut sent = 0u32;
        while got.len() < 200 {
            assert!(
                Instant::now() < deadline,
                "only {} of 200 in order",
                got.len()
            );
            if sent < 200 {
                client.send(PeerId(0), sent.to_le_bytes().to_vec(), Mode::Reliable);
                client.send(PeerId(0), vec![9], Mode::Unreliable);
                sent += 1;
            }
            client.poll();
            for (bytes, mode) in data(&server.poll()) {
                if mode == Mode::Reliable {
                    got.push(u32::from_le_bytes(bytes.try_into().unwrap()));
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(got, (0..200).collect::<Vec<_>>());
    }

    #[test]
    fn goodbye_is_clean_and_silence_is_not() {
        let (mut server, mut client) = pair(Conditions::GOOD);
        server.poll();
        client.poll();
        client.close(PeerId(0));
        assert_eq!(
            server.poll(),
            [LinkEvent::Disconnected(PeerId(1), Ended::Clean)]
        );

        let (mut server, mut client) = pair(Conditions::GOOD);
        server.timeout = Duration::from_millis(50);
        server.poll();
        client.poll();
        std::mem::forget(client);
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(
            server.poll(),
            [LinkEvent::Disconnected(PeerId(1), Ended::Silent)]
        );
    }

    #[test]
    fn a_dial_nobody_answers_gives_up() {
        let mut ends = Loopback::network(2);
        let mut client = Link::dialling(ends.pop().unwrap());
        client.dial_timeout = Duration::from_millis(30);
        client.dial(PeerId(0));
        assert!(client.poll().is_empty());
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            client.poll(),
            [LinkEvent::Disconnected(PeerId(0), Ended::Silent)]
        );
    }
}
