//! From a scene file to things in a world, and from a world to a draw list.
//!
//! The entity store is `hecs`. Nothing here wraps it or hides it: a game
//! wanting to add a component adds one, and the renderer only asks for the
//! three it needs to draw something.

use std::collections::{HashMap, HashSet};

use hecs::World;

use crate::id::EntityId;
use crate::material::Material;
use crate::render::{Camera, Draw, FogSettings, Frame, Lighting, MeshHandle, TextureHandle};
use crate::scene::{Body, EntityDesc, Scene, Transform};

/// Which mesh an entity draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model(pub MeshHandle);

/// What the entity's surface is made of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface(pub Material);

/// An entity's current pose, as skinning matrices.
///
/// The animation system writes it; the frame builder reads it. Keeping the
/// matrices here rather than a clip and a time means the renderer never has
/// to know what a clip is, and two entities playing the same animation at
/// different times are simply two poses.
#[derive(Debug, Clone, PartialEq)]
pub struct Posed(pub Vec<glam::Mat4>);

/// The image on an entity's surface, already uploaded.
///
/// Separate from [`Surface`] because a material is data a scene can hold and
/// a handle is not: one survives a save, the other is valid only for the
/// renderer that issued it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Textured(pub crate::render::TextureHandle);

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

/// What an entity is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parent(pub hecs::Entity);

/// Kept from the scene so that physics can pick entities up later without the
/// scene having to be re-read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Physics(pub Body);

/// A camera, kept from the scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraLens(pub crate::scene::Lens);

/// The collision layer's name, kept from the scene when not `default`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer(pub String);

/// Friction, bounce and density, kept from the scene when not the default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Props(pub crate::scene::BodyProps);

/// What holds the body to another, kept from the scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Jointed(pub crate::scene::Joint);

/// The shape physics sees, kept from the scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape(pub crate::scene::Collider);

/// What could not be spawned, and why. Returned rather than logged: a missing
/// model is a fact the editor wants to show next to the entity, not a line in
/// a terminal nobody is reading.
#[derive(Debug, Clone, PartialEq)]
pub struct Unresolved {
    pub entity_name: String,
    pub model: String,
}

/// Put a scene into a world.
///
/// `resolve` turns the scene's model name into an uploaded mesh. It is a
/// closure rather than a `&Library` so that the caller decides what a name
/// means — a test can answer with one mesh for everything, and the editor can
/// answer with a placeholder for an asset that failed to import.
pub fn spawn_scene(
    scene: &Scene,
    world: &mut World,
    resolve: impl FnMut(&str) -> Option<MeshHandle>,
) -> Vec<Unresolved> {
    // No palette: named materials fall through to the engine's builtins.
    // That is what the reference scene and every test want, neither of which
    // has a library.
    spawn_scene_with(scene, world, resolve, |_| None)
}

