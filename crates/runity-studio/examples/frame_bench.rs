//! `frame_bench` — what one editor frame costs, the GPU waited for.
//!
//! ```text
//! cargo run --release -p runity-studio --example frame_bench -- [scene.ron] [select name…]
//! ```
//!
//! The whole studio at 1440×900 points on a 2× display, as the window
//! draws it, for each graphics preset of the Scene view: the mean and the
//! slowest of 60 frames, each one finished on the GPU before the next.
//! Names after the scene are selected first, so the selection's outline
//! and handles are in what is measured.

use std::path::PathBuf;
use std::time::Instant;

use runity::gpu::OffscreenTarget;
use runity::quality::Quality;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let scene = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(runity_studio::REFERENCE_SCENE));
    let (width, height, scale) = (1440.0, 900.0, 2.0);
    let session = runity_studio::open(&scene)?;
    let mut studio = runity_studio::Studio::new(session, width, height, scale);
    for name in args {
        match studio.session.find(&name) {
            Some(id) => studio.session.add_to_selection(id)?,
            None => eprintln!("nothing called {name:?}"),
        }
    }
    let (pw, ph) = ((width * scale) as u32, (height * scale) as u32);
    let target = OffscreenTarget::new(studio.session.gpu(), pw, ph);
    let mut renderer = studio.renderer(target.format());
    for what in ["builtin:cube", "material:bark", "builtin:sphere"] {
        let t = Instant::now();
        let ok = studio.session.thumbnail(what, 128).is_ok();
        println!("thumbnail {what}: {:.1} ms ({ok})", t.elapsed().as_secs_f64() * 1e3);
    }
    let t = Instant::now();
    let ok = studio.session.scene_thumbnail(&scene, 128, 128).is_ok();
    println!("scene thumbnail: {:.1} ms ({ok})", t.elapsed().as_secs_f64() * 1e3);
    let presets = [None, Some(Quality::High), Some(Quality::Medium), Some(Quality::Low)];
    for preset in presets {
        studio.session.set_quality(preset);
        let mut times = Vec::new();
        for i in 0..80 {
            let t = Instant::now();
            studio.frame();
            studio.draw(&mut renderer, &target.ui_view(), pw, ph);
            let _ = studio
                .session
                .gpu()
                .device
                .poll(wgpu::PollType::wait_indefinitely());
            // The first frames build pipelines and settle the upscaler.
            if i >= 20 {
                times.push(t.elapsed().as_secs_f64() * 1e3);
            }
        }
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        let worst = times.iter().cloned().fold(0.0, f64::max);
        let (sw, sh) = studio.session.size();
        println!(
            "{:<8} {sw}x{sh}  mean {mean:6.1} ms  worst {worst:6.1} ms",
            preset.map_or("scene".to_string(), |q| format!("{q:?}"))
        );
        let mut passes = studio.session.gpu_times();
        passes.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (pass, ms) in passes.iter().take(6) {
            println!("           {pass:<28} {ms:5.1} ms");
        }
        // What goes over the picture, whatever it costs.
        for (pass, ms) in passes.iter().skip(6) {
            if ["overlay", "outline", "tools"].contains(&pass.as_str()) {
                println!("           {pass:<28} {ms:5.2} ms");
            }
        }
    }
    Ok(())
}
