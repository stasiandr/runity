//! From a world to a draw list: the render module's components of a world
//! — what an entity draws with and as, its light, its camera — and the
//! frame a scene and a camera make.

#[allow(unused_imports)]
use crate::prelude::*;


use hecs::World;

use crate::material::Material;
use crate::render::{Camera, Draw, FogSettings, Frame, Lighting, MeshHandle, TextureHandle};
use crate::world::{inactive_in_hierarchy, Layer, Parent, SceneId, WorldTransform};

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
                scene_lighting(&scene.sun()),
                scene_fog(&scene.fog()),
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
    let night = sun.night();
    if night <= 0.0 {
        return Lighting {
            sun_direction: sun.direction(),
            sun_color: sun.color(),
            sun_intensity: sun.intensity,
            sky_color: sky,
            ground_color: albedo * (sun_light + sky * 0.5),
            ground_albedo: albedo,
            sky_sun: None,
            night: 0.0,
        };
    }
    // Night: the moon is the light above — cold, an eighth of the sun, and
    // casting shadows; the sky is lit by the sun where it really is, under
    // the horizon, and the stars come out. At dusk the two cross over.
    let moon = glam::Vec3::new(0.62, 0.72, 1.0);
    let night_sky = glam::Vec3::new(0.012, 0.017, 0.035) * share.max(0.5);
    let (direction, color, intensity) = if night < 0.5 {
        (
            sun.direction(),
            sun.color(),
            sun.intensity * (1.0 - 2.0 * night),
        )
    } else {
        (
            sun.moon_direction(),
            moon,
            sun.intensity * 0.07 * (2.0 * night - 1.0),
        )
    };
    let sky = sky.lerp(night_sky, night);
    let key = color * intensity * (-direction.y).max(0.0);
    Lighting {
        sun_direction: direction,
        sun_color: color,
        sun_intensity: intensity,
        sky_color: sky,
        ground_color: albedo * (key + sky * 0.5),
        ground_albedo: albedo,
        sky_sun: Some((sun.true_direction(), sun.intensity)),
        night,
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
        // A particle's sprite: smoke is a puff, not a square.
        .chain(
            world
                .query::<&crate::particles::Emitting>()
                .iter()
                .filter_map(|e| e.material)
                .flat_map(|m| m.maps().collect::<Vec<_>>())
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
        scene_lighting(&scene.sun()),
        scene_fog(&scene.fog()),
    );
    scene_look(&mut frame, scene);
    post_volumes(&mut frame, world);
    frame.texture_views = texture_views(world, scene, camera);
    frame
}

/// Put a scene's sky and post-processing on a frame built some other way.
pub fn scene_look(frame: &mut Frame, scene: &crate::scene::Scene) {
    if let Some(sky) = scene.sky() {
        frame.sky = sky;
    }
    if let Some(post) = scene.post() {
        frame.post = post;
    }
    if let Some(ambient_occlusion) = scene.ambient_occlusion() {
        frame.ambient_occlusion = ambient_occlusion;
    }
    if let Some(ray_tracing) = scene.ray_tracing() {
        frame.ray_tracing = ray_tracing;
    }
    if let Some(fog) = scene.volumetric_fog() {
        frame.volumetric_fog = fog;
    }
    if let Some(wind) = scene.wind() {
        frame.wind = wind;
    }
    if let Some(weather) = scene.weather() {
        frame.weather = weather;
    }
    if let Some(ssr) = scene.screen_space_reflections() {
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
    // What is switched off, itself or by a parent, is not in the picture.
    let off: std::collections::HashSet<crate::id::EntityId> = inactive_in_hierarchy(world)
        .into_iter()
        .filter_map(|e| world.get::<&SceneId>(e).ok().map(|id| id.0))
        .collect();
    let keep =
        |line: Option<crate::id::EntityId>| line.is_none_or(|id| !off.contains(&id)) && keep(line);
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
