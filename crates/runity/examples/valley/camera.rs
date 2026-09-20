//! The player's head — where the first-person camera is, frame by frame.
//!
//! Not physics, and deliberately not near it: everything here runs once per
//! *frame*, off [`runity_core::physics::Body::render_position`] and the same
//! `alpha` the rest of the world is drawn at, and it is free to call `sin` as
//! often as it likes. The simulation never reads a single value from this
//! file, so nothing here can change where a body ends up.
//!
//! Four things happen to the eye, and they are separable on purpose — each one
//! answers a different complaint about a first-person camera bolted straight
//! onto a capsule (`docs/design/04-player.md`, the camera section):
//!
//! 1. **The eye lags the feet.** A body steps onto a log inside a single tick,
//!    with no airborne phase — `physics::step` lifts it by up to
//!    `Tuning::step_height` and is done. Drawn literally that is a 0.4 m
//!    teleport upwards. An exponential filter spreads it over
//!    [`EYE_SETTLE_TIME`] instead: visible as a rise, never as a jump.
//! 2. **The walk bobs** — by distance travelled, not by time. A shortened,
//!    tired step then changes the rhythm on its own, because the rhythm was
//!    never a clock in the first place.
//! 3. **A landing dips**, deeper the further the fall, and comes back over
//!    [`LANDING_TIME`].
//! 4. **The head turns** with the mouse, every frame, at whatever rate frames
//!    happen to arrive — the one part of the game that must not wait for a
//!    tick.

use runity::prelude::*;
use std::f32::consts::{PI, TAU};

/// How high the eye sits above the feet of a 1.8 m body.
pub const EYE_HEIGHT: f32 = 1.62;

/// How long the eye takes to absorb a sudden change of foot height.
///
/// 0.15 s for the 0.4 m a body steps up in one tick: slow enough to read as a
/// climb, fast enough that the world does not feel rubbery afterwards.
pub const EYE_SETTLE_TIME: f32 = 0.15;

/// Time constant of the eye filter. An exponential is 95% done after three of
/// them, so this is [`EYE_SETTLE_TIME`] divided by three.
const EYE_TAU: f32 = EYE_SETTLE_TIME / 3.0;

/// One pace, in metres — the period of the bob.
const STRIDE: f32 = 0.9;
/// How far the eye rises and falls over a pace, at a full run.
const BOB_HEIGHT: f32 = 0.022;
/// How far it leans side to side, over two paces.
const BOB_SWAY: f32 = 0.015;
/// How quickly the bob fades in when a walk starts and out when it stops.
/// Without it, stopping mid-pace would leave the head parked off-centre.
const BOB_FADE_TAU: f32 = 0.12;
/// The speed the bob reaches full strength at — the tuning's `max_speed`.
const BOB_FULL_SPEED: f32 = 3.4;

/// How deep the eye dips after a fall of [`LANDING_REFERENCE_FALL`].
const LANDING_DIP: f32 = 0.08;
/// The fall [`LANDING_DIP`] is quoted for: the height of the valley's own jump.
const LANDING_REFERENCE_FALL: f32 = 0.6;
/// However far the fall, the knees only give so much.
const LANDING_DIP_MAX: f32 = 0.18;
/// Down and back up again, in seconds.
const LANDING_TIME: f32 = 0.2;
/// A drop smaller than this is a footstep, not a landing.
const LANDING_MIN_FALL: f32 = 0.05;

/// Radians of turn per pixel of mouse movement.
pub const MOUSE_SENSITIVITY: f32 = 0.0022;

/// Just short of straight up or straight down, so the view never flips.
const PITCH_LIMIT: f32 = 1.48;

/// How far the eye dips on landing after falling `fall` metres.
///
/// Proportional to the fall and capped: a fall from the sky bends the knees
/// exactly as far as a fall from a roof, because a head is not a spring.
pub fn landing_depth(fall: f32) -> f32 {
    if fall < LANDING_MIN_FALL {
        return 0.0;
    }
    (LANDING_DIP * fall / LANDING_REFERENCE_FALL).min(LANDING_DIP_MAX)
}

