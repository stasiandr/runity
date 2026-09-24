//! Playing across the internet without a platform: a relay and room codes.
//!
//! Two homes behind NAT cannot reach each other directly, but both can
//! reach a server. A [`RelayServer`] anyone can run (`scrap relay`) passes
//! datagrams between the peers of a room; a room is a short code the host
//! reads out — the invite. [`Relayed`] is a [`Transport`] like
//! [`crate::net::Udp`], so replication does not know the difference. It
//! costs a hop of latency and the relay's bandwidth; a platform's
//! peer-to-peer (Steam) avoids both, and stays possible behind the same
//! trait.
//!
//! Wire: `J room peer` joins (and keeps the place: send it every few
//! seconds), `D room from to bytes` goes to the relay, `d from bytes` comes
//! back out. A place not refreshed for [`RelayServer::FORGET`] is dropped.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use web_time::{Duration, Instant};

use crate::net::{PeerId, Transport};

/// How long a room code is.
pub const CODE_LENGTH: usize = 6;

/// A fresh room code, to read out: letters and digits that cannot be
/// mistaken for each other (no O and 0, no I and 1).
pub fn new_room_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bits = u64::from_str_radix(&crate::EntityId::fresh().to_string(), 16).unwrap_or(0);
    (0..CODE_LENGTH)
        .map(|_| {
            let c = ALPHABET[(bits % ALPHABET.len() as u64) as usize] as char;
            bits /= ALPHABET.len() as u64;
            c
        })
        .collect()
}

fn room_bytes(room: &str) -> Option<[u8; CODE_LENGTH]> {
    let bytes = room.to_ascii_uppercase().into_bytes();
    bytes.try_into().ok()
}

/// The server between the homes.
pub struct RelayServer {
    socket: UdpSocket,
    places: HashMap<([u8; CODE_LENGTH], u32), (SocketAddr, Instant)>,
}

impl RelayServer {
    /// A place in a room not refreshed for this long is forgotten.
    pub const FORGET: Duration = Duration::from_secs(30);

    pub fn bind(address: &str) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(address)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            places: HashMap::new(),
        })
    }

    pub fn local_address(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// How many peers are in rooms now.
    pub fn peers(&self) -> usize {
        self.places.len()
    }

    /// Pass on what arrived since the last call. Returns how many
    /// datagrams were forwarded.
    pub fn pump(&mut self) -> usize {
        let now = Instant::now();
        self.places
            .retain(|_, (_, seen)| now.duration_since(*seen) < Self::FORGET);
        let mut forwarded = 0;
        let mut buffer = vec![0u8; 65_536];
        while let Ok((len, from)) = self.socket.recv_from(&mut buffer) {
            let datagram = &buffer[..len];
            let head = 1 + CODE_LENGTH;
            if len < head + 4 {
                continue;
            }
            let Ok(room) = <[u8; CODE_LENGTH]>::try_from(&datagram[1..head]) else {
                continue;
            };
            let word = |at: usize| u32::from_le_bytes(datagram[at..at + 4].try_into().unwrap());
            match datagram[0] {
                b'J' => {
                    self.places.insert((room, word(head)), (from, now));
                }
                b'D' if len >= head + 8 => {
                    let (sender, to) = (word(head), word(head + 4));
                    // Only a peer that joined from this address speaks for
                    // itself in the room.
                    let joined = self
                        .places
                        .get(&(room, sender))
                        .is_some_and(|(address, _)| *address == from);
                    if let (true, Some((address, _))) = (joined, self.places.get(&(room, to))) {
                        let mut out = vec![b'd'];
                        out.extend_from_slice(&sender.to_le_bytes());
                        out.extend_from_slice(&datagram[head + 8..]);
                        if self.socket.send_to(&out, address).is_ok() {
                            forwarded += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        forwarded
    }

    /// Pump until stopped: what `scrap relay` runs.
    pub fn run(&mut self) -> ! {
        loop {
            self.pump();
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// A peer in a relay room: a [`Transport`] through the relay.
pub struct Relayed {
    socket: UdpSocket,
    relay: SocketAddr,
    room: [u8; CODE_LENGTH],
    me: PeerId,
    joined: Instant,
}

impl Relayed {
    /// How often the place in the room is refreshed.
    pub const KEEP: Duration = Duration::from_secs(5);

    /// Join `room` at `relay` as `me`.
    pub fn join(relay: SocketAddr, room: &str, me: PeerId) -> std::io::Result<Self> {
        let room = room_bytes(room).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("a room code is {CODE_LENGTH} letters and digits, not `{room}`"),
            )
        })?;
        let socket = UdpSocket::bind(if relay.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })?;
        socket.set_nonblocking(true)?;
        let relayed = Self {
            socket,
            relay,
            room,
            me,
            joined: Instant::now(),
        };
        relayed.say_here()?;
        Ok(relayed)
    }

    fn say_here(&self) -> std::io::Result<()> {
        let mut join = vec![b'J'];
        join.extend_from_slice(&self.room);
        join.extend_from_slice(&self.me.0.to_le_bytes());
        self.socket.send_to(&join, self.relay).map(|_| ())
    }
}

impl Transport for Relayed {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>) {
        let mut data = vec![b'D'];
        data.extend_from_slice(&self.room);
        data.extend_from_slice(&self.me.0.to_le_bytes());
        data.extend_from_slice(&to.0.to_le_bytes());
        data.extend_from_slice(&bytes);
        let _ = self.socket.send_to(&data, self.relay);
    }

    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        if self.joined.elapsed() > Self::KEEP {
            let _ = self.say_here();
            self.joined = Instant::now();
        }
        let mut out = Vec::new();
        let mut buffer = vec![0u8; 65_536];
        while let Ok((len, from)) = self.socket.recv_from(&mut buffer) {
            if from != self.relay || len < 5 || buffer[0] != b'd' {
                continue;
            }
            let sender = u32::from_le_bytes(buffer[1..5].try_into().unwrap());
            out.push((PeerId(sender), buffer[5..len].to_vec()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(server: &mut RelayServer) {
        for _ in 0..20 {
            server.pump();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn two_peers_in_a_room_hear_each_other_and_another_room_hears_nothing() {
        let mut server = RelayServer::bind("127.0.0.1:0").unwrap();
        let relay = server.local_address().unwrap();
        let code = new_room_code();
        assert_eq!(code.len(), CODE_LENGTH);
        assert!(!code.contains('0') && !code.contains('O'));

        let mut host = Relayed::join(relay, &code, PeerId::HOST).unwrap();
        let mut guest = Relayed::join(relay, &code.to_lowercase(), PeerId(1)).unwrap();
        let mut stranger = Relayed::join(relay, "ZZZZZZ", PeerId(1)).unwrap();
        settle(&mut server);
        assert_eq!(server.peers(), 3);

        host.send(PeerId(1), b"welcome".to_vec());
        settle(&mut server);
        assert_eq!(guest.receive(), [(PeerId::HOST, b"welcome".to_vec())]);
        assert!(stranger.receive().is_empty(), "another room");

        guest.send(PeerId::HOST, b"thanks".to_vec());
        settle(&mut server);
        assert_eq!(host.receive(), [(PeerId(1), b"thanks".to_vec())]);
        assert!(Relayed::join(relay, "TOO-LONG", PeerId(2)).is_err());
    }
}
