//! The simulation loop that ties shapes, bodies, collision and the solver
//! together.
//!
//! One `step` is:
//!
//! 1. **Integrate velocities.** Gravity and accumulated forces become motion.
//! 2. **Broad phase.** Sweep and prune narrows "every pair" down to the ones
//!    whose bounds overlap.
//! 3. **Narrow phase.** Exact tests produce contact manifolds, and anything
//!    asleep that a moving body touches is woken.
//! 4. **Solve.** Warm-started sequential impulses fix velocities, then a
//!    separate positional pass pushes overlap out without handing the bodies
//!    energy they would keep.
//! 5. **Integrate positions**, put still bodies to sleep, and hand this step's
//!    impulses to the next one.
//!
//! The order is fixed and every intermediate list is sorted, so the same
//! inputs produce bit-identical output on every run. That is not a nicety: a
//! world that is simulated on two machines at once, or replayed from a save,
//! has to land in the same place both times.

use runity_math::Vec3;

use crate::body::{BodyType, RigidBody};
use crate::broadphase::{BroadPhase, Proxy};
use crate::collide::collide;
use crate::raycast::{ray_shape, Ray, RayHit};
use crate::shape::Aabb;
use crate::solver::{ContactSolver, ImpulseCache, PairManifold, SolverSettings};

/// A handle to a body. Removing a body bumps its slot's generation, so a
/// stale handle cannot reach whatever took its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BodyHandle {
    index: u32,
    generation: u32,
}

impl BodyHandle {
    /// Slot number. Dense and stable while the body lives.
    pub fn index(self) -> u32 {
        self.index
    }

    /// How many times this slot has been reused.
    pub fn generation(self) -> u32 {
        self.generation
    }
}

/// Something that started or stopped touching, reported once per step.
///
/// Gameplay hangs off these: a footstep, a crate breaking, a monster
/// connecting with a villager. Polling for overlap instead would miss the
/// contacts that begin and end inside one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactEvent {
    /// The two bodies were apart last step and are touching now.
    Started(BodyHandle, BodyHandle),
    /// They were touching last step and are not any more.
    Ended(BodyHandle, BodyHandle),
}

impl ContactEvent {
    /// The two bodies involved, lower slot first.
    pub fn bodies(&self) -> (BodyHandle, BodyHandle) {
        match self {
            ContactEvent::Started(a, b) | ContactEvent::Ended(a, b) => (*a, *b),
        }
    }

    /// Whether `handle` is one of them.
    pub fn involves(&self, handle: BodyHandle) -> bool {
        let (a, b) = self.bodies();
        a == handle || b == handle
    }
}

/// What a ray found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayCast {
    /// Which body was hit.
    pub handle: BodyHandle,
    /// Where, how far, and which way the surface faces.
    pub hit: RayHit,
}

/// What the last step cost — for a debug overlay and for noticing that a
/// scene has quietly become too expensive.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PhysicsStats {
    /// Bodies in the world.
    pub bodies: usize,
    /// Bodies that are awake and dynamic.
    pub awake: usize,
    /// Pairs the broad phase produced.
    pub pairs: usize,
    /// Pairs that turned out to actually touch.
    pub manifolds: usize,
    /// Contact points across those manifolds.
    pub contacts: usize,
    /// Largest normal impulse applied, a cheap measure of impact force.
    pub max_impulse: f32,
}

