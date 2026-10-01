//! A track's keys and the curve between them — Unity's AnimationCurve, as
//! a line of text a person reads.
//!
//! A key is `(time, value)`, and between two such keys the track's `ease`
//! carries the value, straight by default: what every clip written before
//! keys had shapes still says. A key with a shape says it third —
//! `(0.5, 90.0, Auto)` — and the curve out of and into it then bends
//! through its slope (a cubic Hermite, as Unity's and Maya's are):
//!
//! * `Auto` — the slope of its neighbours, flat where it is a peak or a
//!   valley and never past them: Unity's Clamped Auto, so a curve through
//!   two close keys does not overshoot.
//! * `Flat` — level: eases in and out.
//! * `Linear` — straight to each neighbour.
//! * `Step` — holds until the next key: blocking.
//! * `Free(slope)` — one slope, per second, on both sides.
//! * `Broken(in, out)` — a slope each side.

use serde::{Deserialize, Serialize};

use crate::ease::Ease;

/// How the curve leaves and reaches a key.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Tangent {
    /// No shape of its own: the track's `ease` to the next key.
    #[default]
    Eased,
    Auto,
    Flat,
    Linear,
    Step,
    Free(f32),
    Broken(f32, f32),
}

/// One key: a value at a time, and how the curve passes through it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Key {
    pub time: f32,
    pub value: f32,
    pub tangent: Tangent,
}

impl Key {
    pub fn new(time: f32, value: f32) -> Self {
        Self { time, value, tangent: Tangent::Eased }
    }

    pub fn shaped(time: f32, value: f32, tangent: Tangent) -> Self {
        Self { time, value, tangent }
    }
}

impl From<(f32, f32)> for Key {
    fn from((time, value): (f32, f32)) -> Self {
        Self::new(time, value)
    }
}

impl PartialEq<(f32, f32)> for Key {
    fn eq(&self, other: &(f32, f32)) -> bool {
        self.tangent == Tangent::Eased && self.time == other.0 && self.value == other.1
    }
}

// `(t, v)` or `(t, v, Shape)`: a tuple, so a track's keys stay on its line.
impl Serialize for Key {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;
        let shaped = self.tangent != Tangent::Eased;
        let mut t = s.serialize_tuple(if shaped { 3 } else { 2 })?;
        t.serialize_element(&self.time)?;
        t.serialize_element(&self.value)?;
        if shaped {
            t.serialize_element(&self.tangent)?;
        }
        t.end()
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> serde::de::Visitor<'de> for Visit {
            type Value = Key;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a key, (time, value) or (time, value, Auto)")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Key, A::Error> {
                use serde::de::Error;
                let time = seq.next_element()?.ok_or_else(|| A::Error::invalid_length(0, &self))?;
                let value = seq.next_element()?.ok_or_else(|| A::Error::invalid_length(1, &self))?;
                let tangent = seq.next_element()?.unwrap_or_default();
                Ok(Key { time, value, tangent })
            }
        }
        d.deserialize_tuple(3, Visit)
    }
}

/// A key's slope in and out, per second, as the curve passes it. `None`
/// for a side the track's ease carries (an `Eased` key between two others
/// the same).
pub fn slopes(keys: &[Key], i: usize) -> (f32, f32) {
    let k = keys[i];
    let before = i.checked_sub(1).map(|j| keys[j]);
    let after = keys.get(i + 1).copied();
    let slope = |a: Key, b: Key| if b.time > a.time { (b.value - a.value) / (b.time - a.time) } else { 0.0 };
    let toward_before = before.map_or(0.0, |b| slope(b, k));
    let toward_after = after.map_or(0.0, |a| slope(k, a));
    match k.tangent {
        Tangent::Eased | Tangent::Linear => (
            before.map_or(toward_after, |_| toward_before),
            after.map_or(toward_before, |_| toward_after),
        ),
        Tangent::Step | Tangent::Flat => (0.0, 0.0),
        Tangent::Free(s) => (s, s),
        Tangent::Broken(a, b) => (a, b),
        Tangent::Auto => {
            let (Some(_), Some(_)) = (before, after) else {
                // An end: level, as Unity's is, so a loop's seam rests.
                return (0.0, 0.0);
            };
            // A peak or a valley rests on it.
            if toward_before * toward_after <= 0.0 {
                return (0.0, 0.0);
            }
            let (b, a) = (before.unwrap(), after.unwrap());
            let s = slope(b, a);
            // Never steeper than three times the gentler side
            // (Fritsch–Carlson): the curve does not pass its neighbours.
            let most = 3.0 * toward_before.abs().min(toward_after.abs());
            let s = s.clamp(-most, most);
            (s, s)
        }
    }
}

