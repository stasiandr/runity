//! A capsule that walks: the thing a player actually controls.
//!
//! A first-person player is deliberately *not* a rigid body. Simulated bodies
//! are pushed around by impulses, which is exactly the wrong feel: a player
//! who slides on ice they did not ask for, bounces off a step, or tips over
//! walking into a crate is a player fighting the physics engine. A character
//! controller instead moves where it is told and then refuses the parts of
//! that motion the world will not allow — collide and slide.
//!
//! It still lives in the same world as the bodies: it collides against them,
//! can shove the light ones, and rides on the same fixed step.

use runity_math::Vec3;

use crate::body::BodyType;
use crate::collide::collide;
use crate::shape::{Aabb, Isometry, Shape};
use crate::world::{BodyHandle, PhysicsWorld};

/// The shape and limits of a walking character.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterSettings {
    /// Radius of the capsule.
    pub radius: f32,
    /// Total height, caps included.
    pub height: f32,
    /// Tallest ledge that can be walked onto rather than into.
    pub step_height: f32,
    /// Steepest ground that still counts as ground, in radians from flat.
    pub max_slope: f32,
    /// Gap kept between the capsule and everything else, so that floating
    /// point never quite lets it touch and stick.
    pub skin: f32,
    /// Downward acceleration.
    pub gravity: f32,
    /// Upward speed a jump starts with.
    pub jump_speed: f32,
    /// Impulse applied to dynamic bodies that get in the way. Zero walks
    /// through crates as if they were walls.
    pub push: f32,
}

impl Default for CharacterSettings {
    fn default() -> Self {
        // A person: 1.8 m tall, shoulder-width, able to step onto a kerb but
        // not onto a table, and to walk up a ramp but not a roof.
        Self {
            radius: 0.35,
            height: 1.8,
            step_height: 0.4,
            max_slope: core::f32::consts::FRAC_PI_4,
            skin: 0.01,
            gravity: -18.0,
            jump_speed: 6.0,
            push: 2.0,
        }
    }
}

impl CharacterSettings {
    /// The capsule that represents this character.
    pub fn shape(&self) -> Shape {
        // The straight part is what is left after the two caps.
        let half_height = (self.height * 0.5 - self.radius).max(0.01);
        Shape::capsule(half_height, self.radius)
    }

    /// Distance from the capsule's centre to the soles of its feet.
    pub fn half_height(&self) -> f32 {
        self.height * 0.5
    }
}

/// What the last move ran into.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveReport {
    /// Whether the character ended the step standing on something.
    pub grounded: bool,
    /// Whether it was blocked sideways by something too steep to climb.
    pub blocked: bool,
    /// Whether it bumped its head.
    pub ceiling: bool,
    /// Whether it stepped up onto something.
    pub stepped: bool,
    /// How far it actually moved, which is rarely what it asked for.
    pub moved: f32,
    /// Normal of the ground under it, when grounded.
    pub ground_normal: Vec3,
}

/// A walking capsule.
///
/// Holds its own position and velocity rather than living in the body array:
/// it is not something the solver may push around, and giving it a handle
/// would invite exactly that.
#[derive(Clone, Debug, PartialEq)]
pub struct Character {
    /// Centre of the capsule, half its height above the feet.
    pub position: Vec3,
    /// Current velocity. Horizontal components are usually set by input,
    /// vertical by gravity and jumping.
    pub velocity: Vec3,
    /// Shape and limits.
    pub settings: CharacterSettings,
    grounded: bool,
    ground_normal: Vec3,
    /// Bodies to ignore — the character's own body, a vehicle it is riding.
    ignored: Vec<BodyHandle>,
}

impl Character {
    /// A character standing with its feet at `feet`.
    pub fn new(settings: CharacterSettings, feet: Vec3) -> Self {
        Self {
            position: feet + Vec3::Y * settings.half_height(),
            velocity: Vec3::ZERO,
            settings,
            grounded: false,
            ground_normal: Vec3::Y,
            ignored: Vec::new(),
        }
    }

    /// Where the character's feet are.
    pub fn feet(&self) -> Vec3 {
        self.position - Vec3::Y * self.settings.half_height()
    }

    /// Where the eyes are, a head's height below the top.
    pub fn eyes(&self) -> Vec3 {
        self.position + Vec3::Y * (self.settings.half_height() - 0.15)
    }

    /// Whether it is standing on something.
    pub fn is_grounded(&self) -> bool {
        self.grounded
    }

