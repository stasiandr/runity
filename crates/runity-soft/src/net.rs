//! Soft things over the network (docs/netsim.md): a simulation's state as
//! a compact frame of bytes, and the buffer a replica is shown from.
//!
//! * [`Frame`] — particles, the turns of a rod's links and a few numbers
//!   more (the pull on each end), quantized: positions in 16 bits an axis
//!   inside the frame's box, turns in four bytes. A rope of forty links,
//!   turns and all, is about 450 bytes.
//! * [`PresentedParticles`] — the owner's frames as they came, shown a
//!   couple of network ticks in the past between the two around the clock,
//!   the dacha simulator's interpolation buffer for bodies made to hold a
//!   shape. A change of owner is bent into the new owner's stream; a
//!   takeover starts the solver where the old owner has it *now*, carried
//!   forward by the delay.
//!
//! Headless and free of the network module: the facade registers
//! [`Frame`]'s bytes as a state (`Components::register_state`) and the
//! network carries them as it carries any other.

use std::collections::VecDeque;

use glam::{Quat, Vec3};

/// Network ticks a second: the network module's `NET_HZ`.
pub const NET_HZ: f32 = 30.0;
/// How far behind a replica is shown, network ticks: the network module's
/// `DELAY`.
pub const DELAY: f64 = 2.0;
/// Seconds over which a change of owner is bent into the new stream.
pub const HANDOVER_BLEND: f32 = 0.5;
/// The most change of speed a takeover carries forward, metres a second
/// a second: the bodies' `TAKEOVER_ACCEL`.
pub const TAKEOVER_ACCEL: f32 = 12.0;
/// Frames further apart than this, metres a second at some particle, are
/// a teleport: jumped, not slid through.
pub const SNAP_SPEED: f32 = 35.0;

/// One moment of a soft thing, as its owner sends it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    pub points: Vec<Vec3>,
    /// A rod's links' turns; empty for what has none.
    pub turns: Vec<Quat>,
    /// A few numbers more, the thing's own: a rope's pull on each end.
    pub extra: Vec<f32>,
}

