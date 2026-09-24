//! The desktop shell: a plain window for running a game without the
//! editor — winit's window, gilrs' gamepads, the fixed step and the frame,
//! hot patches through subsecond ([`shell::run`]). The engine never
//! creates a window (it is a guest); this module is the host that does,
//! on the desktop. The editor, iOS and Android are hosts of their own.

pub mod shell;

pub use shell::{run, Game, WindowConfig};

// The core, the GPU, the render and the overlay, under the names this
// module's code knows them by.
#[allow(unused_imports)]
use runity_core::{input, time};
#[allow(unused_imports)]
use runity_gpu::{gpu, surface};
#[allow(unused_imports)]
use runity_overlay::{ui, ui_render};
#[allow(unused_imports)]
use runity_render::render;
