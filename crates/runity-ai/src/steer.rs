//! Turning a decision into movement.
//!
//! A path says where to go; steering says how an agent actually moves there —
//! accelerating, slowing down before it arrives, keeping out of its
//! neighbours' way. It is deliberately independent of the physics crate: most
//! agents in a settlement are not rigid bodies, they are things that walk, and
//! making every villager a simulated capsule would cost far more than it is
//! worth.

use runity_math::Vec3;

use crate::path::Path;
use crate::spatial::ground_distance;

/// How an agent moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Locomotion {
    /// Top speed, in world units per second.
    pub max_speed: f32,
    /// How hard it may accelerate, in units per second squared.
    pub acceleration: f32,
    /// Distance at which it starts slowing down.
    pub arrive_radius: f32,
    /// Distance at which it counts as having arrived.
    pub stop_radius: f32,
    /// Fraction of speed shed per second when nothing is pushing.
    pub damping: f32,
}

impl Default for Locomotion {
    fn default() -> Self {
        // A person walking: about 1.4 m/s, reaching it in half a second.
        Self {
            max_speed: 1.4,
            acceleration: 3.0,
            arrive_radius: 1.5,
            stop_radius: 0.25,
            damping: 4.0,
        }
    }
}

impl Locomotion {
    /// Something quicker — a runner, a cart horse, a monster.
    pub fn fast(max_speed: f32) -> Self {
        Self {
            max_speed,
            acceleration: max_speed * 3.0,
            ..Self::default()
        }
    }
}

/// Desired acceleration toward a target, at full speed.
pub fn seek(position: Vec3, velocity: Vec3, target: Vec3, locomotion: &Locomotion) -> Vec3 {
    let offset = flat(target - position);
    if offset.length_squared() < 1e-8 {
        return -velocity * locomotion.damping;
    }
    let desired = offset.normalized() * locomotion.max_speed;
    clamp_length(desired - velocity, locomotion.acceleration)
}

/// Desired acceleration toward a target, slowing down to stop on it.
///
/// The difference from [`seek`] is the whole reason agents do not look like
/// they are on rails: something that decelerates into its destination reads as
/// deliberate, and something that stops dead reads as a state machine.
pub fn arrive(position: Vec3, velocity: Vec3, target: Vec3, locomotion: &Locomotion) -> Vec3 {
    let offset = flat(target - position);
    let distance = offset.length();
    if distance <= locomotion.stop_radius {
        return clamp_length(-velocity * locomotion.damping, locomotion.acceleration);
    }
    let ramp = if distance < locomotion.arrive_radius && locomotion.arrive_radius > 0.0 {
        distance / locomotion.arrive_radius
    } else {
        1.0
    };
    let desired = offset.normalized() * locomotion.max_speed * ramp;
    clamp_length(desired - velocity, locomotion.acceleration)
}

/// Push away from neighbours that are too close.
///
/// Weighted by how close each one is, so a crowd spreads out instead of
/// oscillating: the nearest neighbour dominates, which is also what people do.
pub fn separation(
    position: Vec3,
    neighbours: impl IntoIterator<Item = Vec3>,
    radius: f32,
    strength: f32,
) -> Vec3 {
    if radius <= 0.0 {
        return Vec3::ZERO;
    }
    let mut push = Vec3::ZERO;
    for other in neighbours {
        let offset = flat(position - other);
        let distance = offset.length();
        if distance >= radius {
            continue;
        }
        if distance < 1e-4 {
            // Exactly coincident: nudge along a fixed axis rather than
            // dividing by zero or picking a random direction that would
            // differ between machines.
            push += Vec3::X * strength;
            continue;
        }
        push += offset * ((radius - distance) / (distance * radius) * strength);
    }
    push
}

/// Steer around an obstacle directly ahead.
///
/// Only the blocking obstacle matters: an agent that dodges everything nearby
/// wanders, while one that dodges what is actually in its way looks like it is
/// paying attention.
pub fn avoid(position: Vec3, velocity: Vec3, obstacle: Vec3, radius: f32, strength: f32) -> Vec3 {
    let heading = flat(velocity);
    if heading.length_squared() < 1e-8 {
        return Vec3::ZERO;
    }
    let heading = heading.normalized();
    let offset = flat(obstacle - position);
    let ahead = offset.dot(heading);
    if ahead <= 0.0 {
        return Vec3::ZERO; // behind us; not our problem
    }
    let lateral = offset - heading * ahead;
    let clearance = lateral.length();
    if clearance >= radius {
        return Vec3::ZERO;
    }
    // Push sideways, away from whichever side the obstacle is on. A tie goes
    // to the left, always, so two agents meeting head-on do not shuffle.
    let side = if clearance < 1e-4 {
        Vec3::new(-heading.z, 0.0, heading.x)
    } else {
        -lateral.normalized()
    };
    side * (strength * (1.0 - clearance / radius))
}

