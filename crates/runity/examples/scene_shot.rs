//! `scene_shot <scene.ron> [-o out.png] [--time N]` — render a scene and
//! write the frame; with `--time`, also render it N more times and print
//! how long a frame took on the GPU and CPU together.
//!
//! The whole headless loop in one command: change a scene or a shader, run
//! this, look at the picture. It needs no window and no graphics card, so it
//! is the same command in CI, on a laptop, and inside an agent's loop.

use std::path::PathBuf;

use runity::shot::Shot;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut scene_path: Option<PathBuf> = None;
    let mut out = PathBuf::from("frame.png");
    let mut library_dir: Option<PathBuf> = None;
    let (mut width, mut height) = (960u32, 540u32);
    let mut timed = 0u32;
    let mut at: Option<f32> = None;
    let mut upscale: Option<f32> = None;
    let mut virtual_shadows = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" | "--out" => out = args.next().map(PathBuf::from).unwrap_or(out),
            "--library" => library_dir = args.next().map(PathBuf::from),
            "--time" => timed = args.next().and_then(|n| n.parse().ok()).unwrap_or(30),
            "--at" => at = args.next().and_then(|n| n.parse().ok()),
            "--upscale" => upscale = args.next().and_then(|n| n.parse().ok()),
            "--virtual-shadows" => virtual_shadows = true,
            "--size" => {
                if let Some(size) = args.next() {
                    let (w, h) = size.split_once('x').ok_or("--size wants WIDTHxHEIGHT")?;
                    width = w.parse()?;
                    height = h.parse()?;
                }
            }
            "-h" | "--help" => {
                println!(
                    "scene_shot <scene.ron> [-o out.png] [--size WxH] [--library DIR] [--time N] [--at SECONDS] [--upscale SCALE] [--virtual-shadows]\n\n\
                     Prefabs and the library come from the project the scene is in;\n\
                     --library overrides the library."
                );
                return Ok(());
            }
            other => scene_path = Some(PathBuf::from(other)),
        }
    }
    let scene_path = scene_path.ok_or("usage: scene_shot <scene.ron>")?;
    // The scene with its project's prefabs, library and shaders, spawned
    // and its frame built (runity::shot).
    let mut shot = Shot::open(&scene_path, width, height, library_dir.as_deref())?;
    for problem in &shot.problems {
        eprintln!("{problem}");
    }
    eprintln!(
        "{}: {} entities on {}",
        scene_path.display(),
        shot.entities,
        shot.gpu.describe()
    );
    let frame = &mut shot.frame;
    if virtual_shadows {
        frame.shadows.virtual_maps = true;
        frame.shadows.max_distance = frame.shadows.max_distance.max(200.0);
        // A pool of a thousand pages: what a 1080p view needs at every level.
        frame.shadows.resolution = frame.shadows.resolution.max(4096);
    }
    if let Some(scale) = upscale {
        frame.post.upscaling.enabled = true;
        frame.post.upscaling.scale = scale;
    }
    // A moment of the scene's clock: where the weather has got to.
    if at.is_some() {
        frame.time = at;
    }
    // Settled first: what reads the last frame (screen-space reflections),
    // and what builds a history over frames — probes, pages, reservoirs,
    // an upscaler's.
    let warm = shot.warm_frames();
    shot.draw(warm);
    let pixels = shot.pixels();
    if timed > 0 {
        shot.renderer.profile_gpu(true);
        if std::env::var_os("RUNITY_NO_OCCLUSION").is_some() {
            shot.renderer.set_occlusion_culling(false);
        }
        let start = std::time::Instant::now();
        shot.draw(timed);
        // Reading a pixel back waits for the last frame to finish.
        shot.pixels();
        let ms = start.elapsed().as_secs_f64() * 1000.0 / timed as f64;
        eprintln!("{ms:.2} ms a frame over {timed}");
        // A few frames more, each waited for, so the passes' times come
        // back: the timer reads them a frame or two late.
        for _ in 0..4 {
            shot.draw(1);
            shot.pixels();
        }
        let passes = shot.renderer.gpu_times();
        if !passes.is_empty() {
            let total: f32 = passes.iter().map(|(_, t)| t).sum();
            eprintln!("on the GPU, {total:.2} ms in passes:");
            for (name, ms) in passes {
                eprintln!("  {name:<14} {ms:6.2} ms");
            }
        }
    }

    write_png(&out, &pixels, width, height)?;
    eprintln!("wrote {} ({width}x{height})", out.display());
    Ok(())
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
