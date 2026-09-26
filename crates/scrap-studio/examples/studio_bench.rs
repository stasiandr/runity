//! `studio_bench` — what opening a project and working in it costs, the
//! GPU waited for: the scene opened, the studio made, the first frames,
//! a few seconds idle (the disk looked at as the editor does), and clicks
//! on the Hierarchy's lines (a selection each).
//!
//! ```text
//! cargo run --release -p scrap-studio --example studio_bench -- scene.ron
//! ```
//!
//! The editor writes down its layout and state in the project's `.scrap/`
//! as it goes; those files are put back as they were at the end.

use std::path::PathBuf;
use std::time::Instant;

use scrap::gpu::OffscreenTarget;
use scrap::input::{InputEvent, MouseButton};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scene = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(scrap_studio::REFERENCE_SCENE));
    let (width, height, scale) = (1440.0, 900.0, 2.0);
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

    let t = Instant::now();
    let session = scrap_studio::open_live(&scene)?;
    println!("open         {:7.1} ms", ms(t));
    let t = Instant::now();
    let mut studio = scrap_studio::Studio::new(session, width, height, scale);
    println!("studio       {:7.1} ms", ms(t));
    let (pw, ph) = ((width * scale) as u32, (height * scale) as u32);
    let target = OffscreenTarget::new(studio.session.gpu(), pw, ph);
    let mut renderer = studio.renderer(target.format());
    let mut frame = |studio: &mut scrap_studio::Studio| {
        let t = Instant::now();
        studio.frame();
        studio.draw(&mut renderer, &target.ui_view(), pw, ph);
        let _ = studio
            .session
            .gpu()
            .device
            .poll(wgpu::PollType::wait_indefinitely());
        ms(t)
    };
    let t = Instant::now();
    let first = frame(&mut studio);
    println!("first frame  {first:7.1} ms   (from start {:7.1} ms)", ms(t));
    for _ in 0..10 {
        frame(&mut studio);
    }

    // Idle for three seconds of wall clock: the editor's looks at the disk
    // fall in it as they would.
    let mut idle = Vec::new();
    let t = Instant::now();
    while t.elapsed().as_secs_f64() < 3.0 {
        idle.push(frame(&mut studio));
    }
    report("idle", &idle);

    // A click on each of the Hierarchy's first lines, in turn.
    let lines: Vec<String> = studio
        .ui
        .dump()
        .lines()
        .filter_map(|l| l.split('#').nth(1))
        .filter_map(|l| l.strip_prefix("line "))
        .map(|l| l.split(" @").next().unwrap_or(l).split(" \"").next().unwrap_or(l).trim().to_string())
        .take(12)
        .collect();
    let mut clicks = Vec::new();
    for round in 0..3 {
        for line in &lines {
            studio.ui.paint();
            let Some(node) = studio.ui.find(&format!("line {line}")) else {
                continue;
            };
            let (x, y) = studio.ui.rect(node).center();
            studio.handle(&InputEvent::MouseMoved { x, y });
            let t = Instant::now();
            studio.handle(&InputEvent::MouseDown(MouseButton::Left));
            studio.handle(&InputEvent::MouseUp(MouseButton::Left));
            frame(&mut studio);
            if round > 0 {
                clicks.push(ms(t));
            }
            for _ in 0..2 {
                frame(&mut studio);
            }
        }
    }
    report("select", &clicks);

    drop(studio);
    for (path, bytes) in kept {
        std::fs::write(path, bytes)?;
    }
    Ok(())
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn report(what: &str, times: &[f64]) {
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |q: f64| sorted.get(((sorted.len() as f64 - 1.0) * q) as usize).copied().unwrap_or(0.0);
    println!(
        "{what:<12} n {:4}  mean {:6.1}  p50 {:6.1}  p95 {:6.1}  worst {:6.1} ms",
        times.len(),
        times.iter().sum::<f64>() / times.len().max(1) as f64,
        at(0.5),
        at(0.95),
        at(1.0)
    );
}