    /// The surface under it, or straight up when there is none.
    pub fn ground_normal(&self) -> Vec3 {
        self.ground_normal
    }

    /// Ignore a body from now on.
    pub fn ignore(&mut self, handle: BodyHandle) {
        if !self.ignored.contains(&handle) {
            self.ignored.push(handle);
        }
    }

    /// Jump, if there is something to jump from.
    ///
    /// Returns whether it happened, so the caller can play a sound only when
    /// it did.
    pub fn jump(&mut self) -> bool {
        if !self.grounded {
            return false;
        }
        self.velocity.y = self.settings.jump_speed;
        self.grounded = false;
        true
    }

    /// Teleport, clearing any accumulated fall.
    pub fn teleport(&mut self, feet: Vec3) {
        self.position = feet + Vec3::Y * self.settings.half_height();
        self.velocity = Vec3::ZERO;
        self.grounded = false;
    }

    /// Advance one fixed step.
    ///
    /// `wish` is the horizontal velocity the player is asking for; vertical
    /// motion belongs to gravity and [`Character::jump`]. Splitting them is
    /// what keeps the controller predictable: input never fights the fall.
    pub fn step(&mut self, world: &mut PhysicsWorld, wish: Vec3, dt: f32) -> MoveReport {
        let mut report = MoveReport::default();
        if !dt.is_finite() || dt <= 0.0 {
            report.grounded = self.grounded;
            report.ground_normal = self.ground_normal;
            return report;
        }

        self.velocity.x = wish.x;
        self.velocity.z = wish.z;
        if self.grounded && self.velocity.y <= 0.0 {
            // Keep a little push into the floor: without it, walking down a
            // ramp becomes a series of small falls.
            self.velocity.y = -1.0;
        } else {
            self.velocity.y += self.settings.gravity * dt;
        }

        let start = self.position;
        let motion = self.velocity * dt;

        // Horizontal first, then vertical: resolving them together turns a
        // wall into a ramp, and a player climbing walls is a bug report.
        let horizontal = Vec3 {
            x: motion.x,
            y: 0.0,
            z: motion.z,
        };
        let mut horizontal_report = self.slide(world, horizontal);
        if horizontal_report.blocked
            && self.settings.step_height > 0.0
            && self.try_step(world, horizontal)
        {
            horizontal_report.blocked = false;
            report.stepped = true;
        }
        report.blocked = horizontal_report.blocked;

        let vertical = Vec3 {
            x: 0.0,
            y: motion.y,
            z: 0.0,
        };
        let vertical_report = self.slide(world, vertical);
        report.ceiling = vertical_report.ceiling;
        if vertical_report.ceiling && self.velocity.y > 0.0 {
            self.velocity.y = 0.0;
        }

        self.ground_check(world);
        if self.grounded && self.velocity.y < 0.0 {
            self.velocity.y = 0.0;
        }

        report.grounded = self.grounded;
        report.ground_normal = self.ground_normal;
        report.moved = (self.position - start).length();
        report
    }

    /// Move by `motion`, sliding along whatever it runs into.
    fn slide(&mut self, world: &mut PhysicsWorld, motion: Vec3) -> MoveReport {
        let mut report = MoveReport::default();
        let length = motion.length();
        if length < 1e-6 {
            self.resolve(world, &mut report);
            return report;
        }

        // Never advance more than a fraction of the radius at once, or a fast
        // fall walks straight through a floor between two tests.
        let max_step = (self.settings.radius * 0.5).max(1e-3);
        let substeps = ((length / max_step).ceil() as usize).clamp(1, 16);
        let step = motion * (1.0 / substeps as f32);

        for _ in 0..substeps {
            self.position += step;
            self.resolve(world, &mut report);
        }
        report
    }

