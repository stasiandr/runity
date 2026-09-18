//! Reliability over an unreliable link.
//!
//! The connection knows nothing about sockets — it takes bytes in and hands
//! bytes out, which is what makes it testable: every behaviour below is
//! checked against a simulated link with loss, reordering and duplication,
//! deterministically, with no network involved at all.
//!
//! Two kinds of message, because a game needs both:
//!
//! * **Unreliable** — a position, a snapshot. If it is lost, a newer one is
//!   along in 50 ms and resending the old one would be worse than useless.
//! * **Reliable ordered** — "I felled this tree", "the granary is finished".
//!   Must arrive, exactly once, in order.
//!
//! TCP gives only the second, and makes the first wait behind it. That is
//! head-of-line blocking, and it is why an action game on TCP feels like it
//! stutters whenever a packet is lost.

use std::collections::VecDeque;

use runity_serialize::{Error, Reader, Result, Writer};

use crate::packet::{
    acked_sequences, AckState, Header, PacketKind, Reception, HEADER_LEN, MAX_PACKET,
};

/// Bytes of message header inside a packet: flags, id and length.
const MESSAGE_OVERHEAD: usize = 8;

/// How much of a packet is available to messages.
pub const MAX_MESSAGE_CHUNK: usize = MAX_PACKET - HEADER_LEN - MESSAGE_OVERHEAD;

/// Tuning for a connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConnectionSettings {
    /// Send something at least this often, so the other side knows we are
    /// alive and so acknowledgements keep flowing.
    pub keep_alive: f32,
    /// Give up after this long with nothing received.
    pub timeout: f32,
    /// Resend a reliable message after this multiple of the round trip time.
    pub resend_factor: f32,
    /// Floor on the resend delay, for links where the round trip is tiny.
    pub min_resend: f32,
    /// Largest packet to put on the wire.
    pub max_packet: usize,
    /// Most packets to send in one update.
    ///
    /// The acknowledgement window is 32 packets wide, so a burst larger than
    /// that arrives with its head already outside it — and a burst is exactly
    /// what a large message produces. Without pacing, every snapshot would
    /// stall behind its own first fragment.
    pub max_packets_per_update: usize,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            keep_alive: 0.1,
            timeout: 5.0,
            resend_factor: 1.5,
            min_resend: 0.05,
            max_packet: MAX_PACKET,
            max_packets_per_update: 8,
        }
    }
}

/// Where a connection is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    /// Asking to be let in.
    Connecting,
    /// Exchanging messages.
    Connected,
    /// Told to go away, or gave up waiting.
    Disconnected,
}

/// What a connection has cost and how it is behaving.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConnectionStats {
    /// Packets handed to the transport.
    pub packets_sent: u64,
    /// Packets accepted from it.
    pub packets_received: u64,
    /// Packets sent that were never acknowledged.
    pub packets_lost: u64,
    /// Messages delivered to the application.
    pub messages_received: u64,
    /// Reliable messages sent more than once.
    pub resends: u64,
    /// Smoothed round trip time, in seconds.
    pub rtt: f32,
    /// Fraction of recent packets that went missing.
    pub loss: f32,
    /// Packets ignored because they were duplicates or older than the
    /// acknowledgement window.
    pub packets_stale: u64,
    /// Reliable messages received out of order and waiting for their turn.
    pub held: usize,
}

/// A reliable message waiting to be acknowledged.
#[derive(Clone, Debug)]
struct Pending {
    id: u16,
    /// True when this is one piece of a larger message.
    fragment: bool,
    /// True when this is the last piece.
    last: bool,
    bytes: Vec<u8>,
    /// When it was last put on the wire; `None` while it is still queued.
    sent_at: Option<f32>,
}

/// A packet we are waiting to hear about.
#[derive(Clone, Debug)]
struct Sent {
    sequence: u16,
    time: f32,
    /// Reliable messages it carried.
    carried: Vec<u16>,
    acked: bool,
}

