//! The core's world: where things are and what hangs off what, which of
//! them the file placed, and which are switched off; and putting a scene's
//! lines into it — the core spawns an entity's identity, place and tree,
//! and each module dresses it from its own fields ([`Dress`]).
//!
//! The entity store is `hecs`. Nothing here wraps it or hides it: a game
//! wanting to add a component adds one.

use std::collections::{HashMap, HashSet};

use hecs::World;

use crate::id::EntityId;
use crate::scene::{EntityDesc, Scene, Transform};

/// Where an entity ends up in the world, with its parents already applied.
///
/// Stored rather than recomputed per frame: a draw list is built every frame
/// and a hierarchy is not, so the work belongs where the change happens.
/// Moving a parent means re-running [`apply_hierarchy`] over its subtree, and
/// nothing else has to know.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldTransform(pub glam::Mat4);

/// Which entity of the scene this one was spawned from.
///
/// The [`EntityId`](crate::EntityId) of its line in the file. It is what
/// lets anything holding an entity answer "which line in the file is this" —
/// an editor's selection, a message about a missing model, a tool writing a
/// change back — and it keeps answering correctly when lines are added or
/// removed above it, which a position in the file would not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneId(pub crate::id::EntityId);

/// Which line of a prefab a run-time entity was spawned from, in that
/// spawn's scope: what the links in its components — a machine's lever, a
/// hole's volume — name it by. Not a [`SceneId`], which a reload of the
/// scene would take for one of its own lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpawnedId(pub crate::id::EntityId);

/// What an entity is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parent(pub hecs::Entity);

/// The name its scene line gives it: what a skin finds its bones by, a
/// Unity skinned mesh's bones being things of the scene, not of the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineName(pub String);

/// Switched off, from a line's `inactive` or [`set_active`]: it and all
/// under it are not drawn and not solid, as Unity's inactive GameObject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inactive;

/// Switch an entity on or off, and so everything under it.
pub fn set_active(world: &mut World, entity: hecs::Entity, active: bool) {
    if active {
        let _ = world.remove_one::<Inactive>(entity);
    } else {
        let _ = world.insert_one(entity, Inactive);
    }
}

/// Whether an entity is on: itself and every parent up to the top.
/// Unity's `activeInHierarchy`.
pub fn is_active(world: &World, entity: hecs::Entity) -> bool {
    let mut at = Some(entity);
    let mut depth = 0;
    while let Some(e) = at {
        if world.get::<&Inactive>(e).is_ok() {
            return false;
        }
        at = world.get::<&Parent>(e).ok().map(|p| p.0);
        depth += 1;
        if depth > 64 {
            break;
        }
    }
    true
}

/// Every entity that is off, itself or by a parent: what the frame and the
/// physics leave out.
pub fn inactive_in_hierarchy(world: &World) -> std::collections::HashSet<hecs::Entity> {
    let mut out = std::collections::HashSet::new();
    if world.query::<&Inactive>().iter().next().is_none() {
        return out;
    }
    for (entity, _) in world.query::<(hecs::Entity, &Transform)>().iter() {
        if !is_active(world, entity) {
            out.insert(entity);
        }
    }
    out
}

/// What could not be spawned, and why. Returned rather than logged: a missing
/// model is a fact the editor wants to show next to the entity, not a line in
/// a terminal nobody is reading.
#[derive(Debug, Clone, PartialEq)]
pub struct Unresolved {
    pub entity_name: String,
    pub model: String,
}

/// Which of a line's module fields changed: all of them for a line being
/// spawned, the ones whose text differs for a line a reload patches.
#[derive(Debug, Clone, Copy)]
pub enum Changed<'a> {
    All,
    Only(&'a [String]),
}

impl Changed<'_> {
    /// Whether any of these fields changed.
    pub fn any(&self, names: &[&str]) -> bool {
        match self {
            Changed::All => true,
            Changed::Only(changed) => names.iter().any(|n| changed.iter().any(|c| c == n)),
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.any(&[name])
    }
}

