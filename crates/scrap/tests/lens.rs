//! The lens: depth of field keeps what is in focus sharp and softens what
//! is not, and motion blur smears an edge the camera turns across.

use scrap::glam::{Mat4, Vec3};
use scrap::lens::{DepthOfField, FocusMode, MotionBlur};
use scrap::material::Shading;
use scrap::post::PostProcess;
use scrap::render::{
    Camera, Draw, Frame, MeshHandle, ShadowSettings, Sky, SkyMode, TextureHandle,
};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 96;

fn white(mesh: MeshHandle, transform: Mat4) -> Draw {
    Draw {
        mesh,
        transform,
        texture: TextureHandle::WHITE,
        material: Material {
            shading: Shading::Unlit,
            ..Material::new(1.0, 1.0, 1.0)
        },
        pose: None,
    }
}

fn frame(camera: Camera, draws: Vec<Draw>, post: PostProcess) -> Frame {
    Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        // Nothing else touches a colour: the lens alone.
        post: PostProcess {
            enabled: true,
            ..post
        },
        ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
        camera,
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        draws,
        ..Frame::default()
    }
}

fn plain() -> PostProcess {
    PostProcess {
        enabled: true,
        ..PostProcess::OFF
    }
}

/// How far from black to white a row goes over a span of columns: the
/// contrast of stripes there.
fn contrast(pixels: &[u8], row: u32, columns: std::ops::Range<u32>) -> i32 {
    let values: Vec<i32> = columns
        .map(|x| OffscreenTarget::pixel(pixels, SIZE, x, row)[0] as i32)
        .collect();
    values.iter().max().unwrap() - values.iter().min().unwrap()
}

#[test]
fn bokeh_blurs_the_far_stripes_and_keeps_the_near_ones_in_focus() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    // Thin white bars: near ones in the bottom half of the view, far ones
    // in the top half, each a bar width apart at their own distance.
    let mut draws = Vec::new();
    for i in -6..=6 {
        let near = Vec3::new(i as f32 * 0.12, -0.35, -3.0);
        draws.push(white(
            cube,
            Mat4::from_translation(near) * Mat4::from_scale(Vec3::new(0.06, 0.5, 0.06)),
        ));
        let far = Vec3::new(i as f32 * 0.6, 3.5, -30.0);
        draws.push(white(
            cube,
            Mat4::from_translation(far) * Mat4::from_scale(Vec3::new(0.3, 5.0, 0.3)),
        ));
    }
    let camera = Camera {
        position: Vec3::ZERO,
        target: Vec3::new(0.0, 0.0, -1.0),
        fov_y_degrees: 30.0,
        ..Camera::default()
    };
    let shoot = |renderer: &mut Renderer, post: PostProcess| {
        renderer.render(&gpu, &target, &frame(camera, draws.clone(), post));
        target.read_rgba(&gpu)
    };
    let (near_row, far_row) = (SIZE * 3 / 4, SIZE / 4);
    let span = SIZE / 4..SIZE * 3 / 4;
    let sharp = shoot(&mut renderer, plain());
    let focused = shoot(
        &mut renderer,
        PostProcess {
            depth_of_field: DepthOfField {
                mode: FocusMode::Bokeh,
                focus_distance: 3.0,
                focal_length: 85.0,
                aperture: 1.4,
                ..Default::default()
            },
            ..plain()
        },
    );
    let (near_before, far_before) = (
        contrast(&sharp, near_row, span.clone()),
        contrast(&sharp, far_row, span.clone()),
    );
    let (near_after, far_after) = (
        contrast(&focused, near_row, span.clone()),
        contrast(&focused, far_row, span.clone()),
    );
    assert!(
        near_before > 200 && far_before > 200,
        "{near_before} {far_before}"
    );
    assert!(
        near_after > 200,
        "in focus, the near bars stay sharp: {near_after}"
    );
    assert!(
        far_after < far_before / 2,
        "the far ones blur: {far_before} -> {far_after}"
    );
}

