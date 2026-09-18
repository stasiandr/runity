//! Translation from X11 keysyms to [`Key`].
//!
//! X11 hands out *keycodes*, which are hardware- and driver-specific. The
//! portable name of a key is its *keysym*, which the server gives us via the
//! `GetKeyboardMapping` request at startup; this module maps those to our enum.

use crate::window::Key;

pub fn key_from_keysym(keysym: u32) -> Key {
    match keysym {
        // Latin letters, upper and lower case map to the same physical key.
        0x61..=0x7a => letter(keysym - 0x61),
        0x41..=0x5a => letter(keysym - 0x41),
        0x30 => Key::Num0,
        0x31 => Key::Num1,
        0x32 => Key::Num2,
        0x33 => Key::Num3,
        0x34 => Key::Num4,
        0x35 => Key::Num5,
        0x36 => Key::Num6,
        0x37 => Key::Num7,
        0x38 => Key::Num8,
        0x39 => Key::Num9,
        0x20 => Key::Space,
        0xff08 => Key::Backspace,
        0xff09 => Key::Tab,
        0xff0d | 0xff8d => Key::Enter,
        0xff1b => Key::Escape,
        0xff51 => Key::Left,
        0xff52 => Key::Up,
        0xff53 => Key::Right,
        0xff54 => Key::Down,
        0xffe1 => Key::LeftShift,
        0xffe2 => Key::RightShift,
        0xffe3 => Key::LeftControl,
        0xffe4 => Key::RightControl,
        0xffe9 => Key::LeftAlt,
        0xffea | 0xfe03 => Key::RightAlt,
        0xffbe..=0xffc9 => function_key(keysym - 0xffbe),
        other => Key::Unknown(other),
    }
}

fn letter(offset: u32) -> Key {
    const LETTERS: [Key; 26] = [
        Key::A,
        Key::B,
        Key::C,
        Key::D,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::S,
        Key::T,
        Key::U,
        Key::V,
        Key::W,
        Key::X,
        Key::Y,
        Key::Z,
    ];
    LETTERS[offset as usize]
}

fn function_key(offset: u32) -> Key {
    const KEYS: [Key; 12] = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
    ];
    KEYS[offset as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_are_case_insensitive() {
        assert_eq!(key_from_keysym(0x77), Key::W);
        assert_eq!(key_from_keysym(0x57), Key::W);
    }

    #[test]
    fn named_keys_map_to_their_variants() {
        assert_eq!(key_from_keysym(0xff1b), Key::Escape);
        assert_eq!(key_from_keysym(0x20), Key::Space);
        assert_eq!(key_from_keysym(0xff53), Key::Right);
        assert_eq!(key_from_keysym(0xffc9), Key::F12);
        assert_eq!(key_from_keysym(0xffbe), Key::F1);
    }

    #[test]
    fn unrecognized_keysyms_are_preserved() {
        assert_eq!(key_from_keysym(0x1234_5678), Key::Unknown(0x1234_5678));
    }
}
