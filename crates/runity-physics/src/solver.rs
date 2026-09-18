//! The contact solver: sequential impulses.
//!
//! Each contact is a constraint — "these two points may not approach each
//! other" — and each one is satisfied by an impulse along the normal. Solving
//! them one at a time breaks the others, so the whole set is swept repeatedly;
//! after a handful of passes the errors are small enough not to see. This is
//! the approach Box2D popularized, and it is what makes a stack of boxes stand
//! up without a matrix solver.
//!
//! Three details do most of the work:
//!
//! * **Accumulated impulses, clamped.** A contact may push, never pull, so the
//!   *total* impulse is clamped at zero rather than each increment.
//! * **Warm starting.** Last frame's impulse is applied before iterating, so a
//!   resting stack starts each step from the answer instead of rediscovering it.
//! * **Split impulse.** Pushing overlapping bodies apart is done with a second,
//!   throwaway velocity, so fixing penetration does not add energy and make the
//!   stack jitter.

use crate::body::RigidBody;
use crate::collide::{FeatureId, Manifold, MAX_CONTACTS};
use runity_math::Vec3;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolverSettings {
    /// Passes over the contact set per step. More is stiffer and slower.
    pub velocity_iterations: usize,
    /// Passes of the penetration-only solve.
    pub position_iterations: usize,
    /// Overlap this deep is left alone, so resting contacts stop twitching.
    pub slop: f32,
    /// How much of the remaining overlap is removed per step, 0..1.
    pub correction: f32,
    /// Impacts slower than this do not bounce, however elastic the material.
    pub restitution_threshold: f32,
    pub warm_starting: bool,
}

impl Default for SolverSettings {
    fn default() -> Self {
        Self {
            velocity_iterations: 8,
            position_iterations: 3,
            slop: 0.005,
            correction: 0.2,
            restitution_threshold: 1.0,
            warm_starting: true,
        }
    }
}

/// A manifold between two bodies, as the solver receives it.
#[derive(Debug, Clone, Copy)]
pub struct PairManifold {
    pub a: usize,
    pub b: usize,
    /// Stable identity of the pair, so impulses can be matched between frames
    /// even as indices move around.
    pub key: (u32, u32),
    pub manifold: Manifold,
}

#[derive(Debug, Clone, Copy)]
struct ConstraintPoint {
    r_a: Vec3,
    r_b: Vec3,
    normal_mass: f32,
    tangent_mass: [f32; 2],
    /// Target separating velocity, from restitution.
    bounce: f32,
    penetration: f32,
    normal_impulse: f32,
    tangent_impulse: [f32; 2],
    /// Impulse used by the position pass, kept apart from the real one.
    pseudo_impulse: f32,
    feature: FeatureId,
}

#[derive(Debug, Clone, Copy)]
struct ContactConstraint {
    a: usize,
    b: usize,
    key: (u32, u32),
    normal: Vec3,
    tangents: [Vec3; 2],
    friction: f32,
    points: [ConstraintPoint; MAX_CONTACTS],
    count: usize,
}

/// One stored contact: which feature it was, and the normal and friction
/// impulses it needed last time.
type StoredImpulse = (FeatureId, f32, [f32; 2]);

/// Impulses from the previous step, so contacts can pick up where they left off.
#[derive(Debug, Default, Clone)]
pub struct ImpulseCache {
    entries: HashMap<(u32, u32), [StoredImpulse; MAX_CONTACTS]>,
}

impl ImpulseCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Forget pairs that are no longer touching, so the cache does not grow
    /// without bound over a long session.
    pub fn retain_pairs(&mut self, live: &[(u32, u32)]) {
        self.entries
            .retain(|key, _| live.binary_search(key).is_ok());
    }

    fn lookup(&self, key: (u32, u32), feature: FeatureId) -> Option<(f32, [f32; 2])> {
        let stored = self.entries.get(&key)?;
        stored
            .iter()
            .find(|(id, _, _)| *id == feature)
            .map(|(_, normal, tangent)| (*normal, *tangent))
    }
}

/// Builds and solves the contact constraints for one step.
#[derive(Debug, Default)]
pub struct ContactSolver {
    constraints: Vec<ContactConstraint>,
    /// Velocity used only to resolve overlap, discarded at the end of the step.
    pseudo: Vec<(Vec3, Vec3)>,
}

