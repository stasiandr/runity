//! Tonemapping: a surface of light too bright for the screen, deep red.
//! Clipped, it is pure red at the top of the scale; AgX rolls it off toward
//! white with its hue kept, rises with the light all the way, and leaves
//! middle grey about middle grey.

use runity::glam::{Mat4, Vec3};
use runity::material::Shading;
use runity::post::{PostProcess, Tonemapping};
use runity::render::{Camera, Draw, Frame, Sky, SkyMode, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 32;

#[test]
fn agx_rolls_bright_red_toward_white_and_keeps_grey() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let shot = |tonemapping: Tonemapping, emission: Vec3| {
        let mut renderer = Renderer::new(&gpu, &target);
        let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: PostProcess {
                enabled: true,
                tonemapping,
                ..PostProcess::OFF
            },
            camera: Camera {
                position: Vec3::new(0.0, 3.0, 0.01),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            draws: vec![Draw {
                mesh: plane,
                transform: Mat4::from_scale(Vec3::splat(20.0)),
                texture: TextureHandle::WHITE,
                material: Material {
                    shading: Shading::Unlit,
                    emission: emission.to_array(),
                    ..Material::new(0.0, 0.0, 0.0)
                },
                pose: None,
            }],
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
    };
    let hot = Vec3::new(40.0, 0.4, 0.2);
    let clipped = shot(Tonemapping::None, hot);
    let agx = shot(Tonemapping::Agx, hot);
    assert!(clipped[0] == 255 && clipped[1] < 200, "clipped: red at the top: {clipped:?}");
    // Rolled off toward white: its other channels risen with it, red still
    // leading — the hue kept.
    assert!(agx[1] > 200 && agx[0] >= agx[1] && agx[1] >= agx[2], "agx toward white: {agx:?}");
    // Brighter in, brighter out, all the way up.
    let mut last = 0u32;
    for scale in [0.05f32, 0.2, 0.8, 3.0, 12.0, 50.0] {
        let p = shot(Tonemapping::Agx, Vec3::new(1.0, 0.5, 0.25) * scale);
        let sum = p[0] as u32 + p[1] as u32 + p[2] as u32;
        assert!(sum >= last, "agx rises with the light: {sum} after {last} at {scale}");
        last = sum;
    }
    // Middle grey (0.18 linear) comes out about the middle, and grey.
    let grey = shot(Tonemapping::Agx, Vec3::splat(0.18));
    assert!((70..=170).contains(&grey[1]), "middle grey stays middling: {grey:?}");
    assert!(grey[0].abs_diff(grey[1]) < 6 && grey[1].abs_diff(grey[2]) < 6, "and grey: {grey:?}");
}
