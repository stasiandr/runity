//! One fixed tick, in the seven passes the design asks for.
//!
//! Each pass runs over every body before the next one starts. That ordering is
//! not incidental: body-body collision needs to see all the bodies at once,
//! and a slope check that ran per-body would see half the world already moved
//! and half of it not.

use super::blocker::Blocker;
use super::body::Body;
use super::command::{PendingInput, TickInput};
use super::space::{PhysicsWorld, Support, Tuning};
use super::tree::FallingTree;
use crate::transform::Transform;
use crate::world::{Entity, World};
use runity_math::{Vec2, Vec3};

/// How high the log a felled tree leaves behind stands. Low enough to be a
/// step and not a wall, which is what makes a cleared forest walkable.
pub const LOG_TOP: f32 = 0.3;

/// A trunk mid-swing, and where its stump is.
struct Sweep {
    stump: Vec2,
    tree: FallingTree,
}

/// Advance the simulation by one [`PhysicsWorld::fixed_delta`].
///
/// The order is fixed and each step is named after the design it comes from:
///
/// 1. horizontal velocity eases towards what was asked for,
/// 2. collisions against other bodies and against statics, resolved by
///    sliding,
/// 3. a climb too steep to take is turned along the hill instead,
/// 4. a rise under `step_height` is simply walked up; anything higher blocks,
/// 5. gravity, then the clamp onto whatever is holding the body up,
/// 6. a jump leaves the ground,
/// 7. a body standing on ground too steep for it slides down.
pub fn step(physics: &mut PhysicsWorld, world: &mut World) {
    let dt = physics.fixed_delta();
    let tuning = physics.tuning;

    // Trees first. A trunk that lands this tick trades its standing blocker
    // for a log, and the grid has to know before a single body asks it a
    // question.
    let sweeps = advance_trees(physics, world, dt);
    physics.sync_statics(world);

    let entities = world.entities_with::<Body>();
    if entities.is_empty() {
        return;
    }
    let mut bodies: Vec<Body> = entities
        .iter()
        .map(|e| {
            *world
                .get::<Body>(*e)
                .expect("entities_with only yields entities that have one")
        })
        .collect();
    let inputs: Vec<TickInput> = entities
        .iter()
        .map(|e| {
            world
                .get_mut::<PendingInput>(*e)
                .map(PendingInput::consume)
                .unwrap_or_default()
        })
        .collect();

    for body in &mut bodies {
        body.prev_position = body.position;
        // `jumped` lasts exactly one tick, so clearing it is the first thing
        // that happens and setting it is nearly the last.
        body.jumped = false;
    }

    let mut nearby: Vec<u32> = Vec::new();

    // --- 1. control ------------------------------------------------------
    for (body, input) in bodies.iter_mut().zip(&inputs) {
        let support = physics.support_with(
            body.position.x,
            body.position.z,
            body.position.y,
            body.radius,
            &mut nearby,
        );
        accelerate(body, input, &tuning, &support, dt);
    }

    // --- 1a. a falling trunk overrules whatever a body wanted -------------
    for body in &mut bodies {
        for sweep in &sweeps {
            let point = Vec2::new(body.position.x, body.position.z);
            if sweep.tree.arc_covers(sweep.stump, point, body.radius) {
                shove(body, sweep.tree.direction, tuning.tree_push_speed);
            }
        }
    }

    // --- 2. move, then resolve what that ran into ------------------------
    for body in &mut bodies {
        body.position.x += body.velocity.x * dt;
        body.position.z += body.velocity.z * dt;
    }
    resolve_body_pairs(&mut bodies);
    for body in &mut bodies {
        resolve_statics(physics, body, &tuning, &mut nearby);
    }

    // --- 3. a slope too steep to climb ------------------------------------
    for body in &mut bodies {
        cancel_steep_climb(physics, body, &tuning, &mut nearby);
    }

    // --- 4. steps and ledges ----------------------------------------------
    let mut supports: Vec<Support> = Vec::with_capacity(bodies.len());
    for body in &mut bodies {
        supports.push(take_the_step(physics, body, &tuning, &mut nearby));
    }

    // --- 5. gravity and the ground ----------------------------------------
    for (body, support) in bodies.iter_mut().zip(&supports) {
        fall(body, support, &tuning, dt);
    }

    // --- 6. jump -----------------------------------------------------------
    for (body, input) in bodies.iter_mut().zip(&inputs) {
        if input.jump && body.grounded {
            body.velocity.y = tuning.jump_speed;
            body.grounded = false;
            body.jumped = true;
        }
    }

    // --- 7. slide ----------------------------------------------------------
    for (body, support) in bodies.iter_mut().zip(&supports) {
        slide_downhill(body, support, &tuning, dt);
    }

    for (entity, body) in entities.iter().zip(&bodies) {
        if let Some(slot) = world.get_mut::<Body>(*entity) {
            *slot = *body;
        }
    }
}