/// Two orthogonal directions perpendicular to `normal`.
fn tangent_basis(normal: Vec3) -> [Vec3; 2] {
    // Normalized here rather than trusted: a manifold normal that is a
    // thousandth off would otherwise scale every friction impulse.
    let normal = normal.normalized();
    // Pick the axis least aligned with the normal, so the cross product is
    // never degenerate.
    let helper = if normal.x.abs() < 0.57735 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let t1 = helper.cross(normal).normalized();
    [t1, normal.cross(t1)]
}

/// Effective mass along `direction` for a contact at `r_a`/`r_b`.
fn effective_mass(a: &RigidBody, b: &RigidBody, r_a: Vec3, r_b: Vec3, direction: Vec3) -> f32 {
    let angular_a = (a.inverse_inertia_world() * r_a.cross(direction))
        .cross(r_a)
        .dot(direction);
    let angular_b = (b.inverse_inertia_world() * r_b.cross(direction))
        .cross(r_b)
        .dot(direction);
    let total = a.inverse_mass() + b.inverse_mass() + angular_a + angular_b;
    if total > 0.0 {
        1.0 / total
    } else {
        0.0
    }
}

fn pair_mut(bodies: &mut [RigidBody], a: usize, b: usize) -> (&mut RigidBody, &mut RigidBody) {
    debug_assert_ne!(a, b, "a body cannot collide with itself");
    if a < b {
        let (left, right) = bodies.split_at_mut(b);
        (&mut left[a], &mut right[0])
    } else {
        let (left, right) = bodies.split_at_mut(a);
        (&mut right[0], &mut left[b])
    }
}

