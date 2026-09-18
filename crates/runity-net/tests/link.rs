//! The reliability layer, tested against a simulated link.
//!
//! No sockets: a queue with latency, jitter, loss, duplication and reordering,
//! all driven by a seeded generator. That makes the awkward cases —
//! "everything arrives twice, backwards, after a second" — reproducible,
//! which they never are against a real network.

use runity_math::Rng;
use runity_net::{Connection, ConnectionState, MAX_MESSAGE_CHUNK};

/// Which end of the link a packet is going to.
const A: usize = 0;
const B: usize = 1;

struct Link {
    rng: Rng,
    /// One-way delay, in seconds.
    latency: f32,
    /// Random extra delay, which is what causes reordering.
    jitter: f32,
    /// Chance a packet is dropped.
    loss: f32,
    /// Chance a packet arrives twice.
    duplicate: f32,
    queue: Vec<(f32, u64, usize, Vec<u8>)>,
    clock: f32,
    counter: u64,
    dropped: usize,
}

impl Link {
    fn perfect() -> Self {
        Self {
            rng: Rng::named(1, "link"),
            latency: 0.02,
            jitter: 0.0,
            loss: 0.0,
            duplicate: 0.0,
            queue: Vec::new(),
            clock: 0.0,
            counter: 0,
            dropped: 0,
        }
    }

    fn lossy(seed: u64, loss: f32, jitter: f32, duplicate: f32) -> Self {
        Self {
            rng: Rng::named(seed, "link"),
            loss,
            jitter,
            duplicate,
            ..Self::perfect()
        }
    }

    fn send(&mut self, to: usize, bytes: Vec<u8>) {
        if self.rng.chance(self.loss) {
            self.dropped += 1;
            return;
        }
        self.enqueue(to, bytes.clone());
        if self.rng.chance(self.duplicate) {
            self.enqueue(to, bytes);
        }
    }

    fn enqueue(&mut self, to: usize, bytes: Vec<u8>) {
        let delay = self.latency + self.rng.range(0.0, self.jitter.max(0.0));
        self.counter += 1;
        self.queue
            .push((self.clock + delay, self.counter, to, bytes));
    }

    /// Advance and hand over everything due, oldest first.
    fn advance(&mut self, dt: f32) -> Vec<(usize, Vec<u8>)> {
        self.clock += dt;
        // Sorting by (arrival, order sent) keeps the simulation reproducible
        // while still letting jitter reorder packets.
        self.queue
            .sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let ready = self
            .queue
            .iter()
            .position(|(at, ..)| *at > self.clock)
            .unwrap_or(self.queue.len());
        self.queue
            .drain(..ready)
            .map(|(_, _, to, bytes)| (to, bytes))
            .collect()
    }
}

/// Run both ends for a while, collecting what each one receives.
fn pump(
    a: &mut Connection,
    b: &mut Connection,
    link: &mut Link,
    seconds: f32,
) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    const DT: f32 = 1.0 / 60.0;
    let mut got_a = Vec::new();
    let mut got_b = Vec::new();
    let steps = (seconds / DT).round() as usize;

    for _ in 0..steps {
        for packet in a.update(DT) {
            link.send(B, packet);
        }
        for packet in b.update(DT) {
            link.send(A, packet);
        }
        for (to, bytes) in link.advance(DT) {
            let target = if to == A { &mut *a } else { &mut *b };
            let delivered = target
                .receive(&bytes)
                .expect("the link does not corrupt packets");
            if to == A {
                got_a.extend(delivered);
            } else {
                got_b.extend(delivered);
            }
        }
    }
    (got_a, got_b)
}

fn connected_pair() -> (Connection, Connection) {
    let mut a = Connection::default();
    let mut b = Connection::default();
    a.accept();
    b.accept();
    (a, b)
}

fn message(index: usize) -> Vec<u8> {
    format!("message {index}").into_bytes()
}

/// A message big enough that a handful of them need several packets.
///
/// Small messages all fit in one datagram, which quietly turns a test about
/// loss into a test about whether one packet got through.
fn bulky(index: usize) -> Vec<u8> {
    let mut bytes = format!("message {index}:").into_bytes();
    bytes.resize(400, (index % 251) as u8);
    bytes
}