    /// Push out of anything the capsule has ended up inside, and record what
    /// was hit.
    fn resolve(&mut self, world: &mut PhysicsWorld, report: &mut MoveReport) {
        let shape = self.settings.shape();
        let cos_limit = self.settings.max_slope.cos();

        // A few passes: pushing out of one wall can push into another, and a
        // corner needs both resolved before the result is stable.
        for _ in 0..4 {
            let mut hit_anything = false;
            let bounds = self.bounds();
            let candidates: Vec<BodyHandle> = world
                .overlapping(bounds)
                .filter(|(handle, body)| {
                    !self.ignored.contains(handle) && body.body_type != BodyType::Kinematic
                })
                .map(|(handle, _)| handle)
                .collect();

            for handle in candidates {
                let Some(body) = world.get(handle) else {
                    continue;
                };
                let at = Isometry::new(self.position, runity_math::Quat::IDENTITY);
                let Some(manifold) = collide(&shape, &at, &body.shape, &body.transform) else {
                    continue;
                };
                let deepest = manifold
                    .contacts()
                    .iter()
                    .fold(0.0f32, |worst, contact| worst.max(contact.penetration));
                if deepest <= 0.0 {
                    continue;
                }

                // The manifold normal points from the character to the body,
                // so pushing out means going the other way.
                let normal = -manifold.normal;
                self.position += normal * (deepest + self.settings.skin);
                hit_anything = true;

                if normal.y > cos_limit {
                    report.grounded = true;
                    report.ground_normal = normal;
                } else if normal.y < -0.5 {
                    report.ceiling = true;
                } else {
                    report.blocked = true;
                }

                // Shove with the speed it had *before* the wall was taken out
                // of it, or a character walking straight at a crate would
                // push with nothing left.
                let approach = Vec3 {
                    x: self.velocity.x,
                    y: 0.0,
                    z: self.velocity.z,
                }
                .length();
                if normal.y <= cos_limit && normal.y >= -0.5 {
                    // Take the wall out of the velocity rather than stopping:
                    // sliding along a wall is what makes movement feel like
                    // movement and not like a series of collisions.
                    let into_wall = self.velocity.dot(normal);
                    if into_wall < 0.0 {
                        self.velocity -= normal * into_wall;
                    }
                }

                self.shove(world, handle, normal, approach);
            }

            if !hit_anything {
                break;
            }
        }
    }

    /// Nudge a light dynamic body the character walked into.
    fn shove(&mut self, world: &mut PhysicsWorld, handle: BodyHandle, normal: Vec3, speed: f32) {
        if self.settings.push <= 0.0 {
            return;
        }
        if speed < 0.1 {
            return;
        }
        let Some(body) = world.get_mut(handle) else {
            return;
        };
        if body.body_type != BodyType::Dynamic {
            return;
        }
        let direction = Vec3 {
            x: -normal.x,
            y: 0.0,
            z: -normal.z,
        };
        if direction.length_squared() < 1e-6 {
            return;
        }
        // Mass deliberately does not appear: the impulse is what the character
        // can deliver, and what it does to the body is then the body's
        // business. A crate skids, a boulder barely twitches.
        body.apply_impulse(direction.normalized() * (speed * self.settings.push * 0.25));
    }

    /// Try to walk up a ledge instead of into it: up, forward, and settle.
    ///
    /// Every stage is an overlap test rather than a push-out. Dropping back
    /// down and depenetrating would shove the capsule off the ledge it just
    /// climbed — the contact it finds down there is the ledge's *side*.
    fn try_step(&mut self, world: &PhysicsWorld, motion: Vec3) -> bool {
        let before = self.position;
        let lift = Vec3::Y * self.settings.step_height;

        let lifted = before + lift;
        if self.overlaps(world, lifted) {
            return false; // no headroom to step into
        }
        let forward = lifted + motion;
        if self.overlaps(world, forward) {
            return false; // the ledge is a wall all the way up
        }

        // Settle back down until something is underfoot, in small steps so
        // the landing is on top of the ledge rather than through it.
        let increment = (self.settings.step_height / 8.0).max(1e-3);
        let mut position = forward;
        let mut dropped = 0.0;
        while dropped < self.settings.step_height {
            let next = position - Vec3::Y * increment;
            if self.overlaps(world, next) {
                break;
            }
            position = next;
            dropped += increment;
        }

        if dropped >= self.settings.step_height {
            return false; // nothing to stand on up there; this was a gap
        }
        self.position = position;
        true
    }

    /// Whether the capsule at `position` is inside anything.
    fn overlaps(&self, world: &PhysicsWorld, position: Vec3) -> bool {
        let shape = self.settings.shape();
        let half = Vec3::new(
            self.settings.radius,
            self.settings.half_height(),
            self.settings.radius,
        );
        let bounds = Aabb::new(position - half, position + half);
        let at = Isometry::new(position, runity_math::Quat::IDENTITY);
        world
            .overlapping(bounds)
            .filter(|(handle, body)| {
                !self.ignored.contains(handle) && body.body_type != BodyType::Kinematic
            })
            .any(|(_, body)| {
                collide(&shape, &at, &body.shape, &body.transform).is_some_and(|manifold| {
                    manifold
                        .contacts()
                        .iter()
                        .any(|c| c.penetration > self.settings.skin)
                })
            })
    }

