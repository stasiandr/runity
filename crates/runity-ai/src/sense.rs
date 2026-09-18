//! What an agent can notice.
//!
//! Perception is deliberately separate from the decision layer. An agent that
//! scores its options from the true state of the world is omniscient, and
//! omniscient agents feel unfair in a way players notice immediately without
//! being able to say why: the monster turned around before it could have seen
//! you. Scoring from what the agent has *perceived* fixes that, and costs one
//! struct.

use runity_math::Vec3;

use crate::spatial::ground_distance;

/// How far and how wide an agent perceives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Senses {
    /// How far it can see.
    pub sight: f32,
    /// Total field of view in radians — the full cone, not the half-angle.
    pub field_of_view: f32,
    /// How far an ordinary sound carries to it.
    pub hearing: f32,
}

impl Default for Senses {
    fn default() -> Self {
        Self::person()
    }
}

impl Senses {
    /// Roughly human: a wide but not complete view, and ears that work behind
    /// the head.
    pub fn person() -> Self {
        Self {
            sight: 30.0,
            field_of_view: 2.0,
            hearing: 20.0,
        }
    }

    /// Something that hunts: further, narrower, and much better hearing.
    pub fn predator() -> Self {
        Self {
            sight: 45.0,
            field_of_view: 1.6,
            hearing: 60.0,
        }
    }

    /// How clearly the target is seen, from 0 (not at all) to 1.
    ///
    /// Falls off with distance and toward the edge of the cone, so an agent
    /// notices what is in front of it long before what is nearly behind it.
    /// Occlusion is not considered here — pass the result through
    /// [`Senses::sees`] with a line-of-sight test for that.
    pub fn visibility(&self, observer: Vec3, facing: Vec3, target: Vec3) -> f32 {
        let distance = ground_distance(observer, target);
        if distance > self.sight || self.sight <= 0.0 {
            return 0.0;
        }
        let offset = Vec3 {
            x: target.x - observer.x,
            y: 0.0,
            z: target.z - observer.z,
        };
        if offset.length_squared() < 1e-8 {
            return 1.0; // standing on top of the observer
        }
        let facing = Vec3 {
            x: facing.x,
            y: 0.0,
            z: facing.z,
        };
        if facing.length_squared() < 1e-8 {
            return 0.0; // facing nowhere sees nothing
        }
        let cosine = facing.normalized().dot(offset.normalized());
        let half = (self.field_of_view * 0.5).clamp(0.0, core::f32::consts::PI);
        if cosine < half.cos() {
            return 0.0;
        }
        // Inside the cone: fade toward its edge and with distance.
        let edge = if half.cos() >= 1.0 {
            1.0
        } else {
            ((cosine - half.cos()) / (1.0 - half.cos())).clamp(0.0, 1.0)
        };
        let range = 1.0 - (distance / self.sight).clamp(0.0, 1.0);
        (edge.sqrt() * range.max(0.05)).clamp(0.0, 1.0)
    }

    /// Whether the target is visible at all, given a line-of-sight test.
    ///
    /// The test is supplied rather than assumed: it might consult a
    /// [`NavGrid`](crate::NavGrid), a physics ray cast, or nothing at all in
    /// an open field.
    pub fn sees(
        &self,
        observer: Vec3,
        facing: Vec3,
        target: Vec3,
        clear: impl Fn(Vec3, Vec3) -> bool,
    ) -> bool {
        self.visibility(observer, facing, target) > 0.0 && clear(observer, target)
    }

    /// Whether a sound of a given loudness reaches the observer.
    ///
    /// `loudness` scales the range: 1 is a person walking, 3 an axe on a tree,
    /// 0.3 someone trying to be quiet. Hearing ignores facing, which is the
    /// entire point of having it as well as sight.
    pub fn hears(&self, observer: Vec3, source: Vec3, loudness: f32) -> bool {
        let range = self.hearing * loudness.max(0.0);
        ground_distance(observer, source) <= range
    }
}

