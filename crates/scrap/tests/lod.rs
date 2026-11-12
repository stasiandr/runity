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
    // The first coarser level: at most seven tenths of the mesh, or
    // `lod::simplify` would not have kept it ("hardly smaller"). A ball's
    // comes to about half (2125 of 4096): its vertices are merged within a
    // cell of space only where they are also near on the picture
    // (`UV_CELLS`, which keeps an atlas's pieces apart), and a ball's
    // picture goes all the way round it.
    assert!(mid.triangles * 10 <= full * 7 && mid.triangles > 0, "mid: coarser, {} of {full}", mid.triangles);
    let far = at(45.0);
    assert!(far.triangles < mid.triangles / 2, "far: coarser still, {} after {}", far.triangles, mid.triangles);
    let gone = at(2000.0);
    assert_eq!(gone.drawn, 0, "a pixel off: not drawn");
    assert_eq!(gone.culled, 1);
}

/// A project without levels of detail (`scrap.ron`'s `game: (lod: false)`)
/// bakes its meshes whole (`cluster::bake_whole`): the same ball is drawn
/// with every triangle near, mid and far, and still drawn where it is a
/// pixel.
#[test]
fn a_ball_baked_whole_is_drawn_whole_at_every_distance() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, 96, 96);
    let mut renderer = Renderer::new(&gpu, &target);
    let mut ball = builtin::sphere(1.0, 64, 32);
    let full = ball.indices.len() as u64 / 3;
    scrap::cluster::bake_whole(&mut ball);
    let path = std::env::temp_dir().join(format!("scrap-lod-whole-{}.scrasset", std::process::id()));
    std::fs::write(&path, scrap::asset::to_bytes(&ball, scrap::asset::MESH).unwrap()).unwrap();
    let bytes = scrap::asset::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let archived = scrap::asset::view::<scrap::asset::MeshAsset>(&bytes).unwrap();
    let handle = renderer.upload_mesh(&gpu, archived);
    assert_eq!(renderer.mesh_levels(handle), 0, "no coarser levels");
    assert!(renderer.mesh_is_whole(handle));
    // 490 m: under `lod::TOO_SMALL` of the screen, within the camera's far
    // plane.
    for distance in [3.0f32, 15.0, 45.0, 490.0] {
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
        let stats = renderer.stats();
        assert_eq!(stats.drawn, 1, "at {distance} m: drawn");
        assert_eq!(stats.triangles, full, "at {distance} m: all of it");
    }
}
