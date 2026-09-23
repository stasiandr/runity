//! Auto exposure: out of a dark passage into the sun the picture is too
//! bright at first and settles over a second or two; and a still shot
//! starts where it is aimed, with nothing before it to adapt from.

use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 64;

fn mean(pixels: &[u8]) -> f32 {
    pixels
        .chunks(4)
        .map(|c| (c[0] as f32 + c[1] as f32 + c[2] as f32) / 3.0)
        .sum::<f32>()
        / (pixels.len() / 4) as f32
}

#[test]
fn out_of_the_dark_into_the_sun_the_eye_takes_a_moment() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let frame = |sun: f32, time: f32| Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess {
            bloom: runity::post::Bloom {
                intensity: 0.0,
                ..Default::default()
            },
            taa: false,
            ..Default::default()
        },
        camera: Camera {
            position: Vec3::new(0.0, 3.0, 0.1),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        // A floor lit by the sun, or by next to nothing: the passage.
        lighting: Lighting {
            sun_direction: Vec3::NEG_Y,
            sun_intensity: sun,
            sky_color: Vec3::splat(0.02 * sun),
            ground_color: Vec3::splat(0.02 * sun),
            ..Lighting::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        time: Some(time),
        draws: vec![Draw {
            mesh: plane,
            transform: Mat4::from_scale(Vec3::splat(20.0)),
            texture: TextureHandle::WHITE,
            material: Material::new(0.6, 0.6, 0.6),
            pose: None,
        }],
        ..Frame::default()
    };
    // In the passage for five seconds: the eye opens up.
    let mut t = 0.0;
    for _ in 0..150 {
        renderer.render(&gpu, &target, &frame(0.08, t));
        t += 1.0 / 30.0;
    }
    let inside = mean(&target.read_rgba(&gpu));
    // Out into the sun.
    renderer.render(&gpu, &target, &frame(3.0, t));
    let first = mean(&target.read_rgba(&gpu));
    // Ten seconds on: the eye takes to the light slowly, so coming out is
    // blinding for a while.
    for _ in 0..300 {
        t += 1.0 / 30.0;
        renderer.render(&gpu, &target, &frame(3.0, t));
    }
    let settled = mean(&target.read_rgba(&gpu));
    // A still shot of the same sun, with nothing before it.
    let mut fresh = Renderer::new(&gpu, &target);
    let plane_again = fresh.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let mut still = frame(3.0, 0.0);
    still.draws[0].mesh = plane_again;
    fresh.render(&gpu, &target, &still);
    let shot = mean(&target.read_rgba(&gpu));
    assert!(inside > 40.0, "in the dark the eye opens up: {inside}");
    assert!(
        first > settled + 30.0,
        "blinding at first: {first} then {settled}"
    );
    assert!(
        (settled - shot).abs() < 12.0,
        "settled where a still shot starts: {settled} vs {shot}"
    );
}
