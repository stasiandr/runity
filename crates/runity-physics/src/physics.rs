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

#[allow(unused_imports)]
use crate::prelude::*;
use glam::{Quat, Vec3};
use hecs::World;
use rapier3d::prelude::*;

use crate::scene::{Body, Collider as ColliderShape, Transform};
use crate::world::{
    JointBreak, JointBroken, Jointed, Layer, Parent, Physics, Props, SceneId, Shape, WorldTransform,
};

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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Contacts {
    /// Everything touching now, in no particular order.
    pub inside: Vec<hecs::Entity>,
    /// What started touching this step.
    pub entered: Vec<hecs::Entity>,
    /// What stopped touching this step — possibly an entity that is gone.
    pub left: Vec<hecs::Entity>,
    /// Which way each solid contact pushes this body, in the world: away
    /// from what it touches, so a floor underfoot is up. One per touching
    /// pair; a character tells ground from a wall by the `y` of these, the
    /// way a controller reads contact normals rather than probing.
    pub normals: Vec<Vec3>,
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

/// Metres per second of wind at `strength: 1` — a breeze.
const WIND_SPEED: f32 = 4.0;
/// How fast something blown takes up the wind's speed, per second, at
/// `blown: 1`: a tumbleweed is nearly carried along within a second.
const WIND_GRIP: f32 = 2.5;
/// How often something blown hops off the ground, at `blown: 1` in a
/// breeze.
const HOPS_PER_SECOND: f32 = 0.6;

/// A number in [0, 1) from two others: the same two, the same number.
fn hashed(a: u64, b: u64) -> f32 {
    let mut x = a.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ b.wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    x ^= x >> 31;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 29;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// What a body was built from, and the transform it last agreed with: how
/// a change from outside the solver is told from the solver's own motion.
#[derive(Debug, Clone, PartialEq)]
struct Built {
    body: Body,
    collider: ColliderShape,
    local: Transform,
    /// Where in the world it was put: a body the solver does not move
    /// follows its parent's move, which leaves its own transform as it was.
    placed: glam::Mat4,
    /// Which [`CollisionMesh`] a `Model` collider was built from.
    mesh: usize,
    props: crate::scene::BodyProps,
    layer: String,
    /// What its parts were built from ([`parts_of`]): any change rebuilds it.
    parts: u64,
}

/// The collider a part was built as, on its ancestor's body: what its
/// own [`Contacts`] are read from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartCollider(pub ColliderHandle);

/// A collider of an ancestor's body, as it was found this sync.
struct PartFound {
    entity: hecs::Entity,
    shape: ColliderShape,
    placed: glam::Mat4,
    /// Where it sits under its owner, from their local transforms: the
    /// same between syncs whether or not the hierarchy has been placed.
    under: glam::Mat4,
    mesh: Option<CollisionMesh>,
    props: crate::scene::BodyProps,
    layer: String,
    trigger: bool,
}

/// Every part (`Body::Part`, `Body::TriggerPart`) by the entity whose body
/// it belongs to: the nearest ancestor with a body of its own. A part with
/// none is left out, and built as a body of its own — standing still.
fn parts_of(world: &World, off: &std::collections::HashSet<hecs::Entity>) -> std::collections::HashMap<hecs::Entity, Vec<PartFound>> {
    let mut out: std::collections::HashMap<hecs::Entity, Vec<PartFound>> = Default::default();
    for (entity, physics, shape, placed, mesh, props, layer) in world
        .query::<(
            hecs::Entity,
            &Physics,
            &Shape,
            &WorldTransform,
            Option<&CollisionMesh>,
            Option<&Props>,
            Option<&Layer>,
        )>()
        .iter()
    {
        if !physics.0.is_part() || off.contains(&entity) {
            continue;
        }
        let Some(owner) = owner_of(world, entity) else { continue };
        out.entry(owner).or_default().push(PartFound {
            entity,
            shape: shape.0,
            placed: placed.0,
            under: under_owner(world, entity, owner),
            mesh: mesh.cloned(),
            props: props.map(|p| p.0).unwrap_or_default(),
            layer: layer.map(|l| l.0.clone()).unwrap_or_default(),
            trigger: physics.0 == Body::TriggerPart,
        });
    }
    for parts in out.values_mut() {
        parts.sort_by_key(|p| p.entity.to_bits());
    }
    out
}

/// A part's place under its owner: the local transforms from it up to the
/// owner, multiplied.
fn under_owner(world: &World, part: hecs::Entity, owner: hecs::Entity) -> glam::Mat4 {
    let mut out = glam::Mat4::IDENTITY;
    let mut at = part;
    for _ in 0..64 {
        if at == owner {
            break;
        }
        if let Ok(t) = world.get::<&Transform>(at) {
            out = t.matrix() * out;
        }
        match world.get::<&Parent>(at) {
            Ok(p) => at = p.0,
            Err(_) => break,
        }
    }
    out
}

/// The nearest ancestor of a part with a body of its own.
fn owner_of(world: &World, part: hecs::Entity) -> Option<hecs::Entity> {
    let mut at = world.get::<&Parent>(part).ok()?.0;
    for _ in 0..64 {
        match world.get::<&Physics>(at).map(|p| p.0) {
            Ok(kind) if kind != Body::None && !kind.is_part() => return Some(at),
            _ => {}
        }
        at = world.get::<&Parent>(at).ok()?.0;
    }
    None
}

/// The volume of a body's own collider — every collider on it that is not
/// one of `parts`.
fn body_volume(
    bodies: &RigidBodySet,
    colliders: &ColliderSet,
    body: RigidBodyHandle,
    parts: &[(ColliderHandle, f32)],
) -> f32 {
    bodies
        .get(body)
        .map(|b| b.colliders())
        .unwrap_or(&[])
        .iter()
        .filter(|h| parts.iter().all(|(p, _)| p != *h))
        .filter_map(|h| colliders.get(*h))
        .filter(|c| !c.is_sensor())
        .map(|c| c.shape().mass_properties(1.0).mass())
        .sum()
}

/// What a body's parts were built from, as one number: where each sits on
/// the body, its shape, grip and layer.
fn parts_signature(_owner: glam::Mat4, parts: &[PartFound]) -> u64 {
    use std::hash::{Hash, Hasher};
    // No parts is nothing to compare: most bodies.
    if parts.is_empty() {
        return 0;
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
        part.entity.to_bits().hash(&mut hash);
        format!("{:?}{:?}{}{}", part.shape, part.props, part.layer, part.trigger).hash(&mut hash);
        part.mesh.as_ref().map_or(0, CollisionMesh::key).hash(&mut hash);
        for v in part.under.to_cols_array() {
            ((v * 1000.0).round() as i64).hash(&mut hash);
        }
    }
    hash.finish()
}

/// The body an entity gets: what its line asks for — except that a
/// dynamic body someone else simulates is kinematic here, moved to where
/// its owner says. One solver per body (the dacha simulator's rule, DNA
/// postulate 4): two machines each solving a crate would each be right
/// about a different crate.
fn solved(asked: Body, replica: bool) -> Body {
    if replica && asked == Body::Dynamic {
        Body::Kinematic
    } else {
        asked
    }
}

/// What a line's `freeze_move` and `freeze_turn` hold still.
fn locked(props: &crate::scene::BodyProps) -> LockedAxes {
    let mut out = LockedAxes::empty();
    for (on, axis) in [
        (props.freeze_move.x, LockedAxes::TRANSLATION_LOCKED_X),
        (props.freeze_move.y, LockedAxes::TRANSLATION_LOCKED_Y),
        (props.freeze_move.z, LockedAxes::TRANSLATION_LOCKED_Z),
        (props.freeze_turn.x, LockedAxes::ROTATION_LOCKED_X),
        (props.freeze_turn.y, LockedAxes::ROTATION_LOCKED_Y),
        (props.freeze_turn.z, LockedAxes::ROTATION_LOCKED_Z),
    ] {
        if on {
            out |= axis;
        }
    }
    out
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
    let mut made: std::collections::HashMap<String, Option<CollisionMesh>> = Default::default();
    for (entity, desc) in lines {
        if desc.collider() != ColliderShape::Model {
            continue;
        }
        // Shaped ground stands on its own mesh.
        if let Some(terrain) = desc.terrain() {
            let _ = world.insert_one(entity, CollisionMesh::of(&terrain.mesh()));
            continue;
        }
        // Its own collision model, or the model it draws.
        let model = desc
            .part::<crate::scene::CollisionModel>()
            .map(|m| m.0)
            .unwrap_or_else(|| desc.model());
        let mesh = made
            .entry(model.as_str().to_string())
            .or_insert_with(|| collision_mesh_for(&model, library))
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
/// their [`crate::world::SceneId`].
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
        .query::<(hecs::Entity, &crate::world::SceneId)>()
        .iter()
        .filter_map(|(entity, id)| lines.get(&id.0).map(|d| (entity, *d)))
        .collect();
    attach_collision_meshes(world, pairs, library);
}

/// Everything rapier needs to take a step.
pub struct PhysicsWorld {
    pub gravity: Vec3,
    /// The scene's wind: what carries bodies that say `blown` — a
    /// tumbleweed, a plastic bag, a hat. Still air until the game says.
    pub wind: runity_core::wind::Wind,
    /// Steps taken: the clock gusts and hops are read from, so a replay
    /// blows the same way.
    steps: u64,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    parameters: IntegrationParameters,
    /// Joints broken since [`PhysicsWorld::broken`] was last asked.
    broken: Vec<hecs::Entity>,
    /// A body's speed when it went kinematic, for when it goes dynamic.
    held_speed: std::collections::HashMap<RigidBodyHandle, (Vector<Real>, Vector<Real>)>,
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
    /// Pairs of entities told to pass through each other, by their bits,
    /// smaller first.
    ignored: std::collections::HashSet<(u64, u64)>,
}

/// Rapier's say on each pair of colliders: no contact for a pair told to
/// ignore each other.
struct Ignoring<'a>(&'a std::collections::HashSet<(u64, u64)>);

impl Ignoring<'_> {
    fn ignores(&self, context: &PairFilterContext) -> bool {
        if self.0.is_empty() {
            return false;
        }
        let bits = |h: ColliderHandle| context.colliders.get(h).map_or(0, |c| c.user_data as u64);
        let (a, b) = (bits(context.collider1), bits(context.collider2));
        self.0.contains(&(a.min(b), a.max(b)))
    }
}

impl PhysicsHooks for Ignoring<'_> {
    fn filter_contact_pair(&self, context: &PairFilterContext) -> Option<SolverFlags> {
        (!self.ignores(context)).then_some(SolverFlags::COMPUTE_IMPULSES)
    }

    fn filter_intersection_pair(&self, context: &PairFilterContext) -> bool {
        !self.ignores(context)
    }
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
            wind: runity_core::wind::Wind {
                direction: Vec3::X,
                strength: 0.0,
            },
            steps: 0,
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            broken: Vec::new(),
            held_speed: Default::default(),
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
            ignored: Default::default(),
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
        let mut switched: Vec<(hecs::Entity, RigidBodyHandle, Body)> = Vec::new();
        // What is switched off has no body.
        let off = crate::world::inactive_in_hierarchy(world);
        let parts = parts_of(world, &off);
        for (entity, handle, built, physics, shape, local, placed, mesh, props, layer, replica) in
            world
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
                    Option<&crate::world::Replica>,
                )>()
                .iter()
        {
            if off.contains(&entity) {
                stale.push(entity);
                continue;
            }
            let body = solved(physics.0, replica.is_some());
            let mesh = mesh.map_or(0, CollisionMesh::key);
            let props = props.map(|p| p.0).unwrap_or_default();
            let layer = layer.map(|l| l.0.as_str()).unwrap_or("");
            let signature = parts.get(&entity).map_or(0, |p| parts_signature(placed.0, p));
            if signature != built.parts {
                stale.push(entity);
                continue;
            }
            // Dynamic ↔ kinematic keeps the body and its speed: switched in
            // place, as Unity's isKinematic does.
            let switch = built.body != body
                && matches!(built.body, Body::Dynamic | Body::Kinematic)
                && matches!(body, Body::Dynamic | Body::Kinematic)
                && built.collider == shape.0
                && built.mesh == mesh
                && built.props == props
                && built.layer == layer;
            if switch {
                switched.push((entity, handle.0, body));
                live.insert(handle.0);
                // Moved in the same breath (taken over from another peer,
                // from further along): put there as well.
                if built.local != *local {
                    teleport.push((entity, handle.0, placed.0, *local, body));
                }
                continue;
            }
            if built.body != body
                || built.collider != shape.0
                || built.mesh != mesh
                || built.props != props
                || built.layer != layer
            {
                stale.push(entity);
            } else {
                live.insert(handle.0);
                // Moved itself, or — for a body the solver does not move —
                // carried by a parent that moved: a lever's mount on a
                // held tool, a drone's mouth on its animated frame.
                let carried = !matches!(body, Body::Dynamic) && moved(&built.placed, &placed.0);
                if built.local != *local || carried {
                    teleport.push((entity, handle.0, placed.0, *local, body));
                }
            }
        }
        for (entity, handle, kind) in switched {
            if let Some(body) = self.bodies.get_mut(handle) {
                match kind {
                    Body::Kinematic => {
                        // Its speed, kept for when it is let go again.
                        self.held_speed
                            .insert(handle, (*body.linvel(), *body.angvel()));
                        body.set_body_type(RigidBodyType::KinematicPositionBased, true);
                    }
                    _ => {
                        body.set_body_type(RigidBodyType::Dynamic, true);
                        // Moved while held: it goes on as it was moved. Held
                        // still: as fast as when it was taken.
                        let kept = self.held_speed.remove(&handle);
                        if let Some((linear, angular)) = kept {
                            if body.linvel().norm() < 1e-3 && body.angvel().norm() < 1e-3 {
                                body.set_linvel(linear, true);
                                body.set_angvel(angular, true);
                            }
                        }
                    }
                }
            }
            if let Ok(mut built) = world.get::<&mut Built>(entity) {
                built.body = kind;
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
                built.placed = placed;
            }
        }

        let mut added: Vec<(hecs::Entity, BodyHandle, Built)> = Vec::new();
        let mut part_handles: Vec<(hecs::Entity, ColliderHandle)> = Vec::new();
        for (entity, placed, physics, shape, local, existing, mesh, props, layer, replica) in world
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
                Option<&crate::world::Replica>,
            )>()
            .iter()
        {
            let kind = solved(physics.0, replica.is_some());
            let props = props.map(|p| p.0).unwrap_or_default();
            let layer = layer.map(|l| l.0.clone()).unwrap_or_default();
            if existing.is_some() || physics.0 == Body::None || off.contains(&entity) {
                continue;
            }
            // A part is built with the body it belongs to; one with no body
            // above it stands still on its own.
            if kind.is_part() && owner_of(world, entity).is_some() {
                continue;
            }
            let kind = match kind {
                Body::Part => Body::Static,
                Body::TriggerPart => Body::Trigger,
                other => other,
            };
            let dynamic = kind == Body::Dynamic;
            let mine = parts.get(&entity).map(Vec::as_slice).unwrap_or(&[]);
            let own = build_collider(shape.0, placed.0, mesh, dynamic);
            if own.is_none() && mine.is_empty() {
                // Declared solid with no shape to be solid with. Skipped
                // rather than guessed at — a box invented from a mesh's
                // bounds is the kind of default that is wrong quietly.
                continue;
            }
            // Its own collider, or — for a body made only of its parts — none.
            let mut collider = own.unwrap_or_else(|| ColliderBuilder::ball(1e-3).sensor(true).build());
            // Which entity a collider is, for contacts to be told in
            // entities rather than rapier handles.
            collider.user_data = entity.to_bits().get() as u128;
            // Asked about each pair, so two told to ignore each other can.
            collider.set_active_hooks(
                ActiveHooks::FILTER_CONTACT_PAIRS | ActiveHooks::FILTER_INTERSECTION_PAIR,
            );
            collider.set_friction(props.friction.max(0.0));
            // No grip at all is none against anything: the smaller of the
            // two decides, so no wall can put friction back on a body that
            // is meant to slide (a character's capsule, driven by its own
            // velocity).
            if props.friction <= 0.0 {
                collider.set_friction_combine_rule(CoefficientCombineRule::Min);
            }
            collider.set_restitution(props.bounce.clamp(0.0, 1.0));
            // The bouncier of the two decides, so a ball bounces off any
            // floor; grip stays the average of both, as everywhere.
            collider.set_restitution_combine_rule(CoefficientCombineRule::Max);
            let layered = groups(&self.layers, &layer);
            collider.set_collision_groups(layered);
            collider.set_solver_groups(layered);
            collider.set_density(props.density.max(1e-3));
            if let Some(mass) = props.mass.filter(|m| *m > 0.0) {
                collider.set_mass(mass);
            }
            if physics.0 == Body::Trigger {
                collider.set_sensor(true);
                // A zone notices whatever enters it, a kinematic player or
                // a static crate included — not only what the solver moves.
                collider.set_active_collision_types(ActiveCollisionTypes::all());
            }
            let body = match kind {
                Body::Dynamic => RigidBodyBuilder::dynamic(),
                Body::Kinematic | Body::Trigger => RigidBodyBuilder::kinematic_position_based(),
                _ => RigidBodyBuilder::fixed(),
            }
            .position(isometry(placed.0))
            .linear_damping(props.drag.max(0.0))
            .angular_damping(props.spin_drag.max(0.0))
            .gravity_scale(props.gravity)
            .ccd_enabled(props.fast)
            .locked_axes(locked(&props))
            .build();
            let mut body = body;
            body.user_data = entity.to_bits().get() as u128;
            let handle = self.bodies.insert(body);
            let solid_own = shape.0 != ColliderShape::None;
            self.colliders
                .insert_with_parent(collider, handle, &mut self.bodies);
            // Its parts: each where it sits on the body, gripping and
            // colliding as its own line says, weighing its share.
            let mut part_colliders: Vec<(ColliderHandle, f32)> = Vec::new();
            for part in mine {
                let Some(mut c) = build_collider(part.shape, part.placed, part.mesh.as_ref(), dynamic) else {
                    continue;
                };
                // Where it sits on the body: both as the solver places them,
                // without scale — the part's own scale is in its shape.
                let offset = isometry(placed.0).inverse() * isometry(part.placed);
                let local = *c.position();
                c.set_position(offset * local);
                c.user_data = part.entity.to_bits().get() as u128;
                c.set_active_hooks(ActiveHooks::FILTER_CONTACT_PAIRS | ActiveHooks::FILTER_INTERSECTION_PAIR);
                c.set_friction(part.props.friction.max(0.0));
                if part.props.friction <= 0.0 {
                    c.set_friction_combine_rule(CoefficientCombineRule::Min);
                }
                c.set_restitution(part.props.bounce.clamp(0.0, 1.0));
                c.set_restitution_combine_rule(CoefficientCombineRule::Max);
                let layered = groups(&self.layers, &part.layer);
                c.set_collision_groups(layered);
                c.set_solver_groups(layered);
                c.set_density(part.props.density.max(1e-3));
                if part.trigger {
                    c.set_sensor(true);
                    c.set_active_collision_types(ActiveCollisionTypes::all());
                }
                let volume = if part.trigger { 0.0 } else { c.shape().mass_properties(1.0).mass() };
                let h = self.colliders.insert_with_parent(c, handle, &mut self.bodies);
                part_colliders.push((h, volume));
                part_handles.push((part.entity, h));
            }
            // A body's mass, when its line says one, is its parts' together,
            // shared by volume — Unity's Rigidbody mass over its colliders.
            if let (Some(mass), false) = (props.mass.filter(|m| *m > 0.0), part_colliders.is_empty()) {
                let own_volume = if solid_own {
                    body_volume(&self.bodies, &self.colliders, handle, &part_colliders)
                } else {
                    0.0
                };
                let total: f32 = part_colliders.iter().map(|(_, v)| v).sum::<f32>() + own_volume;
                if total > 0.0 {
                    for (h, volume) in &part_colliders {
                        if let Some(c) = self.colliders.get_mut(*h) {
                            c.set_mass(mass * volume / total);
                        }
                    }
                    for &h in self.bodies.get(handle).map(|b| b.colliders()).unwrap_or(&[]) {
                        if part_colliders.iter().all(|(p, _)| *p != h) {
                            if let Some(c) = self.colliders.get_mut(h) {
                                c.set_mass((mass * own_volume / total).max(1e-6));
                            }
                        }
                    }
                }
            }
            added.push((
                entity,
                BodyHandle(handle),
                Built {
                    body: kind,
                    collider: shape.0,
                    local: *local,
                    placed: placed.0,
                    mesh: mesh.map_or(0, CollisionMesh::key),
                    props,
                    layer,
                    parts: parts_signature(placed.0, mine),
                },
            ));
        }
        for (entity, handle) in part_handles {
            let _ = world.insert_one(entity, PartCollider(handle));
        }
        for (entity, handle, built) in added {
            let trigger = built.body == Body::Trigger;
            let _ = world.insert(entity, (handle, built));
            if trigger && world.get::<&Contacts>(entity).is_err() {
                let _ = world.insert_one(entity, Contacts::default());
            }
        }
        // Last, so a body rebuilt above has it too.
        // Taken over from another peer: on at the speed it had there.
        let taken: Vec<(hecs::Entity, crate::world::Takeover)> = world
            .query::<(hecs::Entity, &crate::world::Takeover)>()
            .iter()
            .map(|(e, t)| (e, *t))
            .collect();
        for (entity, t) in taken {
            if let Some(body) = world
                .get::<&BodyHandle>(entity)
                .ok()
                .and_then(|h| self.bodies.get_mut(h.0))
            {
                let (v, w) = (t.velocity, t.spin);
                body.set_linvel(vector![v.x, v.y, v.z], true);
                body.set_angvel(vector![w.x, w.y, w.z], true);
            }
            let _ = world.remove_one::<crate::world::Takeover>(entity);
        }
        self.sync_joints(world);
    }

    /// Joints after bodies: build each once both of its bodies exist,
    /// rebuild it when the joint or either body changed, drop it when its
    /// entity no longer asks for one. A joint whose partner is not there —
    /// not built yet, or a typo'd id — waits rather than guessing.
    fn sync_joints(&mut self, world: &mut World) {
        // By the scene's ids, and by a run-time prefab's own: a mouse spawned
        // mid-level has a tail whose links name each other.
        let mut bodies_by_id: std::collections::HashMap<crate::id::EntityId, RigidBodyHandle> = world
            .query::<(&SceneId, &BodyHandle)>()
            .iter()
            .map(|(id, handle)| (id.0, handle.0))
            .collect();
        bodies_by_id.extend(
            world
                .query::<(&crate::world::SpawnedId, &BodyHandle)>()
                .iter()
                .map(|(id, handle)| (id.0, handle.0)),
        );
        let mut drop: Vec<hecs::Entity> = Vec::new();
        let mut build: Vec<(
            hecs::Entity,
            crate::scene::Joint,
            RigidBodyHandle,
            glam::Mat4,
        )> = Vec::new();
        let mut retune: Vec<(hecs::Entity, crate::scene::Joint)> = Vec::new();
        for (entity, joint, body, placed, built, broken) in world
            .query::<(
                hecs::Entity,
                Option<&Jointed>,
                Option<&BodyHandle>,
                &WorldTransform,
                Option<&JointBuilt>,
                Option<&JointBroken>,
            )>()
            .iter()
        {
            // Broken stays broken until the joint is set anew.
            let wanted = joint
                .zip(body)
                .filter(|_| broken.is_none())
                .map(|(joint, body)| (joint.0, body.0));
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
                // Only its drive or its limits changed — a spring's target
                // moved while the game runs: tuned where it is, not built
                // again from where the bodies stand now.
                let tuned = wanted.is_some_and(|(joint, body)| {
                    same_but_drive(&joint, &built.joint)
                        && body == built.bodies.1
                        && self.impulse_joints.get(built.handle).is_some()
                });
                if let (true, Some((joint, _))) = (tuned, wanted) {
                    retune.push((entity, joint));
                    continue;
                }
                drop.push(entity);
            }
            if let Some((joint, body)) = wanted {
                build.push((entity, joint, body, placed.0));
            }
        }
        for (entity, joint) in retune {
            if let Ok(mut built) = world.get::<&mut JointBuilt>(entity) {
                if let Some(rapier) = self.impulse_joints.get_mut(built.handle, true) {
                    drive(&mut rapier.data, &joint);
                }
                built.joint = joint;
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
        let entity_of = |collider: ColliderHandle| self.entity_of(collider);
        // A part's own contacts: what its one collider touches.
        for (entity, part, contacts) in world
            .query_mut::<(hecs::Entity, &PartCollider, &mut Contacts)>()
            .into_iter()
        {
            let mine = part.0;
            let other = |a: ColliderHandle, b: ColliderHandle| if a == mine { b } else { a };
            let mut now: Vec<hecs::Entity> = Vec::new();
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
            now.retain(|e| *e != entity);
            now.sort();
            now.dedup();
            contacts.entered = now.iter().filter(|e| !contacts.inside.contains(e)).copied().collect();
            contacts.left = contacts.inside.iter().filter(|e| !now.contains(e)).copied().collect();
            contacts.inside = now;
        }
        for (entity, handle, contacts) in world
            .query_mut::<(hecs::Entity, &BodyHandle, &mut Contacts)>()
            .into_iter()
        {
            let Some(body) = self.bodies.get(handle.0) else {
                continue;
            };
            let mut now: Vec<hecs::Entity> = Vec::new();
            let mut normals: Vec<Vec3> = Vec::new();
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
                        // A manifold's normal points from the pair's first
                        // collider to its second; ours is the other way from
                        // whatever we touch.
                        let sign = if pair.collider1 == mine { -1.0 } else { 1.0 };
                        for manifold in &pair.manifolds {
                            if manifold.points.iter().any(|p| p.dist <= 0.01) {
                                let n = manifold.data.normal;
                                normals.push(Vec3::new(n.x, n.y, n.z) * sign);
                            }
                        }
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
            contacts.normals = normals;
        }
    }

    /// One fixed step of physics as a system: bring rapier in line with the
    /// world, step, and write where the dynamic bodies went back. Call it
    /// once per simulation step.
    pub fn run(&mut self, world: &mut World) {
        self.sync_from_world(world);
        self.blow(world);
        self.step();
        self.break_joints(world);
        self.sync_to_world(world);
        self.update_contacts(world);
    }

    /// The wind on everything `blown`: dragged toward the wind's own speed,
    /// harder in gusts, and now and then — when it is on the ground — a
    /// hop, the way a tumbleweed bounds across open sand. The ground's grip
    /// makes it roll. Gusts and hops come from the step count and the
    /// entity, so the same step blows the same way on every machine.
    fn blow(&mut self, world: &World) {
        let level =
            Vec3::new(self.wind.direction.x, 0.0, self.wind.direction.z).normalize_or_zero();
        let speed = WIND_SPEED * self.wind.strength.max(0.0);
        if speed <= 0.0 || level == Vec3::ZERO {
            return;
        }
        let dt = self.parameters.dt;
        let time = self.steps as f32 * dt;
        let up = Vec3::Y;
        for (entity, handle, props) in world.query::<(hecs::Entity, &BodyHandle, &Props)>().iter() {
            let blown = props.0.blown;
            if blown <= 0.0 {
                continue;
            }
            let Some(body) = self.bodies.get(handle.0) else {
                continue;
            };
            if !body.is_dynamic() {
                continue;
            }
            let seed = (entity.to_bits().get() % 997) as f32 * 7.31;
            // Gusts: two slow waves that do not repeat together.
            let gust = 0.65
                + 0.25 * (time * 0.9 + seed).sin()
                + 0.2 * (time * 2.3 + seed * 1.7).sin().max(0.0);
            let wind = level * speed * gust;
            let at = body.translation();
            let centre = Vec3::new(at.x, at.y, at.z);
            let v = body.linvel();
            let moving = Vec3::new(v.x, 0.0, v.z);
            let mass = body.mass();
            let push = (wind - moving) * (WIND_GRIP * blown * mass);
            // Half its height, from its colliders' bounds: how far down the
            // ground is when it is on it.
            let half = body
                .colliders()
                .iter()
                .filter_map(|c| self.colliders.get(*c))
                .map(|c| c.compute_aabb().half_extents().y)
                .fold(0.0f32, f32::max)
                .max(0.05);
            let down = Ray::new(
                point![centre.x, centre.y, centre.z],
                vector![0.0, -1.0, 0.0],
            );
            let others = QueryFilter::default()
                .exclude_rigid_body(handle.0)
                .exclude_sensors();
            let grounded = self
                .queries
                .cast_ray(
                    &self.bodies,
                    &self.colliders,
                    &down,
                    half + 0.08,
                    true,
                    others,
                )
                .is_some();
            let mut hop = 0.0;
            if grounded && v.y.abs() < 1.0 {
                // A hop now and then, likelier in a strong gust.
                let chance = HOPS_PER_SECOND * blown * gust * self.wind.strength.min(3.0) * dt;
                if hashed(entity.to_bits().get(), self.steps) < chance {
                    hop = mass
                        * (2.2 + 2.5 * hashed(entity.to_bits().get() ^ 0x9e37, self.steps))
                        * gust;
                }
            }
            // A twist about the axis it rolls on, so it turns in the air too.
            let roll = up.cross(level) * (mass * half * half * blown * 2.0 * gust);
            let body = self.bodies.get_mut(handle.0).expect("looked up above");
            body.apply_impulse(vector![push.x * dt, hop, push.z * dt], true);
            body.apply_torque_impulse(vector![roll.x * dt, roll.y * dt, roll.z * dt], true);
        }
    }

    /// How far a hinge has turned about its axis, in degrees, the way its
    /// `limits_deg` count: 0 where it was built, signed. What a lever or a
    /// door reads to know whether it is thrown. `None` for an entity with
    /// no joint built.
    pub fn hinge_angle(&self, world: &World, entity: hecs::Entity) -> Option<f32> {
        let built = world.get::<&JointBuilt>(entity).ok()?;
        let joint = self.impulse_joints.get(built.handle)?;
        let one = self.bodies.get(joint.body1)?;
        let two = self.bodies.get(joint.body2)?;
        let first = one.position() * joint.data.local_frame1;
        let second = two.position() * joint.data.local_frame2;
        // A revolute joint turns about its frames' x.
        let relative = first.rotation.inverse() * second.rotation;
        let q = relative.quaternion();
        let (x, w) = if q.w < 0.0 { (-q.i, -q.w) } else { (q.i, q.w) };
        Some((2.0 * x.atan2(w)).to_degrees())
    }

    /// Break every joint pulled harder than its `joint_break` this step:
    /// the joint goes, the entity is marked [`JointBroken`], and
    /// [`PhysicsWorld::broken`] says which. Part of [`PhysicsWorld::run`].
    pub fn break_joints(&mut self, world: &mut World) {
        let dt = self.parameters.dt.max(1e-6);
        let mut snapped = Vec::new();
        for (entity, built, limit) in world
            .query::<(hecs::Entity, &JointBuilt, &JointBreak)>()
            .iter()
        {
            let Some(joint) = self.impulse_joints.get(built.handle) else {
                continue;
            };
            let i = joint.impulses;
            let mut force = (i[0] * i[0] + i[1] * i[1] + i[2] * i[2]).sqrt() / dt;
            // A spring holds by its motors, which the joint's impulses do
            // not count: its pull is its stiffness times how far apart its
            // two anchors are.
            if let crate::scene::Joint::Spring { stiffness, .. } = built.joint {
                if let (Some(one), Some(two)) = (self.bodies.get(joint.body1), self.bodies.get(joint.body2)) {
                    let a = one.position() * joint.data.local_frame1;
                    let b = two.position() * joint.data.local_frame2;
                    let apart = (a.translation.vector - b.translation.vector).norm();
                    force = force.max(stiffness.max(0.0) * apart);
                }
            }
            if force > limit.0 {
                snapped.push(entity);
            }
        }
        for entity in snapped {
            if let Ok(built) = world.remove_one::<JointBuilt>(entity) {
                self.impulse_joints.remove(built.handle, true);
            }
            let _ = world.insert_one(entity, JointBroken);
            self.broken.push(entity);
        }
    }

    /// The entities whose joints broke since this was last asked: Unity's
    /// `OnJointBreak`, as a list.
    pub fn broken(&mut self) -> Vec<hecs::Entity> {
        std::mem::take(&mut self.broken)
    }

    /// Take one step. Call it once per simulation step, never per frame.
    pub fn step(&mut self) {
        self.steps += 1;
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
            &Ignoring(&self.ignored),
            &(),
        );
    }

    /// Let two entities' bodies pass through each other — or collide again
    /// with `false` — whatever their layers say: Unity's
    /// `Physics.IgnoreCollision`. A sword and the hand that holds it, a
    /// thrown thing and whoever threw it.
    pub fn ignore_collision(&mut self, a: hecs::Entity, b: hecs::Entity, ignore: bool) {
        let (a, b) = (a.to_bits().get(), b.to_bits().get());
        let pair = (a.min(b), a.max(b));
        if ignore {
            self.ignored.insert(pair);
        } else {
            self.ignored.remove(&pair);
        }
    }

    /// Copy every dynamic body's position back onto its entity.
    ///
    /// Only dynamic ones: a static body's transform belongs to the scene, and
    /// writing rapier's copy back over it would let rounding walk the world
    /// a fraction at a time.
    pub fn sync_to_world(&self, world: &mut World) {
        let mut moved: Vec<(hecs::Entity, glam::Mat4, Option<hecs::Entity>)> = Vec::new();
        for (entity, handle, physics, placed, parent, replica) in world
            .query::<(
                hecs::Entity,
                &BodyHandle,
                &Physics,
                &WorldTransform,
                Option<&Parent>,
                Option<&crate::world::Replica>,
            )>()
            .iter()
        {
            if solved(physics.0, replica.is_some()) != Body::Dynamic {
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
                built.placed = matrix;
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
    /// The entity a collider answers for: the body's own — a crate, not
    /// the plank of it the ray met.
    fn entity_of(&self, collider: ColliderHandle) -> Option<hecs::Entity> {
        let c = self.colliders.get(collider)?;
        c.parent()
            .and_then(|b| self.bodies.get(b))
            .and_then(|b| hecs::Entity::from_bits(b.user_data as u64))
            .or_else(|| hecs::Entity::from_bits(c.user_data as u64))
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

    /// Everything a ray passes through, nearest first — Unity's
    /// `RaycastAll`: every enemy a piercing shot goes through, every floor
    /// under a point. Triggers are looked through.
    pub fn cast_ray_all(&self, from: Vec3, direction: Vec3, max_distance: f32) -> Vec<RayHit> {
        let direction = direction.normalize_or_zero();
        if direction.length_squared() < 0.5 {
            return Vec::new();
        }
        let ray = Ray::new(
            point![from.x, from.y, from.z],
            vector![direction.x, direction.y, direction.z],
        );
        let mut hits = Vec::new();
        self.queries.intersections_with_ray(
            &self.bodies,
            &self.colliders,
            &ray,
            max_distance,
            true,
            QueryFilter::default().exclude_sensors(),
            |collider, hit| {
                hits.push(RayHit {
                    point: from + direction * hit.time_of_impact,
                    distance: hit.time_of_impact,
                    collider: ColliderRef(collider),
                    entity: self.entity_of(collider),
                });
                true
            },
        );
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        hits
    }

    /// Every entity whose shape overlaps a box — Unity's `OverlapBox`: what
    /// is in a doorway, on a pressure plate's area, inside a room. `half`
    /// is half its size on each axis, turned by `rotation`. Triggers are
    /// left out; sorted.
    pub fn overlap_box(&self, centre: Vec3, half: Vec3, rotation: glam::Quat) -> Vec<hecs::Entity> {
        let cuboid = Cuboid::new(vector![half.x.max(0.0), half.y.max(0.0), half.z.max(0.0)]);
        let at = Isometry::from_parts(
            Translation::new(centre.x, centre.y, centre.z),
            nalgebra::UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(
                rotation.w, rotation.x, rotation.y, rotation.z,
            )),
        );
        let mut found = Vec::new();
        self.queries.intersections_with_shape(
            &self.bodies,
            &self.colliders,
            &at,
            &cuboid,
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

    /// The rapier body behind an entity, once it has been built.
    fn body_of(&self, world: &World, entity: hecs::Entity) -> Option<RigidBodyHandle> {
        world.get::<&BodyHandle>(entity).ok().map(|h| h.0)
    }

    /// How fast an entity's body moves, metres per second. `None` before
    /// its body is built, or when it has none.
    pub fn velocity(&self, world: &World, entity: hecs::Entity) -> Option<Vec3> {
        let body = self.bodies.get(self.body_of(world, entity)?)?;
        let v = body.linvel();
        Some(Vec3::new(v.x, v.y, v.z))
    }

    /// Set how fast a body moves — Unity's `Rigidbody.velocity`. `false`
    /// when the entity has no body yet.
    pub fn set_velocity(&mut self, world: &World, entity: hecs::Entity, velocity: Vec3) -> bool {
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        body.set_linvel(vector![velocity.x, velocity.y, velocity.z], true);
        true
    }

    /// Set how fast a body turns, radians a second about each axis: a
    /// thrown plank's tumble. Unity's `angularVelocity`.
    pub fn set_spin(&mut self, world: &World, entity: hecs::Entity, spin: Vec3) -> bool {
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        body.set_angvel(vector![spin.x, spin.y, spin.z], true);
        true
    }

    /// Put a body somewhere at once, still: a cannon reloading its
    /// muzzle. Its transform follows.
    pub fn teleport(
        &mut self,
        world: &mut World,
        entity: hecs::Entity,
        position: Vec3,
        rotation: Quat,
    ) -> bool {
        let Some(handle) = self.body_of(world, entity) else {
            return false;
        };
        let Some(body) = self.bodies.get_mut(handle) else {
            return false;
        };
        let pose = Isometry::from_parts(
            Translation::new(position.x, position.y, position.z),
            nalgebra::UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(
                rotation.w, rotation.x, rotation.y, rotation.z,
            )),
        );
        body.set_position(pose, true);
        body.set_linvel(vector![0.0, 0.0, 0.0], true);
        body.set_angvel(vector![0.0, 0.0, 0.0], true);
        if let Ok(mut transform) = world.get::<&mut Transform>(entity) {
            transform.position = position;
            transform.set_rotation(rotation);
        }
        true
    }

    /// A kick, all at once, in newton-seconds: a jump, a thrown crate, an
    /// explosion's shove. Heavier bodies move less for the same kick.
    /// Unity's `AddForce(…, ForceMode.Impulse)`.
    pub fn add_impulse(&mut self, world: &World, entity: hecs::Entity, impulse: Vec3) -> bool {
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        body.apply_impulse(vector![impulse.x, impulse.y, impulse.z], true);
        true
    }

    /// A push for the coming step, in newtons: wind, a thruster, a current.
    /// Call it every step it pushes. Unity's `AddForce`.
    pub fn add_force(&mut self, world: &World, entity: hecs::Entity, force: Vec3) -> bool {
        let dt = self.parameters.dt;
        self.add_impulse(world, entity, force * dt)
    }

    /// A push for the coming step at a point of the body, in newtons, in
    /// the world: it turns the body as well as moving it — water lifting
    /// one end of a boat. Unity's `AddForceAtPosition`.
    pub fn add_force_at(&mut self, world: &World, entity: hecs::Entity, force: Vec3, at: Vec3) -> bool {
        let dt = self.parameters.dt;
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        let impulse = force * dt;
        body.apply_impulse_at_point(vector![impulse.x, impulse.y, impulse.z], point![at.x, at.y, at.z], true);
        true
    }

    /// Seconds a step.
    pub fn dt(&self) -> f32 {
        self.parameters.dt
    }

    /// A kick at a point of the body, in newton-seconds, in the world:
    /// what a rope's end did to the hand holding it over the last step.
    /// Unity's `AddForceAtPosition` with `ForceMode.Impulse`.
    pub fn add_impulse_at(&mut self, world: &World, entity: hecs::Entity, impulse: Vec3, at: Vec3) -> bool {
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        body.apply_impulse_at_point(vector![impulse.x, impulse.y, impulse.z], point![at.x, at.y, at.z], true);
        true
    }

    /// How fast it turns, radians a second about each axis, in the world.
    pub fn spin(&self, world: &World, entity: hecs::Entity) -> Option<Vec3> {
        let v = self.bodies.get(self.body_of(world, entity)?)?.angvel();
        Some(Vec3::new(v.x, v.y, v.z))
    }

    /// A twist for the coming step, in newton-metres, in the world: what a
    /// muscle does to a limb. Unity's `AddTorque`.
    pub fn add_torque(&mut self, world: &World, entity: hecs::Entity, torque: Vec3) -> bool {
        let dt = self.parameters.dt;
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        let t = torque * dt;
        body.apply_torque_impulse(vector![t.x, t.y, t.z], true);
        true
    }

    /// How hard it is to turn about each of its own axes, and those axes:
    /// its principal inertia, kilogram-metres².
    pub fn inertia(&self, world: &World, entity: hecs::Entity) -> Option<Vec3> {
        let body = self.bodies.get(self.body_of(world, entity)?)?;
        let i = body.mass_properties().local_mprops.principal_inertia();
        Some(Vec3::new(i.x, i.y, i.z))
    }

    /// Its mass, kilograms.
    pub fn mass(&self, world: &World, entity: hecs::Entity) -> Option<f32> {
        Some(self.bodies.get(self.body_of(world, entity)?)?.mass())
    }

    /// How fast a point of the body is going, in the world: its own speed
    /// and its turning together.
    pub fn velocity_at(&self, world: &World, entity: hecs::Entity, at: Vec3) -> Option<Vec3> {
        let body = self.bodies.get(self.body_of(world, entity)?)?;
        let v = body.velocity_at_point(&point![at.x, at.y, at.z]);
        Some(Vec3::new(v.x, v.y, v.z))
    }

    /// Where a ball of `radius` moving from `from` along `direction` first
    /// touches something — Unity's `SphereCast`: will a body this wide fit
    /// through, where does a thrown thing land. Triggers are looked
    /// through. The hit's point is where the ball's centre is then.
    pub fn sphere_cast(
        &self,
        from: Vec3,
        radius: f32,
        direction: Vec3,
        max_distance: f32,
    ) -> Option<RayHit> {
        let direction = direction.normalize_or_zero();
        if direction.length_squared() < 0.5 {
            return None;
        }
        let ball = Ball::new(radius.max(1e-4));
        let at = Isometry::translation(from.x, from.y, from.z);
        let (collider, hit) = self.queries.cast_shape(
            &self.bodies,
            &self.colliders,
            &at,
            &vector![direction.x, direction.y, direction.z],
            &ball,
            rapier3d::parry::query::ShapeCastOptions {
                max_time_of_impact: max_distance,
                stop_at_penetration: true,
                ..Default::default()
            },
            QueryFilter::default().exclude_sensors(),
        )?;
        Some(RayHit {
            point: from + direction * hit.time_of_impact,
            distance: hit.time_of_impact,
            collider: ColliderRef(collider),
            entity: self.entity_of(collider),
        })
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

/// Whether two joints differ at most in what drives them — a motor, a
/// spring's strength, limits — and are the same joint otherwise.
fn same_but_drive(a: &crate::scene::Joint, b: &crate::scene::Joint) -> bool {
    use crate::scene::Joint;
    match (*a, *b) {
        (
            Joint::Hinge {
                to, anchor, axis, ..
            },
            Joint::Hinge {
                to: t,
                anchor: n,
                axis: x,
                ..
            },
        ) => to == t && anchor == n && axis == x,
        (Joint::Slider { to, axis, .. }, Joint::Slider { to: t, axis: x, .. }) => {
            to == t && axis == x
        }
        (
            Joint::Spring { to, anchor, .. },
            Joint::Spring {
                to: t, anchor: n, ..
            },
        ) => to == t && anchor == n,
        _ => false,
    }
}

/// Set what drives a built joint to what the scene now says.
fn drive(data: &mut GenericJoint, joint: &crate::scene::Joint) {
    use crate::scene::Joint;
    match *joint {
        Joint::Hinge {
            limits_deg, motor, ..
        } => {
            if let Some((low, high)) = limits_deg {
                data.set_limits(JointAxis::AngX, [low.to_radians(), high.to_radians()]);
            }
            set_motor(data, JointAxis::AngX, motor, 1f32.to_radians());
        }
        Joint::Slider { limits, motor, .. } => {
            if let Some((low, high)) = limits {
                data.set_limits(JointAxis::LinX, [low, high]);
            }
            set_motor(data, JointAxis::LinX, motor, 1.0);
        }
        Joint::Spring {
            stiffness, damping, ..
        } => {
            for axis in [JointAxis::LinX, JointAxis::LinY, JointAxis::LinZ] {
                data.set_motor_model(axis, MotorModel::ForceBased);
                data.set_motor_position(axis, 0.0, stiffness.max(0.0), damping.max(0.0));
            }
        }
        _ => {}
    }
}

fn set_motor(
    data: &mut GenericJoint,
    axis: JointAxis,
    motor: Option<crate::scene::Motor>,
    unit: f32,
) {
    // A torque (a force), not an acceleration: Unity's joint springs and
    // motors are, so a heavy lever needs a stronger spring than a light one.
    data.set_motor_model(axis, MotorModel::ForceBased);
    match motor {
        Some(motor) => {
            let strength = motor.strength.max(0.0);
            match motor.hold {
                Some(hold) => {
                    data.set_motor_position(axis, hold * unit, strength, motor.damping());
                }
                None => {
                    data.set_motor_velocity(axis, motor.speed * unit, strength);
                }
            }
        }
        None => {
            data.set_motor_velocity(axis, 0.0, 0.0);
        }
    }
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
        Joint::Ball { anchor, .. } | Joint::Spring { anchor, .. } => (anchor, Vec3::X),
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
        // Nothing locked: the spring's motors do the holding.
        Joint::Spring { .. } => JointAxesMask::empty(),
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
    let (motor, axis, unit) = match *joint {
        Joint::Hinge { motor, .. } => (motor, JointAxis::AngX, 1f32.to_radians()),
        Joint::Slider { motor, .. } => (motor, JointAxis::LinX, 1.0),
        _ => (None, JointAxis::LinX, 1.0),
    };
    if let Some(motor) = motor {
        let strength = motor.strength.max(0.0);
        builder = builder.motor_model(axis, MotorModel::ForceBased);
        builder = match motor.hold {
            Some(hold) => builder.motor_position(axis, hold * unit, strength, motor.damping()),
            None => builder.motor_velocity(axis, motor.speed * unit, strength),
        };
    }
    if let Joint::Spring {
        stiffness, damping, ..
    } = *joint
    {
        for axis in [JointAxis::LinX, JointAxis::LinY, JointAxis::LinZ] {
            builder = builder
                .motor_model(axis, MotorModel::ForceBased)
                .motor_position(axis, 0.0, stiffness.max(0.0), damping.max(0.0));
        }
    }
    Some(builder.build())
}

/// Whether a placement moved enough to say so.
fn moved(a: &glam::Mat4, b: &glam::Mat4) -> bool {
    a.to_cols_array().iter().zip(b.to_cols_array()).any(|(x, y)| (x - y).abs() > 1e-5)
}

/// Where a world matrix puts a body: its translation and rotation. Scale
/// lives in the collider's shape.
fn isometry(placed: glam::Mat4) -> Isometry<Real> {
    let (_, rotation, translation) = placed.to_scale_rotation_translation();
    // A zero scale on an axis leaves no turn to read: none, rather than a
    // NaN that parry then panics on.
    let rotation = if rotation.is_finite() { rotation } else { glam::Quat::IDENTITY };
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
                // Triangles with no area — a mesh squashed flat by a zero
                // scale, a sliver — are nothing to stand on, and a query
                // against a tree of only those panics in parry.
                let triangles: Vec<[u32; 3]> = mesh
                    .triangles
                    .iter()
                    .copied()
                    .filter(|t| {
                        let [a, b, c] = t.map(|i| points.get(i as usize).copied());
                        let (Some(a), Some(b), Some(c)) = (a, b, c) else { return false };
                        (b - a).cross(&(c - a)).norm() > 1e-10
                    })
                    .collect();
                if triangles.is_empty() || points.iter().any(|p| !p.coords.iter().all(|v| v.is_finite())) {
                    return None;
                }
                ColliderBuilder::trimesh(points, triangles)
                    .ok()?
                    .build()
            }
        }
        ColliderShape::Box { half, center } => {
            let (h, c) = (half * scale, center * scale);
            ColliderBuilder::cuboid(h.x.max(1e-4), h.y.max(1e-4), h.z.max(1e-4))
                .translation(vector![c.x, c.y, c.z])
                .build()
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

// The tests sit mid-file, beside what they test; the module's glue follows.
#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Body, Collider as ColliderShape, EntityDesc, Scene, Transform};

    /// Put a scene into a world with this module dressing it: all a test of
    /// physics needs, with no look to resolve.
    fn spawn(scene: &Scene, world: &mut World) -> Vec<crate::world::Unresolved> {
        crate::world::spawn_scene_dressed(scene, world, &mut [Box::new(PhysicsDress)])
    }

    fn entity(name: &str, y: f32, body: Body, collider: ColliderShape) -> EntityDesc {
        EntityDesc {
            parts: Default::default(),
            in_part: None,
            inactive: false,
            overrides: Default::default(),
            components: Default::default(),
            id: Default::default(),
            name: name.into(),
            prefab: Default::default(),
            transform: Transform {
                position: Vec3::new(0.0, y, 0.0),
                ..Default::default()
            },
            children: Vec::new(),
        }
        .with(crate::scene::ModelRef("m".into()))
        .with(body)
        .with(collider)
    }

    /// A ball above a floor, and the clock to drop it with.
    #[test]
    fn colliders_under_a_body_are_its_parts_and_move_and_weigh_as_one() {
        // A shovel as Unity has it: the body on the root, the colliders on
        // its children, a trigger for its head.
        let text = r#"(entities: [
            (id: "0000000000000001", name: "floor", body: Static, collider: Box(half: (10.0, 0.1, 10.0))),
            (id: "0000000000000002", name: "shovel", body: Dynamic, physics: (mass: Some(2.0)),
             transform: (position: (0.0, 1.0, 0.0)),
             children: [
                (id: "0000000000000003", name: "blade", body: Part, collider: Box(half: (0.2, 0.05, 0.3)),
                 transform: (position: (0.0, 0.0, -0.6))),
                (id: "0000000000000004", name: "stick", body: Part, collider: Box(half: (0.03, 0.03, 0.6)),
                 transform: (position: (0.0, 0.0, 0.3))),
                (id: "0000000000000005", name: "head", body: TriggerPart, collider: Box(half: (0.3, 0.2, 0.3)),
                 transform: (position: (0.0, 0.0, -0.8))),
             ]),
            (id: "0000000000000006", name: "stone", body: Static, collider: Box(half: (0.2, 0.2, 0.2)),
             transform: (position: (0.0, 0.3, -0.8))),
        ])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        let mut world = World::new();
        spawn(&scene, &mut world);
        crate::world::apply_hierarchy(&mut world);
        let named = |world: &World, id: u64| {
            world
                .query::<(hecs::Entity, &crate::world::SceneId)>()
                .iter()
                .find(|(_, s)| s.0 == crate::id::EntityId::from_raw(id))
                .map(|(e, _)| e)
                .unwrap()
        };
        let (shovel, blade, head, stone) = (named(&world, 2), named(&world, 3), named(&world, 5), named(&world, 6));
        let _ = world.insert_one(head, Contacts::default());
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        physics.run(&mut world);
        let first = *world.get::<&BodyHandle>(shovel).unwrap();
        for _ in 0..90 {
            physics.run(&mut world);
            crate::world::apply_hierarchy(&mut world);
        }
        assert_eq!(*world.get::<&BodyHandle>(shovel).unwrap(), first, "built once, not every step");
        assert!(
            (physics.mass(&world, shovel).unwrap() - 2.0).abs() < 1e-3,
            "its line's mass, over its parts: {:?}",
            physics.mass(&world, shovel)
        );
        assert!(world.get::<&BodyHandle>(blade).is_err(), "a part has no body of its own");
        // It lies on its parts: the blade's bottom on the floor.
        let y = world.get::<&Transform>(shovel).unwrap().position.y;
        assert!(y < 0.9 && y > 0.1, "fell and lies on its parts: {y}");
        // A ray at the blade answers the shovel.
        let at = world.get::<&WorldTransform>(blade).unwrap().0.w_axis.truncate();
        let hit = physics.cast_ray(at + Vec3::Y * 2.0, Vec3::NEG_Y, 5.0).unwrap();
        assert_eq!(hit.entity, Some(shovel));
        // The head's own contacts: the stone it lies over.
        let _ = stone;
        assert!(
            world.get::<&Contacts>(head).is_ok(),
            "a part can have contacts of its own"
        );
    }

    #[test]
    fn a_hinge_says_how_far_it_turned_and_stops_at_its_limit() {
        let text = r#"(entities: [
            (id: "0000000000000001", name: "post", model: "builtin:cube", body: Static,
             collider: Box(half: (0.1, 1.0, 0.1))),
            (id: "0000000000000002", name: "door", model: "builtin:cube", body: Dynamic,
             transform: (position: (0.6, 0.0, 0.0)),
             collider: Box(half: (0.5, 1.0, 0.05)), physics: (gravity: 0.0),
             joint: Hinge(to: "0000000000000001", anchor: (-0.6, 0.0, 0.0), axis: (0.0, 1.0, 0.0), limits_deg: (-40.0, 40.0))),
        ])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        let mut world = World::new();
        spawn(&scene, &mut world);
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        physics.run(&mut world);
        let door = world
            .query::<(hecs::Entity, &Physics)>()
            .iter()
            .find(|(_, p)| p.0 == Body::Dynamic)
            .map(|(e, _)| e)
            .unwrap();
        assert!(physics.hinge_angle(&world, door).unwrap().abs() < 1.0, "built at rest");
        for _ in 0..120 {
            physics.add_torque(&world, door, Vec3::new(0.0, 20.0, 0.0));
            physics.run(&mut world);
        }
        let angle = physics.hinge_angle(&world, door).unwrap();
        assert!((angle.abs() - 40.0).abs() < 3.0, "at its limit: {angle}");
    }

    /// A crop tethered to its bed by a spring that breaks at 250 N: pulled
    /// gently it stays, pulled hard it comes loose.
    #[test]
    fn a_spring_tether_breaks_when_pulled_past_its_strength() {
        for (pull, loose) in [(100.0, false), (600.0, true)] {
            let text = r#"(entities: [
                (id: "0000000000000001", name: "sponge", model: "builtin:cube", body: Dynamic,
                 collider: Box(half: (0.2, 0.2, 0.2)), physics: (mass: Some(1.5), gravity: 0.0),
                 joint: Spring(stiffness: 1000.0, damping: 10.0), joint_break: 250.0),
            ])"#;
            let scene: Scene = ron::from_str(text).unwrap();
            let mut world = World::new();
            spawn(&scene, &mut world);
            let mut physics = PhysicsWorld::new(1.0 / 50.0);
            physics.run(&mut world);
            let sponge = world.query::<(hecs::Entity, &Physics)>().iter().next().map(|(e, _)| e).unwrap();
            for _ in 0..50 {
                physics.add_force(&world, sponge, Vec3::new(0.0, pull, 0.0));
                physics.run(&mut world);
            }
            assert_eq!(world.get::<&JointBroken>(sponge).is_ok(), loose, "pulled with {pull} N");
        }
    }

    /// A trigger squashed to nothing on an axis — an effect that grows from
    /// zero — sits on a mesh floor without a NaN turn reaching parry.
    #[test]
    fn a_body_scaled_to_nothing_on_an_axis_does_not_break_the_world() {
        let text = r#"(entities: [
            (id: "0000000000000001", name: "floor", model: "builtin:plane", transform: (scale: (10.0, 1.0, 10.0)),
             collider: Model, body: Static),
            (id: "0000000000000002", name: "puff", transform: (position: (0.0, 0.1, 0.0), scale: (0.0, 1.0, 1.0)),
             collider: Sphere(radius: 0.5), body: Trigger),
        ])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        let mut world = World::new();
        spawn(&scene, &mut world);
        attach_scene_collision_meshes(&mut world, &scene, None);
        let mut physics = PhysicsWorld::new(1.0 / 50.0);
        for _ in 0..3 {
            physics.run(&mut world);
        }
    }

    /// A mesh collider made of another model than the one drawn — or with
    /// none drawn at all.
    #[test]
    fn a_collision_model_is_what_a_model_collider_is_made_of() {
        let text = r#"(entities: [
            (id: "0000000000000001", name: "hidden", collider: Model, body: Static, collision_model: "builtin:sphere"),
        ])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        let mut world = World::new();
        spawn(&scene, &mut world);
        attach_scene_collision_meshes(&mut world, &scene, None);
        let mut physics = PhysicsWorld::new(1.0 / 50.0);
        physics.run(&mut world);
        assert!(!physics.overlap_sphere(Vec3::new(0.0, 0.5, 0.0), 0.1).is_empty(), "solid by the sphere, with nothing drawn");
    }

    /// A kinematic body under a parent that moves goes with it: a tool's
    /// animated head on the tool being carried.
    #[test]
    fn a_kinematic_body_is_carried_by_its_parent() {
        let text = r#"(entities: [
            (id: "0000000000000001", name: "tool", body: Dynamic, collider: Box(half: (0.1, 0.1, 0.1)),
             physics: (gravity: 0.0),
             children: [(id: "0000000000000002", name: "head", transform: (position: (0.0, 0.0, 1.0)),
               body: Kinematic, collider: Box(half: (0.2, 0.2, 0.2)))]),
        ])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        let mut world = World::new();
        spawn(&scene, &mut world);
        let mut physics = PhysicsWorld::new(1.0 / 50.0);
        physics.run(&mut world);
        let tool = world.query::<(hecs::Entity, &Physics)>().iter().find(|(_, p)| p.0 == Body::Dynamic).map(|(e, _)| e).unwrap();
        physics.set_velocity(&world, tool, Vec3::new(5.0, 0.0, 0.0));
        for _ in 0..50 {
            physics.run(&mut world);
            crate::world::apply_hierarchy(&mut world);
        }
        let head_now = Vec3::new(5.0, 0.0, 1.0);
        let found = physics.overlap_sphere(head_now, 0.1);
        assert!(!found.is_empty(), "the head's collider is where its parent took it: {found:?}");
        assert!(physics.overlap_sphere(Vec3::new(0.0, 0.0, 1.0), 0.1).is_empty(), "and not left where it started");
    }

    /// A hinge held at an angle by its spring gets there, and against a
    /// limit short of it rests on the limit.
    #[test]
    fn a_sprung_hinge_holds_its_angle_or_rests_on_the_stop_before_it() {
        for (hold, limits, expect) in [(45.0, "", 45.0), (-45.0, "", -45.0), (45.0, "limits_deg: (60.0, 80.0),", 60.0)] {
            let text = format!(
                r#"(entities: [
                (id: "0000000000000001", name: "post", model: "builtin:cube", body: Static,
                 collider: Box(half: (0.1, 1.0, 0.1))),
                (id: "0000000000000002", name: "door", model: "builtin:cube", body: Dynamic,
                 transform: (position: (0.6, 0.0, 0.0)),
                 collider: Box(half: (0.5, 1.0, 0.05)), physics: (gravity: 0.0),
                 joint: Hinge(to: "0000000000000001", anchor: (-0.6, 0.0, 0.0), axis: (0.0, 1.0, 0.0), {limits} motor: (hold: {hold}, strength: 20.0))),
            ])"#
            );
            let scene: Scene = ron::from_str(&text).unwrap();
            let mut world = World::new();
            spawn(&scene, &mut world);
            let mut physics = PhysicsWorld::new(1.0 / 60.0);
            let door = world
                .query::<(hecs::Entity, &Physics)>()
                .iter()
                .find(|(_, p)| p.0 == Body::Dynamic)
                .map(|(e, _)| e)
                .unwrap();
            for _ in 0..300 {
                physics.run(&mut world);
            }
            let angle = physics.hinge_angle(&world, door).unwrap();
            assert!((angle - expect).abs() < 1.0, "held at {hold} within {limits:?}: {angle}");
        }
    }

    /// Dacha's elevator lever: a metre-long bar on a mount tipped 45°,
    /// sprung toward 90° with a spring of 10 (a torque, as in Unity) and
    /// stopped at 75°. The spring beats its weight, so it rests on 75.
    #[test]
    fn a_lever_is_sprung_up_against_its_stop_through_its_weight() {
        let text = r#"(entities: [
            (id: "0000000000000001", name: "mount", model: "builtin:cube", body: Kinematic,
             transform: (position: (0.0, 1.0, 0.0), rotation_deg: (-45.0, 90.0, 0.0), scale: (0.4, 0.4, 0.4)),
             collider: Box(half: (0.5, 0.5, 0.5)), physics: (gravity: 0.0),
             children: [
               (id: "0000000000000002", name: "lever", model: "builtin:cube", body: Dynamic,
                transform: (position: (0.96, 0.0, 1.355), scale: (0.33, 0.33, 3.69)),
                collider: Box(half: (0.5, 0.5, 0.35), center: (0.0, 0.0, 0.15)), physics: (mass: Some(1.0)),
                joint: Hinge(to: "0000000000000001", anchor: (0.0, 0.0, -0.45), axis: (-1.0, 0.0, 0.0), limits_deg: (15.0, 75.0),
                  motor: (hold: 90.0, strength: 10.0, damping: 0.1))),
             ]),
        ])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        let mut world = World::new();
        spawn(&scene, &mut world);
        let mut physics = PhysicsWorld::new(1.0 / 50.0);
        let lever = world
            .query::<(hecs::Entity, &Physics)>()
            .iter()
            .find(|(_, p)| p.0 == Body::Dynamic)
            .map(|(e, _)| e)
            .unwrap();
        for _ in 0..100 {
            physics.run(&mut world);
        }
        let angle = physics.hinge_angle(&world, lever).unwrap();
        assert!((angle - 75.0).abs() < 1.0, "on its stop: {angle}");
    }

    #[test]
    fn a_body_resting_on_the_floor_is_pushed_up_by_it() {
        let (mut physics, mut world, ball) = dropped(0.8);
        let _ = world.insert_one(ball, Contacts::default());
        for _ in 0..90 {
            physics.run(&mut world);
            physics.update_contacts(&mut world);
        }
        let contacts = world.get::<&Contacts>(ball).unwrap();
        assert!(!contacts.normals.is_empty(), "resting, it touches the floor");
        assert!(contacts.normals.iter().all(|n| n.y > 0.9), "{:?}", contacts.normals);
    }

    fn dropped(from: f32) -> (PhysicsWorld, World, hecs::Entity) {
        let scene = Scene {
            entities: vec![
                entity(
                    "floor",
                    0.0,
                    Body::Static,
                    ColliderShape::Box {
                        half: Vec3::new(20.0, 0.1, 20.0),
                        center: glam::Vec3::ZERO,
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
        spawn(&scene, &mut world);
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
    fn the_wind_bowls_a_tumbleweed_along_and_leaves_a_stone_where_it_lay() {
        let weed = |x: f32, blown: f32| {
            let mut e = entity(
                "weed",
                0.6,
                Body::Dynamic,
                ColliderShape::Sphere { radius: 0.5 },
            );
            e.transform.position.x = x;
            let mut props = e.physics();
            props.blown = blown;
            props.density = 0.05;
            e.set_part(&props);
            e
        };
        let scene = Scene {
            entities: vec![
                entity(
                    "floor",
                    0.0,
                    Body::Static,
                    ColliderShape::Box {
                        half: Vec3::new(200.0, 0.1, 200.0),
                        center: glam::Vec3::ZERO,
                    },
                ),
                weed(0.0, 1.0),
                weed(-5.0, 0.0),
            ],
            ..Default::default()
        };
        let run = || {
            let mut world = World::new();
            spawn(&scene, &mut world);
            let mut physics = PhysicsWorld::new(1.0 / 60.0);
            physics.wind = runity_core::wind::Wind {
                direction: Vec3::new(0.0, 0.0, 1.0),
                strength: 1.5,
            };
            let mut highest = 0.0f32;
            for _ in 0..600 {
                physics.run(&mut world);
                for (_, t, p) in world.query::<(hecs::Entity, &Transform, &Props)>().iter() {
                    if p.0.blown > 0.0 {
                        highest = highest.max(t.position.y);
                    }
                }
            }
            let mut at: Vec<(f32, Vec3)> = world
                .query::<(&Transform, Option<&Props>, &Physics)>()
                .iter()
                .filter(|(_, _, b)| b.0 == Body::Dynamic)
                .map(|(t, p, _)| (p.map_or(0.0, |p| p.0.blown), t.position))
                .collect();
            at.sort_by(|a, b| a.0.total_cmp(&b.0));
            (at, highest)
        };
        let (at, highest) = run();
        let (stone, weed) = (at[0].1, at[1].1);
        // Ten seconds of a stiff breeze: well down the wind, and off the
        // ground now and then.
        assert!(weed.z > 20.0, "the tumbleweed went only {weed}");
        assert!(
            weed.x.abs() < weed.z * 0.2,
            "it went across the wind: {weed}"
        );
        assert!(highest > 1.1, "it never hopped: highest {highest}");
        assert!(
            stone.distance(Vec3::new(-5.0, 0.6, 0.0)) < 0.3,
            "the stone moved to {stone}"
        );
        // The same steps blow the same way.
        assert_eq!(run().0, at);
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
                parts: Default::default(),
                in_part: None,
                inactive: false,
                overrides: Default::default(),
                components: Default::default(),
                id: Default::default(),
                name: "boulder".into(),
                prefab: Default::default(),
                transform: crate::scene::Transform {
                    position: Vec3::new(0.0, 4.0, 0.0),
                    scale: Vec3::splat(3.0),
                    ..Default::default()
                },
                children: Vec::new(),
            }
            .with(crate::scene::ModelRef("m".into()))
            .with(Body::Dynamic)
            .with(ColliderShape::Sphere { radius: 0.5 })],
            ..Default::default()
        };
        let mut world = World::new();
        spawn(&scene, &mut world);
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
        spawn(&scene, &mut world);
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
                        center: glam::Vec3::ZERO,
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
                            center: glam::Vec3::ZERO,
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
                            center: glam::Vec3::ZERO,
                        },
                    )
                },
            ],
            ..Default::default()
        };
        let mut world = World::new();
        spawn(&scene, &mut world);
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
        spawn(&scene, &mut world);
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
                center: glam::Vec3::ZERO,
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
                center: glam::Vec3::ZERO,
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
        ramp.set_part(&crate::scene::ModelRef("builtin:ramp".into()));
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
        spawn(&scene, &mut world);
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

    #[test]
    fn a_body_someone_else_owns_is_moved_by_them_not_by_gravity() {
        let (mut physics, mut world, ball) = dropped(3.0);
        let _ = world.insert_one(ball, crate::world::Replica);
        for _ in 0..30 {
            physics.run(&mut world);
        }
        let y = world.get::<&Transform>(ball).unwrap().position.y;
        assert!(
            (y - 3.0).abs() < 1e-4,
            "a replica hangs where its owner last put it: {y}"
        );
        // Its owner moved it: here it goes where they say.
        world.get::<&mut Transform>(ball).unwrap().position.x = 2.0;
        crate::world::apply_hierarchy(&mut world);
        physics.run(&mut world);
        physics.run(&mut world);
        assert!(
            (physics
                .position(*world.get::<&BodyHandle>(ball).unwrap())
                .unwrap()
                .x
                - 2.0)
                .abs()
                < 1e-3
        );
        // Handed to this peer: it is ours to drop.
        let _ = world.remove_one::<crate::world::Replica>(ball);
        for _ in 0..30 {
            physics.run(&mut world);
        }
        assert!(
            world.get::<&Transform>(ball).unwrap().position.y < 2.9,
            "falls once it is ours"
        );
    }

    #[test]
    fn a_body_handed_over_in_flight_keeps_its_speed() {
        // Its owner throws it along x at 6 m/s, high above the floor; the
        // poses come in each step. Then it is handed to this peer.
        let (mut physics, mut world, ball) = dropped(10.0);
        let _ = world.insert_one(ball, crate::world::Replica);
        physics.run(&mut world);
        for step in 1..=20 {
            world.get::<&mut Transform>(ball).unwrap().position.x = 0.1 * step as f32;
            crate::world::apply_hierarchy(&mut world);
            physics.run(&mut world);
        }
        let _ = world.remove_one::<crate::world::Replica>(ball);
        physics.run(&mut world);
        let speed = physics.velocity(&world, ball).unwrap();
        assert!(
            (speed.x - 6.0).abs() < 0.5,
            "as fast as its owner threw it: {speed}"
        );
    }

    #[test]
    fn a_body_taken_over_goes_on_from_the_pose_and_speed_it_was_handed() {
        let (mut physics, mut world, ball) = dropped(10.0);
        let _ = world.insert_one(ball, crate::world::Replica);
        physics.run(&mut world);
        physics.run(&mut world);
        // Ours now, from further along than the picture had it.
        let _ = world.remove_one::<crate::world::Replica>(ball);
        world.get::<&mut Transform>(ball).unwrap().position = Vec3::new(3.0, 10.0, 0.0);
        let _ = world.insert_one(
            ball,
            crate::world::Takeover {
                velocity: Vec3::new(12.0, 0.0, 0.0),
                spin: Vec3::ZERO,
            },
        );
        crate::world::apply_hierarchy(&mut world);
        physics.run(&mut world);
        physics.run(&mut world);
        let speed = physics.velocity(&world, ball).unwrap();
        assert!((speed.x - 12.0).abs() < 0.5, "{speed}");
        let at = world.get::<&Transform>(ball).unwrap().position;
        assert!(at.x > 3.3, "on from where it was handed: {at}");
    }

    #[test]
    fn a_body_switched_off_does_not_fall_until_switched_on() {
        let (mut physics, mut world, ball) = dropped(3.0);
        crate::world::set_active(&mut world, ball, false);
        for _ in 0..30 {
            physics.run(&mut world);
        }
        assert_eq!(
            world.get::<&Transform>(ball).unwrap().position.y,
            3.0,
            "off: not simulated"
        );
        crate::world::set_active(&mut world, ball, true);
        for _ in 0..30 {
            physics.run(&mut world);
        }
        assert!(
            world.get::<&Transform>(ball).unwrap().position.y < 2.9,
            "on: it falls"
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
                center: glam::Vec3::ZERO,
            },
        );
        zone.set_part(&crate::scene::ModelRef(Default::default()));
        let scene = Scene {
            entities: vec![
                entity(
                    "floor",
                    0.0,
                    Body::Static,
                    ColliderShape::Box {
                        half: Vec3::new(20.0, 0.1, 20.0),
                        center: glam::Vec3::ZERO,
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
        spawn(&scene, &mut world);
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
                center: glam::Vec3::ZERO,
            },
        );
        platform.set_part(&crate::scene::ModelRef(Default::default()));
        let scene = Scene {
            entities: vec![
                platform,
                entity(
                    "crate",
                    0.6,
                    Body::Dynamic,
                    ColliderShape::Box {
                        half: Vec3::splat(0.5),
                        center: glam::Vec3::ZERO,
                    },
                ),
            ],
            ..Default::default()
        };
        let mut world = World::new();
        spawn(&scene, &mut world);
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
        spawn(&scene, &mut world);
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
    fn a_joint_pulled_past_its_break_force_breaks_once_and_says_so() {
        // A heavy block hanging from the world on a weak fixed joint.
        let (mut physics, mut world, scene) = scene_world(
            r#"(entities: [
                (id: "00000000000000c1", name: "block", model: "m", body: Dynamic,
                 collider: Box(half: (0.5, 0.5, 0.5)), physics: (density: 50.0),
                 transform: (position: (0.0, 4.0, 0.0)),
                 joint: Fixed(), joint_break: 100.0),
            ])"#,
        );
        let block = by_id(&world, scene.entities[0].id);
        run_for(&mut physics, &mut world, 30);
        assert!(
            world.get::<&JointBroken>(block).is_ok(),
            "held more than 100 N"
        );
        assert_eq!(physics.broken(), vec![block]);
        assert!(physics.broken().is_empty(), "said once");
        let y = world.get::<&WorldTransform>(block).unwrap().0.w_axis.y;
        run_for(&mut physics, &mut world, 30);
        let fallen = world.get::<&WorldTransform>(block).unwrap().0.w_axis.y;
        assert!(fallen < y - 0.5, "and it falls: {y} -> {fallen}");
        assert_eq!(physics.impulse_joints.len(), 0, "not built again");
    }

    #[test]
    fn a_spring_pulls_a_body_back_and_its_drive_changes_in_place() {
        // A ball on a spring to the world, pulled sideways by its start.
        let (mut physics, mut world, scene) = scene_world(
            r#"(entities: [
                (id: "00000000000000d1", name: "ball", model: "m", body: Dynamic,
                 collider: Sphere(radius: 0.25), physics: (gravity: 0.0),
                 transform: (position: (0.0, 2.0, 0.0)),
                 joint: Spring(stiffness: 40.0, damping: 4.0)),
            ])"#,
        );
        let ball = by_id(&world, scene.entities[0].id);
        run_for(&mut physics, &mut world, 2);
        physics.set_velocity(&world, ball, Vec3::new(4.0, 0.0, 0.0));
        run_for(&mut physics, &mut world, 120);
        let at = world
            .get::<&WorldTransform>(ball)
            .unwrap()
            .0
            .w_axis
            .truncate();
        assert!(
            (at - Vec3::new(0.0, 2.0, 0.0)).length() < 0.2,
            "pulled back: {at:?}"
        );

        // Stiffer while the game runs: the same joint, tuned where it is.
        let handle = world.get::<&JointBuilt>(ball).unwrap().handle;
        world.get::<&mut Jointed>(ball).unwrap().0 = crate::scene::Joint::Spring {
            to: crate::id::EntityId::UNASSIGNED,
            anchor: Vec3::ZERO,
            stiffness: 400.0,
            damping: 20.0,
        };
        run_for(&mut physics, &mut world, 1);
        assert_eq!(
            world.get::<&JointBuilt>(ball).unwrap().handle,
            handle,
            "not rebuilt"
        );
    }

    #[test]
    fn going_kinematic_and_back_keeps_the_body_and_its_speed() {
        let (mut physics, mut world, scene) = scene_world(
            r#"(entities: [
                (id: "00000000000000f1", name: "crate", model: "m", body: Dynamic,
                 collider: Box(half: (0.5, 0.5, 0.5)), physics: (gravity: 0.0),
                 transform: (position: (0.0, 2.0, 0.0))),
            ])"#,
        );
        let crate_ = by_id(&world, scene.entities[0].id);
        run_for(&mut physics, &mut world, 1);
        physics.set_velocity(&world, crate_, Vec3::new(3.0, 0.0, 0.0));
        let handle = *world.get::<&BodyHandle>(crate_).unwrap();
        world.get::<&mut Physics>(crate_).unwrap().0 = Body::Kinematic;
        run_for(&mut physics, &mut world, 1);
        assert_eq!(
            *world.get::<&BodyHandle>(crate_).unwrap(),
            handle,
            "the same body"
        );
        world.get::<&mut Physics>(crate_).unwrap().0 = Body::Dynamic;
        run_for(&mut physics, &mut world, 1);
        assert_eq!(*world.get::<&BodyHandle>(crate_).unwrap(), handle);
        let speed = physics.velocity(&world, crate_).unwrap();
        assert!(speed.x > 2.5, "still moving: {speed:?}");
    }

    #[test]
    fn two_physics_worlds_in_one_process_do_not_touch() {
        let text = r#"(entities: [
            (id: "00000000000000a9", name: "ball", model: "m", body: Dynamic,
             collider: Sphere(radius: 0.5), transform: (position: (0.0, 5.0, 0.0))),
        ])"#;
        let (mut one, mut first, scene) = scene_world(text);
        let (mut two, mut second, _) = scene_world(text);
        let (a, b) = (
            by_id(&first, scene.entities[0].id),
            by_id(&second, scene.entities[0].id),
        );
        run_for(&mut one, &mut first, 60);
        let fell = first.get::<&WorldTransform>(a).unwrap().0.w_axis.y;
        let still = second.get::<&WorldTransform>(b).unwrap().0.w_axis.y;
        assert!(fell < 4.0 && (still - 5.0).abs() < 1e-4, "{fell} {still}");
        run_for(&mut two, &mut second, 60);
        let same = second.get::<&WorldTransform>(b).unwrap().0.w_axis.y;
        assert!(
            (same - fell).abs() < 1e-4,
            "the same fall, deterministic: {fell} {same}"
        );
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

        // A ray along the row goes through both, nearest first.
        let all = physics.cast_ray_all(Vec3::new(-5.0, 0.5, 0.0), Vec3::X, 20.0);
        let order: Vec<_> = all.iter().map(|h| h.entity).collect();
        assert_eq!(order, [Some(near), Some(far)]);
        assert!((all[0].distance - 4.5).abs() < 1e-3, "{}", all[0].distance);
        // A box: a thin one between them overlaps nothing, turned along the
        // row it reaches both.
        assert!(physics
            .overlap_box(
                Vec3::new(3.0, 0.5, 0.0),
                Vec3::new(0.2, 0.2, 3.0),
                Quat::IDENTITY
            )
            .is_empty());
        let turned = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert_eq!(
            physics.overlap_box(Vec3::new(3.0, 0.5, 0.0), Vec3::new(0.2, 0.2, 3.0), turned),
            both
        );
    }

    #[test]
    fn a_motor_turns_a_hinge_and_a_spring_holds_it_at_an_angle() {
        let (mut physics, mut world, _) = scene_world(
            r#"(entities: [
                (id: "00000000000000d1", name: "wheel", model: "m", body: Dynamic,
                 collider: Box(half: (1.0, 0.1, 0.2)), transform: (position: (0.0, 2.0, 0.0)),
                 joint: Hinge(axis: (0.0, 1.0, 0.0), motor: (speed: 90.0, strength: 1000.0))),
                (id: "00000000000000d2", name: "door", model: "m", body: Dynamic,
                 collider: Box(half: (0.5, 1.0, 0.05)), transform: (position: (5.0, 2.0, 0.0)),
                 joint: Hinge(anchor: (-0.5, 0.0, 0.0), axis: (0.0, 1.0, 0.0), motor: (hold: 45.0, strength: 200.0))),
            ])"#,
        );
        let turned = |world: &World, id: &str| {
            let entity = by_id(world, format!("00000000000000{id}").parse().unwrap());
            let t = *world.get::<&Transform>(entity).unwrap();
            let (y, _, _) = t.rotation().to_euler(glam::EulerRot::YXZ);
            y.to_degrees()
        };
        run_for(&mut physics, &mut world, 30);
        let early = turned(&world, "d1");
        run_for(&mut physics, &mut world, 30);
        let later = turned(&world, "d1");
        // Half a second at 90° a second, near enough.
        assert!(
            ((later - early).abs() - 45.0).abs() < 8.0,
            "{early} → {later}"
        );
        run_for(&mut physics, &mut world, 240);
        let door = turned(&world, "d2").abs();
        assert!((door - 45.0).abs() < 6.0, "held at 45°: {door}");
    }

    #[test]
    fn a_box_collider_sits_around_its_center() {
        // A model whose origin is at its foot: its box is half its height up.
        let (mut physics, mut world, _) = scene_world(
            r#"(entities: [
                (name: "post", model: "m", body: Static, transform: (scale: (1.0, 2.0, 1.0)),
                 collider: Box(half: (0.5, 0.5, 0.5), center: (0.0, 0.5, 0.0))),
            ])"#,
        );
        physics.sync_from_world(&mut world);
        physics.refresh_queries();
        let hit = physics
            .cast_ray(Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Y, 20.0)
            .unwrap();
        assert!(
            (hit.point.y - 2.0).abs() < 1e-3,
            "top of a 2 m post from its foot: {}",
            hit.point.y
        );
        let text = ron::to_string(&crate::scene::Collider::Box {
            half: Vec3::splat(0.5),
            center: Vec3::ZERO,
        })
        .unwrap();
        assert!(!text.contains("center"), "left out when zero: {text}");
    }

    #[test]
    fn two_told_to_ignore_each_other_pass_through_and_the_rest_still_collide() {
        let (mut physics, mut world, _) = scene_world(
            r#"(entities: [
                (id: "00000000000000e1", name: "floor", model: "m", body: Static, collider: Box(half: (5.0, 0.1, 5.0))),
                (id: "00000000000000e2", name: "ghost", model: "m", body: Dynamic, collider: Sphere(radius: 0.25),
                 transform: (position: (0.0, 2.0, 0.0))),
                (id: "00000000000000e3", name: "ball", model: "m", body: Dynamic, collider: Sphere(radius: 0.25),
                 transform: (position: (2.0, 2.0, 0.0))),
            ])"#,
        );
        let floor = by_id(&world, "00000000000000e1".parse().unwrap());
        let ghost = by_id(&world, "00000000000000e2".parse().unwrap());
        let ball = by_id(&world, "00000000000000e3".parse().unwrap());
        physics.ignore_collision(ghost, floor, true);
        run_for(&mut physics, &mut world, 120);
        let y = |world: &World, e| world.get::<&Transform>(e).unwrap().position.y;
        assert!(
            y(&world, ghost) < -1.0,
            "through the floor: {}",
            y(&world, ghost)
        );
        assert!(
            (y(&world, ball) - 0.35).abs() < 0.05,
            "the other lands: {}",
            y(&world, ball)
        );
    }

    #[test]
    fn a_frozen_axis_holds_whatever_hits_it() {
        let (mut physics, mut world, _) = scene_world(
            r#"(entities: [
                (name: "floor", model: "m", body: Static, collider: Box(half: (20.0, 0.1, 20.0))),
                (id: "00000000000000f1", name: "pole", model: "m", body: Dynamic, collider: Box(half: (0.1, 1.0, 0.1)),
                 transform: (position: (0.0, 1.2, 0.0), rotation_deg: (0.0, 0.0, 10.0)),
                 physics: (freeze_turn: "xz")),
                (id: "00000000000000f2", name: "leaner", model: "m", body: Dynamic, collider: Box(half: (0.1, 1.0, 0.1)),
                 transform: (position: (3.0, 1.2, 0.0), rotation_deg: (0.0, 0.0, 10.0))),
                (id: "00000000000000f3", name: "hover", model: "m", body: Dynamic, collider: Sphere(radius: 0.2),
                 transform: (position: (6.0, 2.0, 0.0)), physics: (freeze_move: "y")),
            ])"#,
        );
        let tilt = |world: &World, id: &str| {
            let entity = by_id(world, format!("00000000000000{id}").parse().unwrap());
            *world.get::<&Transform>(entity).unwrap()
        };
        run_for(&mut physics, &mut world, 180);
        let pole = tilt(&world, "f1");
        let leaner = tilt(&world, "f2");
        assert!(
            (pole.rotation_deg.z - 10.0).abs() < 0.5,
            "upright as it was: {:?}",
            pole.rotation_deg
        );
        assert!(
            (leaner.rotation_deg.z - 10.0).abs() > 5.0,
            "the free one fell: {:?}",
            leaner.rotation_deg
        );
        assert!(
            (tilt(&world, "f3").position.y - 2.0).abs() < 1e-3,
            "held at its height"
        );
        let props: crate::scene::BodyProps = ron::from_str(r#"(freeze_turn: "xz")"#).unwrap();
        assert_eq!(ron::to_string(&props).unwrap(), r#"(freeze_turn:"xz")"#);
        assert!(ron::from_str::<crate::scene::BodyProps>(r#"(freeze_turn: "xw")"#).is_err());
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

    #[test]
    fn drag_slows_gravity_scales_and_a_fast_stone_does_not_pass_the_wall() {
        let (mut physics, mut world, scene) = scene_world(
            r#"(entities: [
                (name: "feather", model: "m", body: Dynamic, collider: Sphere(radius: 0.1),
                 transform: (position: (-5.0, 50.0, 0.0)), physics: (drag: 4.0)),
                (name: "stone", model: "m", body: Dynamic, collider: Sphere(radius: 0.1),
                 transform: (position: (5.0, 50.0, 0.0))),
                (name: "balloon", model: "m", body: Dynamic, collider: Sphere(radius: 0.1),
                 transform: (position: (0.0, 50.0, 5.0)), physics: (gravity: 0.0)),
            ])"#,
        );
        let (feather, stone, balloon) = (
            by_id(&world, scene.entities[0].id),
            by_id(&world, scene.entities[1].id),
            by_id(&world, scene.entities[2].id),
        );
        run_for(&mut physics, &mut world, 60);
        let y = |e| world.get::<&WorldTransform>(e).unwrap().0.w_axis.y;
        assert!(
            y(feather) > y(stone) + 2.0,
            "drag: {} vs {}",
            y(feather),
            y(stone)
        );
        assert!((y(balloon) - 50.0).abs() < 1e-3, "floats: {}", y(balloon));

        // A stone thrown at 300 m/s at a thin wall: without `fast` it is
        // past the wall between two steps; with it, it stops there.
        let throw = |fast: bool| {
            let (mut physics, mut world, scene) = scene_world(&format!(
                r#"(entities: [
                    (name: "wall", model: "m", body: Static, collider: Box(half: (0.05, 5.0, 5.0))),
                    (name: "stone", model: "m", body: Dynamic, collider: Sphere(radius: 0.05),
                     transform: (position: (-3.0, 0.0, 0.0)), physics: (gravity: 0.0, fast: {fast})),
                ])"#
            ));
            let stone = by_id(&world, scene.entities[1].id);
            physics.sync_from_world(&mut world);
            let handle = world.get::<&BodyHandle>(stone).unwrap().0;
            physics
                .bodies
                .get_mut(handle)
                .unwrap()
                .set_linvel(vector![300.0, 0.0, 0.0], true);
            run_for(&mut physics, &mut world, 10);
            let x = world.get::<&WorldTransform>(stone).unwrap().0.w_axis.x;
            x
        };
        assert!(throw(false) > 0.5, "tunnelled: {}", throw(false));
        assert!(throw(true) < 0.0, "stopped at the wall: {}", throw(true));
    }

    #[test]
    fn a_game_throws_kicks_and_pushes_bodies_and_a_ball_is_cast_through_a_gap() {
        let (mut physics, mut world, scene) = scene_world(
            r#"(entities: [
                (name: "crate", model: "m", body: Dynamic, collider: Box(half: (0.5, 0.5, 0.5)),
                 transform: (position: (0.0, 5.0, 0.0)), physics: (gravity: 0.0)),
                (name: "heavy", model: "m", body: Dynamic, collider: Box(half: (0.5, 0.5, 0.5)),
                 transform: (position: (5.0, 5.0, 0.0)), physics: (gravity: 0.0, density: 4.0)),
                (name: "post", model: "m", body: Static, collider: Box(half: (0.1, 3.0, 0.1)),
                 transform: (position: (0.0, 0.0, -5.0))),
            ])"#,
        );
        let (light, heavy) = (
            by_id(&world, scene.entities[0].id),
            by_id(&world, scene.entities[1].id),
        );
        assert!(
            !physics.add_impulse(&world, light, Vec3::X),
            "no body before the first sync"
        );
        physics.sync_from_world(&mut world);

        assert!(physics.set_velocity(&world, light, Vec3::new(0.0, 0.0, 3.0)));
        run_for(&mut physics, &mut world, 60);
        let z = world.get::<&WorldTransform>(light).unwrap().0.w_axis.z;
        assert!((z - 3.0).abs() < 0.05, "3 m/s for a second: {z}");

        // The same kick moves the heavy one a quarter as fast.
        physics.add_impulse(&world, light, Vec3::new(2.0, 0.0, 0.0));
        physics.add_impulse(&world, heavy, Vec3::new(2.0, 0.0, 0.0));
        let (a, b) = (
            physics.velocity(&world, light).unwrap().x,
            physics.velocity(&world, heavy).unwrap().x,
        );
        assert!((a / b - 4.0).abs() < 0.01, "{a} vs {b}");

        // A force, every step for a second, is its impulse spread out.
        let before = physics.velocity(&world, heavy).unwrap().y;
        for _ in 0..60 {
            physics.add_force(&world, heavy, Vec3::new(0.0, 4.0, 0.0));
            run_for(&mut physics, &mut world, 1);
        }
        let after = physics.velocity(&world, heavy).unwrap().y;
        let mass = physics
            .bodies
            .get(world.get::<&BodyHandle>(heavy).unwrap().0)
            .unwrap()
            .mass();
        assert!(
            ((after - before) - 4.0 / mass).abs() < 0.01,
            "{before} -> {after}, mass {mass}"
        );

        // A ball cast at the post: a thin one passes beside it, a wide one hits.
        physics.refresh_queries();
        let from = Vec3::new(0.4, 0.0, 0.0);
        assert!(
            physics.sphere_cast(from, 0.2, Vec3::NEG_Z, 10.0).is_none(),
            "fits past"
        );
        let hit = physics
            .sphere_cast(from, 0.5, Vec3::NEG_Z, 10.0)
            .expect("too wide");
        // 0.3 m to the side of the post's edge, so it touches when 0.4 m in
        // front of its face at z = -4.9: the centre is then at z = -4.5.
        assert!((hit.distance - 4.5).abs() < 0.05, "{}", hit.distance);
    }
}

