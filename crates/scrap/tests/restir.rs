//! ReSTIR: twelve lamps round a wall, lit by one shadow ray a pixel, come
//! out on average as a ray a lamp does — the same light, the same shadow
//! behind the wall.

use scrap::glam::{Mat4, Quat, Vec3};
use scrap::render::{Camera, Draw, Frame, Lighting, PointLight, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 96;

fn frame(cube: scrap::render::MeshHandle, restir: bool) -> Frame {
    let solid = |transform, colour: f32| Draw {
        mesh: cube,
        transform,
        texture: TextureHandle::WHITE,
        material: Material::new(colour, colour, colour),
        pose: None,
    };
    let lights = (0..12)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 12.0;
            let hue = [(1.0, 0.6, 0.3), (0.4, 0.7, 1.0), (1.0, 1.0, 0.9)][i % 3];
            PointLight {
                position: Vec3::new(a.cos() * 4.0, 1.2, a.sin() * 4.0),
                color: Vec3::new(hue.0, hue.1, hue.2) * 0.35,
                range: 7.0,
                spot: None,
                shadows: true,
            }
        })
        .collect();
    Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: scrap::post::PostProcess::OFF,
        ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
        ray_tracing: scrap::ray::RayTracing {
            light_shadows: true,
            restir,
            ..Default::default()
        },
        camera: Camera {
            position: Vec3::new(0.0, 11.0, 0.01),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        lighting: Lighting {
            sun_intensity: 0.0,
            sky_color: Vec3::ZERO,
            ground_color: Vec3::ZERO,
            ..Lighting::default()
        },
        lights,
        draws: vec![
            solid(Mat4::from_scale_rotation_translation(Vec3::new(14.0, 0.2, 14.0), Quat::IDENTITY, Vec3::new(0.0, -0.1, 0.0)), 0.8),
            solid(Mat4::from_scale_rotation_translation(Vec3::new(0.4, 2.5, 5.0), Quat::IDENTITY, Vec3::new(0.0, 1.25, 0.0)), 0.8),
        ],
        ..Frame::default()
    }
}

fn luminance(pixels: &[f32]) -> f32 {
    pixels.iter().sum::<f32>() / pixels.len() as f32
}

#[test]
fn one_ray_a_pixel_lights_twelve_lamps_as_a_ray_a_lamp_does() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    if !gpu.ray_tracing {
        eprintln!("skipping: {} does not trace rays", gpu.describe());
        return;
    }
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    // The picture's red, averaged over frames.
    let averaged = |restir: bool, frames: u32| {
        let mut renderer = Renderer::new(&gpu, &target);
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
        let frame = frame(cube, restir);
        let mut sum = vec![0.0f32; (SIZE * SIZE) as usize];
        for i in 0..frames + 4 {
            renderer.render(&gpu, &target, &frame);
            if i >= 4 {
                let pixels = target.read_rgba(&gpu);
                for (s, p) in sum.iter_mut().zip(pixels.chunks_exact(4)) {
                    *s += p[0] as f32 / frames as f32;
                }
            }
        }
        sum
    };
    let every = averaged(false, 2);
    let resampled = averaged(true, 48);
    let (a, b) = (luminance(&every), luminance(&resampled));
    let off = every.iter().zip(&resampled).map(|(x, y)| (x - y).abs()).sum::<f32>() / every.len() as f32;
    eprintln!("ray a lamp {a:.1}, restir {b:.1}, mean difference {off:.1}");
    assert!(a > 20.0, "the lamps light the floor: {a:.1}");
    assert!((a - b).abs() < a * 0.15, "as much light: {a:.1} vs {b:.1}");
    assert!(off < 20.0, "in the same places: {off:.1}");
}