/// Apply an acceleration for `dt` seconds, clamping to the top speed.
///
/// Returns the distance moved, which is what a stuck-detector wants.
pub fn integrate(
    position: &mut Vec3,
    velocity: &mut Vec3,
    acceleration: Vec3,
    locomotion: &Locomotion,
    dt: f32,
) -> f32 {
    if !dt.is_finite() || dt <= 0.0 {
        return 0.0;
    }
    *velocity += flat(acceleration) * dt;
    let speed = velocity.length();
    if speed > locomotion.max_speed {
        *velocity *= locomotion.max_speed / speed;
    }
    let step = *velocity * dt;
    *position += step;
    step.length()
}

/// Walks an agent along a finished [`Path`].
///
/// Keeps the cursor, because "which waypoint am I on" is state the agent owns,
/// not something to recompute from geometry every tick — recomputing it is how
/// an agent ends up oscillating between two corners forever.
#[derive(Clone, Debug, PartialEq)]
pub struct PathFollower {
    path: Path,
    cursor: usize,
    /// How close counts as having reached a waypoint.
    pub tolerance: f32,
}

impl PathFollower {
    /// Follow a path, treating a waypoint as reached within `tolerance`.
    pub fn new(path: Path, tolerance: f32) -> Self {
        Self {
            path,
            cursor: 0,
            tolerance: tolerance.max(1e-3),
        }
    }

    /// The point to steer toward from `position`, advancing past waypoints
    /// already reached. `None` once the path is walked out.
    pub fn target(&mut self, position: Vec3) -> Option<Vec3> {
        while self.cursor < self.path.points.len() {
            let waypoint = self.path.points[self.cursor];
            if ground_distance(position, waypoint) > self.tolerance {
                return Some(waypoint);
            }
            self.cursor += 1;
        }
        None
    }

    /// Whether every waypoint has been reached.
    pub fn is_finished(&self) -> bool {
        self.cursor >= self.path.points.len()
    }

    /// Where the path ends.
    pub fn destination(&self) -> Option<Vec3> {
        self.path.destination()
    }

    /// Fraction of waypoints passed, from 0 to 1.
    pub fn progress(&self) -> f32 {
        if self.path.points.is_empty() {
            return 1.0;
        }
        self.cursor as f32 / self.path.points.len() as f32
    }

    /// Straight-line distance still to walk, from `position`.
    pub fn remaining(&self, position: Vec3) -> f32 {
        let Some(next) = self.path.points.get(self.cursor) else {
            return 0.0;
        };
        let mut total = ground_distance(position, *next);
        for pair in self.path.points[self.cursor..].windows(2) {
            total += ground_distance(pair[0], pair[1]);
        }
        total
    }

    /// The path being followed.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Start again from the first waypoint.
    pub fn restart(&mut self) {
        self.cursor = 0;
    }
}

/// Flatten to the ground plane: agents walk, they do not fly.
#[inline]
fn flat(v: Vec3) -> Vec3 {
    Vec3 {
        x: v.x,
        y: 0.0,
        z: v.z,
    }
}