/// How sure an agent is that something is there.
///
/// Sight is momentary; conviction is not. Building awareness over time gives a
/// guard who glances, hesitates and then reacts — and, more usefully, it gives
/// the player a window in which to break the line of sight again.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Awareness {
    level: f32,
}

impl Awareness {
    /// Unaware.
    pub fn new() -> Self {
        Self { level: 0.0 }
    }

    /// Certainty so far, from 0 to 1.
    pub fn level(&self) -> f32 {
        self.level
    }

    /// Whether the agent is convinced enough to act.
    pub fn alerted(&self, threshold: f32) -> bool {
        self.level >= threshold
    }

    /// Feed one tick of evidence.
    ///
    /// `visibility` is what [`Senses::visibility`] returned (zero when the
    /// target cannot be seen). Rising faster than it falls is deliberate:
    /// something half-seen twice should add up, while suspicion that fades as
    /// fast as it grows never produces a reaction at all.
    pub fn update(&mut self, visibility: f32, dt: f32, rise: f32, fall: f32) -> f32 {
        if !dt.is_finite() || dt <= 0.0 {
            return self.level;
        }
        if visibility > 0.0 {
            self.level += visibility * rise * dt;
        } else {
            self.level -= fall * dt;
        }
        self.level = self.level.clamp(0.0, 1.0);
        self.level
    }

    /// Forget everything.
    pub fn reset(&mut self) {
        self.level = 0.0;
    }

    /// Become certain at once — for a shout, a shove, or an arrow.
    pub fn alarm(&mut self) {
        self.level = 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::vec3;

    const NORTH: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: -1.0,
    };

    #[test]
    fn what_is_in_front_is_seen_and_what_is_behind_is_not() {
        let senses = Senses::person();
        let eye = Vec3::ZERO;
        assert!(senses.visibility(eye, NORTH, vec3(0.0, 0.0, -5.0)) > 0.0);
        assert_eq!(
            senses.visibility(eye, NORTH, vec3(0.0, 0.0, 5.0)),
            0.0,
            "behind"
        );
        assert_eq!(
            senses.visibility(eye, NORTH, vec3(0.0, 0.0, -500.0)),
            0.0,
            "out of range"
        );
    }

    #[test]
    fn clarity_falls_off_with_distance_and_toward_the_edge_of_the_cone() {
        let senses = Senses::person();
        let eye = Vec3::ZERO;
        let near = senses.visibility(eye, NORTH, vec3(0.0, 0.0, -5.0));
        let far = senses.visibility(eye, NORTH, vec3(0.0, 0.0, -25.0));
        assert!(near > far, "{near} should beat {far}");

        let centred = senses.visibility(eye, NORTH, vec3(0.0, 0.0, -10.0));
        let edge = senses.visibility(eye, NORTH, vec3(7.0, 0.0, -10.0));
        assert!(centred > edge, "dead ahead beats the corner of the eye");
        assert!(edge > 0.0, "but the corner of the eye still works");
    }

    #[test]
    fn a_wall_hides_what_would_otherwise_be_in_plain_sight() {
        let senses = Senses::person();
        let eye = Vec3::ZERO;
        let target = vec3(0.0, 0.0, -8.0);
        assert!(senses.sees(eye, NORTH, target, |_, _| true));
        assert!(!senses.sees(eye, NORTH, target, |_, _| false));
    }

    #[test]
    fn a_predator_sees_further_and_narrower_than_a_person() {
        let (person, predator) = (Senses::person(), Senses::predator());
        let eye = Vec3::ZERO;
        let far = vec3(0.0, 0.0, -40.0);
        assert_eq!(person.visibility(eye, NORTH, far), 0.0);
        assert!(predator.visibility(eye, NORTH, far) > 0.0);

        // Wide to the side: the person notices, the predator does not.
        let aside = vec3(14.0, 0.0, -10.0);
        assert!(person.visibility(eye, NORTH, aside) > 0.0);
        assert_eq!(predator.visibility(eye, NORTH, aside), 0.0);
    }

