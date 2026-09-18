//! Many connections at once: a server, or a client with one peer.
//!
//! The handshake is deliberately tiny — a request, an acceptance, and
//! afterwards the ordinary packet flow. There is no encryption and no identity
//! here; a real deployment would need both, and pretending otherwise with a
//! home-made scheme would be worse than being explicit that it is missing.

use std::io;

use crate::connection::{Connection, ConnectionSettings, ConnectionState};
use crate::packet::{Header, PacketKind, MAX_PACKET};
use crate::transport::Transport;
use runity_serialize::Reader;

/// Something that happened to a connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEvent<A> {
    /// A peer joined, or our connection attempt was accepted.
    Connected(A),
    /// A peer left, timed out, or was refused.
    Disconnected(A),
    /// A message arrived.
    Message {
        /// Who sent it.
        from: A,
        /// What they sent.
        bytes: Vec<u8>,
    },
}

/// A connection and what we know about how it began.
struct Peer<A> {
    address: A,
    connection: Connection,
    /// True when we are the side that asked to connect.
    initiator: bool,
    /// Seconds since the last connection request went out.
    since_request: f32,
    /// Seconds spent trying to connect, so an attempt can give up.
    attempting: f32,
}

/// Holds every connection and drives them from one transport.
pub struct Host<T: Transport> {
    transport: T,
    peers: Vec<Peer<T::Address>>,
    settings: ConnectionSettings,
    /// How many peers may connect to us. A client sets this to one.
    pub max_peers: usize,
    /// How long to keep asking before giving up on a connection attempt.
    pub connect_timeout: f32,
    buffer: Vec<u8>,
}

impl<T: Transport> Host<T> {
    /// A host over a transport, with room for `max_peers` connections.
    pub fn new(transport: T, max_peers: usize) -> Self {
        Self {
            transport,
            peers: Vec::new(),
            settings: ConnectionSettings::default(),
            max_peers,
            connect_timeout: 5.0,
            buffer: vec![0; MAX_PACKET],
        }
    }

    /// Use different connection settings for peers connected from now on.
    pub fn with_settings(mut self, settings: ConnectionSettings) -> Self {
        self.settings = settings;
        self
    }

    /// The transport underneath, for asking it its address.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Start connecting to a peer. Idempotent.
    pub fn connect(&mut self, address: T::Address) {
        if self.peers.iter().any(|peer| peer.address == address) {
            return;
        }
        self.peers.push(Peer {
            address,
            connection: Connection::new(self.settings),
            initiator: true,
            since_request: self.connect_timeout, // ask immediately
            attempting: 0.0,
        });
    }

    /// Say goodbye to a peer, so it need not wait for a timeout.
    pub fn disconnect(&mut self, address: T::Address) -> io::Result<()> {
        let Some(peer) = self.peers.iter_mut().find(|peer| peer.address == address) else {
            return Ok(());
        };
        let goodbye = peer.connection.control(PacketKind::Disconnect);
        peer.connection.disconnect();
        self.transport.send(address, &goodbye)
    }

