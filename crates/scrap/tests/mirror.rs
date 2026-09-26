//! A camera drawing into a picture, and a material showing it: a mirror.
//! The screen shows a red box behind the viewer only through the picture.

use scrap::asset::AssetId;
use scrap::glam::{Mat4, Quat, Vec3};
use scrap::material::Shading;
use scrap::render::{Camera, SkyMode};
use scrap::scene::{Lens, RenderTexture, Scene};
use scrap::world::{scene_frame, CameraLens, Model, Surface, ToTexture, WorldTransform};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 48;

#[test]
fn a_mirror_shows_what_its_camera_sees() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let unlit = |r, g, b| Material {
        shading: Shading::Unlit,
        ..Material::new(r, g, b)
    };
    let mut world = scrap::hecs::World::new();
    // A red box behind the viewer, at z = +5.
    world.spawn((
        WorldTransform(Mat4::from_translation(Vec3::new(0.0, 0.0, 5.0))),
        Model(cube),
        Surface(unlit(1.0, 0.0, 0.0)),
    ));
    // The mirror: a thin white screen in front of the viewer, at z = -2,
    // showing the picture called "mirror".
    world.spawn((
        WorldTransform(
            Mat4::from_translation(Vec3::new(0.0, 0.0, -2.0))
                * Mat4::from_scale(Vec3::new(3.0, 3.0, 0.05)),
        ),
        Model(cube),
        Surface(Material {
            base_map: Some(AssetId::render_target("mirror")),
            ..unlit(1.0, 1.0, 1.0)
        }),
    ));
    let scene = Scene { ..Scene::default() }
        .with(scrap::render::Sky {
            mode: SkyMode::Color,
            ..Default::default()
        })
        .with(scrap::post::PostProcess::OFF);
    let viewer = Camera {
        position: Vec3::ZERO,
        target: Vec3::new(0.0, 0.0, -1.0),
        ..Camera::default()
    };
    let mut middle = |world: &scrap::hecs::World| {
        let mut frame = scene_frame(world, viewer, &scene);
        frame.clear_color = Vec3::ZERO;
        frame.ambient_occlusion = scrap::ssao::AmbientOcclusion::OFF;
        for picture in &mut frame.texture_views {
            picture.frame.clear_color = Vec3::ZERO;
            picture.frame.ambient_occlusion = scrap::ssao::AmbientOcclusion::OFF;
        }
        renderer.render(&gpu, &target, &frame);
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
    };
    let before = middle(&world);
    assert!(
        before[0] < 128 || before[1] > 128,
        "no camera yet: no red {before:?}"
    );
    // The mirror's camera, at the mirror, looking back towards +z.
    world.spawn((
        WorldTransform(Mat4::from_rotation_translation(
            Quat::IDENTITY,
            Vec3::new(0.0, 0.0, -1.9),
        )),
        CameraLens(Lens {
            fov_deg: 40.0,
            priority: 0,
            ortho: None,
            follow: None,
            blend: None,
        }),
        ToTexture(RenderTexture {
            name: "mirror".into(),
            hide: Vec::new(),
            mirror: false,
            facing: Vec3::Y,
        }),
    ));
    let after = middle(&world);
    assert!(
        after[0] > 150 && after[1] < 80,
        "the red box, through the mirror: {after:?}"
    );
    assert!(
        scrap::world::camera_of(&world).is_none(),
        "a camera drawing into a picture is not the screen's"
    );
}

#[test]
fn a_planar_mirror_shows_what_is_behind_the_viewer() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let unlit = |r, g, b| Material {
        shading: Shading::Unlit,
        ..Material::new(r, g, b)
    };
    let mut world = scrap::hecs::World::new();
    // Red behind the viewer, green behind the mirror.
    world.spawn((
        WorldTransform(Mat4::from_translation(Vec3::new(0.0, 0.0, 3.0))),
        Model(cube),
        Surface(unlit(1.0, 0.0, 0.0)),
        scrap::world::SceneId(scrap::EntityId::from_raw(1)),
    ));
    world.spawn((
        WorldTransform(Mat4::from_translation(Vec3::new(0.0, 0.0, -4.0))),
        Model(cube),
        Surface(unlit(0.0, 1.0, 0.0)),
        scrap::world::SceneId(scrap::EntityId::from_raw(2)),
    ));
    // The mirror: a plane two metres ahead, facing the viewer (+z).
    world.spawn((
        WorldTransform(
            Mat4::from_translation(Vec3::new(0.0, 0.0, -2.0))
                * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)
                * Mat4::from_scale(Vec3::splat(4.0)),
        ),
        Model(plane),
        Surface(Material {
            base_map: Some(AssetId::render_target("glass")),
            screen_map: scrap::material::ScreenMap::Mirror,
            render_face: scrap::material::RenderFace::Both,
            ..unlit(1.0, 1.0, 1.0)
        }),
        ToTexture(RenderTexture {
            name: "glass".into(),
            hide: Vec::new(),
            mirror: true,
            facing: Vec3::Y,
        }),
    ));
    let scene = Scene { ..Scene::default() }
        .with(scrap::render::Sky {
            mode: SkyMode::Color,
            ..Default::default()
        })
        .with(scrap::post::PostProcess::OFF);
    let viewer = Camera {
        position: Vec3::ZERO,
        target: Vec3::new(0.0, 0.0, -1.0),
        ..Camera::default()
    };
    let mut frame = scene_frame(&world, viewer, &scene);
    frame.clear_color = Vec3::ZERO;
    frame.ambient_occlusion = scrap::ssao::AmbientOcclusion::OFF;
    for picture in &mut frame.texture_views {
        picture.frame.clear_color = Vec3::ZERO;
        picture.frame.ambient_occlusion = scrap::ssao::AmbientOcclusion::OFF;
    }
    renderer.render(&gpu, &target, &frame);
    let p = OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2);
    assert!(
        p[0] > 150 && p[1] < 80,
        "the red box behind, in the mirror: {p:?}"
    );
}

