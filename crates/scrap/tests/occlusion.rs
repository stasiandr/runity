//! Occlusion culling on the GPU: fifty cubes hidden behind a wall and ten
//! in front of it. A couple of frames in, only what is seen is kept — the
//! wall, the floor, those in front the frustum lets through — and the
//! picture is the picture without culling. Those behind are blue and those
//! in front red: what is kept is numbered anew, and each must still be
//! shaded as its own instance and not as the one its new number had.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 96;

#[test]
fn what_stands_behind_a_wall_is_left_out_and_nothing_seen_is() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let shot = |culling: bool| {
        let mut renderer = Renderer::new(&gpu, &target);
        renderer.set_occlusion_culling(culling);
        let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let grey = Material::new(0.7, 0.7, 0.7);
        let coloured = |mesh, transform, material| Draw {
            mesh,
            transform,
            texture: TextureHandle::WHITE,
            material,
            pose: None,
        };
        let draw = |mesh, transform| coloured(mesh, transform, grey);
        let mut draws = vec![
            draw(plane, Mat4::from_scale(Vec3::splat(40.0))),
            // The wall, filling the view.
            draw(
                cube,
                Mat4::from_translation(Vec3::new(0.0, 2.0, -2.0)) * Mat4::from_scale(Vec3::new(20.0, 4.0, 0.3)),
            ),
        ];
        for i in 0..50 {
            let x = (i % 10) as f32 - 4.5;
            let z = -5.0 - (i / 10) as f32;
            draws.push(coloured(
                cube,
                Mat4::from_translation(Vec3::new(x, 0.5, z)) * Mat4::from_scale(Vec3::splat(0.5)),
                Material::new(0.1, 0.2, 0.9),
            ));
        }
        for i in 0..10 {
            let x = (i as f32 - 4.5) * 0.4;
            draws.push(coloured(
                cube,
                Mat4::from_translation(Vec3::new(x, 0.15, 1.0)) * Mat4::from_scale(Vec3::splat(0.2)),
                Material::new(0.9, 0.1, 0.1),
            ));
        }
        let frame = Frame {
            camera: Camera {
                position: Vec3::new(0.0, 1.2, 4.0),
                target: Vec3::new(0.0, 1.0, -2.0),
                ..Camera::default()
            },
            draws,
            ..Frame::default()
        };
        for _ in 0..3 {
            renderer.render(&gpu, &target, &frame);
        }
        (target.read_rgba(&gpu), renderer.culled_kept(&gpu), renderer.stats().drawn)
    };
    let (seen, kept, listed) = shot(true);
    let (all, none, _) = shot(false);
    assert_eq!(none, None, "without culling nothing is culled");
    let kept = kept.expect("culled a couple of frames in");
    let differing = seen
        .chunks(4)
        .zip(all.chunks(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 12))
        .count();
    assert!(differing < 40, "the same picture: {differing} pixels differ");
    let red = |p: &[u8]| p[0] > p[2] + 40;
    let reds = all.chunks(4).filter(|p| red(p)).count();
    assert!(reds > 20, "the cubes in front are seen: {reds} red pixels");
    // Red with culling too, a pixel or two of edge aside: shaded as another
    // instance they would be blue.
    let seen_reds = seen.chunks(4).filter(|p| red(p)).count();
    assert!(seen_reds.abs_diff(reds) <= reds / 20, "and are red with culling too: {seen_reds} of {reds}");
    // What the CPU's frustum let through, less the fifty behind the wall.
    assert_eq!(kept, listed - 50, "all that is seen, none of what is behind: {kept} of {listed}");
}
