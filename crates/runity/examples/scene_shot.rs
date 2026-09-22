//! `scene_shot <scene.ron> [-o out.png]` — render a scene and write the frame.
//!
//! The whole headless loop in one command: change a scene or a shader, run
//! this, look at the picture. It needs no window and no graphics card, so it
//! is the same command in CI, on a laptop, and inside an agent's loop.

use std::path::PathBuf;

use runity::builtin;
use runity::glam::Vec3;
use runity::render::{Camera, FogSettings, Lighting};
use runity::{Gpu, Library, MeshHandle, OffscreenTarget, Renderer, Scene};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut scene_path: Option<PathBuf> = None;
    let mut out = PathBuf::from("frame.png");
    let mut library_dir: Option<PathBuf> = None;
    let (mut width, mut height) = (960u32, 540u32);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" | "--out" => out = args.next().map(PathBuf::from).unwrap_or(out),
            "--library" => library_dir = args.next().map(PathBuf::from),
            "--size" => {
                if let Some(size) = args.next() {
                    let (w, h) = size.split_once('x').ok_or("--size wants WIDTHxHEIGHT")?;
                    width = w.parse()?;
                    height = h.parse()?;
                }
            }
            "-h" | "--help" => {
                println!("scene_shot <scene.ron> [-o out.png] [--size WxH] [--library DIR]");
                return Ok(());
            }
            other => scene_path = Some(PathBuf::from(other)),
        }
    }
    let scene_path = scene_path.ok_or("usage: scene_shot <scene.ron>")?;
    let scene = Scene::load(&scene_path)?;

    let gpu = Gpu::headless_blocking(false)?;
    eprintln!(
        "{}: {} entities on {}",
        scene_path.display(),
        scene.entities.len(),
        gpu.describe()
    );

    let target = OffscreenTarget::new(&gpu, width, height);
    let mut renderer = Renderer::new(&gpu, &target);

    // Models resolve from the builtins first, then from a library if one was
    // given. Builtins first is what lets the reference scene open with no
    // pipeline at all.
    let library = match &library_dir {
        Some(dir) => {
            let (library, problems) = Library::open(dir)?;
            for (path, e) in &problems {
                eprintln!("skipped {}: {e}", path.display());
            }
            Some(library)
        }
        None => None,
    };

    let mut world = hecs_world();
    let mut uploaded: Vec<(String, MeshHandle)> = Vec::new();
    // Materials resolve the same way models do: the library first, then the
    // engine's builtins. With no library the scene still draws, in the
    // builtin palette — which is why the reference scene needs no pipeline.
    let missing = runity::spawn_scene_with(
        &scene,
        &mut world,
        |name| {
            if let Some(found) = uploaded.iter().find(|(n, _)| n == name) {
                return Some(found.1);
            }
            let handle = if let Some(mesh) = builtin::by_name(name) {
                renderer.upload_mesh_owned(&gpu, &mesh)
            } else {
                let mesh = library.as_ref()?.mesh_by_name(name)?;
                renderer.upload_mesh(&gpu, mesh)
            };
            uploaded.push((name.to_string(), handle));
            Some(handle)
        },
        |name| library.as_ref()?.material_by_name(name),
    );
    for m in &missing {
        eprintln!("{}: no model named {}", m.entity_name, m.model);
    }

    let lighting = Lighting {
        sun_direction: sun_direction(scene.sun.hour),
        sun_intensity: scene.sun.intensity,
        ..Lighting::default()
    };
    let fog = FogSettings {
        color: Vec3::from_array(scene.fog.color),
        start: scene.fog.start,
        end: scene.fog.end,
    };
    let camera = Camera {
        position: Vec3::new(0.0, 3.4, 12.0),
        target: Vec3::new(0.0, 1.4, -4.0),
        ..Camera::default()
    };

    let frame = runity::build_frame(&world, camera, lighting, fog);
    renderer.render(&gpu, &target, &frame);
    let pixels = target.read_rgba(&gpu);

    write_png(&out, &pixels, width, height)?;
    eprintln!("wrote {} ({width}x{height})", out.display());
    Ok(())
}

fn hecs_world() -> hecs::World {
    hecs::World::new()
}

/// Where the sun is at a given hour: up at noon, along the ground at dawn and
/// dusk. Crude on purpose — the engine ships a plausible default so that a
/// scene's `hour` does something, and a game replaces it with its own curve.
fn sun_direction(hour: f32) -> Vec3 {
    let t = ((hour - 6.0) / 12.0).clamp(0.0, 1.0);
    let angle = t * std::f32::consts::PI;
    Vec3::new(-angle.cos(), -angle.sin().max(0.15), -0.35).normalize()
}

fn write_png(
    path: &std::path::Path,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    image::save_buffer(path, pixels, width, height, image::ColorType::Rgba8)?;
    Ok(())
}
