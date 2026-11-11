//! What the page tells the game, in the browser: a pad drawn on the screen
//! for a phone, which has no keys and usually no pad. The page's sticks and
//! buttons (`scrap_pad_axis`, `scrap_pad_button`, called from its
//! script) come in as a real pad's would — on a player slot of their own
//! ([`crate::shell::PagePad`]), which the page's `scrap_pad_shown` brings
//! in and takes out — so a game's `input.ron` reads them as
//! `LeftX`/`LeftY` and `Pad(South)`, and a game that seats players by
//! their pad's slot seats a thumb like a pad, without knowing they were
//! fingers.

use std::sync::Mutex;

use wasm_bindgen::prelude::wasm_bindgen;

use crate::input::{InputEvent, PadAxis, PadButton};
use crate::shell::PagePad;

static WAITING: Mutex<Vec<PagePad>> = Mutex::new(Vec::new());

/// What the page said since the last frame.
pub(crate) fn drain() -> Vec<PagePad> {
    std::mem::take(&mut *WAITING.lock().unwrap())
}

fn say(what: PagePad) {
    WAITING.lock().unwrap().push(what);
}

/// The page's pad came on the screen (`true`: its slot taken, the game
/// told a pad came) or went (its slot left, as a pad unplugged).
#[wasm_bindgen]
pub fn scrap_pad_shown(on: bool) {
    say(PagePad::Shown(on));
}

/// A stick or trigger on the page moved: `axis` is `LeftX`, `LeftY`,
/// `RightX`, `RightY` (up and right positive, in `-1..=1`), `LeftTrigger`
/// or `RightTrigger` (`0..=1`).
#[wasm_bindgen]
pub fn scrap_pad_axis(axis: &str, value: f32) {
    let axis = match axis {
        "LeftX" => PadAxis::LeftX,
        "LeftY" => PadAxis::LeftY,
        "RightX" => PadAxis::RightX,
        "RightY" => PadAxis::RightY,
        "LeftTrigger" => PadAxis::LeftTrigger,
        "RightTrigger" => PadAxis::RightTrigger,
        _ => return,
    };
    say(PagePad::Event(InputEvent::PadMoved { axis, value }));
}

/// A button on the page went down or up, by where it would be on a pad:
/// `South`, `East`, `West`, `North`, `LeftBumper`, `RightBumper`,
/// `LeftTrigger`, `RightTrigger`, `Select`, `Start`, `LeftStick`,
/// `RightStick`, `DPadUp`, `DPadDown`, `DPadLeft`, `DPadRight`.
#[wasm_bindgen]
pub fn scrap_pad_button(button: &str, down: bool) {
    let button = match button {
        "South" => PadButton::South,
        "East" => PadButton::East,
        "West" => PadButton::West,
        "North" => PadButton::North,
        "LeftBumper" => PadButton::LeftBumper,
        "RightBumper" => PadButton::RightBumper,
        "LeftTrigger" => PadButton::LeftTrigger,
        "RightTrigger" => PadButton::RightTrigger,
        "Select" => PadButton::Select,
        "Start" => PadButton::Start,
        "LeftStick" => PadButton::LeftStick,
        "RightStick" => PadButton::RightStick,
        "DPadUp" => PadButton::DPadUp,
        "DPadDown" => PadButton::DPadDown,
        "DPadLeft" => PadButton::DPadLeft,
        "DPadRight" => PadButton::DPadRight,
        _ => return,
    };
    say(PagePad::Event(if down {
        InputEvent::PadDown(button)
    } else {
        InputEvent::PadUp(button)
    }));
}
