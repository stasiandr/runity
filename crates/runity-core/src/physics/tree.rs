//! A tree coming down: a pendulum hinged at its stump.

use runity_math::Vec2;

/// A quarter turn, in radians — a trunk's whole journey from upright to flat.
///
/// Spelled as a literal rather than `90f32.to_radians()` on purpose: see the
/// module documentation for why nothing here is allowed to call a
/// transcendental function.
pub const QUARTER_TURN: f32 = std::f32::consts::FRAC_PI_2;

/// A tree that can be felled, and the three numbers describing the fall.
///
/// This is not a rigid body and there is no solver. A trunk hinged at its
/// stump under a constant torque is one scalar integration — lean, angular
/// velocity, and the compass direction of the last axe blow — and a trunk
/// coming down is visually indistinguishable from the real thing, which is
/// all the valley needs.
///
/// The lean is kept as a **unit vector, not an angle**, for the same reason
/// [`super::Tuning::max_slope_cos`] is a cosine: `lean.x` is the sine of the
/// angle from upright and `lean.y` its cosine, so both the trunk's reach
/// (`height * lean.x`) and the matrix a renderer needs are already there, and
/// nothing has to call `sin` to get them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FallingTree {
    /// `(sin, cos)` of the angle from upright, as a unit vector.
    pub lean: Vec2,
    /// Radians per second, growing while the trunk falls.
    pub angular_velocity: f32,
    /// Horizontal direction the trunk is falling towards — the direction of
    /// the blow that started it. Unit length.
    pub direction: Vec2,
    /// Length of the trunk, in metres.
    pub height: f32,
    pub trunk_radius: f32,
    /// True between [`FallingTree::topple`] and the trunk touching the ground.
    pub falling: bool,
    /// True once it has touched the ground and become a log.
    pub fallen: bool,
}

impl FallingTree {
    /// A tree standing upright, waiting for an axe.
    pub fn new(height: f32, trunk_radius: f32) -> Self {
        Self {
            lean: Vec2::new(0.0, 1.0),
            angular_velocity: 0.0,
            direction: Vec2::new(1.0, 0.0),
            height,
            trunk_radius,
            falling: false,
            fallen: false,
        }
    }

    /// Start the fall, away from the blow. A zero or already-fallen tree does
    /// nothing.
    pub fn topple(&mut self, direction: Vec2) {
        if self.fallen || self.falling {
            return;
        }
        let direction = direction.normalized();
        if direction == Vec2::ZERO {
            return;
        }
        self.direction = direction;
        self.falling = true;
    }

    /// How far the tip reaches from the stump, horizontally.
    #[inline]
    pub fn reach(&self) -> f32 {
        self.height * self.lean.x
    }

    /// How high the tip is above the stump.
    #[inline]
    pub fn tip_height(&self) -> f32 {
        self.height * self.lean.y
    }

    /// Swing by one tick under a constant torque, returning `true` on the tick
    /// the trunk reaches the ground.
    ///
    /// The lean is advanced along the circle's tangent and renormalized, which
    /// keeps the trunk exactly `height` long without a single call to `sin`.
    /// Over the thirty ticks a fall lasts the difference from the exact arc is
    /// under a hundredth of a radian, which no one has ever seen in a tree.
    pub fn advance(&mut self, dt: f32, torque: f32) -> bool {
        if !self.falling {
            return false;
        }
        self.angular_velocity += torque * dt;
        let delta = self.angular_velocity * dt;
        self.lean = Vec2::new(
            self.lean.x + self.lean.y * delta,
            self.lean.y - self.lean.x * delta,
        )
        .normalized();
        if self.lean.y <= 0.0 {
            self.lean = Vec2::new(1.0, 0.0);
            self.falling = false;
            self.fallen = true;
            return true;
        }
        false
    }

    /// Whether a circle of `radius` at `point` is inside the arc the trunk has
    /// already swung through — the part of the ground it is about to occupy.
    ///
    /// `stump` is the trunk's base in XZ. The near edge is the stump itself
    /// and the far edge is the tip's current reach, so a body is shoved from
    /// the moment the swinging trunk covers it until either the trunk lands or
    /// the body has been pushed clear.
    pub fn arc_covers(&self, stump: Vec2, point: Vec2, radius: f32) -> bool {
        if !self.falling {
            return false;
        }
        let delta = point - stump;
        let along = delta.dot(self.direction);
        if along < 0.0 || along > self.reach() {
            return false;
        }
        delta.cross(self.direction).abs() <= self.trunk_radius + radius
    }

