//! Split screen (scrap-render's `views`): one frame, two cameras, each in
//! its half of the picture with the whole pipeline — sun and shadows,
//! a lamp, post — and its own aspect; a frame of one view over the whole
//! picture is the picture a frame without views makes.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, FrameView, Lighting, Sky, SkyMode, TextureHandle, ViewRect};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 128;

/// A floor, a red box to the west and a blue one to the east, lit by the
/// sun and a warm lamp.
fn scene(renderer: &mut Renderer, gpu: &Gpu, taa: bool) -> Frame {
    let plane = renderer.upload_mesh_owned(gpu, &builtin::plane(1.0, 1));
    let cube = renderer.upload_mesh_owned(gpu, &builtin::cube(1.0));
    let draw = |mesh, transform, material| Draw { mesh, transform, texture: TextureHandle::WHITE, material, pose: None };
    Frame {
        sky: Sky { mode: SkyMode::Color, ..Default::default() },
        clear_color: Vec3::new(0.05, 0.05, 0.05),
        lighting: Lighting::default(),
        lights: [-10.0, 10.0]
            .map(|x| scrap::render::PointLight {
                position: Vec3::new(x, 2.0, 1.5),
                color: Vec3::new(4.0, 3.2, 2.4),
                range: 8.0,
                spot: None,
                shadows: true,
                falloff: Default::default(),
                inner_cone: None,
            })
            .to_vec(),
        post: scrap::post::PostProcess { enabled: true, fxaa: !taa, taa, ..scrap::post::PostProcess::OFF },
        draws: vec![
            draw(plane, Mat4::from_scale(Vec3::splat(30.0)), Material::new(0.5, 0.5, 0.5)),
            draw(cube, Mat4::from_translation(Vec3::new(-10.0, 0.5, 0.0)), Material::new(0.9, 0.1, 0.1)),
            draw(cube, Mat4::from_translation(Vec3::new(10.0, 0.5, 0.0)), Material::new(0.1, 0.2, 0.9)),
        ],
        ..Frame::default()
    }
}

fn looking_at(x: f32) -> Camera {
    Camera { position: Vec3::new(x, 2.5, 3.5), target: Vec3::new(x, 0.5, 0.0), ..Camera::default() }
}

/// Red minus blue, summed over a part of the picture's columns.
fn redness(pixels: &[u8], columns: std::ops::Range<u32>) -> i64 {
    let mut sum = 0i64;
    for y in 0..HEIGHT {
        for x in columns.clone() {
            let p = ((y * WIDTH + x) * 4) as usize;
            sum += pixels[p] as i64 - pixels[p + 2] as i64;
        }
    }
    sum
}

fn shot(renderer: &mut Renderer, gpu: &Gpu, target: &OffscreenTarget, frame: &Frame, times: usize) -> Vec<u8> {
    for _ in 0..times {
        renderer.render(gpu, target, frame);
    }
    target.read_rgba(gpu)
}

#[test]
fn two_cameras_each_draw_their_half_of_the_picture() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, WIDTH, HEIGHT);
    for taa in [false, true] {
        let mut renderer = Renderer::new(&gpu, &target);
        let mut frame = scene(&mut renderer, &gpu, taa);
        frame.views = vec![
            FrameView::new(looking_at(-10.0), ViewRect::LEFT),
            FrameView::new(looking_at(10.0), ViewRect::RIGHT),
        ];
        // A few frames: TAA's history has gathered, each view its own.
        let pixels = shot(&mut renderer, &gpu, &target, &frame, 4);
        let (left, right) = (redness(&pixels, 0..WIDTH / 2), redness(&pixels, WIDTH / 2..WIDTH));
        eprintln!("taa {taa}: left {left}, right {right}");
        assert!(left > 0, "the left half sees the red box (taa {taa}): {left}");
        assert!(right < 0, "the right half sees the blue box (taa {taa}): {right}");
        if let (Ok(dir), false) = (std::env::var("DUMP"), taa) {
            image::save_buffer(format!("{dir}/split_screen.png"), &pixels, WIDTH, HEIGHT, image::ColorType::Rgba8).unwrap();
        }
        // Swapped: the halves swap.
        frame.views.swap(0, 1);
        frame.views[0].rect = ViewRect::LEFT;
        frame.views[1].rect = ViewRect::RIGHT;
        let pixels = shot(&mut renderer, &gpu, &target, &frame, 4);
        assert!(redness(&pixels, 0..WIDTH / 2) < 0 && redness(&pixels, WIDTH / 2..WIDTH) > 0);
    }
}

