//! The physical sky: blue overhead at noon, and a card facing the sun lit
//! warmer and dimmer by a low sun than by a high one — the colour coming from the air,
//! not from a picked one.

use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 48;

fn shoot(
    gpu: &Gpu,
    renderer: &mut Renderer,
    target: &OffscreenTarget,
    to_sun: Vec3,
    look_up: bool,
) -> Vec<u8> {
    let plane = renderer.upload_mesh_owned(gpu, &builtin::plane(1.0, 1));
    let frame = Frame {
        sky: Sky {
            mode: SkyMode::Physical,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        camera: if look_up {
            Camera {
                position: Vec3::new(0.0, 2.0, 0.0),
                target: Vec3::new(0.0, 10.0, 0.01),
                ..Camera::default()
            }
        } else {
            // In front of a card that faces the sun.
            Camera {
                position: to_sun.normalize() * 4.0 + Vec3::new(0.0, 0.01, 0.0),
                target: Vec3::ZERO,
                ..Camera::default()
            }
        },
        lighting: Lighting {
            sun_direction: -to_sun.normalize(),
            // A picked colour the physical sky must ignore.
            sun_color: Vec3::new(0.0, 1.0, 0.0),
            sun_intensity: 1.0,
            ..Lighting::default()
        },
        shadows: ShadowSettings::OFF,
        draws: vec![Draw {
            mesh: plane,
            transform: Mat4::from_quat(runity::glam::Quat::from_rotation_arc(
                Vec3::Y,
                to_sun.normalize(),
            )) * Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0)),
            texture: TextureHandle::WHITE,
            material: Material::new(0.8, 0.8, 0.8),
            pose: None,
        }],
        ..Frame::default()
    };
    renderer.render(gpu, target, &frame);
    target.read_rgba(gpu)
}

#[test]
fn noon_is_blue_overhead_and_a_low_sun_lights_the_floor_warm() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let noon = Vec3::new(0.2, 1.0, 0.1);
    let low = Vec3::new(1.0, 0.12, 0.0);
    let middle = |p: &[u8]| OffscreenTarget::pixel(p, SIZE, SIZE / 2, SIZE / 2);

    let sky = middle(&shoot(&gpu, &mut renderer, &target, noon, true));
    assert!(sky[2] > sky[0] + 30, "blue overhead: {sky:?}");

    let high = middle(&shoot(&gpu, &mut renderer, &target, noon, false));
    let warm = middle(&shoot(&gpu, &mut renderer, &target, low, false));
    assert!(
        high[0] > 100 && (high[1] as i32 - high[0] as i32).abs() < 60,
        "not the picked green: {high:?}"
    );
    let warmth = |p: [u8; 4]| p[0] as f32 / p[2].max(1) as f32;
    assert!(
        warmth(warm) > warmth(high) * 1.2,
        "warmer at sunset: {high:?} -> {warm:?}"
    );
    let sum = |p: [u8; 4]| p[0] as u32 + p[1] as u32 + p[2] as u32;
    assert!(sum(warm) < sum(high), "and dimmer: {high:?} -> {warm:?}");
}