/// One end of a connection: reliability, ordering and acknowledgements.
#[derive(Debug)]
pub struct Connection {
    settings: ConnectionSettings,
    state: ConnectionState,
    sequence: u16,
    acks: AckState,
    sent: VecDeque<Sent>,
    pending: VecDeque<Pending>,
    unreliable: VecDeque<Vec<u8>>,
    next_id: u16,
    /// The next reliable id the application should see.
    expected: u16,
    /// Reliable messages that arrived early, waiting for their turn.
    early: Vec<(u16, bool, bool, Vec<u8>)>,
    /// Fragments of a message being reassembled, in order.
    assembling: Vec<u8>,
    clock: f32,
    last_received: f32,
    last_sent: f32,
    stats: ConnectionStats,
}

impl Default for Connection {
    fn default() -> Self {
        Self::new(ConnectionSettings::default())
    }
}

impl Connection {
    /// A connection that has not said anything yet.
    pub fn new(settings: ConnectionSettings) -> Self {
        Self {
            settings,
            state: ConnectionState::Connecting,
            sequence: 0,
            acks: AckState::new(),
            sent: VecDeque::new(),
            pending: VecDeque::new(),
            unreliable: VecDeque::new(),
            next_id: 0,
            expected: 0,
            early: Vec::new(),
            assembling: Vec::new(),
            clock: 0.0,
            last_received: 0.0,
            last_sent: -1.0,
            stats: ConnectionStats::default(),
        }
    }

    /// Mark the connection as established.
    pub fn accept(&mut self) {
        if self.state == ConnectionState::Connecting {
            self.state = ConnectionState::Connected;
            self.last_received = self.clock;
        }
    }

    /// Current state.
    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// Whether messages can flow.
    pub fn is_connected(&self) -> bool {
        self.state == ConnectionState::Connected
    }

    /// Whether the other side has gone quiet for longer than the timeout.
    ///
    /// A connection that has never heard anything counts too, measured from
    /// when it was created: an attempt to reach a machine that is not there
    /// has to end by itself, or it waits forever.
    pub fn is_timed_out(&self) -> bool {
        self.clock - self.last_received > self.settings.timeout
    }

    /// Stop, and stop resending.
    pub fn disconnect(&mut self) {
        self.state = ConnectionState::Disconnected;
        self.pending.clear();
        self.unreliable.clear();
    }

    /// Smoothed round trip time in seconds.
    pub fn rtt(&self) -> f32 {
        self.stats.rtt
    }

    /// Counters.
    pub fn stats(&self) -> ConnectionStats {
        ConnectionStats {
            held: self.early.len(),
            ..self.stats
        }
    }

    /// Queue a message that must arrive, in order, exactly once.
    ///
    /// Anything longer than a packet is split here and reassembled on the
    /// other side; because the reliable channel is ordered, reassembly is
    /// just "append until the last piece".
    pub fn send_reliable(&mut self, bytes: &[u8]) {
        if self.state == ConnectionState::Disconnected {
            return;
        }
        let budget = self.chunk_budget();
        if bytes.len() <= budget {
            let id = self.take_id();
            self.pending.push_back(Pending {
                id,
                fragment: false,
                last: true,
                bytes: bytes.to_vec(),
                sent_at: None,
            });
            return;
        }
        let chunks: Vec<&[u8]> = bytes.chunks(budget).collect();
        let count = chunks.len();
        for (index, chunk) in chunks.into_iter().enumerate() {
            let id = self.take_id();
            self.pending.push_back(Pending {
                id,
                fragment: true,
                last: index + 1 == count,
                bytes: chunk.to_vec(),
                sent_at: None,
            });
        }
    }

    /// Queue a message that may be dropped.
    ///
    /// Anything that does not fit in one packet is refused rather than split:
    /// an unreliable message that needs every one of five packets to arrive
    /// is not unreliable, it is unlikely.
    pub fn send_unreliable(&mut self, bytes: &[u8]) -> bool {
        if self.state == ConnectionState::Disconnected || bytes.len() > self.chunk_budget() {
            return false;
        }
        self.unreliable.push_back(bytes.to_vec());
        true
    }

    /// How many messages are waiting to go out.
    pub fn queued(&self) -> usize {
        self.pending.len() + self.unreliable.len()
    }

