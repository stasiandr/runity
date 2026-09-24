//! What the page tells the game, in the browser: a pad drawn on the screen
//! for a phone, which has no keys and usually no pad. The page's sticks and
//! buttons (`scrap_pad_axis`, `scrap_pad_button`, called from its
//! script) come in as a real pad's moves would, so a game's `input.ron`
//! reads them as `LeftX`/`LeftY` and `Pad(South)` without knowing they
//! were fingers.

use std::sync::Mutex;

use wasm_bindgen::prelude::wasm_bindgen;

use crate::input::{InputEvent, PadAxis, PadButton};

static WAITING: Mutex<Vec<InputEvent>> = Mutex::new(Vec::new());

/// What the page said since the last frame.
pub(crate) fn drain() -> Vec<InputEvent> {
    std::mem::take(&mut *WAITING.lock().unwrap())
}

/// A stick on the page moved: `axis` is `LeftX`, `LeftY`, `RightX` or
/// `RightY`, up and right positive, in `-1..=1`.
#[wasm_bindgen]
pub fn scrap_pad_axis(axis: &str, value: f32) {
    let axis = match axis {
        "LeftX" => PadAxis::LeftX,
        "LeftY" => PadAxis::LeftY,
        "RightX" => PadAxis::RightX,
        "RightY" => PadAxis::RightY,
        _ => return,
    };
    WAITING
        .lock()
        .unwrap()
        .push(InputEvent::PadMoved { axis, value });
}

/// A button on the page went down or up, by where it would be on a pad:
/// `South`, `East`, `West`, `North`, `Start`, `Select`.
#[wasm_bindgen]
pub fn scrap_pad_button(button: &str, down: bool) {
    let button = match button {
        "South" => PadButton::South,
        "East" => PadButton::East,
        "West" => PadButton::West,
        "North" => PadButton::North,
        "Start" => PadButton::Start,
        "Select" => PadButton::Select,
        _ => return,
    };
    WAITING.lock().unwrap().push(if down {
        InputEvent::PadDown(button)
    } else {
        InputEvent::PadUp(button)
    });
}
