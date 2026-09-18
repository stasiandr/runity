//! Rigid bodies: what moves, how heavy it is, and how it is integrated.

use crate::shape::{Aabb, Isometry, MassProperties, Shape};
use runity_math::{Mat3, Quat, Vec3};

/// How the solver is allowed to move a body.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BodyType {
    /// Moved by forces and contacts.
    #[default]
    Dynamic,
    /// Never moves. Infinite mass; the world can be made of these.
    Static,
    /// Moved by the game, not by the simulation. Pushes dynamic bodies, is
    /// never pushed back — lifts, doors, moving platforms.
    Kinematic,
}

/// Surface properties used when two bodies touch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    /// Coulomb friction coefficient. Combined between two bodies as a
    /// geometric mean, which is the usual compromise.
    pub friction: f32,
    /// 0 sticks, 1 bounces back at the speed it arrived.
    pub restitution: f32,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            friction: 0.5,
            restitution: 0.0,
        }
    }
}

impl Material {
    pub const fn new(friction: f32, restitution: f32) -> Self {
        Self {
            friction,
            restitution,
        }
    }

    /// Rubber: grippy and lively.
    pub const BOUNCY: Material = Material {
        friction: 0.8,
        restitution: 0.75,
    };
    /// Ice.
    pub const SLIPPERY: Material = Material {
        friction: 0.05,
        restitution: 0.0,
    };

    pub fn combine(a: Material, b: Material) -> Material {
        Material {
            friction: (a.friction * b.friction).max(0.0).sqrt(),
            // The livelier surface decides, which matches the intuition that a
            // ball bounces on concrete because the *ball* is bouncy.
            restitution: a.restitution.max(b.restitution),
        }
    }
}

/// Everything about one simulated object.
#[derive(Debug, Clone)]
pub struct RigidBody {
    pub transform: Isometry,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub body_type: BodyType,
    pub shape: Shape,
    pub material: Material,
    /// Fraction of velocity shed per second, roughly. Air resistance, and a
    /// cheap way to keep a simulation from running forever.
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// Multiplier on world gravity — 0 for floating objects, negative for
    /// balloons.
    pub gravity_scale: f32,
    /// Whether this body is allowed to fall asleep when it stops moving.
    pub can_sleep: bool,
    /// Free-form tag, so a game can find out what it hit.
    pub user_data: u64,

    mass_properties: MassProperties,
    inverse_mass: f32,
    inverse_inertia_local: Mat3,
    inverse_inertia_world: Mat3,
    force: Vec3,
    torque: Vec3,
    sleep_timer: f32,
    sleeping: bool,
}

/// Below these, for [`SLEEP_DELAY`] seconds, a body stops being simulated.
pub const SLEEP_LINEAR_THRESHOLD: f32 = 0.04;
pub const SLEEP_ANGULAR_THRESHOLD: f32 = 0.08;
pub const SLEEP_DELAY: f32 = 0.5;

impl RigidBody {
    /// A body that gravity and contacts move around, with mass derived from
    /// its shape and a density of 1.
    pub fn dynamic(shape: Shape) -> Self {
        let mut body = Self::with_type(shape, BodyType::Dynamic);
        body.set_density(1.0);
        body
    }

    /// A body that never moves.
    pub fn fixed(shape: Shape) -> Self {
        Self::with_type(shape, BodyType::Static)
    }

    /// A body the game moves directly.
    pub fn kinematic(shape: Shape) -> Self {
        Self::with_type(shape, BodyType::Kinematic)
    }

    fn with_type(shape: Shape, body_type: BodyType) -> Self {
        Self {
            transform: Isometry::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            body_type,
            shape,
            material: Material::default(),
            linear_damping: 0.01,
            angular_damping: 0.05,
            gravity_scale: 1.0,
            can_sleep: true,
            user_data: 0,
            mass_properties: MassProperties::ZERO,
            inverse_mass: 0.0,
            inverse_inertia_local: Mat3::ZERO,
            inverse_inertia_world: Mat3::ZERO,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
            sleep_timer: 0.0,
            sleeping: false,
        }
    }

    pub fn with_position(mut self, position: Vec3) -> Self {
        self.transform.position = position;
        self
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.transform.rotation = rotation;
        self.refresh_world_inertia();
        self
    }

    pub fn with_velocity(mut self, velocity: Vec3) -> Self {
        self.linear_velocity = velocity;
        self
    }

    pub fn with_angular_velocity(mut self, velocity: Vec3) -> Self {
        self.angular_velocity = velocity;
        self
    }

    pub fn with_material(mut self, material: Material) -> Self {
        self.material = material;
        self
    }

    pub fn with_density(mut self, density: f32) -> Self {
        self.set_density(density);
        self
    }