/// Cut a vector down to a maximum length, leaving its direction alone.
#[inline]
fn clamp_length(v: Vec3, max: f32) -> Vec3 {
    let length = v.length();
    if length > max && length > 0.0 {
        v * (max / length)
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::NavGrid;
    use crate::path::{find_path, PathSettings};
    use runity_math::{vec3, Vec2};

    /// Run an agent until it arrives or gives up, returning where it got to.
    fn walk_to(target: Vec3, steps: usize) -> (Vec3, Vec3, usize) {
        let locomotion = Locomotion::default();
        let mut position = Vec3::ZERO;
        let mut velocity = Vec3::ZERO;
        for step in 0..steps {
            let acceleration = arrive(position, velocity, target, &locomotion);
            integrate(
                &mut position,
                &mut velocity,
                acceleration,
                &locomotion,
                1.0 / 30.0,
            );
            if ground_distance(position, target) < 0.1 && velocity.length() < 0.1 {
                return (position, velocity, step);
            }
        }
        (position, velocity, steps)
    }

    #[test]
    fn an_agent_walks_to_its_target_and_stops_there() {
        let target = vec3(6.0, 0.0, 4.0);
        let (position, velocity, steps) = walk_to(target, 600);
        assert!(
            ground_distance(position, target) < 0.15,
            "ended at {position:?}"
        );
        assert!(velocity.length() < 0.15, "and still moving at {velocity:?}");
        // Seven metres at 1.4 m/s is about five seconds, plus a little to
        // accelerate and slow down.
        assert!(steps < 250, "took {steps} steps, which is far too long");
    }

    #[test]
    fn an_agent_never_exceeds_its_top_speed() {
        let locomotion = Locomotion::default();
        let mut position = Vec3::ZERO;
        let mut velocity = Vec3::ZERO;
        let target = vec3(100.0, 0.0, 0.0);
        for _ in 0..500 {
            let acceleration = seek(position, velocity, target, &locomotion);
            integrate(
                &mut position,
                &mut velocity,
                acceleration,
                &locomotion,
                1.0 / 60.0,
            );
            assert!(
                velocity.length() <= locomotion.max_speed + 1e-4,
                "{:?}",
                velocity
            );
        }
        assert!(position.x > 5.0, "and it did get somewhere");
    }

    #[test]
    fn arriving_slows_down_and_seeking_does_not() {
        let locomotion = Locomotion::default();
        let close = vec3(0.4, 0.0, 0.0);
        let at_speed = vec3(locomotion.max_speed, 0.0, 0.0);

        // Closing on a target at full speed: arrive brakes, seek keeps going.
        let braking = arrive(Vec3::ZERO, at_speed, close, &locomotion);
        let charging = seek(Vec3::ZERO, at_speed, close, &locomotion);
        assert!(
            braking.x < 0.0,
            "arrive should be decelerating: {braking:?}"
        );
        assert!(
            charging.x.abs() < 1e-4,
            "seek is already at speed: {charging:?}"
        );
    }

    #[test]
    fn an_agent_stays_on_the_ground() {
        let locomotion = Locomotion::default();
        let mut position = vec3(0.0, 5.0, 0.0);
        let mut velocity = Vec3::ZERO;
        // A target well below: nothing should push the agent up or down.
        for _ in 0..100 {
            let acceleration = arrive(position, velocity, vec3(3.0, -20.0, 0.0), &locomotion);
            integrate(
                &mut position,
                &mut velocity,
                acceleration,
                &locomotion,
                1.0 / 30.0,
            );
        }
        assert_eq!(
            position.y, 5.0,
            "height is the mover's business, not steering's"
        );
        assert_eq!(velocity.y, 0.0);
    }

    #[test]
    fn separation_pushes_a_crowd_apart() {
        let neighbours = [
            vec3(0.3, 0.0, 0.0),
            vec3(-0.2, 0.0, 0.1),
            vec3(0.0, 0.0, 0.4),
        ];
        let push = separation(Vec3::ZERO, neighbours, 1.0, 1.0);
        assert!(push.length() > 0.0);
        // The nearest neighbour is at -0.2, so the push leans the other way.
        assert!(push.x > 0.0 || push.z < 0.0, "{push:?}");

        // Nobody nearby, nothing to do.
        assert_eq!(
            separation(Vec3::ZERO, [vec3(9.0, 0.0, 9.0)], 1.0, 1.0),
            Vec3::ZERO
        );
        assert_eq!(separation(Vec3::ZERO, [], 1.0, 1.0), Vec3::ZERO);
    }

    #[test]
    fn two_agents_in_exactly_the_same_place_separate_the_same_way_every_time() {
        // Coincident agents are the case that tempts you to pick a random
        // direction, which is exactly what a shared world cannot have.
        let first = separation(Vec3::ZERO, [Vec3::ZERO], 1.0, 1.0);
        let second = separation(Vec3::ZERO, [Vec3::ZERO], 1.0, 1.0);
        assert_eq!(first, second);
        assert!(first.length() > 0.0, "and they do move apart");
    }

    #[test]
    fn a_crowd_actually_spreads_out() {
        let locomotion = Locomotion::default();
        let mut agents: Vec<(Vec3, Vec3)> = (0..8)
            .map(|i| (vec3(i as f32 * 0.05, 0.0, i as f32 * 0.03), Vec3::ZERO))
            .collect();

        let spread = |agents: &[(Vec3, Vec3)]| {
            let mut worst = f32::INFINITY;
            for (i, (a, _)) in agents.iter().enumerate() {
                for (b, _) in &agents[i + 1..] {
                    worst = worst.min(ground_distance(*a, *b));
                }
            }
            worst
        };
        let before = spread(&agents);

        for _ in 0..200 {
            let positions: Vec<Vec3> = agents.iter().map(|(p, _)| *p).collect();
            for (index, (position, velocity)) in agents.iter_mut().enumerate() {
                let others = positions
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != index)
                    .map(|(_, p)| *p);
                let push = separation(*position, others, 1.0, 1.0);
                integrate(position, velocity, push, &locomotion, 1.0 / 30.0);
            }
        }
        assert!(
            spread(&agents) > before + 0.4,
            "the pile should have loosened up"
        );
    }

    #[test]
    fn avoidance_only_reacts_to_what_is_in_the_way() {
        let heading = vec3(1.0, 0.0, 0.0);
        let ahead = avoid(Vec3::ZERO, heading, vec3(3.0, 0.0, 0.2), 1.0, 2.0);
        assert!(
            ahead.length() > 0.0,
            "something in the path should be dodged"
        );
        assert!(ahead.z < 0.0, "and dodged away from it: {ahead:?}");

        let behind = avoid(Vec3::ZERO, heading, vec3(-3.0, 0.0, 0.2), 1.0, 2.0);
        assert_eq!(behind, Vec3::ZERO, "what is behind is not in the way");

        let wide = avoid(Vec3::ZERO, heading, vec3(3.0, 0.0, 9.0), 1.0, 2.0);
        assert_eq!(wide, Vec3::ZERO, "and neither is what it will pass by");

        let stationary = avoid(Vec3::ZERO, Vec3::ZERO, vec3(1.0, 0.0, 0.0), 1.0, 2.0);
        assert_eq!(stationary, Vec3::ZERO, "standing still, nothing is ahead");
    }

    #[test]
    fn a_dead_on_obstacle_is_dodged_to_a_fixed_side() {
        let first = avoid(
            Vec3::ZERO,
            vec3(1.0, 0.0, 0.0),
            vec3(2.0, 0.0, 0.0),
            1.0,
            2.0,
        );
        assert!(first.length() > 0.0);
        assert_eq!(
            first,
            avoid(
                Vec3::ZERO,
                vec3(1.0, 0.0, 0.0),
                vec3(2.0, 0.0, 0.0),
                1.0,
                2.0
            )
        );
    }

    #[test]
    fn a_follower_walks_a_real_path_to_its_end() {
        let mut grid = NavGrid::new(Vec2::ZERO, 1.0, 30, 30);
        for y in 0..24 {
            grid.block(15, y);
        }
        let start = grid.cell_centre(2, 2);
        let goal = grid.cell_centre(27, 2);
        let path = find_path(&grid, start, goal, &PathSettings::default()).unwrap();

        let locomotion = Locomotion::fast(3.0);
        let mut follower = PathFollower::new(path, 0.4);
        let mut position = start;
        let mut velocity = Vec3::ZERO;

        assert_eq!(follower.progress(), 0.0);
        let initial = follower.remaining(position);
        assert!(initial > 25.0, "the detour is long: {initial}");

        let mut steps = 0;
        while let Some(target) = follower.target(position) {
            let acceleration = arrive(position, velocity, target, &locomotion);
            integrate(
                &mut position,
                &mut velocity,
                acceleration,
                &locomotion,
                1.0 / 30.0,
            );
            let (cx, cy) = grid.cell_at(position).expect("still on the grid");
            assert!(
                grid.walkable(cx, cy),
                "walked into the wall at {position:?}"
            );
            steps += 1;
            assert!(steps < 2_000, "never arrived");
        }

        assert!(follower.is_finished());
        assert_eq!(follower.progress(), 1.0);
        assert_eq!(follower.remaining(position), 0.0);
        assert!(
            ground_distance(position, goal) < 0.6,
            "stopped at {position:?}"
        );
    }

    #[test]
    fn a_follower_can_be_restarted_and_reports_its_destination() {
        let grid = NavGrid::new(Vec2::ZERO, 1.0, 10, 10);
        let start = grid.cell_centre(1, 1);
        let goal = grid.cell_centre(8, 8);
        let path = find_path(&grid, start, goal, &PathSettings::default()).unwrap();
        let mut follower = PathFollower::new(path, 0.3);

        assert_eq!(follower.destination(), Some(goal));
        // Visit each waypoint in turn, as an agent walking the path would.
        let waypoints = follower.path().points.clone();
        for waypoint in &waypoints {
            follower.target(*waypoint);
        }
        assert!(follower.target(goal).is_none());
        assert!(follower.is_finished());

        follower.restart();
        assert!(!follower.is_finished());
        // Standing on the first waypoint, the next thing to walk to is the
        // one after it — which across open ground is the goal itself.
        assert_eq!(follower.target(start), Some(goal));
    }

    #[test]
    fn a_zero_or_absurd_step_moves_nothing() {
        let locomotion = Locomotion::default();
        let mut position = vec3(1.0, 0.0, 1.0);
        let mut velocity = vec3(1.0, 0.0, 0.0);
        for dt in [0.0, -1.0, f32::NAN] {
            assert_eq!(
                integrate(&mut position, &mut velocity, Vec3::X, &locomotion, dt),
                0.0
            );
        }
        assert_eq!(position, vec3(1.0, 0.0, 1.0));
        assert_eq!(velocity, vec3(1.0, 0.0, 0.0));
    }
}
