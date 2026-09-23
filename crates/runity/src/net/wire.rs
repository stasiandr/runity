//! The wire: datagrams to and from whoever is at the other end.
//!
//! A [`Transport`] sends bytes to an endpoint and hands back what arrived,
//! and promises nothing else — no order, no delivery, no connection. The
//! [`PeerId`] here names an *endpoint on this wire*: a socket's owner as
//! it introduces itself in every datagram. Who that endpoint is in the
//! game — which client, which seat — is the server's to decide
//! ([`super::server`]); reliability and connections are the link's
//! ([`super::link`]). That split is what lets UDP, a relay, Steam and an
//! in-process loopback all carry the same session.
//!
//! [`Laggy`] wraps any of them in latency, jitter, loss and duplication —
//! the dacha simulator's `LaggyTransport`, which is how netcode is tested
//! against a bad link without a bad link.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::PeerId;

/// The wire: send to one endpoint, take what arrived.
pub trait Transport {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>);
    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)>;
}

/// A transport chosen at run time — UDP, a relay, Steam — is still one.
impl<T: Transport + ?Sized> Transport for Box<T> {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>) {
        (**self).send(to, bytes);
    }

    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        (**self).receive()
    }
}

/// Endpoints over UDP: each datagram starts with the sender's [`PeerId`],
/// and where an endpoint is is learned from what it sends, so a server
/// need not be told where its clients are. Unreliable and unordered.
pub struct Udp {
    socket: std::net::UdpSocket,
    me: PeerId,
    peers: HashMap<PeerId, std::net::SocketAddr>,
}

impl Udp {
    /// Bind to `address` (`"0.0.0.0:7777"`, or port 0 for any) as `me`.
    pub fn bind(address: &str, me: PeerId) -> std::io::Result<Self> {
        let socket = std::net::UdpSocket::bind(address)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            me,
            peers: HashMap::new(),
        })
    }

    pub fn local_address(&self) -> std::io::Result<std::net::SocketAddr> {
        self.socket.local_addr()
    }

    /// Where an endpoint is.
    pub fn connect(&mut self, peer: PeerId, address: std::net::SocketAddr) {
        self.peers.insert(peer, address);
    }
}

impl Transport for Udp {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>) {
        let Some(address) = self.peers.get(&to) else {
            return;
        };
        let mut datagram = self.me.0.to_le_bytes().to_vec();
        datagram.extend_from_slice(&bytes);
        let _ = self.socket.send_to(&datagram, address);
    }

    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        let mut out = Vec::new();
        let mut buffer = vec![0u8; 65_536];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((length, address)) => {
                    if length < 4 {
                        continue;
                    }
                    let from = PeerId(u32::from_le_bytes([
                        buffer[0], buffer[1], buffer[2], buffer[3],
                    ]));
                    // The latest address wins: a client that came back on
                    // another port is still reachable.
                    self.peers.insert(from, address);
                    out.push((from, buffer[4..length].to_vec()));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                // A refused send to a closed port comes back as an error
                // on some systems; it says nothing about the next datagram.
                Err(_) => continue,
            }
        }
        out
    }
}

/// What each endpoint has waiting: who sent it, and the bytes.
type Inboxes = HashMap<PeerId, VecDeque<(PeerId, Vec<u8>)>>;

/// Endpoints in one process: a host's own client and its server, and
/// every test. Exact — nothing lost, nothing reordered — which is why
/// [`Laggy`] exists.
#[derive(Clone)]
pub struct Loopback {
    me: PeerId,
    queues: std::sync::Arc<std::sync::Mutex<Inboxes>>,
}

impl Loopback {
    /// A network of `count` endpoints, 0 to `count - 1`, one end each.
    pub fn network(count: u32) -> Vec<Loopback> {
        let queues = std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
        (0..count)
            .map(|i| Loopback {
                me: PeerId(i),
                queues: queues.clone(),
            })
            .collect()
    }

    pub fn me(&self) -> PeerId {
        self.me
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

/// Nowhere: the far side of a wire nobody listens on.
pub struct Nobody;

impl Transport for Nobody {
    fn send(&mut self, _: PeerId, _: Vec<u8>) {}
    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        Vec::new()
    }
}

/// How bad a link is, one way.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Conditions {
    /// One-way delay.
    pub latency: Duration,
    /// Up to this much more or less, per datagram — which is also what
    /// reorders them.
    pub jitter: Duration,
    /// The share of datagrams lost, 0 to 1.
    pub loss: f32,
    /// The share delivered twice.
    pub duplicate: f32,
}

impl Conditions {
    /// A perfect link.
    pub const GOOD: Conditions = Conditions {
        latency: Duration::ZERO,
        jitter: Duration::ZERO,
        loss: 0.0,
        duplicate: 0.0,
    };
    /// A friend across town on Wi-Fi: 80 ms, some jitter, 3% lost.
    pub const POOR: Conditions = Conditions {
        latency: Duration::from_millis(40),
        jitter: Duration::from_millis(10),
        loss: 0.03,
        duplicate: 0.01,
    };
    /// A friend on another continent on a train: 200 ms, 10% lost.
    pub const AWFUL: Conditions = Conditions {
        latency: Duration::from_millis(100),
        jitter: Duration::from_millis(40),
        loss: 0.10,
        duplicate: 0.02,
    };

