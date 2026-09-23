//! Solid bodies, from rapier.
//!
//! The engine does not implement physics; it holds rapier's world and keeps
//! it in step with the entity world. Two things are worth stating because
//! getting either wrong is silent:
//!
//! * **The step is fixed.** It runs from the simulation clock, never from the
//!   frame delta. A solver fed a variable step gives a different answer at
//!   144 Hz than at 60, and every replay, save file and networked client
//!   depends on it not doing that.
//! * **Transforms flow one way per body kind.** A dynamic body's position is
//!   rapier's to own, and writing to it from the scene fights the solver. A
//!   static body's is the scene's, and rapier only reads it.
//!
//! What is deliberately *not* here: a character controller. How a person
//! walks — what counts as a step, when they are allowed to jump, how fast
//! they slide — is a game's design, not an engine's, and the version that
//! lived here was a guess at a game nobody has written yet. rapier's
//! `KinematicCharacterController` is one `use` away for whoever needs it,
//! and they will want their own numbers anyway.

use glam::{Quat, Vec3};
use hecs::World;
use rapier3d::prelude::*;

use crate::scene::{Body, Collider as ColliderShape};
use crate::world::{Physics, Shape, WorldTransform};

/// What a ray met.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    pub point: Vec3,
    pub distance: f32,
    pub collider: ColliderRef,
}

/// A collider, as a ray reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColliderRef(pub ColliderHandle);

/// The handle rapier knows an entity by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyHandle(pub RigidBodyHandle);

/// Everything rapier needs to take a step.
pub struct PhysicsWorld {
    pub gravity: Vec3,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    parameters: IntegrationParameters,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd: CCDSolver,
    queries: QueryPipeline,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new(1.0 / 60.0)
    }
}