/// Unity's Quad for glass (its face +z here, `facing: (0, 0, 1)`), and
/// what stands to the viewer's left behind them on the mirror's left: a
/// mirror swaps front and back, not left and right; and nothing behind
/// the glass.
#[test]
fn a_quad_mirror_keeps_left_on_the_left_and_clips_behind_its_glass() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let quad = renderer.upload_mesh_owned(&gpu, &builtin::by_name("builtin:unity_quad").unwrap());
    let unlit = |r, g, b| Material {
        shading: Shading::Unlit,
        ..Material::new(r, g, b)
    };
    let mut world = scrap::hecs::World::new();
    let mut id = 0;
    let mut block = |at: Vec3, size: Vec3, m: Material| {
        id += 1;
        world.spawn((
            WorldTransform(Mat4::from_translation(at) * Mat4::from_scale(size)),
            Model(cube),
            Surface(m),
            scrap::world::SceneId(scrap::EntityId::from_raw(id)),
        ));
    };
    // Red behind the viewer to the left, blue to the right.
    block(Vec3::new(-5.0, 0.0, 3.0), Vec3::splat(2.0), unlit(1.0, 0.0, 0.0));
    block(Vec3::new(5.0, 0.0, 3.0), Vec3::splat(2.0), unlit(0.0, 0.0, 1.0));
    // A green wall behind the glass.
    block(Vec3::new(0.0, 0.0, -3.0), Vec3::new(8.0, 8.0, 0.5), unlit(0.0, 1.0, 0.0));
    world.spawn((
        WorldTransform(Mat4::from_translation(Vec3::new(0.0, 0.0, -2.0)) * Mat4::from_scale(Vec3::splat(6.0))),
        Model(quad),
        Surface(Material {
            base_map: Some(AssetId::render_target("glass")),
            screen_map: scrap::material::ScreenMap::Mirror,
            ..unlit(1.0, 1.0, 1.0)
        }),
        ToTexture(RenderTexture {
            name: "glass".into(),
            hide: Vec::new(),
            mirror: true,
            facing: Vec3::Z,
        }),
    ));
    let scene = Scene::default()
        .with(scrap::render::Sky {
            mode: SkyMode::Color,
            ..Default::default()
        })
        .with(scrap::post::PostProcess::OFF);
    let viewer = Camera {
        position: Vec3::ZERO,
        target: Vec3::new(0.0, 0.0, -1.0),
        fov_y_degrees: 90.0,
        ..Camera::default()
    };
    let mut frame = scene_frame(&world, viewer, &scene);
    frame.clear_color = Vec3::ZERO;
    frame.ambient_occlusion = scrap::ssao::AmbientOcclusion::OFF;
    for picture in &mut frame.texture_views {
        picture.frame.clear_color = Vec3::ZERO;
        picture.frame.ambient_occlusion = scrap::ssao::AmbientOcclusion::OFF;
    }
    renderer.render(&gpu, &target, &frame);
    let pixels = target.read_rgba(&gpu);
    let at = |x, y| OffscreenTarget::pixel(&pixels, SIZE, x, y);
    let (left, right, middle) = (at(SIZE / 8, SIZE / 2), at(SIZE * 7 / 8, SIZE / 2), at(SIZE / 2, SIZE / 2));
    assert!(left[0] > 150 && left[2] < 80, "red on the left: {left:?}");
    assert!(right[2] > 150 && right[0] < 80, "blue on the right: {right:?}");
    assert!(middle[1] < 60, "nothing behind the glass in it: {middle:?}");
}

/// What is only for pictures — a first-person player's own body — is in
/// the mirror's frame and not the screen's.
#[test]
fn a_body_only_for_pictures_is_in_the_mirror_and_not_on_the_screen() {
    let mut world = scrap::hecs::World::new();
    let body = Material::new(1.0, 0.0, 0.0);
    world.spawn((
        WorldTransform(Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0))),
        Model(scrap::render::MeshHandle::TEST),
        Surface(body),
        scrap::world::PicturesOnly,
    ));
    world.spawn((
        WorldTransform(Mat4::from_translation(Vec3::new(0.0, 0.0, -2.0))),
        Model(scrap::render::MeshHandle::TEST),
        Surface(Material::new(1.0, 1.0, 1.0)),
        ToTexture(RenderTexture {
            name: "glass".into(),
            hide: Vec::new(),
            mirror: true,
            facing: Vec3::Z,
        }),
    ));
    let frame = scene_frame(&world, Camera { position: Vec3::ZERO, target: Vec3::NEG_Z, ..Camera::default() }, &Scene::default());
    let red = |f: &scrap::render::Frame| f.draws.iter().filter(|d| d.material == body).count();
    assert_eq!(red(&frame), 0, "not on the screen");
    assert_eq!(red(&frame.texture_views[0].frame), 1, "in the mirror");
}
