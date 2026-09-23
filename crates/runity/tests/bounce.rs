//! Light from all round: a rough white wall facing a red one, inside a
//! reflection probe, takes on the red — the probe's blurred picture stands
//! in for the light bounced off it. Without a probe it is lit by the sky.

use runity::glam::{Mat4, Vec3};
use runity::material::Shading;
use runity::reflections::ReflectionProbe;
use runity::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 64;

#[test]
fn a_white_wall_turns_red_facing_a_red_one_inside_a_probe() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let white = Material {
        smoothness: 0.0,
        ..Material::new(0.9, 0.9, 0.9)
    };
    let red = Material {
        shading: Shading::Unlit,
        ..Material::new(1.0, 0.0, 0.0)
    };
    let wall = |x: f32, material| Draw {
        mesh: cube,
        transform: Mat4::from_translation(Vec3::new(x, 2.0, 0.0))
            * Mat4::from_scale(Vec3::new(0.2, 4.0, 10.0)),
        texture: TextureHandle::WHITE,
        material,
        pose: None,
    };
    let draws = vec![wall(-3.0, white), wall(3.0, red)];
    let frame = |probes: Vec<ReflectionProbe>| Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        // Standing by the red wall, looking at the white one.
        camera: Camera {
            position: Vec3::new(2.0, 2.0, 0.0),
            target: Vec3::new(-3.0, 2.0, 0.0),
            ..Camera::default()
        },
        lighting: Lighting {
            sun_intensity: 0.0,
            sky_color: Vec3::splat(0.3),
            ground_color: Vec3::splat(0.3),
            ..Lighting::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::splat(0.3),
        draws: draws.clone(),
        reflection_probes: probes,
        ..Frame::default()
    };
    let middle = |renderer: &mut Renderer, probes| {
        renderer.render(&gpu, &target, &frame(probes));
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
    };
    let without = middle(&mut renderer, Vec::new());
    let probe = ReflectionProbe {
        position: Vec3::new(0.0, 2.0, 0.0),
        extents: Vec3::new(5.0, 3.0, 5.0),
        box_projection: true,
        blend_distance: 0.5,
    };
    let with = middle(&mut renderer, vec![probe]);
    let redness = |p: [u8; 4]| p[0] as i32 - p[1] as i32;
    assert!(
        redness(without) < 8,
        "no probe: grey from the sky: {without:?}"
    );
    assert!(
        redness(with) > 30,
        "with a probe the red wall bleeds onto it: {with:?}"
    );
}

#[test]
fn the_floor_by_a_red_wall_turns_red_with_bounced_light() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let white = Material {
        smoothness: 0.0,
        ..Material::new(0.9, 0.9, 0.9)
    };
    let red = Material {
        shading: Shading::Unlit,
        ..Material::new(1.0, 0.0, 0.0)
    };
    let draws = vec![
        Draw {
            mesh: plane,
            transform: Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0)),
            texture: TextureHandle::WHITE,
            material: white,
            pose: None,
        },
        Draw {
            mesh: cube,
            transform: Mat4::from_translation(Vec3::new(1.0, 2.0, 0.0))
                * Mat4::from_scale(Vec3::new(0.2, 4.0, 10.0)),
            texture: TextureHandle::WHITE,
            material: red,
            pose: None,
        },
    ];
    let frame = |bounce: f32| Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion {
            bounce,
            ..Default::default()
        },
        // Looking down at the floor at the wall's foot, from beside it.
        camera: Camera {
            position: Vec3::new(-2.0, 2.5, 0.0),
            target: Vec3::new(0.5, 0.0, 0.0),
            ..Camera::default()
        },
        lighting: Lighting {
            sun_intensity: 0.0,
            sky_color: Vec3::splat(0.3),
            ground_color: Vec3::splat(0.3),
            ..Lighting::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::splat(0.3),
        draws: draws.clone(),
        ..Frame::default()
    };
    let middle = |renderer: &mut Renderer, bounce| {
        // Twice: the bounce reads the frame before.
        renderer.render(&gpu, &target, &frame(bounce));
        renderer.render(&gpu, &target, &frame(bounce));
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
    };
    let redness = |p: [u8; 4]| p[0] as i32 - p[1] as i32;
    let without = middle(&mut renderer, 0.0);
    let with = middle(&mut renderer, 1.0);
    assert!(
        redness(without) < 8,
        "no bounce: the floor is grey: {without:?}"
    );
    assert!(
        redness(with) > 20,
        "bounced, the wall reddens the floor at its foot: {with:?} (was {without:?})"
    );
}
