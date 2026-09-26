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
//! and they will want their own numbers anyway. What is here is the query
//! such a controller is made of — [`PhysicsWorld::walk_capsule`], a person
//! swept through the world with the caller's numbers — which motion
//! matching walks with.

#[allow(unused_imports)]
use crate::prelude::*;
use glam::{Quat, Vec3};
use hecs::World;
use rapier3d::prelude::*;

use crate::scene::{Body, Collider as ColliderShape, Transform};
use crate::world::{
    JointBreak, JointBroken, Jointed, Layer, Parent, Physics, Props, SceneId, Shape, WorldTransform,
};

// rapier speaks glam too, but its own version of it: the engine's vectors
// and turns cross over by value, here and nowhere else.

/// The engine's vector as rapier's.
fn rv(v: Vec3) -> Vector {
    Vector::new(v.x, v.y, v.z)
}

/// rapier's vector as the engine's.
fn gv(v: Vector) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// The engine's turn as rapier's.
fn rq(q: Quat) -> Rotation {
    Rotation::from_xyzw(q.x, q.y, q.z, q.w)
}

/// rapier's turn as the engine's.
fn gq(q: Rotation) -> Quat {
    Quat::from_xyzw(q.x, q.y, q.z, q.w)
}

/// Who is touching an entity's body: for a [`Body::Trigger`], what is
/// inside it; for a solid body, what it is in contact with.
///
/// A trigger gets one when its body is built. Any other body gets one when
/// the game inserts `Contacts::default()` on the entity — contacts are
/// tracked only where someone asked, because most of a scene never needs
/// to know. Rewritten after every step: `entered` and `left` are this
/// step's changes, so a system that runs every step sees each exactly once.
/// Unity's `OnTriggerEnter`/`OnCollisionEnter`, as data a system queries
/// rather than callbacks on a class — and, as PhysX's, what the step found
/// at its *start*, before it moved anything: a system reading them on the
/// next step reads where things were a step ago, as Unity's scripts do.
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
    /// Of `inside`, what touches through a trigger — this body's or the
    /// other's: Unity's `OnTrigger*`. The rest of `inside` is solid
    /// contact, its `OnCollision*`. Something touching both ways is in
    /// both.
    pub triggers: Vec<hecs::Entity>,
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
    parts: Vec<PartKey>,
    /// Where its parts sit on it ([`parts_placement`]): a change moves
    /// their colliders on the body, which keeps its speed — a collider
    /// under an animated child, as Unity's compound follows it.
    parts_at: u64,
}

/// What one part was built from: compared field by field with what is
/// found each step, which formats and allocates nothing.
#[derive(Debug, Clone, PartialEq)]
struct PartKey {
    entity: hecs::Entity,
    shape: ColliderShape,
    props: crate::scene::BodyProps,
    layer: String,
    trigger: bool,
    mesh: usize,
}

impl PartKey {
    fn of(part: &PartFound) -> Self {
        PartKey {
            entity: part.entity,
            shape: part.shape,
            props: part.props.solved(),
            layer: part.layer.clone(),
            trigger: part.trigger,
            mesh: part.mesh.as_ref().map_or(0, CollisionMesh::key),
        }
    }

    fn is(&self, part: &PartFound) -> bool {
        self.entity == part.entity
            && self.shape == part.shape
            && self.props == part.props.solved()
            && self.layer == part.layer
            && self.trigger == part.trigger
            && self.mesh == part.mesh.as_ref().map_or(0, CollisionMesh::key)
    }
}

