//! `shot` — the editor's window, photographed without a screen.
//!
//! ```text
//! cargo run -p runity-studio --example shot -- [scene.ron] [out.png]
//! ```
//!
//! The window is opened off-screen and rendered by the real Metal renderer,
//! then read back as pixels (GPUI's `VisualTestAppContext`). So this is not
//! a mock of the editor: it is the editor, drawn by the same code that draws
//! it on screen — the layout, the fonts, the panels and the engine's frame
//! inside them.
//!
//! Two things it is for. An agent can see what it changed without asking a
//! person to look at a screen (DNA, postulate 5), and the same picture can
//! be held against a reference, which is how this repository has checked
//! renders since it had one.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{px, size, AppContext as _, VisualTestAppContext};

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

    let session = runity_studio::open(&scene)?;

    let platform = gpui_platform::current_platform(false);
    let mut cx = VisualTestAppContext::new(platform);
    let window = cx.open_offscreen_window(size(px(1440.0), px(900.0)), |_window, cx| {
        cx.new(|cx| runity_studio::Studio::new(session, cx))
    })?;

    // The Scene view asks for the next frame every frame, so a few frames
    // have to go by before the picture is the engine's and not the empty
    // panel behind it. Waiting for the wall clock rather than a frame count
    // keeps this honest on a slow machine.
    let until = Instant::now() + Duration::from_millis(300);
    while Instant::now() < until {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(16));
    }

    let image = cx.capture_screenshot(window.into())?;
    image.save(&out)?;
    println!("wrote {}", out.display());
    Ok(())
}
