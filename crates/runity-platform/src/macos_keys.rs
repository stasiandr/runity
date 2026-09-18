//! Cocoa event decoding that needs no Objective-C: virtual key codes, modifier
//! flags and event types.
//!
//! It lives apart from the FFI so it compiles — and is tested — on every
//! platform, not only on a Mac.

#![allow(dead_code)]

use crate::window::{Key, MouseButton};

// NSEventType
pub const NS_LEFT_MOUSE_DOWN: u64 = 1;
pub const NS_LEFT_MOUSE_UP: u64 = 2;
pub const NS_RIGHT_MOUSE_DOWN: u64 = 3;
pub const NS_RIGHT_MOUSE_UP: u64 = 4;
pub const NS_MOUSE_MOVED: u64 = 5;
pub const NS_LEFT_MOUSE_DRAGGED: u64 = 6;
pub const NS_RIGHT_MOUSE_DRAGGED: u64 = 7;
pub const NS_KEY_DOWN: u64 = 10;
pub const NS_KEY_UP: u64 = 11;
pub const NS_FLAGS_CHANGED: u64 = 12;
pub const NS_SCROLL_WHEEL: u64 = 22;
pub const NS_OTHER_MOUSE_DOWN: u64 = 25;
pub const NS_OTHER_MOUSE_UP: u64 = 26;
pub const NS_OTHER_MOUSE_DRAGGED: u64 = 27;

// NSEventModifierFlags
pub const MOD_SHIFT: u64 = 1 << 17;
pub const MOD_CONTROL: u64 = 1 << 18;
pub const MOD_OPTION: u64 = 1 << 19;
pub const MOD_COMMAND: u64 = 1 << 20;

/// Translate a `kVK_*` virtual key code.
///
/// These codes describe *positions* on an ANSI keyboard and do not change with
/// the user's layout, which is exactly what a game wants: W is the key above S
/// whether the layout is QWERTY, AZERTY or ЙЦУКЕН.
pub fn key_from_virtual_key(code: u16) -> Key {
    match code {
        0x00 => Key::A,
        0x0B => Key::B,
        0x08 => Key::C,
        0x02 => Key::D,
        0x0E => Key::E,
        0x03 => Key::F,
        0x05 => Key::G,
        0x04 => Key::H,
        0x22 => Key::I,
        0x26 => Key::J,
        0x28 => Key::K,
        0x25 => Key::L,
        0x2E => Key::M,
        0x2D => Key::N,
        0x1F => Key::O,
        0x23 => Key::P,
        0x0C => Key::Q,
        0x0F => Key::R,
        0x01 => Key::S,
        0x11 => Key::T,
        0x20 => Key::U,
        0x09 => Key::V,
        0x0D => Key::W,
        0x07 => Key::X,
        0x10 => Key::Y,
        0x06 => Key::Z,

        0x1D => Key::Num0,
        0x12 => Key::Num1,
        0x13 => Key::Num2,
        0x14 => Key::Num3,
        0x15 => Key::Num4,
        0x17 => Key::Num5,
        0x16 => Key::Num6,
        0x1A => Key::Num7,
        0x1C => Key::Num8,
        0x19 => Key::Num9,

        0x24 => Key::Enter,
        0x30 => Key::Tab,
        0x31 => Key::Space,
        0x33 => Key::Backspace,
        0x35 => Key::Escape,

        0x7B => Key::Left,
        0x7C => Key::Right,
        0x7D => Key::Down,
        0x7E => Key::Up,

        0x38 => Key::LeftShift,
        0x3C => Key::RightShift,
        0x3B => Key::LeftControl,
        0x3E => Key::RightControl,
        0x3A => Key::LeftAlt,
        0x3D => Key::RightAlt,
        0x37 => Key::LeftSuper,
        0x36 => Key::RightSuper,

        0x7A => Key::F1,
        0x78 => Key::F2,
        0x63 => Key::F3,
        0x76 => Key::F4,
        0x60 => Key::F5,
        0x61 => Key::F6,
        0x62 => Key::F7,
        0x64 => Key::F8,
        0x65 => Key::F9,
        0x6D => Key::F10,
        0x67 => Key::F11,
        0x6F => Key::F12,

        other => Key::Unknown(other as u32),
    }
}

/// Which mouse button an `NSEvent` of a given type and button number is.
pub fn mouse_button(event_type: u64, button_number: i64) -> Option<MouseButton> {
    match event_type {
        NS_LEFT_MOUSE_DOWN | NS_LEFT_MOUSE_UP => Some(MouseButton::Left),
        NS_RIGHT_MOUSE_DOWN | NS_RIGHT_MOUSE_UP => Some(MouseButton::Right),
        NS_OTHER_MOUSE_DOWN | NS_OTHER_MOUSE_UP => match button_number {
            2 => Some(MouseButton::Middle),
            n if (0..=255).contains(&n) => Some(MouseButton::Other(n as u8)),
            _ => None,
        },
        _ => None,
    }
}

