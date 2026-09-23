//! GPUI's keyboard and mouse, as the engine's [`InputEvent`]s.
//!
//! The Scene view's behaviour is written once, over a frame of
//! [`runity::input::Input`] (`runity_editor::Session::scene_view`), and this
//! is the only place that knows a window delivered it. A key GPUI names and
//! this engine does not becomes [`Key::Other`], so a binding for it is still
//! possible and nothing is silently dropped.
//!
//! Modifiers are not keys in GPUI: every event carries the state of all four
//! instead. So they are not translated but *reconciled* — what the event
//! says is held is compared with what the engine thinks is held, and the
//! difference is sent as presses and releases. That also repairs the state
//! after a shortcut the window itself swallowed, which is how a Cmd held
//! through a menu used to get stuck down.

use gpui::{Keystroke, Modifiers};
use runity::input::{Input, InputEvent, Key, MouseButton};

/// The key a keystroke is about, by GPUI's name for it.
pub fn key_of(keystroke: &Keystroke) -> Option<Key> {
    let key = keystroke.key.as_str();
    let named = match key {
        "escape" => Key::Escape,
        "space" => Key::Space,
        "enter" => Key::Enter,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "insert" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        _ => return letter_or_digit(key),
    };
    Some(named)
}

fn letter_or_digit(key: &str) -> Option<Key> {
    let mut chars = key.chars();
    let (first, rest) = (chars.next()?, chars.next());
    if rest.is_some() {
        // A name this engine has no variant for — a dead key, a media key,
        // "fn". Carried by its first character rather than dropped, so a
        // rebindable editor can still name it.
        return Some(Key::Other(first as u32));
    }
    Some(match first.to_ascii_lowercase() {
        'a' => Key::A,
        'b' => Key::B,
        'c' => Key::C,
        'd' => Key::D,
        'e' => Key::E,
        'f' => Key::F,
        'g' => Key::G,
        'h' => Key::H,
        'i' => Key::I,
        'j' => Key::J,
        'k' => Key::K,
        'l' => Key::L,
        'm' => Key::M,
        'n' => Key::N,
        'o' => Key::O,
        'p' => Key::P,
        'q' => Key::Q,
        'r' => Key::R,
        's' => Key::S,
        't' => Key::T,
        'u' => Key::U,
        'v' => Key::V,
        'w' => Key::W,
        'x' => Key::X,
        'y' => Key::Y,
        'z' => Key::Z,
        '0' => Key::Digit0,
        '1' => Key::Digit1,
        '2' => Key::Digit2,
        '3' => Key::Digit3,
        '4' => Key::Digit4,
        '5' => Key::Digit5,
        '6' => Key::Digit6,
        '7' => Key::Digit7,
        '8' => Key::Digit8,
        '9' => Key::Digit9,
        other => Key::Other(other as u32),
    })
}

/// Bring the engine's idea of the modifiers in line with what this event
/// says they are.
pub fn sync_modifiers(input: &mut Input, modifiers: &Modifiers) {
    // The left-hand key of each pair stands for both: which one a person
    // pressed is not something GPUI says, and nothing in the editor asks.
    for (down, key) in [
        (modifiers.shift, Key::LeftShift),
        (modifiers.control, Key::LeftControl),
        (modifiers.alt, Key::LeftAlt),
        (modifiers.platform, Key::LeftSuper),
    ] {
        match (down, input.held(key)) {
            (true, false) => input.handle(&InputEvent::KeyDown(key)),
            (false, true) => input.handle(&InputEvent::KeyUp(key)),
            _ => {}
        }
    }
}

/// The engine's name for a mouse button.
pub fn button_of(button: gpui::MouseButton) -> Option<MouseButton> {
    match button {
        gpui::MouseButton::Left => Some(MouseButton::Left),
        gpui::MouseButton::Right => Some(MouseButton::Right),
        gpui::MouseButton::Middle => Some(MouseButton::Middle),
        _ => None,
    }
}
