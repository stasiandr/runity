//! Screen-space reflections: a mirror floor shows the red block standing
//! on it — no probe, only what is on the screen — and does not without
//! them, or on the very first frame, before there is a last one to read.

use runity::glam::{Mat4, Vec3};
use runity::material::Shading;
use runity::reflections::ScreenSpaceReflections;
use runity::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 64;

#[test]
fn a_mirror_floor_shows_the_block_on_it() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let plane = builtin::plane(1.0, 1);
    let cube = builtin::cube(1.0);
    let shoot = |ssr: bool, frames: u32| {
        let mut renderer = Renderer::new(&gpu, &target);
        let plane = renderer.upload_mesh_owned(&gpu, &plane);
        let cube = renderer.upload_mesh_owned(&gpu, &cube);
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: runity::post::PostProcess::OFF,
            camera: Camera {
                position: Vec3::new(0.0, 1.2, 5.0),
                target: Vec3::new(0.0, 0.3, 0.0),
                ..Camera::default()
            },
            lighting: Lighting {
                sky_color: Vec3::ZERO,
                ground_color: Vec3::ZERO,
                ..Lighting::default()
            },
            shadows: ShadowSettings::OFF,
            clear_color: Vec3::ZERO,
            draws: vec![
                Draw {
                    mesh: plane,
                    transform: Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0)),
                    texture: TextureHandle::WHITE,
                    material: Material {
                        metallic: 1.0,
                        smoothness: 1.0,
                        ..Material::new(0.9, 0.9, 0.9)
                    },
                    pose: None,
                },
                Draw {
                    mesh: cube,
                    transform: Mat4::from_translation(Vec3::new(0.0, 0.75, -1.0))
                        * Mat4::from_scale(Vec3::splat(1.5)),
                    texture: TextureHandle::WHITE,
                    material: Material {
                        shading: Shading::Unlit,
                        ..Material::new(1.0, 0.0, 0.0)
                    },
                    pose: None,
                },
            ],
            screen_space_reflections: ScreenSpaceReflections {
                enabled: ssr,
                ..Default::default()
            },
            ..Frame::default()
        };
        for _ in 0..frames {
            renderer.render(&gpu, &target, &frame);
        }
        target.read_rgba(&gpu)
    };
    // Below the block on the screen: its reflection in the floor.
    let red_below = |p: &[u8]| {
        (SIZE * 5 / 8..SIZE * 7 / 8)
            .map(|y| OffscreenTarget::pixel(p, SIZE, SIZE / 2, y)[0] as u32)
            .max()
            .unwrap_or(0)
    };
    let without = shoot(false, 2);
    let first = shoot(true, 1);
    let with = shoot(true, 2);
    assert!(
        red_below(&without) < 30,
        "no reflection without: {}",
        red_below(&without)
    );
    assert!(
        red_below(&first) < 30,
        "none before a last frame: {}",
        red_below(&first)
    );
    assert!(
        red_below(&with) > 100,
        "the block in the floor: {}",
        red_below(&with)
    );
}

/// A ball standing on a mirror floor, a few metres off: its reflection is
/// whole, not cut into bands where the ray's long steps went through it.
#[test]
fn a_ball_is_reflected_whole_and_not_in_bands() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    const WIDE: u32 = 128;
    let target = OffscreenTarget::new(&gpu, WIDE, WIDE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let ball = renderer.upload_mesh_owned(&gpu, &builtin::sphere(1.0, 32, 16));
    let frame = Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        camera: Camera {
            position: Vec3::new(0.0, 2.4, 7.5),
            target: Vec3::new(0.0, 0.3, 0.0),
            ..Camera::default()
        },
        lighting: Lighting {
            sky_color: Vec3::ZERO,
            ground_color: Vec3::ZERO,
            ..Lighting::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        draws: vec![
            Draw {
                mesh: plane,
                transform: Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0)),
                texture: TextureHandle::WHITE,
                material: Material {
                    metallic: 1.0,
                    smoothness: 1.0,
                    ..Material::new(0.9, 0.9, 0.9)
                },
                pose: None,
            },
            Draw {
                mesh: ball,
                transform: Mat4::from_translation(Vec3::new(0.0, 0.8, 0.5))
                    * Mat4::from_scale(Vec3::splat(1.6)),
                texture: TextureHandle::WHITE,
                material: Material {
                    shading: Shading::Unlit,
                    ..Material::new(1.0, 0.0, 0.0)
                },
                pose: None,
            },
        ],
        screen_space_reflections: ScreenSpaceReflections {
            enabled: true,
            ..Default::default()
        },
        ..Frame::default()
    };
    for _ in 0..3 {
        renderer.render(&gpu, &target, &frame);
    }
    let pixels = target.read_rgba(&gpu);
    // Below the ball, row by row across its reflection: how many pixels
    // between a row's first red one and its last are not red — holes where
    // the ray's steps went through the ball.
    let red = |x: u32, y: u32| OffscreenTarget::pixel(&pixels, WIDE, x, y)[0] > 60;
    let (mut spans, mut holes) = (0u32, 0u32);
    for y in WIDE * 11 / 20..WIDE {
        let reds: Vec<u32> = (0..WIDE).filter(|&x| red(x, y)).collect();
        let (Some(&first), Some(&last)) = (reds.first(), reds.last()) else {
            continue;
        };
        spans += last - first + 1;
        holes += (last - first + 1) - reds.len() as u32;
    }
    assert!(spans > 300, "hardly any reflection: {spans} pixels");
    assert!(
        holes * 100 < spans,
        "{holes} holes in {spans} pixels of the reflection"
    );
}
