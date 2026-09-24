//! The game's overlay, as it was before `runity-ui`: a frame's interface
//! as a list of quads and text runs ([`ui`]), immediate widgets that turn
//! input into clicks on it ([`widgets`]), screens written in RON and
//! reloaded while the game runs ([`screen`]), and the GPU drawing the
//! list over a frame ([`ui_render`]).
//!
//! A module of its own so that the engine does not carry it: it stands on
//! the core and the GPU, and the render module draws it in the world. It
//! leaves when the game's dialogues and widgets move to `runity-ui` — one
//! UI for the game and the editor (DNA; docs/ui.md, stage 4).

pub mod screen;
pub mod ui;
pub mod ui_render;
pub mod widgets;

pub use ui::{Quad, TextRun, Ui};
pub use ui_render::UiRenderer;

// The core and the GPU, under the names this module's code knows them by.
#[allow(unused_imports)]
use runity_core::{input, strings, Tuned};
#[allow(unused_imports)]
use runity_gpu::{gpu, surface};