    pub fn with_mass(mut self, mass: f32) -> Self {
        self.set_mass(mass);
        self
    }

    pub fn with_user_data(mut self, data: u64) -> Self {
        self.user_data = data;
        self
    }

    /// Recompute mass and inertia from the shape at this density.
    pub fn set_density(&mut self, density: f32) {
        if self.body_type != BodyType::Dynamic || self.shape.is_infinite() {
            self.make_infinite();
            return;
        }
        self.apply_mass_properties(self.shape.mass_properties(density));
    }

    /// Keep the shape's inertia *distribution* but scale it to this mass.
    pub fn set_mass(&mut self, mass: f32) {
        if self.body_type != BodyType::Dynamic || self.shape.is_infinite() || mass <= 0.0 {
            self.make_infinite();
            return;
        }
        let reference = self.shape.mass_properties(1.0);
        let scale = mass / reference.mass.max(1e-9);
        self.apply_mass_properties(MassProperties {
            mass,
            inertia: reference.inertia * scale,
        });
    }

    fn apply_mass_properties(&mut self, properties: MassProperties) {
        self.mass_properties = properties;
        self.inverse_mass = if properties.mass > 0.0 {
            1.0 / properties.mass
        } else {
            0.0
        };
        self.inverse_inertia_local = properties.inverse_local_tensor();
        self.refresh_world_inertia();
    }

    fn make_infinite(&mut self) {
        self.mass_properties = MassProperties::ZERO;
        self.inverse_mass = 0.0;
        self.inverse_inertia_local = Mat3::ZERO;
        self.inverse_inertia_world = Mat3::ZERO;
    }

    /// Rotate the inverse inertia tensor into world space. Called whenever the
    /// orientation changes: `I⁻¹_world = R I⁻¹_local Rᵀ`.
    pub fn refresh_world_inertia(&mut self) {
        if self.inverse_mass == 0.0 {
            self.inverse_inertia_world = Mat3::ZERO;
            return;
        }
        let rotation = Mat3::from_quat(self.transform.rotation);
        self.inverse_inertia_world = Mat3::transform_tensor(rotation, self.inverse_inertia_local);
    }

    #[inline]
    pub fn mass(&self) -> f32 {
        self.mass_properties.mass
    }

    #[inline]
    pub fn inverse_mass(&self) -> f32 {
        self.inverse_mass
    }

    #[inline]
    pub fn inverse_inertia_world(&self) -> Mat3 {
        self.inverse_inertia_world
    }

    #[inline]
    pub fn position(&self) -> Vec3 {
        self.transform.position
    }

    #[inline]
    pub fn rotation(&self) -> Quat {
        self.transform.rotation
    }

    #[inline]
    pub fn is_dynamic(&self) -> bool {
        self.body_type == BodyType::Dynamic
    }

    /// Whether the solver may change this body's velocity.
    #[inline]
    pub fn is_movable(&self) -> bool {
        self.body_type == BodyType::Dynamic && !self.sleeping
    }

    #[inline]
    pub fn is_sleeping(&self) -> bool {
        self.sleeping
    }

    pub fn aabb(&self) -> Aabb {
        self.shape.aabb(&self.transform)
    }

    /// Velocity of the material point currently at `world_point`.
    ///
    /// `v + ω × r` — the reason a spinning wheel's contact patch can be
    /// stationary while its center moves.
    #[inline]
    pub fn velocity_at(&self, world_point: Vec3) -> Vec3 {
        self.linear_velocity
            + self
                .angular_velocity
                .cross(world_point - self.transform.position)
    }

    /// Accumulate a force at the center of mass, applied until the next step.
    pub fn apply_force(&mut self, force: Vec3) {
        if !self.is_dynamic() {
            return;
        }
        self.wake();
        self.force += force;
    }

    /// Accumulate a force somewhere other than the center: it also spins the body.
    pub fn apply_force_at(&mut self, force: Vec3, world_point: Vec3) {
        if !self.is_dynamic() {
            return;
        }
        self.wake();
        self.force += force;
        self.torque += (world_point - self.transform.position).cross(force);
    }

    pub fn apply_torque(&mut self, torque: Vec3) {
        if !self.is_dynamic() {
            return;
        }
        self.wake();
        self.torque += torque;
    }

    /// An instantaneous change in momentum — a kick rather than a push.
    pub fn apply_impulse(&mut self, impulse: Vec3) {
        if !self.is_dynamic() {
            return;
        }
        self.wake();
        self.linear_velocity += impulse * self.inverse_mass;
    }

    pub fn apply_impulse_at(&mut self, impulse: Vec3, world_point: Vec3) {
        if !self.is_dynamic() {
            return;
        }
        self.wake();
        let r = world_point - self.transform.position;
        self.linear_velocity += impulse * self.inverse_mass;
        self.angular_velocity += self.inverse_inertia_world * r.cross(impulse);
    }

