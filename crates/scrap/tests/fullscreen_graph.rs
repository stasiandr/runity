//! A fullscreen graph in a project's `shaders/` (`<name>.post.ron`): found
//! and put in by the same polling as the material shaders, and a frame
//! naming it is drawn through it — the picture's colour read and changed,
//! its depth read — before the post-processing. Broken, refused in words,
//! and the last one keeps drawing.

use scrap::fullscreen::FullscreenPass;
use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, MaterialShaders, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 64;

fn write(path: &std::path::Path, text: &str, second: u64) {
    std::fs::write(path, text).unwrap();
    let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + second);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
}

#[test]
fn a_fullscreen_graph_reads_and_changes_the_picture_and_its_depth() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let dir = std::env::temp_dir().join(format!("scrap-fullscreen-graph-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("swap.post.ron");
    // Red and green swapped; past ten metres, the property's colour.
    write(
        &file,
        r#"(
            params: [("far_colour", Color)],
            nodes: {
                "swapped": Combine(x: "color.g", y: "color.r", z: "color.b"),
                "far": Step(edge: 10.0, of: "depth"),
                "out": Lerp(a: "swapped", b: "far_colour", t: "far"),
            },
            output: (color: "out"),
        )"#,
        0,
    );
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let mut shaders = MaterialShaders::new(&dir);
    let put = shaders.poll(&mut renderer, &gpu);
    assert_eq!(put.len(), 1, "{put:?}");
    assert_eq!(put[0].0, "swap", "named by the file, less `.post.ron`");
    put[0].1.as_ref().unwrap();
    assert!(renderer.has_fullscreen_graph("swap"));

    // A red, unlit ball four metres off, over nothing (the far plane).
    let ball = renderer.upload_mesh_owned(&gpu, &builtin::sphere(1.0, 24, 16));
    let mut frame = Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: scrap::post::PostProcess::OFF,
        clear_color: Vec3::ZERO,
        camera: Camera {
            position: Vec3::new(0.0, 0.0, 4.0),
            target: Vec3::ZERO,
            far: 100.0,
            ..Camera::default()
        },
        draws: vec![Draw {
            mesh: ball,
            transform: Mat4::IDENTITY,
            texture: TextureHandle::WHITE,
            material: Material {
                shading: scrap::material::Shading::Unlit,
                ..Material::new(1.0, 0.0, 0.0)
            },
            pose: None,
        }],
        ..Frame::default()
    };
    renderer.render(&gpu, &target, &frame);
    let centre = |target: &OffscreenTarget| {
        let pixels = target.read_rgba(&gpu);
        (
            OffscreenTarget::pixel(&pixels, SIZE, SIZE / 2, SIZE / 2),
            OffscreenTarget::pixel(&pixels, SIZE, 2, 2),
        )
    };
    let (ball_seen, sky_seen) = centre(&target);
    assert!(ball_seen[0] > 150 && ball_seen[1] < 30, "without the graph, red: {ball_seen:?}");
    assert!(sky_seen[2] < 30, "and black round it: {sky_seen:?}");

    frame.fullscreen = Some(FullscreenPass {
        graph: "swap".into(),
        params: vec![0.0, 0.0, 1.0],
    });
    renderer.render(&gpu, &target, &frame);
    let (ball_seen, sky_seen) = centre(&target);
    assert!(ball_seen[1] > 150 && ball_seen[0] < 30, "through it, green: {ball_seen:?}");
    assert!(sky_seen[2] > 150, "and past ten metres, the property's blue: {sky_seen:?}");

    // Misspelt: refused, the last one draws on.
    write(&file, r#"(nodes: { "c": Multiply(a: "colr", b: 2.0) }, output: (color: "c"))"#, 1);
    let put = shaders.poll(&mut renderer, &gpu);
    let words = put[0].1.as_ref().expect_err("a misspelt input is refused");
    assert!(words.contains("swap.post.ron") && words.contains("did you mean `color`"), "{words}");
    renderer.render(&gpu, &target, &frame);
    assert!(centre(&target).0[1] > 150, "still green");

    // A name nothing answers to: the picture as it was.
    frame.fullscreen = Some(FullscreenPass {
        graph: "nothing".into(),
        params: Vec::new(),
    });
    renderer.render(&gpu, &target, &frame);
    assert!(centre(&target).0[0] > 150, "red again");
    let _ = std::fs::remove_dir_all(&dir);
}
