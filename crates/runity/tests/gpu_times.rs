//! The GPU profiler: with it on, a few frames in, each pass has a time,
//! the main one among them, and none is absurd; with it off, none.

use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, Draw, Frame, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

#[test]
fn with_the_profiler_on_each_pass_has_a_time() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, 128, 128);
    let shoot = |on: bool| {
        let mut renderer = Renderer::new(&gpu, &target);
        renderer.profile_gpu(on);
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let frame = Frame {
            camera: Camera {
                position: Vec3::new(0.0, 1.5, 4.0),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            draws: vec![Draw {
                mesh: cube,
                transform: Mat4::IDENTITY,
                texture: TextureHandle::WHITE,
                material: Material::new(0.7, 0.6, 0.5),
                pose: None,
            }],
            ..Frame::default()
        };
        for _ in 0..12 {
            renderer.render(&gpu, &target, &frame);
            // A frame's wait for its pixels, as a game's present would be.
            let _ = target.read_rgba(&gpu);
        }
        renderer.gpu_times()
    };
    let off = shoot(false);
    assert!(off.is_empty(), "nothing timed when off: {off:?}");
    let on = shoot(true);
    if on.is_empty() {
        eprintln!("skipping: {} has no timestamps", gpu.describe());
        return;
    }
    eprintln!("{on:?}");
    assert!(on.iter().any(|(name, _)| name == "scene"), "the main pass: {on:?}");
    assert!(on.iter().any(|(name, _)| name == "post"), "post: {on:?}");
    for (name, ms) in &on {
        assert!(ms.is_finite() && *ms >= 0.0 && *ms < 100.0, "{name}: {ms} ms");
    }
    assert!(on.iter().map(|(_, ms)| ms).sum::<f32>() > 0.0, "some time taken: {on:?}");
}
