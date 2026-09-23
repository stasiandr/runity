//! `scene_shot <scene.ron> [-o out.png] [--time N]` — render a scene and
//! write the frame; with `--time`, also render it N more times and print
//! how long a frame took on the GPU and CPU together.
//!
//! The whole headless loop in one command: change a scene or a shader, run
//! this, look at the picture. It needs no window and no graphics card, so it
//! is the same command in CI, on a laptop, and inside an agent's loop.

#[allow(unused_imports)]
use runity::prelude::*;
use std::path::PathBuf;

use runity::builtin;
use runity::glam::Vec3;
use runity::render::FogSettings;
use runity::{Gpu, Library, MeshHandle, OffscreenTarget, Renderer, Scene};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut scene_path: Option<PathBuf> = None;
    let mut out = PathBuf::from("frame.png");
    let mut library_dir: Option<PathBuf> = None;
    let (mut width, mut height) = (960u32, 540u32);
    let mut timed = 0u32;
    let mut at: Option<f32> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" | "--out" => out = args.next().map(PathBuf::from).unwrap_or(out),
            "--library" => library_dir = args.next().map(PathBuf::from),
            "--time" => timed = args.next().and_then(|n| n.parse().ok()).unwrap_or(30),
            "--at" => at = args.next().and_then(|n| n.parse().ok()),
            "--size" => {
                if let Some(size) = args.next() {
                    let (w, h) = size.split_once('x').ok_or("--size wants WIDTHxHEIGHT")?;
                    width = w.parse()?;
                    height = h.parse()?;
                }
            }
            "-h" | "--help" => {
                println!(
                    "scene_shot <scene.ron> [-o out.png] [--size WxH] [--library DIR] [--time N] [--at SECONDS]\n\n\
                     Prefabs and the library come from the project the scene is in;\n\
                     --library overrides the library."
                );
                return Ok(());
            }
            other => scene_path = Some(PathBuf::from(other)),
        }
    }
    let scene_path = scene_path.ok_or("usage: scene_shot <scene.ron>")?;
    let document = Scene::load(&scene_path)?;

    // The project the scene is in says where its prefabs and its library
    // are. Every instance is replaced by what it stands for before anything
    // else looks at the scene; nothing downstream knows a prefab existed.
    let project = runity::Project::find(&scene_path).ok();
    let (prefabs, prefab_problems) = project
        .as_ref()
        .map(runity::Prefabs::of)
        .unwrap_or_default();
    for (path, e) in &prefab_problems {
        eprintln!("skipped {}: {e}", path.display());
    }
    let instanced = runity::instantiate(&document, &prefabs);
    for problem in &instanced.problems {
        eprintln!(
            "{}: prefab {} — {}",
            problem.entity_name, problem.prefab, problem.reason
        );
    }
    let scene = instanced.scene;

    let gpu = Gpu::headless_blocking(false)?;
    eprintln!(
        "{}: {} entities on {}",
        scene_path.display(),
        scene.entities.len(),
        gpu.describe()
    );

    let target = OffscreenTarget::new(&gpu, width, height);
    let mut renderer = Renderer::new(&gpu, &target);
    // The project's materials' own shaders.
    if let Some(project) = &project {
        let mut shaders =
            runity::render::MaterialShaders::new(project.root().join(runity::project::SHADERS));
        for (name, result) in shaders.poll(&mut renderer, &gpu) {
            if let Err(problem) = result {
                eprintln!("shader {name}: {problem}");
            }
        }
    }

    // Models resolve from the builtins first, then from a library: the one
    // given, or else the project's own if it has been built. Builtins first
    // is what lets the reference scene open with no pipeline at all.
    let library_dir = library_dir.or_else(|| {
        project
            .as_ref()
            .map(|p| p.library())
            .filter(|dir| dir.is_dir())
    });
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
            let name: &str = name;
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

    let lighting = runity::scene_lighting(&scene.sun());
    let fog = FogSettings {
        color: Vec3::from_array(scene.fog().color),
        start: scene.fog().start,
        end: scene.fog().end,
        ..Default::default()
    };
    // The scene says where it is looked at from, so two renders of the same
    // file are the same picture — and so an agent can frame a shot by
    // editing a line rather than by patching this file.
    let camera = runity::scene_camera(&scene.view());

    runity::terrain::upload_terrains(&mut world, &gpu, &mut renderer);
    for problem in
        runity::world::upload_material_maps(&world, library.as_ref(), &gpu, &mut renderer)
    {
        eprintln!("{problem}");
    }
    let mut frame = runity::build_frame(&world, camera, lighting, fog);
    runity::world::scene_look(&mut frame, &scene);
    // A moment of the scene's clock: where the weather has got to.
    if at.is_some() {
        frame.time = at;
    }
    // Twice: what reads the last frame — screen-space reflections — has
    // one by the second.
    renderer.render(&gpu, &target, &frame);
    renderer.render(&gpu, &target, &frame);
    let pixels = target.read_rgba(&gpu);
    if timed > 0 {
        let start = std::time::Instant::now();
        for _ in 0..timed {
            renderer.render(&gpu, &target, &frame);
        }
        // Reading a pixel back waits for the last frame to finish.
        target.read_rgba(&gpu);
        let ms = start.elapsed().as_secs_f64() * 1000.0 / timed as f64;
        eprintln!("{ms:.2} ms a frame over {timed}");
    }

    write_png(&out, &pixels, width, height)?;
    eprintln!("wrote {} ({width}x{height})", out.display());
    Ok(())
}

fn hecs_world() -> hecs::World {
    hecs::World::new()
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