/// A world of rigid bodies.
pub struct PhysicsWorld {
    bodies: Vec<RigidBody>,
    alive: Vec<bool>,
    generations: Vec<u32>,
    free: Vec<u32>,
    /// Acceleration applied to every dynamic body, scaled per body by
    /// [`RigidBody::gravity_scale`].
    pub gravity: Vec3,
    /// Iteration counts and tolerances for the contact solver.
    pub settings: SolverSettings,
    broad: BroadPhase,
    solver: ContactSolver,
    cache: ImpulseCache,
    pairs: Vec<(usize, usize)>,
    proxies: Vec<Proxy>,
    manifolds: Vec<PairManifold>,
    /// Pairs that were touching at the end of the previous step, sorted.
    touching: Vec<(u32, u32)>,
    live: Vec<(u32, u32)>,
    events: Vec<ContactEvent>,
    stats: PhysicsStats,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsWorld {
    /// Earth gravity, in metres per second squared.
    pub const EARTH_GRAVITY: Vec3 = Vec3 {
        x: 0.0,
        y: -9.81,
        z: 0.0,
    };

    /// An empty world under Earth gravity.
    pub fn new() -> Self {
        Self {
            bodies: Vec::new(),
            alive: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
            gravity: Self::EARTH_GRAVITY,
            settings: SolverSettings::default(),
            broad: BroadPhase::new(),
            solver: ContactSolver::new(),
            cache: ImpulseCache::new(),
            pairs: Vec::new(),
            proxies: Vec::new(),
            manifolds: Vec::new(),
            touching: Vec::new(),
            live: Vec::new(),
            events: Vec::new(),
            stats: PhysicsStats::default(),
        }
    }

    /// An empty world with a gravity of your choosing.
    pub fn with_gravity(gravity: Vec3) -> Self {
        Self {
            gravity,
            ..Self::new()
        }
    }

    // ----------------------------------------------------------- membership

    /// Add a body and get a handle to it.
    pub fn add(&mut self, body: RigidBody) -> BodyHandle {
        match self.free.pop() {
            Some(index) => {
                let slot = index as usize;
                self.bodies[slot] = body;
                self.alive[slot] = true;
                BodyHandle {
                    index,
                    generation: self.generations[slot],
                }
            }
            None => {
                let index = self.bodies.len() as u32;
                self.bodies.push(body);
                self.alive.push(true);
                self.generations.push(0);
                BodyHandle {
                    index,
                    generation: 0,
                }
            }
        }
    }

    /// Remove a body, returning it. A stale handle returns `None`.
    pub fn remove(&mut self, handle: BodyHandle) -> Option<RigidBody> {
        if !self.contains(handle) {
            return None;
        }
        let slot = handle.index as usize;
        self.alive[slot] = false;
        self.generations[slot] = self.generations[slot].wrapping_add(1);
        self.free.push(handle.index);
        // The slot keeps a placeholder so body indices stay stable for the
        // solver; it is excluded from every query and from the broad phase.
        let placeholder = RigidBody::fixed(crate::shape::Shape::sphere(0.0));
        let removed = core::mem::replace(&mut self.bodies[slot], placeholder);
        // Contacts involving this slot must not warm-start whatever is added
        // next.
        self.touching
            .retain(|(a, b)| *a != handle.index && *b != handle.index);
        Some(removed)
    }

    /// Whether the handle still refers to a living body.
    pub fn contains(&self, handle: BodyHandle) -> bool {
        let slot = handle.index as usize;
        self.alive.get(slot).copied().unwrap_or(false)
            && self.generations[slot] == handle.generation
    }

    /// Borrow a body.
    pub fn get(&self, handle: BodyHandle) -> Option<&RigidBody> {
        self.contains(handle)
            .then(|| &self.bodies[handle.index as usize])
    }

    /// Borrow a body exclusively.
    ///
    /// Moving a sleeping body this way will not wake it — call
    /// [`PhysicsWorld::wake`] if you need it simulated again. The impulse and
    /// force methods on [`RigidBody`] wake it themselves.
    pub fn get_mut(&mut self, handle: BodyHandle) -> Option<&mut RigidBody> {
        self.contains(handle)
            .then(|| &mut self.bodies[handle.index as usize])
    }

    /// Wake a body, if the handle is live.
    pub fn wake(&mut self, handle: BodyHandle) {
        if let Some(body) = self.get_mut(handle) {
            body.wake();
        }
    }

    /// Wake everything. Useful after changing gravity.
    pub fn wake_all(&mut self) {
        for (index, body) in self.bodies.iter_mut().enumerate() {
            if self.alive[index] {
                body.wake();
            }
        }
    }

    /// How many bodies the world holds.
    pub fn len(&self) -> usize {
        self.alive.iter().filter(|a| **a).count()
    }

    /// Whether the world is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every body, with its handle.
    pub fn iter(&self) -> impl Iterator<Item = (BodyHandle, &RigidBody)> + '_ {
        self.bodies
            .iter()
            .enumerate()
            .filter(|(index, _)| self.alive[*index])
            .map(|(index, body)| (self.handle(index), body))
    }

