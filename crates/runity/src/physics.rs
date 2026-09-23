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
//! * **The file and the solver share a body without fighting.** A body
//!   remembers what it was built from. A different shape or body kind in the
//!   entity — a live reload, a game changing it — rebuilds it; a different
//!   transform — someone moved it in the scene, or the game teleported it —
//!   moves it there. An entity that is gone takes its body with it. So a
//!   reload patches a running simulation instead of fighting it.
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

use crate::scene::{Body, Collider as ColliderShape, Transform};
use crate::world::{Jointed, Layer, Parent, Physics, Props, SceneId, Shape, WorldTransform};

/// Who is touching an entity's body: for a [`Body::Trigger`], what is
/// inside it; for a solid body, what it is in contact with.
///
/// A trigger gets one when its body is built. Any other body gets one when
/// the game inserts `Contacts::default()` on the entity — contacts are
/// tracked only where someone asked, because most of a scene never needs
/// to know. Rewritten after every step: `entered` and `left` are this
/// step's changes, so a system that runs every step sees each exactly once.
/// Unity's `OnTriggerEnter`/`OnCollisionEnter`, as data a system queries
/// rather than callbacks on a class.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contacts {
    /// Everything touching now, in no particular order.
    pub inside: Vec<hecs::Entity>,
    /// What started touching this step.
    pub entered: Vec<hecs::Entity>,
    /// What stopped touching this step — possibly an entity that is gone.
    pub left: Vec<hecs::Entity>,
}

/// What a ray met.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    pub point: Vec3,
    pub distance: f32,
    pub collider: ColliderRef,
    /// The entity whose body it hit — what a game wants to know: which
    /// crate, which door. `None` only for a body no entity asked for.
    pub entity: Option<hecs::Entity>,
}

/// A collider, as a ray reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColliderRef(pub ColliderHandle);

/// The handle rapier knows an entity by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyHandle(pub RigidBodyHandle);

/// What a body was built from, and the transform it last agreed with: how
/// a change from outside the solver is told from the solver's own motion.
#[derive(Debug, Clone, PartialEq)]
struct Built {
    body: Body,
    collider: ColliderShape,
    local: Transform,
    /// Which [`CollisionMesh`] a `Model` collider was built from.
    mesh: usize,
    props: crate::scene::BodyProps,
    layer: String,
}

/// A model's geometry, for a [`ColliderShape::Model`]. Attached to the
/// entity by whoever knows the mesh — [`attach_collision_meshes`] — because
/// the physics world never sees a library.
#[derive(Debug, Clone)]
pub struct CollisionMesh {
    pub vertices: std::sync::Arc<Vec<Vec3>>,
    pub triangles: std::sync::Arc<Vec<[u32; 3]>>,
}

impl CollisionMesh {
    pub fn of(mesh: &crate::asset::MeshAsset) -> Self {
        Self::from_parts(
            mesh.vertices.iter().map(|v| Vec3::from_array(v.position)),
            mesh.indices.iter().copied(),
        )
    }

    pub fn of_archived(mesh: &crate::asset::ArchivedMeshAsset) -> Self {
        Self::from_parts(
            mesh.vertices.iter().map(|v| {
                Vec3::new(
                    v.position[0].to_native(),
                    v.position[1].to_native(),
                    v.position[2].to_native(),
                )
            }),
            mesh.indices.iter().map(|i| i.to_native()),
        )
    }

    fn from_parts(
        vertices: impl Iterator<Item = Vec3>,
        indices: impl Iterator<Item = u32>,
    ) -> Self {
        let indices: Vec<u32> = indices.collect();
        Self {
            vertices: std::sync::Arc::new(vertices.collect()),
            triangles: std::sync::Arc::new(
                indices
                    .chunks_exact(3)
                    .map(|t| [t[0], t[1], t[2]])
                    .collect(),
            ),
        }
    }

    fn key(&self) -> usize {
        std::sync::Arc::as_ptr(&self.vertices) as usize
    }
}

/// The geometry for a model name: a builtin, or a mesh in the library.
pub fn collision_mesh_for(model: &str, library: Option<&crate::Library>) -> Option<CollisionMesh> {
    match crate::builtin::by_name(model) {
        Some(mesh) => Some(CollisionMesh::of(&mesh)),
        None => Some(CollisionMesh::of_archived(library?.mesh_by_name(model)?)),
    }
}

/// Give every entity whose collider is its model the geometry of that
/// model. `lines` pairs entities with the scene lines they came from; one
/// mesh per model name is made and shared.
pub fn attach_collision_meshes<'a>(
    world: &mut World,
    lines: impl IntoIterator<Item = (hecs::Entity, &'a crate::EntityDesc)>,
    library: Option<&crate::Library>,
) {
    let mut made: std::collections::HashMap<&str, Option<CollisionMesh>> = Default::default();
    for (entity, desc) in lines {
        if desc.collider != ColliderShape::Model {
            continue;
        }
        let mesh = made
            .entry(desc.model.as_str())
            .or_insert_with(|| collision_mesh_for(&desc.model, library))
            .clone();
        match mesh {
            Some(mesh) => {
                let _ = world.insert_one(entity, mesh);
            }
            None => {
                let _ = world.remove_one::<CollisionMesh>(entity);
            }
        }
    }
}

/// [`attach_collision_meshes`] for the entities a scene spawned, found by
/// their [`crate::SceneId`].
pub fn attach_scene_collision_meshes(
    world: &mut World,
    scene: &crate::Scene,
    library: Option<&crate::Library>,
) {
    let lines: std::collections::HashMap<crate::EntityId, &crate::EntityDesc> = scene
        .flatten()
        .into_iter()
        .map(|(d, _)| (d.id, d))
        .collect();
    let pairs: Vec<(hecs::Entity, &crate::EntityDesc)> = world
        .query::<(hecs::Entity, &crate::SceneId)>()
        .iter()
        .filter_map(|(entity, id)| lines.get(&id.0).map(|d| (entity, *d)))
        .collect();
    attach_collision_meshes(world, pairs, library);
}

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
    /// A fixed body with no shape, for joints to the world to hang from.
    /// Made the first time one asks.
    ground: Option<RigidBodyHandle>,
    layers: crate::layers::Layers,
}