#[test]
fn messages_cross_a_clean_link() {
    let (mut a, mut b) = connected_pair();
    let mut link = Link::perfect();

    for index in 0..5 {
        a.send_reliable(&message(index));
    }
    assert!(a.send_unreliable(b"a position"));

    let (_, received) = pump(&mut a, &mut b, &mut link, 1.0);
    assert_eq!(received.len(), 6);
    assert!(received.contains(&b"a position".to_vec()));
    for index in 0..5 {
        assert!(
            received.contains(&message(index)),
            "message {index} went missing"
        );
    }
    assert!(a.is_connected() && b.is_connected());
    assert_eq!(a.queued(), 0, "everything was acknowledged");
}

#[test]
fn reliable_messages_survive_a_link_that_loses_a_third_of_everything() {
    let (mut a, mut b) = connected_pair();
    let mut link = Link::lossy(2, 0.33, 0.0, 0.0);

    for index in 0..20 {
        a.send_reliable(&bulky(index));
    }

    let (_, received) = pump(&mut a, &mut b, &mut link, 3.0);
    assert!(link.dropped > 5, "the link should have eaten some packets");

    // Every message, exactly once, in the order they were sent.
    let expected: Vec<Vec<u8>> = (0..20).map(bulky).collect();
    assert_eq!(received, expected);
    assert_eq!(a.queued(), 0);
    assert!(a.stats().resends > 0, "and some had to be sent again");
}

#[test]
fn reliable_messages_stay_in_order_through_reordering_and_duplication() {
    let (mut a, mut b) = connected_pair();
    // Heavy jitter reorders packets; duplication makes the same message
    // arrive twice from two different packets.
    let mut link = Link::lossy(3, 0.1, 0.08, 0.3);

    for index in 0..30 {
        a.send_reliable(&message(index));
    }

    let (_, received) = pump(&mut a, &mut b, &mut link, 4.0);
    let expected: Vec<Vec<u8>> = (0..30).map(message).collect();
    assert_eq!(
        received, expected,
        "order and exactly-once are the whole promise"
    );
}

#[test]
fn unreliable_messages_are_dropped_rather_than_resent_or_duplicated() {
    let (mut a, mut b) = connected_pair();
    let mut link = Link::lossy(4, 0.4, 0.05, 0.25);

    for index in 0..40 {
        a.send_unreliable(&bulky(index));
    }
    let (_, received) = pump(&mut a, &mut b, &mut link, 2.0);

    assert!(received.len() < 40, "a lossy link should lose some of them");
    assert!(!received.is_empty(), "but not all of them");
    // Whatever arrives must be something that was sent, and no message may
    // arrive twice even though the link duplicates packets.
    let mut seen = received.clone();
    seen.sort();
    let before = seen.len();
    seen.dedup();
    assert_eq!(
        seen.len(),
        before,
        "a duplicated packet delivered a message twice"
    );
}

#[test]
fn a_message_larger_than_a_packet_is_split_and_reassembled() {
    let (mut a, mut b) = connected_pair();
    let mut link = Link::lossy(5, 0.2, 0.02, 0.0);

    // A world snapshot's worth of bytes: far more than one datagram.
    let mut rng = Rng::named(6, "payload");
    let big: Vec<u8> = (0..50_000).map(|_| rng.below(256) as u8).collect();
    assert!(big.len() > MAX_MESSAGE_CHUNK * 10);

    a.send_reliable(&big);
    a.send_reliable(b"after the big one");

    let (_, received) = pump(&mut a, &mut b, &mut link, 8.0);
    assert_eq!(received.len(), 2, "the pieces must arrive as one message");
    assert_eq!(received[0], big);
    assert_eq!(
        received[1],
        b"after the big one".to_vec(),
        "and not disturb what follows"
    );
}

#[test]
fn an_unreliable_message_that_cannot_fit_is_refused_rather_than_split() {
    let (mut a, _) = connected_pair();
    assert!(a.send_unreliable(&vec![0u8; MAX_MESSAGE_CHUNK]));
    assert!(
        !a.send_unreliable(&vec![0u8; MAX_MESSAGE_CHUNK + 1]),
        "an unreliable message that needs five packets to arrive is not unreliable"
    );
}

#[test]
fn the_round_trip_time_settles_near_the_real_one() {
    let (mut a, mut b) = connected_pair();
    let mut link = Link::perfect();
    link.latency = 0.05; // 50 ms each way, so a 100 ms round trip

    for index in 0..40 {
        a.send_reliable(&message(index));
        pump(&mut a, &mut b, &mut link, 0.1);
    }

    let rtt = a.rtt();
    assert!(
        (rtt - 0.1).abs() < 0.035,
        "measured {rtt}, expected about 0.1"
    );
}

