//! Windows, input and presentation — without a windowing crate.
//!
//! Each backend talks to its operating system the way that OS actually wants to
//! be talked to:
//!
//! * `macos` drives AppKit through the Objective-C runtime (`objc_getClass`,
//!   `sel_registerName`, `objc_msgSend`) and presents frames as a `CGImage` on
//!   the content view's layer.
//! * `x11` speaks the X11 wire protocol over a Unix socket, using nothing but
//!   `std::os::unix::net::UnixStream`. No `libX11`, no `xcb`, no `unsafe`.
//! * `win32` declares the handful of `user32`/`gdi32` entry points it needs and
//!   links against the system libraries directly.
//! * `headless` implements the same trait with a `Vec<u32>`, which is what makes
//!   the engine testable on a machine with no display at all.

// The two backends that cannot avoid FFI opt out; everything else — including
// the whole X11 client — is safe Rust.
#![cfg_attr(all(not(windows), not(target_os = "macos")), forbid(unsafe_code))]

pub mod headless;
pub mod window;

#[cfg(target_os = "macos")]
pub mod macos;

/// Cocoa event decoding with no Objective-C in it, so it can be unit-tested on
/// any host.
#[cfg(any(target_os = "macos", test))]
mod macos_keys;

#[cfg(all(unix, not(target_os = "macos")))]
pub mod x11;

#[cfg(windows)]
pub mod win32;

pub use headless::HeadlessWindow;
pub use window::{Event, Key, MouseButton, NativeSurface, Window, WindowConfig};

use std::io;

/// Set this environment variable to run without a display.
pub const HEADLESS_ENV: &str = "RUNITY_HEADLESS";

fn headless_requested() -> bool {
    match std::env::var(HEADLESS_ENV) {
        Ok(v) => !matches!(v.as_str(), "" | "0" | "false"),
        Err(_) => false,
    }
}

/// Open a window using the best backend for this platform.
///
/// With `RUNITY_HEADLESS=1` set, or on a platform with no backend, this returns
/// a [`HeadlessWindow`] that renders into memory.
pub fn open_window(config: &WindowConfig) -> io::Result<Box<dyn Window>> {
    if headless_requested() {
        return Ok(Box::new(HeadlessWindow::new(config)));
    }

    #[cfg(target_os = "macos")]
    {
        return Ok(Box::new(macos::CocoaWindow::open(config)?));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return Ok(Box::new(x11::X11Window::open(config)?));
    }

    #[cfg(windows)]
    {
        return Ok(Box::new(win32::Win32Window::open(config)?));
    }

    #[allow(unreachable_code)]
    Ok(Box::new(HeadlessWindow::new(config)))
}

/// Like [`open_window`], but falls back to a headless window instead of failing
/// when there is no display (no `$DISPLAY`, an SSH session, CI, ...).
///
/// The returned flag tells you whether the fallback kicked in.
pub fn open_window_or_headless(config: &WindowConfig) -> (Box<dyn Window>, Option<io::Error>) {
    match open_window(config) {
        Ok(window) => (window, None),
        Err(e) => (Box::new(HeadlessWindow::new(config)), Some(e)),
    }
}