    /// Advance the clock and produce the packets to send now.
    pub fn update(&mut self, dt: f32) -> Vec<Vec<u8>> {
        if dt.is_finite() && dt > 0.0 {
            self.clock += dt;
        }
        if self.state == ConnectionState::Disconnected {
            return Vec::new();
        }

        self.expire_lost();

        let mut packets = Vec::new();
        let resend_after =
            (self.stats.rtt * self.settings.resend_factor).max(self.settings.min_resend);

        loop {
            let mut writer = Writer::with_capacity(self.settings.max_packet);
            let sequence = self.sequence;
            Header {
                kind: PacketKind::Payload,
                sequence,
                ack: self.acks.newest(),
                ack_bits: self.acks.bits(),
            }
            .write(&mut writer);

            let mut carried = Vec::new();
            let mut wrote_anything = false;

            // Reliable first: they are the ones that cannot simply be dropped.
            for index in 0..self.pending.len() {
                let due = match self.pending[index].sent_at {
                    None => true,
                    Some(at) => self.clock - at >= resend_after,
                };
                if !due {
                    continue;
                }
                let length = self.pending[index].bytes.len();
                if writer.len() + MESSAGE_OVERHEAD + length > self.settings.max_packet {
                    break;
                }
                let message = self.pending[index].clone();
                let flags = 1 | u8::from(message.fragment) << 1 | u8::from(message.last) << 2;
                writer
                    .u8(flags)
                    .u16(message.id)
                    .varint(length as u64)
                    .raw(&message.bytes);
                carried.push(message.id);
                if self.pending[index].sent_at.is_some() {
                    self.stats.resends += 1;
                }
                self.pending[index].sent_at = Some(self.clock);
                wrote_anything = true;
            }

            while let Some(front) = self.unreliable.front() {
                if writer.len() + MESSAGE_OVERHEAD + front.len() > self.settings.max_packet {
                    break;
                }
                let message = self.unreliable.pop_front().expect("just checked the front");
                writer
                    .u8(0)
                    .u16(0)
                    .varint(message.len() as u64)
                    .raw(&message);
                wrote_anything = true;
            }

            let idle =
                self.last_sent < 0.0 || self.clock - self.last_sent >= self.settings.keep_alive;
            if !wrote_anything && (!packets.is_empty() || !idle) {
                break;
            }

            self.sequence = self.sequence.wrapping_add(1);
            self.sent.push_back(Sent {
                sequence,
                time: self.clock,
                carried,
                acked: false,
            });
            while self.sent.len() > 256 {
                self.sent.pop_front();
            }
            self.last_sent = self.clock;
            self.stats.packets_sent += 1;
            packets.push(writer.finish());

            if !wrote_anything {
                break; // that was a keep-alive; one is enough
            }
            if packets.len() >= self.settings.max_packets_per_update.max(1) {
                break; // the rest goes out on the next update
            }
        }

        packets
    }

    /// Take a packet from the wire, returning whatever messages it completes.
    ///
    /// Order is guaranteed for reliable messages; unreliable ones arrive when
    /// they arrive.
    pub fn receive(&mut self, packet: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut reader = Reader::new(packet);
        let header = Header::read(&mut reader)?;
        self.last_received = self.clock;
        if self.state == ConnectionState::Connecting {
            self.state = ConnectionState::Connected;
        }
        self.stats.packets_received += 1;

        self.apply_acks(header.ack, header.ack_bits);

        if header.kind == PacketKind::Disconnect {
            self.state = ConnectionState::Disconnected;
            return Ok(Vec::new());
        }
        let reception = self.acks.receive(header.sequence);
        if reception == Reception::Duplicate {
            // Known to have been handled already; handling it twice would
            // deliver its messages twice.
            self.stats.packets_stale += 1;
            return Ok(Vec::new());
        }
        // Outside the window the receiver cannot tell new from seen. Reliable
        // messages carry their own ids and are deduplicated by them, so they
        // are still safe; unreliable ones are dropped rather than risk a
        // double delivery. This matters more than it sounds: a burst larger
        // than the window arrives reordered, and discarding whole packets here
        // stalls every fragmented message behind the one that was late.
        let reliable_only = reception == Reception::TooOld;
        if reliable_only {
            self.stats.packets_stale += 1;
        }

        let mut delivered = Vec::new();
        while !reader.is_empty() {
            let position = reader.position();
            let flags = reader.u8()?;
            let id = reader.u16()?;
            let length = reader.varint()?;
            if length > reader.remaining() as u64 {
                return Err(Error::LengthOutOfRange {
                    position,
                    length,
                    available: reader.remaining(),
                });
            }
            let bytes = reader.raw(length as usize)?.to_vec();

            if flags & 1 == 0 {
                if reliable_only {
                    continue;
                }
                self.stats.messages_received += 1;
                delivered.push(bytes);
                continue;
            }
            let fragment = flags & 2 != 0;
            let last = flags & 4 != 0;
            self.accept_reliable(id, fragment, last, bytes, &mut delivered);
        }
        Ok(delivered)
    }