    /// Look just below the feet for ground.
    fn ground_check(&mut self, world: &PhysicsWorld) {
        let shape = self.settings.shape();
        let probe = self.position - Vec3::Y * (self.settings.skin * 4.0);
        let at = Isometry::new(probe, runity_math::Quat::IDENTITY);
        let cos_limit = self.settings.max_slope.cos();

        self.grounded = false;
        self.ground_normal = Vec3::Y;
        // Track the most upright surface found, so standing in a corner
        // reports the floor rather than whichever wall was tested last.
        let mut best = cos_limit;

        let bounds = Aabb::new(self.bounds().min - Vec3::Y * 0.1, self.bounds().max);
        let candidates: Vec<BodyHandle> = world
            .overlapping(bounds)
            .filter(|(handle, body)| {
                !self.ignored.contains(handle) && body.body_type != BodyType::Kinematic
            })
            .map(|(handle, _)| handle)
            .collect();

        for handle in candidates {
            let Some(body) = world.get(handle) else {
                continue;
            };
            let Some(manifold) = collide(&shape, &at, &body.shape, &body.transform) else {
                continue;
            };
            let normal = -manifold.normal;
            if normal.y > best {
                best = normal.y;
                self.grounded = true;
                self.ground_normal = normal;
            }
        }
    }

