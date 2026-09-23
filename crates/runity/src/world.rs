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

/// Held by a joint of the parent's skeleton, by the joint's name: from a
/// line's `bone`. [`apply_hierarchy`] places it where the parent's pose
/// puts that joint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnBone(pub String);

/// Kept from the scene so that physics can pick entities up later without the
/// scene having to be re-read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Physics(pub Body);

/// A camera, kept from the scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraLens(pub crate::scene::Lens);

/// A light at an entity, from its line's `light`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightSource(pub crate::scene::Light);

/// Grass bends round an entity within this many metres: its line's
/// `bends_grass`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BendsGrass(pub f32);

/// A decal pressed from an entity: its line's `decal`, and the material it
/// presses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pressing(pub crate::scene::Decal, pub Material);

/// A screen on a thing in the world — a shop terminal, a radio's dial —
/// that the player works by aiming at it: widgets drawn into [`Self::ui`]
/// with [`Self::pointer`] as their input, as on the screen; the thing's
/// model (a flat `builtin:plane`, its top the screen) shows the picture.
/// Each frame: [`WorldUi::aim`] with the crosshair's ray and whether
/// "use" went down or up, then clear [`Self::ui`] and draw the widgets.
#[derive(Debug, Clone)]
pub struct WorldUi {
    /// The picture's name: a material elsewhere can show it too, as
    /// `render:<name>`.
    pub name: String,
    /// Pixels across and down.
    pub size: (u32, u32),
    pub background: glam::Vec4,
    pub ui: crate::ui::Ui,
    /// The crosshair, as a mouse on the picture.
    pub pointer: crate::input::Input,
}

impl WorldUi {
    pub fn new(name: &str, size: (u32, u32)) -> Self {
        Self {
            name: name.to_string(),
            size,
            background: glam::Vec4::new(0.05, 0.06, 0.07, 1.0),
            ui: Default::default(),
            pointer: Default::default(),
        }
    }

    /// Where a ray from `origin` along `direction` meets this screen,
    /// placed at `placed` (a `builtin:plane`: a metre square, facing up),
    /// in its pixels from the top left; `None` when it misses or comes
    /// from behind.
    pub fn hit(
        &self,
        placed: glam::Mat4,
        origin: glam::Vec3,
        direction: glam::Vec3,
    ) -> Option<glam::Vec2> {
        let inverse = placed.inverse();
        let o = inverse.transform_point3(origin);
        let d = inverse.transform_vector3(direction);
        if d.y >= -1e-6 || o.y <= 0.0 {
            return None;
        }
        let t = -o.y / d.y;
        let at = o + d * t;
        let (u, v) = (at.x + 0.5, at.z + 0.5);
        ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v))
            .then(|| glam::Vec2::new(u * self.size.0 as f32, v * self.size.1 as f32))
    }

    /// This frame's pointer: at `at` (from [`Self::hit`]) or off the
    /// screen, the button going `down` or `up` — "use" pressed and
    /// released while aiming.
    pub fn aim(&mut self, at: Option<glam::Vec2>, down: bool, up: bool) {
        use crate::input::{InputEvent, MouseButton};
        self.pointer.begin_frame();
        let at = at.unwrap_or(glam::Vec2::splat(-1e4));
        self.pointer
            .handle(&InputEvent::MouseMoved { x: at.x, y: at.y });
        if down {
            self.pointer
                .handle(&InputEvent::MouseDown(MouseButton::Left));
        }
        if up {
            self.pointer.handle(&InputEvent::MouseUp(MouseButton::Left));
        }
    }
}

/// A camera drawing into a picture, from its line's `render_texture`.
#[derive(Debug, Clone, PartialEq)]
pub struct ToTexture(pub crate::scene::RenderTexture);

/// A camera drawing into a picture: where it looks from, which picture,
/// and the mirror's plane (a point on it and the way it faces) if it is one.
type PictureCamera = (
    Camera,
    crate::scene::RenderTexture,
    Option<(glam::Vec3, glam::Vec3)>,
);

/// Every camera that draws into a picture, as a frame of its own: what
/// [`scene_frame`] puts on a frame for materials to show.
pub fn texture_views(
    world: &World,
    scene: &crate::scene::Scene,
    main: Camera,
) -> Vec<crate::render::TextureView> {
    // A mirror's camera is the screen's, reflected in its plane; anything
    // else's is its own lens.
    let cameras: Vec<PictureCamera> = world
        .query::<(Option<&CameraLens>, &WorldTransform, &ToTexture)>()
        .iter()
        .filter_map(|(lens, placed, picture)| {
            if picture.0.mirror {
                let (_, turn, at) = placed.0.to_scale_rotation_translation();
                let normal = (turn * glam::Vec3::Y).normalize();
                Some((
                    reflected(main, at, normal),
                    picture.0.clone(),
                    Some((at, normal)),
                ))
            } else {
                lens.map(|l| (lens_camera(l.0, placed.0), picture.0.clone(), None))
            }
        })
        .collect();
    cameras
        .into_iter()
        .map(|(camera, picture, plane)| {
            let mut hidden: std::collections::HashSet<crate::id::EntityId> = world
                .query::<(&Layer, &SceneId)>()
                .iter()
                .filter(|(layer, _)| picture.hide.contains(&layer.0))
                .map(|(_, id)| id.0)
                .collect();
            // What is behind a mirror is not in it.
            if let Some((at, normal)) = plane {
                hidden.extend(
                    world
                        .query::<(&WorldTransform, &SceneId)>()
                        .iter()
                        .filter(|(p, _)| (p.0.w_axis.truncate() - at).dot(normal) < -0.05)
                        .map(|(_, id)| id.0),
                );
            }
            let mut frame = build_frame_where(
                world,
                camera,
                scene_lighting(&scene.sun),
                scene_fog(&scene.fog),
                |line| line.is_none_or(|id| !hidden.contains(&id)),
            );
            scene_look(&mut frame, scene);
            frame.post.motion_blur = Default::default();
            crate::render::TextureView {
                id: crate::asset::AssetId::render_target(&picture.name),
                frame: Box::new(frame),
            }
        })
        .collect()
}