    #[test]
    fn hearing_ignores_which_way_you_are_facing() {
        let senses = Senses::person();
        let eye = Vec3::ZERO;
        let behind = vec3(0.0, 0.0, 15.0);
        assert_eq!(senses.visibility(eye, NORTH, behind), 0.0);
        assert!(senses.hears(eye, behind, 1.0), "ears work backwards");

        assert!(
            !senses.hears(eye, vec3(0.0, 0.0, 25.0), 1.0),
            "but not forever"
        );
        assert!(
            senses.hears(eye, vec3(0.0, 0.0, 25.0), 3.0),
            "unless it is loud"
        );
        assert!(
            !senses.hears(eye, vec3(0.0, 0.0, 5.0), 0.0),
            "silence carries nowhere"
        );
    }

    #[test]
    fn degenerate_observers_perceive_nothing_rather_than_panicking() {
        let senses = Senses::person();
        assert_eq!(
            senses.visibility(Vec3::ZERO, Vec3::ZERO, vec3(0.0, 0.0, -5.0)),
            0.0
        );
        assert_eq!(
            senses.visibility(Vec3::ZERO, NORTH, Vec3::ZERO),
            1.0,
            "right on top of us"
        );
        let blind = Senses {
            sight: 0.0,
            ..Senses::person()
        };
        assert_eq!(
            blind.visibility(Vec3::ZERO, NORTH, vec3(0.0, 0.0, -1.0)),
            0.0
        );
    }

    #[test]
    fn awareness_builds_while_seen_and_fades_when_not() {
        let mut awareness = Awareness::new();
        assert_eq!(awareness.level(), 0.0);
        assert!(!awareness.alerted(0.5));

        // Half-visible for a second, at a rise of one per second.
        for _ in 0..10 {
            awareness.update(0.5, 0.1, 1.0, 0.5);
        }
        let after_a_second = awareness.level();
        assert!((after_a_second - 0.5).abs() < 1e-4, "{after_a_second}");

        // Out of sight, and it starts to drain.
        for _ in 0..5 {
            awareness.update(0.0, 0.1, 1.0, 0.5);
        }
        assert!(awareness.level() < after_a_second);
        assert!(awareness.level() > 0.0, "but not instantly forgotten");
    }

    #[test]
    fn awareness_is_bounded_and_can_be_forced_either_way() {
        let mut awareness = Awareness::new();
        for _ in 0..100 {
            awareness.update(1.0, 0.1, 5.0, 1.0);
        }
        assert_eq!(awareness.level(), 1.0, "certainty does not overflow");
        assert!(awareness.alerted(1.0));

        for _ in 0..100 {
            awareness.update(0.0, 0.1, 5.0, 1.0);
        }
        assert_eq!(awareness.level(), 0.0, "and does not go negative");

        awareness.alarm();
        assert_eq!(awareness.level(), 1.0);
        awareness.reset();
        assert_eq!(awareness.level(), 0.0);
    }

    #[test]
    fn a_glimpse_is_not_enough_but_two_glimpses_are() {
        // The property that makes this worth having: evidence adds up over
        // time, so brief exposure is survivable and repeated exposure is not.
        let mut awareness = Awareness::new();
        let glimpse = |awareness: &mut Awareness| {
            for _ in 0..3 {
                awareness.update(0.8, 0.1, 1.5, 0.4);
            }
            for _ in 0..2 {
                awareness.update(0.0, 0.1, 1.5, 0.4);
            }
        };

        glimpse(&mut awareness);
        assert!(
            !awareness.alerted(0.5),
            "one glance is not proof: {}",
            awareness.level()
        );
        glimpse(&mut awareness);
        assert!(awareness.alerted(0.5), "two is: {}", awareness.level());
    }

    #[test]
    fn an_absurd_time_step_changes_nothing() {
        let mut awareness = Awareness::new();
        awareness.update(1.0, 0.5, 1.0, 1.0);
        let level = awareness.level();
        for dt in [0.0, -1.0, f32::NAN] {
            assert_eq!(awareness.update(1.0, dt, 1.0, 1.0), level);
        }
    }
}
