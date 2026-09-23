//! The window, its layout, and the session it has open.
//!
//! Everything the editor *does* lives in `runity-editor`: one document open
//! for editing, and every change to it as a function, tested without a
//! window. This binary owns the window, the layout and the keyboard, and
//! calls those functions — the same ones an agent calls over MCP, so a
//! person's edit and an agent's are one edit (DNA, postulate 5).
//!
//! ```text
//! runity-studio [scene.ron]
//! ```
//!
//! With no scene named it opens the engine's reference scene, so that a
//! fresh clone shows a picture rather than an empty grid.

use std::path::{Path, PathBuf};

use crate::viewport::SceneView;
use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, Context, Entity, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};
use runity_editor::Session;

/// The engine's reference scene: every builtin, no import step.
const REFERENCE_SCENE: &str = "examples/valley/scenes/first-light.ron";

/// The editor window: the Scene view, and the panels that will grow around
/// it.
struct Studio {
    scene_view: Entity<SceneView>,
}

impl Render for Studio {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x14141a))
            .text_color(rgb(0xd6d6dc))
            .child(div().size_full().child(self.scene_view.clone()))
    }
}

/// Open the window and run until it closes.
pub fn run() {
    let scene = std::env::args().nth(1).map(PathBuf::from);
    let scene = scene.unwrap_or_else(|| PathBuf::from(REFERENCE_SCENE));

    let session = match open(&scene) {
        Ok(session) => session,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    let title = scene
        .file_name()
        .map(|name| format!("runity — {}", name.to_string_lossy()))
        .unwrap_or_else(|| "runity".to_string());

    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(title.into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_window, cx| {
                let scene_view = cx.new(|cx| SceneView::new(session, cx));
                cx.new(|_| Studio { scene_view })
            },
        );
        if let Err(error) = window {
            eprintln!("no window: {error}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}

/// A session with `scene` open, or why it could not be.
///
/// The size here is a first guess: the view resizes the session to whatever
/// the window gives it on the first frame.
fn open(scene: &Path) -> Result<Session, String> {
    let mut session = Session::offscreen(1280, 720).map_err(|e| {
        format!("no renderer: {e}\nRUNITY_RENDERER and a working adapter are what this needs.")
    })?;
    let missing = session
        .open_scene(scene)
        .map_err(|e| format!("cannot open {}: {e}", scene.display()))?;
    for problem in missing {
        eprintln!("{problem}");
    }
    Ok(session)
}