/// A mesh the game rewrites as it goes — the water's surface, a rope
/// between two hands — drawn at the entity with its material. Changing it
/// ([`LiveMesh::set`]) uploads it again on the next frame; leaving it
/// alone costs nothing more than any other mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveMesh {
    vertices: std::sync::Arc<Vec<crate::asset::Vertex>>,
    indices: std::sync::Arc<Vec<u32>>,
    version: u64,
}

impl LiveMesh {
    pub fn new(vertices: Vec<crate::asset::Vertex>, indices: Vec<u32>) -> Self {
        Self {
            vertices: std::sync::Arc::new(vertices),
            indices: std::sync::Arc::new(indices),
            version: 0,
        }
    }

    /// New vertices and triangles.
    pub fn set(&mut self, vertices: Vec<crate::asset::Vertex>, indices: Vec<u32>) {
        self.vertices = std::sync::Arc::new(vertices);
        self.indices = std::sync::Arc::new(indices);
        self.version += 1;
    }

    /// Move the vertices where `to` says, keeping the triangles, and turn
    /// each normal to the triangles round it: a surface pushed about by a
    /// wave is lit as the wave.
    pub fn move_vertices(&mut self, to: impl Fn(usize, glam::Vec3) -> glam::Vec3) {
        let mut vertices = (*self.vertices).clone();
        for (i, v) in vertices.iter_mut().enumerate() {
            v.position = to(i, glam::Vec3::from_array(v.position)).to_array();
        }
        let mut normals = vec![glam::Vec3::ZERO; vertices.len()];
        for t in self.indices.chunks_exact(3) {
            let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
            let at = |i: usize| glam::Vec3::from_array(vertices[i].position);
            let n = (at(b) - at(a)).cross(at(c) - at(a));
            for i in [a, b, c] {
                normals[i] += n;
            }
        }
        for (v, n) in vertices.iter_mut().zip(normals) {
            v.normal = n.normalize_or(glam::Vec3::Y).to_array();
        }
        self.vertices = std::sync::Arc::new(vertices);
        self.version += 1;
    }

    pub fn vertices(&self) -> &[crate::asset::Vertex] {
        &self.vertices
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }
}

/// A local look at an entity, from its line's `post_volume`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PostVolumeBox(pub crate::scene::PostVolume);

/// Lay the world's post volumes over a frame's post-processing, by where
/// its camera is: all of one inside its box, fading out over its
/// `blend_distance` outside, lower priorities first.
pub fn post_volumes(frame: &mut Frame, world: &World) {
    let eye = frame.camera.position;
    let mut found: Vec<(i32, f32, crate::post::PostProcess)> = world
        .query::<(&PostVolumeBox, &WorldTransform)>()
        .iter()
        .filter_map(|(volume, placed)| {
            let v = volume.0;
            // Into the box's own axes, where it is a unit-scaled box.
            let local = placed.0.inverse().transform_point3(eye);
            let (scale, _, _) = placed.0.to_scale_rotation_translation();
            let outside = ((local.abs() - v.size * 0.5).max(glam::Vec3::ZERO)) * scale;
            let distance = outside.length();
            let weight = if distance <= 0.0 {
                1.0
            } else if v.blend_distance <= 0.0 {
                0.0
            } else {
                (1.0 - distance / v.blend_distance).max(0.0)
            };
            (weight > 0.0).then_some((v.priority, weight, v.post))
        })
        .collect();
    found.sort_by_key(|(priority, _, _)| *priority);
    for (_, weight, post) in found {
        frame.post = frame.post.lerp(&post, weight);
    }
}

/// A reflection probe at an entity, from its line's `reflection_probe`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProbeBox(pub crate::scene::Probe);

/// The collision layer's name, kept from the scene when not `default`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer(pub String);

/// Friction, bounce and density, kept from the scene when not the default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Props(pub crate::scene::BodyProps);

/// What holds the body to another, kept from the scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Jointed(pub crate::scene::Joint);

/// How hard its joint may be pulled before it breaks, in newtons.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointBreak(pub f32);