#[test]
fn one_view_over_the_whole_picture_is_the_picture_without_views() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(&gpu, &target);
    let mut frame = scene(&mut renderer, &gpu, false);
    frame.camera = looking_at(-10.0);
    let plain = shot(&mut renderer, &gpu, &target, &frame, 2);
    frame.views = vec![FrameView::new(frame.camera, ViewRect::Share { x: 0.0, y: 0.0, width: 1.0, height: 1.0 })];
    let viewed = shot(&mut renderer, &gpu, &target, &frame, 2);
    let worst = plain.iter().zip(&viewed).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 2, "a whole-picture view differs from no views by {worst}");
}

/// A view's own draws are drawn in that view and no other: both cameras
/// look at one spot, and a red box put in the left view's draws is red on
/// the left and absent on the right — the stub of a wall one player sees
/// while the other sees it whole.
#[test]
fn a_views_own_draws_are_seen_in_that_view_alone() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(&gpu, &target);
    let mut frame = scene(&mut renderer, &gpu, false);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let red = Draw {
        mesh: cube,
        transform: Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)),
        texture: TextureHandle::WHITE,
        material: Material::new(0.9, 0.05, 0.05),
        pose: None,
    };
    frame.views = vec![
        FrameView::new(looking_at(0.0), ViewRect::LEFT).with_draws(vec![red]),
        FrameView::new(looking_at(0.0), ViewRect::RIGHT),
    ];
    let pixels = shot(&mut renderer, &gpu, &target, &frame, 2);
    let (left, right) = (redness(&pixels, 0..WIDTH / 2), redness(&pixels, WIDTH / 2..WIDTH));
    eprintln!("left {left}, right {right}");
    assert!(left > right + 1000, "the left view draws its own red box: {left} vs {right}");
    // The frame's own draws are untouched: drawn again without views'
    // draws, the left half is as plain as the right.
    frame.views[0].draws.clear();
    let pixels = shot(&mut renderer, &gpu, &target, &frame, 2);
    let (left, right) = (redness(&pixels, 0..WIDTH / 2), redness(&pixels, WIDTH / 2..WIDTH));
    assert!((left - right).abs() < 1000, "without it the halves agree: {left} vs {right}");
}

