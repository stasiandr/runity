//! An effect graph in a project's `shaders/`: found and put in by the same
//! polling as the material shaders, and an emitter on the GPU naming it
//! does what it says — here, falls instead of rising, and red instead of
//! its white. Edited, taken in again; broken, refused in words, and the
//! last one keeps drawing.

use scrap::glam::{Mat4, Vec3};
use scrap::particles_gpu::GpuEmitter;
use scrap::render::{Camera, Frame, MaterialShaders, Sky, SkyMode};
use scrap::scene::Emitter;
use scrap::{Gpu, OffscreenTarget, Renderer};

const SIZE: u32 = 96;

fn write(path: &std::path::Path, text: &str, second: u64) {
    std::fs::write(path, text).unwrap();
    let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + second);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
}

/// How many pixels above and below the emitter are lit, and the reddest
/// lit pixel's colour.
fn run(renderer: &mut Renderer, gpu: &Gpu, target: &OffscreenTarget, emitter: &Emitter, key: u64) -> (usize, usize, [u8; 4]) {
    let mut frame = Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: scrap::post::PostProcess::OFF,
        clear_color: Vec3::ZERO,
        camera: Camera {
            position: Vec3::new(0.0, 1.0, 4.0),
            target: Vec3::new(0.0, 1.0, 0.0),
            ..Camera::default()
        },
        ..Frame::default()
    };
    let dt = 1.0 / 30.0;
    for i in 1..=20u32 {
        let lived = i as f32 * dt;
        frame.gpu_particles = vec![GpuEmitter {
            key,
            placed: Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)),
            emitter: emitter.clone(),
            born: (lived * emitter.rate) as u64,
            lived,
        }];
        renderer.render(gpu, target, &frame);
    }
    let pixels = target.read_rgba(gpu);
    let lit = |rows: std::ops::Range<u32>| {
        rows.flat_map(|y| (0..SIZE).map(move |x| (x, y)))
            .filter(|&(x, y)| OffscreenTarget::pixel(&pixels, SIZE, x, y).iter().take(3).any(|c| *c > 40))
            .count()
    };
    let brightest = (0..SIZE * SIZE)
        .map(|i| OffscreenTarget::pixel(&pixels, SIZE, i % SIZE, i / SIZE))
        .max_by_key(|p| p[0] as u32 + p[1] as u32 + p[2] as u32)
        .unwrap();
    (lit(0..SIZE / 2 - 2), lit(SIZE / 2 + 6..SIZE), brightest)
}

#[test]
fn an_effect_graph_makes_an_emitters_particles_fall_red_and_reloads() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let dir = std::env::temp_dir().join(format!("scrap-vfx-graph-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("fall.vfx.ron");
    write(
        &file,
        r#"(
            nodes: {
                "down": Multiply(a: "cone", b: (1.0, -1.0, 1.0)),
                "fast": Multiply(a: "down", b: "speed"),
            },
            spawn: (velocity: "fast"),
            output: (color: (1.0, 0.0, 0.0)),
        )"#,
        0,
    );
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let mut shaders = MaterialShaders::new(&dir);
    let put = shaders.poll(&mut renderer, &gpu);
    assert_eq!(put.len(), 1, "{put:?}");
    assert_eq!(put[0].0, "fall", "named by the file, less `.vfx.ron`");
    put[0].1.as_ref().unwrap();
    assert!(renderer.has_effect_graph("fall"));

    let mut emitter = Emitter {
        rate: 5000.0,
        life: 1.0,
        speed: 2.0,
        spread_deg: 12.0,
        size: 0.05,
        gravity: 0.0,
        color: (1.0, 1.0, 1.0),
        graph: "fall".into(),
        ..Emitter::default()
    };
    let (above, below, colour) = run(&mut renderer, &gpu, &target, &emitter, 1);
    assert!(below > 100, "the graph sends them down: {below} pixels below");
    assert_eq!(above, 0, "none rise: {above}");
    assert!(colour[0] > 100 && colour[1] < 20 && colour[2] < 20, "red, not the emitter's white: {colour:?}");

    // Edited: up again, green.
    write(
        &file,
        r#"(output: (color: (0.0, 1.0, 0.0)))"#,
        1,
    );
    shaders.poll(&mut renderer, &gpu)[0].1.as_ref().unwrap();
    let (above, below, colour) = run(&mut renderer, &gpu, &target, &emitter, 2);
    assert!(above > 150 && below == 0, "the plain spawn again: {above} above, {below} below");
    assert!(colour[1] > 100 && colour[0] < 20, "green: {colour:?}");

    // Misspelt: refused, naming the part, the node and the input; the
    // last one goes on.
    write(
        &file,
        r#"(nodes: { "hue": Multiply(a: "colr", b: 0.5) }, output: (color: "hue"))"#,
        2,
    );
    let put = shaders.poll(&mut renderer, &gpu);
    let words = put[0].1.as_ref().expect_err("a misspelt input is refused");
    assert!(words.contains("fall.vfx.ron") && words.contains("node `hue`, input `a`") && words.contains("did you mean `color`"), "{words}");
    let (_, _, colour) = run(&mut renderer, &gpu, &target, &emitter, 3);
    assert!(colour[1] > 100 && colour[0] < 20, "still green: {colour:?}");

    // A name nothing answers to draws as the plain emitter.
    emitter.graph = "nothing".into();
    let (above, _, colour) = run(&mut renderer, &gpu, &target, &emitter, 4);
    assert!(above > 150 && colour[0] > 100 && colour[1] > 100, "plain white rising: {above}, {colour:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