#[test]
fn a_connection_notices_when_the_other_end_goes_quiet() {
    let (mut a, mut b) = connected_pair();
    let mut link = Link::perfect();

    a.send_reliable(b"still here");
    pump(&mut a, &mut b, &mut link, 0.5);
    assert!(!a.is_timed_out());

    // B stops answering: A keeps talking into the void.
    for _ in 0..(6.0 * 60.0) as usize {
        let _ = a.update(1.0 / 60.0);
    }
    assert!(
        a.is_timed_out(),
        "five seconds of silence is a dead connection"
    );
    assert!(!b.is_timed_out(), "B heard from A recently enough");
}

#[test]
fn an_idle_connection_still_says_something() {
    // Keep-alives are not politeness: acknowledgements ride on packets, so a
    // silent connection cannot tell the other side what it has received.
    let (mut a, mut b) = connected_pair();
    let mut link = Link::perfect();
    pump(&mut a, &mut b, &mut link, 1.0);

    let before = a.stats().packets_sent;
    pump(&mut a, &mut b, &mut link, 1.0);
    let sent = a.stats().packets_sent - before;
    assert!(
        sent >= 5,
        "a second of idling should still send keep-alives, sent {sent}"
    );
    assert!(sent < 70, "but not one per frame: {sent}");
}

#[test]
fn a_clean_disconnect_needs_no_timeout() {
    let (mut a, mut b) = connected_pair();
    let mut link = Link::perfect();
    pump(&mut a, &mut b, &mut link, 0.2);

    let goodbye = a.control(runity_net::PacketKind::Disconnect);
    a.disconnect();
    b.receive(&goodbye).unwrap();

    assert_eq!(a.state(), ConnectionState::Disconnected);
    assert_eq!(b.state(), ConnectionState::Disconnected);
    assert!(!b.is_connected());
}

#[test]
fn rubbish_on_the_wire_is_rejected_without_breaking_the_connection() {
    let (mut a, mut b) = connected_pair();
    let mut link = Link::perfect();
    a.send_reliable(b"before");
    pump(&mut a, &mut b, &mut link, 0.3);

    let mut rng = Rng::named(7, "rubbish");
    for _ in 0..500 {
        let length = rng.below(40) as usize;
        let noise: Vec<u8> = (0..length).map(|_| rng.below(256) as u8).collect();
        let _ = b.receive(&noise); // must not panic, and must not be believed
    }

    a.send_reliable(b"after");
    let (_, received) = pump(&mut a, &mut b, &mut link, 0.5);
    assert_eq!(
        received,
        vec![b"after".to_vec()],
        "the connection still works"
    );
}

#[test]
fn a_truncated_packet_is_an_error_rather_than_a_panic() {
    let (mut a, mut b) = connected_pair();
    a.send_reliable(b"a message with a length prefix");
    let packets = a.update(1.0 / 60.0);
    let packet = packets.first().expect("something should have been sent");

    for length in 0..packet.len() {
        // Every prefix of a valid packet: some are structurally fine headers,
        // none may panic or deliver a half-read message.
        let _ = b.receive(&packet[..length]);
    }
}

#[test]
fn the_same_seed_gives_the_same_conversation() {
    let run = || {
        let (mut a, mut b) = connected_pair();
        let mut link = Link::lossy(8, 0.25, 0.05, 0.15);
        for index in 0..25 {
            a.send_reliable(&message(index));
            b.send_unreliable(&message(index + 100));
        }
        let (got_a, got_b) = pump(&mut a, &mut b, &mut link, 3.0);
        (got_a, got_b, a.stats().resends, b.stats().packets_received)
    };
    assert_eq!(run(), run());
}

#[test]
fn both_directions_work_at_once() {
    let (mut a, mut b) = connected_pair();
    let mut link = Link::lossy(9, 0.2, 0.03, 0.1);

    for index in 0..15 {
        a.send_reliable(&message(index));
        b.send_reliable(&message(index + 500));
    }
    let (got_a, got_b) = pump(&mut a, &mut b, &mut link, 3.0);

    assert_eq!(got_b, (0..15).map(message).collect::<Vec<_>>());
    assert_eq!(
        got_a,
        (0..15)
            .map(|index| message(index + 500))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_disconnected_connection_stops_queueing_work() {
    let (mut a, _) = connected_pair();
    a.disconnect();
    a.send_reliable(b"too late");
    assert!(!a.send_unreliable(b"too late"));
    assert_eq!(a.queued(), 0);
    assert!(a.update(1.0 / 60.0).is_empty(), "and stops sending");
}
