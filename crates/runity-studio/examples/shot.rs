//! `shot` — the editor, photographed without a screen.
//!
//! ```text
//! cargo run -p runity-studio --example shot -- [scene.ron] [out.png] [select name…]
//! ```
//!
//! The whole editor — panels, Scene view and all — drawn by its own code
//! into an off-screen texture on the session's GPU, and read back. Not a
//! mock: the same `Studio` the window runs. An agent sees what it changed
//! without a person looking at a screen (DNA, postulate 5).

use std::path::PathBuf;

use runity::gpu::OffscreenTarget;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let scene = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(runity_studio::REFERENCE_SCENE));
    let out = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("runity-studio.png"));
    let (width, height, scale) = (1440.0, 900.0, 2.0);

    let session = runity_studio::open(&scene)?;
    let mut studio = runity_studio::Studio::new(session, width, height, scale);
    for name in args {
        match studio.session.find(&name) {
            Some(id) => studio.session.add_to_selection(id)?,
            None => eprintln!("nothing called {name:?}"),
        }
    }
    // Names of nodes to click first, comma-separated: what a person would
    // do before looking (`asset campfire,menu bar View`).
    let clicks = std::env::var("RUNITY_SHOT_CLICK").unwrap_or_default();
    // The toolbar as macOS's window has it: the menus in the menu bar.
    if std::env::var_os("RUNITY_SHOT_NATIVE_MENU").is_some() {
        studio.set_native_menu(true);
    }
    studio.frame();
    for name in clicks.split(',').filter(|n| !n.is_empty()) {
        studio.ui.paint();
        let Some(node) = studio.ui.find(name) else {
            eprintln!("no node called {name:?}");
            continue;
        };
        let (x, y) = studio.ui.rect(node).center();
        use runity::input::{InputEvent, MouseButton};
        studio.handle(&InputEvent::MouseMoved { x, y });
        studio.handle(&InputEvent::MouseDown(MouseButton::Left));
        studio.handle(&InputEvent::MouseUp(MouseButton::Left));
        studio.frame();
    }
    // A few frames more, and a moment: what is drawn lazily (the
    // Project's pictures) gets drawn, and git, asked on a thread, answers.
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_millis(1500) {
        studio.frame();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let (pw, ph) = ((width * scale) as u32, (height * scale) as u32);
    let target = OffscreenTarget::new(studio.session.gpu(), pw, ph);
    let mut renderer = studio.renderer(target.format());
    // Two frames: the first lays out and sizes the Scene view, the second
    // draws the scene at that size.
    for _ in 0..2 {
        studio.frame();
        studio.draw(&mut renderer, &target.ui_view(), pw, ph);
    }
    let pixels = target.read_rgba(studio.session.gpu());
    image::save_buffer(&out, &pixels, pw, ph, image::ExtendedColorType::Rgba8)?;
    println!("wrote {}", out.display());
    Ok(())
}