    /// Every connected peer.
    pub fn peers(&self) -> impl Iterator<Item = (T::Address, &Connection)> + '_ {
        self.peers
            .iter()
            .filter(|peer| peer.connection.is_connected())
            .map(|peer| (peer.address, &peer.connection))
    }

    /// How many peers are connected.
    pub fn peer_count(&self) -> usize {
        self.peers().count()
    }

    /// Whether a particular peer is connected.
    pub fn is_connected(&self, address: T::Address) -> bool {
        self.peers
            .iter()
            .any(|peer| peer.address == address && peer.connection.is_connected())
    }

    /// Queue a message that must arrive.
    pub fn send_reliable(&mut self, to: T::Address, bytes: &[u8]) {
        if let Some(peer) = self.peers.iter_mut().find(|peer| peer.address == to) {
            peer.connection.send_reliable(bytes);
        }
    }

    /// Queue a message that may be dropped.
    pub fn send_unreliable(&mut self, to: T::Address, bytes: &[u8]) -> bool {
        match self.peers.iter_mut().find(|peer| peer.address == to) {
            Some(peer) => peer.connection.send_unreliable(bytes),
            None => false,
        }
    }

    /// Send to everybody connected.
    pub fn broadcast_reliable(&mut self, bytes: &[u8]) {
        for peer in self
            .peers
            .iter_mut()
            .filter(|peer| peer.connection.is_connected())
        {
            peer.connection.send_reliable(bytes);
        }
    }

    /// Send to everybody connected, dropping it where it does not fit.
    pub fn broadcast_unreliable(&mut self, bytes: &[u8]) {
        for peer in self
            .peers
            .iter_mut()
            .filter(|peer| peer.connection.is_connected())
        {
            peer.connection.send_unreliable(bytes);
        }
    }

    /// Pump the network: read what arrived, send what is queued, and report
    /// what happened.
    pub fn update(&mut self, dt: f32) -> io::Result<Vec<HostEvent<T::Address>>> {
        let mut events = Vec::new();
        self.read(&mut events)?;
        self.write(dt, &mut events)?;
        Ok(events)
    }

    /// Take everything the transport has for us.
    fn read(&mut self, events: &mut Vec<HostEvent<T::Address>>) -> io::Result<()> {
        loop {
            let mut buffer = core::mem::take(&mut self.buffer);
            let received = self.transport.receive(&mut buffer)?;
            let Some((from, length)) = received else {
                self.buffer = buffer;
                return Ok(());
            };
            let packet = &buffer[..length];

            let known = self.peers.iter().position(|peer| peer.address == from);
            match known {
                Some(index) => {
                    let was_connected = self.peers[index].connection.is_connected();
                    // A packet from a peer that has not finished connecting is
                    // the acceptance itself.
                    match self.peers[index].connection.receive(packet) {
                        Ok(messages) => {
                            for bytes in messages {
                                events.push(HostEvent::Message { from, bytes });
                            }
                        }
                        // Rubbish from a known peer is ignored, not fatal: the
                        // address may simply have been reused.
                        Err(_) => {
                            self.buffer = buffer;
                            continue;
                        }
                    }
                    if !was_connected && self.peers[index].connection.is_connected() {
                        events.push(HostEvent::Connected(from));
                    }
                    if self.peers[index].connection.state() == ConnectionState::Disconnected {
                        self.peers.remove(index);
                        events.push(HostEvent::Disconnected(from));
                    }
                }
                None => {
                    // Somebody new. Only a connection request is interesting;
                    // anything else is a stray packet or an old peer that did
                    // not notice it was dropped.
                    let mut reader = Reader::new(packet);
                    let header = Header::read(&mut reader);
                    let is_request = matches!(
                        header,
                        Ok(Header {
                            kind: PacketKind::Connect,
                            ..
                        })
                    );
                    if is_request && self.peers.len() < self.max_peers {
                        let mut connection = Connection::new(self.settings);
                        connection.accept();
                        let accept = connection.control(PacketKind::Accept);
                        self.transport.send(from, &accept)?;
                        self.peers.push(Peer {
                            address: from,
                            connection,
                            initiator: false,
                            since_request: 0.0,
                            attempting: 0.0,
                        });
                        events.push(HostEvent::Connected(from));
                    }
                }
            }
            self.buffer = buffer;
        }
    }

    /// Send what every connection has queued, and retire the dead ones.
    fn write(&mut self, dt: f32, events: &mut Vec<HostEvent<T::Address>>) -> io::Result<()> {
        let mut dropped = Vec::new();

        for index in 0..self.peers.len() {
            let address = self.peers[index].address;
            let connecting = self.peers[index].connection.state() == ConnectionState::Connecting;

            if connecting && self.peers[index].initiator {
                self.peers[index].attempting += dt;
                if self.peers[index].attempting >= self.connect_timeout {
                    dropped.push(index);
                    continue;
                }
                self.peers[index].since_request += dt;
                if self.peers[index].since_request >= 0.25 {
                    self.peers[index].since_request = 0.0;
                    let request = self.peers[index].connection.control(PacketKind::Connect);
                    self.transport.send(address, &request)?;
                }
                // Nothing else to say until we are let in.
                let _ = self.peers[index].connection.update(dt);
                if self.peers[index].connection.is_timed_out() {
                    dropped.push(index);
                }
                continue;
            }

            for packet in self.peers[index].connection.update(dt) {
                self.transport.send(address, &packet)?;
            }
            if self.peers[index].connection.is_timed_out()
                || self.peers[index].connection.state() == ConnectionState::Disconnected
            {
                dropped.push(index);
            }
        }

        // Remove from the back so the earlier indices stay valid.
        for index in dropped.into_iter().rev() {
            let peer = self.peers.remove(index);
            events.push(HostEvent::Disconnected(peer.address));
        }
        Ok(())
    }
}
