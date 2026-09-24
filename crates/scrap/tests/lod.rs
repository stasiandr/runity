//! Levels of detail: a dense ball near the camera is drawn with all its
//! triangles, far off with far fewer, and so far that it is a pixel, not at
//! all — and it looks the same size either way.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

#[test]
fn a_dense_ball_is_drawn_coarser_the_smaller_it_is_on_the_screen() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, 96, 96);
    let mut renderer = Renderer::new(&gpu, &target);
    let ball = builtin::sphere(1.0, 64, 32);
    let full = ball.indices.len() as u64 / 3;
    let handle = renderer.upload_mesh_owned(&gpu, &ball);
    let mut at = |distance: f32| {
        let frame = Frame {
            camera: Camera {
                position: Vec3::new(0.0, 0.0, distance),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            draws: vec![Draw {
                mesh: handle,
                transform: Mat4::IDENTITY,
                texture: TextureHandle::WHITE,
                material: Material::new(0.8, 0.8, 0.8),
                pose: None,
            }],
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        renderer.stats()
    };
    let near = at(3.0);
    assert_eq!(near.triangles, full, "near: all of it");
    let mid = at(15.0);
    assert!(mid.triangles < full / 2 && mid.triangles > 0, "mid: coarser, {} of {full}", mid.triangles);
    let far = at(45.0);
    assert!(far.triangles < mid.triangles, "far: coarser still, {} after {}", far.triangles, mid.triangles);
    let gone = at(2000.0);
    assert_eq!(gone.drawn, 0, "a pixel off: not drawn");
    assert_eq!(gone.culled, 1);
}
