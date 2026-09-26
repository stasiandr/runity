//! The core's world: where things are and what hangs off what, which of
//! them the file placed, and which are switched off; and putting a scene's
//! lines into it — the core spawns an entity's identity, place and tree,
//! and each module dresses it from its own fields ([`Dress`]).
//!
//! The entity store is `hecs`. Nothing here wraps it or hides it: a game
//! wanting to add a component adds one.

use std::collections::HashMap;

use crate::hash::{FastMap, FastSet};

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

/// Where a fixed step moved it from and to: set by what steps it (physics
/// on a moving body), for the frame to draw it in between ([`interpolate`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stepped {
    pub from: glam::Mat4,
    pub to: glam::Mat4,
    /// Drawn on past `to` as the step moved it, rather than between the
    /// two: extrapolated (Unity's `RigidbodyInterpolation.Extrapolate`).
    pub ahead: bool,
}

/// Where it is drawn this frame, when that is not its [`WorldTransform`]:
/// between the last two fixed steps ([`interpolate`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shown(pub glam::Mat4);

/// A move longer than this in a step is a jump (a respawn, a teleport),
/// drawn where it landed rather than slid along.
const JUMP: f32 = 5.0;

/// Whether two placings are the same one, up to the rounding a hierarchy
/// pass puts into a matrix taken apart and put back together.
fn same_place(a: glam::Mat4, b: glam::Mat4) -> bool {
    a.abs_diff_eq(b, 1e-4)
}

/// Where each stepped thing is drawn this frame: `alpha` of the way from
/// where the last step found it to where it left it — the frame's share of
/// the next step, [`Time::interpolation`](crate::time::Time::interpolation).
/// Without it a simulation at 30 steps a second shown at 144 frames draws
/// every position four or five times, and motion judders. What hangs from a
/// stepped thing (a camera on the player, a skinned body's bones, a lamp on
/// a torch) is drawn with it.
///
/// One step behind, as every such scheme: what is drawn is between the
/// last two steps, never ahead of them — unless the step said
/// [`Stepped::ahead`]. Something moved since its step by other means (its
/// [`WorldTransform`] is no longer where the step left it) is drawn where
/// it is. Only drawing: the world's [`WorldTransform`]s and everything the
/// simulation reads stay where the steps put them.
pub fn interpolate(world: &mut World, alpha: f32) {
    let alpha = alpha.clamp(0.0, 1.0);
    // Where each is drawn, and for a stepped one also how far that is from
    // where it stands (`at * placed⁻¹`), which what hangs from it shares:
    // inverted once per stepped thing, not once per thing under it.
    let mut shown: FastMap<hecs::Entity, (glam::Mat4, Option<glam::Mat4>)> = FastMap::default();
    for (entity, placed, stepped) in world.query::<(hecs::Entity, &WorldTransform, &Stepped)>().iter() {
        if !same_place(placed.0, stepped.to) || stepped.from == stepped.to {
            continue;
        }
        let (fs, fr, ft) = stepped.from.to_scale_rotation_translation();
        let (ts, tr, tt) = stepped.to.to_scale_rotation_translation();
        if ft.distance(tt) > JUMP {
            continue;
        }
        // Extrapolated, the step's own motion once more: `from` to `to`
        // run on past `to` by the frame's share.
        let t = if stepped.ahead { 1.0 + alpha } else { alpha };
        let turn = if stepped.ahead {
            (glam::Quat::IDENTITY.slerp((tr * fr.inverse()).normalize(), alpha) * tr).normalize()
        } else {
            fr.slerp(tr, t)
        };
        let at = glam::Mat4::from_scale_rotation_translation(fs.lerp(ts, t), turn, ft.lerp(tt, t));
        shown.insert(entity, (at, Some(at * placed.0.inverse())));
    }
    // What hangs from them: its place relative to the nearest stepped
    // ancestor kept.
    if !shown.is_empty() {
        let mut hanging = Vec::new();
        for (entity, placed, parent) in world.query::<(hecs::Entity, &WorldTransform, &Parent)>().iter() {
            if shown.contains_key(&entity) {
                continue;
            }
            let mut up = Some(parent.0);
            for _ in 0..16 {
                let Some(ancestor) = up else {
                    break;
                };
                if let Some((_, Some(moved))) = shown.get(&ancestor) {
                    hanging.push((entity, *moved * placed.0));
                    break;
                }
                up = world.get::<&Parent>(ancestor).ok().map(|p| p.0);
            }
        }
        for (entity, at) in hanging {
            shown.insert(entity, (at, None));
        }
    }
    // Written in place where it already is, taken off where it no longer
    // applies, inserted only where it is new.
    let mut stale = Vec::new();
    for (entity, drawn) in world.query_mut::<(hecs::Entity, &mut Shown)>() {
        match shown.remove(&entity) {
            Some((at, _)) => drawn.0 = at,
            None => stale.push(entity),
        }
    }
    for entity in stale {
        crate::world::take_off::<Shown>(world, entity);
    }
    for (entity, (at, _)) in shown {
        let _ = world.insert_one(entity, Shown(at));
    }
}