impl Frame {
    /// The frame as bytes: counts, the box, the points in it at 16 bits
    /// an axis, the turns at four bytes, the extras whole.
    pub fn encode(&self) -> Vec<u8> {
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &self.points {
            low = low.min(*p);
            high = high.max(*p);
        }
        if self.points.is_empty() {
            (low, high) = (Vec3::ZERO, Vec3::ZERO);
        }
        let span = (high - low).max(Vec3::splat(1e-6));
        let mut out = Vec::with_capacity(32 + self.points.len() * 6 + self.turns.len() * 4 + self.extra.len() * 4);
        out.extend_from_slice(&(self.points.len() as u16).to_le_bytes());
        out.extend_from_slice(&(self.turns.len() as u16).to_le_bytes());
        out.push(self.extra.len().min(255) as u8);
        for v in [low, span] {
            for c in v.to_array() {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        for p in &self.points {
            let q = ((*p - low) / span * 65535.0).round().clamp(Vec3::ZERO, Vec3::splat(65535.0));
            for c in q.to_array() {
                out.extend_from_slice(&(c as u16).to_le_bytes());
            }
        }
        for q in &self.turns {
            out.extend_from_slice(&pack_quat(*q).to_le_bytes());
        }
        for e in self.extra.iter().take(255) {
            out.extend_from_slice(&e.to_le_bytes());
        }
        out
    }

    /// A frame back from its bytes; `None` for bytes that do not read —
    /// an old build, a truncated datagram: expected, never fatal.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader { bytes, at: 0 };
        let n = r.u16()? as usize;
        let m = r.u16()? as usize;
        let e = r.take(1)?[0] as usize;
        let low = r.vec3()?;
        let span = r.vec3()?;
        let mut points = Vec::with_capacity(n);
        for _ in 0..n {
            let q = Vec3::new(r.u16()? as f32, r.u16()? as f32, r.u16()? as f32) / 65535.0;
            points.push(low + q * span);
        }
        let mut turns = Vec::with_capacity(m);
        for _ in 0..m {
            turns.push(unpack_quat(u32::from_le_bytes(r.take(4)?.try_into().ok()?)));
        }
        let mut extra = Vec::with_capacity(e);
        for _ in 0..e {
            extra.push(r.f32()?);
        }
        Some(Self { points, turns, extra })
    }

    /// Between this and `other`, `t` of the way: points in a line, turns
    /// the short way round. Where the two differ in shape, this one.
    pub fn lerp(&self, other: &Frame, t: f32) -> Frame {
        if self.points.len() != other.points.len() || self.turns.len() != other.turns.len() {
            return if t < 0.5 { self.clone() } else { other.clone() };
        }
        Frame {
            points: self.points.iter().zip(&other.points).map(|(a, b)| a.lerp(*b, t)).collect(),
            turns: self.turns.iter().zip(&other.turns).map(|(a, b)| a.slerp(*b, t)).collect(),
            extra: if t < 0.5 { self.extra.clone() } else { other.extra.clone() },
        }
    }
}

/// Bytes read in order; `None` past the end.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.bytes.get(self.at..self.at + n)?;
        self.at += n;
        Some(s)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn vec3(&mut self) -> Option<Vec3> {
        Some(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
}

/// A unit quaternion in four bytes: the largest component dropped (its
/// index in two bits, its sign made positive), the other three in ten bits
/// each over ±1/√2 — the dacha simulator's packing, about a tenth of a
/// degree.
pub fn pack_quat(q: Quat) -> u32 {
    let q = q.normalize();
    let a = q.to_array();
    let (largest, _) = a.iter().enumerate().fold((0, 0.0f32), |(i, m), (j, v)| if v.abs() > m { (j, v.abs()) } else { (i, m) });
    let sign = if a[largest] < 0.0 { -1.0 } else { 1.0 };
    let mut out = largest as u32;
    let mut shift = 2;
    for (j, v) in a.iter().enumerate() {
        if j == largest {
            continue;
        }
        let x = (v * sign * std::f32::consts::SQRT_2 * 0.5 + 0.5).clamp(0.0, 1.0);
        out |= ((x * 1023.0).round() as u32) << shift;
        shift += 10;
    }
    out
}

pub fn unpack_quat(bits: u32) -> Quat {
    let largest = (bits & 3) as usize;
    let mut a = [0.0f32; 4];
    let mut shift = 2;
    let mut sum = 0.0;
    for (j, slot) in a.iter_mut().enumerate() {
        if j == largest {
            continue;
        }
        let x = ((bits >> shift) & 1023) as f32 / 1023.0;
        *slot = (x - 0.5) * 2.0 / std::f32::consts::SQRT_2;
        sum += *slot * *slot;
        shift += 10;
    }
    a[largest] = (1.0 - sum).max(0.0).sqrt();
    Quat::from_array(a).normalize()
}

/// What a takeover starts the solver from: where the old owner has the
/// thing now, how fast each particle goes, and the turns.
#[derive(Debug, Clone, PartialEq)]
pub struct Takeover {
    pub points: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    pub turns: Vec<Quat>,
}

/// A replica's frames as its owners sent them, and the clock it is shown
/// by.
#[derive(Debug, Clone, Default)]
pub struct PresentedParticles {
    /// (tick on this buffer's timeline, frame), oldest first.
    samples: VecDeque<(f64, Frame)>,
    sender: Option<u32>,
    /// Added to the current sender's ticks to put them on the timeline.
    offset: f64,
    /// Where the clock stands, timeline ticks.
    render: f64,
    /// A handover being hidden: each point's gap at the join, seconds
    /// since the picture reached it, the join's tick and the old stream's
    /// newest — between the two the gap grows in as the picture slides
    /// from one owner's frame to the other's, so it never jumps.
    blend: Option<(Vec<Vec3>, f32, f64, f64)>,
    /// Frames taken, for a test to count.
    pub taken: u64,
    /// Its particles keep a shape (a rope's links): a takeover starts from
    /// the newest frame as it is, and the ends are carried by what holds
    /// them.
    pub shape_held: bool,
}

/// Who a buffer is seeded as: this peer, showing what it had.
pub const HERE: u32 = u32::MAX;

impl PresentedParticles {
    /// A buffer that starts from what this peer shows now — a thing it
    /// has just stopped simulating — so the new owner's stream is joined
    /// from here, not jumped to (the bodies' buffer does the same).
    pub fn seeded(frame: Frame) -> Self {
        let mut out = Self::default();
        out.push(HERE, 0, frame.clone());
        out.push(HERE, 1, frame);
        out.render = 1.0;
        out
    }

    /// A frame from `sender`, at its tick.
    pub fn push(&mut self, sender: u32, tick: u64, frame: Frame) {
        let tick = tick as f64;
        match self.sender {
            None => {
                self.offset = 0.0;
                self.render = tick - DELAY;
            }
            Some(old) if old != sender => {
                // A new owner is a new clock: its first frame goes a delay
                // ahead of the picture, and the gap between where the old
                // stream was heading and where the new owner has it is
                // hidden over a moment.
                let newest = self.samples.back().map_or(self.render, |(t, _)| *t);
                let join = (self.render + DELAY).ceil().max(newest + 1.0);
                self.offset = join - tick;
                if let Some(heading) = self.frame_at(join) {
                    if heading.points.len() == frame.points.len() {
                        let gap = heading.points.iter().zip(&frame.points).map(|(h, p)| *h - *p).collect();
                        self.blend = Some((gap, 0.0, join, newest));
                    }
                }
            }
            _ => {}
        }
        self.sender = Some(sender);
        let at = tick + self.offset;
        if let Some((newest, last)) = self.samples.back() {
            if at <= *newest {
                return;
            }
            let seconds = ((at - newest) / NET_HZ as f64) as f32;
            let fastest = last.points.iter().zip(&frame.points).map(|(a, b)| a.distance(*b)).fold(0.0, f32::max);
            if last.points.len() != frame.points.len() || fastest / seconds.max(1e-3) > SNAP_SPEED {
                self.samples.clear();
                self.render = at - DELAY;
                self.blend = None;
            }
        }
        self.taken += 1;
        self.samples.push_back((at, frame));
        while self.samples.len() > 8 {
            self.samples.pop_front();
        }
    }

    /// Whether anything has come in.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The frame at a timeline tick, between the two around it; held at
    /// the ends — never guessed past the newest.
    fn frame_at(&self, at: f64) -> Option<Frame> {
        let (first_at, first) = self.samples.front()?;
        if at <= *first_at {
            return Some(first.clone());
        }
        for pair in self.samples.iter().collect::<Vec<_>>().windows(2) {
            let ((a_at, a), (b_at, b)) = (pair[0], pair[1]);
            if at <= *b_at {
                let t = ((at - a_at) / (b_at - a_at).max(1e-6)) as f32;
                return Some(a.lerp(b, t));
            }
        }
        self.samples.back().map(|(_, f)| f.clone())
    }

    /// On by `seconds`: the frame to show now, with any handover's gap
    /// fading out. The clock keeps the delay behind the newest, catching
    /// up gently when it falls behind and waiting when it runs ahead.
    pub fn advance(&mut self, seconds: f32) -> Option<Frame> {
        let newest = self.samples.back()?.0;
        let target = newest - DELAY;
        let ticks = seconds as f64 * NET_HZ as f64;
        self.render += ticks;
        let off = target - self.render;
        if off.abs() > 4.0 {
            self.render = target;
        } else {
            // Steered, not assigned, as the bodies' buffer: a tenth of the
            // way a tick.
            self.render += off * (0.1 * ticks).min(1.0);
        }
        self.render = self.render.min(newest);
        while self.samples.len() > 2 && self.samples[1].0 < self.render - 1.0 {
            self.samples.pop_front();
        }
        let mut frame = self.frame_at(self.render)?;
        if let Some((gap, since, join, from)) = &mut self.blend {
            let weight = if self.render >= *join {
                *since += seconds;
                let x = (*since / HANDOVER_BLEND).min(1.0);
                1.0 - x * x * (3.0 - 2.0 * x)
            } else {
                ((self.render - *from) / (*join - *from).max(1e-6)).clamp(0.0, 1.0) as f32
            };
            if gap.len() == frame.points.len() {
                for (p, g) in frame.points.iter_mut().zip(gap.iter()) {
                    *p += *g * weight;
                }
            }
            if self.render >= *join && *since >= HANDOVER_BLEND {
                self.blend = None;
            }
        }
        Some(frame)
    }

    /// Where the owner has it now and how fast each particle goes: the
    /// newest frame carried forward over the ticks since it was sent —
    /// `one_way` on the way here and one more — by the speed between the
    /// newest two. `None` with fewer than two.
    pub fn takeover(&self, one_way: f64) -> Option<Takeover> {
        let n = self.samples.len();
        if n < 2 {
            return None;
        }
        let (newest_at, newest) = &self.samples[n - 1];
        let (before_at, before) = &self.samples[n - 2];
        if newest.points.len() != before.points.len() {
            return None;
        }
        let ticks = (newest_at - before_at).max(1e-6) as f32;
        let lead = ((self.render + DELAY - newest_at).max(0.0) + 1.0 + one_way.max(0.0)) as f32;
        // As the bodies' takeover: speed from the newest two, and how it
        // was changing from a third (falling, swinging), clamped so that a
        // knock is not carried on — then a thing held by a body and the
        // body are carried forward alike.
        let older = (n >= 3).then(|| &self.samples[n - 3]).filter(|(_, f)| f.points.len() == newest.points.len());
        let most = TAKEOVER_ACCEL / (NET_HZ * NET_HZ);
        let mut points = Vec::with_capacity(newest.points.len());
        let mut velocities = Vec::with_capacity(newest.points.len());
        for i in 0..newest.points.len() {
            let mut per_tick = (newest.points[i] - before.points[i]) / ticks;
            let mut accel = Vec3::ZERO;
            if let Some((older_at, older)) = older {
                let earlier_ticks = (before_at - older_at).max(1e-6) as f32;
                let earlier = (before.points[i] - older.points[i]) / earlier_ticks;
                accel = ((per_tick - earlier) / (0.5 * (ticks + earlier_ticks))).clamp_length_max(most);
                per_tick += accel * (0.5 * ticks);
            }
            if self.shape_held {
                // A thing held in a shape (a chain) is not carried on
                // particle by particle — each on its own line it comes out
                // longer and zigzag; what holds its ends carries it.
                points.push(newest.points[i]);
                velocities.push(per_tick * NET_HZ);
            } else {
                points.push(newest.points[i] + per_tick * lead + accel * (0.5 * lead * lead));
                velocities.push((per_tick + accel * lead) * NET_HZ);
            }
        }
        Some(Takeover { points, velocities, turns: newest.turns.clone() })
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips_within_a_fraction_of_a_millimetre_and_a_tenth_of_a_degree() {
        let frame = Frame {
            points: (0..41).map(|i| Vec3::new(i as f32 * 0.1, (i as f32 * 0.3).sin() * 0.5 + 2.0, -1.0)).collect(),
            turns: (0..40).map(|i| Quat::from_rotation_y(i as f32 * 0.2) * Quat::from_rotation_x(0.3)).collect(),
            extra: vec![12.5, -3.0],
        };
        let bytes = frame.encode();
        assert!(bytes.len() < 460, "{} bytes", bytes.len());
        let back = Frame::decode(&bytes).unwrap();
        for (a, b) in frame.points.iter().zip(&back.points) {
            assert!(a.distance(*b) < 1e-3, "{a} {b}");
        }
        for (a, b) in frame.turns.iter().zip(&back.turns) {
            assert!(a.angle_between(*b) < 0.004, "{a} {b}");
        }
        assert_eq!(back.extra, frame.extra);
        assert!(Frame::decode(&bytes[..10]).is_none());
    }

    fn frame(x: f32) -> Frame {
        Frame { points: vec![Vec3::new(x, 0.0, 0.0), Vec3::new(x, -1.0, 0.0)], ..Default::default() }
    }

    #[test]
    fn a_replica_is_shown_between_frames_a_delay_behind_and_taken_over_ahead() {
        let mut p = PresentedParticles::default();
        // Moving at 0.1 m a tick, shown a frame at a time.
        let mut shown = Frame::default();
        for tick in 0..10u64 {
            p.push(1, tick, frame(tick as f32 * 0.1));
            shown = p.advance(1.0 / NET_HZ).unwrap();
        }
        assert!((shown.points[0].x - 0.7).abs() < 0.06, "a delay behind the newest: {}", shown.points[0].x);
        let t = p.takeover(1.0).unwrap();
        assert!(t.points[0].x > 0.9, "carried past the newest: {}", t.points[0].x);
        // Steady: nothing speeding up, so nothing carried as such.
        assert!((t.velocities[0].x - 3.0).abs() < 1e-3);
    }

    #[test]
    fn a_new_owner_is_joined_not_jumped_to() {
        let mut p = PresentedParticles::default();
        for tick in 0..6u64 {
            p.push(1, tick, frame(0.0));
            p.advance(1.0 / NET_HZ);
        }
        // The new owner has it 0.3 m over, on its own clock.
        p.push(2, 500, frame(0.3));
        p.push(2, 501, frame(0.3));
        let mut last = p.advance(1.0 / NET_HZ).unwrap().points[0].x;
        let mut biggest = 0.0f32;
        for _ in 0..30 {
            let x = p.advance(1.0 / NET_HZ).unwrap().points[0].x;
            biggest = biggest.max((x - last).abs());
            last = x;
        }
        assert!(biggest < 0.05, "no jump: {biggest}");
        assert!((last - 0.3).abs() < 1e-3, "arrived: {last}");
    }
}