/// Whether any key bends the curve: baked in samples, not straight.
pub fn curved(keys: &[Key], ease: Ease) -> bool {
    ease != Ease::Linear
        || keys
            .iter()
            .any(|k| !matches!(k.tangent, Tangent::Eased | Tangent::Linear))
}

/// The value of keys at a time, held past either end.
pub fn sample(keys: &[Key], ease: Ease, time: f32) -> Option<f32> {
    let first = keys.first()?;
    if time <= first.time {
        return Some(first.value);
    }
    let i = keys.partition_point(|k| k.time <= time);
    if i >= keys.len() {
        return keys.last().map(|k| k.value);
    }
    let (k0, k1) = (keys[i - 1], keys[i]);
    let span = k1.time - k0.time;
    if span <= 0.0 {
        return Some(k1.value);
    }
    if k0.tangent == Tangent::Step {
        return Some(k0.value);
    }
    let f = (time - k0.time) / span;
    if k0.tangent == Tangent::Eased && k1.tangent == Tangent::Eased {
        return Some(ease.lerp(k0.value, k1.value, f));
    }
    let out = slopes(keys, i - 1).1;
    let inn = slopes(keys, i).0;
    Some(hermite(k0.value, out * span, k1.value, inn * span, f))
}

/// The value of the last key at or before a time: a switch, which does not
/// pass through a half.
pub fn step(keys: &[Key], time: f32) -> Option<f32> {
    let first = keys.first()?;
    let i = keys.partition_point(|k| k.time <= time);
    Some(if i == 0 { first.value } else { keys[i - 1].value })
}

fn hermite(p0: f32, m0: f32, p1: f32, m1: f32, t: f32) -> f32 {
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * p0 + (t3 - 2.0 * t2 + t) * m0 + (-2.0 * t3 + 3.0 * t2) * p1 + (t3 - t2) * m1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(text: &str) -> Vec<Key> {
        ron::from_str(text).unwrap()
    }

    #[test]
    fn a_key_reads_and_writes_as_a_pair_or_with_its_shape() {
        let k = keys("[(0.0, 1.0), (0.5, 2.0, Auto), (1.0, 0.0, Broken(1.0, -2.0))]");
        assert_eq!(k[0], (0.0, 1.0));
        assert_eq!(k[1].tangent, Tangent::Auto);
        assert_eq!(k[2].tangent, Tangent::Broken(1.0, -2.0));
        let text = ron::to_string(&k).unwrap();
        assert_eq!(text, "[(0.0,1.0),(0.5,2.0,Auto),(1.0,0.0,Broken(1.0,-2.0))]");
    }

    #[test]
    fn plain_keys_are_straight_as_before() {
        let k = keys("[(0.0, 0.0), (1.0, 10.0)]");
        assert_eq!(sample(&k, Ease::Linear, 0.25), Some(2.5));
        assert_eq!(sample(&k, Ease::Linear, 2.0), Some(10.0));
        assert!(!curved(&k, Ease::Linear));
    }

    #[test]
    fn flat_keys_ease_in_and_out() {
        let k = keys("[(0.0, 0.0, Flat), (1.0, 10.0, Flat)]");
        assert_eq!(sample(&k, Ease::Linear, 0.5), Some(5.0));
        assert!(sample(&k, Ease::Linear, 0.1).unwrap() < 1.0, "slow out of the first");
        assert!(curved(&k, Ease::Linear));
    }

    #[test]
    fn auto_never_passes_its_neighbours() {
        // Two close keys and a far one: an unclamped spline swings past 10.
        let k = keys("[(0.0, 0.0, Auto), (0.1, 9.0, Auto), (0.2, 10.0, Auto), (1.0, 10.0, Auto)]");
        for i in 0..=100 {
            let v = sample(&k, Ease::Linear, i as f32 / 100.0).unwrap();
            assert!((-0.001..=10.001).contains(&v), "{v} at {i}");
        }
    }

    #[test]
    fn a_step_holds_until_the_next_key() {
        let k = keys("[(0.0, 1.0, Step), (1.0, 5.0)]");
        assert_eq!(sample(&k, Ease::Linear, 0.99), Some(1.0));
        assert_eq!(sample(&k, Ease::Linear, 1.0), Some(5.0));
    }
}