/// Its joint broke: it is not built again until the entity's joint is set
/// anew (remove this to mend it).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct JointBroken;

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
    resolve: impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
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
    mut resolve: impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: impl Fn(&crate::AssetLink) -> Option<Material>,
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
    resolve: &mut impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: &impl Fn(&crate::AssetLink) -> Option<Material>,
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
    mut resolve: impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: impl Fn(&crate::AssetLink) -> Option<Material>,
) -> (Vec<(hecs::Entity, &'a EntityDesc)>, Vec<Unresolved>) {
    fn walk<'a>(
        desc: &'a EntityDesc,
        parent: Option<hecs::Entity>,
        parent_matrix: glam::Mat4,
        world: &mut World,
        resolve: &mut impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
        palette: &impl Fn(&crate::AssetLink) -> Option<Material>,
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
    resolve: &mut impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: &impl Fn(&crate::AssetLink) -> Option<Material>,
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
    if let Some(force) = desc.joint_break {
        let _ = world.insert_one(entity, JointBreak(force));
    }
    if !desc.bone.is_empty() {
        let _ = world.insert_one(entity, OnBone(desc.bone.clone()));
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
    if let Some(light) = desc.light {
        let _ = world.insert_one(entity, LightSource(light));
    }
    if let Some(probe) = desc.reflection_probe {
        let _ = world.insert_one(entity, ProbeBox(probe));
    }
    if desc.bends_grass > 0.0 {
        let _ = world.insert_one(entity, BendsGrass(desc.bends_grass));
    }
    if let Some(prints) = desc.footprints {
        let _ = world.insert_one(entity, crate::footprints::Trail::new(prints));
    }
    if let Some(volume) = desc.post_volume {
        let _ = world.insert_one(entity, PostVolumeBox(volume));
    }
    if let Some(picture) = &desc.render_texture {
        let _ = world.insert_one(entity, ToTexture(picture.clone()));
    }
    if let Some(route) = &desc.route {
        let _ = world.insert_one(
            entity,
            crate::routes::Travelling::new(route.clone(), desc.transform.position),
        );
    }
    if let Some(emitter) = &desc.particles {
        if let Some(emitting) = emitting(emitter, resolve, palette) {
            let _ = world.insert_one(entity, emitting);
        }
    }
    dress(desc, entity, world, resolve, palette, missing);
    entity
}

/// Give an entity the mesh and surface its line names, or take them away
/// when the mesh cannot be found.
/// An emitter ready to run: what its particles are drawn as (its model, or
/// a small cube) and with (its material, or the plain colour).
fn emitting(
    emitter: &crate::scene::Emitter,
    resolve: &mut impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: &impl Fn(&crate::AssetLink) -> Option<Material>,
) -> Option<crate::particles::Emitting> {
    let cube = crate::AssetLink::named(if emitter.facing {
        "builtin:plane"
    } else {
        "builtin:cube"
    });
    let model = if emitter.model.is_empty() {
        &cube
    } else {
        &emitter.model
    };
    let mesh = resolve(model).or_else(|| resolve(&cube))?;
    let mut out = crate::particles::Emitting::new(emitter.clone(), mesh);
    out.material = emitter.material.as_ref().and_then(palette);
    Some(out)
}

pub(crate) fn dress(
    desc: &EntityDesc,
    entity: hecs::Entity,
    world: &mut World,
    resolve: &mut impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: &impl Fn(&crate::AssetLink) -> Option<Material>,
    missing: &mut Vec<Unresolved>,
) {
    match desc.decal {
        Some(decal) => {
            let _ = world.insert_one(entity, Pressing(decal, desc.material_from(palette)));
        }
        None => {
            let _ = world.remove_one::<Pressing>(entity);
        }
    }
    // Ground made from its numbers: its mesh comes when it is first drawn
    // (`terrain::upload_terrains`), and again only if they changed.
    if let Some(terrain) = desc.terrain {
        let same = world
            .get::<&crate::terrain::Relief>(entity)
            .is_ok_and(|r| r.terrain == terrain);
        if !same {
            let _ = world.remove_one::<Model>(entity);
            let _ = world.insert_one(entity, crate::terrain::Relief::new(terrain));
        }
        let _ = world.insert_one(entity, Surface(desc.material_from(palette)));
        return;
    }
    let _ = world.remove_one::<crate::terrain::Relief>(entity);
    // No model is nothing to draw — a probe, a decal, a light, an empty to
    // hang children on — not a model that could not be found.
    if desc.model.is_empty() {
        let _ = world.remove::<(Model, Surface)>(entity);
        return;
    }
    match resolve(&desc.model) {
        Some(mesh) => {
            let surface = Surface(desc.material_from(palette));
            let _ = world.insert(entity, (Model(mesh), surface));
        }
        None => {
            let _ = world.remove::<(Model, Surface)>(entity);
            missing.push(Unresolved {
                entity_name: desc.name.clone(),
                model: desc.model.to_string(),
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
    mut resolve: impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: impl Fn(&crate::AssetLink) -> Option<Material>,
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
        resolve: &mut impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
        palette: &impl Fn(&crate::AssetLink) -> Option<Material>,
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
        resolve: &mut impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
        palette: &impl Fn(&crate::AssetLink) -> Option<Material>,
    ) -> bool {
        let mut changed = false;
        if was.is_none_or(|(old, _)| old.transform != desc.transform) {
            let _ = world.insert_one(entity, desc.transform);
            changed = true;
        }
        if was.is_none_or(|(old, _)| {
            old.model != desc.model || old.material != desc.material || old.decal != desc.decal
        }) {
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
        if was.is_none_or(|(old, _)| old.route != desc.route) {
            match &desc.route {
                Some(route) => {
                    let _ = world.insert_one(
                        entity,
                        crate::routes::Travelling::new(route.clone(), desc.transform.position),
                    );
                }
                None => {
                    let _ = world.remove_one::<crate::routes::Travelling>(entity);
                }
            }
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.particles != desc.particles) {
            let running = world
                .get::<&mut crate::particles::Emitting>(entity)
                .ok()
                .map(|mut e| {
                    if let Some(emitter) = &desc.particles {
                        // The knobs change; what is in the air stays.
                        if let Some(fresh) = emitting(emitter, resolve, palette) {
                            e.mesh = fresh.mesh;
                            e.material = fresh.material;
                        }
                        e.emitter = emitter.clone();
                    }
                });
            match (&desc.particles, running) {
                (Some(emitter), None) => {
                    if let Some(emitting) = emitting(emitter, resolve, palette) {
                        let _ = world.insert_one(entity, emitting);
                    }
                }
                (None, _) => {
                    let _ = world.remove_one::<crate::particles::Emitting>(entity);
                }
                _ => {}
            }
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.light != desc.light) {
            match desc.light {
                Some(light) => {
                    let _ = world.insert_one(entity, LightSource(light));
                }
                None => {
                    let _ = world.remove_one::<LightSource>(entity);
                }
            }
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.footprints != desc.footprints) {
            // Retuned, it starts a fresh trail: the old prints were made
            // by the old settings.
            match desc.footprints {
                Some(prints) => {
                    let _ = world.insert_one(entity, crate::footprints::Trail::new(prints));
                }
                None => {
                    let _ = world.remove_one::<crate::footprints::Trail>(entity);
                }
            }
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.bends_grass != desc.bends_grass) {
            if desc.bends_grass > 0.0 {
                let _ = world.insert_one(entity, BendsGrass(desc.bends_grass));
            } else {
                let _ = world.remove_one::<BendsGrass>(entity);
            }
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.render_texture != desc.render_texture) {
            match &desc.render_texture {
                Some(picture) => {
                    let _ = world.insert_one(entity, ToTexture(picture.clone()));
                }
                None => {
                    let _ = world.remove_one::<ToTexture>(entity);
                }
            }
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.post_volume != desc.post_volume) {
            match desc.post_volume {
                Some(volume) => {
                    let _ = world.insert_one(entity, PostVolumeBox(volume));
                }
                None => {
                    let _ = world.remove_one::<PostVolumeBox>(entity);
                }
            }
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.reflection_probe != desc.reflection_probe) {
            match desc.reflection_probe {
                Some(probe) => {
                    let _ = world.insert_one(entity, ProbeBox(probe));
                }
                None => {
                    let _ = world.remove_one::<ProbeBox>(entity);
                }
            }
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
            // A joint set anew is whole again.
            let _ = world.remove_one::<JointBroken>(entity);
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.bone != desc.bone) {
            if desc.bone.is_empty() {
                let _ = world.remove_one::<OnBone>(entity);
            } else {
                let _ = world.insert_one(entity, OnBone(desc.bone.clone()));
            }
            changed = true;
        }
        if was.is_none_or(|(old, _)| old.joint_break != desc.joint_break) {
            match desc.joint_break {
                Some(force) => {
                    let _ = world.insert_one(entity, JointBreak(force));
                }
                None => {
                    let _ = world.remove_one::<JointBreak>(entity);
                }
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
    // What is held by a bone is relative to that bone, as the parent's
    // pose has it: the bone's place in the parent's model goes between.
    let mut held: Vec<(hecs::Entity, glam::Mat4)> = Vec::new();
    for (entity, bone, parent) in world.query::<(hecs::Entity, &OnBone, &Parent)>().iter() {
        let mut q = world.query_one::<(&crate::Animator, &Posed)>(parent.0);
        let Ok((animator, posed)) = q.get() else {
            continue;
        };
        let Some((i, joint)) = animator
            .skeleton
            .joints
            .iter()
            .enumerate()
            .find(|(_, j)| j.name == bone.0)
        else {
            continue;
        };
        let Some(skinning) = posed.0.get(i) else {
            continue;
        };
        // Skinning is the joint's place times its inverse bind: undo the
        // second to get the first.
        let placed = *skinning * glam::Mat4::from_cols_array_2d(&joint.inverse_bind).inverse();
        held.push((entity, placed));
    }
    for (entity, bone) in held {
        if let Some((local, _)) = locals.get_mut(&entity) {
            *local = bone * *local;
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

/// The lighting a scene's sun describes.
///
/// One place, so the headless render, the editor and the walk-around light
/// the same scene the same way. They did not: two of them ignored the hour
/// entirely and the third had its own curve, and the difference only showed
/// up when a screenshot was compared with what the editor was showing.
pub fn scene_lighting(sun: &crate::scene::Sun) -> Lighting {
    // The light from all round goes with the sun: a dim sun is dusk or
    // night, and a sky as bright as noon's would light it like day.
    let day = Lighting::default();
    let share = (sun.intensity / day.sun_intensity).clamp(0.05, 1.3);
    // What faces down sees the ground: lit by the sun and the sky, and
    // sending back its own colour — a lot, and warm, off sand.
    let linear = |c: f32| crate::material::srgb_to_linear(c.clamp(0.0, 1.0));
    let albedo = glam::Vec3::new(
        linear(sun.ground[0]),
        linear(sun.ground[1]),
        linear(sun.ground[2]),
    );
    let sky = day.sky_color * share;
    let sun_light = sun.color() * sun.intensity * (-sun.direction().y).max(0.0);
    Lighting {
        sun_direction: sun.direction(),
        sun_color: sun.color(),
        sun_intensity: sun.intensity,
        sky_color: sky,
        ground_color: albedo * (sun_light + sky * 0.5),
        ground_albedo: albedo,
    }
}

/// The fog a scene describes.
pub fn scene_fog(fog: &crate::scene::Fog) -> FogSettings {
    FogSettings {
        color: glam::Vec3::from_array(fog.color),
        start: fog.start,
        end: fog.end,
        mode: fog.mode,
        density: fog.density,
    }
}

/// Put on the GPU every texture the world's materials draw with — their
/// base, normal, mask and emission maps — that is not there yet. What was
/// uploaded is known to the renderer by asset id, so a frame's materials
/// find their maps without anyone keeping handles. Says which maps the
/// library does not have.
pub fn upload_material_maps(
    world: &World,
    library: Option<&crate::Library>,
    gpu: &crate::gpu::Gpu,
    renderer: &mut crate::render::Renderer,
) -> Vec<String> {
    let mut wanted: Vec<crate::asset::AssetId> = world
        .query::<&Surface>()
        .iter()
        .flat_map(|surface| surface.0.maps().collect::<Vec<_>>())
        .chain(
            world
                .query::<&Pressing>()
                .iter()
                .flat_map(|p| p.1.maps().collect::<Vec<_>>())
                .collect::<Vec<_>>(),
        )
        .filter(|id| !id.is_render_target() && renderer.texture_for(*id).is_none())
        .collect();
    wanted.sort();
    wanted.dedup();
    let mut missing = Vec::new();
    for id in wanted {
        match library.and_then(|l| l.texture(id)) {
            Some(texture) => {
                renderer.upload_texture(gpu, texture);
            }
            None => missing.push(format!(
                "a material's map {id} is not in the library; re-import it"
            )),
        }
    }
    missing
}

/// Everything a scene says about how its frame looks — sun, fog, sky and
/// post-processing — around what is in the world. What a game and the
/// editor draw a scene with, so both show the same picture.
pub fn scene_frame(world: &World, camera: Camera, scene: &crate::scene::Scene) -> Frame {
    let mut frame = build_frame(
        world,
        camera,
        scene_lighting(&scene.sun),
        scene_fog(&scene.fog),
    );
    scene_look(&mut frame, scene);
    post_volumes(&mut frame, world);
    frame.texture_views = texture_views(world, scene, camera);
    frame
}

/// Put a scene's sky and post-processing on a frame built some other way.
pub fn scene_look(frame: &mut Frame, scene: &crate::scene::Scene) {
    if let Some(sky) = scene.sky {
        frame.sky = sky;
    }
    if let Some(post) = scene.post {
        frame.post = post;
    }
    if let Some(ambient_occlusion) = scene.ambient_occlusion {
        frame.ambient_occlusion = ambient_occlusion;
    }
    if let Some(ray_tracing) = scene.ray_tracing {
        frame.ray_tracing = ray_tracing;
    }
    if let Some(fog) = scene.volumetric_fog {
        frame.volumetric_fog = fog;
    }
    if let Some(wind) = scene.wind {
        frame.wind = wind;
    }
    if let Some(weather) = scene.weather {
        frame.weather = weather;
    }
    if let Some(ssr) = scene.screen_space_reflections {
        frame.screen_space_reflections = ssr;
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

/// Move every following camera toward its target and turn it to look,
/// damped: call it each frame with the frame's delta, before
/// [`camera_of`]. A camera at the top of the tree is moved in the world;
/// one under a parent is left to its parent.
pub fn follow_cameras(world: &mut World, dt: f32) {
    let targets: std::collections::HashMap<crate::id::EntityId, glam::Vec3> = world
        .query::<(&SceneId, &WorldTransform)>()
        .iter()
        .map(|(id, placed)| (id.0, placed.0.w_axis.truncate()))
        .collect();
    for (lens, transform, placed, parent) in world.query_mut::<(
        &CameraLens,
        &mut crate::scene::Transform,
        &mut WorldTransform,
        Option<&Parent>,
    )>() {
        let Some(follow) = lens.0.follow else {
            continue;
        };
        if parent.is_some() {
            continue;
        }
        let Some(&target) = targets.get(&follow.target) else {
            continue;
        };
        let wanted = target + follow.offset;
        // Exponential: the same softness at any frame rate.
        let keep = if follow.damping <= 0.0 {
            0.0
        } else {
            (-dt / follow.damping * 3.0).exp()
        };
        transform.position = wanted + (transform.position - wanted) * keep;
        if follow.look {
            let ahead = (target - transform.position).normalize_or_zero();
            if ahead != glam::Vec3::ZERO {
                let up = if ahead.y.abs() > 0.999 {
                    glam::Vec3::Z
                } else {
                    glam::Vec3::Y
                };
                let right = up.cross(ahead).normalize();
                let turn =
                    glam::Quat::from_mat3(&glam::Mat3::from_cols(right, ahead.cross(right), ahead));
                transform.set_rotation(turn);
            }
        }
        placed.0 = transform.matrix();
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
        .without::<&ToTexture>()
        .iter()
    {
        let camera = lens_camera(lens.0, placed.0);
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

/// A camera reflected in the plane through `at` facing `normal`: what a
/// mirror there shows, left and right swapped (the mirror's material
/// swaps them back, [`crate::material::ScreenMap::Mirror`]).
pub fn reflected(camera: Camera, at: glam::Vec3, normal: glam::Vec3) -> Camera {
    let point = |p: glam::Vec3| p - 2.0 * (p - at).dot(normal) * normal;
    let direction = |d: glam::Vec3| d - 2.0 * d.dot(normal) * normal;
    Camera {
        position: point(camera.position),
        target: point(camera.target),
        up: direction(camera.up),
        ..camera
    }
}

/// What a camera on an entity sees: from where it is, along its +z.
fn lens_camera(lens: crate::scene::Lens, placed: glam::Mat4) -> Camera {
    let (_, rotation, position) = placed.to_scale_rotation_translation();
    Camera {
        position,
        target: position + rotation * glam::Vec3::Z,
        up: rotation * glam::Vec3::Y,
        fov_y_degrees: lens.fov_deg,
        ortho: lens.ortho,
        ..Camera::default()
    }
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
    // Screens in the world: their pictures, and the things showing them.
    let mut ui_pictures = Vec::new();
    for (entity, screen, line) in world
        .query::<(hecs::Entity, &WorldUi, Option<&SceneId>)>()
        .iter()
    {
        if !keep(line.map(|l| l.0)) {
            continue;
        }
        let id = crate::asset::AssetId::render_target(&screen.name);
        ui_pictures.push(crate::render::UiPicture {
            id,
            size: screen.size,
            background: screen.background,
            ui: screen.ui.clone(),
        });
        if let (Ok(model), Ok(placed)) = (
            world.get::<&Model>(entity),
            world.get::<&WorldTransform>(entity),
        ) {
            if let Some(draw) = draws
                .iter_mut()
                .find(|d| d.mesh == model.0 && d.transform == placed.0)
            {
                draw.material.base_map = Some(id);
            }
        }
    }
    for (emitting, line) in world
        .query::<(&crate::particles::Emitting, Option<&SceneId>)>()
        .iter()
    {
        if keep(line.map(|l| l.0)) {
            draws.extend(emitting.draws_facing(Some(camera.position)));
        }
    }
    let lights = world
        .query::<(&LightSource, &WorldTransform, Option<&SceneId>)>()
        .iter()
        .filter(|(_, _, line)| keep(line.map(|l| l.0)))
        .map(|(light, placed, _)| {
            let l = light.0;
            let linear = |c: f32| crate::material::srgb_to_linear(c.clamp(0.0, 1.0));
            crate::render::PointLight {
                position: placed.0.w_axis.truncate(),
                color: glam::Vec3::new(linear(l.color.0), linear(l.color.1), linear(l.color.2))
                    * l.intensity.max(0.0),
                range: l.range,
                spot: l.cone_deg.map(|cone| {
                    let (_, turn, _) = placed.0.to_scale_rotation_translation();
                    (turn * glam::Vec3::Z, cone)
                }),
                shadows: l.shadows,
            }
        })
        .collect();
    let live_meshes = world
        .query::<(
            hecs::Entity,
            &LiveMesh,
            &WorldTransform,
            Option<&Surface>,
            Option<&SceneId>,
        )>()
        .iter()
        .filter(|(_, _, _, _, line)| keep(line.map(|l| l.0)))
        .map(
            |(entity, live, placed, surface, _)| crate::render::LiveMeshDraw {
                key: entity.to_bits().get(),
                version: live.version,
                vertices: live.vertices.clone(),
                indices: live.indices.clone(),
                transform: placed.0,
                material: surface.map_or_else(Material::default, |s| s.0),
            },
        )
        .collect();
    let flares = world
        .query::<(&LightSource, &WorldTransform, Option<&SceneId>)>()
        .iter()
        .filter(|(light, _, line)| light.0.flare > 0.0 && keep(line.map(|l| l.0)))
        .map(|(light, placed, _)| {
            let l = light.0;
            let linear = |c: f32| crate::material::srgb_to_linear(c.clamp(0.0, 1.0));
            crate::render::Flare {
                position: placed.0.w_axis.truncate(),
                color: glam::Vec3::new(linear(l.color.0), linear(l.color.1), linear(l.color.2)),
                intensity: l.flare,
            }
        })
        .collect();
    let reflection_probes = world
        .query::<(&ProbeBox, &WorldTransform, Option<&SceneId>)>()
        .iter()
        .filter(|(_, _, line)| keep(line.map(|l| l.0)))
        .map(|(probe, placed, _)| crate::reflections::ReflectionProbe {
            position: placed.0.w_axis.truncate(),
            extents: probe.0.size.abs() * 0.5,
            box_projection: probe.0.box_projection,
            blend_distance: probe.0.blend_distance,
        })
        .collect();
    let mut decals: Vec<crate::decals::Decal> = world
        .query::<(&Pressing, &WorldTransform, Option<&SceneId>)>()
        .iter()
        .filter(|(_, _, line)| keep(line.map(|l| l.0)))
        .map(|(pressing, placed, _)| crate::decals::Decal {
            transform: placed.0 * glam::Mat4::from_scale(pressing.0.size),
            material: pressing.1,
            shape: crate::decals::DecalShape::Picture,
        })
        .collect();
    // Walkers' prints, and the dust their steps kick up — the dust the
    // colour of the ground's top, lighter than the print turned over.
    let mut puffs = Vec::new();
    // The terrain drawn finely near the camera: the first there is.
    let terrain = world
        .query::<(&crate::terrain::Relief, &WorldTransform, Option<&SceneId>)>()
        .iter()
        .filter(|(_, _, line)| keep(line.map(|l| l.0)))
        .find_map(|(relief, placed, _)| {
            Some(crate::terrain::TerrainSurface {
                mesh: relief.mesh()?,
                placed: placed.0,
                terrain: relief.terrain,
            })
        });
    // Dune crests, into the world: where the wind may lift sand off them.
    let plumes: Vec<crate::volume::Plume> = world
        .query::<(&crate::terrain::Relief, &WorldTransform, Option<&SceneId>)>()
        .iter()
        .filter(|(_, _, line)| keep(line.map(|l| l.0)))
        .flat_map(|(relief, placed, _)| {
            let m = placed.0;
            let along = m
                .transform_vector3(glam::Vec3::Z)
                .normalize_or(glam::Vec3::Z);
            let half = (relief.terrain.dunes.wavelength * 0.06).max(1.0) * m.z_axis.length();
            relief
                .crests
                .iter()
                .map(move |c| crate::volume::Plume {
                    position: m.transform_point3(*c),
                    along,
                    half_length: half,
                    strength: 1.0,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    for (trail, line) in world
        .query::<(&crate::footprints::Trail, Option<&SceneId>)>()
        .iter()
    {
        if !keep(line.map(|l| l.0)) {
            continue;
        }
        decals.extend(trail.decals());
        let c = trail.settings.color;
        let linear = |v: f32| crate::material::srgb_to_linear((v * 1.25).clamp(0.0, 1.0));
        puffs.extend(trail.dust([linear(c[0]), linear(c[1]), linear(c[2])]));
    }
    Frame {
        camera,
        lighting,
        reflection_probes,
        decals,
        puffs,
        plumes,
        terrain,
        volumetric_fog: Default::default(),
        wind: Default::default(),
        benders: world
            .query::<(&BendsGrass, &WorldTransform, Option<&SceneId>)>()
            .iter()
            .filter(|(_, _, line)| keep(line.map(|l| l.0)))
            .map(|(bends, placed, _)| crate::foliage::Bender {
                position: placed.0.w_axis.truncate(),
                radius: bends.0,
            })
            .collect(),
        time: None,
        weather: Default::default(),
        screen_space_reflections: Default::default(),
        clear_color: fog.color,
        // The horizon is the fog's colour, so the far hills fade into the
        // sky rather than against it.
        sky: crate::render::Sky {
            horizon: fog.color.to_array(),
            ..Default::default()
        },
        fog,
        shadows: crate::render::ShadowSettings::default(),
        draws,
        overlay_draws: Vec::new(),
        lights,
        flares,
        live_meshes,
        texture_views: Vec::new(),
        ui_pictures,
        poses,
        post: Default::default(),
        ambient_occlusion: Default::default(),
        ray_tracing: Default::default(),
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn sand_underfoot_lights_what_faces_down_warm_and_bright() {
        let grass = scene_lighting(&crate::scene::Sun::default());
        let sand = scene_lighting(&crate::scene::Sun {
            ground: [0.78, 0.6, 0.38],
            ..crate::scene::Sun::default()
        });
        assert!(
            sand.ground_color.x > grass.ground_color.x * 3.0,
            "{} vs {}",
            sand.ground_color,
            grass.ground_color
        );
        assert!(
            sand.ground_color.x > sand.ground_color.z * 1.5,
            "warm: {}",
            sand.ground_color
        );
        // With the sun down to a glimmer the ground has little to send back.
        let night = scene_lighting(&crate::scene::Sun {
            intensity: 0.05,
            ground: [0.78, 0.6, 0.38],
            ..crate::scene::Sun::default()
        });
        assert!(
            night.ground_color.x < sand.ground_color.x * 0.2,
            "{}",
            night.ground_color
        );
    }
    use super::*;
    use glam::Vec3;

    /// An entity with nothing set, for `..blank()` in the tests below.
    fn blank() -> EntityDesc {
        EntityDesc {
            camera: None,
            spline: None,
            along: None,
            light: None,
            particles: None,
            reflection_probe: None,
            decal: None,
            footprints: None,
            terrain: None,
            bends_grass: 0.0,
            route: None,
            layer: Default::default(),
            physics: Default::default(),
            joint: Default::default(),
            joint_break: None,
            bone: String::new(),
            post_volume: None,
            render_texture: None,
            overrides: Default::default(),
            components: Default::default(),
            id: Default::default(),
            name: String::new(),
            model: "m".into(),
            prefab: Default::default(),
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
                    spline: None,
                    along: None,
                    light: None,
                    particles: None,
                    reflection_probe: None,
                    decal: None,
                    footprints: None,
                    terrain: None,
                    bends_grass: 0.0,
                    route: None,
                    layer: Default::default(),
                    physics: Default::default(),
                    joint: Default::default(),
                    joint_break: None,
                    bone: String::new(),
                    post_volume: None,
                    render_texture: None,
                    overrides: Default::default(),
                    components: Default::default(),
                    id: Default::default(),
                    name: format!("thing {i}"),
                    model: (*model).into(),
                    prefab: Default::default(),
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
    fn a_walker_with_footprints_leaves_them_in_the_frame_and_retuned_starts_afresh() {
        let text = r#"(entities: [
            (id: "e7", name: "walker", model: "m", transform: (position: (0.0, 0.9, 0.0)), footprints: (feet: 0.9)),
        ])"#;
        let before = scene(text);
        let mut world = spawned(&before);
        let walker = entity(&world, "e7");
        for i in 0..=120 {
            world.get::<&mut WorldTransform>(walker).unwrap().0 =
                glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.9, -0.05 * i as f32));
            crate::footprints::run_footprints(&mut world, 1.0 / 60.0);
        }
        let frame = build_frame(
            &world,
            Camera::default(),
            Lighting::default(),
            FogSettings::default(),
        );
        let prints: Vec<_> = frame
            .decals
            .iter()
            .filter(|d| d.shape == crate::decals::DecalShape::Footprint)
            .collect();
        assert_eq!(prints.len(), 8, "six metres at 0.75 a stride");
        assert!(
            prints.iter().all(|d| d.transform.w_axis.y.abs() < 1e-4),
            "at its feet, not its middle"
        );
        assert!(!frame.puffs.is_empty(), "and dust");
        // Retuned in the file: a fresh trail.
        let after = scene(&text.replace("feet: 0.9", "feet: 0.9, stride: 0.5"));
        patch(&before, &after, &mut world);
        let trail = world.get::<&crate::footprints::Trail>(walker).unwrap();
        assert_eq!(trail.prints(), 0);
        assert_eq!(trail.settings.stride, 0.5);
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
                follow: None,
            }),
            WorldTransform(glam::Mat4::IDENTITY),
        ));
        assert_eq!(camera_of(&world).unwrap().fov_y_degrees, 40.0);
        assert_eq!(camera_of(&world).unwrap().ortho, Some(8.0));

        // A camera following a player keeps behind it, softly, and looks.
        let mut world = World::new();
        let player_id: crate::id::EntityId = "00000000000000a7".parse().unwrap();
        let player = world.spawn((
            SceneId(player_id),
            WorldTransform(glam::Mat4::from_translation(glam::Vec3::new(
                10.0, 0.0, 0.0,
            ))),
        ));
        let lens = crate::scene::Lens {
            fov_deg: 60.0,
            priority: 0,
            ortho: None,
            follow: Some(crate::scene::Follow {
                target: player_id,
                offset: glam::Vec3::new(0.0, 3.0, -6.0),
                damping: 0.3,
                look: true,
            }),
        };
        world.spawn((
            CameraLens(lens),
            crate::scene::Transform::default(),
            WorldTransform(glam::Mat4::IDENTITY),
        ));
        follow_cameras(&mut world, 1.0 / 60.0);
        let early = camera_of(&world).unwrap().position;
        assert!(
            early.x > 0.1 && early.x < 5.0,
            "on its way, softly: {early}"
        );
        for _ in 0..120 {
            follow_cameras(&mut world, 1.0 / 60.0);
        }
        let camera = camera_of(&world).unwrap();
        assert!(
            (camera.position - glam::Vec3::new(10.0, 3.0, -6.0)).length() < 0.01,
            "{}",
            camera.position
        );
        let looking = (camera.target - camera.position).normalize();
        let at_player = (glam::Vec3::new(10.0, 0.0, 0.0) - camera.position).normalize();
        assert!(looking.dot(at_player) > 0.999, "looks at it");
        let _ = player;
        assert!(camera_of(&World::new()).is_none());
    }

    #[test]
    fn a_thing_on_a_bone_goes_where_the_pose_puts_the_bone() {
        use crate::animation::{Joint, PoseTransform, Skeleton};
        let bind = glam::Mat4::from_translation(glam::Vec3::new(1.0, 0.0, 0.0));
        let skeleton = std::sync::Arc::new(Skeleton {
            joints: vec![Joint {
                name: "hand".into(),
                parent: None,
                inverse_bind: bind.inverse().to_cols_array_2d(),
                rest: PoseTransform::default(),
            }],
        });
        let animator = crate::Animator::new(skeleton, std::sync::Arc::new(Vec::new()));
        // The pose lifts the hand to (1, 2, 0) in the model.
        let hand = glam::Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 0.0));
        let mut world = World::new();
        let at = |x: f32, y: f32, z: f32| Transform {
            position: glam::Vec3::new(x, y, z),
            ..Transform::default()
        };
        let body = world.spawn((
            at(10.0, 0.0, 0.0),
            animator,
            Posed(vec![hand * bind.inverse()]),
        ));
        let spade = world.spawn((at(0.0, 0.0, 0.5), Parent(body), OnBone("hand".into())));
        let beside = world.spawn((at(0.0, 0.0, 0.5), Parent(body)));
        apply_hierarchy(&mut world);
        let place = |e| world.get::<&WorldTransform>(e).unwrap().0.w_axis.truncate();
        assert_eq!(place(spade), glam::Vec3::new(11.0, 2.0, 0.5));
        assert_eq!(
            place(beside),
            glam::Vec3::new(10.0, 0.0, 0.5),
            "not on a bone: the body"
        );
    }

    #[test]
    fn a_post_volume_is_all_there_inside_and_fades_out_over_its_blend() {
        let dark = crate::post::PostProcess {
            exposure: -2.0,
            ..Default::default()
        };
        let mut world = World::new();
        world.spawn((
            WorldTransform(glam::Mat4::IDENTITY),
            PostVolumeBox(crate::scene::PostVolume {
                size: glam::Vec3::splat(4.0),
                blend_distance: 2.0,
                priority: 0,
                post: dark,
            }),
        ));
        let exposure_at = |x: f32| {
            let mut frame = Frame::default();
            frame.camera.position = glam::Vec3::new(x, 0.0, 0.0);
            let outside = frame.post.exposure;
            post_volumes(&mut frame, &world);
            (frame.post.exposure, outside)
        };
        assert_eq!(exposure_at(1.0).0, -2.0, "inside");
        let (half, outside) = exposure_at(3.0);
        assert!(
            (half - (outside + (-2.0 - outside) * 0.5)).abs() < 1e-4,
            "{half}"
        );
        assert_eq!(exposure_at(10.0).0, outside, "far away: the scene's");
    }
}