/// The fraction of a full exponential step to take in `dt` with time constant
/// `tau` — frame-rate independent, unlike a plain `lerp(a, b, 0.1)`.
fn settle(dt: f32, tau: f32) -> f32 {
    1.0 - (-dt / tau).exp()
}

/// The player's head: yaw, pitch, and everything the eye does that the feet
/// do not.
#[derive(Debug, Clone)]
pub struct Head {
    /// Compass heading, radians. Zero looks along `-Z`, and grows clockwise
    /// seen from above — the direction the mouse moves right.
    pub yaw: f32,
    /// Up (positive) and down, radians, clamped short of the poles.
    pub pitch: f32,
    /// Where the eye actually is vertically, chasing `feet.y + EYE_HEIGHT`.
    settled_eye: f32,
    /// Horizontal metres walked since the head was made — the bob's clock.
    travelled: f32,
    /// How much of the bob is showing, 0 standing to 1 at a full run.
    bob: f32,
    /// Highest the feet have been since they last left the ground.
    airborne_peak: f32,
    was_grounded: bool,
    /// Depth of the landing the head is still recovering from.
    crouch_depth: f32,
    /// Seconds since that landing; at or past [`LANDING_TIME`] means done.
    crouch_age: f32,
    /// Where the feet were last frame.
    last_feet: Vec2,
    /// The eye, as of the last [`Head::follow`].
    eye: Vec3,
}

impl Head {
    /// A head on a body standing at `feet`, looking along `yaw`.
    pub fn new(feet: Vec3, yaw: f32) -> Self {
        let settled_eye = feet.y + EYE_HEIGHT;
        Self {
            yaw,
            pitch: 0.0,
            settled_eye,
            travelled: 0.0,
            bob: 0.0,
            airborne_peak: feet.y,
            was_grounded: true,
            crouch_depth: 0.0,
            crouch_age: LANDING_TIME,
            last_feet: Vec2::new(feet.x, feet.z),
            eye: Vec3::new(feet.x, settled_eye, feet.z),
        }
    }

    /// Turn by a mouse movement in pixels. Called every frame, never on a
    /// tick: a head that only turned 20 times a second would feel broken long
    /// before anyone could say why.
    pub fn turn(&mut self, delta: Vec2) {
        self.yaw += delta.x * MOUSE_SENSITIVITY;
        // Screen `y` grows downwards, and pushing the mouse forward looks up.
        self.pitch = (self.pitch - delta.y * MOUSE_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// Follow the body for one frame.
    ///
    /// `feet` is `body.render_position(alpha)` — the interpolated position,
    /// not the tick's own, so the head travels the same smooth path as
    /// everything else drawn this frame.
    pub fn follow(&mut self, feet: Vec3, grounded: bool, dt: f32) {
        let dt = dt.max(0.0);
        let here = Vec2::new(feet.x, feet.z);
        let stepped = (here - self.last_feet).length();
        self.last_feet = here;
        self.travelled += stepped;

        // The bob's strength follows speed, so standing still is perfectly
        // still — the phase stays where it was and simply stops advancing.
        if dt > 0.0 {
            let wanted = (stepped / dt / BOB_FULL_SPEED).clamp(0.0, 1.0);
            self.bob += (wanted - self.bob) * settle(dt, BOB_FADE_TAU);
        }

        if grounded && !self.was_grounded {
            let fall = (self.airborne_peak - feet.y).max(0.0);
            let depth = landing_depth(fall);
            if depth > 0.0 {
                self.crouch_depth = depth;
                self.crouch_age = 0.0;
            }
        }
        self.airborne_peak = if grounded {
            feet.y
        } else {
            self.airborne_peak.max(feet.y)
        };
        self.was_grounded = grounded;
        self.crouch_age = (self.crouch_age + dt).min(LANDING_TIME);

        self.settled_eye += (feet.y + EYE_HEIGHT - self.settled_eye) * settle(dt, EYE_TAU);

        let right = self.right();
        let sway = BOB_SWAY * self.bob * (self.travelled * PI / STRIDE).sin();
        self.eye = Vec3::new(
            feet.x + right.x * sway,
            self.settled_eye + self.bob_height() + self.crouch_offset(),
            feet.z + right.z * sway,
        );
    }

    /// The vertical part of the walk's bob — a full cycle per pace, and a pace
    /// is a distance, so a shorter tired step shortens the rhythm with it.
    fn bob_height(&self) -> f32 {
        BOB_HEIGHT * self.bob * (self.travelled * TAU / STRIDE).sin()
    }

    /// Down and back over [`LANDING_TIME`]: a half sine, which starts and ends
    /// at rest with no corner at either end.
    fn crouch_offset(&self) -> f32 {
        if self.crouch_age >= LANDING_TIME {
            return 0.0;
        }
        -self.crouch_depth * (self.crouch_age * PI / LANDING_TIME).sin()
    }

    /// Where the eye is, as of the last [`Head::follow`].
    pub fn eye(&self) -> Vec3 {
        self.eye
    }

    /// Unit vector the head is looking along.
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cp * sy, sp, -cp * cy)
    }

