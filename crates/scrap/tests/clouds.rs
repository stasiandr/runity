//! Clouds: an overcast sky is whiter and greyer than a clear one, and the
//! ground under it is in their shadow.

use scrap::clouds::Clouds;
use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 48;

#[test]
fn an_overcast_sky_is_grey_and_shadows_the_ground() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let shoot = |renderer: &mut Renderer, coverage: f32, look_up: bool| {
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Physical,
                clouds: Clouds {
                    coverage,
                    ..Default::default()
                },
                ..Default::default()
            },
            post: scrap::post::PostProcess::OFF,
            ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
            camera: Camera {
                position: Vec3::new(0.0, 2.0, 0.0),
                target: if look_up {
                    Vec3::new(0.0, 12.0, -1.0)
                } else {
                    Vec3::new(0.0, 0.0, -0.01)
                },
                ..Camera::default()
            },
            lighting: Lighting {
                sun_direction: Vec3::new(0.2, -1.0, 0.1).normalize(),
                sun_intensity: 1.0,
                ..Lighting::default()
            },
            shadows: ShadowSettings::OFF,
            draws: vec![Draw {
                mesh: plane,
                transform: Mat4::from_scale(Vec3::new(40.0, 1.0, 40.0)),
                texture: TextureHandle::WHITE,
                material: Material::new(0.8, 0.8, 0.8),
                pose: None,
            }],
            time: Some(0.0),
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
    };
    let clear_sky = shoot(&mut renderer, 0.0, true);
    let overcast_sky = shoot(&mut renderer, 1.0, true);
    let blueness = |p: [u8; 4]| p[2] as i32 - p[0] as i32;
    assert!(
        blueness(overcast_sky) < blueness(clear_sky) - 20,
        "overcast is less blue: {clear_sky:?} -> {overcast_sky:?}"
    );
    let sum = |p: [u8; 4]| p[0] as u32 + p[1] as u32 + p[2] as u32;
    let sunny = shoot(&mut renderer, 0.0, false);
    let shaded = shoot(&mut renderer, 1.0, false);
    assert!(
        sum(shaded) < sum(sunny) * 3 / 4,
        "the ground is in the clouds' shadow: {sunny:?} -> {shaded:?}"
    );
}
