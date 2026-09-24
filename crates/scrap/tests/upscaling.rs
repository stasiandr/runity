//! Upscaling: a scene drawn at half the screen's width and made up to it
//! is close to the one drawn at the screen's own size, by every upscaler
//! the device has; and dynamic resolution goes down when the frame is too
//! slow for its target.

use scrap::glam::{Mat4, Quat, Vec3};
use scrap::render::{Camera, Draw, Frame, TextureHandle};
use scrap::upscale::{DynamicResolution, Method, Upscaling, Used};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 192;

fn scene(renderer: &mut Renderer, gpu: &Gpu, post: scrap::post::PostProcess) -> Frame {
    let cube = renderer.upload_mesh_owned(gpu, &builtin::cube(1.0));
    let sphere = renderer.upload_mesh_owned(gpu, &builtin::sphere(1.0, 48, 24));
    let mut draws = vec![
        Draw {
            mesh: cube,
            transform: Mat4::from_scale_rotation_translation(Vec3::new(12.0, 0.2, 12.0), Quat::IDENTITY, Vec3::new(0.0, -1.1, 0.0)),
            texture: TextureHandle::WHITE,
            material: Material::new(0.6, 0.55, 0.5),
            pose: None,
        },
        Draw {
            mesh: sphere,
            transform: Mat4::from_translation(Vec3::new(-1.2, 0.0, 0.0)),
            texture: TextureHandle::WHITE,
            material: Material::new(0.8, 0.3, 0.2),
            pose: None,
        },
    ];
    // Bars thinner and thinner: what fewer pixels lose first.
    for i in 0..9 {
        let width = 0.12 * 0.75f32.powi(i);
        draws.push(Draw {
            mesh: cube,
            transform: Mat4::from_scale_rotation_translation(
                Vec3::new(width, 2.0, 0.1),
                Quat::from_rotation_z(0.1),
                Vec3::new(0.3 + i as f32 * 0.28, 0.0, 0.0),
            ),
            texture: TextureHandle::WHITE,
            material: Material::new(0.9, 0.9, 0.95),
            pose: None,
        });
    }
    Frame {
        post,
        camera: Camera {
            position: Vec3::new(0.4, 0.6, 5.0),
            target: Vec3::new(0.4, 0.0, 0.0),
            ..Camera::default()
        },
        draws,
        ..Frame::default()
    }
}

/// Mean difference per channel, 0..255.
fn difference(a: &[u8], b: &[u8]) -> f32 {
    let total: u64 = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(p, q)| (0..3).map(|c| (p[c] as i32 - q[c] as i32).unsigned_abs() as u64).sum::<u64>())
        .sum();
    total as f32 / (a.len() / 4 * 3) as f32
}

fn post(upscaling: Upscaling) -> scrap::post::PostProcess {
    scrap::post::PostProcess {
        upscaling,
        // A still picture: the eye already used to it.
        auto_exposure: scrap::exposure::AutoExposure::OFF,
        ..scrap::post::PostProcess::default()
    }
}

#[test]
fn half_the_pixels_made_up_to_the_screen_are_close_to_all_of_them() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let picture = |upscaling: Upscaling| {
        let mut renderer = Renderer::new(&gpu, &target);
        let frame = scene(&mut renderer, &gpu, post(upscaling));
        for _ in 0..40 {
            renderer.render(&gpu, &target, &frame);
        }
        (target.read_rgba(&gpu), renderer.upscaled())
    };
    let (full, none) = picture(Upscaling::OFF);
    assert_eq!(none, None, "at the screen's own size nothing is made up");
    let half = |method| Upscaling {
        enabled: true,
        scale: 0.5,
        method,
        ..Upscaling::OFF
    };
    // Nearest-neighbour's worth: the picture drawn at half, each pixel
    // doubled — what an upscaler has to beat.
    let (shader, used) = picture(half(Method::Shader));
    assert_eq!(used, Some((Used::Shader, 0.5)));
    let shader_off = difference(&full, &shader);
    eprintln!("engine's own: {shader_off:.2}");
    assert!(shader_off < 6.0, "the engine's own is near: {shader_off:.2}");

    let (auto, used) = picture(half(Method::Auto));
    let auto_off = difference(&full, &auto);
    eprintln!("auto ({used:?}): {auto_off:.2}");
    match used {
        Some((Used::MetalFxTemporal | Used::Dlss, _)) => {
            assert!(auto_off < shader_off, "the temporal upscaler is nearer than the engine's own: {auto_off:.2} vs {shader_off:.2}");
        }
        Some((Used::Shader, _)) => assert_eq!(auto_off, shader_off),
        other => panic!("auto made it up with {other:?}"),
    }
    let (spatial, used) = picture(half(Method::Spatial));
    let spatial_off = difference(&full, &spatial);
    eprintln!("spatial ({used:?}): {spatial_off:.2}");
    assert!(spatial_off < 6.0, "spatial is near: {spatial_off:.2}");
}

#[test]
fn dynamic_resolution_goes_down_when_the_frame_is_too_slow_for_its_target() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    if !gpu.device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        eprintln!("skipping: no GPU timestamps");
        return;
    }
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let scale_after = |target_ms: f32, scale: f32| {
        let mut renderer = Renderer::new(&gpu, &target);
        let frame = scene(
            &mut renderer,
            &gpu,
            post(Upscaling {
                enabled: true,
                scale,
                dynamic: DynamicResolution {
                    enabled: true,
                    target_ms,
                    min_scale: 0.5,
                },
                ..Upscaling::OFF
            }),
        );
        for _ in 0..260 {
            renderer.render(&gpu, &target, &frame);
        }
        renderer.upscaled().map(|(_, s)| s)
    };
    // No frame is drawn in a hundredth of a millisecond: down to the least.
    assert_eq!(scale_after(0.01, 1.0), Some(0.5));
    // A second a frame is time enough: up to the screen's own size.
    let up = scale_after(1000.0, 0.5);
    assert!(up.is_none() || up == Some(1.0), "up to the full size: {up:?}");
}
