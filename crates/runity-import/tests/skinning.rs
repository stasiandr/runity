//! A skinned mesh, from glTF to moved pixels.
//!
//! The data path is tested elsewhere; this is about the GPU actually
//! applying it. Skinning fails in ways that render perfectly: joint indices
//! read as the wrong type, weights that never reach the shader, a pose bound
//! at the wrong dynamic offset. Every one of those draws the mesh in its
//! bind pose and looks like an animation that is not playing.

use std::path::{Path, PathBuf};

use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, Draw, FogSettings, Frame, Lighting, Pose, ShadowSettings};
use runity::{Gpu, Library, MeshAsset, OffscreenTarget, Renderer, TextureHandle};
use runity_import::ImportSettings;

const SIZE: u32 = 192;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Draw the banner with a given pose and return the pixels.
fn shoot(
    gpu: &Gpu,
    renderer: &mut Renderer,
    mesh: runity::MeshHandle,
    pose: Option<Pose>,
) -> Vec<u8> {
    let target = OffscreenTarget::new(gpu, SIZE, SIZE);
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
            position: Vec3::new(0.0, 1.0, 5.0),
            target: Vec3::new(0.0, 1.0, 0.0),
            ..Camera::default()
        },
        lighting: Lighting {
            sun_direction: Vec3::new(0.0, 0.0, -1.0),
            ..Lighting::default()
        },
        fog: FogSettings {
            start: 1000.0,
            end: 2000.0,
            ..FogSettings::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        lights: Vec::new(),
        flares: Vec::new(),
        live_meshes: Vec::new(),
        reflection_probes: Vec::new(),
        decals: Vec::new(),
        volumetric_fog: Default::default(),
        draws: vec![Draw {
            mesh,
            transform: Mat4::IDENTITY,
            texture: TextureHandle::WHITE,
            material: runity::Material::new(1.0, 1.0, 1.0),
            pose: pose.as_ref().map(|_| 0),
        }],
        overlay_draws: Vec::new(),
        poses: pose.into_iter().collect(),
    };
    renderer.render(gpu, &target, &frame);
    target.read_rgba(gpu)
}

/// The horizontal centre of mass of everything that is not background.
fn centre_of_mass(pixels: &[u8]) -> Option<f32> {
    let (mut sum, mut count) = (0.0f64, 0u32);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = OffscreenTarget::pixel(pixels, SIZE, x, y);
            if p[0] as u32 + p[1] as u32 + p[2] as u32 > 40 {
                sum += x as f64;
                count += 1;
            }
        }
    }
    (count > 20).then(|| (sum / count as f64) as f32)
}

#[test]
fn a_pose_moves_the_vertices_the_gpu_draws() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };

    let out_dir = std::env::temp_dir().join("runity-skinning");
    let _ = std::fs::remove_dir_all(&out_dir);
    let library_dir = out_dir.join("library");
    // Sidecar into the temp dir: a test must not write into the repository.
    runity_import::import_to(
        &fixture("skinned_banner.gltf"),
        &library_dir,
        &out_dir.join("skinned_banner.gltf.rimport"),
        ImportSettings {
            origin_to_base: false,
            ..ImportSettings::for_source("skinned_banner.gltf")
        },
    )
    .expect("importing the fixture");

    let (library, problems) = Library::open(&library_dir).expect("opening the library");
    assert!(problems.is_empty(), "{problems:?}");
    let archived = library.mesh_by_name("skinned_banner").expect("the banner");

    // The skeleton and clip come back out of the archive, decoded once here
    // because the pose is computed on the CPU.
    let mesh: MeshAsset = rkyv::deserialize::<MeshAsset, rkyv::rancor::Error>(archived)
        .expect("a mesh round-trips out of its archive");
    let skin = mesh.skin.as_ref().expect("the banner is skinned");
    let clip = &skin.clips[0];

    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let handle = renderer.upload_mesh(&gpu, archived);

    let rest = Pose(skin.skeleton.skinning_matrices(&skin.skeleton.rest_pose()));
    let bent = Pose(skin.skeleton.skinning_matrices(&clip.sample(
        &skin.skeleton,
        clip.duration,
        false,
    )));

    let at_rest = shoot(&gpu, &mut renderer, handle, Some(rest));
    let animated = shoot(&gpu, &mut renderer, handle, Some(bent));

    image::save_buffer(
        out_dir.join("animated.png"),
        &animated,
        SIZE,
        SIZE,
        image::ColorType::Rgba8,
    )
    .expect("writing the frame out");

    let still = centre_of_mass(&at_rest).expect("the banner is on screen at rest");
    let moved = centre_of_mass(&animated).expect("and still on screen when posed");
    assert!(
        (moved - still).abs() > 4.0,
        "a quarter turn at the elbow should shift the shape sideways; \
         it sat at {still:.1} and then at {moved:.1}, which means the pose \
         never reached the vertex stage"
    );
}

#[test]
fn a_mesh_drawn_without_a_pose_stands_in_its_bind_position() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let out_dir = std::env::temp_dir().join("runity-skinning-bind");
    let _ = std::fs::remove_dir_all(&out_dir);
    let library_dir = out_dir.join("library");
    runity_import::import_to(
        &fixture("skinned_banner.gltf"),
        &library_dir,
        &out_dir.join("skinned_banner.gltf.rimport"),
        ImportSettings {
            origin_to_base: false,
            ..ImportSettings::for_source("skinned_banner.gltf")
        },
    )
    .unwrap();
    let (library, _) = Library::open(&library_dir).unwrap();
    let archived = library.mesh_by_name("skinned_banner").unwrap();
    let mesh: MeshAsset = rkyv::deserialize::<MeshAsset, rkyv::rancor::Error>(archived).unwrap();
    let skin = mesh.skin.as_ref().unwrap();

    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let handle = renderer.upload_mesh(&gpu, archived);

    // A skinned mesh with no pose goes through the ordinary pipeline, and
    // must land exactly where the rest pose puts it — otherwise attaching a
    // skeleton to a model moves it before anything is animated.
    let unposed = shoot(&gpu, &mut renderer, handle, None);
    let rest = Pose(skin.skeleton.skinning_matrices(&skin.skeleton.rest_pose()));
    let posed_at_rest = shoot(&gpu, &mut renderer, handle, Some(rest));

    let a = centre_of_mass(&unposed).expect("drawn without a pose");
    let b = centre_of_mass(&posed_at_rest).expect("drawn at rest");
    assert!(
        (a - b).abs() < 1.0,
        "rest pose and no pose should agree, got {a:.2} and {b:.2}"
    );
}
