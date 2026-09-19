//! The two fixed things a settler's needs are relieved against: a hearth's
//! warmth and a sleeping spot's shelter.
//!
//! Neither has a position of its own — like [`crate::physics::Blocker`], each
//! sits at the [`crate::Transform`] of the entity carrying it.

use runity_math::Vec3;

/// A source of warmth. A point within [`Hearth::radius`] of the entity's
/// [`crate::Transform`] is warmed by it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hearth {
    pub radius: f32,
}

impl Hearth {
    pub fn new(radius: f32) -> Self {
        Self { radius }
    }

    /// Whether `point` falls inside this hearth's radius, given the hearth
    /// sits at `hearth_position`.
    #[inline]
    pub fn warms(&self, hearth_position: Vec3, point: Vec3) -> bool {
        (point - hearth_position).length_squared() <= self.radius * self.radius
    }
}

/// A proper place to sleep. A point within [`SleepingSpot::radius`] of the
/// entity's [`crate::Transform`] is sheltered by it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SleepingSpot {
    pub radius: f32,
}

impl SleepingSpot {
    pub fn new(radius: f32) -> Self {
        Self { radius }
    }

    /// Whether `point` falls inside this spot's radius, given the spot sits
    /// at `spot_position`.
    #[inline]
    pub fn shelters(&self, spot_position: Vec3, point: Vec3) -> bool {
        (point - spot_position).length_squared() <= self.radius * self.radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hearth_warms_only_within_its_radius() {
        let hearth = Hearth::new(2.0);
        let at = Vec3::new(5.0, 0.0, 0.0);
        assert!(
            hearth.warms(at, Vec3::new(6.0, 0.0, 0.0)),
            "1 m away, inside"
        );
        assert!(
            hearth.warms(at, Vec3::new(7.0, 0.0, 0.0)),
            "exactly at the radius"
        );
        assert!(
            !hearth.warms(at, Vec3::new(7.1, 0.0, 0.0)),
            "just past the radius"
        );
    }

    #[test]
    fn a_sleeping_spot_shelters_only_within_its_radius() {
        let spot = SleepingSpot::new(1.0);
        let at = Vec3::ZERO;
        assert!(spot.shelters(at, Vec3::new(0.5, 0.0, 0.5)));
        assert!(!spot.shelters(at, Vec3::new(1.0, 0.0, 1.0)));
    }
}