/// How a module puts its fields of a line on the entity spawned from it —
/// a body, a light, a sound, what it looks like — and takes off what the
/// line no longer asks for: as the line spawns, and again when a reload
/// changed one of the fields it reads (docs/modules.md). The core spawns an
/// entity's identity, place and tree; everything else is a module's
/// dresser.
pub trait Dress {
    /// The fields it reads: a reload that changed none of them passes it by.
    fn parts(&self) -> &[&'static str];

    /// Dress `entity` from `line`, for the fields `changed` names. Models
    /// and other things asked for that nothing answers to go in `missing`.
    fn dress(
        &mut self,
        line: &EntityDesc,
        entity: hecs::Entity,
        world: &mut World,
        changed: Changed,
        missing: &mut Vec<Unresolved>,
    );
}

/// Put a scene into a world with these modules' dressers.
pub fn spawn_scene_dressed(scene: &Scene, world: &mut World, dressers: &mut [Box<dyn Dress + '_>]) -> Vec<Unresolved> {
    let mut missing = Vec::new();
    for desc in &scene.entities {
        spawn_subtree(desc, None, glam::Mat4::IDENTITY, world, dressers, &mut missing);
    }
    missing
}

/// Spawn one entity and everything hanging off it.
///
/// A child whose model is missing still spawns, because its own children may
/// be fine and dropping the branch would move them. It just gets nothing to
/// draw.
fn spawn_subtree(
    desc: &EntityDesc,
    parent: Option<hecs::Entity>,
    parent_matrix: glam::Mat4,
    world: &mut World,
    dressers: &mut [Box<dyn Dress + '_>],
    missing: &mut Vec<Unresolved>,
) {
    let world_matrix = parent_matrix * desc.transform.matrix();
    let entity = spawn_one(desc, parent, world_matrix, world, dressers, missing);
    for child in &desc.children {
        spawn_subtree(child, Some(entity), world_matrix, world, dressers, missing);
    }
}

/// Spawn one entity from its line in the file, without its children: the
/// core's part — its place, its identity, its parent, whether it is on —
/// then every module's.
fn spawn_one(
    desc: &EntityDesc,
    parent: Option<hecs::Entity>,
    world_matrix: glam::Mat4,
    world: &mut World,
    dressers: &mut [Box<dyn Dress + '_>],
    missing: &mut Vec<Unresolved>,
) -> hecs::Entity {
    let entity = world.spawn((desc.transform, WorldTransform(world_matrix), SceneId(desc.id), LineName(desc.name.clone())));
    if let Some(parent) = parent {
        let _ = world.insert_one(entity, Parent(parent));
    }
    if desc.inactive {
        let _ = world.insert_one(entity, Inactive);
    }
    dress_layer(desc, entity, world);
    for dresser in dressers.iter_mut() {
        dresser.dress(desc, entity, world, Changed::All, missing);
    }
    entity
}

/// What [`patch_scene`] did to a world.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Patched {
    /// Lines new in the file, spawned.
    pub spawned: usize,
    /// Lines that changed, patched in the fields that changed and no others.
    pub updated: usize,
    /// Lines gone from the file, despawned with whatever hung off them.
    pub despawned: usize,
    /// Models the new or changed lines name and nothing answers to.
    pub missing: Vec<Unresolved>,
}

impl Patched {
    /// Whether the world was left as it was.
    pub fn is_empty(&self) -> bool {
        self.spawned == 0 && self.updated == 0 && self.despawned == 0
    }
}

/// [`patch_scene`] with these modules' dressers.
pub fn patch_scene_dressed(
    before: &Scene,
    after: &Scene,
    world: &mut World,
    dressers: &mut [Box<dyn Dress + '_>],
) -> Patched {
    let mut old: HashMap<EntityId, (&EntityDesc, Option<EntityId>)> = HashMap::new();
    index(&before.entities, None, &mut old);
    let live: HashMap<EntityId, hecs::Entity> = world
        .query::<(hecs::Entity, &SceneId)>()
        .iter()
        .map(|(entity, id)| (id.0, entity))
        .collect();

    let mut patch = Patch {
        old: &old,
        live: &live,
        kept: HashSet::new(),
        out: Patched::default(),
    };
    for desc in &after.entities {
        patch.subtree(desc, None, glam::Mat4::IDENTITY, world, dressers);
    }
    let Patch { kept, mut out, .. } = patch;

    // Only lines this scene had: a world can hold several scenes, and a line
    // another scene spawned is not this one's to remove.
    let mut doomed: HashSet<hecs::Entity> = live
        .iter()
        .filter(|(id, _)| !kept.contains(*id) && old.contains_key(*id))
        .map(|(_, entity)| *entity)
        .collect();
    loop {
        let hanging: Vec<hecs::Entity> = world
            .query::<(hecs::Entity, &Parent)>()
            .iter()
            .filter(|(entity, parent)| doomed.contains(&parent.0) && !doomed.contains(entity))
            .map(|(entity, _)| entity)
            .collect();
        if hanging.is_empty() {
            break;
        }
        doomed.extend(hanging);
    }
    for entity in &doomed {
        let _ = world.despawn(*entity);
    }
    out.despawned = doomed.len();

    apply_hierarchy(world);
    out
}

/// Every line of a scene by ID, with the ID of the line it hangs under.
fn index<'a>(
    entities: &'a [EntityDesc],
    parent: Option<EntityId>,
    out: &mut HashMap<EntityId, (&'a EntityDesc, Option<EntityId>)>,
) {
    for desc in entities {
        out.insert(desc.id, (desc, parent));
        index(&desc.children, Some(desc.id), out);
    }
}

struct Patch<'a> {
    old: &'a HashMap<EntityId, (&'a EntityDesc, Option<EntityId>)>,
    live: &'a HashMap<EntityId, hecs::Entity>,
    kept: HashSet<EntityId>,
    out: Patched,
}

impl Patch<'_> {
    fn subtree(
        &mut self,
        desc: &EntityDesc,
        parent: Option<(EntityId, hecs::Entity)>,
        parent_matrix: glam::Mat4,
        world: &mut World,
        dressers: &mut [Box<dyn Dress + '_>],
    ) {
        self.kept.insert(desc.id);
        let world_matrix = parent_matrix * desc.transform.matrix();
        let entity = match self.live.get(&desc.id) {
            Some(&entity) if world.contains(entity) => {
                let was = self.old.get(&desc.id).copied();
                if self.update(desc, was, parent, entity, world, dressers) {
                    self.out.updated += 1;
                }
                entity
            }
            _ => {
                self.out.spawned += 1;
                spawn_one(
                    desc,
                    parent.map(|(_, entity)| entity),
                    world_matrix,
                    world,
                    dressers,
                    &mut self.out.missing,
                )
            }
        };
        for child in &desc.children {
            self.subtree(child, Some((desc.id, entity)), world_matrix, world, dressers);
        }
    }

    /// Write the fields of one line that differ from its previous version:
    /// the core's here, each module's by its dresser. Returns whether
    /// anything did.
    fn update(
        &mut self,
        desc: &EntityDesc,
        was: Option<(&EntityDesc, Option<EntityId>)>,
        parent: Option<(EntityId, hecs::Entity)>,
        entity: hecs::Entity,
        world: &mut World,
        dressers: &mut [Box<dyn Dress + '_>],
    ) -> bool {
        let mut changed = false;
        if was.is_none_or(|(old, _)| old.transform != desc.transform) {
            let _ = world.insert_one(entity, desc.transform);
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.inactive != desc.inactive) {
            set_active(world, entity, !desc.inactive);
            changed = true;
        }
        if was.is_none_or(|(_, old_parent)| old_parent != parent.map(|(id, _)| id)) {
            match parent {
                Some((_, parent)) => {
                    let _ = world.insert_one(entity, Parent(parent));
                }
                None => {
                    let _ = world.remove_one::<Parent>(entity);
                }
            }
            changed = true;
        }
        // The module fields whose text differs, both ways: set, changed
        // or taken off.
        let differ: Vec<String> = match was {
            None => Vec::new(),
            Some((old, _)) => desc
                .parts
                .names()
                .chain(old.parts.names())
                .filter(|name| old.parts.raw(name) != desc.parts.raw(name))
                .map(str::to_string)
                .collect(),
        };
        if fields_changed(was.map(|(old, _)| old), desc, "layer") {
            dress_layer(desc, entity, world);
            changed = true;
        }
        let fields = match was {
            None => Changed::All,
            Some(_) => Changed::Only(&differ),
        };
        for dresser in dressers.iter_mut() {
            if fields.any(dresser.parts()) {
                dresser.dress(desc, entity, world, fields, &mut self.out.missing);
                changed = true;
            }
        }
        changed
    }
}

/// Recompute every [`WorldTransform`] from the local transforms and parents.
///
/// Call it after moving something that has children. It resolves each entity
/// by walking up to its root, and it stops at a depth limit rather than
/// looping: nothing built from a scene file can contain a cycle, but an
/// editor that sets a parent by hand can make one, and hanging is a worse
/// answer than a wrong transform.
pub fn apply_hierarchy(world: &mut World) {
    const MAX_DEPTH: usize = 64;

    // Snapshotted first, then resolved, then written back. Walking up the
    // tree while holding a query borrow would mean reading the world through
    // the same handle that is iterating it; taking the local transforms out
    // once is both simpler and cheaper than resolving inside the loop.
    let mut locals: std::collections::HashMap<hecs::Entity, (glam::Mat4, Option<hecs::Entity>)> =
        std::collections::HashMap::new();
    for (entity, local, parent) in world
        .query::<(hecs::Entity, &Transform, Option<&Parent>)>()
        .iter()
    {
        locals.insert(entity, (local.matrix(), parent.map(|p| p.0)));
    }
    // What stands between a thing and its parent — the bone of the
    // parent's skeleton it rides on — goes between.
    for (entity, between) in world.query::<(hecs::Entity, &Between)>().iter() {
        if let Some((local, _)) = locals.get_mut(&entity) {
            *local = between.0 * *local;
        }
    }

    let mut resolved: Vec<(hecs::Entity, glam::Mat4)> = Vec::with_capacity(locals.len());
    for (entity, (local, parent)) in &locals {
        let mut matrix = *local;
        let mut current = *parent;
        let mut depth = 0;
        while let Some(ancestor) = current {
            let Some((ancestor_local, ancestor_parent)) = locals.get(&ancestor) else {
                // A parent that no longer exists leaves the child where it
                // is rather than dropping it: a despawn should not teleport
                // whatever was attached.
                break;
            };
            matrix = *ancestor_local * matrix;
            current = *ancestor_parent;
            depth += 1;
            if depth >= MAX_DEPTH {
                // Nothing built from a scene file can contain a cycle — a
                // tree written as a tree cannot describe one — but an editor
                // setting a parent by hand can. A wrong transform is a better
                // answer than a hang.
                break;
            }
        }
        resolved.push((*entity, matrix));
    }

    for (entity, matrix) in resolved {
        let _ = world.insert_one(entity, WorldTransform(matrix));
    }
}

/// The collision layer's name, kept from the scene when not `default`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer(pub String);

/// What stands between an entity and its parent, in the parent's space: a
/// bone of the parent's skeleton it rides on, set by the module that knows
/// where the bone is (`animator::hold_on_bones`). [`apply_hierarchy`]
/// puts it between the parent and the entity's own transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Between(pub glam::Mat4);

/// Spawn an entity and everything under it as the game's own rather than
/// the file's: without [`SceneId`], so a reload of the scene never touches
/// it — the file does not know it exists. What a game spawning a prefab at
/// run time builds on. Returns every entity spawned with the line it came
/// from, root first, and what nothing answered to.
pub fn spawn_owned_dressed<'a>(
    desc: &'a EntityDesc,
    parent: Option<hecs::Entity>,
    world: &mut World,
    dressers: &mut [Box<dyn Dress + '_>],
) -> (Vec<(hecs::Entity, &'a EntityDesc)>, Vec<Unresolved>) {
    fn walk<'a>(
        desc: &'a EntityDesc,
        parent: Option<hecs::Entity>,
        parent_matrix: glam::Mat4,
        world: &mut World,
        dressers: &mut [Box<dyn Dress + '_>],
        out: &mut (Vec<(hecs::Entity, &'a EntityDesc)>, Vec<Unresolved>),
    ) {
        let matrix = parent_matrix * desc.transform.matrix();
        let entity = spawn_one(desc, parent, matrix, world, dressers, &mut out.1);
        let _ = world.remove_one::<SceneId>(entity);
        out.0.push((entity, desc));
        for child in &desc.children {
            walk(child, Some(entity), matrix, world, dressers, out);
        }
    }
    let parent_matrix = parent
        .and_then(|p| world.get::<&WorldTransform>(p).ok().map(|w| w.0))
        .unwrap_or(glam::Mat4::IDENTITY);
    let mut out = (Vec::new(), Vec::new());
    walk(desc, parent, parent_matrix, world, dressers, &mut out);
    out
}

/// The layer a line is on, as a component; none for the default one.
fn dress_layer(desc: &EntityDesc, entity: hecs::Entity, world: &mut World) {
    let layer = desc.layer();
    if layer.is_empty() {
        let _ = world.remove_one::<Layer>(entity);
    } else {
        let _ = world.insert_one(entity, Layer(layer));
    }
}

fn fields_changed(old: Option<&EntityDesc>, new: &EntityDesc, name: &str) -> bool {
    old.is_none_or(|old| old.parts.raw(name) != new.parts.raw(name))
}

// Who simulates an entity. The network module decides and moves it
// between peers; every module that simulates — physics, routes, the game's
// own systems — filters on it, which is why it is here and not there.

/// This peer simulates it: what simulating systems filter on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owned;

/// Someone else simulates it; this peer shows what they say. A body with
/// this is kinematic; nothing simulates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replica;

// What the game spawned at run time, by the identity it is known by: the
// network names it by this, and a save writes it down by it.

/// The network identity of an entity the game spawned at run time — one
/// the scene file does not have, so no [`SceneId`] names it. Minted by the
/// spawner, so the entity is live the same frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetId(pub EntityId);

/// Which prefab a run-time entity was spawned from, so a peer joining
/// later can spawn the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetPrefab(pub String);

/// Every entity the network can name: the scene's by [`SceneId`], the
/// run-time ones by [`NetId`].
pub fn addressable(world: &hecs::World) -> HashMap<EntityId, hecs::Entity> {
    let mut out: HashMap<EntityId, hecs::Entity> = world
        .query::<(hecs::Entity, &SceneId)>()
        .without::<&crate::netsim::Unshared>()
        .iter()
        .map(|(entity, id)| (id.0, entity))
        .collect();
    out.extend(
        world
            .query::<(hecs::Entity, &NetId)>()
            .iter()
            .map(|(entity, id)| (id.0, entity)),
    );
    out
}

/// How the network names an entity.
pub fn network_id(world: &hecs::World, entity: hecs::Entity) -> Option<EntityId> {
    world
        .get::<&NetId>(entity)
        .map(|n| n.0)
        .ok()
        .or_else(|| world.get::<&SceneId>(entity).map(|s| s.0).ok())
}

/// An entity and everything parented to it.
pub fn despawn_tree(world: &mut hecs::World, root: hecs::Entity) {
    let mut doomed = vec![root];
    let mut i = 0;
    while i < doomed.len() {
        let parent = doomed[i];
        doomed.extend(
            world
                .query::<(hecs::Entity, &crate::world::Parent)>()
                .iter()
                .filter(|(_, p)| p.0 == parent)
                .map(|(e, _)| e),
        );
        i += 1;
    }
    for entity in doomed {
        let _ = world.despawn(entity);
    }
}