/// Cocoa has no key events for modifiers — it sends `FlagsChanged` with the
/// whole mask, so presses and releases are the difference between two masks.
pub fn modifier_changes(previous: u64, current: u64) -> Vec<(Key, bool)> {
    const TRACKED: [(u64, Key); 4] = [
        (MOD_SHIFT, Key::LeftShift),
        (MOD_CONTROL, Key::LeftControl),
        (MOD_OPTION, Key::LeftAlt),
        (MOD_COMMAND, Key::LeftSuper),
    ];
    let mut changes = Vec::new();
    for (flag, key) in TRACKED {
        let was = previous & flag != 0;
        let is = current & flag != 0;
        if was != is {
            changes.push((key, is));
        }
    }
    changes
}

/// What the poll loop does with one `NSEvent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Decode it for the game and drop it; AppKit never sees it.
    Swallow,
    /// Offer it to the main menu by hand. Either way it is not passed on to
    /// `-[NSApplication sendEvent:]`; the game gets it only if no menu item
    /// claimed it.
    OfferToMenu,
    /// Hand it to `-[NSApplication sendEvent:]`, which is where window
    /// dragging, the close button and the menu bar live.
    AppKit,
}

/// Where one `NSEvent` has to go.
///
/// Everything but keys goes to AppKit, as it always did. Keys cannot: the
/// content view is a plain `NSView`, whose `acceptsFirstResponder` is `NO`, so
/// the first responder is the window, `-[NSWindow keyDown:]` has nothing to do
/// with the key, and `-[NSResponder noResponderFor:]` answers with `NSBeep()`.
/// Every keystroke in a game would ring.
///
/// Cmd is the exception to the exception: key equivalents are dispatched by the
/// menu bar, so Cmd+W and friends have to be offered to it explicitly — see
/// [`Route::OfferToMenu`]. Only `MOD_COMMAND` counts; Ctrl and Option belong to
/// the game, which binds them like any other key.
pub fn route(event_type: u64, modifier_flags: u64) -> Route {
    match event_type {
        NS_KEY_DOWN if modifier_flags & MOD_COMMAND != 0 => Route::OfferToMenu,
        NS_KEY_DOWN | NS_KEY_UP => Route::Swallow,
        _ => Route::AppKit,
    }
}

/// Cocoa's window origin is bottom-left; ours is top-left.
#[inline]
pub fn flip_y(y: f64, view_height: f64) -> f64 {
    view_height - y
}

