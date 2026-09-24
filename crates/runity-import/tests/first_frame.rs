//! The whole path, end to end: a source model becomes an asset, the asset
//! becomes a mesh on the GPU, and the GPU produces a frame — on a machine
//! with no graphics card.
//!
//! The model is `tests/fixtures/conifer.obj`, which belongs to the engine and
//! not to any game. The engine has to be testable on its own; borrowing a
//! game's art for that would make the two repositories depend on each other
//! in the one direction that is supposed to stay closed.
//!
//! This is the test the rest of the visual work hangs off. It is not a golden
//! image, deliberately: a software adapter does not produce the same bytes as
//! a card, and a reference captured here would fail on anyone's machine. What
//! it asserts instead are facts about the frame that hold on any correct
//! renderer — the sky is above, the tree is in the middle, the far tree is
//! fogged, the lit side is brighter than the shaded one. Those catch the
//! failures that actually happen: nothing drawn, drawn inside out, lit from
//! underneath, depth test backwards.

use std::path::{Path, PathBuf};

use runity::render::{Camera, Draw, FogSettings, Frame, Lighting};
use runity::{Gpu, Library, MeshAsset, OffscreenTarget, Renderer};
use runity_import::ImportSettings;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    OffscreenTarget::pixel(pixels, WIDTH, x, y)
}

fn brightness(p: [u8; 4]) -> u32 {
    p[0] as u32 + p[1] as u32 + p[2] as u32
}