    pub fn apply_angular_impulse(&mut self, impulse: Vec3) {
        if !self.is_dynamic() {
            return;
        }
        self.wake();
        self.angular_velocity += self.inverse_inertia_world * impulse;
    }

    /// Used by the solver, which has already decided the body should move.
    #[inline]
    pub(crate) fn apply_impulse_unchecked(&mut self, impulse: Vec3, r: Vec3) {
        self.linear_velocity += impulse * self.inverse_mass;
        self.angular_velocity += self.inverse_inertia_world * r.cross(impulse);
    }

    /// Put the body back to work, and reset its sleep countdown.
    pub fn wake(&mut self) {
        self.sleeping = false;
        self.sleep_timer = 0.0;
    }

    /// Stop simulating this body until something touches it.
    pub fn sleep(&mut self) {
        if !self.can_sleep {
            return;
        }
        self.sleeping = true;
        self.linear_velocity = Vec3::ZERO;
        self.angular_velocity = Vec3::ZERO;
    }

    pub fn clear_forces(&mut self) {
        self.force = Vec3::ZERO;
        self.torque = Vec3::ZERO;
    }

    /// First half of the step: forces and gravity become velocity.
    pub fn integrate_velocity(&mut self, gravity: Vec3, dt: f32) {
        if !self.is_movable() {
            return;
        }
        self.linear_velocity +=
            (gravity * self.gravity_scale + self.force * self.inverse_mass) * dt;
        self.angular_velocity += self.inverse_inertia_world * self.torque * dt;

        // Exponential damping, framerate independent: (1 - d)^dt rather than
        // 1 - d*dt, so halving the step does not halve the drag.
        let linear = (1.0 - self.linear_damping).clamp(0.0, 1.0).powf(dt);
        let angular = (1.0 - self.angular_damping).clamp(0.0, 1.0).powf(dt);
        self.linear_velocity *= linear;
        self.angular_velocity *= angular;
    }

    /// Second half: velocity becomes position, after the solver has had its say.
    pub fn integrate_position(&mut self, dt: f32) {
        match self.body_type {
            BodyType::Static => return,
            BodyType::Dynamic if self.sleeping => return,
            _ => {}
        }
        self.transform.position += self.linear_velocity * dt;
        self.transform.rotation = self.transform.rotation.integrate(self.angular_velocity, dt);
        self.refresh_world_inertia();
    }

