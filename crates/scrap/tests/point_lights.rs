//! A light at a point: a pool of it on the floor under a lamp, dark past
//! its range — with the sun and the sky both off, so what is lit is lit by
//! the lamp.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{
    Camera, Draw, FogSettings, Frame, Lighting, PointLight, ShadowSettings, TextureHandle,
};
use scrap::{builtin, Gpu, OffscreenTarget, Renderer};

const SIZE: u32 = 64;

fn shoot(
    gpu: &Gpu,
    renderer: &mut Renderer,
    target: &OffscreenTarget,
    lights: Vec<PointLight>,
) -> Vec<u8> {
    let floor = renderer.upload_mesh_owned(gpu, &builtin::plane(1.0, 1));
    let frame = Frame {
        // Counted in exact colours: no sky, no post-processing.
        sky: scrap::render::Sky {
            mode: scrap::render::SkyMode::Color,
            ..Default::default()
        },
        post: scrap::post::PostProcess::OFF,
        ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
        ray_tracing: Default::default(),
        flares: Vec::new(),
        live_meshes: Vec::new(),
        texture_views: Vec::new(),
        ui_pictures: Vec::new(),
        camera: Camera {
            position: Vec3::new(0.0, 10.0, 0.01),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        lighting: Lighting {
            sun_intensity: 0.0,
            sky_color: Vec3::ZERO,
            ground_color: Vec3::ZERO,
            ..Lighting::default()
        },
        fog: FogSettings {
            start: 100.0,
            end: 200.0,
            ..FogSettings::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        lights,
        reflection_probes: Vec::new(),
        irradiance_volumes: Vec::new(),
        decals: Vec::new(),
        volumetric_fog: Default::default(),
        wind: Default::default(),
        benders: Vec::new(),
        time: None,
        weather: Default::default(),
        screen_space_reflections: Default::default(),
        puffs: Vec::new(),
        smoke: Vec::new(),
        distance_field: None,
        gpu_particles: Vec::new(),
        fullscreen: None,
        plumes: Vec::new(),
        terrain: None,
        draws: vec![Draw {
            mesh: floor,
            transform: Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0)),
            texture: TextureHandle::WHITE,
            material: scrap::Material::new(1.0, 1.0, 1.0),
            pose: None,
        }],
        overlay_draws: Vec::new(),
        outline_draws: Vec::new(),
        outline_width: 2.0,
        poses: Vec::new(),
    };
    renderer.render(gpu, target, &frame);
    target.read_rgba(gpu)
}

#[test]
fn a_lamp_lights_a_pool_under_it_and_nothing_past_its_range() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let red = |pixels: &[u8], x: u32, y: u32| OffscreenTarget::pixel(pixels, SIZE, x, y)[0];

    let dark = shoot(&gpu, &mut renderer, &target, Vec::new());
    assert!(
        red(&dark, SIZE / 2, SIZE / 2) < 5,
        "no sun, no sky, no lamp: dark"
    );

    let lamp = PointLight {
        falloff: Default::default(),
        inner_cone: None,
        position: Vec3::new(0.0, 1.0, 0.0),
        color: Vec3::new(3.0, 1.0, 0.3),
        range: 3.0,
        spot: None,
        shadows: false,
    };
    let lit = shoot(&gpu, &mut renderer, &target, vec![lamp]);
    let under = red(&lit, SIZE / 2, SIZE / 2);
    let edge = red(&lit, 2, 2);
    assert!(under > 100, "a pool under the lamp: {under}");
    assert!(edge < 5, "dark past its range: {edge}");

    // A spot pointing down in a narrow cone: lit under it, dark beside it
    // although well within its range.
    let torch = PointLight {
        falloff: Default::default(),
        inner_cone: None,
        position: Vec3::new(0.0, 2.0, 0.0),
        color: Vec3::new(3.0, 1.0, 0.3),
        range: 10.0,
        spot: Some((Vec3::NEG_Y, 30.0)),
        shadows: false,
    };
    let lit = shoot(&gpu, &mut renderer, &target, vec![torch]);
    let under = red(&lit, SIZE / 2, SIZE / 2);
    // Two metres to the side, at the image's scale (the view is ~10 m wide).
    let beside = red(&lit, SIZE / 2 + SIZE / 5, SIZE / 2);
    assert!(under > 100, "lit under the torch: {under}");
    assert!(beside < 5, "dark outside its cone: {beside}");
}