    /// The log this trunk becomes: half-extents along and across the trunk,
    /// and the centre offset from the stump.
    pub fn log_footprint(&self) -> (Vec2, Vec2) {
        (
            Vec2::new(self.height * 0.5, self.trunk_radius),
            self.direction * (self.height * 0.5),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::Tuning;

    fn tuning() -> Tuning {
        Tuning::default()
    }

    #[test]
    fn a_standing_tree_does_not_move() {
        let mut tree = FallingTree::new(6.0, 0.35);
        for _ in 0..100 {
            assert!(!tree.advance(1.0 / 20.0, tuning().tree_torque));
        }
        assert_eq!(tree.lean, Vec2::new(0.0, 1.0));
        assert_eq!(tree.reach(), 0.0);
    }

    #[test]
    fn a_fall_takes_about_thirty_ticks_at_twenty_hertz() {
        let mut tree = FallingTree::new(6.0, 0.35);
        tree.topple(Vec2::new(0.0, 1.0));
        let mut ticks = 0;
        while !tree.fallen {
            tree.advance(1.0 / 20.0, tuning().tree_torque);
            ticks += 1;
            assert!(ticks < 100, "the trunk must come down");
        }
        assert_eq!(ticks, 30, "a second and a half at 20 Hz");
        assert!(!tree.falling);
        assert_eq!(tree.lean, Vec2::new(1.0, 0.0), "flat on the ground");
    }

    #[test]
    fn the_lean_stays_a_unit_vector_and_the_trunk_keeps_its_length() {
        let mut tree = FallingTree::new(6.0, 0.35);
        tree.topple(Vec2::new(1.0, 0.0));
        while !tree.fallen {
            tree.advance(1.0 / 20.0, tuning().tree_torque);
            assert!(
                (tree.lean.length() - 1.0).abs() < 1e-5,
                "{:?}",
                tree.lean.length()
            );
            let reach = tree.reach();
            let up = tree.tip_height();
            assert!(
                ((reach * reach + up * up).sqrt() - 6.0).abs() < 1e-3,
                "trunk length drifted: {reach}, {up}"
            );
        }
    }

    // The check that the tangent step really does track the arc lives in
    // `tests/physics_trigonometry.rs`: it is the one test that has to call
    // `sin` to have anything to compare against, and no file under
    // `physics/` — test module included — is allowed to.

    #[test]
    fn the_reach_only_grows_while_the_trunk_comes_down() {
        let mut tree = FallingTree::new(6.0, 0.35);
        tree.topple(Vec2::new(0.0, 1.0));
        let mut previous = 0.0;
        while !tree.fallen {
            tree.advance(1.0 / 20.0, tuning().tree_torque);
            let reach = tree.reach();
            assert!(reach >= previous, "{reach} after {previous}");
            previous = reach;
        }
        assert!((tree.reach() - 6.0).abs() < 1e-4, "{}", tree.reach());
    }

    #[test]
    fn the_arc_is_the_swath_the_trunk_has_swept_so_far() {
        let mut tree = FallingTree::new(6.0, 0.35);
        tree.topple(Vec2::new(1.0, 0.0));
        for _ in 0..15 {
            tree.advance(1.0 / 20.0, tuning().tree_torque);
        }
        let reach = tree.reach();
        let stump = Vec2::ZERO;
        assert!(tree.arc_covers(stump, Vec2::new(reach - 0.5, 0.0), 0.3));
        assert!(
            !tree.arc_covers(stump, Vec2::new(reach + 1.0, 0.0), 0.3),
            "the trunk has not got there yet"
        );
        assert!(
            !tree.arc_covers(stump, Vec2::new(reach * 0.5, 2.0), 0.3),
            "well to the side of the swath"
        );
        assert!(
            !tree.arc_covers(stump, Vec2::new(-1.0, 0.0), 0.3),
            "behind the stump"
        );
    }

    #[test]
    fn nothing_is_in_the_arc_of_a_tree_that_is_not_falling() {
        let tree = FallingTree::new(6.0, 0.35);
        assert!(!tree.arc_covers(Vec2::ZERO, Vec2::new(1.0, 0.0), 0.3));
    }

    #[test]
    fn a_fallen_tree_cannot_be_toppled_again() {
        let mut tree = FallingTree::new(6.0, 0.35);
        tree.topple(Vec2::new(1.0, 0.0));
        while !tree.fallen {
            tree.advance(1.0 / 20.0, tuning().tree_torque);
        }
        tree.topple(Vec2::new(0.0, 1.0));
        assert!(!tree.falling);
        assert_eq!(tree.direction, Vec2::new(1.0, 0.0), "it fell where it fell");
    }

    #[test]
    fn a_blow_with_no_direction_fells_nothing() {
        let mut tree = FallingTree::new(6.0, 0.35);
        tree.topple(Vec2::ZERO);
        assert!(!tree.falling);
    }

    #[test]
    fn the_log_lies_between_the_stump_and_where_the_tip_landed() {
        let mut tree = FallingTree::new(6.0, 0.35);
        tree.topple(Vec2::new(0.0, 1.0));
        let (half, offset) = tree.log_footprint();
        assert_eq!(half, Vec2::new(3.0, 0.35));
        assert_eq!(offset, Vec2::new(0.0, 3.0));
    }
}