/// 1. Ease the horizontal velocity towards what the tick was asked for.
///
/// Acceleration is capped, so a body leans into a run over
/// [`Tuning::acceleration_time`] rather than snapping to top speed — and the
/// same cap brings it back to a stop.
///
/// The exception is a body standing on ground too steep to stand on. Control
/// needs something to push against, and a slope past `max_slope_cos` offers
/// none: an empty command there does not brake, so pass 7's slide is not
/// undone the instant it happens. Pushing *is* still allowed, which is what
/// lets a body traverse a steep face along its contour.
fn accelerate(body: &mut Body, input: &TickInput, tuning: &Tuning, support: &Support, dt: f32) {
    let desired = input.desired_velocity(tuning.max_speed);
    let sliding = body.grounded && support.is_steep(tuning);
    if sliding && desired == Vec3::ZERO {
        return;
    }
    let current = Vec3::new(body.velocity.x, 0.0, body.velocity.z);
    let delta = desired - current;
    let distance = delta.length();
    let budget = tuning.acceleration() * dt;
    let applied = if distance <= budget {
        delta
    } else {
        delta * (budget / distance)
    };
    body.velocity.x += applied.x;
    body.velocity.z += applied.z;
}

/// Give a body at least `speed` along `direction`, leaving the part of its
/// velocity across that direction alone.
fn shove(body: &mut Body, direction: Vec2, speed: f32) {
    let along = body.velocity.x * direction.x + body.velocity.z * direction.y;
    if along < speed {
        let boost = speed - along;
        body.velocity.x += direction.x * boost;
        body.velocity.z += direction.y * boost;
    }
}

/// 2a. Every pair of bodies, tested outright.
///
/// Twelve settlers is sixty-six pairs. A grid to avoid them would cost more to
/// rebuild every tick than the pairs cost to test, and would be a second
/// spatial index to keep honest.
fn resolve_body_pairs(bodies: &mut [Body]) {
    for i in 0..bodies.len() {
        let (left, right) = bodies.split_at_mut(i + 1);
        let a = &mut left[i];
        for b in right.iter_mut() {
            if !a.overlaps_span(b.position.y, b.top()) {
                continue;
            }
            let delta = Vec2::new(b.position.x - a.position.x, b.position.z - a.position.z);
            let reach = a.radius + b.radius;
            let distance = delta.length();
            if distance >= reach {
                continue;
            }
            let normal = if distance > 1e-6 {
                delta * (1.0 / distance)
            } else {
                Vec2::new(1.0, 0.0)
            };
            let half = (reach - distance) * 0.5;
            a.position.x -= normal.x * half;
            a.position.z -= normal.y * half;
            b.position.x += normal.x * half;
            b.position.z += normal.y * half;
            // Each keeps whatever part of its velocity runs along the contact
            // — that is the slide. Only the part driving into the other body
            // is taken away.
            slide_along(&mut a.velocity, normal * -1.0);
            slide_along(&mut b.velocity, normal);
        }
    }
}

/// Remove the part of a horizontal velocity that points opposite `escape`,
/// which is the unit direction out of the thing just hit.
fn slide_along(velocity: &mut Vec3, escape: Vec2) {
    let into = velocity.x * escape.x + velocity.z * escape.y;
    if into < 0.0 {
        velocity.x -= escape.x * into;
        velocity.z -= escape.y * into;
    }
}

/// 2b. Push out of any static tall enough to count as a wall.
///
/// The step-height rule decides what a wall even is: a blocker whose top is no
/// more than `step_height` above the foot the body started the tick with is
/// not an obstacle at all, it is something to walk over, and pass 4 will lift
/// the body onto it.
fn resolve_statics(
    physics: &PhysicsWorld,
    body: &mut Body,
    tuning: &Tuning,
    nearby: &mut Vec<u32>,
) {
    let ceiling = body.prev_position.y + tuning.step_height;
    physics.nearby_statics(body.position.x, body.position.z, nearby);
    for &index in nearby.iter() {
        let shape = physics.shape(index);
        if shape.top() <= ceiling || shape.base.y >= body.top() {
            continue;
        }
        let point = Vec2::new(body.position.x, body.position.z);
        if let Some((normal, depth)) = shape.resolve(point, body.radius) {
            body.position.x += normal.x * depth;
            body.position.z += normal.y * depth;
            slide_along(&mut body.velocity, normal);
        }
    }
}

/// 3. Refuse the uphill part of a move onto ground too steep to climb.
///
/// The move is not cancelled, only its component straight up the hill: what is
/// left runs along the contour, so a body pressed into a cliff walks sideways
/// along it instead of sticking to it.
fn cancel_steep_climb(
    physics: &PhysicsWorld,
    body: &mut Body,
    tuning: &Tuning,
    nearby: &mut Vec<u32>,
) {
    let moved = Vec2::new(
        body.position.x - body.prev_position.x,
        body.position.z - body.prev_position.z,
    );
    if moved == Vec2::ZERO {
        return;
    }
    let support = physics.support_with(
        body.position.x,
        body.position.z,
        body.prev_position.y,
        body.radius,
        nearby,
    );
    if !support.is_steep(tuning) {
        return;
    }
    let uphill = support.downhill() * -1.0;
    let climb = moved.dot(uphill);
    if climb <= 0.0 {
        return;
    }
    body.position.x -= uphill.x * climb;
    body.position.z -= uphill.y * climb;
    // Without this the velocity would keep pressing into the hill and rebuild
    // the same doomed climb every tick.
    let into_hill = body.velocity.x * uphill.x + body.velocity.z * uphill.y;
    if into_hill > 0.0 {
        body.velocity.x -= uphill.x * into_hill;
        body.velocity.z -= uphill.y * into_hill;
    }
}

