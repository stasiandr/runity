//! Packet headers, sequence numbers and acknowledgements.
//!
//! UDP gives one guarantee: a datagram that arrives is the datagram that was
//! sent. Everything else — that it arrives at all, once, or in order — has to
//! be built, and a game wants to build it *selectively*. Positions want to be
//! fast and may be dropped, because a newer one is along in 50 ms; a command
//! to fell a tree must arrive exactly once. TCP cannot express that difference
//! and makes everything wait behind the oldest lost byte, which is why games
//! do not use it.

use runity_serialize::{Error, Reader, Result, Writer};

/// Four bytes at the front of every packet: anything else is not ours.
///
/// Not a security measure — it stops a stray datagram on a reused port from
/// being decoded as a game packet, which is otherwise a very confusing hour.
pub const PROTOCOL: u32 = 0x524E_5401; // "RNT" + version 1

/// Bytes of header before the payload.
pub const HEADER_LEN: usize = 14;

/// Largest packet worth sending.
///
/// 1200 bytes fits inside the smallest MTU anyone still has, with room for
/// IPv6 and UDP headers. Anything larger risks fragmentation at the IP layer,
/// where losing one fragment loses the whole datagram.
pub const MAX_PACKET: usize = 1200;

/// What a packet is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketKind {
    /// A connection request from a client that is not connected yet.
    Connect,
    /// The server accepting one.
    Accept,
    /// Carrying messages.
    Payload,
    /// Nothing to say, but proof of being alive and a round-trip sample.
    KeepAlive,
    /// A clean goodbye, so the other side need not wait for a timeout.
    Disconnect,
}

impl PacketKind {
    fn tag(self) -> u8 {
        match self {
            PacketKind::Connect => 0,
            PacketKind::Accept => 1,
            PacketKind::Payload => 2,
            PacketKind::KeepAlive => 3,
            PacketKind::Disconnect => 4,
        }
    }

    fn from_tag(tag: u8) -> Option<PacketKind> {
        Some(match tag {
            0 => PacketKind::Connect,
            1 => PacketKind::Accept,
            2 => PacketKind::Payload,
            3 => PacketKind::KeepAlive,
            4 => PacketKind::Disconnect,
            _ => return None,
        })
    }
}

/// The header every packet carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// What this packet is.
    pub kind: PacketKind,
    /// This packet's own number.
    pub sequence: u16,
    /// The newest packet number seen from the other side.
    pub ack: u16,
    /// Which of the 32 packets before `ack` were also seen.
    ///
    /// Repeating the acknowledgement history in every packet is what makes
    /// acks themselves robust: losing one costs nothing, because the next
    /// packet says the same thing again.
    pub ack_bits: u32,
}

impl Header {
    /// Write the header.
    pub fn write(&self, writer: &mut Writer) {
        writer
            .u32(PROTOCOL)
            .u8(self.kind.tag())
            .u8(0) // reserved: alignment for a future channel or flags byte
            .u16(self.sequence)
            .u16(self.ack)
            .u32(self.ack_bits);
    }

    /// Read a header, rejecting anything that is not one of ours.
    pub fn read(reader: &mut Reader) -> Result<Header> {
        let position = reader.position();
        if reader.u32()? != PROTOCOL {
            return Err(Error::InvalidValue {
                position,
                what: "packet protocol",
            });
        }
        let tag = reader.u8()?;
        let kind = PacketKind::from_tag(tag).ok_or(Error::InvalidValue {
            position,
            what: "packet kind",
        })?;
        let _reserved = reader.u8()?;
        Ok(Header {
            kind,
            sequence: reader.u16()?,
            ack: reader.u16()?,
            ack_bits: reader.u32()?,
        })
    }
}

/// Whether `a` is newer than `b`, allowing for the counter wrapping.
///
/// Sequence numbers are 16 bits and will wrap after about twenty minutes at
/// 60 packets a second. Comparing them as plain integers means that at that
/// moment every packet looks ancient and the connection dies. The trick is to
/// ask which one is *closer going forward*.
#[inline]
pub fn newer_than(a: u16, b: u16) -> bool {
    const HALF: u16 = 32_768;
    (a > b && a - b <= HALF) || (b > a && b - a > HALF)
}

/// Difference between two sequence numbers, wrapping.
#[inline]
pub fn distance(a: u16, b: u16) -> u16 {
    a.wrapping_sub(b)
}