    /// Track how long the body has been nearly still, and put it to sleep once
    /// that has gone on long enough.
    pub fn update_sleep(&mut self, dt: f32) {
        if !self.is_dynamic() || self.sleeping {
            return;
        }
        if !self.can_sleep {
            self.sleep_timer = 0.0;
            return;
        }
        let still = self.linear_velocity.length() < SLEEP_LINEAR_THRESHOLD
            && self.angular_velocity.length() < SLEEP_ANGULAR_THRESHOLD;
        if still {
            self.sleep_timer += dt;
            if self.sleep_timer >= SLEEP_DELAY {
                self.sleep();
            }
        } else {
            self.sleep_timer = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ball() -> RigidBody {
        RigidBody::dynamic(Shape::sphere(0.5))
    }

    #[test]
    fn a_dynamic_body_gets_mass_from_its_shape() {
        let body = ball();
        assert!(body.mass() > 0.0);
        assert!((body.inverse_mass() - 1.0 / body.mass()).abs() < 1e-6);

        // Static and infinite bodies have none, whatever their shape.
        assert_eq!(RigidBody::fixed(Shape::sphere(1.0)).inverse_mass(), 0.0);
        assert_eq!(RigidBody::dynamic(Shape::ground()).inverse_mass(), 0.0);
    }

    #[test]
    fn setting_a_mass_keeps_the_shapes_inertia_distribution() {
        let long = Shape::cuboid(Vec3::new(2.0, 0.25, 0.25));
        let heavy = RigidBody::dynamic(long).with_mass(10.0);
        assert!((heavy.mass() - 10.0).abs() < 1e-4);
        // Still easier to spin about its long axis.
        let tensor = heavy.mass_properties.inertia;
        assert!(tensor.x < tensor.y);
    }

    #[test]
    fn gravity_accelerates_a_body_and_damping_slows_it() {
        let mut body = ball();
        body.linear_damping = 0.0;
        body.integrate_velocity(Vec3::new(0.0, -10.0, 0.0), 0.5);
        assert!((body.linear_velocity.y + 5.0).abs() < 1e-5);

        let mut damped = ball();
        damped.linear_damping = 0.5;
        damped.linear_velocity = Vec3::new(10.0, 0.0, 0.0);
        damped.integrate_velocity(Vec3::ZERO, 1.0);
        assert!(
            (damped.linear_velocity.x - 5.0).abs() < 1e-4,
            "{:?}",
            damped.linear_velocity
        );
    }

    #[test]
    fn damping_does_not_depend_on_the_step_size() {
        let run = |steps: u32| {
            let mut body = ball();
            body.linear_damping = 0.5;
            body.linear_velocity = Vec3::X;
            let dt = 1.0 / steps as f32;
            for _ in 0..steps {
                body.integrate_velocity(Vec3::ZERO, dt);
            }
            body.linear_velocity.x
        };
        assert!(
            (run(1) - run(100)).abs() < 1e-4,
            "{} vs {}",
            run(1),
            run(100)
        );
    }

    #[test]
    fn an_off_center_impulse_makes_a_body_spin() {
        let mut body = ball();
        let hit = body.position() + Vec3::new(0.0, 0.5, 0.0);
        body.apply_impulse_at(Vec3::X, hit);
        assert!(body.linear_velocity.x > 0.0, "it also moves");
        assert!(
            body.angular_velocity.z < 0.0,
            "and turns: {:?}",
            body.angular_velocity
        );

        // Through the center, it only moves.
        let mut straight = ball();
        straight.apply_impulse_at(Vec3::X, straight.position());
        assert_eq!(straight.angular_velocity, Vec3::ZERO);
    }

    #[test]
    fn velocity_at_a_point_includes_the_spin() {
        let mut body = ball();
        body.linear_velocity = Vec3::X;
        body.angular_velocity = Vec3::Y; // 1 rad/s about Y
                                         // A point one unit along +Z moves with v + ω × r = X + (Y × Z) = 2X.
        let at = body.position() + Vec3::Z;
        assert!((body.velocity_at(at) - Vec3::X * 2.0).length() < 1e-5);
    }

    #[test]
    fn a_still_body_falls_asleep_and_a_moving_one_does_not() {
        let mut body = ball();
        for _ in 0..40 {
            body.update_sleep(1.0 / 60.0);
        }
        assert!(body.is_sleeping(), "half a second of stillness is enough");
        assert!(!body.is_movable(), "and the solver skips it");

        body.apply_impulse(Vec3::Y * 10.0);
        assert!(!body.is_sleeping(), "an impulse wakes it");

        let mut busy = ball();
        busy.linear_velocity = Vec3::X * 5.0;
        for _ in 0..120 {
            busy.update_sleep(1.0 / 60.0);
        }
        assert!(!busy.is_sleeping());
    }

    #[test]
    fn static_bodies_ignore_forces_entirely() {
        let mut wall = RigidBody::fixed(Shape::cuboid(Vec3::ONE));
        wall.apply_impulse(Vec3::X * 100.0);
        wall.apply_force(Vec3::X * 100.0);
        wall.integrate_velocity(Vec3::new(0.0, -10.0, 0.0), 1.0);
        wall.integrate_position(1.0);
        assert_eq!(wall.linear_velocity, Vec3::ZERO);
        assert_eq!(wall.position(), Vec3::ZERO);
    }

    #[test]
    fn a_kinematic_body_moves_but_is_not_pushed() {
        let mut platform = RigidBody::kinematic(Shape::cuboid(Vec3::ONE));
        platform.linear_velocity = Vec3::Y;
        platform.integrate_position(1.0);
        assert!(
            (platform.position().y - 1.0).abs() < 1e-6,
            "it goes where it is told"
        );

        platform.apply_impulse(Vec3::X * 100.0);
        assert_eq!(platform.linear_velocity, Vec3::Y, "and ignores impulses");
        assert_eq!(platform.inverse_mass(), 0.0);
    }

    #[test]
    fn the_world_inertia_tensor_turns_with_the_body() {
        let long = Shape::cuboid(Vec3::new(2.0, 0.25, 0.25));
        let mut body = RigidBody::dynamic(long);
        let upright = body.inverse_inertia_world();
        // Easiest to spin about X, so its inverse inertia is the largest.
        assert!(upright.cols[0].x > upright.cols[1].y);

        body.transform.rotation = Quat::from_axis_angle(Vec3::Z, core::f32::consts::FRAC_PI_2);
        body.refresh_world_inertia();
        let turned = body.inverse_inertia_world();
        assert!(
            turned.cols[1].y > turned.cols[0].x,
            "the easy axis moved to Y"
        );
    }

    #[test]
    fn materials_combine_predictably() {
        let ice = Material::SLIPPERY;
        let rubber = Material::BOUNCY;
        let mixed = Material::combine(ice, rubber);
        assert!(mixed.friction > ice.friction && mixed.friction < rubber.friction);
        assert_eq!(
            mixed.restitution, rubber.restitution,
            "the livelier surface wins"
        );
    }
}
