//! The Frame Debugger on a real frame: every pass and draw listed by name,
//! and stopped at a draw, the picture of its pass as it stood then — what
//! was drawn after it is not in it.

use scrap::frame_debugger::Step;
use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, Lighting, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 128;

fn frame(renderer: &mut Renderer, gpu: &Gpu) -> Frame {
    let mut cube = builtin::cube(1.0);
    cube.name = "red box".into();
    let red_box = renderer.upload_mesh_owned(gpu, &cube);
    let mut cube = builtin::cube(1.0);
    cube.name = "blue box".into();
    let blue_box = renderer.upload_mesh_owned(gpu, &cube);
    let draw = |mesh, x: f32, colour: Material| Draw {
        mesh,
        transform: Mat4::from_translation(Vec3::new(x, 0.0, 0.0)),
        texture: TextureHandle::WHITE,
        material: colour,
        pose: None,
    };
    Frame {
        sky: Sky { mode: SkyMode::Color, ..Default::default() },
        post: scrap::post::PostProcess::OFF,
        camera: Camera { position: Vec3::new(0.0, 0.5, 4.0), target: Vec3::ZERO, ..Camera::default() },
        lighting: Lighting::default(),
        clear_color: Vec3::ZERO,
        draws: vec![draw(red_box, -1.0, Material::new(1.0, 0.0, 0.0)), draw(blue_box, 1.0, Material::new(0.0, 0.0, 1.0))],
        ..Frame::default()
    }
}

/// How much of the picture's left and right halves is lit.
fn halves(pixels: &[u8], width: u32, height: u32) -> (usize, usize) {
    let mut out = (0, 0);
    for y in 0..height {
        for x in 0..width {
            let p = OffscreenTarget::pixel(pixels, width, x, y);
            if p[0] as u32 + p[1] as u32 + p[2] as u32 > 30 {
                if x < width / 2 {
                    out.0 += 1;
                } else {
                    out.1 += 1;
                }
            }
        }
    }
    out
}

#[test]
fn a_frame_is_listed_pass_by_pass_and_stopped_at_a_draw_shows_what_was_drawn_by_then() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let frame = frame(&mut renderer, &gpu);

    // Nothing asked: nothing recorded.
    renderer.render(&gpu, &target, &frame);
    assert!(renderer.frame_capture().is_none());

    renderer.debug_frame(None);
    renderer.render(&gpu, &target, &frame);
    let capture = renderer.frame_capture().expect("recorded").clone();
    let text = capture.text();
    eprintln!("{text}");
    let passes: Vec<&str> = capture.passes().iter().map(|p| p.0).collect();
    assert!(passes.contains(&"shadows"), "{text}");
    assert!(passes.contains(&"scene"), "{text}");
    let scene_draws: Vec<(usize, String)> = capture
        .events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.pass == "scene")
        .filter_map(|(i, e)| match &e.step {
            Step::Draw(d) => Some((i, d.what.clone())),
            Step::Pass(_) => None,
        })
        .collect();
    let names: Vec<&str> = scene_draws.iter().map(|d| d.1.as_str()).collect();
    assert!(names.contains(&"red box") && names.contains(&"blue box"), "the draws by their meshes' names: {text}");
    assert!(capture.triangles() > 0);
    assert!(capture.picture.is_none(), "not stopped: the frame is the picture");

    // Stopped at the scene's first box: the picture has it and not the other.
    let (first, first_name) = scene_draws[0].clone();
    renderer.debug_frame(Some(first));
    renderer.render(&gpu, &target, &frame);
    let capture = renderer.frame_capture().unwrap().clone();
    let picture = capture.picture.clone().expect("a picture of the scene pass");
    assert_eq!(picture.pass, "scene");
    assert_eq!(picture.target, "hdr");
    let (pixels, (w, h)) = renderer.read_debug_picture(&gpu).expect("the picture");
    let (left, right) = halves(&pixels, w, h);
    eprintln!("stopped at {first_name}: lit left {left}, right {right}");
    if first_name == "red box" {
        assert!(left > 200 && right < 20, "only the red box, on the left: {left} {right}");
    } else {
        assert!(right > 200 && left < 20, "only the blue box, on the right: {left} {right}");
    }

    // Stopped at the scene pass itself: the whole pass.
    let scene = capture.events.iter().position(|e| e.pass == "scene" && matches!(e.step, Step::Pass(_))).unwrap();
    renderer.debug_frame(Some(scene));
    renderer.render(&gpu, &target, &frame);
    let (pixels, (w, h)) = renderer.read_debug_picture(&gpu).expect("the picture");
    let (left, right) = halves(&pixels, w, h);
    assert!(left > 200 && right > 200, "both boxes: {left} {right}");

    // Stopped in a shadow cascade: its depth.
    let shadow = capture.events.iter().position(|e| e.pass == "shadows").unwrap();
    renderer.debug_frame(Some(shadow));
    renderer.render(&gpu, &target, &frame);
    let capture = renderer.frame_capture().unwrap();
    assert_eq!(capture.picture.as_ref().map(|p| p.target), Some("depth"));

    // Done: nothing recorded, and every draw drawn again.
    renderer.stop_debugging();
    renderer.render(&gpu, &target, &frame);
    assert!(renderer.frame_capture().is_none());
    let (left, right) = halves(&target.read_rgba(&gpu), SIZE, SIZE);
    assert!(left > 200 && right > 200, "{left} {right}");
}
