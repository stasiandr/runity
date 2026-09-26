//! The editor, as a window.
//!
//! Everything the editor *does* lives in `scrap-editor`: one document open
//! for editing, and every change to it as a function, tested without a
//! window. This crate lays out the panels on `scrap-ui`, routes input
//! between them and the Scene view, and turns menu entries and buttons into
//! those functions — the same ones an agent calls over MCP, so a person's
//! edit and an agent's are one edit (DNA, postulate 5).
//!
//! ```text
//! scrap-studio [scene.ron]
//! ```
//!
//! With no scene named it opens the one the project last had open, or the
//! engine's reference scene, so that a fresh clone shows a picture.
//!
//! [`Studio`] is the whole editor without a window: the tests and the
//! `shot` example drive it off-screen. [`window::run`] is winit around it.

mod animator;
pub mod appearance;
mod bottom;
mod clipboard;
mod configs;
mod dialogues;
mod shader_graphs;
mod dock;
mod git_marks;
mod git_tab;
mod hierarchy;
mod inspector;
pub mod keymap;
pub mod layouts;
pub mod menu;
pub mod native_menu;
pub mod preferences;
mod screens;
mod studio;
mod table;
pub mod theme;
mod tools;
pub mod window;

use std::path::Path;

use scrap_editor::console::Level;
use scrap_editor::Session;

pub use studio::{MenuState, Studio, REFERENCE_SCENE};

/// A session with `scene` open, or why it could not be.
///
/// The size is a first guess: the Scene view resizes the session to
/// whatever the layout gives it on the first frame.
pub fn open(scene: &Path) -> Result<Session, String> {
    open_with(scene, false)
}

/// [`open`] for a window a person works in: the scene is drawn as its
/// meshes and pipelines come in rather than after all of them are built
/// ([`Session::draw_while_building`]).
pub fn open_live(scene: &Path) -> Result<Session, String> {
    open_with(scene, true)
}

fn open_with(scene: &Path, live: bool) -> Result<Session, String> {
    let mut session = Session::offscreen(1280, 720).map_err(|e| {
        format!("no renderer: {e}\nSCRAP_RENDERER and a working adapter are what this needs.")
    })?;
    if live {
        session.draw_while_building();
    }
    let missing = session
        .open_scene(scene)
        .map_err(|e| format!("cannot open {}: {e}", scene.display()))?;
    session.say(Level::Info, format!("opened {}", scene.display()));
    for problem in missing {
        session.say(Level::Warning, problem);
    }
    Ok(session)
}