    /// Build a bare packet of a given kind — handshakes and goodbyes.
    pub fn control(&mut self, kind: PacketKind) -> Vec<u8> {
        let mut writer = Writer::with_capacity(HEADER_LEN);
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        Header {
            kind,
            sequence,
            ack: self.acks.newest(),
            ack_bits: self.acks.bits(),
        }
        .write(&mut writer);
        self.last_sent = self.clock;
        self.stats.packets_sent += 1;
        writer.finish()
    }

    fn chunk_budget(&self) -> usize {
        self.settings
            .max_packet
            .saturating_sub(HEADER_LEN + MESSAGE_OVERHEAD)
            .max(1)
    }

    fn take_id(&mut self) -> u16 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Deliver a reliable message, or hold it until its turn comes.
    fn accept_reliable(
        &mut self,
        id: u16,
        fragment: bool,
        last: bool,
        bytes: Vec<u8>,
        delivered: &mut Vec<Vec<u8>>,
    ) {
        if crate::packet::newer_than(self.expected, id) || id == self.expected.wrapping_sub(1) {
            return; // already delivered; a resend crossed the acknowledgement
        }
        if id != self.expected {
            if !self.early.iter().any(|(held, ..)| *held == id) {
                self.early.push((id, fragment, last, bytes));
            }
            return;
        }

        self.take(fragment, last, bytes, delivered);
        // Whatever arrived early may now be next, possibly several in a row.
        loop {
            let Some(index) = self
                .early
                .iter()
                .position(|(held, ..)| *held == self.expected)
            else {
                break;
            };
            let (_, fragment, last, bytes) = self.early.remove(index);
            self.take(fragment, last, bytes, delivered);
        }
    }

    /// Consume one in-order reliable message, reassembling fragments.
    fn take(&mut self, fragment: bool, last: bool, bytes: Vec<u8>, delivered: &mut Vec<Vec<u8>>) {
        self.expected = self.expected.wrapping_add(1);
        if !fragment {
            self.stats.messages_received += 1;
            delivered.push(bytes);
            return;
        }
        self.assembling.extend_from_slice(&bytes);
        if last {
            self.stats.messages_received += 1;
            delivered.push(core::mem::take(&mut self.assembling));
        }
    }

    /// Tick off everything the other side says it has.
    fn apply_acks(&mut self, ack: u16, ack_bits: u32) {
        for sequence in acked_sequences(ack, ack_bits) {
            let Some(entry) = self.sent.iter_mut().find(|sent| sent.sequence == sequence) else {
                continue;
            };
            if entry.acked {
                continue;
            }
            entry.acked = true;

            // Round trip: measured from a packet we know the other side saw.
            let sample = (self.clock - entry.time).max(0.0);
            self.stats.rtt = if self.stats.rtt == 0.0 {
                sample
            } else {
                // Exponential smoothing: one late packet should not convince
                // us the link got slow, and one fast one should not convince
                // us it got quick.
                self.stats.rtt * 0.9 + sample * 0.1
            };

            let carried = entry.carried.clone();
            for id in carried {
                if let Some(index) = self.pending.iter().position(|message| message.id == id) {
                    self.pending.remove(index);
                }
            }
        }
    }

    /// Count packets that are old enough to be considered lost.
    fn expire_lost(&mut self) {
        let horizon = (self.stats.rtt * 3.0).max(0.2);
        while let Some(front) = self.sent.front() {
            if self.clock - front.time < horizon.max(1.0) {
                break;
            }
            if !front.acked {
                self.stats.packets_lost += 1;
            }
            self.sent.pop_front();
        }
        let tracked = self.stats.packets_sent.max(1);
        self.stats.loss = self.stats.packets_lost as f32 / tracked as f32;
    }
}
