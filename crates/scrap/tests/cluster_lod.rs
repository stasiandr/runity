//! Clusters at every level of detail (scrap-render's `cluster_lod`, a
//! Nanite of our own): dense balls from near to far drawn at the levels
//! whose error is under a pixel look as they do drawn whole, and far fewer
//! clusters are drawn.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 320;

#[test]
fn far_dense_meshes_drawn_at_a_pixels_error_look_as_they_do_whole() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let run = |error: f32| {
        let mut renderer = Renderer::new(&gpu, &target);
        renderer.set_cluster_error(error);
        // Sixty-five thousand triangles a ball.
        let ball = renderer.upload_mesh_owned(&gpu, &builtin::sphere(1.0, 256, 128));
        let draws: Vec<Draw> = [3.0f32, 6.0, 12.0, 25.0, 50.0, 80.0]
            .iter()
            .enumerate()
            .map(|(i, &d)| Draw {
                mesh: ball,
                transform: Mat4::from_translation(Vec3::new((i as f32 - 2.5) * d * 0.28, 0.0, -d)),
                texture: TextureHandle::WHITE,
                material: Material::new(0.8, 0.45, 0.3),
                pose: None,
            })
            .collect();
        let frame = Frame {
            post: scrap::post::PostProcess::OFF,
            camera: Camera { position: Vec3::new(0.0, 0.5, 0.0), target: Vec3::new(0.0, 0.0, -10.0), ..Camera::default() },
            draws,
            ..Frame::default()
        };
        let mut pixels = Vec::new();
        let mut kept = None;
        for _ in 0..3 {
            renderer.render(&gpu, &target, &frame);
            pixels = target.read_rgba(&gpu);
            kept = renderer.clusters_kept(&gpu);
        }
        (pixels, kept)
    };
    let (whole, whole_kept) = run(0.0);
    let (lod, lod_kept) = run(1.0);
    let (whole_kept, lod_kept) = (whole_kept.expect("culled by clusters"), lod_kept.expect("culled by clusters"));
    let diff: Vec<u32> = whole.chunks(4).zip(lod.chunks(4)).map(|(a, b)| (0..3).map(|c| a[c].abs_diff(b[c]) as u32).max().unwrap()).collect();
    let mean = diff.iter().sum::<u32>() as f32 / diff.len() as f32;
    let off = diff.iter().filter(|d| **d > 32).count();
    eprintln!("clusters drawn: whole {whole_kept}, at a pixel's error {lod_kept}; mean difference {mean:.3}, {off} pixels off by more than 32");
    assert!(lod_kept * 2 < whole_kept, "the far balls were not drawn coarser: {lod_kept} of {whole_kept} clusters");
    assert!(mean < 0.5 && off < 40, "a pixel's error is not the same picture: mean {mean:.3}, {off} pixels off");
}