impl PhysicsWorld {
    /// `fixed_delta` must be the simulation clock's step, not a frame delta.
    pub fn new(fixed_delta: f32) -> Self {
        let parameters = IntegrationParameters {
            dt: fixed_delta,
            ..Default::default()
        };
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            parameters,
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            queries: QueryPipeline::new(),
        }
    }

    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }

    /// Build a rapier body for every entity that asks for one.
    ///
    /// Idempotent in the sense that entities already carrying a
    /// [`BodyHandle`] are skipped, so it can run again after more of a scene
    /// is spawned.
    pub fn sync_from_world(&mut self, world: &mut World) {
        let mut added: Vec<(hecs::Entity, BodyHandle)> = Vec::new();
        for (entity, placed, physics, shape, existing) in world
            .query::<(
                hecs::Entity,
                &WorldTransform,
                &Physics,
                &Shape,
                Option<&BodyHandle>,
            )>()
            .iter()
        {
            if existing.is_some() || physics.0 == Body::None {
                continue;
            }
            let Some(collider) = build_collider(shape.0, placed.0) else {
                // Declared solid with no shape to be solid with. Skipped
                // rather than guessed at — a box invented from a mesh's
                // bounds is the kind of default that is wrong quietly.
                continue;
            };

            let (scale, rotation, translation) = placed.0.to_scale_rotation_translation();
            let _ = scale;
            let position = Isometry::from_parts(
                nalgebra::Translation3::new(translation.x, translation.y, translation.z),
                nalgebra::Unit::new_normalize(nalgebra::Quaternion::new(
                    rotation.w, rotation.x, rotation.y, rotation.z,
                )),
            );
            let body = match physics.0 {
                Body::Dynamic => RigidBodyBuilder::dynamic(),
                _ => RigidBodyBuilder::fixed(),
            }
            .position(position)
            .build();

            let handle = self.bodies.insert(body);
            self.colliders
                .insert_with_parent(collider, handle, &mut self.bodies);
            added.push((entity, BodyHandle(handle)));
        }
        for (entity, handle) in added {
            let _ = world.insert_one(entity, handle);
        }
    }

    /// Take one step. Call it once per simulation step, never per frame.
    pub fn step(&mut self) {
        let gravity = vector![self.gravity.x, self.gravity.y, self.gravity.z];
        self.pipeline.step(
            &gravity,
            &self.parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd,
            Some(&mut self.queries),
            &(),
            &(),
        );
    }

    /// Copy every dynamic body's position back onto its entity.
    ///
    /// Only dynamic ones: a static body's transform belongs to the scene, and
    /// writing rapier's copy back over it would let rounding walk the world
    /// a fraction at a time.
    pub fn sync_to_world(&self, world: &mut World) {
        let mut moved: Vec<(hecs::Entity, glam::Mat4)> = Vec::new();
        for (entity, handle, physics, placed) in world
            .query::<(hecs::Entity, &BodyHandle, &Physics, &WorldTransform)>()
            .iter()
        {
            if physics.0 != Body::Dynamic {
                continue;
            }
            let Some(body) = self.bodies.get(handle.0) else {
                continue;
            };
            let position = body.position();
            let translation = Vec3::new(
                position.translation.x,
                position.translation.y,
                position.translation.z,
            );
            let rotation = Quat::from_xyzw(
                position.rotation.i,
                position.rotation.j,
                position.rotation.k,
                position.rotation.w,
            );
            // Rapier knows where a body is and how it is turned; it does not
            // know how big the thing being drawn is, because scale lives in
            // the collider's shape rather than the body. Keeping the
            // entity's own scale is the difference between a crate falling
            // and a crate falling while shrinking to a unit cube.
            let (scale, _, _) = placed.0.to_scale_rotation_translation();
            moved.push((
                entity,
                glam::Mat4::from_scale_rotation_translation(scale, rotation, translation),
            ));
        }
        for (entity, matrix) in moved {
            let _ = world.insert_one(entity, WorldTransform(matrix));
        }
    }

    /// The first thing a ray hits, as a point in the world and the distance
    /// to it.
    ///
    /// This is what mouse picking in an editor is built from, and what
    /// anything that needs to ask how far the ground is uses.
    pub fn cast_ray(&self, from: Vec3, direction: Vec3, max_distance: f32) -> Option<RayHit> {
        let direction = direction.normalize_or_zero();
        if direction.length_squared() < 0.5 {
            return None;
        }
        let ray = Ray::new(
            point![from.x, from.y, from.z],
            vector![direction.x, direction.y, direction.z],
        );
        let (collider, distance) = self.queries.cast_ray(
            &self.bodies,
            &self.colliders,
            &ray,
            max_distance,
            // Solid: a ray starting inside a shape stops at zero rather than
            // passing through to the far wall. A camera inside a rock should
            // report the rock.
            true,
            QueryFilter::default(),
        )?;
        Some(RayHit {
            point: from + direction * distance,
            distance,
            collider: ColliderRef(collider),
        })
    }

    /// Where a body is now, for tests and for anything that wants one
    /// position without walking the world.
    pub fn position(&self, handle: BodyHandle) -> Option<Vec3> {
        let body = self.bodies.get(handle.0)?;
        Some(Vec3::new(
            body.position().translation.x,
            body.position().translation.y,
            body.position().translation.z,
        ))
    }
}

