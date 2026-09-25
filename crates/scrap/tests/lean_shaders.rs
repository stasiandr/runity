//! The lean lit shaders (`LEAN` in render.wgsl, scrap-render's `lean`):
//! a frame that asks for none of what they leave out is drawn by them, and
//! looks the same as by the standard ones; a frame that asks for any of it
//! — here, wet ground — is not.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, Lighting, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 160;

fn scene(renderer: &mut Renderer, gpu: &Gpu) -> Frame {
    let plane = renderer.upload_mesh_owned(gpu, &builtin::plane(1.0, 1));
    let cube = renderer.upload_mesh_owned(gpu, &builtin::cube(1.0));
    let draw = |mesh, transform, material| Draw { mesh, transform, texture: TextureHandle::WHITE, material, pose: None };
    let mut shiny = Material::new(0.8, 0.3, 0.2);
    shiny.smoothness = 0.8;
    Frame {
        // Unity's procedural sky, reflected by the shiny box.
        sky: Sky { mode: SkyMode::Procedural, ..Default::default() },
        camera: Camera { position: Vec3::new(0.0, 2.0, 5.0), target: Vec3::new(0.0, 0.5, 0.0), ..Camera::default() },
        lighting: Lighting::default(),
        // Contact shadows march the depth: the lean shaders leave them out.
        shadows: scrap::render::ShadowSettings { contact: 0.0, ..Default::default() },
        draws: vec![
            draw(plane, Mat4::from_scale(Vec3::splat(20.0)), Material::new(0.6, 0.55, 0.5)),
            draw(cube, Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)), shiny),
        ],
        ..Frame::default()
    }
}

/// Frames drawn until the lean pipelines are in (they are built on a
/// thread of their own), or a while has passed.
fn settle(renderer: &mut Renderer, gpu: &Gpu, target: &OffscreenTarget, frame: &Frame) -> usize {
    let start = std::time::Instant::now();
    let mut last = 0;
    loop {
        renderer.render(gpu, target, frame);
        let _ = target.read_rgba(gpu);
        let now = renderer.lean_pipelines();
        if (now > 0 && now == last) || start.elapsed().as_secs() > 600 {
            return now;
        }
        last = now;
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[test]
fn a_frame_that_needs_nothing_the_lean_shaders_leave_out_is_drawn_by_them_and_looks_the_same() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let frame = scene(&mut renderer, &gpu);
    // Post (TAA's jitter, grain) off: two frames of the same picture.
    let frame = Frame { post: scrap::post::PostProcess::OFF, ..frame };

    renderer.set_lean_shaders(false);
    for _ in 0..3 {
        renderer.render(&gpu, &target, &frame);
    }
    let standard = target.read_rgba(&gpu);
    assert_eq!(renderer.lean_pipelines(), 0);

    renderer.set_lean_shaders(true);
    let lean = settle(&mut renderer, &gpu, &target, &frame);
    assert!(lean > 0, "the lean pipelines never came in");
    renderer.render(&gpu, &target, &frame);
    let leaned = target.read_rgba(&gpu);
    let worst = standard.iter().zip(&leaned).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
    assert!(worst <= 2, "the lean picture differs from the standard one by {worst}");

    // Wet ground is weather the lean shaders leave out: drawn standard.
    let wet = Frame { weather: scrap::weather::Weather { wetness: 1.0, ..Default::default() }, ..frame };
    renderer.render(&gpu, &target, &wet);
    assert_eq!(renderer.lean_pipelines(), 0, "a wet frame must not be drawn lean");
}
