//! Sand: ripples the wind laid in it show close by and fade far off
//! rather than shimmer; in a gale, sand drifting over it lightens it.

use scrap::foliage::Wind;
use scrap::glam::{Mat4, Vec3};
use scrap::material::Shading;
use scrap::render::{Camera, Draw, Frame, Lighting, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 128;

fn luminance(p: [u8; 4]) -> f32 {
    (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0
}

/// The spread of brightness over a band of rows.
fn spread(pixels: &[u8], rows: std::ops::Range<u32>) -> f32 {
    let values: Vec<f32> = rows
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .map(|(x, y)| luminance(OffscreenTarget::pixel(pixels, SIZE, x, y)))
        .collect();
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32).sqrt()
}

fn mean(pixels: &[u8], rows: std::ops::Range<u32>) -> f32 {
    let values: Vec<f32> = rows
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .map(|(x, y)| luminance(OffscreenTarget::pixel(pixels, SIZE, x, y)))
        .collect();
    values.iter().sum::<f32>() / values.len() as f32
}

#[test]
fn sand_ripples_close_by_fade_far_off_and_drift_in_a_gale() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let shot = |renderer: &mut Renderer, shading: Shading, wind: f32| {
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: scrap::post::PostProcess::OFF,
            ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
            // Looking down the sand, a low sun across the ripples.
            camera: Camera {
                position: Vec3::new(0.0, 1.2, 0.0),
                target: Vec3::new(0.0, 0.3, -4.0),
                ..Camera::default()
            },
            lighting: Lighting {
                sun_direction: Vec3::new(-0.8, -0.35, 0.1).normalize(),
                ..Lighting::default()
            },
            wind: Wind {
                direction: Vec3::X,
                strength: wind,
            },
            time: Some(3.0),
            clear_color: Vec3::splat(0.5),
            draws: vec![Draw {
                mesh: plane,
                transform: Mat4::from_scale(Vec3::new(400.0, 1.0, 400.0)),
                texture: TextureHandle::WHITE,
                material: Material {
                    shading,
                    smoothness: 0.1,
                    ..Material::new(0.8, 0.64, 0.44)
                },
                pose: None,
            }],
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        target.read_rgba(&gpu)
    };
    let plain = shot(&mut renderer, Shading::Lit, 0.5);
    let sand = shot(&mut renderer, Shading::Sand, 0.5);
    let near = SIZE * 3 / 4..SIZE;
    assert!(
        spread(&sand, near.clone()) > spread(&plain, near.clone()) + 3.0,
        "ripples close by: {} vs {}",
        spread(&sand, near.clone()),
        spread(&plain, near)
    );
    // Just under the horizon (at about a third of the way down) the
    // ripples are far smaller than a pixel: gone, not shimmering.
    let far = SIZE * 5 / 16..SIZE * 6 / 16;
    assert!(
        (spread(&sand, far.clone()) - spread(&plain, far.clone())).abs() < 2.0,
        "far off, as plain: {} vs {}",
        spread(&sand, far.clone()),
        spread(&plain, far)
    );
    let gale = shot(&mut renderer, Shading::Sand, 3.0);
    let rows = SIZE * 3 / 8..SIZE;
    assert!(
        mean(&gale, rows.clone()) > mean(&sand, rows.clone()) + 1.0,
        "drifting sand lightens it: {} vs {}",
        mean(&gale, rows.clone()),
        mean(&sand, rows)
    );
}
