//! The lens: depth of field keeps what is in focus sharp and softens what
//! is not, and motion blur smears an edge the camera turns across.

use runity::glam::{Mat4, Vec3};
use runity::lens::{DepthOfField, FocusMode, MotionBlur};
use runity::material::Shading;
use runity::post::PostProcess;
use runity::render::{
    Camera, Draw, Frame, MeshHandle, ShadowSettings, Sky, SkyMode, TextureHandle,
};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

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
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
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