/// 4. Walk up a small rise; refuse a large one.
///
/// Under `step_height` the body is simply lifted, inside this tick, with no
/// airborne phase — a kerb or a felled log is not a jump. At or above it, the
/// horizontal move is given back: that is a ledge, and the way up is around or
/// over.
///
/// Returns the surface the body ends the pass over, which pass 5 lands on.
fn take_the_step(
    physics: &PhysicsWorld,
    body: &mut Body,
    tuning: &Tuning,
    nearby: &mut Vec<u32>,
) -> Support {
    let foot = body.prev_position.y;
    let support = physics.support_with(body.position.x, body.position.z, foot, body.radius, nearby);
    if support.height - foot >= tuning.step_height {
        let here = physics.support_with(
            body.prev_position.x,
            body.prev_position.z,
            foot,
            body.radius,
            nearby,
        );
        // Unless the body was already inside something that high, in which
        // case refusing the move would wedge it there forever.
        if here.height - foot < tuning.step_height {
            let moved = Vec2::new(
                body.position.x - body.prev_position.x,
                body.position.z - body.prev_position.z,
            );
            body.position.x = body.prev_position.x;
            body.position.z = body.prev_position.z;
            slide_along(&mut body.velocity, moved.normalized() * -1.0);
            return here;
        }
        return support;
    }
    if body.grounded && support.height > body.position.y {
        // The lift happens here, inside the tick: there is no airborne phase
        // and nothing for a renderer to interpolate through.
        body.position.y = support.height;
        body.velocity.y = 0.0;
        // Standing higher can bring a taller blocker within stepping reach,
        // so the support that pass 5 lands on is re-read from the new foot.
        return physics.support_with(
            body.position.x,
            body.position.z,
            body.position.y,
            body.radius,
            nearby,
        );
    }
    support
}

/// 5. Gravity, then the clamp onto whatever is holding the body up.
///
/// The half-step term is not decoration: plain symplectic Euler at 20 Hz turns
/// the designed 0.60 m jump into 0.51 m, because it charges a whole tick of
/// gravity before the first tick of travel. `v·t − g·t²/2` is exact for a
/// constant acceleration, so the arc a player feels is the arc the tuning
/// describes.
///
/// This clamp is also the reason no test ever finds a body below the ground it
/// can see: `support.height` starts at the heightfield's own `height_at`, the
/// very function that built the mesh.
///
/// A body already on the ground also reaches *down* for it, by the same
/// [`Tuning::step_height`] that decides what it can climb. Without that, going
/// downhill launches: one tick moves a body 0.17 m along a slope that drops
/// 0.2 m, while gravity has only pulled it 0.012 m, so the feet leave the
/// surface and `grounded` flickers off every other tick — which stops a jump
/// working on a descent, and stops pass 7's slide dead, since pass 1 brakes
/// anything it believes to be airborne. Reaching down is the same rule as
/// stepping up, pointed the other way: a 0.3 m drop is a step down, and a
/// cliff is still a cliff.
fn fall(body: &mut Body, support: &Support, tuning: &Tuning, dt: f32) {
    let was_grounded = body.grounded;
    let vy = body.velocity.y;
    body.velocity.y = vy - tuning.gravity * dt;
    body.position.y += vy * dt - 0.5 * tuning.gravity * dt * dt;
    let landed = body.position.y <= support.height;
    let stepped_down =
        was_grounded && vy <= 0.0 && body.position.y - support.height <= tuning.step_height;
    if landed || stepped_down {
        body.position.y = support.height;
        if body.velocity.y < 0.0 {
            body.velocity.y = 0.0;
        }
        body.grounded = true;
    } else {
        body.grounded = false;
    }
}

/// 7. Ground steeper than a body can stand on takes it downhill.
///
/// The horizontal pull of gravity on a slope is `g·sinθ·cosθ`, and both
/// factors are already in the normal: `sinθ` is the length of its horizontal
/// part and `cosθ` is its `y`. No angle is ever formed.
///
/// The slide is capped at [`Tuning::max_speed`], which is not friction dressed
/// up — it is the anti-tunnelling invariant of [`super::PhysicsWorld::new`]
/// applied to the one source of horizontal speed that pass 1 does not already
/// bound. A long enough cliff would otherwise accelerate a body past
/// `min_body_radius / fixed_delta` and hand it the ability to cross a wall
/// between two ticks.
fn slide_downhill(body: &mut Body, support: &Support, tuning: &Tuning, dt: f32) {
    if !body.grounded || !support.is_steep(tuning) {
        return;
    }
    let horizontal = Vec2::new(support.normal.x, support.normal.z);
    let pull = tuning.gravity * horizontal.length() * support.normal.y;
    let downhill = horizontal.normalized();
    body.velocity.x += downhill.x * pull * dt;
    body.velocity.z += downhill.y * pull * dt;
    let speed = Vec2::new(body.velocity.x, body.velocity.z).length();
    if speed > tuning.max_speed {
        let scale = tuning.max_speed / speed;
        body.velocity.x *= scale;
        body.velocity.z *= scale;
    }
}

