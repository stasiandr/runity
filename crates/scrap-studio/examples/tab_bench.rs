//! `tab_bench` — what a click on a dock's tab costs, the GPU waited for.
//!
//! ```text
//! cargo run --release -p scrap-studio --example tab_bench -- [scene.ron] [tab…]
//! ```
//!
//! The whole studio at 1440×900 points on a 2× display. Each round clicks
//! every named tab in turn (by default the ones the default layout shares
//! a stack with) and times the click's frame: the event, the panels caught
//! up, the UI laid out and drawn, finished on the GPU. Run with
//! `SCRAP_STUDIO_TIMING=1` for the parts of each frame.
//!
//! The editor writes down its layout and state in the project's `.scrap/`
//! as it goes; those files are put back as they were at the end.

use std::path::PathBuf;
use std::time::Instant;

use scrap::gpu::OffscreenTarget;
use scrap::input::{InputEvent, MouseButton};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let scene = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(scrap_studio::REFERENCE_SCENE));
    let mut tabs: Vec<String> = args.collect();
    if tabs.is_empty() {
        tabs = ["project", "console", "history", "git", "settings", "profiler", "animation"]
            .into_iter()
            .map(String::from)
            .collect();
    }
    let (width, height, scale) = (1440.0, 900.0, 2.0);
    // Before the scene opens: opening it writes it down as the last one.
    let kept: Vec<(PathBuf, Vec<u8>)> = std::fs::canonicalize(&scene)?
        .ancestors()
        .map(|dir| dir.join(".scrap"))
        .find(|dir| dir.is_dir())
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter_map(|p| std::fs::read(&p).ok().map(|bytes| (p, bytes)))
        .collect();
    let session = scrap_studio::open(&scene)?;
    let mut studio = scrap_studio::Studio::new(session, width, height, scale);
    let (pw, ph) = ((width * scale) as u32, (height * scale) as u32);
    let target = OffscreenTarget::new(studio.session.gpu(), pw, ph);
    let mut renderer = studio.renderer(target.format());
    let mut frame = |studio: &mut scrap_studio::Studio| {
        studio.frame();
        studio.draw(&mut renderer, &target.ui_view(), pw, ph);
        let _ = studio
            .session
            .gpu()
            .device
            .poll(wgpu::PollType::wait_indefinitely());
    };
    // Pipelines built, the upscaler settled.
    for _ in 0..20 {
        frame(&mut studio);
    }
    // Idle frames, for comparison.
    let mut idle = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        frame(&mut studio);
        idle.push(t.elapsed().as_secs_f64() * 1e3);
    }
    println!("idle frame   mean {:6.1} ms", mean(&idle));
    let mut per_tab: Vec<Vec<f64>> = vec![Vec::new(); tabs.len()];
    for round in 0..6 {
        for (i, tab) in tabs.iter().enumerate() {
            studio.ui.paint();
            let Some(node) = studio.ui.find(&format!("tab {tab}")) else {
                eprintln!("no tab {tab:?}");
                continue;
            };
            let (x, y) = studio.ui.rect(node).center();
            studio.handle(&InputEvent::MouseMoved { x, y });
            let t = Instant::now();
            studio.handle(&InputEvent::MouseDown(MouseButton::Left));
            studio.handle(&InputEvent::MouseUp(MouseButton::Left));
            if std::env::var_os("SCRAP_STUDIO_TIMING").is_some() {
                eprintln!("-- tab {tab}");
            }
            frame(&mut studio);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            // The first round opens each panel for the first time.
            if round > 0 {
                per_tab[i].push(ms);
            } else {
                println!("first {tab:<10} {ms:6.1} ms");
            }
            // A few frames after, as a person's pointer would wait.
            for _ in 0..3 {
                frame(&mut studio);
            }
        }
    }
    let mut all = Vec::new();
    for (tab, times) in tabs.iter().zip(&per_tab) {
        let worst = times.iter().cloned().fold(0.0, f64::max);
        println!("tab {tab:<10} mean {:6.1} ms  worst {worst:6.1} ms", mean(times));
        all.extend(times);
    }
    println!("all tabs     mean {:6.1} ms", mean(&all));
    drop(studio);
    for (path, bytes) in kept {
        std::fs::write(path, bytes)?;
    }
    Ok(())
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}
