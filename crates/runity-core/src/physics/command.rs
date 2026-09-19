//! What a body is asked to do, and the latch that stops a request falling
//! between a frame and a tick.

use runity_math::Vec3;

/// An intent handed to physics. Not a key, and not a state of a key.
///
/// The direction arrives as a vector, already pointing where the body should
/// go. That is deliberate: turning a mouse delta into a heading needs
/// trigonometry, and trigonometry is the one thing this module will not have —
/// see the module documentation for why. The caller owns its own `sin` and
/// `cos`; physics only ever sees the result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    /// Go this way. `direction` is horizontal (its `y` is ignored) and is
    /// clamped to unit length, so holding two keys is not faster than one.
    /// `speed_scale` is the fraction of top speed to use — 1.0 to run, 0.4 to
    /// creep.
    Move { direction: Vec3, speed_scale: f32 },
    /// Leave the ground on the next tick that has ground to leave.
    Jump,
}

/// A body's commands, held until a tick collects them.
///
/// Attach one to the same entity as the [`super::Body`] it drives.
///
/// This exists because of a race that is otherwise invisible until a player
/// complains. [`crate::Input::begin_frame`] clears the press and release edges
/// every frame, and the fixed tick is slower than the frame rate — 20 Hz
/// against 60 or 144. A `Jump` pressed during a frame that runs no fixed step
/// would therefore be cleared before any tick could see it, and roughly two
/// jumps in three would simply not happen. So the edge is latched here
/// instead: set on the frame the key goes down, cleared by the tick that acts
/// on it and by nothing else. Direction is a level, not an edge, so it is
/// simply overwritten — the newest one a tick sees is the right one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PendingInput {
    /// Where the body is being asked to go, horizontally.
    pub direction: Vec3,
    /// Fraction of top speed, clamped to `[0, 1]`.
    pub speed_scale: f32,
    /// A jump nobody has acted on yet.
    pub jump_requested: bool,
}

impl Default for PendingInput {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingInput {
    pub fn new() -> Self {
        Self {
            direction: Vec3::ZERO,
            speed_scale: 1.0,
            jump_requested: false,
        }
    }

    /// Record a command. Call as often as the frame rate demands.
    pub fn push(&mut self, command: Command) {
        match command {
            Command::Move {
                direction,
                speed_scale,
            } => {
                self.direction = Vec3::new(direction.x, 0.0, direction.z);
                self.speed_scale = speed_scale.clamp(0.0, 1.0);
            }
            Command::Jump => self.jump_requested = true,
        }
    }

    /// Take everything a tick should act on, disarming the jump latch.
    ///
    /// Only a consumed tick clears `jump_requested`; a frame that runs no
    /// fixed step leaves it armed, which is the entire point.
    pub fn consume(&mut self) -> TickInput {
        let jump = std::mem::replace(&mut self.jump_requested, false);
        TickInput {
            direction: self.direction,
            speed_scale: self.speed_scale,
            jump,
        }
    }

    /// Forget everything, including an armed jump — for a body that has just
    /// lost control of itself (focus lost, a cutscene, a death).
    pub fn clear(&mut self) {
        *self = Self::new();
    }
}

/// One tick's worth of intent, as [`PendingInput::consume`] hands it over.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TickInput {
    pub direction: Vec3,
    pub speed_scale: f32,
    pub jump: bool,
}

impl TickInput {
    /// The horizontal velocity this intent asks for, at `max_speed`.
    ///
    /// A direction longer than one unit is normalized first — pressing two
    /// keys must not be `sqrt(2)` times faster than pressing one.
    pub fn desired_velocity(&self, max_speed: f32) -> Vec3 {
        let flat = Vec3::new(self.direction.x, 0.0, self.direction.z);
        let length = flat.length();
        if length <= 1e-6 {
            return Vec3::ZERO;
        }
        let unit = if length > 1.0 {
            flat * (1.0 / length)
        } else {
            flat
        };
        unit * (max_speed * self.speed_scale.clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_jump_survives_frames_that_run_no_tick() {
        let mut pending = PendingInput::new();
        pending.push(Command::Jump);
        // Two more frames go by with nothing pressed and no fixed step due.
        assert!(pending.jump_requested);
        assert!(pending.jump_requested, "still armed a frame later");
        // The first tick that does run collects it.
        assert!(pending.consume().jump);
        assert!(!pending.jump_requested, "and exactly one tick gets it");
        assert!(!pending.consume().jump);
    }

    #[test]
    fn a_direction_is_a_level_and_the_newest_one_wins() {
        let mut pending = PendingInput::new();
        pending.push(Command::Move {
            direction: Vec3::new(1.0, 0.0, 0.0),
            speed_scale: 1.0,
        });
        pending.push(Command::Move {
            direction: Vec3::new(0.0, 0.0, -1.0),
            speed_scale: 0.5,
        });
        let tick = pending.consume();
        assert_eq!(tick.direction, Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(tick.speed_scale, 0.5);
        // Unlike the jump, it is still there for the next tick.
        assert_eq!(pending.consume().direction, Vec3::new(0.0, 0.0, -1.0));
    }

    #[test]
    fn a_vertical_component_of_a_direction_is_ignored() {
        let mut pending = PendingInput::new();
        pending.push(Command::Move {
            direction: Vec3::new(0.0, 5.0, 1.0),
            speed_scale: 1.0,
        });
        assert_eq!(pending.direction, Vec3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn a_diagonal_is_not_faster_than_a_straight_line() {
        let straight = TickInput {
            direction: Vec3::new(0.0, 0.0, 1.0),
            speed_scale: 1.0,
            jump: false,
        };
        let diagonal = TickInput {
            direction: Vec3::new(1.0, 0.0, 1.0),
            ..straight
        };
        let a = straight.desired_velocity(3.4).length();
        let b = diagonal.desired_velocity(3.4).length();
        assert!((a - 3.4).abs() < 1e-5);
        assert!((b - 3.4).abs() < 1e-5, "{b}");
    }

    #[test]
    fn a_short_direction_creeps_instead_of_snapping_to_full_speed() {
        // An analogue stick barely pushed asks for barely any speed.
        let tick = TickInput {
            direction: Vec3::new(0.0, 0.0, 0.25),
            speed_scale: 1.0,
            jump: false,
        };
        assert!((tick.desired_velocity(4.0).length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn no_direction_asks_for_no_velocity() {
        let tick = TickInput::default();
        assert_eq!(tick.desired_velocity(3.4), Vec3::ZERO);
    }

    #[test]
    fn clearing_disarms_a_pending_jump() {
        let mut pending = PendingInput::new();
        pending.push(Command::Jump);
        pending.clear();
        assert!(!pending.consume().jump);
    }
}
