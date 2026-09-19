//! Hardware rendering for runity, on Metal.
//!
//! The software rasterizer in `runity-render` is the reference: it is where
//! the pipeline is written down, it is what the golden images are made of, and
//! it runs on a build machine with no display and no GPU. This crate is the
//! other thing — the path a game actually runs on, drawing the same frames on
//! the hardware.
//!
//! What keeps the two honest is that they are compared rather than trusted.
//! [`diff::assert_agrees`] draws one closure twice, once through the
//! rasterizer and once through an offscreen Metal target, and holds the frames
//! against each other pixel by pixel. A shader that exists only on one side is
//! not a shader this engine ships.
//!
//! ```no_run
//! use runity_gpu::{Gpu, Target};
//! use runity_render::{Color, Framebuffer};
//!
//! let mut gpu = Gpu::new().expect("a Metal device");
//! let mut frame = Framebuffer::new(64, 48);
//! if gpu.begin_frame(Target::Offscreen { width: 64, height: 48 }, Color::BLACK) {
//!     // ... gpu.draw(&mesh, &shader, ...) ...
//!     gpu.end_frame(Some(&mut frame));
//! }
//! ```
//!
//! **Where the `unsafe` is.** All of it is the Objective-C runtime and the
//! Metal framework, in [`metal`]. `runity-render` stays
//! `#![forbid(unsafe_code)]`; this crate takes data types from it —
//! [`runity_render::Mesh`], `Vertex`, `Color`, `Texture`, `Framebuffer`,
//! `Blend`, `CullMode` — and nothing else.

pub mod diff;
pub mod lines;
pub mod shader;
pub mod wire;

#[cfg(target_os = "macos")]
pub mod metal;
#[cfg(not(target_os = "macos"))]
mod unsupported;

#[cfg(target_os = "macos")]
pub use metal::{Gpu, Target};
#[cfg(not(target_os = "macos"))]
pub use unsupported::{Gpu, Target};

pub use shader::{
    assert_uniform_layout, BasicUniforms, GpuShader, PulseUniforms, UnlitUniforms, ENGINE_SOURCE,
    PULSE_FRAGMENT, PULSE_VERTEX,
};
pub use wire::{Float4, GpuVertex, Matrix};

use std::fmt;

/// Set this to `cpu` or `gpu` to override which renderer the main loop builds.
pub const RENDERER_ENV: &str = "RUNITY_RENDERER";

/// Every way starting the GPU renderer can fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuError {
    /// This is not macOS. The backend is Metal, and Metal is only there.
    Unsupported,
    /// `MTLCreateSystemDefaultDevice` returned nil, or the device would not
    /// make a command queue.
    NoDevice,
    /// There is no native view to put a `CAMetalLayer` on — a headless window,
    /// or a backend that does not have one.
    NoSurface,
    /// The Metal compiler said no. The text is its `localizedDescription`,
    /// which names the line.
    ShaderCompilation(String),
}

/// The sentence every failure message ends with. Whatever went wrong, there is
/// always a way to keep working.
pub const FALLBACK_ADVICE: &str =
    "run with RUNITY_RENDERER=cpu to work on the software rasterizer.";

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuError::Unsupported => {
                write!(
                    f,
                    "the GPU backend is built for macOS only; {FALLBACK_ADVICE}"
                )
            }
            GpuError::NoDevice => write!(
                f,
                "no Metal device is available on this machine; {FALLBACK_ADVICE}"
            ),
            GpuError::NoSurface => write!(
                f,
                "this window has no native surface to present a Metal layer on; \
                 {FALLBACK_ADVICE}"
            ),
            GpuError::ShaderCompilation(reason) => write!(
                f,
                "the Metal shaders did not compile:\n{reason}\n{FALLBACK_ADVICE}"
            ),
        }
    }
}

impl std::error::Error for GpuError {}

impl From<GpuError> for std::io::Error {
    fn from(error: GpuError) -> Self {
        std::io::Error::other(error.to_string())
    }
}

/// Whether a GPU renderer can be built here at all.
///
/// Cheap and side-effect free on a platform with no backend; on macOS it opens
/// the device, so call it once and keep the answer.
pub fn available() -> bool {
    Gpu::new().is_ok()
}

/// Show the reason a GPU start-up failed and leave with a non-zero status.
///
/// On macOS that means an `NSAlert` as well as stderr, because the app may
/// well have been double-clicked and a silent exit tells nobody anything.
pub fn fail(error: &GpuError) -> ! {
    #[cfg(target_os = "macos")]
    {
        metal::alert::fail("SmallWorld cannot start on the GPU", &error.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("runity: {error}");
        std::process::exit(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_failure_says_how_to_keep_working() {
        for error in [
            GpuError::Unsupported,
            GpuError::NoDevice,
            GpuError::NoSurface,
            GpuError::ShaderCompilation("program_source:12:5: error: no".to_string()),
        ] {
            let text = error.to_string();
            assert!(
                text.trim_end().ends_with(FALLBACK_ADVICE),
                "{error:?} does not end with the way out: {text}"
            );
        }
    }

    #[test]
    fn a_compiler_message_survives_into_the_text() {
        let error = GpuError::ShaderCompilation("program_source:12:5: error: no".to_string());
        assert!(error.to_string().contains("program_source:12:5"));
    }

    #[test]
    fn a_gpu_error_converts_to_an_io_error_without_losing_the_reason() {
        let io: std::io::Error = GpuError::NoDevice.into();
        assert!(io.to_string().contains("no Metal device"));
    }
}
