//! The desktop shell: a plain window for running a game without the
//! editor — winit's window, gilrs' gamepads, the fixed step and the frame,
//! hot patches through subsecond ([`shell::run`]). The engine never
//! creates a window (it is a guest); this module is the host that does,
//! on the desktop. The editor, iOS and Android are hosts of their own.

pub mod shell;
/// Sticks and buttons drawn on a touch screen, fed in as a pad's.
pub mod touch_pad;
/// The scene delegate UIKit asks for, holding winit's window.
#[cfg(target_os = "ios")]
mod ios;
/// The page's own controls, in the browser: sticks and buttons on screen.
#[cfg(target_arch = "wasm32")]
pub mod web;

pub use shell::{run, Game, StepContext, WindowConfig};
pub use touch_pad::{TouchButton, TouchLayout, TouchPad};

// The core, the GPU, the render and the overlay, under the names this
// module's code knows them by.
#[allow(unused_imports)]
use scrap_core::{input, time};
#[allow(unused_imports)]
use scrap_gpu::{gpu, surface};
#[allow(unused_imports)]
use scrap_overlay::{ui, ui_render};
#[allow(unused_imports)]
use scrap_render::render;

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = scrap_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "shell");
        let problems = manifest.part_problems(&Vec::new());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