    /// Unit vector along the ground, straight ahead — what `W` means.
    pub fn ahead(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(sy, 0.0, -cy)
    }

    /// Unit vector along the ground, to the right — what `D` means.
    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, 0.0, sy)
    }

    /// Point a camera where this head is looking.
    pub fn aim(&self, camera: &mut Camera) {
        camera.position = self.eye;
        camera.target = self.eye + self.forward();
        camera.up = Vec3::Y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

    /// Hold a body still at `feet` for `seconds`, returning the head.
    fn stand(head: &mut Head, feet: Vec3, seconds: f32) {
        let frames = (seconds / FRAME).round() as usize;
        for _ in 0..frames {
            head.follow(feet, true, FRAME);
        }
    }

    #[test]
    fn a_head_that_is_not_moving_does_not_move() {
        // The first thing anyone notices in a camera like this is a tremble on
        // flat ground that has no business being there.
        let mut head = Head::new(Vec3::ZERO, 0.0);
        let resting = head.eye();
        stand(&mut head, Vec3::ZERO, 2.0);
        assert!(
            (head.eye() - resting).length() < 1e-4,
            "{:?} drifted from {resting:?}",
            head.eye()
        );
    }

    #[test]
    fn stepping_up_four_tenths_of_a_metre_is_a_rise_and_not_a_jump() {
        // `physics::step` lifts a body onto a log inside one tick, with no
        // airborne phase at all: drawn literally, that is a teleport.
        let mut head = Head::new(Vec3::ZERO, 0.0);
        let resting = head.eye().y;
        let stepped = Vec3::new(0.0, 0.4, 0.0);

        head.follow(stepped, true, FRAME);
        let after_one_frame = (head.eye().y - resting) / 0.4;
        assert!(
            after_one_frame < 0.4,
            "the eye snapped {after_one_frame} of the way up in a single frame"
        );

        // EYE_SETTLE_TIME after the step, all but a few percent is done.
        let frames = (EYE_SETTLE_TIME / FRAME).round() as usize;
        for _ in 1..frames {
            head.follow(stepped, true, FRAME);
        }
        let covered = (head.eye().y - resting) / 0.4;
        assert!(
            (0.9..=1.0).contains(&covered),
            "{covered} of the step covered after {EYE_SETTLE_TIME} s"
        );
    }

    #[test]
    fn stepping_down_is_absorbed_the_same_way() {
        let mut head = Head::new(Vec3::new(0.0, 0.4, 0.0), 0.0);
        let resting = head.eye().y;
        let dropped = Vec3::ZERO;
        head.follow(dropped, true, FRAME);
        assert!(resting - head.eye().y < 0.4 * 0.4, "not a drop, a settle");
        let frames = (EYE_SETTLE_TIME / FRAME).round() as usize;
        for _ in 1..frames {
            head.follow(dropped, true, FRAME);
        }
        let covered = (resting - head.eye().y) / 0.4;
        assert!((0.9..=1.0).contains(&covered), "{covered}");
    }

    /// Walk `distance` metres along +X at `speed`, returning the eye's height
    /// above the resting height at the end.
    fn walk(distance: f32, speed: f32) -> (Head, f32) {
        let mut head = Head::new(Vec3::ZERO, 0.0);
        let resting = head.eye().y;
        let frames = (distance / speed / FRAME).round() as usize;
        for frame in 1..=frames {
            let x = speed * frame as f32 * FRAME;
            head.follow(Vec3::new(x, 0.0, 0.0), true, FRAME);
        }
        let offset = head.eye().y - resting;
        (head, offset)
    }

    #[test]
    fn the_bob_is_driven_by_distance_walked_and_not_by_the_clock() {
        // The same 6.8 m, once at a run and once at half that speed over twice
        // as long. Distance decides the phase, so both end on the same side of
        // the pace; only the strength differs, and it differs by the speed.
        let (fast, fast_offset) = walk(6.8, 3.4);
        let (slow, slow_offset) = walk(6.8, 1.7);
        assert!(
            (fast.travelled - slow.travelled).abs() < 1e-3,
            "{} vs {}",
            fast.travelled,
            slow.travelled
        );
        assert!(
            fast_offset.signum() == slow_offset.signum(),
            "same distance, opposite phase: {fast_offset} vs {slow_offset}"
        );
        let ratio = fast_offset / slow_offset;
        assert!(
            (1.7..2.3).contains(&ratio),
            "half the speed should bob about half as far, got {ratio}"
        );
    }

    #[test]
    fn walking_level_ground_bobs_weakly_and_never_jerks() {
        let mut head = Head::new(Vec3::ZERO, 0.0);
        let resting = head.eye().y;
        let mut previous = head.eye();
        let mut worst_offset: f32 = 0.0;
        let mut worst_frame: f32 = 0.0;
        for frame in 1..=120 {
            let x = 3.4 * frame as f32 * FRAME;
            head.follow(Vec3::new(x, 0.0, 0.0), true, FRAME);
            let eye = head.eye();
            worst_offset = worst_offset.max((eye.y - resting).abs());
            // Compare only what the bob added: the eye travels forward with
            // the body, which is not a jerk.
            worst_frame = worst_frame.max((eye.y - previous.y).abs());
            previous = eye;
        }
        assert!(worst_offset < 0.03, "the bob is loud: {worst_offset} m");
        assert!(
            worst_offset > 0.01,
            "the bob is invisible: {worst_offset} m"
        );
        assert!(worst_frame < 0.01, "a frame moved the eye {worst_frame} m");
    }

    #[test]
    fn the_bob_fades_out_when_the_walk_stops() {
        let (mut head, _) = walk(6.8, 3.4);
        let feet = Vec3::new(6.8, 0.0, 0.0);
        stand(&mut head, feet, 1.0);
        assert!(
            (head.eye() - Vec3::new(feet.x, EYE_HEIGHT, feet.z)).length() < 1e-3,
            "a stopped head is centred over its feet, got {:?}",
            head.eye()
        );
    }

    /// Drop the feet from `height` onto the ground and keep following for
    /// `after` seconds, returning the deepest and the final eye height.
    fn drop_from(height: f32, after: f32) -> (f32, f32, f32) {
        let mut head = Head::new(Vec3::new(0.0, height, 0.0), 0.0);
        let resting = head.eye().y;
        // Airborne, falling under the valley's gravity.
        let mut y = height;
        let mut vy = 0.0f32;
        while y > 0.0 {
            vy -= 9.81 * FRAME;
            y = (y + vy * FRAME).max(0.0);
            head.follow(Vec3::new(0.0, y, 0.0), false, FRAME);
        }
        let mut deepest = f32::MAX;
        let frames = (after / FRAME).round() as usize;
        for _ in 0..frames {
            head.follow(Vec3::ZERO, true, FRAME);
            deepest = deepest.min(head.eye().y);
        }
        (resting, deepest, head.eye().y)
    }

    #[test]
    fn landing_dips_the_eye_and_a_longer_fall_dips_it_further() {
        let (resting, shallow, _) = drop_from(0.6, 0.3);
        let dip = resting - 0.6 - shallow;
        assert!(dip > 0.03, "a jump's landing should be felt, got {dip} m");

        let (resting_high, deep, _) = drop_from(2.0, 0.3);
        let deep_dip = resting_high - 2.0 - deep;
        assert!(
            deep_dip > dip,
            "a 2 m fall should dip deeper than a 0.6 m one: {deep_dip} vs {dip}"
        );
        assert!(deep_dip < 0.3, "but not collapse: {deep_dip} m");
    }

    #[test]
    fn the_landing_crouch_is_over_within_a_fifth_of_a_second() {
        let (resting, _, settled) = drop_from(0.6, LANDING_TIME + EYE_SETTLE_TIME);
        // The feet ended on the ground, so the eye ends at the same height it
        // started a metre higher up: resting minus the fall.
        assert!(
            ((resting - 0.6) - settled).abs() < 0.01,
            "still crouching {settled} against {}",
            resting - 0.6
        );
    }

    #[test]
    fn landing_depth_is_proportional_until_the_knees_run_out() {
        assert_eq!(landing_depth(0.0), 0.0);
        assert_eq!(landing_depth(0.02), 0.0, "a footstep is not a landing");
        assert!((landing_depth(0.6) - 0.08).abs() < 1e-6);
        assert!((landing_depth(1.2) - 0.16).abs() < 1e-6);
        assert_eq!(landing_depth(40.0), LANDING_DIP_MAX);
    }

    #[test]
    fn the_head_turns_with_the_mouse_and_stops_short_of_the_poles() {
        let mut head = Head::new(Vec3::ZERO, 0.0);
        assert!((head.forward() - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-6);

        // Mouse right turns right: forward swings towards +X.
        head.turn(Vec2::new(100.0, 0.0));
        assert!(head.forward().x > 0.0, "{:?}", head.forward());
        assert!((head.yaw - 100.0 * MOUSE_SENSITIVITY).abs() < 1e-6);

        // Mouse forward (screen y down is positive) looks up.
        head.turn(Vec2::new(0.0, -100.0));
        assert!(head.forward().y > 0.0);

        head.turn(Vec2::new(0.0, -100_000.0));
        assert!(head.pitch < PI / 2.0, "{}", head.pitch);
        assert!(head.forward().y < 1.0);
        head.turn(Vec2::new(0.0, 200_000.0));
        assert!(head.pitch > -PI / 2.0, "{}", head.pitch);
    }

    #[test]
    fn ahead_and_right_are_the_ground_frame_the_keys_move_in() {
        let mut head = Head::new(Vec3::ZERO, 0.0);
        head.pitch = 1.0; // looking at the sky must not slow a walk down
        assert!((head.ahead() - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-6);
        assert!((head.right() - Vec3::X).length() < 1e-6);
        assert!((head.ahead().length() - 1.0).abs() < 1e-6);

        head.yaw = PI / 2.0;
        assert!(
            (head.ahead() - Vec3::X).length() < 1e-6,
            "{:?}",
            head.ahead()
        );
        assert!((head.right() - Vec3::Z).length() < 1e-6);
    }

    #[test]
    fn aiming_a_camera_puts_it_at_the_eye_looking_forward() {
        let mut head = Head::new(Vec3::new(3.0, 1.0, -2.0), 0.7);
        head.follow(Vec3::new(3.0, 1.0, -2.0), true, FRAME);
        let mut camera = Camera::default();
        head.aim(&mut camera);
        assert_eq!(camera.position, head.eye());
        assert!(((camera.target - camera.position) - head.forward()).length() < 1e-6);
    }
}