    /// Bounds of the capsule where it currently is.
    fn bounds(&self) -> Aabb {
        let half = Vec3::new(
            self.settings.radius,
            self.settings.half_height(),
            self.settings.radius,
        );
        Aabb::new(self.position - half, self.position + half)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::RigidBody;
    use runity_math::vec3;

    const STEP: f32 = 1.0 / 60.0;

    fn world_with_ground() -> PhysicsWorld {
        let mut world = PhysicsWorld::new();
        world.add(RigidBody::fixed(Shape::ground()));
        world
    }

    fn block(world: &mut PhysicsWorld, centre: Vec3, half: Vec3) -> BodyHandle {
        world.add(RigidBody::fixed(Shape::cuboid(half)).with_position(centre))
    }

    /// Walk for `seconds` with a constant wish velocity.
    fn walk(
        character: &mut Character,
        world: &mut PhysicsWorld,
        wish: Vec3,
        seconds: f32,
    ) -> MoveReport {
        let steps = (seconds / STEP).round() as usize;
        let mut last = MoveReport::default();
        for _ in 0..steps {
            last = character.step(world, wish, STEP);
        }
        last
    }

    #[test]
    fn a_character_falls_and_lands_on_its_feet() {
        let mut world = world_with_ground();
        let mut character = Character::new(CharacterSettings::default(), vec3(0.0, 4.0, 0.0));
        assert!(!character.is_grounded());

        walk(&mut character, &mut world, Vec3::ZERO, 2.0);

        assert!(character.is_grounded(), "it should have landed");
        let feet = character.feet().y;
        assert!(feet.abs() < 0.05, "feet ended at {feet}, not on the floor");
        assert!(character.velocity.y.abs() < 0.1, "and stopped falling");
        assert!(
            character.eyes().y > character.feet().y + 1.5,
            "eyes are near the top"
        );
    }

    #[test]
    fn a_character_walks_at_the_speed_it_is_given() {
        let mut world = world_with_ground();
        let mut character = Character::new(CharacterSettings::default(), Vec3::ZERO);
        walk(&mut character, &mut world, Vec3::ZERO, 0.5); // settle

        let start = character.position;
        walk(&mut character, &mut world, vec3(3.0, 0.0, 0.0), 1.0);
        let travelled = character.position.x - start.x;
        assert!(
            (travelled - 3.0).abs() < 0.2,
            "asked for 3 m/s, moved {travelled} m in a second"
        );
        assert!(
            character.is_grounded(),
            "walking on flat ground does not leave it"
        );
    }

    #[test]
    fn a_wall_is_slid_along_rather_than_walked_into() {
        let mut world = world_with_ground();
        // A wall along the x axis, in front of a character heading into it
        // at an angle.
        block(&mut world, vec3(0.0, 1.0, -2.0), vec3(8.0, 1.0, 0.25));

        let mut character = Character::new(CharacterSettings::default(), Vec3::ZERO);
        walk(&mut character, &mut world, Vec3::ZERO, 0.5);
        let start = character.position;

        let report = walk(&mut character, &mut world, vec3(2.0, 0.0, -2.0), 1.5);

        assert!(report.blocked, "it should notice the wall");
        assert!(
            character.position.z > -1.8,
            "it must not be inside the wall: {:?}",
            character.position
        );
        // The whole point: the sideways part of the motion survived.
        assert!(
            character.position.x - start.x > 1.5,
            "it should have slid along, moved {} in x",
            character.position.x - start.x
        );
    }

    #[test]
    fn a_low_ledge_is_stepped_onto_and_a_high_one_is_not() {
        let climb = |height: f32| {
            let mut world = world_with_ground();
            block(
                &mut world,
                vec3(2.0, height * 0.5, 0.0),
                vec3(1.0, height * 0.5, 4.0),
            );
            let mut character = Character::new(CharacterSettings::default(), Vec3::ZERO);
            walk(&mut character, &mut world, Vec3::ZERO, 0.5);
            // One second at 2 m/s, which lands squarely on top of a ledge
            // that spans x = 1 to 3 — walk longer and it strolls off the far
            // side, which is correct and makes for a confusing assertion.
            walk(&mut character, &mut world, vec3(2.0, 0.0, 0.0), 1.0);
            character
        };

        let stepped = climb(0.3);
        assert!(
            stepped.feet().y > 0.2,
            "a kerb should be stepped onto, feet at {}",
            stepped.feet().y
        );
        assert!(stepped.position.x > 1.5, "and walked across");

        let stopped = climb(1.2);
        assert!(
            stopped.feet().y < 0.2,
            "a wall is not a step: feet at {}",
            stopped.feet().y
        );
        assert!(
            stopped.position.x < 1.2,
            "and it should not have got past: {:?}",
            stopped.position
        );
    }

    #[test]
    fn jumping_leaves_the_ground_and_comes_back() {
        let mut world = world_with_ground();
        let mut character = Character::new(CharacterSettings::default(), Vec3::ZERO);
        walk(&mut character, &mut world, Vec3::ZERO, 0.5);
        assert!(character.is_grounded());

        assert!(
            character.jump(),
            "standing on the floor, a jump should happen"
        );
        assert!(!character.jump(), "but not a second one in mid air");

        let mut peak: f32 = 0.0;
        for _ in 0..120 {
            character.step(&mut world, Vec3::ZERO, STEP);
            peak = peak.max(character.feet().y);
        }
        // v^2 / 2g with the controller's own numbers.
        let expected = character.settings.jump_speed.powi(2) / (2.0 * -character.settings.gravity);
        assert!(
            (peak - expected).abs() < 0.2,
            "jumped {peak} m, expected about {expected}"
        );
        assert!(character.is_grounded(), "and came back down");
        assert!(character.feet().y.abs() < 0.05);
    }

    #[test]
    fn a_ceiling_stops_a_jump_without_sticking() {
        let mut world = world_with_ground();
        block(&mut world, vec3(0.0, 2.6, 0.0), vec3(3.0, 0.25, 3.0));

        let mut character = Character::new(CharacterSettings::default(), Vec3::ZERO);
        walk(&mut character, &mut world, Vec3::ZERO, 0.5);
        character.jump();

        let mut bumped = false;
        for _ in 0..120 {
            let report = character.step(&mut world, Vec3::ZERO, STEP);
            bumped |= report.ceiling;
            assert!(character.position.y < 2.6, "it went through the ceiling");
        }
        assert!(bumped, "it should have hit its head");
        assert!(character.is_grounded(), "and fallen back down");
    }

    #[test]
    fn a_fast_fall_does_not_pass_through_a_thin_floor() {
        // The classic discrete-collision failure: at 60 m/s a step is a metre,
        // and a floor 20 cm thick is not there for any of the tests.
        let mut world = PhysicsWorld::new();
        block(&mut world, vec3(0.0, 0.0, 0.0), vec3(4.0, 0.1, 4.0));

        let mut character = Character::new(CharacterSettings::default(), vec3(0.0, 40.0, 0.0));
        for _ in 0..600 {
            character.step(&mut world, Vec3::ZERO, STEP);
            assert!(
                character.position.y > -1.0,
                "fell through at {:?}",
                character.position
            );
        }
        assert!(character.is_grounded());
        assert!(
            (character.feet().y - 0.1).abs() < 0.06,
            "landed at {}",
            character.feet().y
        );
    }

    #[test]
    fn a_gentle_slope_is_ground_and_a_steep_one_is_not() {
        let slope = |degrees: f32| {
            let mut world = PhysicsWorld::new();
            let radians = degrees.to_radians();
            // A half-space tilted around the z axis.
            let normal = vec3(-radians.sin(), radians.cos(), 0.0);
            world.add(RigidBody::fixed(Shape::HalfSpace { normal }));
            let mut character = Character::new(CharacterSettings::default(), vec3(0.0, 2.0, 0.0));
            walk(&mut character, &mut world, Vec3::ZERO, 2.0);
            character
        };

        let walkable = slope(20.0);
        assert!(walkable.is_grounded(), "twenty degrees is a ramp");
        assert!(walkable.ground_normal().y > 0.9);

        let cliff = slope(70.0);
        assert!(!cliff.is_grounded(), "seventy degrees is a cliff face");
    }

    #[test]
    fn the_same_inputs_produce_the_same_walk() {
        let run = || {
            let mut world = world_with_ground();
            block(&mut world, vec3(3.0, 1.0, 0.0), vec3(0.5, 1.0, 4.0));
            let mut character = Character::new(CharacterSettings::default(), Vec3::ZERO);
            for tick in 0..300 {
                let wish = vec3(2.0, 0.0, ((tick % 60) as f32 - 30.0) * 0.05);
                character.step(&mut world, wish, STEP);
            }
            (
                character.position,
                character.velocity,
                character.is_grounded(),
            )
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn a_character_can_shove_a_crate_but_not_a_wall() {
        let mut world = world_with_ground();
        let crate_body = world.add(
            RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.4)))
                .with_position(vec3(1.5, 0.4, 0.0))
                .with_mass(4.0),
        );
        let wall = block(&mut world, vec3(-3.0, 1.0, 0.0), vec3(0.3, 1.0, 3.0));

        let mut character = Character::new(CharacterSettings::default(), Vec3::ZERO);
        walk(&mut character, &mut world, Vec3::ZERO, 0.5);
        let crate_start = world.get(crate_body).unwrap().position();

        for _ in 0..180 {
            character.step(&mut world, vec3(2.0, 0.0, 0.0), STEP);
            world.step(STEP);
        }

        let moved = world.get(crate_body).unwrap().position().x - crate_start.x;
        assert!(
            moved > 0.2,
            "the crate should have been pushed along, moved {moved}"
        );
        assert_eq!(
            world.get(wall).unwrap().position(),
            vec3(-3.0, 1.0, 0.0),
            "the wall stays"
        );
    }

