//! An image from the repository, through the importer, onto a surface.
//!
//! The assertion that matters is that the texture reaches the pixels at all:
//! a sampler bound to the wrong group, UVs left at zero, or a white default
//! that never gets replaced all produce a frame that renders fine and is
//! flat. Comparing a textured draw against an untextured one is what tells
//! those apart.

#[allow(unused_imports)]
use runity::prelude::*;
use std::path::{Path, PathBuf};

use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, Draw, FogSettings, Frame, Lighting, ShadowSettings, TextureHandle};
use runity::{builtin, Gpu, Library, OffscreenTarget, Renderer};
use runity_import::ImportSettings;

const SIZE: u32 = 192;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/runity-import is two levels down")
        .to_path_buf()
}

/// Draw a plane facing the camera, once with a texture and once without.
fn shoot(gpu: &Gpu, renderer: &mut Renderer, texture: TextureHandle) -> Vec<u8> {
    let target = OffscreenTarget::new(gpu, SIZE, SIZE);
    let plane = builtin::plane(2.0, 1);
    let mesh = renderer.upload_mesh_owned(gpu, &plane);
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
            position: Vec3::new(0.0, 3.0, 0.0),
            target: Vec3::ZERO,
            // Looking straight down, so "up" cannot be up.
            up: Vec3::Z,
            ..Camera::default()
        },
        lighting: Lighting {
            sun_direction: Vec3::new(0.0, -1.0, 0.0),
            ..Lighting::default()
        },
        fog: FogSettings {
            start: 1000.0,
            end: 2000.0,
            ..FogSettings::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::new(0.0, 0.0, 0.0),
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
        smoke: Vec::new(),
        distance_field: None,
        gpu_particles: Vec::new(),
        plumes: Vec::new(),
        terrain: None,
        draws: vec![Draw {
            mesh,
            transform: Mat4::IDENTITY,
            texture,
            material: runity::Material::new(1.0, 1.0, 1.0),
            pose: None,
        }],
        overlay_draws: Vec::new(),
        poses: Vec::new(),
    };
    renderer.render(gpu, &target, &frame);
    target.read_rgba(gpu)
}

/// How much the pixels differ from each other — a flat surface scores near
/// zero however bright it is.
///
/// Measured over the middle of the frame only. The plane does not fill the
/// view, and the background around it varies more than any texture would,
/// which would drown the thing being measured.
fn variation(pixels: &[u8]) -> u32 {
    let (lo, hi) = (SIZE / 4, SIZE * 3 / 4);
    let luma: Vec<u32> = (lo..hi)
        .flat_map(|y| (lo..hi).map(move |x| (x, y)))
        .map(|(x, y)| {
            let p = OffscreenTarget::pixel(pixels, SIZE, x, y);
            p[0] as u32 + p[1] as u32 + p[2] as u32
        })
        .collect();
    let mean = luma.iter().sum::<u32>() / luma.len().max(1) as u32;
    luma.iter().map(|l| l.abs_diff(mean)).sum::<u32>() / luma.len() as u32
}

#[test]
fn a_texture_from_the_library_reaches_the_pixels() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };

    let source = repository_root().join("examples/valley/assets/textures/valley_atlas.png");
    let out_dir = std::env::temp_dir().join("runity-textured");
    let _ = std::fs::remove_dir_all(&out_dir);
    let library_dir = out_dir.join("library");
    // Sidecar into the temp dir, not beside the committed image: a test
    // must not write into the repository it is testing.
    runity_import::import_to(
        &source,
        &library_dir,
        &out_dir.join("valley_atlas.png.rimport"),
        ImportSettings::for_source("examples/valley/assets/textures/valley_atlas.png"),
    )
    .expect("importing a committed image");

    let (library, problems) = Library::open(&library_dir).expect("opening the library");
    assert!(problems.is_empty(), "{problems:?}");
    let atlas = library
        .texture_by_name("valley_atlas")
        .expect("the atlas we just imported");
    assert!(atlas.width.to_native() >= 16);

    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let handle = renderer.upload_texture(&gpu, atlas);

    let plain = shoot(&gpu, &mut renderer, TextureHandle::WHITE);
    let textured = shoot(&gpu, &mut renderer, handle);

    image::save_buffer(
        out_dir.join("textured.png"),
        &textured,
        SIZE,
        SIZE,
        image::ColorType::Rgba8,
    )
    .expect("writing the frame out");

    assert!(
        variation(&plain) < 4,
        "an untextured plane under a straight-down sun should be flat, \
         scored {}",
        variation(&plain)
    );
    assert!(
        variation(&textured) > 12,
        "the atlas has sixteen colours across it; a textured plane cannot be \
         as flat as an untextured one (scored {} against {})",
        variation(&textured),
        variation(&plain)
    );
}

#[test]
fn the_white_default_leaves_a_materials_colour_alone() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    // Every renderer's handle 0 is one white pixel, so an untextured surface
    // is its own colour times one rather than a second pipeline.
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let pixels = shoot(&gpu, &mut renderer, TextureHandle::WHITE);
    let middle = OffscreenTarget::pixel(&pixels, SIZE, SIZE / 2, SIZE / 2);
    assert!(
        middle[0] > 100 && middle[0] == middle[1] && middle[1] == middle[2],
        "a white material under a white sun should stay grey, got {middle:?}"
    );
}
