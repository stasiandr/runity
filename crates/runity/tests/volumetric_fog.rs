//! Volumetric fog: a lamp in the dark lights the air round it where there
//! is fog and not where there is none, and a white wall far off is dimmed
//! by the air in front of it.

use runity::glam::{Mat4, Vec3};
use runity::material::Shading;
use runity::render::{
    Camera, Draw, Frame, Lighting, PointLight, ShadowSettings, Sky, SkyMode, TextureHandle,
};
use runity::volume::VolumetricFog;
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 64;

fn brightness(p: [u8; 4]) -> u32 {
    p[0] as u32 + p[1] as u32 + p[2] as u32
}

#[test]
fn a_lamp_lights_the_fog_round_it_and_the_fog_dims_what_is_far() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    // A white wall far off, filling the left half of the view.
    let wall = Draw {
        mesh: cube,
        transform: Mat4::from_translation(Vec3::new(-10.0, 0.0, -40.0))
            * Mat4::from_scale(Vec3::new(20.0, 40.0, 1.0)),
        texture: TextureHandle::WHITE,
        material: Material {
            shading: Shading::Unlit,
            ..Material::new(1.0, 1.0, 1.0)
        },
        pose: None,
    };
    let frame = |fog: VolumetricFog| Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        camera: Camera {
            position: Vec3::ZERO,
            target: Vec3::new(0.0, 0.0, -1.0),
            ..Camera::default()
        },
        lighting: Lighting {
            sun_intensity: 0.0,
            sky_color: Vec3::ZERO,
            ground_color: Vec3::ZERO,
            ..Lighting::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        // A lamp hanging in the air to the right, nothing near it to light.
        lights: vec![PointLight {
            position: Vec3::new(1.2, 0.0, -4.0),
            color: Vec3::splat(4.0),
            range: 4.0,
            spot: None,
            shadows: false,
        }],
        draws: vec![wall],
        volumetric_fog: fog,
        ..Frame::default()
    };
    let shoot = |renderer: &mut Renderer, fog| {
        renderer.render(&gpu, &target, &frame(fog));
        target.read_rgba(&gpu)
    };
    let clear = shoot(&mut renderer, VolumetricFog::OFF);
    let misty = shoot(
        &mut renderer,
        VolumetricFog {
            enabled: true,
            density: 0.08,
            height_falloff: 0.0,
            ..VolumetricFog::OFF
        },
    );
    let lamp_x = SIZE / 2 + SIZE * 3 / 16;
    let near_lamp = |p: &[u8]| brightness(OffscreenTarget::pixel(p, SIZE, lamp_x, SIZE / 2));
    let far_wall = |p: &[u8]| brightness(OffscreenTarget::pixel(p, SIZE, SIZE / 8, SIZE / 2));
    assert!(
        near_lamp(&clear) < 5,
        "no fog: the air is dark: {}",
        near_lamp(&clear)
    );
    assert!(
        near_lamp(&misty) > 40,
        "fog: it glows round the lamp: {}",
        near_lamp(&misty)
    );
    assert!(
        far_wall(&misty) < far_wall(&clear) * 3 / 4,
        "the fog dims the far wall: {} -> {}",
        far_wall(&clear),
        far_wall(&misty)
    );
}
