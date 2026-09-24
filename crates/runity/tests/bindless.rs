//! Bindless maps: thirty crates, each its own picture, are thirty draws
//! with a bind group each, and one draw with every texture in an array —
//! the same picture either way.

use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, Draw, Frame, Sky, SkyMode};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 96;

fn shot(gpu: &Gpu) -> (Vec<u8>, u32) {
    let target = OffscreenTarget::new(gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(gpu, &target);
    let cube = renderer.upload_mesh_owned(gpu, &builtin::cube(1.0));
    let mut draws = Vec::new();
    for i in 0..30u32 {
        // A little picture of its own: two colours in a checker.
        let a = [(i * 37 % 255) as u8, (i * 91 % 255) as u8, (i * 53 % 255) as u8, 255];
        let b = [255 - a[0], a[2], a[1], 255];
        let pixels: Vec<u8> = (0..16)
            .flat_map(|p| if (p % 4 + p / 4) % 2 == 0 { a } else { b })
            .collect();
        let texture = renderer.upload_texture_rgba(gpu, 4, 4, &pixels, true);
        draws.push(Draw {
            mesh: cube,
            transform: Mat4::from_translation(Vec3::new((i % 6) as f32 * 1.3 - 3.25, (i / 6) as f32 * 1.3 - 2.6, 0.0)),
            texture,
            material: Material {
                shading: runity::material::Shading::Unlit,
                ..Material::new(1.0, 1.0, 1.0)
            },
            pose: None,
        });
    }
    let frame = Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        camera: Camera {
            position: Vec3::new(0.0, 0.0, 9.0),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        draws,
        ..Frame::default()
    };
    renderer.render(gpu, &target, &frame);
    (target.read_rgba(gpu), renderer.stats().batches)
}

#[test]
fn thirty_pictures_are_one_draw_bindless_and_the_same_picture() {
    let Ok(mut gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    if !gpu.bindless {
        eprintln!("skipping: {} has no texture binding arrays", gpu.describe());
        return;
    }
    let (bindless, one) = shot(&gpu);
    gpu.bindless = false;
    let (bound, thirty) = shot(&gpu);
    assert_eq!(one, 1, "one batch bindless");
    assert_eq!(thirty, 30, "a batch a picture bound");
    let differ = bindless.iter().zip(&bound).filter(|(a, b)| a != b).count();
    assert!(differ < bindless.len() / 200, "the same picture: {differ} bytes differ");
    // And every crate shows its own colours.
    let colours: std::collections::HashSet<[u8; 3]> =
        bindless.chunks_exact(4).map(|p| [p[0] / 8, p[1] / 8, p[2] / 8]).collect();
    assert!(colours.len() > 40, "{} colours", colours.len());
}