#[test]
fn motion_blur_smears_an_edge_the_camera_turns_across() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    // A white wall filling the left of the view, black on the right.
    let draws = vec![white(
        cube,
        Mat4::from_translation(Vec3::new(-5.0, 0.0, -10.0)) * Mat4::from_scale(Vec3::splat(10.0)),
    )];
    let looking = |x: f32| Camera {
        position: Vec3::ZERO,
        target: Vec3::new(x, 0.0, -1.0),
        ..Camera::default()
    };
    // Grey pixels along the middle row: the width of the edge.
    let edge = |renderer: &mut Renderer, post: PostProcess| {
        renderer.render(&gpu, &target, &frame(looking(0.0), draws.clone(), post));
        renderer.render(&gpu, &target, &frame(looking(0.08), draws.clone(), post));
        let pixels = target.read_rgba(&gpu);
        (0..SIZE)
            .filter(|&x| {
                let v = OffscreenTarget::pixel(&pixels, SIZE, x, SIZE / 2)[0];
                (20..235).contains(&v)
            })
            .count()
    };
    let still = edge(&mut renderer, plain());
    let blurred = edge(
        &mut renderer,
        PostProcess {
            motion_blur: MotionBlur {
                intensity: 1.0,
                clamp: 0.2,
                samples: 16,
            },
            ..plain()
        },
    );
    assert!(still <= 2, "without blur the edge is sharp: {still} px");
    assert!(blurred >= 4, "turning, it smears: {blurred} px");
}

/// Vertical white bars across the view, at `post`.
fn bars(post: PostProcess, fov: f32) -> Vec<u8> {
    let gpu = Gpu::headless_blocking(false).unwrap();
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let draws = (-10..=10)
        .map(|i| {
            white(
                cube,
                Mat4::from_translation(Vec3::new(i as f32 * 0.8, 0.0, -5.0))
                    * Mat4::from_scale(Vec3::new(0.3, 20.0, 0.3)),
            )
        })
        .collect();
    let camera = Camera {
        position: Vec3::ZERO,
        target: Vec3::new(0.0, 0.0, -1.0),
        fov_y_degrees: fov,
        ..Camera::default()
    };
    renderer.render(&gpu, &target, &frame(camera, draws, post));
    target.read_rgba(&gpu)
}

/// Columns that differ between two pictures along a row.
fn differing(a: &[u8], b: &[u8], row: u32) -> Vec<u32> {
    (0..SIZE)
        .filter(|&x| {
            let p = OffscreenTarget::pixel(a, SIZE, x, row)[0] as i32;
            let q = OffscreenTarget::pixel(b, SIZE, x, row)[0] as i32;
            (p - q).abs() > 60
        })
        .collect()
}

#[test]
fn lens_distortion_and_panini_bend_the_edges_and_leave_the_middle() {
    if Gpu::headless_blocking(false).is_err() {
        eprintln!("skipping: no adapter");
        return;
    }
    let straight = bars(plain(), 90.0);
    let barrel = bars(
        PostProcess {
            lens_distortion: scrap::post::LensDistortion {
                intensity: 0.6,
                ..Default::default()
            },
            ..plain()
        },
        90.0,
    );
    let panini = bars(
        PostProcess {
            panini_projection: scrap::post::PaniniProjection {
                distance: 1.0,
                crop_to_fit: 1.0,
            },
            ..plain()
        },
        90.0,
    );
    for (name, bent) in [("distortion", &barrel), ("panini", &panini)] {
        let changed = differing(&straight, bent, SIZE / 2);
        assert!(changed.len() >= 4, "{name} moves the bars: {changed:?}");
        let middle = SIZE / 2 - 2..SIZE / 2 + 2;
        assert!(
            changed.iter().all(|x| !middle.contains(x)),
            "{name} leaves the middle where it was: {changed:?}"
        );
    }
    assert_eq!(bars(plain(), 90.0), straight, "the same frame twice");
}

