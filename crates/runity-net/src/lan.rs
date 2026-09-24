//! Finding games on the local network: the lobby list for a LAN party,
//! with no platform service in between.
//!
//! A player looking for games shouts "who is playing?" to the network
//! ([`find_games`]); every host that listens ([`LanHost`], answered each
//! frame) says its game, its name, how many play and where to connect. The
//! game's name keeps two games on one network from listing each other.
//! Joining is then [`crate::net::Udp::connect`] to the address found.
//!
//! What this is not: invites through friends lists and getting past NAT
//! between homes. Those need a platform (Steam) or a relay, and stay open.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use web_time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Where hosts listen for "who is playing?", unless the game says another.
pub const PORT: u16 = 47_777;

const ASK: &[u8] = b"runity?";
const ANSWER: &[u8] = b"runity!";

/// What a host says about its game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameInfo {
    /// Which game: the project's name. Only the same game answers.
    pub game: String,
    /// What the host called this session: "Ada's valley".
    pub name: String,
    pub players: u32,
    pub max_players: u32,
    /// The port the game itself is on.
    pub port: u16,
}

/// A game found on the network, and where to connect to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub info: GameInfo,
    pub address: SocketAddr,
}

/// A host being findable: it answers every "who is playing?" for its game.
pub struct LanHost {
    socket: UdpSocket,
    pub info: GameInfo,
}

impl LanHost {
    /// Listen on `port` ([`PORT`] normally) on every interface.
    pub fn open(port: u16, info: GameInfo) -> std::io::Result<Self> {
        Self::open_at(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)), info)
    }

    /// Listen at one address — a test's loopback.
    pub fn open_at(address: SocketAddr, info: GameInfo) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(address)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket, info })
    }

    pub fn local_address(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Answer whoever asked since the last call; call it every frame.
    /// Returns how many were answered.
    pub fn answer(&self) -> usize {
        let mut answered = 0;
        let mut buffer = [0u8; 512];
        while let Ok((len, from)) = self.socket.recv_from(&mut buffer) {
            let Some(game) = buffer[..len].strip_prefix(ASK) else {
                continue;
            };
            if game != self.info.game.as_bytes() {
                continue;
            }
            let mut reply = ANSWER.to_vec();
            reply.extend(postcard::to_stdvec(&self.info).unwrap_or_default());
            if self.socket.send_to(&reply, from).is_ok() {
                answered += 1;
            }
        }
        answered
    }
}

/// Ask the whole local network which hosts play `game`, and wait `wait`
/// for answers: the lobby list. Nearest (first to answer) first.
pub fn find_games(port: u16, game: &str, wait: Duration) -> std::io::Result<Vec<Found>> {
    find_games_at(&[SocketAddr::from((Ipv4Addr::BROADCAST, port))], game, wait)
}

/// Ask particular addresses instead of everyone — a known server, a test.
pub fn find_games_at(to: &[SocketAddr], game: &str, wait: Duration) -> std::io::Result<Vec<Found>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_broadcast(true)?;
    let mut ask = ASK.to_vec();
    ask.extend_from_slice(game.as_bytes());
    for address in to {
        socket.send_to(&ask, address)?;
    }
    let until = Instant::now() + wait;
    let mut found: Vec<Found> = Vec::new();
    let mut buffer = [0u8; 512];
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        socket.set_read_timeout(Some(left.max(Duration::from_millis(1))))?;
        let Ok((len, from)) = socket.recv_from(&mut buffer) else {
            break;
        };
        let Some(body) = buffer[..len].strip_prefix(ANSWER) else {
            continue;
        };
        let Ok(info) = postcard::from_bytes::<GameInfo>(body) else {
            continue;
        };
        if info.game != game {
            continue;
        }
        let address = SocketAddr::new(from.ip(), info.port);
        if !found.iter().any(|f| f.address == address) {
            found.push(Found { info, address });
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(game: &str, name: &str) -> GameInfo {
        GameInfo {
            game: game.into(),
            name: name.into(),
            players: 2,
            max_players: 4,
            port: 40_000,
        }
    }

    #[test]
    fn a_host_answers_for_its_own_game_and_is_found_with_where_to_connect() {
        let loopback = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let valley = LanHost::open_at(loopback, info("valley", "Ada's valley")).unwrap();
        let other = LanHost::open_at(loopback, info("moss", "someone else")).unwrap();
        let hosts = [
            valley.local_address().unwrap(),
            other.local_address().unwrap(),
        ];
        let answering = std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_millis(400);
            let mut answered = 0;
            while Instant::now() < until {
                answered += valley.answer() + other.answer();
                std::thread::sleep(Duration::from_millis(5));
            }
            answered
        });
        let found = find_games_at(&hosts, "valley", Duration::from_millis(300)).unwrap();
        assert_eq!(answering.join().unwrap(), 1, "only the valley answered");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].info.name, "Ada's valley");
        assert_eq!(
            found[0].address,
            SocketAddr::from((Ipv4Addr::LOCALHOST, 40_000))
        );
    }
}