/// Swing every trunk that is coming down, and turn the ones that arrive into
/// logs. Returns the arcs bodies have to be shoved out of this tick.
fn advance_trees(physics: &mut PhysicsWorld, world: &mut World, dt: f32) -> Vec<Sweep> {
    let torque = physics.tuning.tree_torque;
    let mut sweeps = Vec::new();
    for entity in world.entities_with::<FallingTree>() {
        let Some(base) = world.get::<Transform>(entity).map(|t| t.position) else {
            continue;
        };
        let Some(tree) = world.get_mut::<FallingTree>(entity) else {
            continue;
        };
        if !tree.falling {
            continue;
        }
        let landed = tree.advance(dt, torque);
        let mut sweeping = *tree;
        // The tick a trunk arrives still sweeps everything under it.
        sweeping.falling = true;
        sweeps.push(Sweep {
            stump: Vec2::new(base.x, base.z),
            tree: sweeping,
        });
        if landed {
            let tree = *tree;
            lay_the_log(physics, world, entity, base, &tree);
        }
    }
    sweeps
}

/// Take the standing blocker off a tree that has landed and put a log where it
/// came to rest.
fn lay_the_log(
    physics: &mut PhysicsWorld,
    world: &mut World,
    tree_entity: Entity,
    base: Vec3,
    tree: &FallingTree,
) {
    world.remove::<Blocker>(tree_entity);
    let (half, offset) = tree.log_footprint();
    let log = world.spawn();
    world.insert(
        log,
        Transform::from_position(Vec3::new(base.x + offset.x, base.y, base.z + offset.y)),
    );
    world.insert(
        log,
        Blocker::Box {
            half,
            facing: tree.direction,
            top: LOG_TOP,
        },
    );
    physics.mark_statics_dirty();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::body::BodyKind;
    use crate::physics::command::Command;
    use crate::physics::heightfield::Heightfield;
    use crate::physics::space::Tuning;

    /// The valley's tick: 20 Hz.
    const DT: f32 = 1.0 / 20.0;

    fn flat_ground() -> Heightfield {
        Heightfield::new(Vec2::splat(-40.0), 4.0, 21, 21, vec![0.0; 441])
    }

    /// Ground rising along +X at `gradient` metres per metre. A gradient is
    /// `tan` of the slope, but nothing here ever has to know that: the normal
    /// the heightfield hands back is what the slope test reads.
    fn ramp(gradient: f32) -> Heightfield {
        let (cols, rows, cell) = (21usize, 21usize, 4.0f32);
        let origin = Vec2::splat(-40.0);
        let mut heights = Vec::with_capacity(cols * rows);
        for _ in 0..rows {
            for col in 0..cols {
                heights.push((origin.x + col as f32 * cell) * gradient);
            }
        }
        Heightfield::new(origin, cell, cols, rows, heights)
    }

    /// A world, its bodies, and a tick button.
    struct Scene {
        physics: PhysicsWorld,
        world: World,
    }

    impl Scene {
        fn new(ground: Heightfield) -> Self {
            Self {
                physics: PhysicsWorld::new(ground, Tuning::default(), DT),
                world: World::new(),
            }
        }

        fn flat() -> Self {
            Self::new(flat_ground())
        }

        /// A body standing on the ground at `(x, z)`, with a [`PendingInput`]
        /// to drive it.
        fn walker(&mut self, x: f32, z: f32) -> Entity {
            let y = self.physics.ground.height_at(x, z);
            let entity = self.world.spawn();
            self.world.insert(entity, Body::new(Vec3::new(x, y, z)));
            self.world.insert(entity, PendingInput::new());
            entity
        }

        fn blocker(&mut self, position: Vec3, blocker: Blocker) -> Entity {
            let entity = self.world.spawn();
            self.world
                .insert(entity, Transform::from_position(position));
            self.world.insert(entity, blocker);
            self.physics.mark_statics_dirty();
            entity
        }

        fn walk(&mut self, entity: Entity, direction: Vec3) {
            self.world
                .get_mut::<PendingInput>(entity)
                .expect("a walker has one")
                .push(Command::Move {
                    direction,
                    speed_scale: 1.0,
                });
        }

        fn jump(&mut self, entity: Entity) {
            self.world
                .get_mut::<PendingInput>(entity)
                .expect("a walker has one")
                .push(Command::Jump);
        }

        fn tick(&mut self) {
            step(&mut self.physics, &mut self.world);
        }

        fn ticks(&mut self, count: usize) {
            for _ in 0..count {
                self.tick();
            }
        }

        fn body(&self, entity: Entity) -> Body {
            *self.world.get::<Body>(entity).expect("a body")
        }
    }

    // --- 1. control ------------------------------------------------------

    #[test]
    fn a_body_leans_into_a_run_rather_than_snapping_to_top_speed() {
        let mut scene = Scene::flat();
        let walker = scene.walker(0.0, 0.0);
        scene.walk(walker, Vec3::new(0.0, 0.0, 1.0));

        scene.tick();
        let after_one = scene.body(walker).velocity.z;
        assert!(
            after_one < 3.4,
            "one tick must not reach top speed, got {after_one}"
        );

        // 0.12 s of acceleration is between two and three ticks at 20 Hz.
        scene.ticks(2);
        let settled = scene.body(walker).velocity.z;
        assert!(
            (settled - 3.4).abs() < 1e-4,
            "up to speed within the acceleration time, got {settled}"
        );

        // And the same cap brings it back down.
        scene.walk(walker, Vec3::ZERO);
        scene.ticks(3);
        let stopped = scene.body(walker).velocity;
        assert!(
            stopped.x.abs() < 1e-4 && stopped.z.abs() < 1e-4,
            "{stopped:?}"
        );
    }

    #[test]
    fn a_body_with_no_input_at_all_simply_stands_there() {
        let mut scene = Scene::flat();
        let idle = scene.walker(1.0, -2.0);
        scene.ticks(40);
        let body = scene.body(idle);
        assert_eq!(body.position, Vec3::new(1.0, 0.0, -2.0));
        assert!(body.grounded);
        assert_eq!(body.velocity, Vec3::ZERO);
    }

    // --- the ground is never below a body -------------------------------

    #[test]
    fn a_body_never_ends_a_tick_below_the_ground_it_can_see() {
        // A heightfield with a different height at every node, so a body
        // crossing it meets both triangles of many cells and every cell edge
        // and diagonal between them. `height_at` is the same function the mesh
        // is built from, so "below the ground" here means below what is drawn.
        let (cols, rows, cell) = (13usize, 13usize, 2.0f32);
        let origin = Vec2::splat(-12.0);
        let mut heights = Vec::with_capacity(cols * rows);
        for row in 0..rows {
            for col in 0..cols {
                // Deterministic, varied, and gentle enough to stay walkable.
                let bump = ((row * 7 + col * 3) % 5) as f32 * 0.15;
                heights.push(bump + ((col * 5 + row) % 3) as f32 * 0.1);
            }
        }
        let mut scene = Scene::new(Heightfield::new(origin, cell, cols, rows, heights));

        // Walk the diagonal: every cell is cut along its own diagonal, so a
        // body travelling this way crosses the seam again and again.
        let walker = scene.walker(-8.0, -8.0);
        scene.walk(walker, Vec3::new(1.0, 0.0, 1.0));
        for tick in 0..200 {
            scene.tick();
            let body = scene.body(walker);
            let ground = scene
                .physics
                .ground
                .height_at(body.position.x, body.position.z);
            assert!(
                body.position.y >= ground - 1e-4,
                "tick {tick}: body at {:?} is below the ground at {ground}",
                body.position
            );
        }

        // And the same going the other way across the diagonals.
        let crosser = scene.walker(-8.0, 8.0);
        scene.walk(crosser, Vec3::new(1.0, 0.0, -1.0));
        for tick in 0..200 {
            scene.tick();
            let body = scene.body(crosser);
            let ground = scene
                .physics
                .ground
                .height_at(body.position.x, body.position.z);
            assert!(
                body.position.y >= ground - 1e-4,
                "tick {tick}: body at {:?} is below the ground at {ground}",
                body.position
            );
        }
    }

    #[test]
    fn a_body_dropped_from_a_height_lands_on_the_ground_and_stays_there() {
        let mut scene = Scene::flat();
        let entity = scene.world.spawn();
        scene
            .world
            .insert(entity, Body::new(Vec3::new(0.0, 12.0, 0.0)));
        for _ in 0..60 {
            scene.tick();
            assert!(scene.body(entity).position.y >= -1e-5);
        }
        let body = scene.body(entity);
        assert_eq!(body.position.y, 0.0);
        assert!(body.grounded && body.velocity.y == 0.0);
    }

    // --- 3. slopes --------------------------------------------------------

    #[test]
    fn a_slope_past_the_limit_is_not_climbed_but_traversed() {
        // A gradient of 1.2 is about 50 degrees — past the 40 degree limit.
        let mut scene = Scene::new(ramp(1.2));
        let walker = scene.walker(0.0, 0.0);
        assert!(
            scene
                .physics
                .support_at(0.0, 0.0, 0.0, 0.3)
                .is_steep(&scene.physics.tuning),
            "the test ramp has to actually be too steep"
        );

        // Pushing straight uphill and along the hill at once: the uphill half
        // is refused and the along-the-hill half is not.
        let start = scene.body(walker).position;
        scene.walk(walker, Vec3::new(1.0, 0.0, 1.0));
        scene.ticks(40);
        let body = scene.body(walker);
        assert!(
            body.position.x <= start.x + 1e-3,
            "the body climbed to x = {} from {}",
            body.position.x,
            start.x
        );
        assert!(
            body.position.z > start.z + 1.0,
            "but it should have travelled along the contour, z = {}",
            body.position.z
        );
    }

    #[test]
    fn a_body_left_alone_on_a_steep_slope_slides_down_it() {
        let mut scene = Scene::new(ramp(1.2));
        let walker = scene.walker(0.0, 0.0);
        let start = scene.body(walker).position;
        scene.ticks(20);
        let body = scene.body(walker);
        assert!(
            body.position.x < start.x - 0.5,
            "it should have slid downhill, x = {} from {}",
            body.position.x,
            start.x
        );
        assert!(
            body.position.y < start.y,
            "and downhill on this ramp is also downwards"
        );
        assert!(body.grounded, "sliding is not falling");
        assert!(
            body.velocity.x < -1.0,
            "and it picks up speed: {:?}",
            body.velocity
        );
    }

    #[test]
    fn a_slope_inside_the_limit_is_walked_up_normally() {
        // A gradient of 0.5 is about 27 degrees — well inside the limit.
        let mut scene = Scene::new(ramp(0.5));
        let walker = scene.walker(0.0, 0.0);
        let start = scene.body(walker).position;
        scene.walk(walker, Vec3::new(1.0, 0.0, 0.0));
        scene.ticks(40);
        let body = scene.body(walker);
        assert!(body.position.x > start.x + 2.0, "{:?}", body.position);
        assert!(body.position.y > start.y + 1.0, "and it gained height");
        assert!(
            (body.position.y
                - scene
                    .physics
                    .ground
                    .height_at(body.position.x, body.position.z))
            .abs()
                < 1e-4,
            "walking up a hill keeps the feet on it"
        );
        assert!(body.grounded, "with no airborne phase anywhere");
    }

    #[test]
    fn a_body_standing_on_a_gentle_slope_does_not_creep() {
        let mut scene = Scene::new(ramp(0.5));
        let walker = scene.walker(0.0, 0.0);
        let start = scene.body(walker).position;
        scene.ticks(40);
        assert_eq!(scene.body(walker).position, start, "gentle ground holds it");
    }

    // --- 4. steps and ledges ---------------------------------------------

    #[test]
    fn a_rise_under_the_step_height_is_walked_up_with_no_airborne_phase() {
        let mut scene = Scene::flat();
        // Wide enough that thirty ticks end on top of it rather than off the
        // far side — stepping back *down* is a separate question.
        scene.blocker(
            Vec3::new(6.0, 0.0, 0.0),
            Blocker::Cylinder {
                radius: 3.0,
                top: 0.3,
            },
        );
        let walker = scene.walker(0.0, 0.0);
        scene.walk(walker, Vec3::new(1.0, 0.0, 0.0));

        for tick in 0..30 {
            scene.tick();
            let body = scene.body(walker);
            assert!(
                body.grounded,
                "tick {tick}: a step is walked up, not jumped: {:?}",
                body.position
            );
            assert!(
                body.position.y == 0.0 || body.position.y == 0.3,
                "tick {tick}: the foot is on the ground or on the step, never between: {}",
                body.position.y
            );
        }
        let body = scene.body(walker);
        assert!(
            body.position.x > 3.0,
            "it walked on over: {:?}",
            body.position
        );
        assert_eq!(body.position.y, 0.3, "and it is standing on the step");
    }

    #[test]
    fn a_drop_under_the_step_height_is_walked_down_the_same_way() {
        // The mirror of walking up: a 0.3 m ledge is a step in both
        // directions, so leaving one is not a fall either.
        let mut scene = Scene::flat();
        scene.blocker(
            Vec3::new(0.0, 0.0, 0.0),
            Blocker::Cylinder {
                radius: 3.0,
                top: 0.3,
            },
        );
        let walker = scene.walker(0.0, 0.0);
        scene.tick();
        assert_eq!(scene.body(walker).position.y, 0.3, "standing on the ledge");

        scene.walk(walker, Vec3::new(1.0, 0.0, 0.0));
        for tick in 0..30 {
            scene.tick();
            let body = scene.body(walker);
            assert!(body.grounded, "tick {tick}: {:?}", body.position);
            assert!(
                body.position.y == 0.0 || body.position.y == 0.3,
                "tick {tick}: never between the two: {}",
                body.position.y
            );
        }
        assert_eq!(scene.body(walker).position.y, 0.0, "back on the ground");
    }

    #[test]
    fn a_drop_taller_than_a_step_is_still_a_fall() {
        let mut scene = Scene::flat();
        scene.blocker(
            Vec3::new(0.0, 0.0, 0.0),
            Blocker::Cylinder {
                radius: 3.0,
                top: 2.0,
            },
        );
        // Standing on top of the pillar — from up there its top is the floor.
        let entity = scene.world.spawn();
        scene
            .world
            .insert(entity, Body::new(Vec3::new(0.0, 2.0, 0.0)));
        scene.world.insert(entity, PendingInput::new());
        scene.tick();
        assert!(scene.body(entity).grounded && scene.body(entity).position.y == 2.0);

        scene.walk(entity, Vec3::new(1.0, 0.0, 0.0));
        let mut airborne = 0;
        for _ in 0..40 {
            scene.tick();
            if !scene.body(entity).grounded {
                airborne += 1;
            }
        }
        assert!(airborne > 0, "a two metre drop is not a step down");
        assert_eq!(scene.body(entity).position.y, 0.0, "and it lands");
    }

    #[test]
    fn a_rise_at_or_above_the_step_height_blocks() {
        let mut scene = Scene::flat();
        scene.blocker(
            Vec3::new(3.0, 0.0, 0.0),
            Blocker::Cylinder {
                radius: 1.0,
                top: 0.4,
            },
        );
        let walker = scene.walker(0.0, 0.0);
        scene.walk(walker, Vec3::new(1.0, 0.0, 0.0));
        scene.ticks(40);

        let body = scene.body(walker);
        assert!(
            body.position.x < 2.0 - 0.3 + 1e-3,
            "0.4 m is a wall, not a step: the body reached {:?}",
            body.position
        );
        assert_eq!(body.position.y, 0.0, "and it never got on top");
    }

    #[test]
    fn a_step_is_measured_from_the_foot_so_a_stair_can_be_climbed() {
        // Each tread is 0.3 above the last: from the ground the second one is
        // 0.6 up and unreachable, but from the first tread it is 0.3 and easy.
        let mut scene = Scene::flat();
        // Two treads and a landing, each 0.3 above the last.
        for (x, half_x, top) in [(3.0f32, 0.5f32, 0.3f32), (4.0, 0.5, 0.6), (9.0, 4.5, 0.9)] {
            scene.blocker(
                Vec3::new(x, 0.0, 0.0),
                Blocker::Box {
                    half: Vec2::new(half_x, 3.0),
                    facing: Vec2::new(1.0, 0.0),
                    top,
                },
            );
        }
        let walker = scene.walker(0.0, 0.0);
        scene.walk(walker, Vec3::new(1.0, 0.0, 0.0));
        for tick in 0..40 {
            scene.tick();
            assert!(
                scene.body(walker).grounded,
                "tick {tick}: stairs are walked up"
            );
        }
        let body = scene.body(walker);
        assert!(
            (body.position.y - 0.9).abs() < 1e-5,
            "the body should be on the top tread: {:?}",
            body.position
        );
    }

    #[test]
    fn a_body_that_starts_inside_a_wall_is_not_wedged_there_forever() {
        let mut scene = Scene::flat();
        scene.blocker(
            Vec3::new(0.0, 0.0, 0.0),
            Blocker::Cylinder {
                radius: 1.0,
                top: 3.0,
            },
        );
        let walker = scene.walker(0.2, 0.0);
        scene.ticks(20);
        let body = scene.body(walker);
        let distance = Vec2::new(body.position.x, body.position.z).length();
        assert!(
            distance >= 1.3 - 1e-3,
            "pushed clear of the wall: {distance}"
        );
    }

    // --- 6. jump -----------------------------------------------------------

    #[test]
    fn a_jump_peaks_at_about_sixty_centimetres_and_lasts_fourteen_ticks() {
        let mut scene = Scene::flat();
        let walker = scene.walker(0.0, 0.0);
        scene.jump(walker);

        let mut peak: f32 = 0.0;
        let mut airborne = 0;
        for _ in 0..40 {
            scene.tick();
            let body = scene.body(walker);
            peak = peak.max(body.position.y);
            if !body.grounded {
                airborne += 1;
            }
        }
        assert!(
            (peak - 0.6).abs() < 0.01,
            "the design says 0.6 m, got {peak}"
        );
        assert_eq!(airborne, 14, "0.7 s of flight is fourteen ticks at 20 Hz");
        assert!(scene.body(walker).grounded, "and then it is down again");
    }

    #[test]
    fn jumped_is_set_on_the_tick_of_take_off_and_cleared_on_the_next() {
        let mut scene = Scene::flat();
        let walker = scene.walker(0.0, 0.0);
        assert!(!scene.body(walker).jumped);

        scene.jump(walker);
        scene.tick();
        assert!(scene.body(walker).jumped, "the take-off tick");
        assert!(!scene.body(walker).grounded);

        scene.tick();
        assert!(!scene.body(walker).jumped, "and exactly that one tick");
    }

    #[test]
    fn a_jump_asked_for_in_mid_air_is_spent_and_does_not_double_the_height() {
        let mut scene = Scene::flat();
        let walker = scene.walker(0.0, 0.0);
        scene.jump(walker);
        scene.ticks(4);
        assert!(!scene.body(walker).grounded);

        let before = scene.body(walker).velocity.y;
        scene.jump(walker);
        scene.tick();
        assert!(
            scene.body(walker).velocity.y < before,
            "the second jump had no ground to push off"
        );
        assert!(!scene.body(walker).jumped);
    }

    // --- 2. collisions -----------------------------------------------------

    #[test]
    fn two_overlapping_bodies_are_pushed_apart() {
        let mut scene = Scene::flat();
        let a = scene.walker(0.0, 0.0);
        let b = scene.walker(0.3, 0.0);
        scene.tick();

        let (pa, pb) = (scene.body(a).position, scene.body(b).position);
        let gap = Vec2::new(pb.x - pa.x, pb.z - pa.z).length();
        assert!((gap - 0.6).abs() < 1e-4, "exactly touching, got {gap}");
        // Each moved half the overlap — neither is privileged.
        assert!((pa.x + pb.x - 0.3).abs() < 1e-4, "{pa:?} {pb:?}");
    }

    #[test]
    fn a_body_walking_into_another_slides_around_it_rather_than_stopping() {
        let mut scene = Scene::flat();
        let walker = scene.walker(0.0, 0.0);
        // Dead ahead, but a little to one side — which is what makes it a
        // slide and not a stop.
        let obstacle = scene.walker(2.0, 0.25);
        scene.walk(walker, Vec3::new(1.0, 0.0, 0.0));
        scene.ticks(30);

        let body = scene.body(walker);
        assert!(
            body.position.x > 2.5,
            "it should have got past: {:?}",
            body.position
        );
        assert!(
            body.position.z < -0.2,
            "by going round the low side: {:?}",
            body.position
        );
        // The obstacle was shoved a little, which is what body-body contact is.
        assert!(scene.body(obstacle).position.z > 0.25);
    }

    #[test]
    fn a_body_walking_into_a_wall_slides_along_it() {
        let mut scene = Scene::flat();
        // A wall across Z at x = 2, thin along X.
        scene.blocker(
            Vec3::new(2.0, 0.0, 0.0),
            Blocker::Box {
                half: Vec2::new(0.2, 8.0),
                facing: Vec2::new(1.0, 0.0),
                top: 3.0,
            },
        );
        let walker = scene.walker(0.0, 0.0);
        scene.walk(walker, Vec3::new(1.0, 0.0, 1.0));
        scene.ticks(40);

        let body = scene.body(walker);
        assert!(body.position.x < 1.81, "the wall held: {:?}", body.position);
        assert!(
            body.position.z > 3.0,
            "but the body kept moving along it rather than sticking: {:?}",
            body.position
        );
    }

    // --- the falling tree --------------------------------------------------

    /// A tree standing at the origin, ready to be felled.
    fn plant_a_tree(scene: &mut Scene, at: Vec3) -> Entity {
        let entity = scene.blocker(
            at,
            Blocker::Cylinder {
                radius: 0.35,
                top: 6.0,
            },
        );
        scene.world.insert(entity, FallingTree::new(6.0, 0.35));
        entity
    }

    #[test]
    fn a_falling_trunk_shoves_a_body_out_of_the_arc_without_hurting_it() {
        let mut scene = Scene::flat();
        let tree = plant_a_tree(&mut scene, Vec3::ZERO);
        let victim = scene.walker(3.0, 0.0);
        scene.tick();
        let start = scene.body(victim).position;

        scene
            .world
            .get_mut::<FallingTree>(tree)
            .expect("planted")
            .topple(Vec2::new(1.0, 0.0));

        let mut fastest: f32 = 0.0;
        for _ in 0..40 {
            scene.tick();
            fastest = fastest.max(scene.body(victim).velocity.x);
        }
        assert!(
            (fastest - 3.0).abs() < 0.2,
            "the trunk shoves at about 3 m/s, got {fastest}"
        );

        let moved = scene.body(victim).position.x - start.x;
        assert!(
            (1.0..3.0).contains(&moved),
            "knocked roughly a metre and a half clear, got {moved}"
        );
        // There is no damage in this card at all, and nothing about a body
        // records any: it is simply somewhere else.
        assert_eq!(scene.body(victim).kind, BodyKind::Character);
    }

    #[test]
    fn a_body_out_of_the_arc_is_left_alone() {
        let mut scene = Scene::flat();
        let tree = plant_a_tree(&mut scene, Vec3::ZERO);
        // Well to the side of the swath the trunk sweeps.
        let bystander = scene.walker(0.0, 4.0);
        scene
            .world
            .get_mut::<FallingTree>(tree)
            .expect("planted")
            .topple(Vec2::new(1.0, 0.0));
        scene.ticks(40);
        assert_eq!(scene.body(bystander).position, Vec3::new(0.0, 0.0, 4.0));
    }

    #[test]
    fn a_landed_trunk_trades_its_standing_blocker_for_a_log() {
        let mut scene = Scene::flat();
        let tree = plant_a_tree(&mut scene, Vec3::ZERO);
        scene.tick();
        assert_eq!(scene.physics.statics().len(), 1);
        assert!(scene.world.has::<Blocker>(tree));

        scene
            .world
            .get_mut::<FallingTree>(tree)
            .expect("planted")
            .topple(Vec2::new(1.0, 0.0));
        // The fall is thirty ticks, a second and a half.
        scene.ticks(31);

        assert!(
            !scene.world.has::<Blocker>(tree),
            "the standing trunk is gone"
        );
        assert!(scene.world.get::<FallingTree>(tree).unwrap().fallen);
        assert_eq!(scene.physics.statics().len(), 1, "and a log replaced it");
        let log = scene.physics.statics().shapes()[0];
        assert_eq!(log.top(), LOG_TOP);
        match log.blocker {
            Blocker::Box { half, facing, top } => {
                assert_eq!(half, Vec2::new(3.0, 0.35), "half the trunk, each way");
                assert_eq!(facing, Vec2::new(1.0, 0.0), "lying where it fell");
                assert_eq!(top, LOG_TOP);
            }
            other => panic!("a felled tree is a box, got {other:?}"),
        }
        // Its centre is halfway between the stump and where the tip landed.
        assert!((log.base.x - 3.0).abs() < 1e-5, "{:?}", log.base);
    }

    #[test]
    fn a_log_can_be_walked_over_where_the_tree_could_not_be_walked_through() {
        let mut scene = Scene::flat();
        let tree = plant_a_tree(&mut scene, Vec3::new(4.0, 0.0, 0.0));

        // Standing, the trunk is a wall.
        let walker = scene.walker(0.0, 0.0);
        scene.walk(walker, Vec3::new(1.0, 0.0, 0.0));
        scene.ticks(40);
        assert!(
            scene.body(walker).position.x < 3.4,
            "a standing tree stops a body: {:?}",
            scene.body(walker).position
        );

        // Felled across the path, it is a 0.3 m step instead.
        scene
            .world
            .get_mut::<FallingTree>(tree)
            .expect("planted")
            .topple(Vec2::new(0.0, 1.0));
        scene.ticks(31);
        scene.walk(walker, Vec3::new(1.0, 0.0, 0.0));
        scene.ticks(40);
        let body = scene.body(walker);
        assert!(
            body.position.x > 5.0,
            "with the tree down the way is open: {:?}",
            body.position
        );
        assert!(body.grounded, "and it was walked, not jumped");
    }
}
