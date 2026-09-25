//! Unlit see-through things drawn at half size and laid over the picture
//! (scrap-render's `lowres`): a field of sprites over a wall and a floor
//! looks as it does drawn at full size — the same light, the same cover —
//! bar the softness of half the pixels at their edges.

use scrap::glam::{Mat4, Quat, Vec3};
use scrap::material::{Blend, Shading, SurfaceType};
use scrap::render::{Camera, Draw, Frame, Lighting, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 192;

fn scene(renderer: &mut Renderer, gpu: &Gpu, blend: Blend) -> Frame {
    let plane = renderer.upload_mesh_owned(gpu, &builtin::plane(1.0, 1));
    let cube = renderer.upload_mesh_owned(gpu, &builtin::cube(1.0));
    let draw = |mesh, transform, material| Draw { mesh, transform, texture: TextureHandle::WHITE, material, pose: None };
    let mut draws = vec![
        draw(plane, Mat4::from_scale(Vec3::splat(20.0)), Material::new(0.5, 0.45, 0.4)),
        draw(cube, Mat4::from_scale_rotation_translation(Vec3::new(1.0, 2.0, 1.0), Quat::IDENTITY, Vec3::new(0.0, 1.0, -1.0)), Material::new(0.3, 0.4, 0.6)),
    ];
    // Sprites, unlit and half see-through, some in front of the box and
    // some behind it: facing the camera, in rows.
    for i in 0..24 {
        let mut dust = Material::new(0.9, 0.7, 0.4);
        dust.shading = Shading::Unlit;
        dust.surface = SurfaceType::Transparent;
        dust.blend = blend;
        dust.alpha = 0.35;
        let x = (i % 6) as f32 * 0.8 - 2.0;
        let z = (i / 6) as f32 * 1.1 - 2.5;
        let at = Mat4::from_scale_rotation_translation(Vec3::splat(1.2), Quat::from_rotation_x(std::f32::consts::FRAC_PI_2), Vec3::new(x, 1.0, z));
        draws.push(draw(plane, at, dust));
    }
    Frame {
        sky: Sky { mode: SkyMode::Color, ..Default::default() },
        clear_color: Vec3::new(0.3, 0.35, 0.4),
        camera: Camera { position: Vec3::new(0.0, 1.5, 5.0), target: Vec3::new(0.0, 1.0, 0.0), ..Camera::default() },
        lighting: Lighting::default(),
        // One sample — FXAA, not TAA, so two frames are the same picture.
        post: scrap::post::PostProcess { enabled: true, fxaa: true, taa: false, ..scrap::post::PostProcess::OFF },
        draws,
        ..Frame::default()
    }
}

fn shot(renderer: &mut Renderer, gpu: &Gpu, target: &OffscreenTarget, frame: &Frame) -> Vec<u8> {
    for _ in 0..2 {
        renderer.render(gpu, target, frame);
    }
    target.read_rgba(gpu)
}

#[test]
fn sprites_at_half_size_look_as_they_do_at_full_size() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    for blend in [Blend::Alpha, Blend::Additive, Blend::Premultiply] {
        let mut renderer = Renderer::new(&gpu, &target);
        let frame = scene(&mut renderer, &gpu, blend);
        renderer.set_half_size_particles(false);
        let full = shot(&mut renderer, &gpu, &target, &frame);
        assert!(!renderer.halved_particles());
        renderer.set_half_size_particles(true);
        let half = shot(&mut renderer, &gpu, &target, &frame);
        assert!(renderer.halved_particles(), "{blend:?}: the sprites were not drawn at half size");
        // On average the same picture; the edges of half-size texels
        // differ, a pixel's width of them.
        let diff: Vec<u32> = full.chunks(4).zip(half.chunks(4)).map(|(a, b)| (0..3).map(|c| a[c].abs_diff(b[c]) as u32).max().unwrap()).collect();
        let mean = diff.iter().sum::<u32>() as f32 / diff.len() as f32;
        let off = diff.iter().filter(|d| **d > 24).count() as f32 / diff.len() as f32;
        eprintln!("{blend:?}: mean {mean:.2}, {:.2}% of pixels off by more than 24", off * 100.0);
        assert!(mean < 2.0 && off < 0.02, "{blend:?}: half size is not the same picture (mean {mean:.2}, off {off:.4})");
    }
    // Multiplying sprites need the picture under them: full size.
    let mut renderer = Renderer::new(&gpu, &target);
    let frame = scene(&mut renderer, &gpu, Blend::Multiply);
    shot(&mut renderer, &gpu, &target, &frame);
    assert!(!renderer.halved_particles(), "multiplying sprites were drawn at half size");
}