/// Where it is drawn this frame: [`Shown`] when [`interpolate`] set it,
/// else its [`WorldTransform`].
pub fn drawn_at(placed: &WorldTransform, shown: Option<&Shown>) -> glam::Mat4 {
    shown.map_or(placed.0, |s| s.0)
}

/// Where an entity is drawn this frame ([`drawn_at`]): what a camera on it,
/// a hand drawn holding it or a label over it wants, rather than where the
/// last step left it.
pub fn drawn(world: &World, entity: hecs::Entity) -> Option<glam::Mat4> {
    let placed = world.get::<&WorldTransform>(entity).ok()?;
    Some(drawn_at(&placed, world.get::<&Shown>(entity).ok().as_deref()))
}

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

/// Take `T` off an entity if it has one: `remove_one`, for a caller that
/// does not want it back. Asked first, because a dresser takes off what a
/// line does not ask for, and on a line just spawned that is nearly every
/// part it knows — where `remove_one` on a missing component still flushes
/// the world and looks through the entity's archetype for it.
pub fn take_off<T: hecs::Component>(world: &mut World, entity: hecs::Entity) {
    if world.satisfies::<&T>(entity) {
        let _ = world.remove_one::<T>(entity);
    }
}

/// Switch an entity on or off, and so everything under it.
pub fn set_active(world: &mut World, entity: hecs::Entity, active: bool) {
    if active {
        crate::world::take_off::<Inactive>(world, entity);
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
pub fn inactive_in_hierarchy(world: &World) -> crate::hash::FastSet<hecs::Entity> {
    inactive_among(world, world.query::<(hecs::Entity, &Transform)>().iter().map(|(e, _)| e))
}

/// Which of `entities` are off, themselves or by a parent: what
/// [`inactive_in_hierarchy`] says about them, looking up only their own
/// lines to the root — the physics asks about its few hundred bodies, not
/// every entity in the level.
pub fn inactive_among(
    world: &World,
    entities: impl IntoIterator<Item = hecs::Entity>,
) -> crate::hash::FastSet<hecs::Entity> {
    let mut out = crate::hash::FastSet::default();
    if world.query::<&Inactive>().iter().next().is_none() {
        return out;
    }
    // Each entity's answer is its parent's unless it is off itself, so the
    // answers are remembered on the way up: one look per entity, not one
    // per entity per level. Remembered by the entity's slot (0 not yet
    // known, 1 on, 2 off), which hashes nothing: the world holds still
    // while this looks, so a slot is one entity throughout.
    let mut known: Vec<u8> = Vec::new();
    let recall = |known: &Vec<u8>, e: hecs::Entity| match known.get(e.id() as usize) {
        Some(1) => Some(false),
        Some(2) => Some(true),
        _ => None,
    };
    let remember = |known: &mut Vec<u8>, e: hecs::Entity, off: bool| {
        let slot = e.id() as usize;
        if slot >= known.len() {
            known.resize(slot + 1, 0);
        }
        known[slot] = if off { 2 } else { 1 };
    };
    let mut chain = Vec::new();
    for entity in entities {
        chain.clear();
        let mut at = Some(entity);
        let mut off = false;
        while let Some(e) = at {
            if let Some(k) = recall(&known, e) {
                off = k;
                break;
            }
            let Ok(line) = world.entity(e) else { break };
            if line.has::<Inactive>() {
                off = true;
                remember(&mut known, e, true);
                break;
            }
            chain.push(e);
            if chain.len() > 64 {
                break;
            }
            at = line.get::<&Parent>().map(|p| p.0);
        }
        for &e in &chain {
            remember(&mut known, e, off);
        }
        if off {
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
    let mut old: FastMap<EntityId, (&EntityDesc, Option<EntityId>)> = FastMap::default();
    index(&before.entities, None, &mut old);
    let live: FastMap<EntityId, hecs::Entity> = world
        .query::<(hecs::Entity, &SceneId)>()
        .iter()
        .map(|(entity, id)| (id.0, entity))
        .collect();

    let mut patch = Patch {
        old: &old,
        live: &live,
        kept: FastSet::default(),
        out: Patched::default(),
    };
    for desc in &after.entities {
        patch.subtree(desc, None, glam::Mat4::IDENTITY, world, dressers);
    }
    let Patch { kept, mut out, .. } = patch;

    // Only lines this scene had: a world can hold several scenes, and a line
    // another scene spawned is not this one's to remove.
    let mut doomed: FastSet<hecs::Entity> = live
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
    out: &mut FastMap<EntityId, (&'a EntityDesc, Option<EntityId>)>,
) {
    for desc in entities {
        out.insert(desc.id, (desc, parent));
        index(&desc.children, Some(desc.id), out);
    }
}

struct Patch<'a> {
    old: &'a FastMap<EntityId, (&'a EntityDesc, Option<EntityId>)>,
    live: &'a FastMap<EntityId, hecs::Entity>,
    kept: FastSet<EntityId>,
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
                    crate::world::take_off::<Parent>(world, entity);
                }
            }
            changed = true;
        }
        // The module fields whose text differs, both ways: set, changed
        // or taken off.
        let differ: Vec<String> = match was {
            None => Vec::new(),
            // Most lines of a reload: every field as it was, in the same
            // order — one pass, no name looked up in the other line.
            Some((old, _)) if old.parts.iter().eq(desc.parts.iter()) => Vec::new(),
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
/// Call it after moving something that has children. Each entity's matrix
/// is computed once, from its parent's, and a cycle is cut where it closes
/// rather than looped: nothing built from a scene file can contain one, but
/// an editor that sets a parent by hand can, and hanging is a worse answer
/// than a wrong transform.
pub fn apply_hierarchy(world: &mut World) {
    // Snapshotted first, then resolved, then written back. Walking up the
    // tree while holding a query borrow would mean reading the world through
    // the same handle that is iterating it; taking the local transforms out
    // once is both simpler and cheaper than resolving inside the loop.
    //
    // Every entity is a slot, in the order the query visits them. Only an
    // entity something hangs from needs finding by handle, so only those go
    // in a map; each slot's world matrix is computed once and reused by all
    // under it — one product per entity, not one per entity per level.
    let mut is_parent: FastSet<hecs::Entity> = FastSet::default();
    for (_, parent) in world.query::<(hecs::Entity, &Parent)>().iter() {
        is_parent.insert(parent.0);
    }
    let mut slot_of: FastMap<hecs::Entity, u32> = FastMap::default();
    let mut entities: Vec<hecs::Entity> = Vec::new();
    let mut locals: Vec<glam::Mat4> = Vec::new();
    let mut parents: Vec<Option<hecs::Entity>> = Vec::new();
    for (entity, local, parent, between) in world
        .query::<(hecs::Entity, &Transform, Option<&Parent>, Option<&Between>)>()
        .iter()
    {
        if !is_parent.is_empty() && is_parent.contains(&entity) {
            slot_of.insert(entity, locals.len() as u32);
        }
        entities.push(entity);
        // What stands between a thing and its parent — the bone of the
        // parent's skeleton it rides on — goes between.
        locals.push(match between {
            Some(between) => between.0 * local.matrix(),
            None => local.matrix(),
        });
        parents.push(parent.map(|p| p.0));
    }
    // A parent that no longer exists leaves the child where it is rather
    // than dropping it: a despawn should not teleport whatever was attached.
    let up: Vec<Option<u32>> = parents
        .iter()
        .map(|p| p.and_then(|p| slot_of.get(&p).copied()))
        .collect();

    const TODO: u8 = 0;
    const WALKING: u8 = 1;
    const DONE: u8 = 2;
    let mut state = vec![TODO; locals.len()];
    let mut resolved = vec![glam::Mat4::IDENTITY; locals.len()];
    let mut chain: Vec<u32> = Vec::new();
    for start in 0..locals.len() {
        if state[start] == DONE {
            continue;
        }
        if up[start].is_none() {
            // A root, most of a level: its own place.
            resolved[start] = locals[start];
            state[start] = DONE;
            continue;
        }
        // Up to the first ancestor already resolved (or the root), then
        // back down, each slot once.
        chain.clear();
        let mut at = Some(start as u32);
        let mut above = glam::Mat4::IDENTITY;
        while let Some(slot) = at {
            match state[slot as usize] {
                DONE => {
                    above = resolved[slot as usize];
                    break;
                }
                // Nothing built from a scene file can contain a cycle — a
                // tree written as a tree cannot describe one — but an editor
                // setting a parent by hand can. A wrong transform is a better
                // answer than a hang: the cycle is cut where it closes.
                WALKING => break,
                _ => {}
            }
            state[slot as usize] = WALKING;
            chain.push(slot);
            at = up[slot as usize];
        }
        for &slot in chain.iter().rev() {
            above *= locals[slot as usize];
            resolved[slot as usize] = above;
            state[slot as usize] = DONE;
        }
    }

    // Written in place where the entity already has one; inserted (which
    // moves it to another archetype) only the first time. The same set of
    // entities as the snapshot, so the same order; checked, not assumed.
    let mut fresh: Vec<(hecs::Entity, glam::Mat4)> = Vec::new();
    let mut every: Option<FastMap<hecs::Entity, usize>> = None;
    for (slot, (entity, placed)) in world
        .query_mut::<(hecs::Entity, Option<&mut WorldTransform>)>()
        .with::<&Transform>()
        .into_iter()
        .enumerate()
    {
        let matrix = if entities.get(slot) == Some(&entity) {
            resolved[slot]
        } else {
            let every = every.get_or_insert_with(|| entities.iter().enumerate().map(|(i, e)| (*e, i)).collect());
            match every.get(&entity) {
                Some(&at) => resolved[at],
                None => continue,
            }
        };
        match placed {
            Some(placed) => placed.0 = matrix,
            None => fresh.push((entity, matrix)),
        }
    }
    for (entity, matrix) in fresh {
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
        crate::world::take_off::<SceneId>(world, entity);
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
        crate::world::take_off::<Layer>(world, entity);
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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};

    #[test]
    fn a_stepped_thing_is_drawn_between_its_steps_and_what_hangs_from_it_with_it() {
        let mut world = World::new();
        let from = Mat4::from_translation(Vec3::ZERO);
        let to = Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0));
        let body = world.spawn((WorldTransform(to), Stepped { from, to, ahead: false }));
        // A camera half a metre above it.
        let camera = world.spawn((WorldTransform(Mat4::from_translation(Vec3::new(1.0, 0.5, 0.0))), Parent(body)));
        // Jumped five metres and more in a step: drawn where it landed.
        let far = Mat4::from_translation(Vec3::new(9.0, 0.0, 0.0));
        let jumped = world.spawn((WorldTransform(far), Stepped { from, to: far, ahead: false }));
        // Moved since its step by something else: drawn where it is.
        let moved = world.spawn((WorldTransform(Mat4::from_translation(Vec3::Y)), Stepped { from, to, ahead: false }));

        interpolate(&mut world, 0.25);
        fn at(world: &World, e: hecs::Entity) -> Option<Vec3> {
            world.get::<&Shown>(e).ok().map(|s| s.0.w_axis.truncate())
        }
        assert!(at(&world, body).unwrap().abs_diff_eq(Vec3::new(0.25, 0.0, 0.0), 1e-5));
        assert!(at(&world, camera).unwrap().abs_diff_eq(Vec3::new(0.25, 0.5, 0.0), 1e-5));
        assert_eq!(at(&world, jumped), None);
        assert!(drawn(&world, jumped).unwrap().w_axis.truncate().abs_diff_eq(Vec3::new(9.0, 0.0, 0.0), 1e-5));
        assert_eq!(at(&world, moved), None);
        // Ahead: on past where the step left it, as the step moved it.
        let ahead = world.spawn((WorldTransform(to), Stepped { from, to, ahead: true }));
        interpolate(&mut world, 0.25);
        assert!(at(&world, ahead).unwrap().abs_diff_eq(Vec3::new(1.25, 0.0, 0.0), 1e-5));
        let _ = world.despawn(ahead);
        // Put back together by a hierarchy pass, a hair off where the step
        // left it: still the step's own place.
        let _ = world.insert_one(body, WorldTransform(Mat4::from_translation(Vec3::new(1.0 + 1e-6, 0.0, 0.0))));
        interpolate(&mut world, 0.25);
        assert!(at(&world, body).is_some());
        let _ = world.insert_one(body, WorldTransform(to));
        assert_eq!(drawn_at(&WorldTransform(to), None), to);

        // Once it stops being stepped, what was shown goes.
        let _ = world.remove_one::<Stepped>(body);
        interpolate(&mut world, 0.5);
        assert_eq!(at(&world, body), None);
        assert_eq!(at(&world, camera), None);
    }
}
