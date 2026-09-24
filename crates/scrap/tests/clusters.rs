//! Clusters: dense meshes culled a cluster at a time on the GPU draw the
//! same picture as without, and leave out what is off the screen, turned
//! away or behind a wall.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 160;

fn difference(a: &[u8], b: &[u8]) -> f32 {
    let total: u64 = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(p, q)| (0..3).map(|c| (p[c] as i32 - q[c] as i32).unsigned_abs() as u64).sum::<u64>())
        .sum();
    total as f32 / (a.len() / 4 * 3) as f32
}

#[test]
fn dense_meshes_culled_by_clusters_draw_the_same_and_keep_what_is_seen() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let run = |clusters: bool| {
        let mut renderer = Renderer::new(&gpu, &target);
        renderer.set_cluster_culling(clusters);
        // A sphere of nine thousand triangles: some in view, some behind
        // the eye, some behind a wall.
        let ball = renderer.upload_mesh_owned(&gpu, &builtin::sphere(1.0, 96, 48));
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let mut draws = Vec::new();
        let at = |p: Vec3| Draw {
            mesh: ball,
            transform: Mat4::from_translation(p),
            texture: TextureHandle::WHITE,
            material: Material::new(0.8, 0.4, 0.3),
            pose: None,
        };
        for x in -2..=2 {
            draws.push(at(Vec3::new(x as f32 * 2.2, 0.0, 0.0)));
            draws.push(at(Vec3::new(x as f32 * 2.2, 0.0, 12.0)));
            draws.push(at(Vec3::new(x as f32 * 2.2, 0.0, -9.0)));
        }
        draws.push(Draw {
            mesh: cube,
            transform: Mat4::from_scale_rotation_translation(Vec3::new(30.0, 8.0, 0.4), Default::default(), Vec3::new(0.0, 0.0, -5.0)),
            texture: TextureHandle::WHITE,
            material: Material::new(0.5, 0.5, 0.55),
            pose: None,
        });
        let frame = Frame {
            post: scrap::post::PostProcess {
                auto_exposure: scrap::exposure::AutoExposure::OFF,
                ..scrap::post::PostProcess::default()
            },
            camera: Camera {
                position: Vec3::new(0.0, 1.0, 8.0),
                target: Vec3::new(0.0, 0.0, 0.0),
                ..Camera::default()
            },
            draws,
            ..Frame::default()
        };
        for _ in 0..12 {
            renderer.render(&gpu, &target, &frame);
        }
        (target.read_rgba(&gpu), renderer.clusters_kept(&gpu))
    };
    let (plain, none) = run(false);
    assert_eq!(none, None, "nothing culled by clusters with it off");
    let (clustered, kept) = run(true);
    let Some(kept) = kept else {
        eprintln!("skipping: the device cannot cull by clusters");
        return;
    };
    let off = difference(&plain, &clustered);
    assert!(off < 0.5, "the same picture: {off:.3}");
    // Fifteen balls of a few dozen clusters each: the five in view, and of
    // them only what faces the eye.
    let per = (96 * 48 * 2usize).div_ceil(scrap::cluster::TRIANGLES as usize) as u32;
    let all = per * 15;
    eprintln!("{kept} of {all} clusters kept");
    assert!(kept > per * 5 / 3, "the five in view are drawn: {kept}");
    assert!(kept < per * 5, "half of each faces away, the rest out of view or hidden: {kept} of {all}");
}