    /// Every body, mutably.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (BodyHandle, &mut RigidBody)> + '_ {
        let alive = &self.alive;
        let generations = &self.generations;
        self.bodies
            .iter_mut()
            .enumerate()
            .filter(move |(index, _)| alive[*index])
            .map(move |(index, body)| {
                (
                    BodyHandle {
                        index: index as u32,
                        generation: generations[index],
                    },
                    body,
                )
            })
    }

    fn handle(&self, index: usize) -> BodyHandle {
        BodyHandle {
            index: index as u32,
            generation: self.generations[index],
        }
    }

    // ----------------------------------------------------------------- step

    /// Advance the world by `dt` seconds.
    ///
    /// Call it with a fixed `dt`. A variable step makes contacts jitter and
    /// makes the result depend on frame rate, which is exactly what a
    /// simulation shared between machines cannot afford.
    pub fn step(&mut self, dt: f32) {
        self.events.clear();
        // A zero, negative or NaN step is a no-op rather than a way to run
        // the world backwards or fill it with NaNs.
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }

        self.integrate_velocities(dt);
        self.find_pairs();
        self.build_manifolds();
        self.report_contacts();

        self.cache.retain_pairs(&self.live);
        self.solver
            .prepare(&self.bodies, &self.manifolds, &self.cache, &self.settings);
        if self.settings.warm_starting {
            self.solver.warm_start(&mut self.bodies);
        }
        self.solver.solve_velocity(&mut self.bodies, &self.settings);
        self.solver.solve_position(&self.bodies, &self.settings, dt);
        self.integrate_positions(dt);
        self.solver.store_impulses(&mut self.cache);

        self.stats = PhysicsStats {
            bodies: self.len(),
            awake: self
                .iter()
                .filter(|(_, b)| b.is_dynamic() && !b.is_sleeping())
                .count(),
            pairs: self.pairs.len(),
            manifolds: self.manifolds.len(),
            contacts: self.solver.contact_count(),
            max_impulse: self.solver.max_impulse(),
        };
    }

    fn integrate_velocities(&mut self, dt: f32) {
        let gravity = self.gravity;
        for (index, body) in self.bodies.iter_mut().enumerate() {
            if !self.alive[index] {
                continue;
            }
            body.integrate_velocity(gravity, dt);
            body.clear_forces();
        }
    }

    fn find_pairs(&mut self) {
        self.proxies.clear();
        for (index, body) in self.bodies.iter().enumerate() {
            if !self.alive[index] {
                continue;
            }
            self.proxies.push(Proxy {
                index,
                aabb: body.aabb(),
                // Inert means "not going anywhere by itself this step": a
                // static body or a sleeping one. A kinematic body never
                // qualifies, however still it looks — the game may be about
                // to drive it straight through something.
                inert: body.body_type == BodyType::Static || body.is_sleeping(),
            });
        }
        self.broad.find_pairs(&self.proxies, &mut self.pairs);
    }

    fn build_manifolds(&mut self) {
        self.manifolds.clear();
        self.live.clear();
        let mut to_wake: Vec<usize> = Vec::new();

        for index in 0..self.pairs.len() {
            let (a, b) = self.pairs[index];
            let key = (a as u32, b as u32);
            let manifold = {
                let body_a = &self.bodies[a];
                let body_b = &self.bodies[b];
                if body_a.inverse_mass() + body_b.inverse_mass() == 0.0 {
                    continue;
                }
                collide(
                    &body_a.shape,
                    &body_a.transform,
                    &body_b.shape,
                    &body_b.transform,
                )
            };
            let Some(manifold) = manifold else {
                continue;
            };

            // Anything the broad phase handed us has at least one awake body,
            // so a sleeper in this pair has just been disturbed.
            if self.bodies[a].is_sleeping() {
                to_wake.push(a);
            }
            if self.bodies[b].is_sleeping() {
                to_wake.push(b);
            }

            self.manifolds.push(PairManifold {
                a,
                b,
                key,
                manifold,
            });
            self.live.push(key);
        }

        for index in to_wake {
            self.bodies[index].wake();
        }
    }

    /// Diff this step's touching pairs against the previous step's.
    ///
    /// Both lists are sorted, so one merge walk finds what appeared and what
    /// disappeared. A pair that vanished only because both bodies went to
    /// sleep is carried over rather than reported as ended: a crate resting
    /// on the floor has not stopped touching it just because neither of them
    /// is being simulated any more.
    fn report_contacts(&mut self) {
        let mut next: Vec<(u32, u32)> = Vec::with_capacity(self.live.len());
        let (mut current, mut previous) = (0, 0);
        loop {
            let now = self.live.get(current).copied();
            let before = self.touching.get(previous).copied();
            match (now, before) {
                (Some(a), Some(b)) if a == b => {
                    next.push(a);
                    current += 1;
                    previous += 1;
                }
                (Some(a), Some(b)) if a < b => {
                    self.events.push(self.started(a));
                    next.push(a);
                    current += 1;
                }
                (Some(_), Some(b)) => {
                    self.retire(b, &mut next);
                    previous += 1;
                }
                (Some(a), None) => {
                    self.events.push(self.started(a));
                    next.push(a);
                    current += 1;
                }
                (None, Some(b)) => {
                    self.retire(b, &mut next);
                    previous += 1;
                }
                (None, None) => break,
            }
        }
        self.touching = next;
    }

    /// A pair the broad phase did not produce this step: either both bodies
    /// have gone dormant and it still holds, or it really has ended.
    fn retire(&mut self, key: (u32, u32), next: &mut Vec<(u32, u32)>) {
        if self.is_dormant(key.0) && self.is_dormant(key.1) {
            next.push(key);
        } else {
            self.events.push(self.ended(key));
        }
    }

    fn is_dormant(&self, index: u32) -> bool {
        let slot = index as usize;
        if !self.alive.get(slot).copied().unwrap_or(false) {
            return false;
        }
        let body = &self.bodies[slot];
        body.body_type == BodyType::Static || body.is_sleeping()
    }

    fn started(&self, key: (u32, u32)) -> ContactEvent {
        ContactEvent::Started(self.handle(key.0 as usize), self.handle(key.1 as usize))
    }

    fn ended(&self, key: (u32, u32)) -> ContactEvent {
        ContactEvent::Ended(self.handle(key.0 as usize), self.handle(key.1 as usize))
    }

    fn integrate_positions(&mut self, dt: f32) {
        for index in 0..self.bodies.len() {
            if !self.alive[index] {
                continue;
            }
            // The positional pass produced a velocity that exists only for
            // this integration: add it, move, take it back off, so the body
            // does not keep the energy used to separate it.
            let (linear, angular) = self.solver.pseudo_velocity(index);
            let body = &mut self.bodies[index];
            body.linear_velocity += linear;
            body.angular_velocity += angular;
            body.integrate_position(dt);
            body.linear_velocity -= linear;
            body.angular_velocity -= angular;
            body.update_sleep(dt);
        }
    }

    // -------------------------------------------------------------- queries

    /// Contacts that began or ended during the last step.
    pub fn contacts(&self) -> &[ContactEvent] {
        &self.events
    }

    /// Whether two bodies are touching right now.
    pub fn are_touching(&self, a: BodyHandle, b: BodyHandle) -> bool {
        let key = if a.index < b.index {
            (a.index, b.index)
        } else {
            (b.index, a.index)
        };
        self.contains(a) && self.contains(b) && self.touching.binary_search(&key).is_ok()
    }

    /// What the last step cost.
    pub fn stats(&self) -> PhysicsStats {
        self.stats
    }

    /// The nearest body along a ray.
    pub fn cast_ray(&self, ray: &Ray) -> Option<RayCast> {
        self.cast_ray_filtered(ray, |_, _| true)
    }

    /// The nearest body along a ray that the filter accepts.
    ///
    /// The filter is how a player's own capsule stays out of the way of the
    /// ray that decides what they are looking at.
    pub fn cast_ray_filtered(
        &self,
        ray: &Ray,
        accept: impl Fn(BodyHandle, &RigidBody) -> bool,
    ) -> Option<RayCast> {
        let mut nearest: Option<RayCast> = None;
        for (handle, body) in self.iter() {
            if !accept(handle, body) {
                continue;
            }
            let Some(hit) = ray_shape(ray, &body.shape, &body.transform) else {
                continue;
            };
            // Ties go to the lower slot, so a ray that grazes two coincident
            // bodies always reports the same one.
            let better = match &nearest {
                None => true,
                Some(current) => hit.distance < current.hit.distance,
            };
            if better {
                nearest = Some(RayCast { handle, hit });
            }
        }
        nearest
    }

    /// Every body whose bounds overlap `region`.
    ///
    /// Bounds, not shapes: this is the cheap query for "what is around here",
    /// meant for gameplay ranges rather than exact overlap.
    pub fn overlapping(&self, region: Aabb) -> impl Iterator<Item = (BodyHandle, &RigidBody)> + '_ {
        self.iter()
            .filter(move |(_, body)| body.aabb().overlaps(&region))
    }

    /// Forget every cached contact impulse.
    ///
    /// After teleporting bodies or rebuilding a scene, last step's impulses
    /// describe a world that no longer exists.
    pub fn reset_contacts(&mut self) {
        self.cache.clear();
        self.touching.clear();
        self.events.clear();
    }
}