    #[test]
    fn ignored_bodies_are_walked_through() {
        let mut world = world_with_ground();
        let ghost = block(&mut world, vec3(2.0, 1.0, 0.0), vec3(0.5, 1.0, 3.0));

        let mut character = Character::new(CharacterSettings::default(), Vec3::ZERO);
        character.ignore(ghost);
        walk(&mut character, &mut world, Vec3::ZERO, 0.5);
        walk(&mut character, &mut world, vec3(2.0, 0.0, 0.0), 2.0);

        assert!(
            character.position.x > 3.0,
            "it should have passed straight through"
        );
    }

    #[test]
    fn teleporting_moves_the_feet_and_clears_the_fall() {
        let mut world = world_with_ground();
        let mut character = Character::new(CharacterSettings::default(), vec3(0.0, 20.0, 0.0));
        walk(&mut character, &mut world, Vec3::ZERO, 1.0);
        assert!(character.velocity.y < -1.0, "it should be falling");

        character.teleport(vec3(5.0, 0.0, 5.0));
        assert_eq!(character.feet(), vec3(5.0, 0.0, 5.0));
        assert_eq!(character.velocity, Vec3::ZERO);
    }

    #[test]
    fn an_absurd_step_changes_nothing() {
        let mut world = world_with_ground();
        let mut character = Character::new(CharacterSettings::default(), vec3(0.0, 1.0, 0.0));
        let before = character.clone();
        for dt in [0.0, -1.0, f32::NAN] {
            character.step(&mut world, vec3(5.0, 0.0, 5.0), dt);
        }
        assert_eq!(character.position, before.position);
        assert_eq!(character.velocity, before.velocity);
    }
}