/// Whether the polled window state means the user actually closed the window.
///
/// With no delegate, closing has to be read off the window itself, and
/// `-[NSWindow isVisible]` alone is not enough: a window miniaturized into the
/// Dock (Cmd+M) and every window of a hidden application (Cmd+H) report `NO`
/// too. Neither is a request to quit, so both are ruled out first.
#[inline]
pub fn window_was_closed(visible: bool, miniaturized: bool, app_hidden: bool) -> bool {
    !visible && !miniaturized && !app_hidden
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_follow_the_ansi_layout_positions() {
        // W A S D sit where a player expects them, regardless of layout.
        assert_eq!(key_from_virtual_key(0x0D), Key::W);
        assert_eq!(key_from_virtual_key(0x00), Key::A);
        assert_eq!(key_from_virtual_key(0x01), Key::S);
        assert_eq!(key_from_virtual_key(0x02), Key::D);
    }

    #[test]
    fn named_keys_map_to_their_variants() {
        assert_eq!(key_from_virtual_key(0x35), Key::Escape);
        assert_eq!(key_from_virtual_key(0x31), Key::Space);
        assert_eq!(key_from_virtual_key(0x7E), Key::Up);
        assert_eq!(key_from_virtual_key(0x6F), Key::F12);
        assert_eq!(key_from_virtual_key(0x0999), Key::Unknown(0x0999));
    }

    #[test]
    fn every_mapped_code_is_unique() {
        // A duplicated key code would silently shadow a key.
        let mut seen = std::collections::HashMap::new();
        for code in 0u16..=0x7F {
            let key = key_from_virtual_key(code);
            if matches!(key, Key::Unknown(_)) {
                continue;
            }
            if let Some(previous) = seen.insert(key, code) {
                panic!("{key:?} is mapped from both {previous:#04x} and {code:#04x}");
            }
        }
        assert!(seen.len() > 50, "only {} keys mapped", seen.len());
    }

    #[test]
    fn modifier_changes_are_edges_not_levels() {
        assert_eq!(modifier_changes(0, MOD_SHIFT), vec![(Key::LeftShift, true)]);
        assert_eq!(
            modifier_changes(MOD_SHIFT, MOD_SHIFT),
            vec![],
            "held is not an edge"
        );
        assert_eq!(
            modifier_changes(MOD_SHIFT, 0),
            vec![(Key::LeftShift, false)]
        );

        let both = MOD_SHIFT | MOD_CONTROL;
        assert_eq!(
            modifier_changes(MOD_SHIFT, both),
            vec![(Key::LeftControl, true)],
            "only the flag that moved is reported"
        );
    }

    #[test]
    fn mouse_buttons_are_decoded_per_event_type() {
        assert_eq!(mouse_button(NS_LEFT_MOUSE_DOWN, 0), Some(MouseButton::Left));
        assert_eq!(mouse_button(NS_RIGHT_MOUSE_UP, 1), Some(MouseButton::Right));
        assert_eq!(
            mouse_button(NS_OTHER_MOUSE_DOWN, 2),
            Some(MouseButton::Middle)
        );
        assert_eq!(
            mouse_button(NS_OTHER_MOUSE_DOWN, 4),
            Some(MouseButton::Other(4))
        );
        assert_eq!(mouse_button(NS_KEY_DOWN, 0), None);
    }

    #[test]
    fn only_a_real_close_ends_the_app() {
        // visible, miniaturized, app_hidden
        assert!(
            window_was_closed(false, false, false),
            "an invisible window that is neither in the Dock nor hidden was closed"
        );
        assert!(
            !window_was_closed(false, true, false),
            "Cmd+M puts the window in the Dock; the app keeps running"
        );
        assert!(
            !window_was_closed(false, false, true),
            "Cmd+H hides the app; the app keeps running"
        );
        assert!(
            !window_was_closed(false, true, true),
            "hiding an app whose window is already in the Dock is still not a close"
        );
        assert!(!window_was_closed(true, false, false), "plainly on screen");
    }

    #[test]
    fn plain_keys_never_reach_appkit() {
        // The whole point: a key AppKit sees with no responder for it beeps.
        assert_eq!(route(NS_KEY_DOWN, 0), Route::Swallow, "W is just W");
        assert_eq!(route(NS_KEY_DOWN, MOD_SHIFT), Route::Swallow);
        assert_eq!(
            route(NS_KEY_DOWN, MOD_CONTROL | MOD_OPTION),
            Route::Swallow,
            "Ctrl and Option are the game's to bind"
        );
        assert_eq!(route(NS_KEY_UP, 0), Route::Swallow);
        assert_eq!(
            route(NS_KEY_UP, MOD_COMMAND),
            Route::Swallow,
            "a release is never a key equivalent"
        );
    }

    #[test]
    fn cmd_keys_are_offered_to_the_menu() {
        assert_eq!(route(NS_KEY_DOWN, MOD_COMMAND), Route::OfferToMenu);
        assert_eq!(
            route(NS_KEY_DOWN, MOD_COMMAND | MOD_CONTROL),
            Route::OfferToMenu,
            "Cmd anywhere in the mask is still Cmd"
        );
        assert_eq!(
            route(NS_KEY_DOWN, MOD_COMMAND | MOD_SHIFT | MOD_OPTION),
            Route::OfferToMenu
        );
    }

    #[test]
    fn everything_that_is_not_a_key_still_goes_to_appkit() {
        // Dragging the title bar, the close button and the menu bar all need it.
        for event_type in [
            NS_LEFT_MOUSE_DOWN,
            NS_LEFT_MOUSE_UP,
            NS_RIGHT_MOUSE_DOWN,
            NS_MOUSE_MOVED,
            NS_LEFT_MOUSE_DRAGGED,
            NS_SCROLL_WHEEL,
            NS_OTHER_MOUSE_DOWN,
            NS_FLAGS_CHANGED,
            0,
            u64::MAX,
        ] {
            assert_eq!(
                route(event_type, 0),
                Route::AppKit,
                "event type {event_type} must keep going to sendEvent:"
            );
            assert_eq!(
                route(event_type, MOD_COMMAND),
                Route::AppKit,
                "holding Cmd does not turn event type {event_type} into a key"
            );
        }
    }

    #[test]
    fn y_is_flipped_into_our_top_left_space() {
        assert_eq!(
            flip_y(0.0, 600.0),
            600.0,
            "Cocoa's bottom edge is our bottom edge"
        );
        assert_eq!(flip_y(600.0, 600.0), 0.0, "Cocoa's top is our origin");
    }
}