/// Volumetric fog is each view's own: the grid of lit air is made again
/// for each view's camera (it keeps no history between frames), so the
/// right half of a split frame is the right camera's picture alone — fog,
/// lamps' glow through it and all — and the left the left's.
#[test]
fn volumetric_fog_in_each_view_is_that_views_own() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let foggy = |frame: &mut Frame| {
        frame.volumetric_fog = scrap::volume::VolumetricFog { enabled: true, density: 0.08, ..scrap::volume::VolumetricFog::OFF };
    };
    let half = OffscreenTarget::new(&gpu, WIDTH / 2, HEIGHT);
    let alone: Vec<Vec<u8>> = [-10.0, 10.0]
        .iter()
        .map(|x| {
            let mut renderer = Renderer::new(&gpu, &half);
            let mut frame = scene(&mut renderer, &gpu, false);
            foggy(&mut frame);
            frame.camera = looking_at(*x);
            shot(&mut renderer, &gpu, &half, &frame, 3)
        })
        .collect();
    // The fog is there to see: the left camera without it differs.
    let clear = {
        let mut renderer = Renderer::new(&gpu, &half);
        let mut frame = scene(&mut renderer, &gpu, false);
        frame.camera = looking_at(-10.0);
        shot(&mut renderer, &gpu, &half, &frame, 3)
    };
    let fogged: u64 = clear.iter().zip(&alone[0]).map(|(a, b)| a.abs_diff(*b) as u64).sum();
    assert!(fogged as f32 / clear.len() as f32 > 2.0, "the fog shows: {fogged}");
    let target = OffscreenTarget::new(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(&gpu, &target);
    let mut frame = scene(&mut renderer, &gpu, false);
    foggy(&mut frame);
    frame.views = vec![FrameView::new(looking_at(-10.0), ViewRect::LEFT), FrameView::new(looking_at(10.0), ViewRect::RIGHT)];
    let split = shot(&mut renderer, &gpu, &target, &frame, 3);
    for (v, picture) in alone.iter().enumerate() {
        let mut sum = 0u64;
        for y in 0..HEIGHT {
            for x in 0..WIDTH / 2 {
                let a = ((y * (WIDTH / 2) + x) * 4) as usize;
                let b = ((y * WIDTH + x + v as u32 * WIDTH / 2) * 4) as usize;
                sum += (0..3).map(|c| picture[a + c].abs_diff(split[b + c]) as u64).sum::<u64>();
            }
        }
        let mean = sum as f32 / (WIDTH / 2 * HEIGHT * 3) as f32;
        eprintln!("view {v}: {mean} off its camera alone");
        assert!(mean < 2.0, "view {v}'s fog is another camera's: {mean} off");
    }
}

/// The upscaler is each view's own: drawn at half the size and made up
/// (the device's best — MetalFX temporal with TAA on an Apple GPU, its
/// history the view's — and the engine's own spatial one), each half of a
/// split frame is the picture its camera makes alone at that render scale,
/// not the other camera's history blended in.
#[test]
fn the_upscaler_in_each_view_is_that_views_own() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    for method in [scrap::upscale::Method::Auto, scrap::upscale::Method::Shader] {
        let scaled = |frame: &mut Frame| {
            frame.post.upscaling = scrap::upscale::Upscaling { enabled: true, scale: 0.5, method, ..scrap::upscale::Upscaling::OFF };
        };
        let half = OffscreenTarget::new(&gpu, WIDTH / 2, HEIGHT);
        let alone: Vec<Vec<u8>> = [-10.0, 10.0]
            .iter()
            .map(|x| {
                let mut renderer = Renderer::new(&gpu, &half);
                let mut frame = scene(&mut renderer, &gpu, true);
                scaled(&mut frame);
                frame.camera = looking_at(*x);
                shot(&mut renderer, &gpu, &half, &frame, 8)
            })
            .collect();
        let target = OffscreenTarget::new(&gpu, WIDTH, HEIGHT);
        let mut renderer = Renderer::new(&gpu, &target);
        let mut frame = scene(&mut renderer, &gpu, true);
        scaled(&mut frame);
        frame.views = vec![FrameView::new(looking_at(-10.0), ViewRect::LEFT), FrameView::new(looking_at(10.0), ViewRect::RIGHT)];
        let split = shot(&mut renderer, &gpu, &target, &frame, 8);
        eprintln!("{method:?}: upscaled by {:?}", renderer.upscaled());
        for (v, picture) in alone.iter().enumerate() {
            let mut sum = 0u64;
            for y in 0..HEIGHT {
                for x in 0..WIDTH / 2 {
                    let a = ((y * (WIDTH / 2) + x) * 4) as usize;
                    let b = ((y * WIDTH + x + v as u32 * WIDTH / 2) * 4) as usize;
                    sum += (0..3).map(|c| picture[a + c].abs_diff(split[b + c]) as u64).sum::<u64>();
                }
            }
            let mean = sum as f32 / (WIDTH / 2 * HEIGHT * 3) as f32;
            eprintln!("{method:?} view {v}: {mean} off its camera alone");
            assert!(mean < 0.25, "{method:?}: view {v} is upscaled from another camera's picture: {mean} off");
        }
    }
}