#[test]
fn a_lens_flare_throws_a_ghost_across_the_middle() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let sphere = renderer.upload_mesh_owned(&gpu, &builtin::sphere(0.5, 16, 8));
    // A very bright light up and to the left.
    let sun = Draw {
        material: Material {
            shading: Shading::Unlit,
            emission: [40.0, 40.0, 40.0],
            ..Material::new(1.0, 1.0, 1.0)
        },
        ..white(sphere, Mat4::from_translation(Vec3::new(-1.6, 1.6, -5.0)))
    };
    let camera = Camera {
        position: Vec3::ZERO,
        target: Vec3::new(0.0, 0.0, -1.0),
        ..Camera::default()
    };
    let bloom = scrap::post::Bloom {
        intensity: 1.0,
        ..Default::default()
    };
    let shoot = |renderer: &mut Renderer, post: PostProcess| {
        renderer.render(&gpu, &target, &frame(camera, vec![sun], post));
        target.read_rgba(&gpu)
    };
    let without = shoot(&mut renderer, PostProcess { bloom, ..plain() });
    let with = shoot(
        &mut renderer,
        PostProcess {
            bloom,
            lens_flare: scrap::post::LensFlare {
                intensity: 1.0,
                streaks: 0.0,
                ..Default::default()
            },
            ..plain()
        },
    );
    // Mirrored through the middle: down and to the right.
    let sum = |p: &[u8]| -> u32 {
        (SIZE * 5 / 8..SIZE * 7 / 8)
            .flat_map(|y| (SIZE * 5 / 8..SIZE * 7 / 8).map(move |x| (x, y)))
            .map(|(x, y)| OffscreenTarget::pixel(p, SIZE, x, y)[1] as u32)
            .sum()
    };
    assert!(
        sum(&with) > sum(&without) + 200,
        "a ghost opposite the light: {} -> {}",
        sum(&without),
        sum(&with)
    );
}

#[test]
fn hot_air_shimmers_far_off_and_mirrors_the_sky_at_the_horizon() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    // Brown ground to the horizon under a blue sky, looking level.
    let ground = Draw {
        material: Material {
            shading: Shading::Unlit,
            ..Material::new(0.5, 0.3, 0.1)
        },
        ..white(plane, Mat4::from_scale(Vec3::new(2000.0, 1.0, 2000.0)))
    };
    let shoot = |renderer: &mut Renderer, heat: scrap::lens::HeatHaze, time: f32| {
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Procedural,
                zenith: [0.1, 0.3, 0.9],
                horizon: [0.2, 0.5, 1.0],
                sun_size: 0.0,
                ..Default::default()
            },
            post: PostProcess {
                heat_haze: heat,
                ..plain()
            },
            ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
            camera: Camera {
                // High enough that just below the horizon is 150 m off.
                position: Vec3::new(0.0, 5.0, 0.0),
                target: Vec3::new(0.0, 5.0, -10.0),
                ..Camera::default()
            },
            shadows: ShadowSettings::OFF,
            draws: vec![ground],
            time: Some(time),
            fog: scrap::render::FogSettings {
                start: 5000.0,
                end: 6000.0,
                ..Default::default()
            },
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        target.read_rgba(&gpu)
    };
    // Just below the horizon: the middle row is the horizon.
    let low = |p: &[u8]| OffscreenTarget::pixel(p, SIZE, SIZE / 2, SIZE / 2 + 2);
    let clear = shoot(&mut renderer, scrap::lens::HeatHaze::OFF, 0.0);
    let mirage = shoot(
        &mut renderer,
        scrap::lens::HeatHaze {
            mirage: 1.0,
            ..scrap::lens::HeatHaze::OFF
        },
        0.0,
    );
    assert!(
        low(&clear)[0] > low(&clear)[2],
        "brown ground there: {:?}",
        low(&clear)
    );
    assert!(
        low(&mirage)[2] > low(&mirage)[0],
        "the sky mirrored there: {:?}",
        low(&mirage)
    );

    let shimmer = scrap::lens::HeatHaze {
        intensity: 1.0,
        ..scrap::lens::HeatHaze::OFF
    };
    let a = shoot(&mut renderer, shimmer, 0.0);
    let b = shoot(&mut renderer, shimmer, 0.7);
    assert_ne!(a, b, "the air moves");
    let near = |p: &[u8]| OffscreenTarget::pixel(p, SIZE, SIZE / 2, SIZE - 2);
    assert_eq!(near(&a), near(&clear), "and not at your feet");
}
