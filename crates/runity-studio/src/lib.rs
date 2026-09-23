//! The editor, as a window.
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
//!
//! **macOS only, for now.** Not because GPUI is: because how the engine's
//! picture reaches a GPUI window is settled on macOS and nowhere else (DNA,
//! open question 1 — Windows is a shared D3D handle, and nobody has written
//! it). Building the window everywhere would mean every clone on every
//! platform compiling GPUI's dependency tree for a binary that cannot draw,
//! and postulate 1 is the reason not to.

#[cfg(target_os = "macos")]
mod keys;
#[cfg(target_os = "macos")]
mod studio;
#[cfg(target_os = "macos")]
mod viewport;

#[cfg(target_os = "macos")]
pub use studio::{open, run, Studio, REFERENCE_SCENE};