impl core::fmt::Debug for PhysicsWorld {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PhysicsWorld")
            .field("bodies", &self.len())
            .field("gravity", &self.gravity)
            .field("stats", &self.stats)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{BodyType, Material};
    use crate::shape::Shape;
    use runity_math::Vec3;

    const STEP: f32 = 1.0 / 60.0;

    fn ground(world: &mut PhysicsWorld) -> BodyHandle {
        world.add(RigidBody::fixed(Shape::ground()))
    }

    fn run(world: &mut PhysicsWorld, seconds: f32) {
        let steps = (seconds / STEP).round() as usize;
        for _ in 0..steps {
            world.step(STEP);
        }
    }

    #[test]
    fn a_ball_falls_and_comes_to_rest_on_the_ground() {
        let mut world = PhysicsWorld::new();
        ground(&mut world);
        let ball = world
            .add(RigidBody::dynamic(Shape::sphere(0.5)).with_position(Vec3::new(0.0, 5.0, 0.0)));

        run(&mut world, 3.0);

        let body = world.get(ball).unwrap();
        // Resting on a half-space means the centre sits exactly one radius up,
        // give or take the solver's allowed overlap.
        assert!(
            (body.position().y - 0.5).abs() < 0.02,
            "{}",
            body.position().y
        );
        assert!(
            body.linear_velocity.length() < 0.05,
            "{:?}",
            body.linear_velocity
        );
    }