    /// `good`, `poor`, `awful`, or `latency=80,jitter=10,loss=3,dup=1`
    /// (milliseconds of round trip, percent) — what `RUNITY_LINK` says.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim() {
            "" | "good" => return Ok(Self::GOOD),
            "poor" => return Ok(Self::POOR),
            "awful" => return Ok(Self::AWFUL),
            _ => {}
        }
        let mut out = Self::GOOD;
        for part in text.split(',') {
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| format!("`{part}`: want key=number, or good, poor, awful"))?;
            let n: f32 = value
                .trim()
                .parse()
                .map_err(|_| format!("`{part}`: `{value}` is not a number"))?;
            let half = Duration::from_micros((n.max(0.0) * 500.0) as u64);
            match key.trim() {
                // A round trip crosses the link twice.
                "latency" => out.latency = half,
                "jitter" => out.jitter = half,
                "loss" => out.loss = (n / 100.0).clamp(0.0, 1.0),
                "dup" => out.duplicate = (n / 100.0).clamp(0.0, 1.0),
                other => return Err(format!("`{other}`: want latency, jitter, loss or dup")),
            }
        }
        Ok(out)
    }
}

/// Any transport, made as bad as asked: each datagram is delayed leaving
/// and again arriving, lost or doubled by chance. The link above it
/// ([`super::link`]) is what makes the reliable plane reliable again, so
/// this tests that too — unlike the dacha simulator's, which sits above a
/// transport that is reliable itself and so only ever hurts snapshots.
pub struct Laggy<T: Transport> {
    inner: T,
    pub conditions: Conditions,
    outbound: Vec<(Instant, PeerId, Vec<u8>)>,
    inbound: Vec<(Instant, PeerId, Vec<u8>)>,
    seed: u64,
}

impl<T: Transport> Laggy<T> {
    pub fn new(inner: T, conditions: Conditions, seed: u64) -> Self {
        Self {
            inner,
            conditions,
            outbound: Vec::new(),
            inbound: Vec::new(),
            seed: seed | 1,
        }
    }

    /// A number in 0..1, the same sequence for the same seed.
    fn roll(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    fn delay(&mut self) -> Duration {
        let jitter = self.conditions.jitter.as_secs_f32() * (self.roll() * 2.0 - 1.0);
        Duration::from_secs_f32((self.conditions.latency.as_secs_f32() + jitter).max(0.0))
    }

    /// When each copy of a datagram goes: none if lost, two if doubled.
    fn schedule(&mut self) -> Vec<Instant> {
        if self.roll() < self.conditions.loss {
            return Vec::new();
        }
        let now = Instant::now();
        let mut out = vec![now + self.delay()];
        if self.roll() < self.conditions.duplicate {
            out.push(now + self.delay());
        }
        out
    }

    fn due(queue: &mut Vec<(Instant, PeerId, Vec<u8>)>) -> Vec<(PeerId, Vec<u8>)> {
        let now = Instant::now();
        let mut due: Vec<_> = Vec::new();
        queue.retain(|(at, peer, bytes)| {
            if *at <= now {
                due.push((*at, *peer, bytes.clone()));
                false
            } else {
                true
            }
        });
        due.sort_by_key(|(at, _, _)| *at);
        due.into_iter().map(|(_, p, b)| (p, b)).collect()
    }
}

impl<T: Transport> Transport for Laggy<T> {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>) {
        for at in self.schedule() {
            self.outbound.push((at, to, bytes.clone()));
        }
        for (to, bytes) in Self::due(&mut self.outbound) {
            self.inner.send(to, bytes);
        }
    }

    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        for (to, bytes) in Self::due(&mut self.outbound) {
            self.inner.send(to, bytes);
        }
        for (from, bytes) in self.inner.receive() {
            for at in self.schedule() {
                self.inbound.push((at, from, bytes.clone()));
            }
        }
        Self::due(&mut self.inbound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bad_link_is_named_or_spelled_out() {
        assert_eq!(Conditions::parse("poor").unwrap(), Conditions::POOR);
        let c = Conditions::parse("latency=80, loss=5").unwrap();
        assert_eq!(c.latency, Duration::from_millis(40));
        assert!((c.loss - 0.05).abs() < 1e-6);
        assert!(Conditions::parse("latency=fast").is_err());
        assert!(Conditions::parse("ping=3").is_err());
    }

    #[test]
    fn a_laggy_link_loses_some_and_delays_the_rest() {
        let mut ends = Loopback::network(2);
        let far = ends.pop().unwrap();
        let near = ends.pop().unwrap();
        let conditions = Conditions {
            latency: Duration::from_millis(20),
            loss: 0.25,
            ..Conditions::GOOD
        };
        let mut near = Laggy::new(near, conditions, 7);
        let mut far = far;
        for i in 0..200u8 {
            near.send(PeerId(1), vec![i]);
        }
        assert!(far.receive().is_empty(), "nothing is early");
        std::thread::sleep(Duration::from_millis(30));
        near.receive();
        let got = far.receive().len();
        assert!((120..180).contains(&got), "{got} of 200 through a quarter's loss");
    }
}
