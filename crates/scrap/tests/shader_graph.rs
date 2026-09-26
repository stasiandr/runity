//! A shader graph in a project's `shaders/`: found and put in by the same
//! polling that puts in hand-written shaders, drawn with the material's own
//! numbers, taken in again when the file changes, refused in words — the
//! node and its input — when broken, while the last one that built keeps
//! drawing.

use scrap::glam::{Mat4, Vec3};
use scrap::material::Shading;
use scrap::render::{Camera, Draw, Frame, MaterialShaders, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 32;

fn write(path: &std::path::Path, text: &str, second: u64) {
    std::fs::write(path, text).unwrap();
    // Each write a second apart as the file system sees it: the polling
    // goes by modification time.
    let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + second);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
}

#[test]
fn a_shader_graph_in_the_projects_shaders_draws_reloads_and_is_refused_in_words() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let dir = std::env::temp_dir().join(format!("scrap-shader-graph-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("paint.graph.ron");
    write(
        &file,
        r#"(
            params: ["red", "green"],
            nodes: { "paint": Combine(x: "red", y: "green", z: 0.0) },
            surface: (albedo: "paint"),
        )"#,
        0,
    );
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(2.0));
    let mut shaders = MaterialShaders::new(&dir);
    let put = shaders.poll(&mut renderer, &gpu);
    assert_eq!(put.len(), 1, "{put:?}");
    assert_eq!(put[0].0, "paint", "named by the file, less `.graph.ron`");
    put[0].1.as_ref().unwrap();
    assert!(shaders.poll(&mut renderer, &gpu).is_empty(), "nothing new, nothing done");

    let mut params = [0.0; 8];
    params[0] = 1.0;
    let frame = Frame {
        camera: Camera {
            position: Vec3::new(0.0, 0.0, 4.0),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        sky: scrap::render::Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        clear_color: Vec3::ZERO,
        post: scrap::post::PostProcess::OFF,
        ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
        draws: vec![Draw {
            mesh: cube,
            transform: Mat4::IDENTITY,
            texture: TextureHandle::WHITE,
            material: Material {
                shading: Shading::Unlit,
                shader: Some(scrap::asset::shader_id("paint")),
                params,
                ..Material::new(1.0, 1.0, 1.0)
            },
            pose: None,
        }],
        ..Frame::default()
    };
    let middle = |renderer: &mut Renderer| {
        renderer.render(&gpu, &target, &frame);
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
    };
    let red = middle(&mut renderer);
    assert!(red[0] > 200 && red[1] < 40 && red[2] < 40, "the material's `red` is 1: {red:?}");

    // Edited on disk: the next poll takes it in.
    write(
        &file,
        r#"(
            params: ["red", "green"],
            nodes: { "paint": Combine(x: "green", y: "red", z: 0.0) },
            surface: (albedo: "paint"),
        )"#,
        1,
    );
    let put = shaders.poll(&mut renderer, &gpu);
    put[0].1.as_ref().unwrap();
    let green = middle(&mut renderer);
    assert!(green[1] > 200 && green[0] < 40, "swapped: green {green:?}");

    // A misspelt input: refused, naming the node and the input, and the
    // last one keeps drawing.
    write(
        &file,
        r#"(
            params: ["red", "green"],
            nodes: { "paint": Combine(x: "gren", y: "red", z: 0.0) },
            surface: (albedo: "paint"),
        )"#,
        2,
    );
    let put = shaders.poll(&mut renderer, &gpu);
    let words = put[0].1.as_ref().expect_err("a misspelt input is refused");
    assert!(words.contains("paint.graph.ron"), "{words}");
    assert!(words.contains("node `paint`, input `x`") && words.contains("did you mean `green`"), "{words}");
    assert_eq!(middle(&mut renderer), green, "the last one that built keeps drawing");

    // One shader, one file.
    write(&dir.join("paint.wgsl"), "fn surface(in: SurfaceIn, out: Surface) -> Surface { return out; }", 3);
    write(
        &file,
        r#"(nodes: {}, surface: (albedo: (1.0, 1.0, 1.0)))"#,
        4,
    );
    let put = shaders.poll(&mut renderer, &gpu);
    let graph = put.iter().find(|(_, r)| r.as_ref().is_err_and(|e| e.contains("graph.ron"))).expect("the graph refused");
    assert!(graph.1.as_ref().unwrap_err().contains("one shader, one file"), "{put:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// What a graph reads of the scene — the depth behind it, last frame's
/// picture, where it is on the screen, which side is seen, the tangent —
/// builds and draws on an opaque surface as on a see-through one.
#[test]
fn a_graph_reading_the_scene_draws() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let dir = std::env::temp_dir().join(format!("scrap-shader-graph-scene-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(
        &dir.join("seen.graph.ron"),
        r#"(
            nodes: {
                "behind": SceneColor(),
                "deep": Subtract(a: "scene_depth", b: "depth"),
                "edge": Saturate(of: "deep"),
                "lit": Multiply(a: "behind", b: "edge"),
                "side": Multiply(a: "tangent", b: "front"),
                "at": Combine(x: "screen.x", y: "screen.y", z: 0.0),
                "sum": Add(a: "lit", b: "side"),
                "all": Add(a: "sum", b: "at"),
                "red": Combine(x: 1.0, y: 0.0, z: 0.0),
                "paint": Lerp(a: "red", b: "all", t: 0.001),
            },
            surface: (albedo: "paint"),
        )"#,
        0,
    );
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(2.0));
    let mut shaders = MaterialShaders::new(&dir);
    for (name, result) in shaders.poll(&mut renderer, &gpu) {
        result.unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    for (shading, alpha) in [(Shading::Unlit, 1.0), (Shading::Lit, 1.0), (Shading::Lit, 0.5), (Shading::Unlit, 0.5)] {
        let frame = Frame {
            camera: Camera {
                position: Vec3::new(0.0, 0.0, 4.0),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            sky: scrap::render::Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            clear_color: Vec3::ZERO,
            post: scrap::post::PostProcess::OFF,
            ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
            draws: vec![Draw {
                mesh: cube,
                transform: Mat4::IDENTITY,
                texture: TextureHandle::WHITE,
                material: Material {
                    shading,
                    shader: Some(scrap::asset::shader_id("seen")),
                    alpha,
                    ..Material::new(1.0, 1.0, 1.0)
                },
                pose: None,
            }],
            ..Frame::default()
        };
        // Twice: the second frame has a last frame to read.
        renderer.render(&gpu, &target, &frame);
        renderer.render(&gpu, &target, &frame);
        let middle = OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2);
        assert!(middle[0] > 20 && middle[0] > middle[2], "drawn, red ({shading:?}, alpha {alpha}): {middle:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