    #[test]
    fn a_resting_body_falls_asleep_and_stays_where_it_is() {
        let mut world = PhysicsWorld::new();
        ground(&mut world);
        let ball = world
            .add(RigidBody::dynamic(Shape::sphere(0.5)).with_position(Vec3::new(0.0, 1.0, 0.0)));

        run(&mut world, 4.0);
        assert!(
            world.get(ball).unwrap().is_sleeping(),
            "a still body should stop costing anything"
        );

        let resting = world.get(ball).unwrap().position();
        run(&mut world, 4.0);
        assert_eq!(
            world.get(ball).unwrap().position(),
            resting,
            "and not drift while asleep"
        );
        assert_eq!(world.stats().awake, 0);
    }

    #[test]
    fn a_sleeping_body_wakes_when_something_lands_on_it() {
        let mut world = PhysicsWorld::new();
        ground(&mut world);
        let bottom = world.add(
            RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5)))
                .with_position(Vec3::new(0.0, 0.5, 0.0)),
        );
        run(&mut world, 4.0);
        assert!(world.get(bottom).unwrap().is_sleeping());

        world.add(
            RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5)))
                .with_position(Vec3::new(0.0, 4.0, 0.0)),
        );
        run(&mut world, 1.5);
        assert!(
            !world.get(bottom).unwrap().is_sleeping(),
            "being landed on is a disturbance"
        );
    }

    #[test]
    fn a_stack_settles_without_sinking_into_itself() {
        let mut world = PhysicsWorld::new();
        ground(&mut world);
        let mut boxes = Vec::new();
        for level in 0..4 {
            boxes.push(world.add(
                RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5))).with_position(Vec3::new(
                    0.0,
                    0.5 + level as f32 * 1.02,
                    0.0,
                )),
            ));
        }

        run(&mut world, 5.0);

        for (level, handle) in boxes.iter().enumerate() {
            let expected = 0.5 + level as f32;
            let actual = world.get(*handle).unwrap().position().y;
            assert!(
                (actual - expected).abs() < 0.08,
                "level {level} rested at {actual}"
            );
        }
    }

    #[test]
    fn a_bouncy_ball_bounces_and_a_dead_one_does_not() {
        let drop_height = |restitution: f32| {
            let mut world = PhysicsWorld::new();
            ground(&mut world);
            let ball = world.add(
                RigidBody::dynamic(Shape::sphere(0.5))
                    .with_position(Vec3::new(0.0, 3.0, 0.0))
                    .with_material(Material {
                        restitution,
                        ..Material::default()
                    }),
            );
            // Fall, hit, and watch how high it comes back.
            let mut peak: f32 = 0.0;
            let mut hit = false;
            for _ in 0..240 {
                world.step(STEP);
                let body = world.get(ball).unwrap();
                if body.position().y < 0.6 {
                    hit = true;
                }
                if hit {
                    peak = peak.max(body.position().y);
                }
            }
            peak
        };

        let bouncy = drop_height(0.8);
        let dead = drop_height(0.0);
        assert!(
            bouncy > 1.2,
            "a bouncy ball should come back up, reached {bouncy}"
        );
        assert!(dead < 0.7, "a dead ball should stay down, reached {dead}");
    }

    #[test]
    fn friction_stops_a_sliding_box_and_slippery_ones_keep_going() {
        let slide = |friction: f32| {
            let mut world = PhysicsWorld::new();
            ground(&mut world);
            let body = world.add(
                RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5)))
                    .with_position(Vec3::new(0.0, 0.5, 0.0))
                    .with_velocity(Vec3::new(6.0, 0.0, 0.0))
                    .with_material(Material {
                        friction,
                        restitution: 0.0,
                    }),
            );
            run(&mut world, 2.0);
            world.get(body).unwrap().position().x
        };

        let rough = slide(0.9);
        let slippery = slide(0.0);
        assert!(
            rough < slippery * 0.6,
            "friction {rough} vs frictionless {slippery}"
        );
    }

    #[test]
    fn the_same_scene_simulates_identically_twice() {
        // Determinism is the property every other guarantee rests on: saves
        // that replay, tests that stay valid, machines that agree.
        let build = || {
            let mut world = PhysicsWorld::new();
            world.add(RigidBody::fixed(Shape::ground()));
            for i in 0..12 {
                let x = (i % 3) as f32 * 0.7 - 0.7;
                let z = (i % 4) as f32 * 0.6 - 0.9;
                world.add(
                    RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.4)))
                        .with_position(Vec3::new(x, 1.0 + i as f32 * 0.9, z))
                        .with_angular_velocity(Vec3::new(0.3, -0.2, 0.1)),
                );
            }
            world
        };

        let mut first = build();
        let mut second = build();
        run(&mut first, 4.0);
        run(&mut second, 4.0);

        for ((_, a), (_, b)) in first.iter().zip(second.iter()) {
            assert_eq!(
                a.position(),
                b.position(),
                "positions must match bit for bit"
            );
            assert_eq!(a.rotation(), b.rotation());
            assert_eq!(a.linear_velocity, b.linear_velocity);
        }
    }

    #[test]
    fn nothing_gains_energy_out_of_nowhere() {
        // A stack that is not being pushed must never end up higher than it
        // started; contact solvers that get this wrong turn into fireworks.
        let mut world = PhysicsWorld::new();
        ground(&mut world);
        let mut handles = Vec::new();
        for level in 0..5 {
            handles.push(world.add(
                RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5))).with_position(Vec3::new(
                    0.0,
                    0.5 + level as f32 * 1.01,
                    0.0,
                )),
            ));
        }
        let start: f32 = handles
            .iter()
            .map(|h| world.get(*h).unwrap().position().y)
            .sum();

        run(&mut world, 6.0);

        let end: f32 = handles
            .iter()
            .map(|h| world.get(*h).unwrap().position().y)
            .sum();
        assert!(end <= start + 0.05, "the stack rose from {start} to {end}");
        for handle in &handles {
            let body = world.get(*handle).unwrap();
            assert!(
                body.position().length() < 10.0,
                "a box flew off to {:?}",
                body.position()
            );
        }
    }

    #[test]
    fn contacts_are_reported_when_they_begin_and_when_they_end() {
        let mut world = PhysicsWorld::new();
        let floor = ground(&mut world);
        let ball = world.add(
            RigidBody::dynamic(Shape::sphere(0.5))
                .with_position(Vec3::new(0.0, 2.0, 0.0))
                .with_material(Material {
                    restitution: 0.0,
                    friction: 0.5,
                }),
        );

        let mut started = 0;
        let mut ended = 0;
        for _ in 0..120 {
            world.step(STEP);
            for event in world.contacts() {
                assert!(event.involves(ball) && event.involves(floor));
                match event {
                    ContactEvent::Started(..) => started += 1,
                    ContactEvent::Ended(..) => ended += 1,
                }
            }
        }
        assert_eq!(started, 1, "landing once means one Started");
        assert_eq!(ended, 0, "and it never leaves the ground again");
        assert!(world.are_touching(ball, floor));

        // Kick it away and the contact ends.
        world
            .get_mut(ball)
            .unwrap()
            .apply_impulse(Vec3::new(0.0, 12.0, 0.0));
        let mut ended_after_kick = 0;
        for _ in 0..10 {
            world.step(STEP);
            ended_after_kick += world
                .contacts()
                .iter()
                .filter(|e| matches!(e, ContactEvent::Ended(..)))
                .count();
        }
        assert_eq!(ended_after_kick, 1);
        assert!(!world.are_touching(ball, floor));
    }

    #[test]
    fn a_ray_finds_the_nearest_body_and_the_filter_skips_it() {
        let mut world = PhysicsWorld::new();
        let near = world
            .add(RigidBody::fixed(Shape::sphere(0.5)).with_position(Vec3::new(0.0, 0.0, -2.0)));
        let far = world
            .add(RigidBody::fixed(Shape::sphere(0.5)).with_position(Vec3::new(0.0, 0.0, -6.0)));

        let ray = Ray::new(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), 20.0);
        let hit = world.cast_ray(&ray).expect("the ray should hit something");
        assert_eq!(hit.handle, near);
        assert!(
            (hit.hit.distance - 1.5).abs() < 1e-4,
            "{}",
            hit.hit.distance
        );

        // A player's own body is exactly what a filter is for.
        let behind = world
            .cast_ray_filtered(&ray, |handle, _| handle != near)
            .unwrap();
        assert_eq!(behind.handle, far);

        let away = Ray::new(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 20.0);
        assert!(world.cast_ray(&away).is_none());
    }

    #[test]
    fn removing_a_body_invalidates_its_handle_and_frees_the_slot() {
        let mut world = PhysicsWorld::new();
        let first = world.add(RigidBody::dynamic(Shape::sphere(0.5)));
        assert_eq!(world.len(), 1);

        let returned = world.remove(first).expect("the body should come back");
        assert!(matches!(returned.shape, Shape::Sphere { .. }));
        assert!(!world.contains(first));
        assert!(world.get(first).is_none());
        assert!(world.remove(first).is_none(), "removing twice is a no-op");
        assert!(world.is_empty());

        let second = world.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5))));
        assert_eq!(second.index(), first.index(), "the slot is reused");
        assert_ne!(second.generation(), first.generation());
        assert!(
            world.get(first).is_none(),
            "the old handle still sees nothing"
        );
        assert!(world.get(second).is_some());
    }

    #[test]
    fn a_removed_body_stops_affecting_the_simulation() {
        let mut world = PhysicsWorld::new();
        ground(&mut world);
        let blocker = world.add(
            RigidBody::fixed(Shape::cuboid(Vec3::new(2.0, 0.5, 2.0)))
                .with_position(Vec3::new(0.0, 2.0, 0.0)),
        );
        let ball = world
            .add(RigidBody::dynamic(Shape::sphere(0.4)).with_position(Vec3::new(0.0, 4.0, 0.0)));

        run(&mut world, 1.5);
        let resting_on_blocker = world.get(ball).unwrap().position().y;
        assert!(
            resting_on_blocker > 2.0,
            "the ball should be held up, was at {resting_on_blocker}"
        );

        world.remove(blocker);
        world.wake(ball);
        run(&mut world, 3.0);
        assert!(
            world.get(ball).unwrap().position().y < 0.6,
            "with the blocker gone it should fall"
        );
    }

    #[test]
    fn static_bodies_never_move_and_kinematic_ones_push() {
        let mut world = PhysicsWorld::new();
        let wall = world.add(
            RigidBody::fixed(Shape::cuboid(Vec3::new(0.5, 2.0, 2.0)))
                .with_position(Vec3::new(2.0, 0.0, 0.0)),
        );
        let pusher = world.add(
            RigidBody::kinematic(Shape::cuboid(Vec3::splat(0.5)))
                .with_position(Vec3::new(-3.0, 0.0, 0.0))
                .with_velocity(Vec3::new(2.0, 0.0, 0.0)),
        );
        let crate_body = world.add(
            RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5)))
                .with_position(Vec3::new(0.0, 0.0, 0.0)),
        );
        world.gravity = Vec3::ZERO;

        run(&mut world, 2.0);

        assert_eq!(
            world.get(wall).unwrap().position(),
            Vec3::new(2.0, 0.0, 0.0),
            "a wall stays put"
        );
        assert!(
            world.get(pusher).unwrap().position().x > -1.0,
            "the kinematic body keeps its course"
        );
        assert!(
            world.get(crate_body).unwrap().position().x > 0.2,
            "and shoves the crate along: {:?}",
            world.get(crate_body).unwrap().position()
        );
    }

    #[test]
    fn switching_a_body_to_static_makes_it_immovable() {
        let mut world = PhysicsWorld::new();
        ground(&mut world);
        let body = world
            .add(RigidBody::dynamic(Shape::sphere(0.5)).with_position(Vec3::new(0.0, 3.0, 0.0)));

        world.get_mut(body).unwrap().set_body_type(BodyType::Static);
        run(&mut world, 1.0);
        assert_eq!(
            world.get(body).unwrap().position().y,
            3.0,
            "static bodies ignore gravity"
        );
        assert_eq!(
            world.get(body).unwrap().inverse_mass(),
            0.0,
            "and have no inverse mass left"
        );

        world
            .get_mut(body)
            .unwrap()
            .set_body_type(BodyType::Dynamic);
        run(&mut world, 1.0);
        assert!(
            world.get(body).unwrap().position().y < 3.0,
            "and fall again once dynamic"
        );
    }

    #[test]
    fn gravity_can_be_changed_and_scaled_per_body() {
        let mut world = PhysicsWorld::with_gravity(Vec3::new(0.0, 0.0, -10.0));
        let drifting = world.add(RigidBody::dynamic(Shape::sphere(0.5)));
        let mut floaty = RigidBody::dynamic(Shape::sphere(0.5));
        floaty.gravity_scale = 0.0;
        floaty.transform.position = Vec3::new(20.0, 0.0, 0.0);
        let floating = world.add(floaty);

        run(&mut world, 1.0);
        assert!(world.get(drifting).unwrap().position().z < -4.0);
        assert_eq!(
            world.get(floating).unwrap().position(),
            Vec3::new(20.0, 0.0, 0.0)
        );
    }

    #[test]
    fn the_broad_phase_keeps_distant_bodies_apart() {
        let mut world = PhysicsWorld::new();
        world.gravity = Vec3::ZERO;
        for i in 0..20 {
            world.add(
                RigidBody::dynamic(Shape::sphere(0.5)).with_position(Vec3::new(
                    i as f32 * 10.0,
                    0.0,
                    0.0,
                )),
            );
        }
        world.step(STEP);
        assert_eq!(world.stats().pairs, 0, "nothing here is near anything else");
        assert_eq!(world.stats().manifolds, 0);
        assert_eq!(world.stats().bodies, 20);
    }

    #[test]
    fn a_region_query_finds_what_is_nearby() {
        let mut world = PhysicsWorld::new();
        world.gravity = Vec3::ZERO;
        let close = world
            .add(RigidBody::dynamic(Shape::sphere(0.5)).with_position(Vec3::new(1.0, 0.0, 0.0)));
        world.add(RigidBody::dynamic(Shape::sphere(0.5)).with_position(Vec3::new(40.0, 0.0, 0.0)));

        let region = Aabb::from_center_half_extents(Vec3::ZERO, Vec3::splat(3.0));
        let found: Vec<BodyHandle> = world.overlapping(region).map(|(h, _)| h).collect();
        assert_eq!(found, vec![close]);
    }

    #[test]
    fn a_step_of_zero_changes_nothing() {
        let mut world = PhysicsWorld::new();
        ground(&mut world);
        let ball = world
            .add(RigidBody::dynamic(Shape::sphere(0.5)).with_position(Vec3::new(0.0, 5.0, 0.0)));
        world.step(0.0);
        world.step(-1.0);
        world.step(f32::NAN);
        assert_eq!(
            world.get(ball).unwrap().position(),
            Vec3::new(0.0, 5.0, 0.0)
        );
        assert_eq!(world.get(ball).unwrap().linear_velocity, Vec3::ZERO);
    }

    #[test]
    fn resetting_contacts_clears_the_cached_state() {
        let mut world = PhysicsWorld::new();
        let floor = ground(&mut world);
        let ball = world
            .add(RigidBody::dynamic(Shape::sphere(0.5)).with_position(Vec3::new(0.0, 0.6, 0.0)));
        run(&mut world, 1.0);
        assert!(world.are_touching(ball, floor));

        world.reset_contacts();
        assert!(!world.are_touching(ball, floor));

        // And the world keeps working: the contact is rediscovered.
        world.wake(ball);
        world.step(STEP);
        assert!(world.are_touching(ball, floor));
        assert_eq!(world.contacts().len(), 1);
    }

    #[test]
    fn stats_describe_the_last_step() {
        let mut world = PhysicsWorld::new();
        ground(&mut world);
        world.add(RigidBody::dynamic(Shape::sphere(0.5)).with_position(Vec3::new(0.0, 0.6, 0.0)));
        run(&mut world, 0.5);
        let stats = world.stats();
        assert_eq!(stats.bodies, 2);
        assert_eq!(stats.awake, 1);
        assert!(stats.pairs >= 1);
        assert!(stats.contacts >= 1);
        assert!(
            stats.max_impulse > 0.0,
            "resting on the ground takes a force"
        );
    }
}