/// What has been received, and the acknowledgement to send back.
///
/// Keeps the last 32 packets as a bitfield alongside the newest sequence —
/// the format the header carries, so answering costs nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AckState {
    newest: u16,
    bits: u32,
    started: bool,
}

impl AckState {
    /// Nothing received yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The newest sequence seen.
    pub fn newest(&self) -> u16 {
        self.newest
    }

    /// The bitfield of the 32 packets before it.
    pub fn bits(&self) -> u32 {
        self.bits
    }

    /// Whether a given sequence has been seen.
    pub fn seen(&self, sequence: u16) -> bool {
        if !self.started {
            return false;
        }
        if sequence == self.newest {
            return true;
        }
        if newer_than(sequence, self.newest) {
            return false;
        }
        let back = distance(self.newest, sequence);
        back <= 32 && self.bits & (1 << (back - 1)) != 0
    }

    /// Record a received sequence.
    pub fn receive(&mut self, sequence: u16) -> Reception {
        if !self.started {
            self.started = true;
            self.newest = sequence;
            return Reception::Fresh;
        }
        if sequence == self.newest || self.seen(sequence) {
            return Reception::Duplicate;
        }
        if newer_than(sequence, self.newest) {
            let shift = distance(sequence, self.newest);
            if shift >= 32 {
                // A long gap: everything in the old window is unreachable now.
                self.bits = 0;
            } else {
                self.bits = (self.bits << shift) | (1 << (shift - 1));
            }
            self.newest = sequence;
            Reception::Fresh
        } else {
            let back = distance(self.newest, sequence);
            if back > 32 {
                // Beyond the window there is no way to tell a new packet from
                // one already seen. Saying so, rather than guessing, lets the
                // caller decide per message: reliable ones are deduplicated by
                // their own ids, so they are safe to process; unreliable ones
                // are not, and are expendable anyway.
                return Reception::TooOld;
            }
            self.bits |= 1 << (back - 1);
            Reception::Fresh
        }
    }
}

/// What arrived, from the acknowledgement window's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reception {
    /// Not seen before; recorded and acknowledged.
    Fresh,
    /// Seen before. Whatever is inside it has already been dealt with.
    Duplicate,
    /// So far behind that the window cannot say either way.
    TooOld,
}

impl Reception {
    /// Whether this packet is one to acknowledge and process fully.
    pub fn is_fresh(&self) -> bool {
        *self == Reception::Fresh
    }
}