impl ContactSolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn constraint_count(&self) -> usize {
        self.constraints.len()
    }

    pub fn contact_count(&self) -> usize {
        self.constraints.iter().map(|c| c.count).sum()
    }

    /// Turn manifolds into constraints: precompute the masses, capture the
    /// bounce, and pick up last frame's impulses.
    pub fn prepare(
        &mut self,
        bodies: &[RigidBody],
        manifolds: &[PairManifold],
        cache: &ImpulseCache,
        settings: &SolverSettings,
    ) {
        self.constraints.clear();
        self.pseudo.clear();
        self.pseudo.resize(bodies.len(), (Vec3::ZERO, Vec3::ZERO));

        for pair in manifolds {
            let a = &bodies[pair.a];
            let b = &bodies[pair.b];
            if a.inverse_mass() + b.inverse_mass() == 0.0 {
                continue; // two immovable bodies: nothing to solve
            }
            let material = crate::body::Material::combine(a.material, b.material);
            let normal = pair.manifold.normal;
            let mut constraint = ContactConstraint {
                a: pair.a,
                b: pair.b,
                key: pair.key,
                normal,
                tangents: tangent_basis(normal),
                friction: material.friction,
                points: [ConstraintPoint {
                    r_a: Vec3::ZERO,
                    r_b: Vec3::ZERO,
                    normal_mass: 0.0,
                    tangent_mass: [0.0; 2],
                    bounce: 0.0,
                    penetration: 0.0,
                    normal_impulse: 0.0,
                    tangent_impulse: [0.0; 2],
                    pseudo_impulse: 0.0,
                    feature: 0,
                }; MAX_CONTACTS],
                count: 0,
            };

            for contact in pair.manifold.contacts() {
                let r_a = contact.position - a.position();
                let r_b = contact.position - b.position();
                let relative = b.velocity_at(contact.position) - a.velocity_at(contact.position);
                let approach = relative.dot(normal);

                // Restitution is captured now, from the impact speed, and used
                // as a target for the whole step — measuring it later would
                // read the velocity the solver itself produced.
                let bounce = if approach < -settings.restitution_threshold {
                    -material.restitution * approach
                } else {
                    0.0
                };

                let (normal_impulse, tangent_impulse) = if settings.warm_starting {
                    cache
                        .lookup(pair.key, contact.feature)
                        .unwrap_or((0.0, [0.0; 2]))
                } else {
                    (0.0, [0.0; 2])
                };

                constraint.points[constraint.count] = ConstraintPoint {
                    r_a,
                    r_b,
                    normal_mass: effective_mass(a, b, r_a, r_b, normal),
                    tangent_mass: [
                        effective_mass(a, b, r_a, r_b, constraint.tangents[0]),
                        effective_mass(a, b, r_a, r_b, constraint.tangents[1]),
                    ],
                    bounce,
                    penetration: contact.penetration,
                    normal_impulse,
                    tangent_impulse,
                    pseudo_impulse: 0.0,
                    feature: contact.feature,
                };
                constraint.count += 1;
            }

            if constraint.count > 0 {
                self.constraints.push(constraint);
            }
        }
    }

    /// Re-apply the previous step's impulses before iterating.
    pub fn warm_start(&mut self, bodies: &mut [RigidBody]) {
        for constraint in &self.constraints {
            let (a, b) = pair_mut(bodies, constraint.a, constraint.b);
            for point in &constraint.points[..constraint.count] {
                let impulse = constraint.normal * point.normal_impulse
                    + constraint.tangents[0] * point.tangent_impulse[0]
                    + constraint.tangents[1] * point.tangent_impulse[1];
                a.apply_impulse_unchecked(-impulse, point.r_a);
                b.apply_impulse_unchecked(impulse, point.r_b);
            }
        }
    }

    /// One pass over every contact: normal first, then friction.
    pub fn solve_velocity(&mut self, bodies: &mut [RigidBody], settings: &SolverSettings) {
        for _ in 0..settings.velocity_iterations {
            for constraint in &mut self.constraints {
                let (a, b) = pair_mut(bodies, constraint.a, constraint.b);
                for point in &mut constraint.points[..constraint.count] {
                    // --- normal ---
                    let contact = a.position() + point.r_a;
                    let relative = b.velocity_at(contact) - a.velocity_at(contact);
                    let approach = relative.dot(constraint.normal);
                    let mut lambda = (-approach + point.bounce) * point.normal_mass;

                    // Clamp the accumulated impulse, not this increment: a
                    // contact may push but never pull.
                    let total = (point.normal_impulse + lambda).max(0.0);
                    lambda = total - point.normal_impulse;
                    point.normal_impulse = total;

                    let impulse = constraint.normal * lambda;
                    a.apply_impulse_unchecked(-impulse, point.r_a);
                    b.apply_impulse_unchecked(impulse, point.r_b);

                    // --- friction ---
                    let limit = constraint.friction * point.normal_impulse;
                    for axis in 0..2 {
                        let tangent = constraint.tangents[axis];
                        let relative = b.velocity_at(contact) - a.velocity_at(contact);
                        let sliding = relative.dot(tangent);
                        let mut lambda = -sliding * point.tangent_mass[axis];
                        let total = (point.tangent_impulse[axis] + lambda).clamp(-limit, limit);
                        lambda = total - point.tangent_impulse[axis];
                        point.tangent_impulse[axis] = total;

                        let impulse = tangent * lambda;
                        a.apply_impulse_unchecked(-impulse, point.r_a);
                        b.apply_impulse_unchecked(impulse, point.r_b);
                    }
                }
            }
        }
    }

    /// Resolve overlap with a throwaway velocity, so separating bodies does not
    /// hand them energy they keep.
    pub fn solve_position(&mut self, bodies: &[RigidBody], settings: &SolverSettings, dt: f32) {
        if settings.position_iterations == 0 || dt <= 0.0 {
            return;
        }
        for (pseudo, _) in self.pseudo.iter_mut() {
            *pseudo = Vec3::ZERO;
        }
        for (_, pseudo) in self.pseudo.iter_mut() {
            *pseudo = Vec3::ZERO;
        }

        for _ in 0..settings.position_iterations {
            for constraint in &mut self.constraints {
                for point in &mut constraint.points[..constraint.count] {
                    let depth = point.penetration - settings.slop;
                    if depth <= 0.0 {
                        continue;
                    }
                    let (linear_a, angular_a) = self.pseudo[constraint.a];
                    let (linear_b, angular_b) = self.pseudo[constraint.b];
                    let velocity_a = linear_a + angular_a.cross(point.r_a);
                    let velocity_b = linear_b + angular_b.cross(point.r_b);
                    let approach = (velocity_b - velocity_a).dot(constraint.normal);

                    let target = settings.correction * depth / dt;
                    let mut lambda = (target - approach) * point.normal_mass;
                    let total = (point.pseudo_impulse + lambda).max(0.0);
                    lambda = total - point.pseudo_impulse;
                    point.pseudo_impulse = total;

                    let impulse = constraint.normal * lambda;
                    let a = &bodies[constraint.a];
                    let b = &bodies[constraint.b];
                    let entry = &mut self.pseudo[constraint.a];
                    entry.0 -= impulse * a.inverse_mass();
                    entry.1 -= a.inverse_inertia_world() * point.r_a.cross(impulse);
                    let entry = &mut self.pseudo[constraint.b];
                    entry.0 += impulse * b.inverse_mass();
                    entry.1 += b.inverse_inertia_world() * point.r_b.cross(impulse);
                }
            }
        }
    }

    /// Extra velocity for one body, used for this step's integration only.
    pub fn pseudo_velocity(&self, body: usize) -> (Vec3, Vec3) {
        self.pseudo
            .get(body)
            .copied()
            .unwrap_or((Vec3::ZERO, Vec3::ZERO))
    }

    /// Hand this step's impulses to the cache for the next one.
    pub fn store_impulses(&self, cache: &mut ImpulseCache) {
        for constraint in &self.constraints {
            let mut stored: [StoredImpulse; MAX_CONTACTS] = [(0, 0.0, [0.0; 2]); MAX_CONTACTS];
            for (slot, point) in stored
                .iter_mut()
                .zip(&constraint.points[..constraint.count])
            {
                *slot = (point.feature, point.normal_impulse, point.tangent_impulse);
            }
            cache.entries.insert(constraint.key, stored);
        }
    }

    /// Largest impulse applied this step — a cheap measure of how hard the
    /// contacts were working, useful for impact sounds and debugging.
    pub fn max_impulse(&self) -> f32 {
        self.constraints
            .iter()
            .flat_map(|c| c.points[..c.count].iter())
            .map(|p| p.normal_impulse)
            .fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collide::collide;
    use crate::shape::{Isometry, Shape};

    fn solve_once(bodies: &mut [RigidBody], settings: &SolverSettings, dt: f32) -> ContactSolver {
        let mut manifolds = Vec::new();
        for a in 0..bodies.len() {
            for b in a + 1..bodies.len() {
                if let Some(manifold) = collide(
                    &bodies[a].shape,
                    &bodies[a].transform,
                    &bodies[b].shape,
                    &bodies[b].transform,
                ) {
                    manifolds.push(PairManifold {
                        a,
                        b,
                        key: (a as u32, b as u32),
                        manifold,
                    });
                }
            }
        }
        let cache = ImpulseCache::new();
        let mut solver = ContactSolver::new();
        solver.prepare(bodies, &manifolds, &cache, settings);
        solver.solve_velocity(bodies, settings);
        solver.solve_position(bodies, settings, dt);
        solver
    }

    #[test]
    fn a_tangent_basis_is_orthonormal_for_any_normal() {
        for normal in [
            Vec3::Y,
            Vec3::X,
            -Vec3::Z,
            Vec3::new(0.3, -0.5, 0.8).normalized(),
            Vec3::new(0.577, 0.577, 0.577),
        ] {
            let [t1, t2] = tangent_basis(normal);
            assert!((t1.length() - 1.0).abs() < 1e-4, "{t1:?}");
            assert!((t2.length() - 1.0).abs() < 1e-4);
            assert!(t1.dot(normal).abs() < 1e-4, "t1 is perpendicular");
            assert!(t2.dot(normal).abs() < 1e-4);
            assert!(t1.dot(t2).abs() < 1e-4, "and so are they to each other");
        }
    }

    #[test]
    fn a_falling_body_stops_against_the_ground() {
        let mut bodies = vec![
            RigidBody::dynamic(Shape::sphere(0.5)).with_position(Vec3::new(0.0, 0.45, 0.0)),
            RigidBody::fixed(Shape::ground()),
        ];
        bodies[0].linear_velocity = Vec3::new(0.0, -5.0, 0.0);

        solve_once(&mut bodies, &SolverSettings::default(), 1.0 / 60.0);
        assert!(
            bodies[0].linear_velocity.y.abs() < 0.2,
            "downward motion should be removed: {:?}",
            bodies[0].linear_velocity
        );
        assert_eq!(
            bodies[1].linear_velocity,
            Vec3::ZERO,
            "the ground does not move"
        );
    }

    #[test]
    fn a_bouncy_body_comes_back_up() {
        let mut bodies = vec![
            RigidBody::dynamic(Shape::sphere(0.5))
                .with_position(Vec3::new(0.0, 0.45, 0.0))
                .with_material(crate::body::Material::new(0.5, 0.8)),
            RigidBody::fixed(Shape::ground()),
        ];
        bodies[0].linear_velocity = Vec3::new(0.0, -5.0, 0.0);
        solve_once(&mut bodies, &SolverSettings::default(), 1.0 / 60.0);
        assert!(
            bodies[0].linear_velocity.y > 3.0,
            "{:?}",
            bodies[0].linear_velocity
        );
    }

    #[test]
    fn a_slow_touch_does_not_bounce_however_elastic() {
        let mut bodies = vec![
            RigidBody::dynamic(Shape::sphere(0.5))
                .with_position(Vec3::new(0.0, 0.45, 0.0))
                .with_material(crate::body::Material::new(0.5, 1.0)),
            RigidBody::fixed(Shape::ground()),
        ];
        bodies[0].linear_velocity = Vec3::new(0.0, -0.2, 0.0);
        solve_once(&mut bodies, &SolverSettings::default(), 1.0 / 60.0);
        assert!(
            bodies[0].linear_velocity.y.abs() < 0.1,
            "below the threshold it should settle, not bounce: {:?}",
            bodies[0].linear_velocity
        );
    }

    #[test]
    fn friction_slows_a_body_sliding_along_the_ground() {
        let make = |friction: f32| {
            let mut bodies = vec![
                RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5)))
                    .with_position(Vec3::new(0.0, 0.495, 0.0))
                    .with_material(crate::body::Material::new(friction, 0.0)),
                RigidBody::fixed(Shape::ground()),
            ];
            bodies[0].linear_velocity = Vec3::new(4.0, -1.0, 0.0);
            solve_once(&mut bodies, &SolverSettings::default(), 1.0 / 60.0);
            bodies[0].linear_velocity.x
        };
        let grippy = make(1.0);
        let slippery = make(0.0);
        assert!(
            (slippery - 4.0).abs() < 1e-3,
            "frictionless keeps its speed: {slippery}"
        );
        assert!(grippy < slippery, "friction takes some away: {grippy}");
    }

    #[test]
    fn overlap_is_resolved_without_adding_velocity() {
        // Deeply overlapping, both at rest: the position pass separates them,
        // and the real velocity stays zero.
        let mut bodies = vec![
            RigidBody::dynamic(Shape::sphere(0.5)).with_position(Vec3::new(0.0, 0.2, 0.0)),
            RigidBody::fixed(Shape::ground()),
        ];
        let solver = solve_once(&mut bodies, &SolverSettings::default(), 1.0 / 60.0);
        assert!(
            bodies[0].linear_velocity.length() < 1e-4,
            "no free energy: {:?}",
            bodies[0].linear_velocity
        );
        let (linear, _) = solver.pseudo_velocity(0);
        assert!(linear.y > 0.0, "but it is being pushed out: {linear:?}");
    }

    #[test]
    fn warm_starting_reapplies_the_previous_impulse() {
        let mut bodies = vec![
            RigidBody::dynamic(Shape::sphere(0.5)).with_position(Vec3::new(0.0, 0.5, 0.0)),
            RigidBody::fixed(Shape::ground()),
        ];
        let settings = SolverSettings::default();
        let manifolds = vec![PairManifold {
            a: 0,
            b: 1,
            key: (0, 1),
            manifold: collide(
                &bodies[0].shape,
                &bodies[0].transform,
                &bodies[1].shape,
                &bodies[1].transform,
            )
            .expect("touching"),
        }];

        let mut cache = ImpulseCache::new();
        let mut solver = ContactSolver::new();
        bodies[0].linear_velocity = Vec3::new(0.0, -3.0, 0.0);
        solver.prepare(&bodies, &manifolds, &cache, &settings);
        solver.solve_velocity(&mut bodies, &settings);
        solver.store_impulses(&mut cache);
        assert_eq!(cache.len(), 1);
        assert!(solver.max_impulse() > 0.0);

        // A second step starts from that impulse rather than from zero.
        let mut warm = ContactSolver::new();
        warm.prepare(&bodies, &manifolds, &cache, &settings);
        warm.warm_start(&mut bodies);
        assert!(
            bodies[0].linear_velocity.y > 0.0,
            "the stored push is reapplied"
        );
    }

    #[test]
    fn two_immovable_bodies_produce_no_constraint() {
        let bodies = vec![
            RigidBody::fixed(Shape::cuboid(Vec3::ONE)),
            RigidBody::fixed(Shape::cuboid(Vec3::ONE)),
        ];
        let manifolds = vec![PairManifold {
            a: 0,
            b: 1,
            key: (0, 1),
            manifold: collide(
                &bodies[0].shape,
                &Isometry::IDENTITY,
                &bodies[1].shape,
                &Isometry::IDENTITY,
            )
            .expect("overlapping"),
        }];
        let mut solver = ContactSolver::new();
        solver.prepare(
            &bodies,
            &manifolds,
            &ImpulseCache::new(),
            &SolverSettings::default(),
        );
        assert_eq!(solver.constraint_count(), 0);
    }
}
