//! The moving half of the simulation: an upright cylinder.

use runity_math::Vec3;

/// What a body is, for the few places where a settler and a dropped axe must
/// behave differently. Collision itself does not care.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BodyKind {
    /// A settler or the player: walks, climbs steps, jumps.
    #[default]
    Character,
    /// A thing lying around: pushed by the world, never asks to move.
    Item,
}

/// An upright cylinder — not a capsule.
///
/// A capsule buys a rounded foot, which matters when a body has to roll over a
/// kerb; nothing in the valley does. A cylinder's ground test is one height
/// query at its axis and its body-body test is a circle overlap in XZ, which
/// is the whole reason this file is short.
///
/// The position here is the **only** truth about where a body is:
/// [`crate::Transform`] is never written back to, so nothing can disagree
/// about it. To draw a body, ask [`Body::render_position`] with
/// [`crate::Time::fixed_alpha`] — the fixed tick is slower than the frame
/// rate, and interpolating between the last two ticks is what keeps motion
/// smooth without letting rendering touch the simulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    /// Centre of the base of the cylinder (the feet), in world space.
    pub position: Vec3,
    /// Where [`Body::position`] was at the end of the previous tick.
    pub prev_position: Vec3,
    /// Metres per second. `y` is the only component gravity touches.
    pub velocity: Vec3,
    /// Radius of the cylinder in the XZ plane.
    pub radius: f32,
    /// How tall the cylinder is, from `position` upwards.
    pub height: f32,
    /// Whether the last tick ended with the feet resting on something.
    pub grounded: bool,
    /// Set on the tick a jump left the ground, cleared on the next one — so a
    /// renderer or a sound can catch the take-off without polling.
    pub jumped: bool,
    pub kind: BodyKind,
}

impl Default for Body {
    fn default() -> Self {
        Self::new(Vec3::ZERO)
    }
}

impl Body {
    /// A character-sized body (0.3 m radius, 1.8 m tall) with its feet at
    /// `position`.
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            prev_position: position,
            velocity: Vec3::ZERO,
            radius: 0.3,
            height: 1.8,
            grounded: false,
            jumped: false,
            kind: BodyKind::Character,
        }
    }

    /// Same, sized and marked as a loose item.
    pub fn item(position: Vec3, radius: f32, height: f32) -> Self {
        Self {
            radius,
            height,
            kind: BodyKind::Item,
            ..Self::new(position)
        }
    }

    pub fn with_size(mut self, radius: f32, height: f32) -> Self {
        self.radius = radius;
        self.height = height;
        self
    }

    /// Where to draw this body, `alpha` of the way from the previous tick to
    /// the current one.
    ///
    /// This is the only supported way to read a body's position for drawing.
    /// It returns a value and mutates nothing: in particular it does not write
    /// to a [`crate::Transform`], because a second copy of a body's position
    /// is a second thing that can be wrong.
    #[inline]
    pub fn render_position(&self, alpha: f32) -> Vec3 {
        self.prev_position
            .lerp(self.position, alpha.clamp(0.0, 1.0))
    }

    /// Height of the top of the cylinder.
    #[inline]
    pub fn top(&self) -> f32 {
        self.position.y + self.height
    }

    /// Whether this body's vertical span overlaps `[bottom, top]`.
    #[inline]
    pub fn overlaps_span(&self, bottom: f32, top: f32) -> bool {
        self.position.y < top && bottom < self.top()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_position_interpolates_between_the_two_last_ticks() {
        let mut body = Body::new(Vec3::new(0.0, 0.0, 0.0));
        body.prev_position = Vec3::new(0.0, 0.0, 0.0);
        body.position = Vec3::new(2.0, 1.0, -4.0);
        assert_eq!(body.render_position(0.0), body.prev_position);
        assert_eq!(body.render_position(1.0), body.position);
        assert_eq!(body.render_position(0.5), Vec3::new(1.0, 0.5, -2.0));
    }

    #[test]
    fn render_position_clamps_an_alpha_outside_the_tick() {
        // A frame can arrive with a stale accumulator; extrapolating past the
        // tick would put a body somewhere the simulation never placed it.
        let mut body = Body::new(Vec3::ZERO);
        body.position = Vec3::new(10.0, 0.0, 0.0);
        assert_eq!(body.render_position(4.0), body.position);
        assert_eq!(body.render_position(-1.0), body.prev_position);
    }

    #[test]
    fn render_position_does_not_mutate_the_body() {
        let mut body = Body::new(Vec3::ZERO);
        body.position = Vec3::new(1.0, 2.0, 3.0);
        let before = body;
        let _ = body.render_position(0.37);
        assert_eq!(body, before);
    }

    #[test]
    fn spans_overlap_only_when_they_really_do() {
        let body = Body::new(Vec3::new(0.0, 1.0, 0.0));
        assert!(body.overlaps_span(0.0, 1.5), "the feet are inside");
        assert!(body.overlaps_span(2.0, 4.0), "the head is inside");
        assert!(!body.overlaps_span(3.0, 4.0), "entirely above the head");
        assert!(!body.overlaps_span(-2.0, 0.5), "entirely below the feet");
    }
}