/// Whether a body's parts are still the ones it was built with.
fn same_parts(built: &[PartKey], found: &[PartFound]) -> bool {
    built.len() == found.len() && built.iter().zip(found).all(|(k, p)| k.is(p))
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
fn parts_of(world: &World, off: &scrap_core::hash::FastSet<hecs::Entity>) -> scrap_core::hash::FastMap<hecs::Entity, Vec<PartFound>> {
    let mut out: scrap_core::hash::FastMap<hecs::Entity, Vec<PartFound>> = Default::default();
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

/// Where a body's parts sit on it, to the millimetre.
fn parts_placement(parts: &[PartFound]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
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
/// Where a step took a body from and to, for a frame between steps to
/// draw it by ([`crate::world::interpolate`]); drawn [`AtStep`], both are
/// where it is now, and it is drawn there.
///
/// [`AtStep`]: crate::scene::Drawn::AtStep
fn stepped(from: glam::Mat4, to: glam::Mat4, drawn: crate::scene::Drawn) -> crate::world::Stepped {
    use crate::scene::Drawn;
    crate::world::Stepped {
        from: if drawn == Drawn::AtStep { to } else { from },
        to,
        ahead: drawn == Drawn::Ahead,
    }
}

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
                scrap_core::world::take_off::<CollisionMesh>(world, entity);
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
    pub wind: scrap_core::wind::Wind,
    /// Steps taken: the clock gusts and hops are read from, so a replay
    /// blows the same way.
    steps: u64,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    parameters: IntegrationParameters,
    /// Joints broken since [`PhysicsWorld::broken`] was last asked.
    broken: Vec<hecs::Entity>,
    /// A body's speed when it went kinematic, for when it goes dynamic.
    held_speed: std::collections::HashMap<RigidBodyHandle, (Vector, Vector)>,
    /// When a still body falls asleep: slower than this (m/s, rad/s) for
    /// so long (s). `None` is rapier's own (0.4 m/s, 0.5 rad/s, 2 s).
    sleep: Option<(f32, f32, f32)>,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd: CCDSolver,
    /// None are made; rapier steps them with the rest.
    soft_bodies: SoftBodySet,
    /// A fixed body with no shape, for joints to the world to hang from.
    /// Made the first time one asks.
    ground: Option<RigidBodyHandle>,
    layers: crate::layers::Layers,
    /// Pairs of entities told to pass through each other, by their bits,
    /// smaller first.
    ignored: std::collections::HashSet<(u64, u64)>,
    /// Shapes built from meshes, kept for the next body of the same mesh.
    mesh_shapes: MeshShapes,
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
        (!self.ignores(context)).then_some(SolverFlags::COMPUTE_RIGID_IMPULSES)
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
    /// Just built and hung on the world: after the next step it lets go of
    /// the still things it was put into ([`PhysicsWorld::free_hinged`]).
    fresh: bool,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new(1.0 / 60.0)
    }
}

impl PhysicsWorld {
    /// How many times a step the solver goes over the contacts and joints:
    /// more holds a heavy jointed thing to its hinge where contacts push it
    /// (a door standing in the ground), at a little more time a step. Four
    /// unless set.
    pub fn set_solver_iterations(&mut self, iterations: usize) {
        let (was, now) = (self.parameters.num_solver_iterations, iterations.max(1));
        self.parameters.num_solver_iterations = now;
        // A body that asked for more than the world keeps what it asked.
        for (_, body) in self.bodies.iter_mut() {
            let extra = body.additional_solver_iterations();
            if extra > 0 {
                body.set_additional_solver_iterations((was + extra).saturating_sub(now));
            }
        }
    }

    /// When a still body falls asleep: slower than `linear` m/s and
    /// `angular` rad/s for `seconds`. PhysX (Unity) sleeps a body after 0.4 s
    /// under about a tenth of a metre a second; rapier waits two seconds,
    /// long enough for a standing thing's last drift to topple it.
    pub fn set_sleep(&mut self, linear: f32, angular: f32, seconds: f32) {
        self.sleep = Some((linear, angular, seconds));
        for (_, body) in self.bodies.iter_mut() {
            let a = body.activation_mut();
            a.normalized_linear_threshold = linear;
            a.angular_threshold = angular;
            a.time_until_sleep = seconds;
        }
    }

    /// Wake a body through the islands, so what it is joined to or rests
    /// on wakes with it — PhysX wakes a joined chain when one link is
    /// pushed; a body woken by itself stays held by its sleeping neighbours
    /// as by a wall.
    fn wake(&mut self, world: &World, entity: hecs::Entity) {
        if let Some(handle) = self.body_of(world, entity) {
            self.islands.wake_up(&mut self.bodies, handle, true);
        }
    }

    /// `fixed_delta` must be the simulation clock's step, not a frame delta.
    pub fn new(fixed_delta: f32) -> Self {
        let parameters = IntegrationParameters {
            dt: fixed_delta,
            ..Default::default()
        };
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
            wind: scrap_core::wind::Wind {
                direction: Vec3::X,
                strength: 0.0,
            },
            steps: 0,
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            broken: Vec::new(),
            held_speed: Default::default(),
            sleep: None,
            parameters,
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            soft_bodies: SoftBodySet::new(),
            ground: None,
            layers: crate::layers::Layers::default(),
            ignored: Default::default(),
            mesh_shapes: Default::default(),
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

    /// What an entity's body is pressed against now, by entity, with the
    /// deepest contact's depth (negative is sunk in): for finding what
    /// pushes a thing.
    pub fn pressed_against(&self, world: &World, entity: hecs::Entity) -> Vec<(Option<hecs::Entity>, f32)> {
        let Ok(handle) = world.get::<&BodyHandle>(entity).map(|h| h.0) else { return Vec::new() };
        let Some(body) = self.bodies.get(handle) else { return Vec::new() };
        let mut out = Vec::new();
        for &collider in body.colliders() {
            for pair in self.narrow_phase.contact_pairs_with(collider) {
                if !pair.has_any_active_contact() {
                    continue;
                }
                let other = if pair.collider1 == collider { pair.collider2 } else { pair.collider1 };
                let who = self.colliders.get(other).and_then(|c| hecs::Entity::from_bits(c.user_data as u64));
                let depth = pair.manifolds().iter().flat_map(|m| m.points.iter()).map(|p| p.dist).fold(f32::MAX, f32::min);
                out.push((who, depth));
            }
        }
        out
    }

    /// Every joint built, as the two entities it holds together (`None`
    /// for the world's ground): what a line's `joint` came to.
    pub fn joined(&self) -> Vec<(Option<hecs::Entity>, Option<hecs::Entity>)> {
        let entity = |h: RigidBodyHandle| {
            self.bodies.get(h).and_then(|b| hecs::Entity::from_bits(b.user_data as u64))
        };
        self.impulse_joints
            .iter()
            .map(|(_, j)| (entity(j.body1()), entity(j.body2())))
            .collect()
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
        let mut live: scrap_core::hash::FastSet<RigidBodyHandle> = Default::default();
        let mut switched: Vec<(hecs::Entity, RigidBodyHandle, Body)> = Vec::new();
        let mut reseat: Vec<(hecs::Entity, glam::Mat4)> = Vec::new();
        // What is switched off has no body.
        // Asked only about what has physics: the rest of the level's
        // entities are nothing to the solver.
        let off = crate::world::inactive_among(world, world.query::<(hecs::Entity, &Physics)>().iter().map(|(e, _)| e));
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
            if !same_parts(&built.parts, parts.get(&entity).map(Vec::as_slice).unwrap_or(&[])) {
                stale.push(entity);
                continue;
            }
            if let Some(mine) = parts.get(&entity) {
                if parts_placement(mine) != built.parts_at {
                    reseat.push((entity, placed.0));
                }
            }
            // Dynamic ↔ kinematic keeps the body and its speed: switched in
            // place, as Unity's isKinematic does.
            let switch = built.body != body
                && matches!(built.body, Body::Dynamic | Body::Kinematic)
                && matches!(body, Body::Dynamic | Body::Kinematic)
                && built.collider == shape.0
                && built.mesh == mesh
                && built.props.solved() == props.solved()
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
                || built.props.solved() != props.solved()
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
        // Parts that moved on their body: their colliders moved with them.
        for (owner, placed) in reseat {
            let Some(mine) = parts.get(&owner) else { continue };
            for part in mine {
                let Ok(handle) = world.get::<&PartCollider>(part.entity).map(|h| h.0) else { continue };
                let own = shape_offset(part.shape, part.placed);
                let offset = isometry(placed).inverse() * isometry(part.placed);
                if let Some(c) = self.colliders.get_mut(handle) {
                    c.set_position_wrt_parent(offset * own);
                }
            }
            if let Ok(mut built) = world.get::<&mut Built>(owner) {
                built.parts_at = parts_placement(mine);
            }
        }
        for (entity, handle, kind) in switched {
            if let Some(body) = self.bodies.get_mut(handle) {
                match kind {
                    Body::Kinematic => {
                        // Its speed, kept for when it is let go again.
                        self.held_speed
                            .insert(handle, (body.linvel(), body.angvel()));
                        body.set_body_type(RigidBodyType::KinematicPositionBased, true);
                    }
                    _ => {
                        body.set_body_type(RigidBodyType::Dynamic, true);
                        // Moved while held: it goes on as it was moved. Held
                        // still: as fast as when it was taken.
                        let kept = self.held_speed.remove(&handle);
                        if let Some((linear, angular)) = kept {
                            if body.linvel().length() < 1e-3 && body.angvel().length() < 1e-3 {
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
                &mut self.soft_bodies,
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
                        body.set_linvel(Vector::new(0.0, 0.0, 0.0), true);
                        body.set_angvel(Vector::new(0.0, 0.0, 0.0), true);
                    }
                }
            }
            if let Ok(mut built) = world.get::<&mut Built>(entity) {
                built.local = local;
                built.placed = placed;
            }
        }

        self.make_mesh_shapes(world, &off, &parts);
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
            // Almost every body is already built: out before anything is
            // copied for it.
            if existing.is_some() || physics.0 == Body::None || off.contains(&entity) {
                continue;
            }
            let kind = solved(physics.0, replica.is_some());
            let props = props.map(|p| p.0).unwrap_or_default();
            let layer = layer.map(|l| l.0.clone()).unwrap_or_default();
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
            let own = build_collider(shape.0, placed.0, mesh, dynamic, &self.mesh_shapes);
            if own.is_none() && mine.is_empty() && kind != Body::Kinematic {
                // Declared solid with no shape to be solid with. Skipped
                // rather than guessed at — a box invented from a mesh's
                // bounds is the kind of default that is wrong quietly. A
                // kinematic one is kept, shapeless: it goes only where it
                // is carried, and is something to hang a joint on, as
                // Unity's Rigidbody with no collider is — the anchor a
                // mouse's tail swings from.
                continue;
            }
            // Its own collider, or — for a body made only of its parts — none.
            let mut collider = own.unwrap_or_else(|| ColliderBuilder::ball(1e-3).sensor(true).build());
            // rapier sweeps any dynamic body fast for its size, asked or
            // not: an unswept one is made of shapes too thick to sweep.
            let unswept = dynamic && props.unswept;
            if unswept {
                unsweep(&mut collider);
            }
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
            if kind == Body::Trigger {
                collider.set_sensor(true);
                collider.set_active_collision_types(zone_notices(props.notices_still));
            }
            let body = match kind {
                Body::Dynamic => RigidBodyBuilder::dynamic(),
                Body::Kinematic | Body::Trigger => RigidBodyBuilder::kinematic_position_based(),
                _ => RigidBodyBuilder::fixed(),
            }
            .pose(isometry(placed.0))
            .linear_damping(props.drag.max(0.0))
            .angular_damping(props.spin_drag.max(0.0))
            .gravity_scale(props.gravity)
            // Swept between steps: a thin thing — a shovel's stick, a
            // knife — falling onto ground that is a mesh with no inside
            // would otherwise pass through it. Rapier sweeps only what
            // moves further in a step than it is thick, so a resting
            // level costs nothing for it; against what stands still it
            // sweeps every dynamic body so (an unswept one is kept out by
            // its shapes, above), and this sweeps it against what moves
            // too. (`fast` is kept for lines that ask; it is always so now.)
            .ccd_enabled(props.fast || (kind == Body::Dynamic && !props.unswept))
            .additional_solver_iterations(
                (props.solver_iterations as usize).saturating_sub(self.parameters.num_solver_iterations),
            )
            .locked_axes(locked(&props))
            .build();
            let mut body = body;
            body.user_data = entity.to_bits().get() as u128;
            if let Some((linear, angular, seconds)) = self.sleep {
                let a = body.activation_mut();
                a.normalized_linear_threshold = linear;
                a.angular_threshold = angular;
                a.time_until_sleep = seconds;
            }
            let handle = self.bodies.insert(body);
            let solid_own = shape.0 != ColliderShape::None;
            self.colliders
                .insert_with_parent(collider, handle, &mut self.bodies);
            // Its parts: each where it sits on the body, gripping and
            // colliding as its own line says, weighing its share.
            let mut part_colliders: Vec<(ColliderHandle, f32)> = Vec::new();
            for part in mine {
                let Some(mut c) = build_collider(part.shape, part.placed, part.mesh.as_ref(), dynamic, &self.mesh_shapes) else {
                    continue;
                };
                if unswept {
                    unsweep(&mut c);
                }
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
                    // On a body that moves, a trigger meets what stands
                    // still as well (Unity's trigger under a Rigidbody); on
                    // one that does not, it is a zone like any other.
                    c.set_active_collision_types(if matches!(kind, Body::Dynamic | Body::Kinematic) {
                        ActiveCollisionTypes::all()
                    } else {
                        zone_notices(part.props.notices_still || props.notices_still)
                    });
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
                    parts: mine.iter().map(PartKey::of).collect(),
                    parts_at: parts_placement(mine),
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
                body.set_linvel(Vector::new(v.x, v.y, v.z), true);
                body.set_angvel(Vector::new(w.x, w.y, w.z), true);
            }
            scrap_core::world::take_off::<crate::world::Takeover>(world, entity);
        }
        self.sync_joints(world);
    }

    /// Build, before the bodies that want them, every mesh shape not built
    /// yet — each once, however many bodies share it, and several at once
    /// across the cores. A shape is the same whoever builds it, so this only
    /// changes when it is ready, not what it is. Shapes whose mesh nothing
    /// else holds any more are let go.
    fn make_mesh_shapes(
        &mut self,
        world: &World,
        off: &scrap_core::hash::FastSet<hecs::Entity>,
        parts: &scrap_core::hash::FastMap<hecs::Entity, Vec<PartFound>>,
    ) {
        let mut wanted: Vec<(MeshShapeKey, CollisionMesh, Vec3)> = Vec::new();
        let want = |mesh: &CollisionMesh, placed: glam::Mat4, dynamic: bool, wanted: &mut Vec<(MeshShapeKey, CollisionMesh, Vec3)>| {
            let (scale, _, _) = placed.to_scale_rotation_translation();
            let key = MeshShapeKey::of(mesh, scale, dynamic);
            if !self.mesh_shapes.contains_key(&key) && wanted.iter().all(|w| w.0 != key) {
                wanted.push((key, mesh.clone(), scale));
            }
        };
        for (entity, placed, physics, shape, existing, mesh, replica) in world
            .query::<(
                hecs::Entity,
                &WorldTransform,
                &Physics,
                &Shape,
                Option<&BodyHandle>,
                Option<&CollisionMesh>,
                Option<&crate::world::Replica>,
            )>()
            .iter()
        {
            if existing.is_some() || physics.0 == Body::None || physics.0.is_part() || off.contains(&entity) {
                continue;
            }
            let dynamic = solved(physics.0, replica.is_some()) == Body::Dynamic;
            if let (ColliderShape::Model, Some(mesh)) = (shape.0, mesh) {
                want(mesh, placed.0, dynamic, &mut wanted);
            }
            for part in parts.get(&entity).map(Vec::as_slice).unwrap_or(&[]) {
                if let (ColliderShape::Model, Some(mesh)) = (part.shape, part.mesh.as_ref()) {
                    want(mesh, part.placed, dynamic, &mut wanted);
                }
            }
        }
        if wanted.is_empty() {
            return;
        }
        let built = scrap_core::jobs::map(&wanted, 1, |(key, mesh, scale)| model_shape(mesh, *scale, key.dynamic));
        for ((key, mesh, _), shape) in wanted.into_iter().zip(built) {
            if let Some(shape) = shape {
                self.mesh_shapes.insert(key, MadeShape { mesh, shape });
            }
        }
        // Held by more than the shapes kept of it (one a scale): something
        // still has the mesh.
        let mut kept: scrap_core::hash::FastMap<usize, usize> = Default::default();
        for key in self.mesh_shapes.keys() {
            *kept.entry(key.mesh).or_default() += 1;
        }
        self.mesh_shapes
            .retain(|key, made| std::sync::Arc::strong_count(&made.mesh.vertices) > kept[&key.mesh]);
    }

    /// Joints after bodies: build each once both of its bodies exist,
    /// rebuild it when the joint or either body changed, drop it when its
    /// entity no longer asks for one. A joint whose partner is not there —
    /// not built yet, or a typo'd id — waits rather than guessing.
    fn sync_joints(&mut self, world: &mut World) {
        // No joint asked for and none built: most worlds, most steps.
        if world.query::<&Jointed>().iter().next().is_none()
            && world.query::<&JointBuilt>().iter().next().is_none()
        {
            return;
        }
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
            // The other body's scale, which its end's anchor is given in.
            let one_scale = hecs::Entity::from_bits(one.user_data as u64)
                .and_then(|e| world.get::<&WorldTransform>(e).ok().map(|t| t.0.to_scale_rotation_translation().0))
                .unwrap_or(Vec3::ONE);
            let data = joint_data(&joint, placed, one.position(), two.position(), one_scale);
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
                    // Only a hinge or a slider — a door set flush in the
                    // sand, a handle in its slot. A thing
                    // tethered on a spring (a ripe sponge in its bed) stands
                    // on what it was set into: freed of it, it fell through
                    // and tore its tether.
                    fresh: Some(other) == self.ground
                        && matches!(joint, crate::scene::Joint::Hinge { .. } | crate::scene::Joint::Slider { .. }),
                },
            );
        }
    }

    /// Rewrite every [`Contacts`] from what rapier found this step. Part of
    /// [`PhysicsWorld::run`]; call it after [`PhysicsWorld::step`] when
    /// stepping by hand.
    ///
    /// rapier (since 0.36) finds contacts at the start of a step, before it
    /// moves anything, as PhysX does for Unity: what is reported is where
    /// things touched before the step, not after it.
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
            let mut sensed: Vec<hecs::Entity> = Vec::new();
            for (a, b, touching) in self.narrow_phase.intersection_pairs_with(mine) {
                if touching {
                    sensed.extend(entity_of(other(a, b)));
                }
            }
            now.extend(sensed.iter().copied());
            for pair in self.narrow_phase.contact_pairs_with(mine) {
                if pair.has_any_active_contact() {
                    now.extend(entity_of(other(pair.collider1, pair.collider2)));
                }
            }
            now.retain(|e| *e != entity);
            now.sort();
            now.dedup();
            sensed.retain(|e| *e != entity);
            sensed.sort();
            sensed.dedup();
            contacts.entered = now.iter().filter(|e| !contacts.inside.contains(e)).copied().collect();
            contacts.left = contacts.inside.iter().filter(|e| !now.contains(e)).copied().collect();
            contacts.inside = now;
            contacts.triggers = sensed;
        }
        for (entity, handle, contacts) in world
            .query_mut::<(hecs::Entity, &BodyHandle, &mut Contacts)>()
            .into_iter()
        {
            let Some(body) = self.bodies.get(handle.0) else {
                continue;
            };
            let mut now: Vec<hecs::Entity> = Vec::new();
            let mut sensed: Vec<hecs::Entity> = Vec::new();
            let mut normals: Vec<Vec3> = Vec::new();
            for &mine in body.colliders() {
                let other = |a: ColliderHandle, b: ColliderHandle| if a == mine { b } else { a };
                for (a, b, touching) in self.narrow_phase.intersection_pairs_with(mine) {
                    if touching {
                        now.extend(entity_of(other(a, b)));
                        sensed.extend(entity_of(other(a, b)));
                    }
                }
                for pair in self.narrow_phase.contact_pairs_with(mine) {
                    if pair.has_any_active_contact() {
                        now.extend(entity_of(other(pair.collider1, pair.collider2)));
                        // A manifold's normal points from the pair's first
                        // collider to its second; ours is the other way from
                        // whatever we touch.
                        let sign = if pair.collider1 == mine { -1.0 } else { 1.0 };
                        for manifold in pair.manifolds() {
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
            sensed.retain(|e| *e != entity);
            sensed.sort();
            sensed.dedup();
            contacts.triggers = sensed;
        }
    }

    /// One fixed step of physics as a system: bring rapier in line with the
    /// world, step, and write where the dynamic bodies went back. Call it
    /// once per simulation step.
    pub fn run(&mut self, world: &mut World) {
        self.sync_from_world(world);
        self.blow(world);
        self.step();
        self.free_hinged(world);
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
            let down = Ray::new(rv(centre), Vector::NEG_Y);
            let others = QueryFilter::default()
                .exclude_rigid_body(handle.0)
                .exclude_sensors();
            let grounded = self
                .queries(others)
                .cast_ray(&down, half + 0.08, true)
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
            body.apply_impulse(Vector::new(push.x * dt, hop, push.z * dt), true);
            body.apply_torque_impulse(Vector::new(roll.x * dt, roll.y * dt, roll.z * dt), true);
        }
    }

    /// How far a hinge has turned about its axis, in degrees, the way its
    /// `limits_deg` count: 0 where it was built, signed. What a lever or a
    /// door reads to know whether it is thrown. `None` for an entity with
    /// no joint built.
    pub fn hinge_angle(&self, world: &World, entity: hecs::Entity) -> Option<f32> {
        let built = world.get::<&JointBuilt>(entity).ok()?;
        let joint = self.impulse_joints.get(built.handle)?;
        let one = self.bodies.get(joint.body1())?;
        let two = self.bodies.get(joint.body2())?;
        let first = one.position() * joint.data.local_frame1;
        let second = two.position() * joint.data.local_frame2;
        // A revolute joint turns about its frames' x.
        let q = first.rotation.inverse() * second.rotation;
        let (x, w) = if q.w < 0.0 { (-q.x, -q.w) } else { (q.x, q.w) };
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
                if let (Some(one), Some(two)) = (self.bodies.get(joint.body1()), self.bodies.get(joint.body2())) {
                    let a = one.position() * joint.data.local_frame1;
                    let b = two.position() * joint.data.local_frame2;
                    let apart = (a.translation - b.translation).length();
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
        self.pipeline.step(
            rv(self.gravity),
            &self.parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            &mut self.ccd,
            &Ignoring(&self.ignored),
            &(),
        );
    }

    /// A thing just hinged or slid on the world — a door, a handle — stops
    /// colliding with the still things it was put into: a door set flush
    /// in the sand would otherwise grind against it, and a stiff solver
    /// hold it shut by that friction. Still is fixed or kinematic: a padlock
    /// the scene set into a door's leaf would push the leaf past its stop,
    /// to tremble there awake for good. What it only comes to touch later
    /// (a wall it swings into, the lock's bar it rests on) it still meets.
    /// Once, after the first step its contacts are known.
    fn free_hinged(&mut self, world: &mut World) {
        let fresh: Vec<(hecs::Entity, RigidBodyHandle)> = world
            .query::<(hecs::Entity, &JointBuilt)>()
            .iter()
            .filter(|(_, b)| b.fresh)
            .map(|(e, b)| (e, b.bodies.1))
            .collect();
        for (entity, body) in fresh {
            if let Ok(mut built) = world.get::<&mut JointBuilt>(entity) {
                built.fresh = false;
            }
            let Some(rigid) = self.bodies.get(body) else { continue };
            for &collider in rigid.colliders() {
                for pair in self.narrow_phase.contact_pairs_with(collider) {
                    let other = if pair.collider1 == collider { pair.collider2 } else { pair.collider1 };
                    let still = self
                        .colliders
                        .get(other)
                        .and_then(|c| c.parent())
                        .and_then(|h| self.bodies.get(h))
                        .is_none_or(|b| b.is_fixed() || b.is_kinematic());
                    let deep = pair
                        .manifolds()
                        .iter()
                        .flat_map(|m| m.points.iter())
                        .any(|p| p.dist < -0.01);
                    if still && deep {
                        let bits = |h: ColliderHandle| self.colliders.get(h).map_or(0, |c| c.user_data as u64);
                        let (a, b) = (bits(collider), bits(other));
                        if a != 0 && b != 0 {
                            self.ignored.insert((a.min(b), a.max(b)));
                        }
                    }
                }
            }
        }
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
        let mut moved: Vec<(hecs::Entity, glam::Mat4, glam::Mat4, Option<hecs::Entity>, crate::scene::Drawn)> = Vec::new();
        // What the game moves (a kinematic body, a train, a lift): where it
        // is now, drawn between there and where the last step left it.
        let mut carried: Vec<(hecs::Entity, glam::Mat4, glam::Mat4, crate::scene::Drawn)> = Vec::new();
        let mut unstepped: Vec<hecs::Entity> = Vec::new();
        for (entity, handle, physics, placed, parent, replica, props, stepped) in world
            .query::<(
                hecs::Entity,
                &BodyHandle,
                &Physics,
                &WorldTransform,
                Option<&Parent>,
                Option<&crate::world::Replica>,
                Option<&Props>,
                Option<&crate::world::Stepped>,
            )>()
            .iter()
        {
            let drawn = props.map(|p| p.0.drawn).unwrap_or_default();
            let kind = solved(physics.0, replica.is_some());
            // A replica is shown by the network, between the poses its
            // owner sent, every frame.
            if kind == Body::Kinematic && replica.is_none() && drawn != crate::scene::Drawn::AtStep {
                carried.push((entity, stepped.map_or(placed.0, |s| s.to), placed.0, drawn));
                continue;
            }
            if kind != Body::Dynamic {
                if stepped.is_some() {
                    unstepped.push(entity);
                }
                continue;
            }
            let Some(body) = self.bodies.get(handle.0) else {
                continue;
            };
            let position = body.position();
            let translation = gv(position.translation);
            let rotation = gq(position.rotation);
            // Rapier knows where a body is and how it is turned; it does not
            // know how big the thing being drawn is, because scale lives in
            // the collider's shape rather than the body. Keeping the
            // entity's own scale is the difference between a crate falling
            // and a crate falling while shrinking to a unit cube.
            let (scale, _, _) = placed.0.to_scale_rotation_translation();
            moved.push((
                entity,
                placed.0,
                glam::Mat4::from_scale_rotation_translation(scale, rotation, translation),
                parent.map(|p| p.0),
                drawn,
            ));
        }
        for entity in unstepped {
            let _ = world.remove_one::<crate::world::Stepped>(entity);
        }
        for (entity, from, to, drawn) in carried {
            let _ = world.insert_one(entity, stepped(from, to, drawn));
        }
        for (entity, was, matrix, parent, drawn) in moved {
            // The local transform too, relative to the parent: it is what
            // the hierarchy is recomputed from, and what a reload compares
            // with the file. Writing only the world one would let the next
            // hierarchy pass put the body back where the scene had it.
            //
            // Only its place and turn are the body's: its own scale stays
            // what it was, as Unity keeps a Rigidbody's `localScale`. Under
            // a parent scaled unevenly and turned, the world matrix is
            // sheared, and reading a scale back out of it and into the
            // local one grew the thing a little every step — a mouse's jaw
            // on a hinge, under its model's 275×325×282 bones, a metre
            // wider each second.
            let parent_world = parent
                .and_then(|p| world.get::<&WorldTransform>(p).ok().map(|w| w.0))
                .unwrap_or(glam::Mat4::IDENTITY);
            // What the hierarchy puts between them (a bone it rides on).
            let between = world.get::<&crate::world::Between>(entity).ok().map(|b| b.0);
            let parent_matrix = parent_world * between.unwrap_or(glam::Mat4::IDENTITY);
            let (parent_scale, parent_rotation, _) = parent_matrix.to_scale_rotation_translation();
            let uneven = parent.is_some()
                && parent_scale.abs().max_element() > parent_scale.abs().min_element() * 1.001;
            let kept = world.get::<&Transform>(entity).ok().map(|t| t.scale);
            let (local, matrix) = match kept.filter(|_| uneven) {
                // Its place and turn under the parent; its scale its own.
                Some(scale) => {
                    let (_, rotation, translation) = matrix.to_scale_rotation_translation();
                    let mut local = Transform {
                        position: parent_matrix.inverse().transform_point3(translation),
                        scale,
                        ..Transform::default()
                    };
                    local.set_rotation((parent_rotation.inverse() * rotation).normalize());
                    // What the hierarchy makes of it: the parent's shear and all.
                    (local, parent_matrix * local.matrix())
                }
                // An evenly scaled parent — mirrored too — shears nothing:
                // the local transform is the world one undone by it.
                None => {
                    let (scale, rotation, translation) = (parent_world.inverse() * matrix).to_scale_rotation_translation();
                    let mut local = Transform {
                        position: translation,
                        scale,
                        ..Transform::default()
                    };
                    local.set_rotation(rotation);
                    (local, matrix)
                }
            };
            // Where the step took it from and to: a frame between steps
            // draws it between them (`world::interpolate`). Written in place:
            // after the first step a body has all of these, and an insert
            // would look up the archetype to move it to on every one.
            let stepped = stepped(was, matrix, drawn);
            let Ok((placed, own, step, built)) = world.query_one_mut::<(
                &mut WorldTransform,
                Option<&mut Transform>,
                Option<&mut crate::world::Stepped>,
                Option<&mut Built>,
            )>(entity) else {
                continue;
            };
            placed.0 = matrix;
            if let Some(built) = built {
                built.local = local;
                built.placed = matrix;
            }
            let own = own.map(|own| *own = local).is_some();
            match step {
                Some(step) => *step = stepped,
                None => {
                    let _ = world.insert_one(entity, stepped);
                }
            }
            if !own {
                let _ = world.insert_one(entity, local);
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
        let ray = Ray::new(rv(from), rv(direction));
        // A trigger is a zone, not a surface: a ray passes through it.
        let (collider, distance) = self.queries(QueryFilter::default().exclude_sensors()).cast_ray(
            &ray,
            max_distance,
            // Solid: a ray starting inside a shape stops at zero rather than
            // passing through to the far wall. A camera inside a rock should
            // report the rock.
            true,
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
        let ray = Ray::new(rv(from), rv(direction));
        let filter = if statics_only {
            QueryFilter::only_fixed()
        } else {
            QueryFilter::default()
        }
        .exclude_sensors();
        let (_, hit) = self.queries(filter).cast_ray_and_get_normal(&ray, max_distance, true)?;
        Some((
            from + direction * hit.time_of_impact,
            gv(hit.normal),
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
        let ray = Ray::new(rv(from), rv(direction));
        let only = InteractionGroups::new(
            Group::ALL,
            Group::from_bits_truncate(self.layers.mask(layers)),
            InteractionTestMode::And,
        );
        let (collider, distance) = self
            .queries(QueryFilter::default().exclude_sensors().groups(only))
            .cast_ray(&ray, max_distance, true)?;
        Some(RayHit {
            point: from + direction * distance,
            distance,
            collider: ColliderRef(collider),
            entity: self.entity_of(collider),
        })
    }

    /// [`Self::sphere_cast`] meeting only the named layers: a camera kept
    /// out of walls and ground but not its own player (Cinemachine's
    /// Collider, its Collide Against mask).
    pub fn sphere_cast_among(
        &self,
        from: Vec3,
        radius: f32,
        direction: Vec3,
        max_distance: f32,
        layers: &[&str],
    ) -> Option<RayHit> {
        let direction = direction.normalize_or_zero();
        if direction.length_squared() < 0.5 {
            return None;
        }
        let only = InteractionGroups::new(
            Group::ALL,
            Group::from_bits_truncate(self.layers.mask(layers)),
            InteractionTestMode::And,
        );
        let ball = Ball::new(radius.max(1e-4));
        let at = Pose::translation(from.x, from.y, from.z);
        let (collider, hit) = self.queries(QueryFilter::default().exclude_sensors().groups(only)).cast_shape(
            &at,
            rv(direction),
            &ball,
            rapier3d::parry::query::ShapeCastOptions {
                max_time_of_impact: max_distance,
                stop_at_penetration: true,
                ..Default::default()
            },
        )?;
        Some(RayHit {
            point: from + direction * hit.time_of_impact,
            distance: hit.time_of_impact,
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
        let at = Pose::translation(centre.x, centre.y, centre.z);
        let queries = self.queries(QueryFilter::default().exclude_sensors());
        let mut found: Vec<hecs::Entity> = queries
            .intersect_shape(at, &ball)
            .filter_map(|(collider, _)| self.entity_of(collider))
            .collect();
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
        let ray = Ray::new(rv(from), rv(direction));
        let queries = self.queries(QueryFilter::default().exclude_sensors());
        let mut hits: Vec<RayHit> = queries
            .intersect_ray(ray, max_distance, true)
            .map(|(collider, _, hit)| RayHit {
                point: from + direction * hit.time_of_impact,
                distance: hit.time_of_impact,
                collider: ColliderRef(collider),
                entity: self.entity_of(collider),
            })
            .collect();
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        hits
    }

    /// Every entity whose shape overlaps a box — Unity's `OverlapBox`: what
    /// is in a doorway, on a pressure plate's area, inside a room. `half`
    /// is half its size on each axis, turned by `rotation`. Triggers are
    /// left out; sorted.
    pub fn overlap_box(&self, centre: Vec3, half: Vec3, rotation: glam::Quat) -> Vec<hecs::Entity> {
        let cuboid = Cuboid::new(rv(half.max(Vec3::ZERO)));
        let at = Pose::from_parts(rv(centre), rq(rotation.normalize()));
        let queries = self.queries(QueryFilter::default().exclude_sensors());
        let mut found: Vec<hecs::Entity> = queries
            .intersect_shape(at, &cuboid)
            .filter_map(|(collider, _)| self.entity_of(collider))
            .collect();
        found.sort();
        found.dedup();
        found
    }

    /// The rapier body behind an entity, once it has been built.
    fn body_of(&self, world: &World, entity: hecs::Entity) -> Option<RigidBodyHandle> {
        world.get::<&BodyHandle>(entity).ok().map(|h| h.0)
    }

    /// Whether an entity's body is asleep — still long enough that the
    /// solver leaves it until something touches it: Unity's
    /// `Rigidbody.IsSleeping`. `None` for an entity with no body.
    pub fn asleep(&self, world: &World, entity: hecs::Entity) -> Option<bool> {
        let handle = world.get::<&BodyHandle>(entity).ok()?.0;
        Some(self.bodies.get(handle)?.is_sleeping())
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
        self.wake(world, entity);
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        body.set_linvel(Vector::new(velocity.x, velocity.y, velocity.z), true);
        true
    }

    /// Set how fast a body turns, radians a second about each axis: a
    /// thrown plank's tumble. Unity's `angularVelocity`.
    pub fn set_spin(&mut self, world: &World, entity: hecs::Entity, spin: Vec3) -> bool {
        self.wake(world, entity);
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        body.set_angvel(Vector::new(spin.x, spin.y, spin.z), true);
        true
    }

    /// Turn a body where it stands, at once, keeping how it moves — as
    /// Unity does when a script sets a Rigidbody's `transform.rotation`
    /// (a billboard on a loose crate). Its transform follows.
    pub fn turn(&mut self, world: &mut World, entity: hecs::Entity, rotation: Quat) -> bool {
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        let mut pose = *body.position();
        pose.rotation = rq(rotation.normalize());
        body.set_position(pose, true);
        // The transform is its parent's: the turn as the parent sees it.
        let parent = world
            .get::<&Parent>(entity)
            .ok()
            .and_then(|p| world.get::<&WorldTransform>(p.0).ok().map(|t| t.0));
        let local = match parent {
            Some(m) => {
                let (_, turn, _) = m.to_scale_rotation_translation();
                turn.inverse() * rotation
            }
            None => rotation,
        };
        if let Ok(mut transform) = world.get::<&mut Transform>(entity) {
            transform.set_rotation(local);
        }
        if let Ok(mut built) = world.get::<&mut Built>(entity) {
            built.local.set_rotation(local);
        }
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
        let pose = Pose::from_parts(rv(position), rq(rotation.normalize()));
        body.set_position(pose, true);
        body.set_linvel(Vector::new(0.0, 0.0, 0.0), true);
        body.set_angvel(Vector::new(0.0, 0.0, 0.0), true);
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
        self.wake(world, entity);
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        body.apply_impulse(Vector::new(impulse.x, impulse.y, impulse.z), true);
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
        self.wake(world, entity);
        let dt = self.parameters.dt;
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        let impulse = force * dt;
        body.apply_impulse_at_point(Vector::new(impulse.x, impulse.y, impulse.z), Vector::new(at.x, at.y, at.z), true);
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
        self.wake(world, entity);
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        body.apply_impulse_at_point(Vector::new(impulse.x, impulse.y, impulse.z), Vector::new(at.x, at.y, at.z), true);
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
        self.wake(world, entity);
        let dt = self.parameters.dt;
        let Some(body) = self
            .body_of(world, entity)
            .and_then(|h| self.bodies.get_mut(h))
        else {
            return false;
        };
        let t = torque * dt;
        body.apply_torque_impulse(Vector::new(t.x, t.y, t.z), true);
        true
    }

    /// How hard it is to turn about each of its own axes, and those axes:
    /// its principal inertia, kilogram-metres².
    pub fn inertia(&self, world: &World, entity: hecs::Entity) -> Option<Vec3> {
        let body = self.bodies.get(self.body_of(world, entity)?)?;
        let i = body.mass_properties().local_mprops.principal_inertia();
        Some(Vec3::new(i.x, i.y, i.z))
    }

    /// Its inverse inertia in the world's axes, as it is turned now: how
    /// much a torque about each axis spins it. Zero rows about an axis it
    /// cannot turn about. What a hand's spring budget reads to know how light
    /// a thing held off its centre feels (Dacha's `DragMath.InverseInertia`).
    pub fn world_inverse_inertia(&self, world: &World, entity: hecs::Entity) -> Option<glam::Mat3> {
        let body = self.bodies.get(self.body_of(world, entity)?)?;
        let props = body.mass_properties().local_mprops;
        let i = props.principal_inertia();
        let frame = props.principal_inertia_local_frame;
        let q = gq(*body.rotation() * frame);
        let basis = glam::Mat3::from_quat(q);
        let inverse = |v: f32| if v > 1e-9 { 1.0 / v } else { 0.0 };
        let diag = glam::Mat3::from_diagonal(glam::Vec3::new(inverse(i.x), inverse(i.y), inverse(i.z)));
        Some(basis * diag * basis.transpose())
    }

    /// Where its centre of mass is, in the world.
    pub fn center_of_mass(&self, world: &World, entity: hecs::Entity) -> Option<Vec3> {
        let body = self.bodies.get(self.body_of(world, entity)?)?;
        let c = body.center_of_mass();
        Some(Vec3::new(c.x, c.y, c.z))
    }

    /// Its mass, kilograms.
    pub fn mass(&self, world: &World, entity: hecs::Entity) -> Option<f32> {
        Some(self.bodies.get(self.body_of(world, entity)?)?.mass())
    }

    /// What a body carries besides its colliders' own weight — the water
    /// in a bucket: `mass` kilograms at `centre`, a point in the body's own
    /// space (its position and turn, not its scale). Its mass and centre
    /// of mass are then its colliders' and this together, as Unity's are
    /// after writing `Rigidbody.mass` and `centerOfMass`; zero takes it
    /// off. Replaces what was carried before, and wakes the body when it
    /// changes. False when the entity has no body yet.
    pub fn set_carried_mass(&mut self, world: &World, entity: hecs::Entity, mass: f32, centre: Vec3) -> bool {
        let Some(handle) = self.body_of(world, entity) else { return false };
        let Some(body) = self.bodies.get_mut(handle) else { return false };
        let props = MassProperties::new(Vector::new(centre.x, centre.y, centre.z), mass.max(0.0), Vector::new(0.0, 0.0, 0.0));
        body.set_additional_mass_properties(props, true);
        true
    }

    /// How fast a point of the body is going, in the world: its own speed
    /// and its turning together.
    pub fn velocity_at(&self, world: &World, entity: hecs::Entity, at: Vec3) -> Option<Vec3> {
        let body = self.bodies.get(self.body_of(world, entity)?)?;
        let v = body.velocity_at_point(rv(at));
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
        let at = Pose::translation(from.x, from.y, from.z);
        let (collider, hit) = self.queries(QueryFilter::default().exclude_sensors()).cast_shape(
            &at,
            rv(direction),
            &ball,
            rapier3d::parry::query::ShapeCastOptions {
                max_time_of_impact: max_distance,
                stop_at_penetration: true,
                ..Default::default()
            },
        )?;
        Some(RayHit {
            point: from + direction * hit.time_of_impact,
            distance: hit.time_of_impact,
            collider: ColliderRef(collider),
            entity: self.entity_of(collider),
        })
    }

    /// A standing person — feet at `feet`, `radius` round and `height`
    /// tall — moved by `by` among what is solid. The body is a capsule
    /// floating a `step` above the feet: it slides along walls and never
    /// catches on an edge low enough to step on. The feet are put on the
    /// ground a ray finds under it, up or down no more than a `step` (or
    /// how far `by` falls); ground steeper than `slope` radians is a wall.
    /// Where the feet end up, and whether they stand on something. A
    /// query, not a body: nothing in the world moves, and the numbers — how
    /// a person walks — stay the caller's. Triggers and moving bodies are
    /// looked through.
    #[allow(clippy::too_many_arguments)]
    pub fn walk_capsule(
        &self,
        feet: Vec3,
        by: Vec3,
        radius: f32,
        height: f32,
        step: f32,
        slope: f32,
        dt: f32,
    ) -> (Vec3, bool) {
        use rapier3d::control::{CharacterLength, KinematicCharacterController};
        let radius = radius.max(0.01);
        let step = step.clamp(0.0, (height - 2.0 * radius).max(0.0));
        let half = ((height - step) * 0.5 - radius).max(0.0);
        let capsule = Capsule::new_y(half, radius);
        let lift = step + half + radius;
        let controller = KinematicCharacterController {
            offset: CharacterLength::Absolute(0.01),
            snap_to_ground: None,
            // The capsule meets walls only: the ground is the ray's.
            max_slope_climb_angle: 0.0,
            ..Default::default()
        };
        let queries = self.queries(QueryFilter::only_fixed().exclude_sensors());
        let centre = feet + Vec3::Y * lift;
        let along = Vec3::new(by.x, 0.0, by.z);
        let moved = controller.move_shape(
            dt.max(1e-4),
            &queries,
            &capsule,
            &Pose::translation(centre.x, centre.y, centre.z),
            rv(along),
            |_| {},
        );
        let mut to = feet + Vec3::new(moved.translation.x, 0.0, moved.translation.z);
        // The ground under the new place: no higher than a step up, no
        // lower than a step down or the fall.
        let below = step + (-by.y).max(0.0);
        let ground = self.cast_ray_with_normal(to + Vec3::Y * step, -Vec3::Y, step + below, true);
        match ground {
            Some((hit, normal, _)) if normal.y >= slope.cos() - 1e-4 || hit.y <= feet.y + 1e-3 => {
                to.y = hit.y;
                (to, true)
            }
            Some(_) => {
                // Too steep to walk up: a wall after all.
                (Vec3::new(feet.x, feet.y, feet.z), true)
            }
            None => {
                to.y = feet.y + by.y.min(0.0);
                (to, false)
            }
        }
    }

    /// Move a character's body by `desired` (metres) the way Unity's
    /// `CharacterController.Move` does: sliding along walls, up steps no
    /// higher than `step`, up slopes no steeper than `slope_deg`, and kept
    /// on the ground going down them. The body is the entity's own
    /// (kinematic, its first collider the character's shape); what it
    /// touches is the rest of the scene, triggers aside. Returns how far it
    /// can go — for the game to add to its transform — and whether it
    /// stands on ground after. A collision primitive, not a controller:
    /// how fast, when to jump and how to fall stay the game's (player.md).
    pub fn move_character(
        &self,
        world: &World,
        entity: hecs::Entity,
        desired: Vec3,
        step: f32,
        slope_deg: f32,
    ) -> Option<(Vec3, bool)> {
        use rapier3d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
        let handle = self.body_of(world, entity)?;
        let body = self.bodies.get(handle)?;
        let collider = self.colliders.get(*body.colliders().first()?)?;
        let controller = KinematicCharacterController {
            offset: CharacterLength::Absolute(0.02),
            slide: true,
            autostep: (step > 0.0).then_some(CharacterAutostep {
                max_height: CharacterLength::Absolute(step),
                min_width: CharacterLength::Absolute(0.1),
                include_dynamic_bodies: false,
            }),
            max_slope_climb_angle: slope_deg.to_radians(),
            min_slope_slide_angle: slope_deg.to_radians(),
            // Kept on the ground going down a slope — never pulled down
            // out of a jump.
            snap_to_ground: (desired.y <= 0.0).then_some(CharacterLength::Absolute(step.max(0.1))),
            ..Default::default()
        };
        let filter = QueryFilter::default().exclude_sensors().exclude_rigid_body(handle).groups(collider.collision_groups());
        // Where the entity is now, not where the last step left the
        // shape: a kinematic body follows its transform a step behind.
        let placed = world
            .get::<&WorldTransform>(entity)
            .ok()
            .map(|w| {
                let (_, r, t) = w.0.to_scale_rotation_translation();
                Pose::from_parts(rv(t), rapier3d::math::Rotation::from_xyzw(r.x, r.y, r.z, r.w))
            })
            .unwrap_or(*body.position());
        let at = match collider.position_wrt_parent() {
            Some(local) => placed * *local,
            None => *collider.position(),
        };
        let moved = controller.move_shape(
            self.parameters.dt,
            &self.queries(filter),
            collider.shape(),
            &at,
            rv(desired),
            |_| {},
        );
        let t = moved.translation;
        // Rapier misses ground under a mesh floor's seams: a short cast of
        // the shape down from where it ends, onto something not too steep,
        // is ground too — as a CharacterController's skin touching it is.
        // Going up is never landing (a CharacterController's flags say
        // Below only for a move down): a jump beside a slope leaves it.
        let mut grounded = moved.grounded && desired.y <= 0.0;
        if !grounded && desired.y <= 0.0 {
            let mut ended = at;
            ended.translation += t;
            let probe = self.queries(filter).cast_shape(
                &ended,
                rv(Vec3::NEG_Y),
                collider.shape(),
                rapier3d::parry::query::ShapeCastOptions {
                    max_time_of_impact: 0.06,
                    stop_at_penetration: true,
                    ..Default::default()
                },
            );
            grounded = probe.is_some_and(|(_, hit)| {
                let n = gv(hit.normal1);
                n.y >= slope_deg.to_radians().cos() - 1e-3 || gv(hit.normal2).y <= -(slope_deg.to_radians().cos() - 1e-3)
            });
        }
        Some((Vec3::new(t.x, t.y, t.z), grounded))
    }

    /// Bring ray queries up to date with the bodies, without a step: after
    /// [`PhysicsWorld::sync_from_world`], before asking where things are.
    pub fn refresh_queries(&mut self) {
        // Queries walk the broad phase's tree, which a step keeps; without
        // one, each collider's box is put there by hand.
        for (handle, collider) in self.colliders.iter() {
            let aabb = collider.compute_broad_phase_aabb(&self.parameters, &self.bodies);
            self.broad_phase.set_aabb(&self.parameters, handle, aabb);
        }
    }

    /// The scene's queries, through `filter`.
    fn queries<'a>(&'a self, filter: QueryFilter<'a>) -> QueryPipeline<'a> {
        self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            filter,
        )
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
        InteractionTestMode::And,
    )
}

/// Whether two joints differ at most in what drives them — a motor, a
/// spring's strength, limits — and are the same joint otherwise.
fn same_but_drive(a: &crate::scene::Joint, b: &crate::scene::Joint) -> bool {
    use crate::scene::Joint;
    match (*a, *b) {
        (
            Joint::Hinge {
                to, anchor, axis, connected, ..
            },
            Joint::Hinge {
                to: t,
                anchor: n,
                axis: x,
                connected: c,
                ..
            },
        ) => to == t && anchor == n && axis == x && connected == c,
        (Joint::Slider { to, axis, .. }, Joint::Slider { to: t, axis: x, .. }) => {
            to == t && axis == x
        }
        (
            Joint::Spring { to, anchor, connected, .. },
            Joint::Spring {
                to: t, anchor: n, connected: c, ..
            },
        ) => to == t && anchor == n && connected == c,
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
    one: &Pose,
    two: &Pose,
    one_scale: Vec3,
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
    let frame = Pose::from_parts(rv(at), rq(turn.normalize()));
    let locked = match joint {
        Joint::Fixed { .. } => JointAxesMask::LOCKED_FIXED_AXES,
        Joint::Hinge { .. } => JointAxesMask::LOCKED_REVOLUTE_AXES,
        Joint::Ball { .. } => JointAxesMask::LOCKED_SPHERICAL_AXES,
        Joint::Slider { .. } => JointAxesMask::LOCKED_PRISMATIC_AXES,
        // Nothing locked: the spring's motors do the holding.
        Joint::Spring { .. } => JointAxesMask::empty(),
        Joint::None => return None,
    };
    // The other body's end: where the anchor is now, or where the joint
    // says in that body's own space — Unity's connected anchor, which holds
    // a rope's links a set length apart however they were laid out.
    let connected = match *joint {
        Joint::Ball { connected, .. } | Joint::Hinge { connected, .. } | Joint::Spring { connected, .. } => connected,
        _ => None,
    };
    let mut frame1 = one.inverse() * frame;
    if let Some(c) = connected {
        let c = c * one_scale;
        frame1.translation = rv(c);
    }
    let mut builder = GenericJointBuilder::new(locked)
        .local_frame1(frame1)
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
        Joint::Ball {
            limits_deg: Some(limits),
            ..
        } => {
            for (axis, (low, high)) in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ].into_iter().zip(limits) {
                builder = builder.limits(axis, [low.to_radians(), high.to_radians()]);
            }
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
fn isometry(placed: glam::Mat4) -> Pose {
    let (_, rotation, translation) = placed.to_scale_rotation_translation();
    // A zero scale on an axis leaves no turn to read: none, rather than a
    // NaN that parry then panics on.
    let rotation = if rotation.is_finite() { rotation } else { glam::Quat::IDENTITY };
    Pose::from_parts(rv(translation), rq(rotation.normalize()))
}

/// The convex hull of `points`, as a collider that stands on the whole of
/// a broad face (see [`crate::shapes::Hull`]).
fn hull(points: &[Vector]) -> Option<Collider> {
    let polyhedron = rapier3d::parry::shape::ConvexPolyhedron::from_convex_hull(points)?;
    Some(ColliderBuilder::new(SharedShape::new(crate::shapes::Hull::new(polyhedron))).build())
}

/// What a zone of its own (`Body::Trigger`) is tested against: whatever
/// moves — dynamic or kinematic, a player carried by the game included —
/// and, only when its line asks, what stands still. As Unity's triggers: one
/// with no Rigidbody meets only colliders that have one, which keeps a
/// level-wide zone from being tested against every wall of the level each
/// step (Dacha's sandstorm box, a third of a level's physics).
fn zone_notices(still: bool) -> ActiveCollisionTypes {
    if still {
        ActiveCollisionTypes::all()
    } else {
        ActiveCollisionTypes::all() - ActiveCollisionTypes::KINEMATIC_FIXED - ActiveCollisionTypes::FIXED_FIXED
    }
}

/// `collider`'s shape, never swept between steps (see
/// [`crate::shapes::Unswept`]).
fn unsweep(collider: &mut Collider) {
    let shape = collider.shared_shape().clone();
    collider.set_shape(SharedShape::new(crate::shapes::Unswept(shape)));
}

/// Where [`build_collider`] puts a shape on its body, without building it:
/// what a part that moved on its body is moved by, which for a hull or a
/// mesh would otherwise be worked out anew every step.
fn shape_offset(shape: ColliderShape, transform: glam::Mat4) -> Pose {
    let (scale, _, _) = transform.to_scale_rotation_translation();
    match shape {
        ColliderShape::Box { center, .. }
        | ColliderShape::Sphere { center, .. }
        | ColliderShape::Capsule { center, .. } => {
            let c = center * scale;
            Pose::translation(c.x, c.y, c.z)
        }
        ColliderShape::None
        | ColliderShape::Model
        | ColliderShape::Cylinder { .. }
        | ColliderShape::Ramp { .. }
        | ColliderShape::Stairs { .. } => Pose::IDENTITY,
    }
}

/// A `Model` collider's shape at a scale: its hull for a body the solver
/// moves, its triangles for one that stands still. Building the triangles'
/// tree is most of a level's first step, so what is built is kept
/// ([`MeshShapes`]) and several are built at once.
fn model_shape(mesh: &CollisionMesh, scale: Vec3, dynamic: bool) -> Option<SharedShape> {
    let points: Vec<Vector> = mesh
        .vertices
        .iter()
        .map(|v| {
            let v = *v * scale;
            Vector::new(v.x, v.y, v.z)
        })
        .collect();
    if dynamic {
        let polyhedron = rapier3d::parry::shape::ConvexPolyhedron::from_convex_hull(&points)?;
        Some(SharedShape::new(crate::shapes::Hull::new(polyhedron)))
    } else {
        // Triangles with no area — a mesh squashed flat by a zero
        // scale, a sliver — are nothing to stand on, and a query
        // against a tree of only those panics in parry. A mirroring
        // scale turns every triangle inside out: turned back, so
        // each still faces the side it was drawn to face.
        let mirrored = scale.x * scale.y * scale.z < 0.0;
        let triangles: Vec<[u32; 3]> = mesh
            .triangles
            .iter()
            .map(|&[a, b, c]| if mirrored { [a, c, b] } else { [a, b, c] })
            .filter(|t| {
                let [a, b, c] = t.map(|i| points.get(i as usize).copied());
                let (Some(a), Some(b), Some(c)) = (a, b, c) else { return false };
                (b - a).cross(c - a).length() > 1e-10
            })
            .collect();
        if triangles.is_empty() || points.iter().any(|p| !p.is_finite()) {
            return None;
        }
        // Ground as PhysX has it. Its internal edges fixed: a body
        // sliding over the seam between two triangles is pushed by
        // the surface the two make, not bumped up by the edge of the
        // one ahead. And one-sided — each triangle solid on the side
        // it faces — so a body swept between steps is stopped by
        // the ground it would pass through, not by the next rise of
        // the ground it slides over (it ends the step in front of
        // that, and rapier's sweep lets it go on).
        SharedShape::trimesh_with_flags(
            points,
            triangles,
            TriMeshFlags::FIX_INTERNAL_EDGES | TriMeshFlags::ORIENTED,
        )
        .ok()
    }
}

/// Which [`model_shape`] a collider wants: one mesh at one scale, moving or
/// still.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MeshShapeKey {
    mesh: usize,
    scale: [u32; 3],
    dynamic: bool,
}

impl MeshShapeKey {
    fn of(mesh: &CollisionMesh, scale: Vec3, dynamic: bool) -> Self {
        MeshShapeKey { mesh: mesh.key(), scale: scale.to_array().map(f32::to_bits), dynamic }
    }
}

/// A shape built from a mesh, with the mesh it was built from: held, so the
/// key's address stays that mesh's for as long as the shape is kept.
struct MadeShape {
    mesh: CollisionMesh,
    shape: SharedShape,
}

/// Shapes built from meshes, shared by every collider of the same mesh at
/// the same scale — a level's twenty-eight lengths of rail are one.
type MeshShapes = scrap_core::hash::FastMap<MeshShapeKey, MadeShape>;

/// Turn a scene's shape into a rapier collider, scaled by the transform.
fn build_collider(
    shape: ColliderShape,
    transform: glam::Mat4,
    mesh: Option<&CollisionMesh>,
    dynamic: bool,
    made: &MeshShapes,
) -> Option<Collider> {
    let (scale, _, _) = transform.to_scale_rotation_translation();
    Some(match shape {
        ColliderShape::None => return None,
        ColliderShape::Model => {
            // No geometry yet — the model is not imported, or nobody
            // attached it: no collider, and the next sync tries again.
            let mesh = mesh?;
            let shape = match made.get(&MeshShapeKey::of(mesh, scale, dynamic)) {
                Some(made) => made.shape.clone(),
                None => model_shape(mesh, scale, dynamic)?,
            };
            ColliderBuilder::new(shape).build()
        }
        ColliderShape::Box { half, center } => {
            let (h, c) = (half * scale, center * scale);
            ColliderBuilder::cuboid(h.x.max(1e-4), h.y.max(1e-4), h.z.max(1e-4))
                .translation(Vector::new(c.x, c.y, c.z))
                .build()
        }
        ColliderShape::Sphere { radius, center } => {
            // One radius, so a sphere scaled unevenly takes the largest —
            // a ball that is not a ball is an ellipsoid, and rapier's ball
            // cannot be one.
            let r = radius * scale.max_element();
            let c = center * scale;
            ColliderBuilder::ball(r.max(1e-4)).translation(Vector::new(c.x, c.y, c.z)).build()
        }
        ColliderShape::Capsule {
            half_height,
            radius,
            center,
            axis,
        } => {
            let c = center * scale;
            // Its length along its axis, its girth across it.
            let (along, across) = match axis {
                0 => (scale.x, scale.y.max(scale.z)),
                2 => (scale.z, scale.x.max(scale.y)),
                _ => (scale.y, scale.x.max(scale.z)),
            };
            let (h, r) = ((half_height * along).max(1e-4), (radius * across).max(1e-4));
            match axis {
                0 => ColliderBuilder::capsule_x(h, r),
                2 => ColliderBuilder::capsule_z(h, r),
                _ => ColliderBuilder::capsule_y(h, r),
            }
            .translation(Vector::new(c.x, c.y, c.z))
            .build()
        }
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
            let points: Vec<Vector> = [
                (-1.0, -1.0, -1.0),
                (1.0, -1.0, -1.0),
                (1.0, -1.0, 1.0),
                (-1.0, -1.0, 1.0),
                (-1.0, 1.0, -1.0),
                (1.0, 1.0, -1.0),
            ]
            .into_iter()
            .map(|(x, y, z)| Vector::new(x * h.x, y * h.y, z * h.z))
            .collect();
            hull(&points)?
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
                        Pose::translation(0.0, y, z),
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

    #[test]
    fn a_walking_capsule_climbs_steps_stops_at_walls_and_steps_off_edges() {
        // A floor, three 17 cm steps up to a ledge along +z, a wall along +x,
        // and a drop at the ledge's far end.
        let text = r#"(entities: [
            (id: "0000000000000001", name: "floor", body: Static, collider: Box(half: (20.0, 0.5, 20.0)),
             transform: (position: (0.0, -0.5, 0.0))),
            (id: "0000000000000002", name: "one", body: Static, collider: Box(half: (1.0, 0.085, 0.15)),
             transform: (position: (0.0, 0.085, 2.15))),
            (id: "0000000000000003", name: "two", body: Static, collider: Box(half: (1.0, 0.17, 0.15)),
             transform: (position: (0.0, 0.17, 2.45))),
            (id: "0000000000000004", name: "ledge", body: Static, collider: Box(half: (1.0, 0.255, 1.0)),
             transform: (position: (0.0, 0.255, 3.6))),
            (id: "0000000000000005", name: "wall", body: Static, collider: Box(half: (0.25, 1.0, 2.0)),
             transform: (position: (4.25, 1.0, 0.0))),
        ])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        let mut world = World::new();
        spawn(&scene, &mut world);
        crate::world::apply_hierarchy(&mut world);
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        physics.sync_from_world(&mut world);
        physics.refresh_queries();
        let walk = |mut feet: Vec3, velocity: Vec3, seconds: f32| {
            let dt = 1.0 / 60.0;
            let mut fall = 0.0;
            for _ in 0..(seconds / dt) as usize {
                let (to, grounded) = physics.walk_capsule(feet, velocity * dt - Vec3::Y * fall * dt, 0.3, 1.75, 0.35, 0.87, dt);
                fall = if grounded { 0.0 } else { fall + 9.81 * dt };
                feet = to;
            }
            feet
        };
        let up = walk(Vec3::ZERO, Vec3::Z * 1.2, 3.0);
        assert!((up.y - 0.51).abs() < 1e-3 && up.z > 3.0, "up the steps onto the ledge: {up}");
        let off = walk(up, Vec3::Z * 1.2, 2.0);
        assert!(off.y.abs() < 1e-3 && off.z > 4.8, "off its far edge, down to the floor: {off}");
        let wall = walk(Vec3::ZERO, Vec3::X * 1.2, 5.0);
        assert!((wall.x - (4.0 - 0.3)).abs() < 0.03, "stopped by the wall: {wall}");
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

    /// A bucket of 1 kg carrying 3 kg of water low down: 4 kg, its centre
    /// pulled toward the water's; carrying nothing again is the bucket.
    #[test]
    fn a_body_weighs_what_it_carries() {
        let text = r#"(entities: [
            (id: "0000000000000001", name: "bucket", body: Dynamic, physics: (mass: Some(1.0), gravity: 0.0),
             collider: Box(half: (0.2, 0.2, 0.2)), transform: (position: (0.0, 1.0, 0.0))),
        ])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        let mut world = World::new();
        spawn(&scene, &mut world);
        crate::world::apply_hierarchy(&mut world);
        let bucket = world.query::<(hecs::Entity, &crate::world::SceneId)>().iter().next().map(|(e, _)| e).unwrap();
        let mut physics = PhysicsWorld::new(1.0 / 30.0);
        assert!(!physics.set_carried_mass(&world, bucket, 3.0, Vec3::ZERO), "no body before the first step");
        physics.run(&mut world);
        assert!(physics.set_carried_mass(&world, bucket, 3.0, Vec3::new(0.0, -0.1, 0.0)));
        physics.run(&mut world);
        assert!((physics.mass(&world, bucket).unwrap() - 4.0).abs() < 1e-4);
        let centre = physics.center_of_mass(&world, bucket).unwrap();
        let y = world.get::<&Transform>(bucket).unwrap().position.y;
        assert!((centre.y - (y - 0.075)).abs() < 1e-4, "(1·0 + 3·-0.1)/4 below its origin: {centre} at {y}");
        physics.set_carried_mass(&world, bucket, 0.0, Vec3::ZERO);
        physics.run(&mut world);
        assert!((physics.mass(&world, bucket).unwrap() - 1.0).abs() < 1e-4);
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

    /// A door hung with a kinematic lock set into its leaf — a padlock the
    /// scene placed a few centimetres into it — stays where it was hung and
    /// goes to sleep, whichever side the lock is on: pushed out of the lock
    /// against its own stop, it would tremble there awake for good.
    #[test]
    fn a_door_hung_into_a_kinematic_lock_rests_where_it_was_hung() {
        for side in [1.0f32, -1.0] {
            let text = format!(
                r#"(entities: [
                (id: "0000000000000001", name: "door", model: "builtin:cube", body: Dynamic,
                 transform: (position: (0.6, 1.0, 0.0)),
                 collider: Box(half: (0.5, 1.0, 0.05)), physics: (mass: Some(20.0), gravity: 0.0),
                 joint: Hinge(anchor: (-0.6, 0.0, 0.0), axis: (0.0, -1.0, 0.0), limits_deg: (0.0, 90.0))),
                (id: "0000000000000002", name: "lock", model: "builtin:cube", body: Kinematic,
                 transform: (position: (1.0, 1.0, {z})),
                 collider: Box(half: (0.1, 0.1, 0.06))),
            ])"#,
                z = side * 0.08
            );
            let scene: Scene = ron::from_str(&text).unwrap();
            let mut world = World::new();
            spawn(&scene, &mut world);
            let mut physics = PhysicsWorld::new(1.0 / 30.0);
            let door = the(&world, Body::Dynamic);
            run_for(&mut physics, &mut world, 150);
            let angle = physics.hinge_angle(&world, door).unwrap();
            assert!(angle.abs() < 2.0, "lock on side {side}: where it was hung, not pushed to {angle}°");
            assert_eq!(physics.asleep(&world, door), Some(true), "lock on side {side}: asleep");
        }
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

    /// A dynamic body under a parent scaled unevenly and turned keeps its
    /// own scale, as Unity keeps a Rigidbody's `localScale`: reading one
    /// back out of its sheared world matrix grew it every step (Dacha's
    /// metal sphere mouse's jaw, a hinged body under 275×325×282 bones,
    /// was metres wide in seconds).
    #[test]
    fn a_body_under_an_unevenly_scaled_turned_parent_keeps_its_scale() {
        let text = r#"(entities: [
            (id: "0000000000000001", name: "bones", transform: (rotation_deg: (0.0, -90.0, 0.0), scale: (275.0, 325.0, 282.0)),
             children: [(id: "0000000000000002", name: "jaw",
               transform: (position: (0.0, 0.01, 0.0), rotation_deg: (0.0, -177.0, 136.0), scale: (0.009, 0.0077, 0.0089)),
               body: Dynamic, collider: Box(half: (0.5, 0.5, 0.5)), physics: (gravity: 0.0))]),
        ])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        let mut world = World::new();
        spawn(&scene, &mut world);
        crate::world::apply_hierarchy(&mut world);
        let jaw = world.query::<(hecs::Entity, &Physics)>().iter().find(|(_, p)| p.0 == Body::Dynamic).map(|(e, _)| e).unwrap();
        let was = world.get::<&Transform>(jaw).unwrap().scale;
        let mut physics = PhysicsWorld::new(1.0 / 30.0);
        physics.run(&mut world);
        physics.set_spin(&world, jaw, Vec3::new(0.0, 1.0, 2.0));
        for _ in 0..90 {
            physics.run(&mut world);
            crate::world::apply_hierarchy(&mut world);
        }
        let now = world.get::<&Transform>(jaw).unwrap().scale;
        assert!((now - was).abs().max_element() < 1e-6, "its own scale kept: {was} then {now}");
        let wide = world.get::<&WorldTransform>(jaw).unwrap().0.to_scale_rotation_translation().0.max_element();
        assert!(wide < 4.0, "and not grown in the world: {wide}");
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
                    ColliderShape::Sphere { radius: 0.5, center: Vec3::ZERO },
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
                ColliderShape::Sphere { radius: 0.5, center: Vec3::ZERO },
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
            physics.wind = scrap_core::wind::Wind {
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
        // Where the last sync took it from and to, for a frame between
        // steps to draw it between them.
        let stepped = *world.get::<&crate::world::Stepped>(ball).unwrap();
        assert_eq!(stepped.to, world.get::<&WorldTransform>(ball).unwrap().0);

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
            .with(ColliderShape::Sphere { radius: 0.5, center: Vec3::ZERO })],
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
    fn a_shape_sits_on_its_body_where_its_collider_is_built() {
        let placed = glam::Mat4::from_scale_rotation_translation(
            Vec3::new(1.5, 0.5, 2.0),
            glam::Quat::from_rotation_y(0.7),
            Vec3::new(3.0, 1.0, -2.0),
        );
        let center = Vec3::new(0.1, -0.2, 0.3);
        for shape in [
            ColliderShape::Box { half: Vec3::splat(0.5), center },
            ColliderShape::Sphere { radius: 0.4, center },
            ColliderShape::Capsule { half_height: 0.5, radius: 0.2, center, axis: 0 },
            ColliderShape::Capsule { half_height: 0.5, radius: 0.2, center, axis: 1 },
            ColliderShape::Capsule { half_height: 0.5, radius: 0.2, center, axis: 2 },
            ColliderShape::Cylinder { half_height: 0.5, radius: 0.2 },
            ColliderShape::Ramp { half: Vec3::splat(0.5) },
            ColliderShape::Stairs { half: Vec3::splat(0.5), steps: 3 },
        ] {
            let built = *build_collider(shape, placed, None, true, &MeshShapes::default()).unwrap().position();
            let offset = shape_offset(shape, placed);
            assert!(
                (built.translation - offset.translation).length() < 1e-6
                    && built.rotation.angle_between(offset.rotation) < 1e-6,
                "{shape:?}: built at {built:?}, offset says {offset:?}"
            );
        }
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
            .insert_one(ball, Shape(ColliderShape::Sphere { radius: 2.0, center: Vec3::ZERO }))
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
            ColliderShape::Sphere { radius: 0.2, center: Vec3::ZERO },
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
            ColliderShape::Sphere { radius: 0.2, center: Vec3::ZERO },
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

    /// Game Kit's breakable box: a dynamic body colliding as its model's
    /// hull, with a box collider on a child (one compound), resting on a
    /// mesh floor.
    #[test]
    fn a_dynamic_hull_with_a_child_collider_rests_on_a_mesh_floor() {
        for with_child in [false, true] {
            let mut floor = entity("floor", -0.5, Body::Static, ColliderShape::Model);
            floor.set_part(&crate::scene::ModelRef("builtin:cube".into()));
            floor.transform.scale = Vec3::new(20.0, 1.0, 20.0);
            let mut crate_ = entity("crate", 1.0, Body::Dynamic, ColliderShape::Model);
            crate_.set_part(&crate::scene::ModelRef("builtin:cube".into()));
            if with_child {
                let child = entity(
                    "crate collider",
                    0.0,
                    Body::None,
                    ColliderShape::Box { half: Vec3::splat(0.5), center: Vec3::ZERO },
                );
                crate_.children.push(child);
            }
            let mut scene = Scene { entities: vec![floor, crate_], ..Default::default() };
            scene.assign_ids();
            let mut world = World::new();
            spawn(&scene, &mut world);
            attach_scene_collision_meshes(&mut world, &scene, None);
            let mut physics = PhysicsWorld::new(1.0 / 60.0);
            run_for(&mut physics, &mut world, 120);
            let body = world
                .query::<(hecs::Entity, &Physics)>()
                .iter()
                .find(|(_, p)| p.0 == Body::Dynamic)
                .map(|(e, _)| e)
                .unwrap();
            let y = world.get::<&Transform>(body).unwrap().position.y;
            assert!((y - 0.5).abs() < 0.1, "with a child collider {with_child}: rests on the floor at {y}");
        }
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

    /// A zone meets what moves — a kinematic thing carried into it — and
    /// not what stands still in it, unless its line says it notices that
    /// too: Unity's trigger with no Rigidbody, and one on a kinematic one.
    #[test]
    fn a_zone_notices_what_stands_still_only_when_asked() {
        let cube = ColliderShape::Box { half: Vec3::splat(0.25), center: Vec3::ZERO };
        let inside = |notices_still: bool| {
            let mut zone = entity("zone", 1.0, Body::Trigger, ColliderShape::Box { half: Vec3::splat(2.0), center: Vec3::ZERO });
            if notices_still {
                zone.set_part(&crate::scene::BodyProps { notices_still: true, ..Default::default() });
            }
            let mut scene = Scene {
                entities: vec![zone, entity("crate", 1.0, Body::Static, cube), entity("lift", 1.5, Body::Kinematic, cube)],
                ..Default::default()
            };
            scene.assign_ids();
            let mut world = World::new();
            spawn(&scene, &mut world);
            let mut physics = PhysicsWorld::new(1.0 / 30.0);
            run_for(&mut physics, &mut world, 3);
            let zone = the(&world, Body::Trigger);
            let names: Vec<String> = world
                .get::<&Contacts>(zone)
                .unwrap()
                .inside
                .iter()
                .map(|e| world.get::<&crate::world::LineName>(*e).map(|n| n.0.clone()).unwrap_or_default())
                .collect();
            names
        };
        assert_eq!(inside(false), ["lift"], "what moves, not what stands still");
        let mut both = inside(true);
        both.sort();
        assert_eq!(both, ["crate", "lift"], "asked: what stands still too");
    }

    /// A body that asks for more solver passes than the world gets them,
    /// and keeps what it asked when the world's number changes.
    #[test]
    fn a_body_keeps_the_solver_passes_it_asks_for() {
        let mut door = entity("door", 1.0, Body::Dynamic, ColliderShape::Box { half: Vec3::splat(0.5), center: Vec3::ZERO });
        door.set_part(&crate::scene::BodyProps { solver_iterations: 16, ..Default::default() });
        let mut scene = Scene {
            entities: vec![door, entity("crate", 3.0, Body::Dynamic, ColliderShape::Box { half: Vec3::splat(0.5), center: Vec3::ZERO })],
            ..Default::default()
        };
        scene.assign_ids();
        let mut world = World::new();
        spawn(&scene, &mut world);
        let mut physics = PhysicsWorld::new(1.0 / 30.0);
        physics.set_solver_iterations(4);
        physics.sync_from_world(&mut world);
        let passes = |physics: &PhysicsWorld, name: &str| {
            let e = scene.entities.iter().find(|d| d.name == name).map(|d| by_id(&world, d.id)).unwrap();
            let body = &physics.bodies[world.get::<&BodyHandle>(e).unwrap().0];
            physics.parameters.num_solver_iterations + body.additional_solver_iterations()
        };
        assert_eq!((passes(&physics, "door"), passes(&physics, "crate")), (16, 4));
        physics.set_solver_iterations(8);
        assert_eq!((passes(&physics, "door"), passes(&physics, "crate")), (16, 8));
        physics.set_solver_iterations(20);
        assert_eq!((passes(&physics, "door"), passes(&physics, "crate")), (20, 20));
    }

    /// A trigger part with no body above it is a zone of its own: what falls
    /// into it falls through.
    #[test]
    fn a_trigger_part_on_its_own_stops_nothing() {
        let mut scene = Scene {
            entities: vec![
                entity("floor", 0.0, Body::Static, ColliderShape::Box { half: Vec3::new(5.0, 0.1, 5.0), center: Vec3::ZERO }),
                entity("zone", 2.0, Body::TriggerPart, ColliderShape::Box { half: Vec3::new(2.0, 0.5, 2.0), center: Vec3::ZERO }),
                entity("ball", 4.0, Body::Dynamic, ColliderShape::Sphere { radius: 0.25, center: Vec3::ZERO }),
            ],
            ..Default::default()
        };
        scene.assign_ids();
        let mut world = World::new();
        spawn(&scene, &mut world);
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        run_for(&mut physics, &mut world, 180);
        let y = world.get::<&WorldTransform>(the(&world, Body::Dynamic)).unwrap().0.w_axis.y;
        assert!((y - 0.35).abs() < 0.05, "through the zone to the floor: {y}");
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
                    ColliderShape::Sphere { radius: 0.25, center: Vec3::ZERO },
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
                assert_eq!(contacts.triggers, [ball], "through the trigger");
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

    /// 30 steps a second drawn at 144 frames: a body gliding at a steady
    /// speed is drawn a steady distance further on every frame — not still
    /// for four frames and then a step's worth at once — and so is what
    /// hangs from it, and a lift the game moves. Drawn `AtStep`, it
    /// stair-steps, as Unity's `None` does. The simulation is the same
    /// either way.
    #[test]
    fn a_moving_body_is_drawn_smoothly_between_steps_at_any_frame_rate() {
        use crate::scene::{BodyProps, Drawn};
        use scrap_core::time::{Time, TimeSettings};
        const SPEED: f32 = 3.0;
        const FRAME: f32 = 1.0 / 144.0;
        let run = |drawn: Drawn| {
            let floating = BodyProps { gravity: 0.0, drawn, ..BodyProps::default() };
            let mut glider = entity("glider", 5.0, Body::Dynamic, ColliderShape::Sphere { radius: 0.5, center: Vec3::ZERO }).with(floating);
            // A lamp riding on it.
            glider.children.push(EntityDesc { name: "lamp".into(), transform: Transform { position: Vec3::Y, ..Default::default() }, ..Default::default() });
            let lift = entity("lift", -5.0, Body::Kinematic, ColliderShape::Box { half: Vec3::splat(0.5), center: Vec3::ZERO })
                .with(BodyProps { drawn, ..BodyProps::default() });
            let scene = Scene { entities: vec![glider, lift], ..Default::default() };
            let mut world = World::new();
            spawn(&scene, &mut world);
            crate::world::apply_hierarchy(&mut world);
            let named = |world: &World, name: &str| {
                world.query::<(hecs::Entity, &crate::world::LineName)>().iter().find(|(_, n)| n.0 == name).map(|(e, _)| e).unwrap()
            };
            let (glider, lamp, lift) = (named(&world, "glider"), named(&world, "lamp"), named(&world, "lift"));
            let mut time = Time::new(TimeSettings { fixed_delta: 1.0 / 30.0, ..Default::default() });
            let mut physics = PhysicsWorld::new(1.0 / 30.0);
            physics.sync_from_world(&mut world);
            physics.set_velocity(&world, glider, Vec3::new(SPEED, 0.0, 0.0));
            let mut drawn_x = Vec::new();
            let mut stepped_x = Vec::new();
            for _ in 0..288 {
                time.advance(FRAME);
                while time.next_step().is_some() {
                    // The game moves the lift up at the same speed.
                    world.get::<&mut Transform>(lift).unwrap().position.y += SPEED / 30.0;
                    crate::world::apply_hierarchy(&mut world);
                    physics.run(&mut world);
                    crate::world::apply_hierarchy(&mut world);
                }
                crate::world::interpolate(&mut world, time.interpolation());
                let at = |e| crate::world::drawn(&world, e).unwrap().w_axis;
                drawn_x.push((at(glider).x, at(lamp).x, at(lift).y));
                stepped_x.push(world.get::<&WorldTransform>(glider).unwrap().0.w_axis.x);
            }
            (drawn_x, stepped_x)
        };
        // From the third step on: a lift the game has only just started
        // moving is drawn where its first step put it.
        let deltas = |xs: &[f32]| xs.windows(2).skip(12).map(|w| w[1] - w[0]).collect::<Vec<f32>>();
        let each = SPEED * FRAME;
        let (between, simulated) = run(Drawn::Between);
        for (what, xs) in [
            ("the body", between.iter().map(|d| d.0).collect::<Vec<_>>()),
            ("the lamp on it", between.iter().map(|d| d.1).collect()),
            ("the lift", between.iter().map(|d| d.2).collect()),
        ] {
            let d = deltas(&xs);
            let (lo, hi) = d.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
            assert!(
                (lo - each).abs() < each * 0.02 && (hi - each).abs() < each * 0.02,
                "{what} moves {each} a frame, drawn between steps: from {lo} to {hi}"
            );
        }
        // Where it is drawn is only drawing: the steps are the same.
        let (at_step, simulated_at_step) = run(Drawn::AtStep);
        assert_eq!(simulated, simulated_at_step);
        let d = deltas(&at_step.iter().map(|d| d.0).collect::<Vec<_>>());
        let still = d.iter().filter(|x| x.abs() < 1e-6).count();
        let jumps = d.iter().filter(|x| (**x - SPEED / 30.0).abs() < 1e-3).count();
        assert!(still > d.len() / 2 && jumps + still == d.len(), "drawn at its steps it stands and jumps: {d:?}");
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
        assert!(
            world.get::<&Contacts>(crate_).unwrap().triggers.is_empty(),
            "a solid contact, not a trigger's"
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
    fn a_body_made_only_of_a_scaled_part_lands_on_the_floor() {
        // The greybox shovel: a dynamic root with no collider of its own, a
        // unit box under it scaled to a stick.
        let (mut physics, mut world, scene) = scene_world(
            r#"(entities: [
                (id: "00000000000000a1", name: "floor", model: "m", body: Static,
                 collider: Box(half: (5.0, 0.5, 5.0)), transform: (position: (0.0, -0.5, 0.0))),
                (id: "00000000000000a2", name: "shovel", model: "m", body: Dynamic, physics: (mass: Some(3.0)),
                 transform: (position: (0.0, 1.5, 0.0)), children: [
                    (id: "00000000000000a3", name: "Mesh", model: "builtin:cube", body: Part,
                     collider: Box(half: (0.5, 0.5, 0.5)), transform: (scale: (0.18, 0.18, 0.7))),
                ]),
            ])"#,
        );
        let shovel = by_id(&world, scene.entities[1].id);
        run_for(&mut physics, &mut world, 120);
        let y = world.get::<&WorldTransform>(shovel).unwrap().0.w_axis.y;
        assert!(y > 0.0 && y < 0.2, "lies on the floor: {y}");
    }

    #[test]
    fn a_body_whose_part_is_animated_falls_as_freely_as_any() {
        // A seed packet: its fruit, a collider, bobs on an animated child;
        // the packet falls all the same, its speed kept step to step.
        let (mut physics, mut world, scene) = scene_world(
            r#"(entities: [
                (id: "00000000000000f1", name: "packet", model: "m", body: Dynamic,
                 collider: Box(half: (0.18, 0.16, 0.18)), transform: (position: (0.0, 20.0, 0.0)), children: [
                    (id: "00000000000000f2", name: "fruit", model: "m", body: Part,
                     collider: Sphere(radius: 0.1), transform: (position: (0.0, 0.3, 0.0))),
                ]),
            ])"#,
        );
        let packet = by_id(&world, scene.entities[0].id);
        let fruit = by_id(&world, crate::id::EntityId::from_raw(0xf2));
        run_for(&mut physics, &mut world, 1);
        let start = world.get::<&WorldTransform>(packet).unwrap().0.w_axis.y;
        for i in 0..30 {
            world.get::<&mut Transform>(fruit).unwrap().position.y = 0.3 + 0.05 * (i as f32 * 0.7).sin();
            run_for(&mut physics, &mut world, 1);
        }
        let fell = start - world.get::<&WorldTransform>(packet).unwrap().0.w_axis.y;
        // Half a second of free fall is 1.2 m; rebuilt every step it was
        // a few centimetres.
        assert!(fell > 1.0, "fell {fell} m in half a second");
    }

    #[test]
    fn a_rope_hung_on_a_shapeless_anchor_follows_what_carries_it() {
        // A mouse's tail: a segment on a ball joint to a kinematic anchor
        // that has no collider, under the mouse, which moves.
        let (mut physics, mut world, scene) = scene_world(
            r#"(entities: [
                (id: "00000000000000e1", name: "mouse", model: "m", transform: (position: (0.0, 2.0, 0.0)), children: [
                    (id: "00000000000000e2", name: "anchor", model: "m", body: Kinematic, physics: (gravity: 0.0)),
                ]),
                (id: "00000000000000e3", name: "segment", model: "m", body: Dynamic,
                 collider: Sphere(radius: 0.05), physics: (mass: 0.001),
                 transform: (position: (0.0, 1.7, 0.0)),
                 joint: Ball(to: "00000000000000e2")),
            ])"#,
        );
        let mouse = by_id(&world, scene.entities[0].id);
        let segment = by_id(&world, scene.entities[1].id);
        run_for(&mut physics, &mut world, 10);
        assert!(world.get::<&JointBuilt>(segment).is_ok(), "hung on the anchor");
        for _ in 0..60 {
            world.get::<&mut Transform>(mouse).unwrap().position.x += 0.05;
            run_for(&mut physics, &mut world, 1);
        }
        let at = world.get::<&WorldTransform>(segment).unwrap().0.w_axis.truncate();
        assert!(at.x > 2.0 && (at - Vec3::new(3.0, 2.0, 0.0)).length() < 0.6, "dragged along: {at:?}");
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
            connected: None,
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

    /// Unity's CharacterController.Move: a step up is taken, a wall
    /// stops, the ground is stood on.
    #[test]
    fn a_character_walks_up_a_step_and_stops_at_a_wall() {
        let (mut physics, mut world, _) = scene_world(
            r#"(entities: [
                (id: "00000000000000f1", name: "floor", model: "m", body: Static,
                 collider: Box(half: (20.0, 0.5, 20.0)), transform: (position: (0.0, -0.5, 0.0))),
                (id: "00000000000000f2", name: "step", model: "m", body: Static,
                 collider: Box(half: (1.0, 0.1, 5.0)), transform: (position: (3.0, 0.1, 0.0))),
                (id: "00000000000000f3", name: "wall", model: "m", body: Static,
                 collider: Box(half: (0.5, 2.0, 5.0)), transform: (position: (8.0, 2.0, 0.0))),
                (id: "00000000000000f4", name: "ellen", model: "m", body: Kinematic,
                 collider: Capsule(half_height: 0.5, radius: 0.3), transform: (position: (0.0, 0.82, 0.0))),
            ])"#,
        );
        physics.sync_from_world(&mut world);
        physics.refresh_queries();
        let ellen = by_id(&world, "00000000000000f4".parse().unwrap());
        let mut grounded = false;
        let mut highest: f32 = 0.0;
        for _ in 0..200 {
            let (moved, on) = physics
                .move_character(&world, ellen, Vec3::new(0.05, -0.02, 0.0), 0.3, 45.0)
                .unwrap();
            grounded = on;
            world.get::<&mut Transform>(ellen).unwrap().position += moved;
            crate::world::apply_hierarchy(&mut world);
            physics.sync_from_world(&mut world);
            physics.refresh_queries();
            highest = highest.max(world.get::<&Transform>(ellen).unwrap().position.y);
        }
        let at = world.get::<&Transform>(ellen).unwrap().position;
        assert!(highest > 0.95, "up the 0.2 m step: {highest}");
        assert!(at.x < 7.5 && at.x > 7.0, "stopped at the wall: {at}");
        assert!(grounded, "on the floor");
    }

    /// A jump beside a slope leaves the ground: moving up is never landing.
    #[test]
    fn a_character_jumping_against_a_slope_is_not_grounded() {
        let (mut physics, mut world, _) = scene_world(
            r#"(entities: [
                (id: "00000000000000f1", name: "floor", model: "m", body: Static,
                 collider: Box(half: (20.0, 0.5, 20.0)), transform: (position: (0.0, -0.5, 0.0))),
                (id: "00000000000000f2", name: "slope", model: "m", body: Static,
                 collider: Box(half: (2.0, 2.0, 5.0)), transform: (position: (1.9, 0.0, 0.0), rotation_deg: (0.0, 0.0, 50.0))),
                (id: "00000000000000f4", name: "ellen", model: "m", body: Kinematic,
                 collider: Capsule(half_height: 0.5, radius: 0.3), transform: (position: (0.0, 0.82, 0.0))),
            ])"#,
        );
        physics.sync_from_world(&mut world);
        physics.refresh_queries();
        let ellen = by_id(&world, "00000000000000f4".parse().unwrap());
        let mut rose = 0.0;
        for _ in 0..5 {
            let (moved, on) = physics.move_character(&world, ellen, Vec3::new(0.1, 0.2, 0.0), 0.3, 45.0).unwrap();
            assert!(!on, "going up is not landing");
            rose += moved.y;
            world.get::<&mut Transform>(ellen).unwrap().position += moved;
            crate::world::apply_hierarchy(&mut world);
            physics.sync_from_world(&mut world);
            physics.refresh_queries();
        }
        assert!(rose > 0.5, "she rose: {rose}");
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

    /// A hinge on a moving kinematic body (a lever on the train) rides
    /// with it, held upright by its spring, as in Unity's PhysX. rapier 0.26
    /// dragged it back as if by wind: it put the kinematic end of a joint
    /// where the body stood at the start of the step and never moved it
    /// over the substeps, so the joint pulled the lever toward where the
    /// train had been, and Dacha's train lever took three seconds to
    /// return to centre instead of half a second. rapier 0.36 solves a
    /// kinematic body with the rest; this keeps it so.
    #[test]
    fn a_lever_on_a_moving_kinematic_body_is_not_dragged() {
        let (_, mut world, _) = scene_world(
            r#"(entities: [
                (id: "00000000000000f0", name: "train", model: "m", body: Kinematic,
                 collider: Box(half: (1.0, 0.1, 3.0)), transform: (position: (0.0, 1.0, 0.0))),
                (id: "00000000000000f1", name: "lever", model: "m", body: Dynamic,
                 collider: Box(half: (0.05, 0.5, 0.05)), transform: (position: (0.0, 2.0, 0.0)),
                 physics: (gravity: 0.0, mass: Some(1.0)),
                 joint: Hinge(to: "00000000000000f0", anchor: (0.0, -0.5, 0.0), axis: (-1.0, 0.0, 0.0),
                              motor: (hold: 0.0, strength: 100.0, damping: 10.0))),
            ])"#,
        );
        let mut physics = PhysicsWorld::new(1.0 / 30.0);
        let lever = by_id(&world, "00000000000000f1".parse().unwrap());
        let train = by_id(&world, "00000000000000f0".parse().unwrap());
        // Up to speed gently, then two seconds at 9 m/s.
        let (mut z, mut v) = (0.0f32, 0.0f32);
        for _ in 0..120 {
            v = (v + 3.0 / 30.0).min(9.0);
            z -= v / 30.0;
            world.get::<&mut Transform>(train).unwrap().position.z = z;
            run_for(&mut physics, &mut world, 1);
        }
        let angle = physics.hinge_angle(&world, lever).unwrap();
        assert!(angle.abs() < 2.0, "a spring holding it upright on a steady train: {angle}°");
    }

    #[test]
    fn a_turned_body_keeps_moving() {
        let (mut physics, mut world, _) = scene_world(
            r#"(entities: [
                (id: "00000000000000c1", name: "crate", model: "m", body: Dynamic,
                 collider: Box(half: (0.5, 0.5, 0.5)), transform: (position: (0.0, 5.0, 0.0)),
                 physics: (gravity: 0.0)),
            ])"#,
        );
        let crate_ = by_id(&world, "00000000000000c1".parse().unwrap());
        run_for(&mut physics, &mut world, 1);
        physics.set_velocity(&world, crate_, Vec3::new(3.0, 0.0, 0.0));
        let quarter = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert!(physics.turn(&mut world, crate_, quarter));
        run_for(&mut physics, &mut world, 10);
        let t = *world.get::<&Transform>(crate_).unwrap();
        assert!(t.rotation().angle_between(quarter) < 1e-3, "turned: {:?}", t.rotation());
        assert!((t.position.x - 0.5).abs() < 0.05, "still going: {}", t.position.x);
    }

    /// `ground` standing still as a mesh (scaled by `scale`) and `thing`
    /// over it, its mesh `thing_mesh` if it is a model: their world and the
    /// thing, stepped at 30 Hz as the game steps.
    fn on_mesh(
        ground: CollisionMesh,
        scale: Vec3,
        thing: EntityDesc,
        thing_mesh: Option<CollisionMesh>,
    ) -> (PhysicsWorld, World, hecs::Entity) {
        let mut floor = entity("ground", 0.0, Body::Static, ColliderShape::Model);
        floor.transform.scale = scale;
        let mut scene = Scene { entities: vec![floor, thing], ..Default::default() };
        scene.assign_ids();
        let mut world = World::new();
        spawn(&scene, &mut world);
        let (floor, thing) = (by_id(&world, scene.entities[0].id), by_id(&world, scene.entities[1].id));
        let _ = world.insert_one(floor, ground);
        if let Some(mesh) = thing_mesh {
            let _ = world.insert_one(thing, mesh);
        }
        (PhysicsWorld::new(1.0 / 30.0), world, thing)
    }

    /// A grid of 1 m squares, each two triangles facing up, its height at
    /// (x, z) `height(x, z)`.
    fn ground_mesh(cells: i32, height: impl Fn(f32, f32) -> f32) -> CollisionMesh {
        let side = cells + 1;
        let vertices = (0..side * side).map(|k| {
            let (x, z) = ((k % side - cells / 2) as f32, (k / side - cells / 2) as f32);
            Vec3::new(x, height(x, z), z)
        });
        let indices = (0..cells * cells).flat_map(|k| {
            let (i, j) = (k % cells, k / cells);
            let a = (j * side + i) as u32;
            let (b, c, d) = (a + 1, a + side as u32, a + side as u32 + 1);
            [a, c, b, b, c, d]
        });
        CollisionMesh::from_parts(vertices, indices)
    }

    /// A mine, round and flat, lands on a slope of mesh ground and lies on
    /// it — on the whole of its round bottom, not tipped over the side of
    /// it parry would stand on, and not into the mesh and through.
    #[test]
    fn a_disc_lands_flat_on_a_slope_of_mesh_ground() {
        let slope = |x: f32, _z: f32| 0.2 * x;
        let rim: Vec<Vec3> = (0..24)
            .flat_map(|i| {
                let a = i as f32 / 24.0 * std::f32::consts::TAU;
                [Vec3::new(0.39 * a.cos(), -0.06, 0.39 * a.sin()), Vec3::new(0.39 * a.cos(), 0.06, 0.39 * a.sin())]
            })
            .collect();
        let disc = CollisionMesh::from_parts(rim.into_iter(), std::iter::empty());
        let mut mine = entity("mine", 0.0, Body::Dynamic, ColliderShape::Model);
        mine.transform.position = Vec3::new(0.3, 1.0, 0.2);
        let (mut physics, mut world, mine) = on_mesh(ground_mesh(8, slope), Vec3::ONE, mine, Some(disc));
        run_for(&mut physics, &mut world, 90);
        let placed = world.get::<&WorldTransform>(mine).unwrap().0;
        let at = placed.w_axis.truncate();
        let up = placed.transform_vector3(Vec3::Y).normalize();
        let normal = Vec3::new(-0.2, 1.0, 0.0).normalize();
        let above = (at - Vec3::new(at.x, slope(at.x, at.z), at.z)).dot(normal);
        assert!((above - 0.06).abs() < 0.02, "lying on the ground: {above} above it, at {at}");
        assert!(up.angle_between(normal).to_degrees() < 2.0, "flat on the slope: up {up}");
    }

    /// Two lengths of the same mesh ground are one shape, built once; a
    /// third at another scale is its own; and once nothing has the mesh,
    /// its shape is let go.
    #[test]
    fn bodies_of_one_mesh_at_one_scale_share_its_shape() {
        let mesh = ground_mesh(4, |_, _| 0.0);
        let mut lengths: Vec<EntityDesc> = (0..3)
            .map(|i| {
                let mut e = entity(&format!("rail {i}"), 0.0, Body::Static, ColliderShape::Model);
                e.transform.position = Vec3::new(10.0 * i as f32, 0.0, 0.0);
                e
            })
            .collect();
        lengths[2].transform.scale = Vec3::splat(2.0);
        let mut scene = Scene { entities: lengths, ..Default::default() };
        scene.assign_ids();
        let mut world = World::new();
        spawn(&scene, &mut world);
        let rails: Vec<hecs::Entity> = scene.entities.iter().map(|e| by_id(&world, e.id)).collect();
        for &rail in &rails {
            let _ = world.insert_one(rail, mesh.clone());
        }
        drop(mesh);
        let mut physics = PhysicsWorld::new(1.0 / 30.0);
        physics.sync_from_world(&mut world);
        let shape = |e: hecs::Entity| {
            let body = world.get::<&BodyHandle>(e).unwrap().0;
            let collider = physics.bodies[body].colliders()[0];
            physics.colliders[collider].shared_shape().clone()
        };
        let (a, b, c) = (shape(rails[0]), shape(rails[1]), shape(rails[2]));
        assert!(std::sync::Arc::ptr_eq(&a.0, &b.0), "one scale, one shape");
        assert!(!std::sync::Arc::ptr_eq(&a.0, &c.0), "another scale, another shape");
        assert_eq!(physics.mesh_shapes.len(), 2);
        for &rail in &rails {
            let _ = world.despawn(rail);
        }
        physics.sync_from_world(&mut world);
        assert_eq!(physics.mesh_shapes.len(), 2, "kept until something new is built");
        let other = ground_mesh(2, |_, _| 0.0);
        let mut scene = Scene { entities: vec![entity("ground", 0.0, Body::Static, ColliderShape::Model)], ..Default::default() };
        scene.assign_ids();
        spawn(&scene, &mut world);
        let ground = by_id(&world, scene.entities[0].id);
        let _ = world.insert_one(ground, other);
        physics.sync_from_world(&mut world);
        assert_eq!(physics.mesh_shapes.len(), 1, "the rails' shapes let go, the new ground's kept");
    }

    /// Ground mirrored by its scale still faces up: a ball lands on it.
    #[test]
    fn a_ball_lands_on_mesh_ground_its_scale_mirrors() {
        let ball = entity("ball", 2.0, Body::Dynamic, ColliderShape::Sphere { radius: 0.5, center: Vec3::ZERO });
        let (mut physics, mut world, ball) = on_mesh(ground_mesh(4, |_, _| 0.0), Vec3::new(-1.0, 1.0, 1.0), ball, None);
        run_for(&mut physics, &mut world, 60);
        let y = world.get::<&WorldTransform>(ball).unwrap().0.w_axis.y;
        assert!((y - 0.5).abs() < 0.05, "on the ground: y {y}");
    }

    /// Two links of a rope on a ball joint, unswept, stood on end and
    /// dropped a metre onto the floor: they land and stay, as Unity's
    /// (Discrete) links do. Swept, rapier stops the lower link at the floor
    /// mid-step and not the upper, and the joint flings both back up.
    #[test]
    fn unswept_links_dropped_on_end_land_without_a_bounce() {
        let (_, mut world, scene) = scene_world(
            r#"(entities: [
                (id: "00000000000000f0", name: "floor", body: Static, collider: Box(half: (10.0, 0.5, 10.0)), transform: (position: (0.0, -0.5, 0.0))),
                (id: "00000000000000f1", name: "upper", body: Dynamic, collider: Capsule(half_height: 0.4, radius: 0.1),
                 physics: (mass: Some(0.4), unswept: true), transform: (position: (0.0, 2.5, 0.0), rotation_deg: (180.0, 0.0, 0.0))),
                (id: "00000000000000f2", name: "lower", body: Dynamic, collider: Capsule(half_height: 0.4, radius: 0.1),
                 physics: (mass: Some(0.4), unswept: true), transform: (position: (0.0, 1.5, 0.0), rotation_deg: (180.0, 0.0, 0.0)),
                 joint: Ball(to: "00000000000000f1", anchor: (0.0, -0.5, 0.0), connected: (0.0, 0.5, 0.0))),
            ])"#,
        );
        let mut physics = PhysicsWorld::new(1.0 / 30.0);
        physics.set_solver_iterations(16);
        let (upper, lower) = (by_id(&world, scene.entities[1].id), by_id(&world, scene.entities[2].id));
        physics.ignore_collision(upper, lower, true);
        let mut highest_after_landing = f32::MIN;
        let mut landed = false;
        for _ in 0..45 {
            run_for(&mut physics, &mut world, 1);
            let y = world.get::<&WorldTransform>(lower).unwrap().0.w_axis.y;
            landed |= y < 0.6;
            if landed {
                highest_after_landing = highest_after_landing.max(y);
            }
        }
        assert!(landed, "it fell");
        assert!(highest_after_landing < 0.75, "no bounce: the lower link rose to {highest_after_landing}");
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

        // A stone thrown at 300 m/s at a thin wall stops there, `fast` or
        // not: every dynamic body is swept between steps, so a shovel's
        // stick does not fall through a mesh ground either. (Unity's
        // discrete bodies would let the stone through; nothing in a game
        // wants that, and a stick through the sand is what it cost.)
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
                .set_linvel(Vector::new(300.0, 0.0, 0.0), true);
            run_for(&mut physics, &mut world, 10);
            let x = world.get::<&WorldTransform>(stone).unwrap().0.w_axis.x;
            x
        };
        assert!(throw(false) < 0.0, "stopped at the wall: {}", throw(false));
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
                scrap_core::world::take_off::<Props>(world, entity);
            } else {
                let _ = world.insert_one(entity, Props(props));
            }
        }
        if changed.has("joint") {
            let joint = line.joint();
            if joint.is_none() {
                scrap_core::world::take_off::<Jointed>(world, entity);
            } else {
                let _ = world.insert_one(entity, Jointed(joint));
            }
            // A joint set anew is whole again.
            scrap_core::world::take_off::<JointBroken>(world, entity);
        }
        if changed.has("joint_break") {
            match line.joint_break() {
                Some(force) => {
                    let _ = world.insert_one(entity, JointBreak(force));
                }
                None => {
                    scrap_core::world::take_off::<JointBreak>(world, entity);
                }
            }
        }
    }
}