/// The joint a body was given, and what it was built between, so a change
/// to either end rebuilds it.
struct JointBuilt {
    joint: crate::scene::Joint,
    handle: ImpulseJointHandle,
    bodies: (RigidBodyHandle, RigidBodyHandle),
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
            ground: None,
            layers: crate::layers::Layers::default(),
        }
    }

    /// Use a project's collision layers ([`crate::layers`]): bodies already
    /// built are moved onto them at once, so a changed `layers.ron` takes
    /// effect in a running game.
    pub fn set_layers(&mut self, layers: crate::layers::Layers, world: &World) {
        self.layers = layers;
        for (_, collider) in self.colliders.iter_mut() {
            let Some(entity) = hecs::Entity::from_bits(collider.user_data as u64) else {
                continue;
            };
            let name = world
                .get::<&Layer>(entity)
                .map(|l| l.0.clone())
                .unwrap_or_default();
            let groups = groups(&self.layers, &name);
            collider.set_collision_groups(groups);
            collider.set_solver_groups(groups);
        }
    }

    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }

    /// Bring rapier in line with the world: build a body for every entity
    /// that asks for one, rebuild the ones whose body kind or shape changed,
    /// move the ones whose transform was changed from outside the solver,
    /// and remove the ones whose entity is gone or no longer asks.
    ///
    /// Cheap when nothing changed, so it runs every step.
    pub fn sync_from_world(&mut self, world: &mut World) {
        // Gone, or changed into something else: the old body goes.
        let mut stale: Vec<hecs::Entity> = Vec::new();
        let mut teleport: Vec<(hecs::Entity, RigidBodyHandle, glam::Mat4, Transform, Body)> =
            Vec::new();
        let mut live: std::collections::HashSet<RigidBodyHandle> = Default::default();
        for (entity, handle, built, physics, shape, local, placed, mesh, props, layer) in world
            .query::<(
                hecs::Entity,
                &BodyHandle,
                &Built,
                &Physics,
                &Shape,
                &Transform,
                &WorldTransform,
                Option<&CollisionMesh>,
                Option<&Props>,
                Option<&Layer>,
            )>()
            .iter()
        {
            let mesh = mesh.map_or(0, CollisionMesh::key);
            let props = props.map(|p| p.0).unwrap_or_default();
            let layer = layer.map(|l| l.0.as_str()).unwrap_or("");
            if built.body != physics.0
                || built.collider != shape.0
                || built.mesh != mesh
                || built.props != props
                || built.layer != layer
            {
                stale.push(entity);
            } else {
                live.insert(handle.0);
                if built.local != *local {
                    teleport.push((entity, handle.0, placed.0, *local, physics.0));
                }
            }
        }
        for entity in stale {
            let _ = world.remove::<(BodyHandle, Built)>(entity);
        }
        let orphans: Vec<RigidBodyHandle> = self
            .bodies
            .iter()
            .map(|(handle, _)| handle)
            .filter(|handle| !live.contains(handle) && Some(*handle) != self.ground)
            .collect();
        for handle in orphans {
            self.bodies.remove(
                handle,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                true,
            );
        }
        for (entity, handle, placed, local, kind) in teleport {
            if let Some(body) = self.bodies.get_mut(handle) {
                match kind {
                    // Moved, not teleported: the solver sees the motion
                    // over the step and pushes what is in the way.
                    Body::Kinematic | Body::Trigger => {
                        body.set_next_kinematic_position(isometry(placed));
                    }
                    _ => {
                        body.set_position(isometry(placed), true);
                        body.set_linvel(vector![0.0, 0.0, 0.0], true);
                        body.set_angvel(vector![0.0, 0.0, 0.0], true);
                    }
                }
            }
            if let Ok(mut built) = world.get::<&mut Built>(entity) {
                built.local = local;
            }
        }

        let mut added: Vec<(hecs::Entity, BodyHandle, Built)> = Vec::new();
        for (entity, placed, physics, shape, local, existing, mesh, props, layer) in world
            .query::<(
                hecs::Entity,
                &WorldTransform,
                &Physics,
                &Shape,
                &Transform,
                Option<&BodyHandle>,
                Option<&CollisionMesh>,
                Option<&Props>,
                Option<&Layer>,
            )>()
            .iter()
        {
            let props = props.map(|p| p.0).unwrap_or_default();
            let layer = layer.map(|l| l.0.clone()).unwrap_or_default();
            if existing.is_some() || physics.0 == Body::None {
                continue;
            }
            let dynamic = physics.0 == Body::Dynamic;
            let Some(mut collider) = build_collider(shape.0, placed.0, mesh, dynamic) else {
                // Declared solid with no shape to be solid with. Skipped
                // rather than guessed at — a box invented from a mesh's
                // bounds is the kind of default that is wrong quietly.
                continue;
            };
            // Which entity a collider is, for contacts to be told in
            // entities rather than rapier handles.
            collider.user_data = entity.to_bits().get() as u128;
            collider.set_friction(props.friction.max(0.0));
            collider.set_restitution(props.bounce.clamp(0.0, 1.0));
            // The bouncier of the two decides, so a ball bounces off any
            // floor; grip stays the average of both, as everywhere.
            collider.set_restitution_combine_rule(CoefficientCombineRule::Max);
            let layered = groups(&self.layers, &layer);
            collider.set_collision_groups(layered);
            collider.set_solver_groups(layered);
            collider.set_density(props.density.max(1e-3));
            if physics.0 == Body::Trigger {
                collider.set_sensor(true);
                // A zone notices whatever enters it, a kinematic player or
                // a static crate included — not only what the solver moves.
                collider.set_active_collision_types(ActiveCollisionTypes::all());
            }
            let body = match physics.0 {
                Body::Dynamic => RigidBodyBuilder::dynamic(),
                Body::Kinematic | Body::Trigger => RigidBodyBuilder::kinematic_position_based(),
                _ => RigidBodyBuilder::fixed(),
            }
            .position(isometry(placed.0))
            .build();
            let handle = self.bodies.insert(body);
            self.colliders
                .insert_with_parent(collider, handle, &mut self.bodies);
            added.push((
                entity,
                BodyHandle(handle),
                Built {
                    body: physics.0,
                    collider: shape.0,
                    local: *local,
                    mesh: mesh.map_or(0, CollisionMesh::key),
                    props,
                    layer,
                },
            ));
        }
        for (entity, handle, built) in added {
            let trigger = built.body == Body::Trigger;
            let _ = world.insert(entity, (handle, built));
            if trigger && world.get::<&Contacts>(entity).is_err() {
                let _ = world.insert_one(entity, Contacts::default());
            }
        }
        self.sync_joints(world);
    }

    /// Joints after bodies: build each once both of its bodies exist,
    /// rebuild it when the joint or either body changed, drop it when its
    /// entity no longer asks for one. A joint whose partner is not there —
    /// not built yet, or a typo'd id — waits rather than guessing.
    fn sync_joints(&mut self, world: &mut World) {
        let bodies_by_id: std::collections::HashMap<crate::id::EntityId, RigidBodyHandle> = world
            .query::<(&SceneId, &BodyHandle)>()
            .iter()
            .map(|(id, handle)| (id.0, handle.0))
            .collect();
        let mut drop: Vec<hecs::Entity> = Vec::new();
        let mut build: Vec<(
            hecs::Entity,
            crate::scene::Joint,
            RigidBodyHandle,
            glam::Mat4,
        )> = Vec::new();
        for (entity, joint, body, placed, built) in world
            .query::<(
                hecs::Entity,
                Option<&Jointed>,
                Option<&BodyHandle>,
                &WorldTransform,
                Option<&JointBuilt>,
            )>()
            .iter()
        {
            let wanted = joint.zip(body).map(|(joint, body)| (joint.0, body.0));
            let partner = |joint: &crate::scene::Joint| -> Option<RigidBodyHandle> {
                match joint.to() {
                    Some(to) if !to.is_unassigned() => bodies_by_id.get(&to).copied(),
                    _ => self.ground,
                }
            };
            if let Some(built) = built {
                let still = wanted.is_some_and(|(joint, body)| {
                    joint == built.joint
                        && body == built.bodies.1
                        && partner(&joint).is_none_or(|p| p == built.bodies.0)
                        && self.impulse_joints.get(built.handle).is_some()
                });
                if still {
                    continue;
                }
                drop.push(entity);
            }
            if let Some((joint, body)) = wanted {
                build.push((entity, joint, body, placed.0));
            }
        }
        for entity in drop {
            if let Ok(built) = world.remove_one::<JointBuilt>(entity) {
                self.impulse_joints.remove(built.handle, true);
            }
        }
        for (entity, joint, body, placed) in build {
            let other = match joint.to() {
                Some(to) if !to.is_unassigned() => match bodies_by_id.get(&to) {
                    Some(handle) => *handle,
                    None => continue,
                },
                _ => *self
                    .ground
                    .get_or_insert_with(|| self.bodies.insert(RigidBodyBuilder::fixed().build())),
            };
            let (Some(one), Some(two)) = (self.bodies.get(other), self.bodies.get(body)) else {
                continue;
            };
            let data = joint_data(&joint, placed, one.position(), two.position());
            let Some(data) = data else {
                continue;
            };
            let handle = self.impulse_joints.insert(other, body, data, true);
            let _ = world.insert_one(
                entity,
                JointBuilt {
                    joint,
                    handle,
                    bodies: (other, body),
                },
            );
        }
    }

    /// Rewrite every [`Contacts`] from what rapier found this step. Part of
    /// [`PhysicsWorld::run`]; call it after [`PhysicsWorld::step`] when
    /// stepping by hand.
    pub fn update_contacts(&self, world: &mut World) {
        let entity_of = |collider: ColliderHandle| {
            self.colliders
                .get(collider)
                .and_then(|c| hecs::Entity::from_bits(c.user_data as u64))
        };
        for (entity, handle, contacts) in world
            .query_mut::<(hecs::Entity, &BodyHandle, &mut Contacts)>()
            .into_iter()
        {
            let Some(body) = self.bodies.get(handle.0) else {
                continue;
            };
            let mut now: Vec<hecs::Entity> = Vec::new();
            for &mine in body.colliders() {
                let other = |a: ColliderHandle, b: ColliderHandle| if a == mine { b } else { a };
                for (a, b, touching) in self.narrow_phase.intersection_pairs_with(mine) {
                    if touching {
                        now.extend(entity_of(other(a, b)));
                    }
                }
                for pair in self.narrow_phase.contact_pairs_with(mine) {
                    if pair.has_any_active_contact {
                        now.extend(entity_of(other(pair.collider1, pair.collider2)));
                    }
                }
            }
            now.retain(|e| *e != entity);
            now.sort();
            now.dedup();
            contacts.entered = now
                .iter()
                .filter(|e| !contacts.inside.contains(e))
                .copied()
                .collect();
            contacts.left = contacts
                .inside
                .iter()
                .filter(|e| !now.contains(e))
                .copied()
                .collect();
            contacts.inside = now;
        }
    }

    /// One fixed step of physics as a system: bring rapier in line with the
    /// world, step, and write where the dynamic bodies went back. Call it
    /// once per simulation step.
    pub fn run(&mut self, world: &mut World) {
        self.sync_from_world(world);
        self.step();
        self.sync_to_world(world);
        self.update_contacts(world);
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
        let mut moved: Vec<(hecs::Entity, glam::Mat4, Option<hecs::Entity>)> = Vec::new();
        for (entity, handle, physics, placed, parent) in world
            .query::<(
                hecs::Entity,
                &BodyHandle,
                &Physics,
                &WorldTransform,
                Option<&Parent>,
            )>()
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
                parent.map(|p| p.0),
            ));
        }
        for (entity, matrix, parent) in moved {
            // The local transform too, relative to the parent: it is what
            // the hierarchy is recomputed from, and what a reload compares
            // with the file. Writing only the world one would let the next
            // hierarchy pass put the body back where the scene had it.
            let parent_matrix = parent
                .and_then(|p| world.get::<&WorldTransform>(p).ok().map(|w| w.0))
                .unwrap_or(glam::Mat4::IDENTITY);
            let local_matrix = parent_matrix.inverse() * matrix;
            let (scale, rotation, translation) = local_matrix.to_scale_rotation_translation();
            let mut local = Transform {
                position: translation,
                scale,
                ..Transform::default()
            };
            local.set_rotation(rotation);
            let _ = world.insert_one(entity, WorldTransform(matrix));
            let _ = world.insert_one(entity, local);
            if let Ok(mut built) = world.get::<&mut Built>(entity) {
                built.local = local;
            }
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
            // A trigger is a zone, not a surface: a ray passes through it.
            QueryFilter::default().exclude_sensors(),
        )?;
        Some(RayHit {
            point: from + direction * distance,
            distance,
            collider: ColliderRef(collider),
            entity: self.entity_of(collider),
        })
    }

    /// The first thing a ray hits, with the surface's normal there. With
    /// `statics_only`, bodies that move — dynamic or kinematic — are looked
    /// through: what a navigation bake wants, since a crate on the floor is
    /// not the floor. Triggers are always looked through.
    pub fn cast_ray_with_normal(
        &self,
        from: Vec3,
        direction: Vec3,
        max_distance: f32,
        statics_only: bool,
    ) -> Option<(Vec3, Vec3, f32)> {
        let direction = direction.normalize_or_zero();
        if direction.length_squared() < 0.5 {
            return None;
        }
        let ray = Ray::new(
            point![from.x, from.y, from.z],
            vector![direction.x, direction.y, direction.z],
        );
        let filter = if statics_only {
            QueryFilter::only_fixed()
        } else {
            QueryFilter::default()
        }
        .exclude_sensors();
        let (_, hit) = self.queries.cast_ray_and_get_normal(
            &self.bodies,
            &self.colliders,
            &ray,
            max_distance,
            true,
            filter,
        )?;
        Some((
            from + direction * hit.time_of_impact,
            Vec3::new(hit.normal.x, hit.normal.y, hit.normal.z),
            hit.time_of_impact,
        ))
    }

    /// [`PhysicsWorld::cast_ray`], seeing only bodies on the named layers:
    /// the ground under a player's feet without the debris at them.
    pub fn cast_ray_among(
        &self,
        from: Vec3,
        direction: Vec3,
        max_distance: f32,
        layers: &[&str],
    ) -> Option<RayHit> {
        let direction = direction.normalize_or_zero();
        if direction.length_squared() < 0.5 {
            return None;
        }
        let ray = Ray::new(
            point![from.x, from.y, from.z],
            vector![direction.x, direction.y, direction.z],
        );
        let only = InteractionGroups::new(
            Group::ALL,
            Group::from_bits_truncate(self.layers.mask(layers)),
        );
        let (collider, distance) = self.queries.cast_ray(
            &self.bodies,
            &self.colliders,
            &ray,
            max_distance,
            true,
            QueryFilter::default().exclude_sensors().groups(only),
        )?;
        Some(RayHit {
            point: from + direction * distance,
            distance,
            collider: ColliderRef(collider),
            entity: self.entity_of(collider),
        })
    }

    /// Which entity a collider belongs to.
    fn entity_of(&self, collider: ColliderHandle) -> Option<hecs::Entity> {
        self.colliders
            .get(collider)
            .and_then(|c| hecs::Entity::from_bits(c.user_data as u64))
    }

    /// Every entity whose shape overlaps a ball — Unity's `OverlapSphere`:
    /// what an explosion reaches, what is within grabbing distance.
    /// Triggers are left out; they are zones, not things. Sorted, so the
    /// same world gives the same answer.
    pub fn overlap_sphere(&self, centre: Vec3, radius: f32) -> Vec<hecs::Entity> {
        let ball = Ball::new(radius.max(0.0));
        let at = Isometry::translation(centre.x, centre.y, centre.z);
        let mut found = Vec::new();
        self.queries.intersections_with_shape(
            &self.bodies,
            &self.colliders,
            &at,
            &ball,
            QueryFilter::default().exclude_sensors(),
            |collider| {
                found.extend(self.entity_of(collider));
                true
            },
        );
        found.sort();
        found.dedup();
        found
    }

    /// Bring ray queries up to date with the bodies, without a step: after
    /// [`PhysicsWorld::sync_from_world`], before asking where things are.
    pub fn refresh_queries(&mut self) {
        self.queries.update(&self.colliders);
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

/// A layer's rapier groups.
fn groups(layers: &crate::layers::Layers, name: &str) -> InteractionGroups {
    let (member, filter) = layers.groups(name);
    InteractionGroups::new(
        Group::from_bits_truncate(member),
        Group::from_bits_truncate(filter),
    )
}

/// A scene joint as rapier's, between a body at `one` and the jointed body
/// at `two`, from where the jointed entity stands now.
///
/// Both ends get the same frame in the world — at the anchor, its X along
/// the joint's axis — expressed in each body's own frame, so the joint
/// holds the two where they are at the moment it is made. The bodies do
/// not collide with each other: a door rubbing on its own frame is jitter,
/// not physics.
fn joint_data(
    joint: &crate::scene::Joint,
    placed: glam::Mat4,
    one: &Isometry<Real>,
    two: &Isometry<Real>,
) -> Option<GenericJoint> {
    use crate::scene::Joint;
    let (_, rotation, _) = placed.to_scale_rotation_translation();
    let (anchor, axis) = match *joint {
        Joint::None => return None,
        Joint::Fixed { .. } => (Vec3::ZERO, Vec3::X),
        Joint::Hinge { anchor, axis, .. } => (anchor, axis),
        Joint::Ball { anchor, .. } => (anchor, Vec3::X),
        Joint::Slider { axis, .. } => (Vec3::ZERO, axis),
    };
    let at = placed.transform_point3(anchor);
    let along = (rotation * axis).normalize_or_zero();
    if along == Vec3::ZERO {
        return None;
    }
    let turn = Quat::from_rotation_arc(Vec3::X, along);
    let frame = Isometry::from_parts(
        nalgebra::Translation3::new(at.x, at.y, at.z),
        nalgebra::Unit::new_normalize(nalgebra::Quaternion::new(turn.w, turn.x, turn.y, turn.z)),
    );
    let locked = match joint {
        Joint::Fixed { .. } => JointAxesMask::LOCKED_FIXED_AXES,
        Joint::Hinge { .. } => JointAxesMask::LOCKED_REVOLUTE_AXES,
        Joint::Ball { .. } => JointAxesMask::LOCKED_SPHERICAL_AXES,
        Joint::Slider { .. } => JointAxesMask::LOCKED_PRISMATIC_AXES,
        Joint::None => return None,
    };
    let mut builder = GenericJointBuilder::new(locked)
        .local_frame1(one.inverse() * frame)
        .local_frame2(two.inverse() * frame)
        .contacts_enabled(false);
    match *joint {
        Joint::Hinge {
            limits_deg: Some((low, high)),
            ..
        } => {
            builder = builder.limits(JointAxis::AngX, [low.to_radians(), high.to_radians()]);
        }
        Joint::Slider {
            limits: Some((low, high)),
            ..
        } => {
            builder = builder.limits(JointAxis::LinX, [low, high]);
        }
        _ => {}
    }
    Some(builder.build())
}

/// Where a world matrix puts a body: its translation and rotation. Scale
/// lives in the collider's shape.
fn isometry(placed: glam::Mat4) -> Isometry<Real> {
    let (_, rotation, translation) = placed.to_scale_rotation_translation();
    Isometry::from_parts(
        nalgebra::Translation3::new(translation.x, translation.y, translation.z),
        nalgebra::Unit::new_normalize(nalgebra::Quaternion::new(
            rotation.w, rotation.x, rotation.y, rotation.z,
        )),
    )
}

/// Turn a scene's shape into a rapier collider, scaled by the transform.
fn build_collider(
    shape: ColliderShape,
    transform: glam::Mat4,
    mesh: Option<&CollisionMesh>,
    dynamic: bool,
) -> Option<Collider> {
    let (scale, _, _) = transform.to_scale_rotation_translation();
    Some(match shape {
        ColliderShape::None => return None,
        ColliderShape::Model => {
            // No geometry yet — the model is not imported, or nobody
            // attached it: no collider, and the next sync tries again.
            let mesh = mesh?;
            let points: Vec<Point<Real>> = mesh
                .vertices
                .iter()
                .map(|v| {
                    let v = *v * scale;
                    point![v.x, v.y, v.z]
                })
                .collect();
            if dynamic {
                ColliderBuilder::convex_hull(&points)?.build()
            } else {
                ColliderBuilder::trimesh(points, mesh.triangles.to_vec())
                    .ok()?
                    .build()
            }
        }
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
        ColliderShape::Cylinder {
            half_height,
            radius,
        } => ColliderBuilder::cylinder(
            (half_height * scale.y).max(1e-4),
            (radius * scale.x.max(scale.z)).max(1e-4),
        )
        .build(),
        ColliderShape::Ramp { half } => {
            let h = half * scale;
            let points: Vec<Point<Real>> = [
                (-1.0, -1.0, -1.0),
                (1.0, -1.0, -1.0),
                (1.0, -1.0, 1.0),
                (-1.0, -1.0, 1.0),
                (-1.0, 1.0, -1.0),
                (1.0, 1.0, -1.0),
            ]
            .into_iter()
            .map(|(x, y, z)| point![x * h.x, y * h.y, z * h.z])
            .collect();
            ColliderBuilder::convex_hull(&points)?.build()
        }
        ColliderShape::Stairs { half, steps } => {
            // One box per step, each from the ground up, as the mesh has
            // them: a flight a body can climb, not a slope it slides on.
            let h = half * scale;
            let steps = steps.max(1);
            let depth = 2.0 * h.z / steps as f32;
            let parts = (0..steps)
                .map(|i| {
                    let height = 2.0 * h.y * (i + 1) as f32 / steps as f32;
                    let z = h.z - depth * (i as f32 + 0.5);
                    let y = -h.y + height * 0.5;
                    (
                        Isometry::translation(0.0, y, z),
                        SharedShape::cuboid(
                            h.x.max(1e-4),
                            (height * 0.5).max(1e-4),
                            (depth * 0.5).max(1e-4),
                        ),
                    )
                })
                .collect();
            ColliderBuilder::compound(parts).build()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Body, Collider as ColliderShape, EntityDesc, Scene, Transform};
    use crate::MeshHandle;

    fn entity(name: &str, y: f32, body: Body, collider: ColliderShape) -> EntityDesc {
        EntityDesc {
            camera: None,
            layer: Default::default(),
            physics: Default::default(),
            joint: Default::default(),
            overrides: Default::default(),
            components: Default::default(),
            id: Default::default(),
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
                camera: None,
                layer: Default::default(),
                physics: Default::default(),
                joint: Default::default(),
                overrides: Default::default(),
                components: Default::default(),
                id: Default::default(),
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

    // --- physics as a system, under live reload -----------------------------

    /// Physics and the hierarchy pass, the way a game's step runs them.
    fn run_for(physics: &mut PhysicsWorld, world: &mut World, steps: usize) {
        for _ in 0..steps {
            physics.run(world);
            crate::world::apply_hierarchy(world);
        }
    }

    #[test]
    fn a_falling_body_is_not_put_back_by_the_hierarchy_pass() {
        // The hierarchy is recomputed from local transforms; if physics wrote
        // only the world one, the ball would snap back up every step.
        let (mut physics, mut world, ball) = dropped(5.0);
        run_for(&mut physics, &mut world, 30);
        let local = world.get::<&Transform>(ball).unwrap().position.y;
        assert!(
            local < 4.5,
            "it fell, and its local transform says so: {local}"
        );
    }

    #[test]
    fn a_body_moved_in_the_file_is_moved_in_the_simulation() {
        let (mut physics, mut world, ball) = dropped(5.0);
        run_for(&mut physics, &mut world, 5);
        // A reload wrote a new place for it.
        world
            .insert_one(
                ball,
                Transform {
                    position: Vec3::new(8.0, 3.0, 0.0),
                    ..Transform::default()
                },
            )
            .unwrap();
        crate::world::apply_hierarchy(&mut world);
        run_for(&mut physics, &mut world, 1);
        let at = world.get::<&WorldTransform>(ball).unwrap().0.w_axis;
        assert!((at.x - 8.0).abs() < 0.01, "moved across: {at:?}");
        assert!(at.y < 3.0 && at.y > 2.5, "and falling from there: {at:?}");
    }

    #[test]
    fn a_despawned_body_leaves_no_ghost_collider() {
        let (mut physics, mut world, ball) = dropped(5.0);
        assert_eq!(physics.body_count(), 2);
        let floor = world
            .query::<(hecs::Entity, &Physics)>()
            .iter()
            .find(|(_, p)| p.0 == Body::Static)
            .map(|(e, _)| e)
            .unwrap();
        world.despawn(floor).unwrap();
        run_for(&mut physics, &mut world, 120);
        assert_eq!(physics.body_count(), 1, "the floor's body went with it");
        let y = world.get::<&Transform>(ball).unwrap().position.y;
        assert!(y < 0.0, "and the ball falls through where it was: {y}");
    }

    #[test]
    fn a_changed_shape_rebuilds_the_body() {
        let (mut physics, mut world, ball) = dropped(0.6);
        let before = *world.get::<&BodyHandle>(ball).unwrap();
        world
            .insert_one(ball, Shape(ColliderShape::Sphere { radius: 2.0 }))
            .unwrap();
        run_for(&mut physics, &mut world, 1);
        let after = *world.get::<&BodyHandle>(ball).unwrap();
        assert_ne!(before, after, "a new body for the new shape");
        assert_eq!(
            physics.body_count(),
            2,
            "and not a second one beside the old"
        );
    }

    // --- blockout colliders --------------------------------------------------

    /// A static shape at the origin and a dynamic thing above it.
    fn over(
        ground: ColliderShape,
        scale: Vec3,
        thing: ColliderShape,
        at: Vec3,
    ) -> (PhysicsWorld, World, hecs::Entity) {
        let mut floor = entity("shape", 0.0, Body::Static, ground);
        floor.transform.scale = scale;
        let mut dropped = entity("thing", 0.0, Body::Dynamic, thing);
        dropped.transform.position = at;
        let scene = Scene {
            entities: vec![floor, dropped],
            ..Default::default()
        };
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        let physics = PhysicsWorld::new(1.0 / 60.0);
        let thing = world
            .query::<(hecs::Entity, &Physics)>()
            .iter()
            .find(|(_, p)| p.0 == Body::Dynamic)
            .map(|(e, _)| e)
            .unwrap();
        (physics, world, thing)
    }

    #[test]
    fn a_ball_on_a_ramp_rolls_down_toward_the_low_edge() {
        let (mut physics, mut world, ball) = over(
            ColliderShape::Ramp {
                half: Vec3::splat(0.5),
            },
            Vec3::new(4.0, 2.0, 4.0),
            ColliderShape::Sphere { radius: 0.2 },
            Vec3::new(0.0, 1.5, -1.0),
        );
        run_for(&mut physics, &mut world, 60);
        let at = world.get::<&Transform>(ball).unwrap().position;
        assert!(at.z > -0.5, "it rolled toward the front: {at:?}");
        assert!(at.y < 1.5, "and down: {at:?}");
    }

    #[test]
    fn a_box_dropped_on_stairs_rests_on_a_step() {
        // Four steps in a 2 m cube, a quarter of the height each; the top
        // step's surface is at y = 1.
        let (mut physics, mut world, block) = over(
            ColliderShape::Stairs {
                half: Vec3::splat(0.5),
                steps: 4,
            },
            Vec3::splat(2.0),
            ColliderShape::Box {
                half: Vec3::splat(0.1),
            },
            Vec3::new(0.0, 2.0, -0.75),
        );
        run_for(&mut physics, &mut world, 120);
        let y = world.get::<&Transform>(block).unwrap().position.y;
        assert!(
            (y - 1.1).abs() < 0.05,
            "on the top step, not inside the flight: {y}"
        );
    }

    #[test]
    fn a_cylinder_is_as_tall_as_it_says() {
        let (mut physics, mut world, block) = over(
            ColliderShape::Cylinder {
                half_height: 0.5,
                radius: 0.5,
            },
            Vec3::new(1.0, 3.0, 1.0),
            ColliderShape::Box {
                half: Vec3::splat(0.1),
            },
            Vec3::new(0.0, 3.0, 0.0),
        );
        run_for(&mut physics, &mut world, 120);
        let y = world.get::<&Transform>(block).unwrap().position.y;
        assert!((y - 1.6).abs() < 0.05, "on the top of a 3 m pillar: {y}");
    }

    #[test]
    fn a_model_collider_is_the_models_own_shape() {
        // A static ramp that collides as the ramp it draws, and a dynamic
        // cube that collides as its convex hull.
        let mut ramp = entity("ramp", 0.0, Body::Static, ColliderShape::Model);
        ramp.model = "builtin:ramp".into();
        ramp.transform.scale = Vec3::new(4.0, 2.0, 4.0);
        let mut ball = entity(
            "ball",
            0.0,
            Body::Dynamic,
            ColliderShape::Sphere { radius: 0.2 },
        );
        ball.transform.position = Vec3::new(0.0, 1.5, -1.0);
        let scene = Scene {
            entities: vec![ramp, ball],
            ..Default::default()
        };
        let mut scene = scene;
        scene.assign_ids();
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        run_for(&mut physics, &mut world, 1);
        assert_eq!(
            physics.body_count(),
            1,
            "no geometry attached: no ramp body yet"
        );

        attach_scene_collision_meshes(&mut world, &scene, None);
        run_for(&mut physics, &mut world, 60);
        assert_eq!(physics.body_count(), 2, "and now there is");
        let ball = world
            .query::<(hecs::Entity, &Physics)>()
            .iter()
            .find(|(_, p)| p.0 == Body::Dynamic)
            .map(|(e, _)| e)
            .unwrap();
        let at = world.get::<&Transform>(ball).unwrap().position;
        assert!(
            at.z > -0.5 && at.y < 1.5,
            "rolled down the modelled slope: {at:?}"
        );
    }

    /// The one entity with this kind of body.
    fn the(world: &World, kind: Body) -> hecs::Entity {
        world
            .query::<(hecs::Entity, &Physics)>()
            .iter()
            .find(|(_, p)| p.0 == kind)
            .map(|(e, _)| e)
            .unwrap()
    }

    #[test]
    fn a_trigger_says_who_came_in_and_who_left_and_stops_nothing() {
        // A ball falls through a zone, onto the floor below it.
        let mut zone = entity(
            "zone",
            3.0,
            Body::Trigger,
            ColliderShape::Box {
                half: Vec3::new(1.0, 0.5, 1.0),
            },
        );
        zone.model = String::new();
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
                zone,
                entity(
                    "ball",
                    6.0,
                    Body::Dynamic,
                    ColliderShape::Sphere { radius: 0.25 },
                ),
            ],
            ..Default::default()
        };
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        let (zone, ball) = (the(&world, Body::Trigger), the(&world, Body::Dynamic));

        let mut entered_at = None;
        let mut left_at = None;
        for step in 0..180 {
            run_for(&mut physics, &mut world, 1);
            let contacts = world
                .get::<&Contacts>(zone)
                .expect("a trigger gets Contacts");
            if contacts.entered.contains(&ball) {
                assert!(entered_at.is_none(), "entered once");
                entered_at = Some(step);
                assert_eq!(contacts.inside, [ball]);
            }
            if contacts.left.contains(&ball) {
                assert!(left_at.is_none(), "left once");
                left_at = Some(step);
                assert!(contacts.inside.is_empty());
            }
        }
        let (entered, left) = (entered_at.expect("it came in"), left_at.expect("and left"));
        assert!(entered < left);
        let y = world.get::<&WorldTransform>(ball).unwrap().0.w_axis.y;
        assert!(
            (y - 0.35).abs() < 0.05,
            "fell through the zone to the floor: {y}"
        );

        // A ray from above passes through the zone to the floor.
        physics.refresh_queries();
        let hit = physics
            .cast_ray(Vec3::new(0.3, 10.0, 0.3), Vec3::NEG_Y, 20.0)
            .unwrap();
        assert!(
            hit.point.y < 0.2,
            "the zone is not a surface: {:?}",
            hit.point
        );
    }

    #[test]
    fn a_kinematic_platform_moved_by_the_game_carries_what_is_on_it() {
        let mut platform = entity(
            "platform",
            0.0,
            Body::Kinematic,
            ColliderShape::Box {
                half: Vec3::new(2.0, 0.1, 2.0),
            },
        );
        platform.model = String::new();
        let scene = Scene {
            entities: vec![
                platform,
                entity(
                    "crate",
                    0.6,
                    Body::Dynamic,
                    ColliderShape::Box {
                        half: Vec3::splat(0.5),
                    },
                ),
            ],
            ..Default::default()
        };
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        let (platform, crate_) = (the(&world, Body::Kinematic), the(&world, Body::Dynamic));
        world.insert_one(crate_, Contacts::default()).unwrap();
        run_for(&mut physics, &mut world, 30);
        assert!(
            world
                .get::<&Contacts>(crate_)
                .unwrap()
                .inside
                .contains(&platform),
            "resting on it"
        );

        // A lift: the game raises it two metres over two seconds.
        for step in 1..=120 {
            world.get::<&mut Transform>(platform).unwrap().position.y = step as f32 / 60.0;
            crate::world::apply_hierarchy(&mut world);
            run_for(&mut physics, &mut world, 1);
        }
        let lifted = world.get::<&WorldTransform>(platform).unwrap().0.w_axis.y;
        assert!(
            (lifted - 2.0).abs() < 1e-3,
            "where the game put it: {lifted}"
        );
        let y = world.get::<&WorldTransform>(crate_).unwrap().0.w_axis.y;
        assert!(y > 2.4, "the crate rode up on it: {y}");
    }

    fn scene_world(text: &str) -> (PhysicsWorld, World, Scene) {
        let mut scene: Scene = ron::from_str(text).unwrap();
        scene.assign_ids();
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        (PhysicsWorld::new(1.0 / 60.0), world, scene)
    }

    fn by_id(world: &World, id: crate::id::EntityId) -> hecs::Entity {
        world
            .query::<(hecs::Entity, &SceneId)>()
            .iter()
            .find(|(_, s)| s.0 == id)
            .map(|(e, _)| e)
            .unwrap()
    }

    #[test]
    fn a_plank_on_a_hinge_swings_down_to_its_limit_and_its_hinge_stays_put() {
        // Two metres long, level, hinged to the world at its left end about
        // z, allowed thirty degrees down.
        let (mut physics, mut world, scene) = scene_world(
            r#"(entities: [(name: "flap", model: "m", body: Dynamic,
                collider: Box(half: (0.5, 0.5, 0.5)),
                transform: (position: (0.0, 3.0, 0.0), scale: (2.0, 0.1, 0.5)),
                joint: Hinge(anchor: (-0.5, 0.0, 0.0), axis: (0.0, 0.0, 1.0), limits_deg: (-30.0, 0.0)))])"#,
        );
        let flap = by_id(&world, scene.entities[0].id);
        run_for(&mut physics, &mut world, 180);
        let placed = world.get::<&WorldTransform>(flap).unwrap().0;
        let (_, rotation, _) = placed.to_scale_rotation_translation();
        let (axis, angle) = rotation.to_axis_angle();
        let degrees = angle.to_degrees() * axis.z.signum();
        assert!((degrees + 30.0).abs() < 3.0, "down to the limit: {degrees}");
        let hinge = placed.transform_point3(Vec3::new(-0.5, 0.0, 0.0));
        assert!(
            (hinge - Vec3::new(-1.0, 3.0, 0.0)).length() < 0.05,
            "the hinge did not move: {hinge:?}"
        );
    }

    #[test]
    fn two_bodies_held_fixed_fall_together() {
        let (mut physics, mut world, scene) = scene_world(
            r#"(entities: [
                (id: "00000000000000b1", name: "head", model: "m", body: Dynamic,
                 collider: Sphere(radius: 0.3), transform: (position: (0.0, 5.0, 0.0))),
                (id: "00000000000000b2", name: "tail", model: "m", body: Dynamic,
                 collider: Sphere(radius: 0.3), transform: (position: (1.0, 5.0, 0.0)),
                 joint: Fixed(to: "00000000000000b1")),
            ])"#,
        );
        let (head, tail) = (
            by_id(&world, scene.entities[0].id),
            by_id(&world, scene.entities[1].id),
        );
        run_for(&mut physics, &mut world, 40);
        let a = world
            .get::<&WorldTransform>(head)
            .unwrap()
            .0
            .w_axis
            .truncate();
        let b = world
            .get::<&WorldTransform>(tail)
            .unwrap()
            .0
            .w_axis
            .truncate();
        assert!(a.y < 4.0, "fell: {a:?}");
        assert!(
            ((b - a).length() - 1.0).abs() < 0.02,
            "a metre apart still: {a:?} {b:?}"
        );
        assert!((a.y - b.y).abs() < 0.02, "and level: {a:?} {b:?}");

        // The joint taken out of the line (a reload): the tail goes free.
        world.remove_one::<Jointed>(tail).unwrap();
        run_for(&mut physics, &mut world, 1);
        assert!(world.get::<&JointBuilt>(tail).is_err());
        assert_eq!(physics.impulse_joints.len(), 0);
    }

    #[test]
    fn a_ray_names_the_entity_it_hit_and_a_ball_finds_what_it_overlaps() {
        let (mut physics, mut world, _) = scene_world(
            r#"(entities: [
                (id: "00000000000000e1", name: "near", model: "m", body: Static,
                 collider: Box(half: (0.5, 0.5, 0.5)), transform: (position: (0.0, 0.5, 0.0))),
                (id: "00000000000000e2", name: "far", model: "m", body: Static,
                 collider: Sphere(radius: 0.5), transform: (position: (6.0, 0.5, 0.0))),
                (id: "00000000000000e3", name: "zone", model: "m", body: Trigger,
                 collider: Box(half: (3.0, 3.0, 3.0))),
            ])"#,
        );
        physics.sync_from_world(&mut world);
        physics.refresh_queries();
        let near = by_id(&world, "00000000000000e1".parse().unwrap());
        let far = by_id(&world, "00000000000000e2".parse().unwrap());

        let hit = physics
            .cast_ray(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y, 10.0)
            .unwrap();
        assert_eq!(hit.entity, Some(near));
        assert_eq!(
            physics.overlap_sphere(Vec3::new(1.0, 0.5, 0.0), 0.7),
            [near]
        );
        let mut both = vec![near, far];
        both.sort();
        assert_eq!(
            physics.overlap_sphere(Vec3::new(3.0, 0.5, 0.0), 3.0),
            both,
            "the zone around them is not one of them"
        );
        assert!(physics
            .overlap_sphere(Vec3::new(3.0, 5.0, 0.0), 0.5)
            .is_empty());
    }

    #[test]
    fn a_bouncy_ball_bounces_a_sandbag_does_not_and_iron_weighs_more() {
        let (mut physics, mut world, scene) = scene_world(
            r#"(entities: [
                (name: "floor", model: "m", body: Static, collider: Box(half: (20.0, 0.1, 20.0))),
                (name: "ball", model: "m", body: Dynamic, collider: Sphere(radius: 0.25),
                 transform: (position: (-2.0, 3.0, 0.0)), physics: (bounce: 0.8, friction: 0.2)),
                (name: "sandbag", model: "m", body: Dynamic, collider: Sphere(radius: 0.25),
                 transform: (position: (2.0, 3.0, 0.0))),
            ])"#,
        );
        let (ball, sandbag) = (
            by_id(&world, scene.entities[1].id),
            by_id(&world, scene.entities[2].id),
        );
        let height = |world: &World, e| world.get::<&WorldTransform>(e).unwrap().0.w_axis.y;
        // Fall, hit the floor, and see how high each comes back up.
        let (mut ball_top, mut bag_top) = (0.0f32, 0.0f32);
        let mut landed = false;
        for _ in 0..150 {
            run_for(&mut physics, &mut world, 1);
            let (b, s) = (height(&world, ball), height(&world, sandbag));
            if b < 0.5 {
                landed = true;
            }
            if landed {
                ball_top = ball_top.max(b);
                bag_top = bag_top.max(s);
            }
        }
        assert!(ball_top > 1.2, "the ball came back up: {ball_top}");
        assert!(bag_top < 0.5, "the sandbag stayed down: {bag_top}");

        // Density is mass: the same ball, eight times as dense.
        let mass = |physics: &PhysicsWorld, world: &World, e| {
            let handle = world.get::<&BodyHandle>(e).unwrap().0;
            physics.bodies.get(handle).unwrap().mass()
        };
        let light = mass(&physics, &world, sandbag);
        world
            .insert_one(
                sandbag,
                Props(crate::scene::BodyProps {
                    density: 8.0,
                    ..Default::default()
                }),
            )
            .unwrap();
        run_for(&mut physics, &mut world, 1);
        let heavy = mass(&physics, &world, sandbag);
        assert!((heavy / light - 8.0).abs() < 0.01, "{light} -> {heavy}");
    }

    #[test]
    fn debris_falls_through_the_player_and_a_ray_can_look_past_it() {
        let (mut physics, mut world, scene) = scene_world(
            r#"(entities: [
                (name: "floor", model: "m", body: Static, collider: Box(half: (20.0, 0.1, 20.0))),
                (name: "player", model: "m", body: Static, layer: "player",
                 collider: Box(half: (1.0, 0.5, 1.0)), transform: (position: (0.0, 1.0, 0.0))),
                (name: "shard", model: "m", body: Dynamic, layer: "debris",
                 collider: Sphere(radius: 0.2), transform: (position: (0.0, 4.0, 0.0))),
            ])"#,
        );
        let layers = crate::layers::Layers {
            layers: vec!["default".into(), "player".into(), "debris".into()],
            ignore: vec![("debris".into(), "player".into())],
        };
        physics.set_layers(layers.clone(), &world);
        let shard = by_id(&world, scene.entities[2].id);
        run_for(&mut physics, &mut world, 120);
        let y = world.get::<&WorldTransform>(shard).unwrap().0.w_axis.y;
        assert!(
            (y - 0.3).abs() < 0.05,
            "through the player onto the floor: {y}"
        );

        physics.refresh_queries();
        let floor = by_id(&world, scene.entities[0].id);
        let down = Vec3::new(0.0, 5.0, 0.0);
        let any = physics.cast_ray(down, Vec3::NEG_Y, 10.0).unwrap();
        assert_ne!(any.entity, Some(floor), "the player is in the way");
        let ground = physics
            .cast_ray_among(down, Vec3::NEG_Y, 10.0, &["default"])
            .unwrap();
        assert_eq!(ground.entity, Some(floor));

        // The ignore taken out of layers.ron while running: they collide.
        world
            .insert_one(
                shard,
                Transform {
                    position: Vec3::new(0.0, 4.0, 0.0),
                    ..Transform::default()
                },
            )
            .unwrap();
        crate::world::apply_hierarchy(&mut world);
        physics.set_layers(
            crate::layers::Layers {
                ignore: Vec::new(),
                ..layers
            },
            &world,
        );
        run_for(&mut physics, &mut world, 120);
        let y = world.get::<&WorldTransform>(shard).unwrap().0.w_axis.y;
        assert!(y > 1.6, "now it lands on the player: {y}");
    }
}