/// The target is sRGB, so the GPU encodes on the way out and a stored byte is
/// not the linear value the shader computed. Lighting is done in linear and
/// only the final write is encoded — which is correct, and which means a test
/// comparing against a colour it set has to decode first.
fn to_linear(byte: u8) -> f32 {
    let c = byte as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[test]
fn a_source_model_becomes_a_frame() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter — install mesa-vulkan-drivers to render here");
        return;
    };
    eprintln!("rendering on {}", gpu.describe());

    // 1. Import a real model, not a fixture.
    let source = fixture("conifer.obj");
    let out_dir = std::env::temp_dir().join("runity-first-frame");
    let _ = std::fs::remove_dir_all(&out_dir);
    let library_dir = out_dir.join("library");
    let mut settings = ImportSettings::for_source("tests/fixtures/conifer.obj");
    settings.recompute_normals = true;
    // The sidecar goes to the temp dir with the library: a test must not
    // write into the repository it is testing.
    runity_import::import_to(
        &source,
        &library_dir,
        &out_dir.join("conifer.obj.rimport"),
        settings,
    )
    .expect("importing the fixture");

    // 2. Open it the way the game will: by library, with no parser in sight.
    let (library, problems) = Library::open(&library_dir).expect("opening the library");
    assert!(problems.is_empty(), "{problems:?}");
    let mesh = library
        .mesh_by_name("conifer")
        .expect("the tree we just imported");
    let height = mesh.bounds.max[1].to_native();
    assert!(height > 1.0, "the tree should be metres tall, got {height}");

    // 3. Upload and draw: one near, one far, so fog has something to do.
    let target = OffscreenTarget::new(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(&gpu, &target);
    let handle = renderer.upload_mesh(&gpu, mesh);

    let lighting = Lighting {
        // Straight from the right, so one side of the trunk is lit and the
        // other is not — which is the thing being measured below.
        sun_direction: runity::glam::Vec3::new(-1.0, -0.25, 0.0).normalize(),
        ..Lighting::default()
    };
    let fog = FogSettings {
        start: 20.0,
        end: 70.0,
        ..FogSettings::default()
    };
    let frame = Frame {
        // Counted in exact colours: no sky, no post-processing.
        sky: runity::render::Sky {
            mode: runity::render::SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        ray_tracing: Default::default(),
        camera: Camera {
            position: runity::glam::Vec3::new(0.0, height * 0.55, height * 1.5),
            target: runity::glam::Vec3::new(0.0, height * 0.5, 0.0),
            ..Camera::default()
        },
        lighting,
        fog,
        // Off: this test is about the path from a file to pixels, and a
        // shadow falling across the tree it measures would make the
        // lit-versus-shaded comparison mean something else.
        shadows: runity::ShadowSettings::OFF,
        clear_color: fog.color,
        lights: Vec::new(),
        flares: Vec::new(),
        live_meshes: Vec::new(),
        texture_views: Vec::new(),
        ui_pictures: Vec::new(),
        reflection_probes: Vec::new(),
        decals: Vec::new(),
        volumetric_fog: Default::default(),
        wind: Default::default(),
        benders: Vec::new(),
        time: None,
        weather: Default::default(),
        screen_space_reflections: Default::default(),
        puffs: Vec::new(),
        gpu_particles: Vec::new(),
        plumes: Vec::new(),
        terrain: None,
        draws: vec![
            Draw {
                mesh: handle,
                transform: runity::glam::Mat4::IDENTITY,
                texture: runity::TextureHandle::WHITE,
                material: runity::material::builtin::NEEDLE,
                pose: None,
            },
            Draw {
                mesh: handle,
                // Off to the side and well back, so fog has something
                // visible to act on rather than hiding behind the near one.
                transform: runity::glam::Mat4::from_translation(runity::glam::Vec3::new(
                    9.0, 0.0, -38.0,
                )),
                texture: runity::TextureHandle::WHITE,
                material: runity::material::builtin::NEEDLE,
                pose: None,
            },
        ],
        overlay_draws: Vec::new(),
        poses: Vec::new(),
    };
    renderer.render(&gpu, &target, &frame);
    let pixels = target.read_rgba(&gpu);

    // Keep the frame around: when one of these assertions fails, the picture
    // is what explains why, and a number never does.
    let png = out_dir.join("first-frame.png");
    image::save_buffer(&png, &pixels, WIDTH, HEIGHT, image::ColorType::Rgba8)
        .expect("writing the frame out");
    eprintln!("wrote {}", png.display());

    // The sky survives where nothing was drawn.
    let sky = pixel(&pixels, 4, 4);
    let fog_color = fog.color;
    assert!(
        (to_linear(sky[0]) - fog_color.x).abs() < 0.02,
        "the corner should still be sky, got {sky:?} = {:.3} linear, wanted {:.3}",
        to_linear(sky[0]),
        fog_color.x
    );

    // Something was actually drawn in the middle. This is the assertion that
    // catches a mesh wound inside out: with back-face culling on, a reversed
    // model leaves the clear colour behind and nothing else.
    let middle = pixel(&pixels, WIDTH / 2, HEIGHT / 2);
    assert_ne!(
        middle, sky,
        "the tree should cover the centre of the frame, but the sky is showing through"
    );

    // The lit side is brighter than the shaded side. Measured over the
    // tree's own pixels, not a scanline: a band across the frame is mostly
    // sky, and the sky would drown the difference being tested.
    //
    // This catches a sun pointing the wrong way and normals left at zero.
    // Both of those render, and both look wrong in a way that no "did
    // anything draw" assertion would notice.
    let is_tree = |p: [u8; 4]| p != sky;
    let (mut lit, mut lit_n, mut shaded, mut shaded_n) = (0u32, 0u32, 0u32, 0u32);
    for y in 0..HEIGHT {
        for x in 0..WIDTH / 2 + 40 {
            let p = pixel(&pixels, x, y);
            if !is_tree(p) {
                continue;
            }
            if x < WIDTH / 2 {
                shaded += brightness(p);
                shaded_n += 1;
            } else {
                lit += brightness(p);
                lit_n += 1;
            }
        }
    }
    assert!(lit_n > 100 && shaded_n > 100, "too little tree to measure");
    let (lit, shaded) = (lit / lit_n, shaded / shaded_n);
    assert!(
        lit > shaded,
        "the sun comes from the right, so the right side of the trunk should be \
         brighter (lit {lit} vs shaded {shaded})"
    );

    // The far tree is nearer the fog colour than the near one. Without fog
    // both read as the same value and a forest is a flat wall.
    let near = pixel(&pixels, WIDTH / 2, HEIGHT / 2);
    // Scanned from the right edge inward, so the first thing found is the
    // rightmost drawn pixel — which is the far tree, since it is placed to
    // the right of the near one.
    let mut far = None;
    for x in (WIDTH / 2..WIDTH).rev() {
        for y in 0..HEIGHT {
            let p = pixel(&pixels, x, y);
            if is_tree(p) {
                far = Some(p);
                break;
            }
        }
        if far.is_some() {
            break;
        }
    }
    let far = far.expect("the second tree should be visible off to the right");
    let distance_to_sky = |p: [u8; 4]| brightness(p).abs_diff(brightness(sky));
    assert!(
        distance_to_sky(far) < distance_to_sky(near),
        "fog should pull the far tree toward the sky: far {far:?}, near {near:?}, sky {sky:?}"
    );
}

#[test]
fn an_asset_from_an_older_format_does_not_reach_the_gpu() {
    // The library skips what it cannot read rather than failing to open, so
    // one bad file costs one missing model instead of a black screen.
    let dir = std::env::temp_dir().join("runity-first-frame-bad");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut bytes = runity::asset::to_bytes(
        &MeshAsset {
            id: runity::AssetId::from_source("x", 0),
            name: "x".into(),
            vertices: Vec::new(),
            indices: Vec::new(),
            submeshes: Vec::new(),
            skin: None,
            bounds: runity::Bounds::of(&[]),
        },
        runity::asset::AssetKind::Mesh,
    )
    .unwrap();
    bytes[8] = 0;
    std::fs::write(dir.join("stale.rasset"), bytes).unwrap();

    let (library, problems) = Library::open(&dir).unwrap();
    assert_eq!(problems.len(), 1);
    assert!(library.is_empty());
}
