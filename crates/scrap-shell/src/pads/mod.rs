//! Gamepads: one backend per platform behind one interface, by player
//! slot ([`PadSlots`]) — docs/gamepads.md.
//!
//! * macOS and iOS — Apple's GameController framework ([`gc`] over
//!   `apple`): what the system's game-controller daemon hands games. A
//!   DualSense over Bluetooth gives IOKit (gilrs) nothing on macOS 27, and
//!   gilrs has no iOS backend at all. Light bar, player LEDs, rumble and
//!   haptics, adaptive triggers, touchpad, motion, battery.
//! * Windows, Linux, the browser — gilrs (the browser's through its
//!   Gamepad API). Buttons, sticks, analog triggers; no output yet.
//!
//! `SCRAP_PADS=gilrs` takes gilrs on a Mac to compare, `SCRAP_PADS=none`
//! none. Only one backend runs: a pad is never read twice.

use scrap_core::input::{InputEvent, PadFeedback, PadInfo, PadSlots, TriggerThreshold};

pub mod gc;
pub(crate) mod gilrs_pads;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple;

pub use gilrs_pads::GilrsPads;

/// The environment variable that picks the backend: `gamecontroller`
/// (`gc`), `gilrs`, `none`.
pub const PADS_VAR: &str = "SCRAP_PADS";

/// A way of reading gamepads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Apple's GameController framework (macOS, iOS).
    GameController,
    /// gilrs: IOKit on a Mac, XInput/WGI on Windows, evdev on Linux, the
    /// Gamepad API in a browser.
    Gilrs,
    /// No pads read: keyboard, mouse, touch and scripts only.
    None,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::GameController => "gamecontroller",
            Backend::Gilrs => "gilrs",
            Backend::None => "none",
        }
    }
}

/// The backend to use: `SCRAP_PADS`'s when it names one this platform has,
/// otherwise GameController on Apple's platforms and gilrs elsewhere.
pub fn choose(var: Option<&str>, apple: bool) -> Backend {
    match var.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("gilrs") => Backend::Gilrs,
        Some("none" | "off" | "0") => Backend::None,
        Some("gc" | "gamecontroller") if apple => Backend::GameController,
        _ if apple => Backend::GameController,
        _ => Backend::Gilrs,
    }
}

/// [`choose`] for this platform and its environment.
pub fn chosen() -> Backend {
    choose(std::env::var(PADS_VAR).ok().as_deref(), cfg!(any(target_os = "macos", target_os = "ios")))
}

/// A pad as a listing shows it: its slot, what it is, and every element
/// the platform names on it (for `scrap pads`).
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub slot: usize,
    pub info: PadInfo,
    pub elements: Vec<String>,
}

/// A gamepad backend: reads devices into events on player slots, and
/// sends a game's [`PadFeedback`] to them.
pub trait PadSource {
    fn backend(&self) -> Backend;

    /// The pads now: connections, buttons and axes on their slots, into
    /// `out`. With `live` false (the window in the background) the pads
    /// are followed and nothing is said but their coming and going.
    /// Whether a button went down: a real pad in the hands.
    fn poll(&mut self, slots: &mut PadSlots, live: bool, out: &mut Vec<InputEvent>) -> bool;

    /// What the game asked of the pads' outputs. Takes its pulses even
    /// where there is no output, so none pile up.
    fn apply(&mut self, slots: &PadSlots, feedback: &mut PadFeedback, now: f64) {
        let _ = (slots, now);
        feedback.take_pulses();
    }

    /// The app goes to the background: haptics stop.
    fn suspend(&mut self) {}

    /// Turn the platform's event loop a moment, where nothing else does
    /// (the CLI's `scrap pads`, the editor's game): the window's loop does
    /// it for a game in a window.
    fn pump(&mut self, _seconds: f64) {}

    /// Where a trigger's analog pull counts as its button.
    fn set_thresholds(&mut self, _thresholds: TriggerThreshold) {}

    /// Every pad connected now.
    fn list(&self, slots: &PadSlots) -> Vec<Listed>;
}

/// The backend for `which`, or `None` when it cannot start (said on
/// stderr) or is [`Backend::None`]. `background` — read pads while the
/// window is in the background (the CLI, the editor's game).
pub fn open(which: Backend, background: bool) -> Option<Box<dyn PadSource>> {
    match which {
        Backend::None => None,
        Backend::Gilrs => match GilrsPads::new() {
            Ok(pads) => Some(Box::new(pads)),
            Err(e) => {
                eprintln!("no gamepads (gilrs): {e}");
                None
            }
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        Backend::GameController => Some(Box::new(gc::GcPads::new(apple::AppleControllers::new(background)))),
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        Backend::GameController => {
            let _ = background;
            eprintln!("no GameController framework on this platform: gilrs instead");
            open(Backend::Gilrs, background)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_platforms_read_pads_through_gamecontroller_and_others_through_gilrs() {
        assert_eq!(choose(None, true), Backend::GameController);
        assert_eq!(choose(None, false), Backend::Gilrs);
        assert_eq!(choose(Some("gilrs"), true), Backend::Gilrs, "the fallback to compare");
        assert_eq!(choose(Some("GC"), false), Backend::Gilrs, "no GameController off Apple");
        assert_eq!(choose(Some("none"), true), Backend::None);
        assert_eq!(choose(Some("what"), true), Backend::GameController);
    }
}