/// Turn a scene's shape into a rapier collider, scaled by the transform.
fn build_collider(shape: ColliderShape, transform: glam::Mat4) -> Option<Collider> {
    let (scale, _, _) = transform.to_scale_rotation_translation();
    Some(match shape {
        ColliderShape::None => return None,
        ColliderShape::Box { half } => {
            let h = half * scale;
            ColliderBuilder::cuboid(h.x.max(1e-4), h.y.max(1e-4), h.z.max(1e-4)).build()
        }
        ColliderShape::Sphere { radius } => {
            // One radius, so a sphere scaled unevenly takes the largest —
            // a ball that is not a ball is an ellipsoid, and rapier's ball
            // cannot be one.
            let r = radius * scale.max_element();
            ColliderBuilder::ball(r.max(1e-4)).build()
        }
        ColliderShape::Capsule {
            half_height,
            radius,
        } => ColliderBuilder::capsule_y(
            (half_height * scale.y).max(1e-4),
            (radius * scale.x.max(scale.z)).max(1e-4),
        )
        .build(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Body, Collider as ColliderShape, EntityDesc, Scene, Transform};
    use crate::MeshHandle;

    fn entity(name: &str, y: f32, body: Body, collider: ColliderShape) -> EntityDesc {
        EntityDesc {
            name: name.into(),
            model: "m".into(),
            prefab: String::new(),
            transform: Transform {
                position: Vec3::new(0.0, y, 0.0),
                ..Default::default()
            },
            material: Default::default(),
            body,
            collider,
            children: Vec::new(),
        }
    }

    /// A ball above a floor, and the clock to drop it with.
    fn dropped(from: f32) -> (PhysicsWorld, World, hecs::Entity) {
        let scene = Scene {
            entities: vec![
                entity(
                    "floor",
                    0.0,
                    Body::Static,
                    ColliderShape::Box {
                        half: Vec3::new(20.0, 0.1, 20.0),
                    },
                ),
                entity(
                    "ball",
                    from,
                    Body::Dynamic,
                    ColliderShape::Sphere { radius: 0.5 },
                ),
            ],
            ..Default::default()
        };
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        physics.sync_from_world(&mut world);

        let ball = world
            .query::<(hecs::Entity, &Physics)>()
            .iter()
            .find(|(_, p)| p.0 == Body::Dynamic)
            .map(|(e, _)| e)
            .expect("the ball is the dynamic one");
        (physics, world, ball)
    }

    #[test]
    fn a_dropped_ball_lands_on_the_floor_instead_of_through_it() {
        let (mut physics, mut world, ball) = dropped(6.0);
        assert_eq!(physics.body_count(), 2);

        for _ in 0..240 {
            physics.step();
        }
        physics.sync_to_world(&mut world);

        let y = world.get::<&WorldTransform>(ball).unwrap().0.w_axis.y;
        // Floor's top is 0.1, ball's radius 0.5, so it rests at 0.6.
        assert!(
            (y - 0.6).abs() < 0.05,
            "the ball should come to rest on the floor, got y = {y}"
        );
    }

    #[test]
    fn a_falling_thing_keeps_the_size_the_scene_gave_it() {
        // Rapier knows where a body is, not how big the thing drawn from it
        // is — scale lives in the collider's shape. Taking the position back
        // without the scale makes everything dynamic snap to a unit cube on
        // the first step, which reads as the physics being wrong.
        let scene = Scene {
            entities: vec![crate::EntityDesc {
                name: "boulder".into(),
                model: "m".into(),
                prefab: String::new(),
                transform: crate::Transform {
                    position: Vec3::new(0.0, 4.0, 0.0),
                    scale: Vec3::splat(3.0),
                    ..Default::default()
                },
                material: Default::default(),
                body: Body::Dynamic,
                collider: ColliderShape::Sphere { radius: 0.5 },
                children: Vec::new(),
            }],
            ..Default::default()
        };
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        physics.sync_from_world(&mut world);
        for _ in 0..30 {
            physics.step();
        }
        physics.sync_to_world(&mut world);

        let placed = world
            .query::<(&WorldTransform, &Physics)>()
            .iter()
            .map(|(placed, _)| placed.0)
            .next()
            .unwrap();
        let (scale, _, translation) = placed.to_scale_rotation_translation();
        assert!(
            translation.y < 3.9,
            "it should have fallen, y = {}",
            translation.y
        );
        assert!(
            (scale - Vec3::splat(3.0)).length() < 1e-3,
            "and kept its size, got {scale}"
        );
    }

    #[test]
    fn a_static_body_stays_exactly_where_the_scene_put_it() {
        // Its transform is the scene's, and rapier only reads it. Writing
        // rapier's copy back would let rounding walk the world a fraction at
        // a time, which is invisible for an hour and then is not.
        let (mut physics, mut world, _) = dropped(6.0);
        let floor = world
            .query::<(hecs::Entity, &Physics)>()
            .iter()
            .find(|(_, p)| p.0 == Body::Static)
            .map(|(e, _)| e)
            .unwrap();
        let before = world.get::<&WorldTransform>(floor).unwrap().0;

        for _ in 0..120 {
            physics.step();
        }
        physics.sync_to_world(&mut world);

        assert_eq!(world.get::<&WorldTransform>(floor).unwrap().0, before);
    }

    #[test]
    fn declaring_a_body_without_a_shape_adds_nothing() {
        // A thing can be marked solid and have nothing to be solid with.
        // Inventing a box from the mesh's bounds would be a default that is
        // wrong quietly.
        let scene = Scene {
            entities: vec![entity("ghost", 1.0, Body::Dynamic, ColliderShape::None)],
            ..Default::default()
        };
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        physics.sync_from_world(&mut world);
        assert_eq!(physics.body_count(), 0);
    }

    #[test]
    fn syncing_twice_does_not_give_one_entity_two_bodies() {
        // `sync_from_world` runs again whenever more of a scene is spawned.
        let (mut physics, mut world, _) = dropped(3.0);
        physics.sync_from_world(&mut world);
        physics.sync_from_world(&mut world);
        assert_eq!(physics.body_count(), 2);
    }

    /// A floor, a wall, and a low kerb: something for a ray to find and for
    /// a falling body to land on.
    fn obstacle_course() -> (PhysicsWorld, World) {
        let scene = Scene {
            entities: vec![
                entity(
                    "floor",
                    -0.1,
                    Body::Static,
                    ColliderShape::Box {
                        half: Vec3::new(20.0, 0.1, 20.0),
                    },
                ),
                EntityDesc {
                    transform: Transform {
                        position: Vec3::new(2.0, 1.0, 0.0),
                        ..Default::default()
                    },
                    ..entity(
                        "wall",
                        0.0,
                        Body::Static,
                        ColliderShape::Box {
                            half: Vec3::new(0.2, 1.0, 5.0),
                        },
                    )
                },
                EntityDesc {
                    transform: Transform {
                        position: Vec3::new(-2.0, 0.1, 0.0),
                        ..Default::default()
                    },
                    ..entity(
                        "kerb",
                        0.0,
                        Body::Static,
                        ColliderShape::Box {
                            half: Vec3::new(0.5, 0.1, 5.0),
                        },
                    )
                },
            ],
            ..Default::default()
        };
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        physics.sync_from_world(&mut world);
        // The query pipeline is built by the step, so a world that has never
        // stepped answers no queries.
        physics.step();
        (physics, world)
    }

    #[test]
    fn a_ray_finds_the_ground_and_reports_how_far_it_is() {
        // What mouse picking is built from.
        let (physics, _) = obstacle_course();
        let hit = physics
            .cast_ray(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y, 100.0)
            .expect("straight down from above the floor");
        assert!(
            (hit.distance - 5.0).abs() < 0.05,
            "the floor's top is at y = 0, so the distance is 5, got {}",
            hit.distance
        );
        assert!((hit.point.y).abs() < 0.05);
    }

    #[test]
    fn a_ray_into_empty_sky_finds_nothing_rather_than_the_origin() {
        let (physics, _) = obstacle_course();
        assert!(physics
            .cast_ray(Vec3::new(0.0, 5.0, 0.0), Vec3::Y, 100.0)
            .is_none());
    }

    #[test]
    fn the_step_is_the_simulation_clocks_and_not_the_frames() {
        // Two worlds stepped the same number of times must agree exactly,
        // whatever the wall clock did in between. This is what save files,
        // replays and networked clients rest on.
        let (mut a, mut world_a, ball_a) = dropped(4.0);
        let (mut b, mut world_b, ball_b) = dropped(4.0);
        for _ in 0..90 {
            a.step();
        }
        for _ in 0..90 {
            b.step();
        }
        a.sync_to_world(&mut world_a);
        b.sync_to_world(&mut world_b);
        assert_eq!(
            world_a.get::<&WorldTransform>(ball_a).unwrap().0,
            world_b.get::<&WorldTransform>(ball_b).unwrap().0
        );
    }
}