/// Put a scene into a world, resolving named materials through a palette.
///
/// The second closure is what makes a material asset worth having: pass
/// `|name| library.material_by_name(name)` and every scene that says
/// `material: "mossy_stone"` picks up the one asset, so changing it changes
/// every scene at once.
pub fn spawn_scene_with(
    scene: &Scene,
    world: &mut World,
    mut resolve: impl FnMut(&str) -> Option<MeshHandle>,
    palette: impl Fn(&str) -> Option<Material>,
) -> Vec<Unresolved> {
    let mut missing = Vec::new();
    for desc in &scene.entities {
        spawn_subtree(
            desc,
            None,
            glam::Mat4::IDENTITY,
            world,
            &mut resolve,
            &palette,
            &mut missing,
        );
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
    resolve: &mut impl FnMut(&str) -> Option<MeshHandle>,
    palette: &impl Fn(&str) -> Option<Material>,
    missing: &mut Vec<Unresolved>,
) {
    let world_matrix = parent_matrix * desc.transform.matrix();
    let entity = spawn_one(desc, parent, world_matrix, world, resolve, palette, missing);
    for child in &desc.children {
        spawn_subtree(
            child,
            Some(entity),
            world_matrix,
            world,
            resolve,
            palette,
            missing,
        );
    }
}

/// Spawn an entity and everything under it as the game's own rather than
/// the file's: without [`SceneId`], so a reload of the scene never touches
/// it — the file does not know it exists. What a game spawning a prefab at
/// run time builds on. Returns every entity spawned with the line it came
/// from, root first, and the models nothing answered to.
pub fn spawn_owned<'a>(
    desc: &'a EntityDesc,
    parent: Option<hecs::Entity>,
    world: &mut World,
    mut resolve: impl FnMut(&str) -> Option<MeshHandle>,
    palette: impl Fn(&str) -> Option<Material>,
) -> (Vec<(hecs::Entity, &'a EntityDesc)>, Vec<Unresolved>) {
    fn walk<'a>(
        desc: &'a EntityDesc,
        parent: Option<hecs::Entity>,
        parent_matrix: glam::Mat4,
        world: &mut World,
        resolve: &mut impl FnMut(&str) -> Option<MeshHandle>,
        palette: &impl Fn(&str) -> Option<Material>,
        out: &mut (Vec<(hecs::Entity, &'a EntityDesc)>, Vec<Unresolved>),
    ) {
        let matrix = parent_matrix * desc.transform.matrix();
        let entity = spawn_one(desc, parent, matrix, world, resolve, palette, &mut out.1);
        let _ = world.remove_one::<SceneId>(entity);
        out.0.push((entity, desc));
        for child in &desc.children {
            walk(child, Some(entity), matrix, world, resolve, palette, out);
        }
    }
    let parent_matrix = parent
        .and_then(|p| world.get::<&WorldTransform>(p).ok().map(|w| w.0))
        .unwrap_or(glam::Mat4::IDENTITY);
    let mut out = (Vec::new(), Vec::new());
    walk(
        desc,
        parent,
        parent_matrix,
        world,
        &mut resolve,
        &palette,
        &mut out,
    );
    out
}

/// Spawn one entity from its line in the file, without its children.
fn spawn_one(
    desc: &EntityDesc,
    parent: Option<hecs::Entity>,
    world_matrix: glam::Mat4,
    world: &mut World,
    resolve: &mut impl FnMut(&str) -> Option<MeshHandle>,
    palette: &impl Fn(&str) -> Option<Material>,
    missing: &mut Vec<Unresolved>,
) -> hecs::Entity {
    let entity = world.spawn((
        desc.transform,
        WorldTransform(world_matrix),
        Physics(desc.body),
        Shape(desc.collider),
        SceneId(desc.id),
    ));
    if let Some(parent) = parent {
        let _ = world.insert_one(entity, Parent(parent));
    }
    if !desc.joint.is_none() {
        let _ = world.insert_one(entity, Jointed(desc.joint));
    }
    if !desc.physics.is_default() {
        let _ = world.insert_one(entity, Props(desc.physics));
    }
    if !desc.layer.is_empty() {
        let _ = world.insert_one(entity, Layer(desc.layer.clone()));
    }
    if let Some(lens) = desc.camera {
        let _ = world.insert_one(entity, CameraLens(lens));
    }
    dress(desc, entity, world, resolve, palette, missing);
    entity
}

/// Give an entity the mesh and surface its line names, or take them away
/// when the mesh cannot be found.
pub(crate) fn dress(
    desc: &EntityDesc,
    entity: hecs::Entity,
    world: &mut World,
    resolve: &mut impl FnMut(&str) -> Option<MeshHandle>,
    palette: &impl Fn(&str) -> Option<Material>,
    missing: &mut Vec<Unresolved>,
) {
    match resolve(&desc.model) {
        Some(mesh) => {
            let surface = Surface(desc.material_from(palette));
            let _ = world.insert(entity, (Model(mesh), surface));
        }
        None => {
            let _ = world.remove::<(Model, Surface)>(entity);
            missing.push(Unresolved {
                entity_name: desc.name.clone(),
                model: desc.model.clone(),
            });
        }
    }
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

/// Bring a running world from one version of a scene to the next.
///
/// This is what reloading a scene while the game runs means here, and it is
/// the thing an entity store makes cheap: every entity spawned from the file
/// carries its line's [`SceneId`], so the new version is matched to the
/// world line by line, and for each line only the **fields that differ
/// between the two versions** are written. What the file did not change,
/// the world keeps — a door the game swung open stays open when someone
/// recolours the wall beside it; a component the game added is never
/// touched; an entity the game spawned itself is not the file's to remove.
///
/// Lines new in `after` are spawned. Lines `before` had and `after` does
/// not are despawned — only those, so scenes loaded side by side into one
/// world leave each other alone — and
/// with them everything parented to them, game-spawned or not — what hangs off
/// a thing goes with it. A line the world has but `before` lacks is treated
/// as changed in every field.
///
/// Both versions are what [`crate::instantiate`] returns — prefab
/// instances already expanded — so a changed prefab is a changed scene.
pub fn patch_scene(
    before: &Scene,
    after: &Scene,
    world: &mut World,
    mut resolve: impl FnMut(&str) -> Option<MeshHandle>,
    palette: impl Fn(&str) -> Option<Material>,
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
        patch.subtree(
            desc,
            None,
            glam::Mat4::IDENTITY,
            world,
            &mut resolve,
            &palette,
        );
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
        resolve: &mut impl FnMut(&str) -> Option<MeshHandle>,
        palette: &impl Fn(&str) -> Option<Material>,
    ) {
        self.kept.insert(desc.id);
        let world_matrix = parent_matrix * desc.transform.matrix();
        let entity = match self.live.get(&desc.id) {
            Some(&entity) if world.contains(entity) => {
                let was = self.old.get(&desc.id).copied();
                if self.update(desc, was, parent, entity, world, resolve, palette) {
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
                    resolve,
                    palette,
                    &mut self.out.missing,
                )
            }
        };
        for child in &desc.children {
            self.subtree(
                child,
                Some((desc.id, entity)),
                world_matrix,
                world,
                resolve,
                palette,
            );
        }
    }

    /// Write the fields of one line that differ from its previous version.
    /// Returns whether anything did.
    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        desc: &EntityDesc,
        was: Option<(&EntityDesc, Option<EntityId>)>,
        parent: Option<(EntityId, hecs::Entity)>,
        entity: hecs::Entity,
        world: &mut World,
        resolve: &mut impl FnMut(&str) -> Option<MeshHandle>,
        palette: &impl Fn(&str) -> Option<Material>,
    ) -> bool {
        let mut changed = false;
        if was.is_none_or(|(old, _)| old.transform != desc.transform) {
            let _ = world.insert_one(entity, desc.transform);
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.model != desc.model || old.material != desc.material) {
            dress(desc, entity, world, resolve, palette, &mut self.out.missing);
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.body != desc.body) {
            let _ = world.insert_one(entity, Physics(desc.body));
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.collider != desc.collider) {
            let _ = world.insert_one(entity, Shape(desc.collider));
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.camera != desc.camera) {
            match desc.camera {
                Some(lens) => {
                    let _ = world.insert_one(entity, CameraLens(lens));
                }
                None => {
                    let _ = world.remove_one::<CameraLens>(entity);
                }
            }
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.layer != desc.layer) {
            if desc.layer.is_empty() {
                let _ = world.remove_one::<Layer>(entity);
            } else {
                let _ = world.insert_one(entity, Layer(desc.layer.clone()));
            }
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.physics != desc.physics) {
            if desc.physics.is_default() {
                let _ = world.remove_one::<Props>(entity);
            } else {
                let _ = world.insert_one(entity, Props(desc.physics));
            }
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.joint != desc.joint) {
            if desc.joint.is_none() {
                let _ = world.remove_one::<Jointed>(entity);
            } else {
                let _ = world.insert_one(entity, Jointed(desc.joint));
            }
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

/// The lighting a scene's sun describes.
///
/// One place, so the headless render, the editor and the walk-around light
/// the same scene the same way. They did not: two of them ignored the hour
/// entirely and the third had its own curve, and the difference only showed
/// up when a screenshot was compared with what the editor was showing.
pub fn scene_lighting(sun: &crate::scene::Sun) -> Lighting {
    Lighting {
        sun_direction: sun.direction(),
        sun_color: sun.color(),
        sun_intensity: sun.intensity,
        ..Lighting::default()
    }
}

/// The fog a scene describes.
pub fn scene_fog(fog: &crate::scene::Fog) -> FogSettings {
    FogSettings {
        color: glam::Vec3::from_array(fog.color),
        start: fog.start,
        end: fog.end,
    }
}

/// The camera a scene's view describes.
///
/// Here rather than on either type: a scene is not allowed to know about the
/// renderer, and the renderer is not allowed to know about scene files. This
/// module is where the two already meet.
pub fn scene_camera(view: &crate::scene::View) -> Camera {
    Camera {
        position: view.position,
        target: view.target,
        fov_y_degrees: view.fov_deg,
        ..Camera::default()
    }
}

/// What the world's camera sees: the entity with a [`CameraLens`] of the
/// highest priority (the lowest id among equals, so the answer does not
/// change between runs), from where it is and along its +z. `None` when no
/// entity has one — the game falls back to the scene's `view`.
pub fn camera_of(world: &World) -> Option<Camera> {
    let mut best: Option<(i32, std::cmp::Reverse<crate::id::EntityId>, Camera)> = None;
    for (lens, placed, id) in world
        .query::<(&CameraLens, &WorldTransform, Option<&SceneId>)>()
        .iter()
    {
        let (_, rotation, position) = placed.0.to_scale_rotation_translation();
        let camera = Camera {
            position,
            target: position + rotation * glam::Vec3::Z,
            up: rotation * glam::Vec3::Y,
            fov_y_degrees: lens.0.fov_deg,
            ortho: lens.0.ortho,
            ..Camera::default()
        };
        let key = (
            lens.0.priority,
            std::cmp::Reverse(id.map(|i| i.0).unwrap_or_default()),
        );
        if best.as_ref().is_none_or(|(p, i, _)| key > (*p, *i)) {
            best = Some((key.0, key.1, camera));
        }
    }
    best.map(|(_, _, camera)| camera)
}

/// The view to write back into a scene for a camera.
pub fn captured_view(camera: &Camera) -> crate::scene::View {
    crate::scene::View {
        position: camera.position,
        target: camera.target,
        fov_deg: camera.fov_y_degrees,
    }
}

/// Collect everything drawable in the world into a frame.
pub fn build_frame(world: &World, camera: Camera, lighting: Lighting, fog: FogSettings) -> Frame {
    build_frame_where(world, camera, lighting, fog, |_| true)
}

/// [`build_frame`] with only what `keep` says yes to, by the scene line an
/// entity came from (`None` for one spawned by code): the editor's hidden
/// and isolated entities, left out of the frame and nowhere else.
pub fn build_frame_where(
    world: &World,
    camera: Camera,
    lighting: Lighting,
    fog: FogSettings,
    keep: impl Fn(Option<crate::id::EntityId>) -> bool,
) -> Frame {
    let mut draws = Vec::new();
    let mut poses: Vec<crate::render::Pose> = Vec::new();
    for (placed, model, surface, textured, posed, line) in world
        .query::<(
            &WorldTransform,
            &Model,
            &Surface,
            Option<&Textured>,
            Option<&Posed>,
            Option<&SceneId>,
        )>()
        .iter()
    {
        if !keep(line.map(|l| l.0)) {
            continue;
        }
        let pose = posed.map(|p| {
            poses.push(crate::render::Pose(p.0.clone()));
            poses.len() as u32 - 1
        });
        draws.push(Draw {
            mesh: model.0,
            transform: placed.0,
            texture: textured.map(|t| t.0).unwrap_or(TextureHandle::WHITE),
            material: surface.0,
            pose,
        });
    }
    Frame {
        camera,
        lighting,
        clear_color: fog.color,
        fog,
        shadows: crate::render::ShadowSettings::default(),
        draws,
        overlay_draws: Vec::new(),
        poses,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    /// An entity with nothing set, for `..blank()` in the tests below.
    fn blank() -> EntityDesc {
        EntityDesc {
            camera: None,
            layer: Default::default(),
            physics: Default::default(),
            joint: Default::default(),
            overrides: Default::default(),
            components: Default::default(),
            id: Default::default(),
            name: String::new(),
            model: "m".into(),
            prefab: String::new(),
            transform: Transform::default(),
            material: Default::default(),
            body: Body::None,
            collider: crate::scene::Collider::None,
            children: Vec::new(),
        }
    }

    fn scene_with(models: &[&str]) -> Scene {
        Scene {
            entities: models
                .iter()
                .enumerate()
                .map(|(i, model)| EntityDesc {
                    camera: None,
                    layer: Default::default(),
                    physics: Default::default(),
                    joint: Default::default(),
                    overrides: Default::default(),
                    components: Default::default(),
                    id: Default::default(),
                    name: format!("thing {i}"),
                    model: (*model).into(),
                    prefab: String::new(),
                    transform: Transform {
                        position: Vec3::new(i as f32, 0.0, 0.0),
                        ..Default::default()
                    },
                    material: Default::default(),
                    body: Body::Static,
                    collider: crate::scene::Collider::None,
                    children: Vec::new(),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_scene_becomes_entities_and_then_a_draw_list() {
        let scene = scene_with(&["pine", "pine", "rock"]);
        let mut world = World::new();
        let missing = spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        assert!(missing.is_empty());

        let frame = build_frame(
            &world,
            Camera::default(),
            Lighting::default(),
            FogSettings::default(),
        );
        assert_eq!(frame.draws.len(), 3);
        // The clear colour is the fog colour, so the horizon and the far
        // distance meet instead of showing a seam.
        assert_eq!(frame.clear_color, frame.fog.color);
    }

    #[test]
    fn a_child_is_placed_relative_to_its_parent() {
        // A cart at x = 10 with a wheel at x = 1 puts the wheel at 11, and
        // that is the whole point of a hierarchy: place the cart, and the
        // wheel comes along.
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "cart".into(),
                model: "m".into(),
                transform: Transform {
                    position: Vec3::new(10.0, 0.0, 0.0),
                    ..Default::default()
                },
                children: vec![EntityDesc {
                    name: "wheel".into(),
                    model: "m".into(),
                    transform: Transform {
                        position: Vec3::new(1.0, 0.0, 0.0),
                        ..Default::default()
                    },
                    ..blank()
                }],
                ..blank()
            }],
            ..Default::default()
        };
        let mut world = World::new();
        spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));

        let mut positions: Vec<f32> = world
            .query::<&WorldTransform>()
            .iter()
            .map(|placed| placed.0.w_axis.x)
            .collect();
        positions.sort_by(f32::total_cmp);
        assert_eq!(positions, vec![10.0, 11.0]);
    }

    #[test]
    fn a_parents_rotation_carries_its_children_around_with_it() {
        // Translation alone would pass even if the child's matrix were being
        // added rather than multiplied. A quarter turn tells them apart.
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "turntable".into(),
                model: "m".into(),
                transform: Transform {
                    rotation_deg: Vec3::new(0.0, 90.0, 0.0),
                    ..Default::default()
                },
                children: vec![EntityDesc {
                    name: "arm".into(),
                    model: "m".into(),
                    transform: Transform {
                        position: Vec3::new(2.0, 0.0, 0.0),
                        ..Default::default()
                    },
                    ..blank()
                }],
                ..blank()
            }],
            ..Default::default()
        };
        let mut world = World::new();
        spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));

        // +X rotated 90° about Y lands on -Z in a right-handed system.
        let arm = world
            .query::<&WorldTransform>()
            .iter()
            .map(|placed| placed.0.w_axis.truncate())
            .find(|p| p.length() > 0.5)
            .expect("the arm is offset from its parent");
        assert!(arm.x.abs() < 1e-4, "expected the arm off the X axis: {arm}");
        assert!((arm.z + 2.0).abs() < 1e-4, "expected z = -2, got {arm}");
    }

    #[test]
    fn moving_a_parent_moves_everything_under_it() {
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "root".into(),
                model: "m".into(),
                children: vec![EntityDesc {
                    name: "child".into(),
                    model: "m".into(),
                    transform: Transform {
                        position: Vec3::new(0.0, 3.0, 0.0),
                        ..Default::default()
                    },
                    children: vec![EntityDesc {
                        name: "grandchild".into(),
                        model: "m".into(),
                        transform: Transform {
                            position: Vec3::new(0.0, 3.0, 0.0),
                            ..Default::default()
                        },
                        ..blank()
                    }],
                    ..blank()
                }],
                ..blank()
            }],
            ..Default::default()
        };
        let mut world = World::new();
        spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));

        // Shove the root sideways and re-resolve.
        let root = world
            .query::<(hecs::Entity, &Transform)>()
            .iter()
            .find(|(_, t)| t.position == Vec3::ZERO)
            .map(|(e, _)| e)
            .expect("the root sits at the origin");
        world.get::<&mut Transform>(root).unwrap().position = Vec3::new(5.0, 0.0, 0.0);
        apply_hierarchy(&mut world);

        let mut heights: Vec<(f32, f32)> = world
            .query::<&WorldTransform>()
            .iter()
            .map(|placed| (placed.0.w_axis.x, placed.0.w_axis.y))
            .collect();
        heights.sort_by(|a, b| a.1.total_cmp(&b.1));
        assert_eq!(heights, vec![(5.0, 0.0), (5.0, 3.0), (5.0, 6.0)]);
    }

    #[test]
    fn a_child_whose_model_is_missing_keeps_its_own_children_in_place() {
        // Dropping the branch would move the grandchild, which is a worse
        // failure than a hole where one model should be.
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "root".into(),
                model: "m".into(),
                children: vec![EntityDesc {
                    name: "broken".into(),
                    model: "missing".into(),
                    transform: Transform {
                        position: Vec3::new(4.0, 0.0, 0.0),
                        ..Default::default()
                    },
                    children: vec![EntityDesc {
                        name: "fine".into(),
                        model: "m".into(),
                        transform: Transform {
                            position: Vec3::new(1.0, 0.0, 0.0),
                            ..Default::default()
                        },
                        ..blank()
                    }],
                    ..blank()
                }],
                ..blank()
            }],
            ..Default::default()
        };
        let mut world = World::new();
        let missing = spawn_scene(&scene, &mut world, |name| {
            (name != "missing").then_some(MeshHandle::TEST)
        });
        assert_eq!(missing.len(), 1);

        let drawn: Vec<f32> = world
            .query::<(&WorldTransform, &Model)>()
            .iter()
            .map(|(placed, _)| placed.0.w_axis.x)
            .collect();
        assert!(
            drawn.contains(&5.0),
            "the grandchild should still stand at 4 + 1, got {drawn:?}"
        );
    }

    #[test]
    fn a_model_that_cannot_be_resolved_is_reported_not_skipped_in_silence() {
        let scene = scene_with(&["pine", "missing"]);
        let mut world = World::new();
        let missing = spawn_scene(&scene, &mut world, |name| {
            (name != "missing").then_some(MeshHandle::TEST)
        });
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].model, "missing");
        assert_eq!(missing[0].entity_name, "thing 1");

        // The entity still exists — it just has nothing to draw. Dropping it
        // would move anything parented to it, and a hole where one model
        // should be is a smaller failure than a subtree that silently moved.
        assert_eq!(world.len(), 2);
        assert_eq!(world.query::<&Model>().iter().count(), 1);
    }

    // --- live reload ------------------------------------------------------

    fn scene(text: &str) -> Scene {
        let mut scene: Scene = ron::from_str(text).unwrap();
        scene.assign_ids();
        scene
    }

    fn spawned(scene: &Scene) -> World {
        let mut world = World::new();
        spawn_scene(scene, &mut world, |_| Some(MeshHandle::TEST));
        world
    }

    fn entity(world: &World, id: &str) -> hecs::Entity {
        let id: EntityId = id.parse().unwrap();
        world
            .query::<(hecs::Entity, &SceneId)>()
            .iter()
            .find(|(_, s)| s.0 == id)
            .map(|(e, _)| e)
            .unwrap_or_else(|| panic!("no entity {id}"))
    }

    fn patch(before: &Scene, after: &Scene, world: &mut World) -> Patched {
        patch_scene(before, after, world, |_| Some(MeshHandle::TEST), |_| None)
    }

    /// Something the game put on an entity, which no file knows about.
    #[derive(Debug, PartialEq)]
    struct Health(u32);

    const DOOR: &str = r#"(entities: [
        (id: "d", name: "door", model: "m", transform: (position: (0.0, 0.0, 0.0))),
        (id: "a1", name: "wall", model: "m", material: "stone"),
    ])"#;

    #[test]
    fn a_reload_writes_only_what_the_file_changed() {
        let before = scene(DOOR);
        let mut world = spawned(&before);
        let door = entity(&world, "d");
        // The game swings the door open and gives it a component of its own.
        world
            .insert_one(
                door,
                Transform {
                    position: Vec3::X,
                    ..Transform::default()
                },
            )
            .unwrap();
        world.insert_one(door, Health(3)).unwrap();

        // Someone recolours the wall.
        let after = scene(&DOOR.replace(r#"material: "stone""#, r#"material: "bark""#));
        let done = patch(&before, &after, &mut world);
        assert_eq!(
            (done.spawned, done.updated, done.despawned),
            (0, 1, 0),
            "{done:?}"
        );

        assert_eq!(
            world.get::<&Transform>(door).unwrap().position,
            Vec3::X,
            "the door stays where the game put it"
        );
        assert_eq!(*world.get::<&Health>(door).unwrap(), Health(3));
        let wall = entity(&world, "a1");
        assert_eq!(
            world.get::<&Surface>(wall).unwrap().0,
            crate::material::builtin::BARK,
            "and the wall has its new colour"
        );
    }

    #[test]
    fn a_field_the_file_changed_wins_over_the_game() {
        // The file is the source: editing the door's position means you want
        // it there, whatever the game had done with it.
        let before = scene(DOOR);
        let mut world = spawned(&before);
        let door = entity(&world, "d");
        world
            .insert_one(
                door,
                Transform {
                    position: Vec3::X,
                    ..Transform::default()
                },
            )
            .unwrap();

        let after = scene(&DOOR.replace("(0.0, 0.0, 0.0)", "(0.0, 0.0, 5.0)"));
        patch(&before, &after, &mut world);
        assert_eq!(
            world.get::<&Transform>(door).unwrap().position,
            Vec3::new(0.0, 0.0, 5.0)
        );
        assert_eq!(
            world.get::<&WorldTransform>(door).unwrap().0.w_axis.z,
            5.0,
            "and its world transform follows"
        );
    }

    #[test]
    fn lines_added_and_removed_are_spawned_and_despawned() {
        let before = scene(DOOR);
        let mut world = spawned(&before);
        // The game hangs a torch on the wall, and spawns a bird of its own.
        let wall = entity(&world, "a1");
        let torch = world.spawn((Transform::default(), Parent(wall)));
        let bird = world.spawn((Transform::default(),));

        let after = scene(
            r#"(entities: [
                (id: "d", name: "door", model: "m", transform: (position: (0.0, 0.0, 0.0))),
                (id: "b2", name: "roof", model: "m"),
            ])"#,
        );
        let done = patch(&before, &after, &mut world);
        assert_eq!((done.spawned, done.despawned), (1, 2), "{done:?}");
        assert!(!world.contains(wall));
        assert!(
            !world.contains(torch),
            "what hung off the wall goes with it"
        );
        assert!(world.contains(bird), "what the game made on its own stays");
        entity(&world, "b2");
    }

    #[test]
    fn a_reparented_line_moves_under_its_new_parent() {
        let before = scene(
            r#"(entities: [(id: "a", name: "a", model: "m", transform: (position: (10.0, 0.0, 0.0))), (id: "b", name: "b", model: "m")])"#,
        );
        let mut world = spawned(&before);
        let after = scene(
            r#"(entities: [(id: "a", name: "a", model: "m", transform: (position: (10.0, 0.0, 0.0)), children: [(id: "b", name: "b", model: "m")])])"#,
        );
        let done = patch(&before, &after, &mut world);
        assert_eq!(done.updated, 1, "{done:?}");
        let (a, b) = (entity(&world, "a"), entity(&world, "b"));
        assert_eq!(world.get::<&Parent>(b).unwrap().0, a);
        assert_eq!(world.get::<&WorldTransform>(b).unwrap().0.w_axis.x, 10.0);
    }

    #[test]
    fn the_same_file_twice_changes_nothing() {
        let before = scene(DOOR);
        let mut world = spawned(&before);
        assert!(patch(&before, &before.clone(), &mut world).is_empty());
    }

    #[test]
    fn a_model_nothing_answers_to_is_reported_and_leaves_nothing_drawn() {
        let before = scene(DOOR);
        let mut world = spawned(&before);
        let after = scene(&DOOR.replace(
            r#"name: "door", model: "m""#,
            r#"name: "door", model: "gone""#,
        ));
        let done = patch_scene(
            &before,
            &after,
            &mut world,
            |name| (name == "m").then_some(MeshHandle::TEST),
            |_| None,
        );
        assert_eq!(done.missing.len(), 1);
        assert_eq!(done.missing[0].model, "gone");
        assert!(world.get::<&Model>(entity(&world, "d")).is_err());
    }

    #[test]
    fn two_scenes_in_one_world_leave_each_other_alone() {
        let village = scene(DOOR);
        let forest = scene(r#"(entities: [(id: "f1", name: "oak", model: "m")])"#);
        let mut world = spawned(&village);
        spawn_scene(&forest, &mut world, |_| Some(MeshHandle::TEST));

        // The village reloads with the wall gone: the forest is not its.
        let smaller = scene(r#"(entities: [(id: "d", name: "door", model: "m")])"#);
        let done = patch(&village, &smaller, &mut world);
        assert_eq!(done.despawned, 1, "{done:?}");
        entity(&world, "f1");

        // And the village unloads alone.
        let done = patch(&smaller, &Scene::default(), &mut world);
        assert_eq!(done.despawned, 1);
        entity(&world, "f1");
    }

    #[test]
    fn a_camera_on_the_player_sees_from_its_eyes_and_follows_it() {
        let mut scene: Scene = ron::from_str(
            r#"(entities: [
                (id: "00000000000000a1", name: "player", model: "builtin:cube",
                 transform: (position: (3.0, 0.0, 0.0), rotation_deg: (0.0, 90.0, 0.0)),
                 children: [(id: "00000000000000a2", name: "eyes", transform: (position: (0.0, 1.6, 0.0)),
                             camera: (fov_deg: 70.0))]),
            ])"#,
        )
        .unwrap();
        scene.assign_ids();
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        let camera = camera_of(&world).expect("a camera");
        assert!(
            (camera.position - Vec3::new(3.0, 1.6, 0.0)).length() < 1e-4,
            "{:?}",
            camera.position
        );
        let looking = (camera.target - camera.position).normalize();
        assert!(
            (looking - Vec3::X).length() < 1e-4,
            "where the player faces: {looking:?}"
        );
        assert_eq!(camera.fov_y_degrees, 70.0);

        let player = world
            .query::<(hecs::Entity, &SceneId)>()
            .iter()
            .find(|(_, s)| s.0 == scene.entities[0].id)
            .map(|(e, _)| e)
            .unwrap();
        world.get::<&mut Transform>(player).unwrap().position.z = 10.0;
        apply_hierarchy(&mut world);
        assert!(
            (camera_of(&world).unwrap().position.z - 10.0).abs() < 1e-4,
            "it follows"
        );

        // A second camera that asks for it wins.
        world.spawn((
            CameraLens(crate::scene::Lens {
                fov_deg: 40.0,
                priority: 1,
                ortho: Some(8.0),
            }),
            WorldTransform(glam::Mat4::IDENTITY),
        ));
        assert_eq!(camera_of(&world).unwrap().fov_y_degrees, 40.0);
        assert_eq!(camera_of(&world).unwrap().ortho, Some(8.0));
        assert!(camera_of(&World::new()).is_none());
    }
}