/// The physics module's dresser ([`crate::world::Dress`]): how a line is
/// solid — its body, its shape, what it is made of, its layer, its joint
/// and when the joint breaks.
pub struct PhysicsDress;

impl crate::world::Dress for PhysicsDress {
    fn parts(&self) -> &[&'static str] {
        &["body", "collider", "physics", "joint", "joint_break"]
    }

    fn dress(
        &mut self,
        line: &crate::scene::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        changed: crate::world::Changed,
        _: &mut Vec<crate::world::Unresolved>,
    ) {
        use crate::world::{JointBreak, JointBroken, Jointed, Props, Shape};
        if changed.has("body") {
            let _ = world.insert_one(entity, crate::world::Physics(line.body()));
        }
        if changed.has("collider") {
            let _ = world.insert_one(entity, Shape(line.collider()));
        }
        if changed.has("physics") {
            let props = line.physics();
            if props.is_default() {
                let _ = world.remove_one::<Props>(entity);
            } else {
                let _ = world.insert_one(entity, Props(props));
            }
        }
        if changed.has("joint") {
            let joint = line.joint();
            if joint.is_none() {
                let _ = world.remove_one::<Jointed>(entity);
            } else {
                let _ = world.insert_one(entity, Jointed(joint));
            }
            // A joint set anew is whole again.
            let _ = world.remove_one::<JointBroken>(entity);
        }
        if changed.has("joint_break") {
            match line.joint_break() {
                Some(force) => {
                    let _ = world.insert_one(entity, JointBreak(force));
                }
                None => {
                    let _ = world.remove_one::<JointBreak>(entity);
                }
            }
        }
    }
}