/// Walk an acknowledgement back into the individual sequences it covers.
///
/// The header says "I have `ack`, and these 32 before it"; the sender needs
/// that as a list to tick off what it can stop resending.
pub fn acked_sequences(ack: u16, ack_bits: u32) -> impl Iterator<Item = u16> {
    core::iter::once(ack).chain(
        (0..32)
            .filter(move |bit| ack_bits & (1 << bit) != 0)
            .map(move |bit| ack.wrapping_sub(bit + 1)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_serialize::Reader;

    #[test]
    fn a_header_survives_a_round_trip() {
        let header = Header {
            kind: PacketKind::Payload,
            sequence: 40_000,
            ack: 39_990,
            ack_bits: 0xDEAD_BEEF,
        };
        let mut writer = Writer::new();
        header.write(&mut writer);
        assert_eq!(writer.len(), HEADER_LEN);

        let mut reader = Reader::new(writer.as_bytes());
        assert_eq!(Header::read(&mut reader).unwrap(), header);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn a_foreign_packet_is_refused() {
        let mut writer = Writer::new();
        writer.u32(0xDEAD_0000).u8(2).u8(0).u16(1).u16(0).u32(0);
        assert!(matches!(
            Header::read(&mut Reader::new(writer.as_bytes())),
            Err(Error::InvalidValue {
                what: "packet protocol",
                ..
            })
        ));

        let mut writer = Writer::new();
        writer.u32(PROTOCOL).u8(99).u8(0).u16(1).u16(0).u32(0);
        assert!(matches!(
            Header::read(&mut Reader::new(writer.as_bytes())),
            Err(Error::InvalidValue {
                what: "packet kind",
                ..
            })
        ));

        assert!(
            Header::read(&mut Reader::new(&[1, 2])).is_err(),
            "a runt packet is not a header"
        );
    }

    #[test]
    fn newer_than_survives_the_counter_wrapping() {
        assert!(newer_than(2, 1));
        assert!(!newer_than(1, 2));
        // The case that kills naive comparison: twenty minutes in, the
        // counter rolls over and 0 has to count as newer than 65535.
        assert!(newer_than(0, 65_535));
        assert!(newer_than(10, 65_530));
        assert!(!newer_than(65_535, 0));
        assert!(!newer_than(5, 5), "a sequence is not newer than itself");
    }

    #[test]
    fn acks_record_what_arrived() {
        let mut acks = AckState::new();
        assert!(!acks.seen(1));

        assert_eq!(acks.receive(1), Reception::Fresh);
        assert_eq!(acks.newest(), 1);
        assert!(acks.seen(1));
        assert_eq!(
            acks.receive(1),
            Reception::Duplicate,
            "the same packet twice"
        );

        assert_eq!(acks.receive(2), Reception::Fresh);
        assert_eq!(acks.receive(3), Reception::Fresh);
        assert_eq!(acks.newest(), 3);
        assert!(acks.seen(1) && acks.seen(2) && acks.seen(3));
    }

    #[test]
    fn a_gap_is_remembered_and_can_be_filled_later() {
        let mut acks = AckState::new();
        acks.receive(1);
        acks.receive(4); // 2 and 3 went missing
        assert!(acks.seen(1) && acks.seen(4));
        assert!(!acks.seen(2) && !acks.seen(3));

        // They turn up late, out of order, and are still counted.
        assert_eq!(acks.receive(3), Reception::Fresh);
        assert!(acks.seen(3));
        assert_eq!(acks.receive(2), Reception::Fresh);
        assert!(acks.seen(2));
        assert_eq!(
            acks.newest(),
            4,
            "arriving late does not make a packet newest"
        );
        assert_eq!(
            acks.receive(2),
            Reception::Duplicate,
            "still a duplicate the second time"
        );
    }

    #[test]
    fn packets_older_than_the_window_are_dropped() {
        let mut acks = AckState::new();
        for sequence in 1..=100 {
            acks.receive(sequence);
        }
        assert!(acks.seen(100) && acks.seen(70));
        assert!(!acks.seen(50), "fifty packets back is beyond the window");
        assert_eq!(
            acks.receive(50),
            Reception::TooOld,
            "and cannot be recorded any more"
        );
    }

    #[test]
    fn a_long_silence_resets_the_window_rather_than_corrupting_it() {
        let mut acks = AckState::new();
        acks.receive(1);
        acks.receive(1_000); // a huge gap: nothing in between is knowable
        assert_eq!(acks.newest(), 1_000);
        assert!(acks.seen(1_000));
        assert!(!acks.seen(999));
        assert!(!acks.seen(1));
    }

    #[test]
    fn acks_work_across_the_wrap() {
        let mut acks = AckState::new();
        acks.receive(65_534);
        acks.receive(65_535);
        acks.receive(0);
        acks.receive(1);
        assert_eq!(acks.newest(), 1);
        for sequence in [65_534, 65_535, 0, 1] {
            assert!(acks.seen(sequence), "{sequence} should still be remembered");
        }
    }

    #[test]
    fn an_acknowledgement_expands_into_the_sequences_it_covers() {
        let covered: Vec<u16> = acked_sequences(100, 0b1011).collect();
        assert_eq!(covered, vec![100, 99, 98, 96]);

        // And across the wrap, which is where off-by-one arithmetic shows up.
        let covered: Vec<u16> = acked_sequences(1, 0b11).collect();
        assert_eq!(covered, vec![1, 0, 65_535]);

        assert_eq!(
            acked_sequences(7, 0).count(),
            1,
            "no bits means just the ack itself"
        );
    }

    #[test]
    fn what_was_received_is_what_gets_acknowledged() {
        // The loop that matters: whatever AckState recorded must come back
        // out of the header it produces.
        let mut acks = AckState::new();
        let arrived = [1u16, 2, 5, 6, 9, 20, 21];
        for sequence in arrived {
            acks.receive(sequence);
        }
        let covered: Vec<u16> = acked_sequences(acks.newest(), acks.bits()).collect();
        for sequence in arrived {
            assert!(
                covered.contains(&sequence),
                "{sequence} arrived but was not acknowledged"
            );
        }
        for missing in [3u16, 4, 7, 8, 10] {
            assert!(!covered.contains(&missing), "{missing} never arrived");
        }
    }
}
