//! A camera drawing into a picture, and a material showing it: a mirror.
//! The screen shows a red box behind the viewer only through the picture.

use runity::asset::AssetId;
use runity::glam::{Mat4, Quat, Vec3};
use runity::material::Shading;
use runity::render::{Camera, SkyMode};
use runity::scene::{Lens, RenderTexture, Scene};
use runity::world::{scene_frame, CameraLens, Model, Surface, ToTexture, WorldTransform};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

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
    let mut world = runity::hecs::World::new();
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
    let scene = Scene {
        sky: Some(runity::render::Sky {
            mode: SkyMode::Color,
            ..Default::default()
        }),
        post: Some(runity::post::PostProcess::OFF),
        ..Scene::default()
    };
    let viewer = Camera {
        position: Vec3::ZERO,
        target: Vec3::new(0.0, 0.0, -1.0),
        ..Camera::default()
    };
    let mut middle = |world: &runity::hecs::World| {
        let mut frame = scene_frame(world, viewer, &scene);
        frame.clear_color = Vec3::ZERO;
        frame.ambient_occlusion = runity::ssao::AmbientOcclusion::OFF;
        for picture in &mut frame.texture_views {
            picture.frame.clear_color = Vec3::ZERO;
            picture.frame.ambient_occlusion = runity::ssao::AmbientOcclusion::OFF;
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
        }),
        ToTexture(RenderTexture {
            name: "mirror".into(),
            hide: Vec::new(),
        }),
    ));
    let after = middle(&world);
    assert!(
        after[0] > 150 && after[1] < 80,
        "the red box, through the mirror: {after:?}"
    );
    assert!(
        runity::world::camera_of(&world).is_none(),
        "a camera drawing into a picture is not the screen's"
    );
}
