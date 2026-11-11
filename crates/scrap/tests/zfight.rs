//! Z-fighting found from pictures (scrap-render's `zfight`): a frame drawn
//! as draw ids reads back as the ids, and a decal flush with its floor
//! flickers between the two as the camera moves a hair — where a decal
//! lifted a centimetre and an edge moving under the jitter do not count.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, TextureHandle};
use scrap::zfight::{self, Flicker};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 128;

fn frame(renderer: &mut Renderer, gpu: &Gpu, lift: f32) -> Frame {
    let plane = renderer.upload_mesh_owned(gpu, &builtin::plane(1.0, 1));
    let cube = renderer.upload_mesh_owned(gpu, &builtin::cube(1.0));
    let draw = |mesh, transform, material| Draw { mesh, transform, texture: TextureHandle::WHITE, material, pose: None };
    Frame {
        camera: Camera { position: Vec3::new(0.0, 9.0, 9.0), target: Vec3::ZERO, near: 0.3, far: 120.0, fov_y_degrees: 40.0, ..Camera::default() },
        draws: vec![
            draw(plane, Mat4::from_scale(Vec3::splat(8.0)), Material::new(0.5, 0.5, 0.5)),
            draw(plane, Mat4::from_scale_rotation_translation(Vec3::splat(3.0), Default::default(), Vec3::new(-1.0, lift, 0.5)), Material::new(0.9, 0.1, 0.1)),
            draw(cube, Mat4::from_translation(Vec3::new(2.5, 0.5, 0.0)), Material::new(0.1, 0.2, 0.9)),
        ],
        ..Frame::default()
    }
}

/// The ids of `frame` drawn from a few cameras a hair apart, both orders.
fn pictures(renderer: &mut Renderer, gpu: &Gpu, target: &OffscreenTarget, frame: &Frame) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    for k in 0..6 {
        let mut f = zfight::id_frame(frame, k % 2 == 1);
        let nudge = Vec3::new((k as f32 * 0.37).sin(), (k as f32 * 0.71).cos(), (k as f32 * 1.3).sin()) * 0.003;
        f.camera.position += nudge;
        renderer.render(gpu, target, &f);
        out.push(zfight::decode_ids(&target.read_rgba(gpu)));
    }
    out
}

#[test]
fn a_decal_flush_with_its_floor_flickers_and_a_lifted_one_does_not() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut fights = Vec::new();
    for lift in [0.0, 0.01] {
        let mut renderer = Renderer::new(&gpu, &target);
        renderer.keep_geometry(true);
        let frame = frame(&mut renderer, &gpu, lift);
        let pics = pictures(&mut renderer, &gpu, &target, &frame);
        // Every pixel of the floor, the decal and the box reads as an id.
        let centre = pics[0][(SIZE / 2 * SIZE + SIZE / 2) as usize];
        assert!(centre == 1 || centre == 2, "the middle shows the floor or the decal, not {centre}");
        let read = pics[0].iter().filter(|i| **i != zfight::MIXED).count();
        assert!(read as f32 > 0.9 * (SIZE * SIZE) as f32, "{read} pixels read");
        // Fighting: the winner changes and the two are one depth along the
        // pixel's ray.
        let flicker = Flicker::of(&pics, SIZE, SIZE);
        let draws = zfight::draw_list(&frame);
        let mut fighting = 0;
        for (i, pair) in flicker.pairs.iter().enumerate() {
            let Some((a, b)) = pair else { continue };
            let (x, y) = (i as u32 % SIZE, i as u32 / SIZE);
            let (from, dir) = zfight::pixel_ray(&frame.camera, (0, 0, SIZE, SIZE), x, y);
            let depth = |id: u32| {
                let d = &draws[id as usize - 1];
                zfight::ray_depth(renderer.mesh_geometry(d.mesh).unwrap(), d.transform, from, dir)
            };
            if let (Some(da), Some(db)) = (depth(*a), depth(*b)) {
                if (da - db).abs() < 0.004 {
                    fighting += 1;
                }
            }
        }
        fights.push(fighting);
    }
    assert!(fights[0] > 200, "the flush decal fights over many pixels: {fights:?}");
    assert_eq!(fights[1], 0, "a centimetre up it does not: {fights:?}");
}
