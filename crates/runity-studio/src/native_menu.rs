//! The menus in the system's menu bar, where a system has one (macOS).
//!
//! The same [`menu::menu_bar`] the toolbar draws, turned into the
//! platform's menus: its titles, lines, separators and keys, with the
//! application menu Mac apps have in front ("runity": About, Hide, Quit).
//! A line chosen there runs the same [`Action`] as the line in the
//! toolbar's menu does; the toolbar's menus go away (`Studio::
//! set_native_menu`). Elsewhere the toolbar keeps drawing them, and the
//! tests drive those.
//!
//! **One owner for keys: the studio.** Keys reach the studio as keys on
//! every platform, and the Scene view's handling (`Session::scene_view`)
//! and the UI's fields answer them — the in-window menus only *show* them.
//! A Mac menu cannot show a key without answering it: AppKit hands a
//! ⌘-key to the menu before the window sees it, and the window never gets
//! it. So a line chosen *by its key* is not run as an action: the key goes
//! on to the studio as the key it was ([`Chosen::by_key`]), and ⌘C in a
//! text field still copies the text, ⌘Z still undoes the typing. Only a
//! line chosen with the mouse runs its action. Nothing fires twice, and
//! the keys do the same with the menus in the window or in the bar.
//!
//! A key that types something (a letter, Space) with neither ⌘ nor Ctrl
//! is not given to the system menu at all ([`Shortcut::native`]): AppKit
//! would take it from a text field before the field could type it.
//!
//! What is built here without a display — [`tree`] and [`Shortcut`] — is
//! tested on every platform; `mac` puts it into `muda`.

use runity::input::Key;

use crate::menu::{Action, MenuItem};

/// A shortcut as the menus write it — "⇧⌘Z", "Ctrl+Shift+P", "F2" — read
/// back into keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub command: bool,
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Shortcut {
    /// Read a shortcut: the Mac's glyphs in front of a key ("⌥⌘P"), or
    /// names joined by `+` ("Ctrl+Alt+P"). `None` for what is not one.
    pub fn parse(text: &str) -> Option<Self> {
        let mut shortcut = Shortcut {
            command: false,
            control: false,
            shift: false,
            alt: false,
            key: Key::Escape,
        };
        let mut rest = text.trim();
        // The glyphs, in any order, in front.
        loop {
            let mut chars = rest.chars();
            match chars.next() {
                Some('⌘') => shortcut.command = true,
                Some('⇧') => shortcut.shift = true,
                Some('⌥') => shortcut.alt = true,
                Some('⌃') => shortcut.control = true,
                _ => break,
            }
            rest = chars.as_str();
        }
        // Then names with `+`; the last is the key. A lone "+" is a key.
        let parts: Vec<&str> = if rest.len() > 1 {
            rest.split('+').map(str::trim).collect()
        } else {
            vec![rest]
        };
        let (key, modifiers) = parts.split_last()?;
        for m in modifiers {
            match m.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "super" => shortcut.command = true,
                "ctrl" | "control" => shortcut.control = true,
                "shift" => shortcut.shift = true,
                "alt" | "option" | "opt" => shortcut.alt = true,
                _ => return None,
            }
        }
        shortcut.key = key_named(key)?;
        Some(shortcut)
    }

    /// Whether the system menu may answer this key: with ⌘ or Ctrl held,
    /// or a key that types nothing (F2, Delete, Esc, End).
    pub fn native(&self) -> bool {
        self.command || self.control || !types(self.key)
    }
}

/// Whether a key alone puts a character into a text field.
fn types(key: Key) -> bool {
    use Key::*;
    matches!(
        key,
        A | B
            | C
            | D
            | E
            | F
            | G
            | H
            | I
            | J
            | K
            | L
            | M
            | N
            | O
            | P
            | Q
            | R
            | S
            | T
            | U
            | V
            | W
            | X
            | Y
            | Z
            | Digit0
            | Digit1
            | Digit2
            | Digit3
            | Digit4
            | Digit5
            | Digit6
            | Digit7
            | Digit8
            | Digit9
            | Space
            | Enter
            | Tab
            | Backspace
    )
}

/// A key by the name a menu gives it.
fn key_named(name: &str) -> Option<Key> {
    use Key::*;
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        let letters = [
            A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
        ];
        let digits = [
            Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
        ];
        let c = c.to_ascii_uppercase();
        return match c {
            'A'..='Z' => Some(letters[c as usize - 'A' as usize]),
            '0'..='9' => Some(digits[c as usize - '0' as usize]),
            '⌫' => Some(Backspace),
            '⌦' => Some(Delete),
            '⎋' => Some(Escape),
            '↩' => Some(Enter),
            '⇥' => Some(Tab),
            '←' => Some(Left),
            '→' => Some(Right),
            '↑' => Some(Up),
            '↓' => Some(Down),
            _ => None,
        };
    }
    let fs = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    Some(match name.to_ascii_lowercase().as_str() {
        "esc" | "escape" => Escape,
        "space" => Space,
        "enter" | "return" => Enter,
        "tab" => Tab,
        "backspace" => Backspace,
        "delete" | "del" => Delete,
        "insert" | "ins" => Insert,
        "home" => Home,
        "end" => End,
        "pageup" | "page up" => PageUp,
        "pagedown" | "page down" => PageDown,
        "left" => Left,
        "right" => Right,
        "up" => Up,
        "down" => Down,
        f => {
            let n: usize = f.strip_prefix('f')?.parse().ok()?;
            *fs.get(n.checked_sub(1)?)?
        }
    })
}

/// The application menu's own lines, which the system knows how to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standard {
    About,
    Hide,
    HideOthers,
    ShowAll,
}

/// One line of a menu in the bar.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// A line that runs an action. `id` is how the system names it back
    /// when it is chosen; `check`, whether it shows a tick for a state.
    Item {
        id: String,
        label: String,
        shortcut: Option<Shortcut>,
        action: Action,
        check: bool,
    },
    Separator,
    Standard(Standard),
    /// Quit (⌘Q): the window's close, which asks about unsaved work, not
    /// the system's, which would not.
    Quit,
}

/// A menu of the bar: its title and its lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Submenu {
    pub title: String,
    pub entries: Vec<Entry>,
}

/// The name the application menu goes by.
pub const APP: &str = "runity";

/// The bar as the system shows it: the application menu, then `bar`'s
/// menus in their order. Keys the system must not answer
/// ([`Shortcut::native`]) are left off.
pub fn tree(bar: Vec<(&'static str, Vec<MenuItem>)>) -> Vec<Submenu> {
    let mut menus = vec![Submenu {
        title: APP.into(),
        entries: vec![
            Entry::Standard(Standard::About),
            Entry::Separator,
            Entry::Standard(Standard::Hide),
            Entry::Standard(Standard::HideOthers),
            Entry::Standard(Standard::ShowAll),
            Entry::Separator,
            Entry::Quit,
        ],
    }];
    for (title, items) in bar {
        let entries = items
            .into_iter()
            .enumerate()
            .map(|(i, item)| match item.action {
                None => Entry::Separator,
                Some(action) => Entry::Item {
                    id: format!("{title}/{i}"),
                    label: item.label,
                    shortcut: item
                        .shortcut
                        .and_then(Shortcut::parse)
                        .filter(Shortcut::native),
                    check: toggles(&action),
                    action,
                },
            })
            .collect();
        menus.push(Submenu {
            title: title.into(),
            entries,
        });
    }
    menus
}

/// Whether a line of the menu is a state that is on or off: it shows a
/// tick (`Studio::menu_state` says which).
pub fn toggles(action: &Action) -> bool {
    matches!(
        action,
        Action::ToggleGrid
            | Action::ToggleColliders
            | Action::ToggleSnap
            | Action::ToggleNavigation
            | Action::GameView(_)
            | Action::TogglePanel(_)
            | Action::Maximize
            | Action::Play
            | Action::Pause
    )
}

/// A line chosen in the system's menu, as the window hears of it: which,
/// and whether by its key rather than by the mouse.
#[derive(Debug, Clone)]
pub struct Chosen {
    pub id: String,
    pub by_key: bool,
}

#[cfg(target_os = "macos")]
pub mod mac {
    //! The tree in `muda`'s menus, set as the application's.

    use std::collections::HashMap;

    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{
        AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    };
    use runity::input::Key;

    use super::{Chosen, Entry, Shortcut, Standard};
    use crate::menu::Action;
    use crate::studio::Studio;

    enum Line {
        Plain(MenuItem),
        Check(CheckMenuItem),
    }

    /// What a chosen line asks for.
    pub enum Asked {
        /// Run the action (chosen with the mouse).
        Run(Action),
        /// Hand the studio this key (chosen by it).
        Key(Shortcut),
        Quit,
    }

    /// The menu bar, alive while the application runs.
    pub struct MenuBar {
        _menu: Menu,
        lines: Vec<(Line, Action, String, Option<Shortcut>)>,
        by_id: HashMap<String, usize>,
        quit: String,
        /// The studio's menu revision the lines last showed.
        seen: Option<u64>,
    }

    impl MenuBar {
        /// Build the bar from the menus and make it the application's.
        /// After the application has launched: before, winit would put
        /// its own over it.
        pub fn install() -> Self {
            let menu = Menu::new();
            let mut lines = Vec::new();
            let mut by_id = HashMap::new();
            let quit_id = "quit".to_string();
            for sub in super::tree(crate::menu::menu_bar()) {
                let submenu = Submenu::new(&sub.title, true);
                for entry in sub.entries {
                    let _ = match entry {
                        Entry::Separator => submenu.append(&PredefinedMenuItem::separator()),
                        Entry::Standard(Standard::About) => {
                            submenu.append(&PredefinedMenuItem::about(
                                Some("About runity"),
                                Some(AboutMetadata {
                                    name: Some("runity".into()),
                                    version: Some(env!("CARGO_PKG_VERSION").into()),
                                    ..Default::default()
                                }),
                            ))
                        }
                        Entry::Standard(Standard::Hide) => {
                            submenu.append(&PredefinedMenuItem::hide(Some("Hide runity")))
                        }
                        Entry::Standard(Standard::HideOthers) => {
                            submenu.append(&PredefinedMenuItem::hide_others(None))
                        }
                        Entry::Standard(Standard::ShowAll) => {
                            submenu.append(&PredefinedMenuItem::show_all(None))
                        }
                        Entry::Quit => submenu.append(&MenuItem::with_id(
                            quit_id.clone(),
                            "Quit runity",
                            true,
                            Some(Accelerator::new(Modifiers::META, Code::KeyQ)),
                        )),
                        Entry::Item {
                            id,
                            label,
                            shortcut,
                            action,
                            check,
                        } => {
                            let accelerator = shortcut.and_then(accelerator);
                            let line = if check {
                                Line::Check(CheckMenuItem::with_id(
                                    id.clone(),
                                    &label,
                                    true,
                                    false,
                                    accelerator,
                                ))
                            } else {
                                Line::Plain(MenuItem::with_id(
                                    id.clone(),
                                    &label,
                                    true,
                                    accelerator,
                                ))
                            };
                            let appended = match &line {
                                Line::Plain(item) => submenu.append(item),
                                Line::Check(item) => submenu.append(item),
                            };
                            by_id.insert(id, lines.len());
                            lines.push((line, action, label, shortcut));
                            appended
                        }
                    };
                }
                if sub.title == "Window" {
                    submenu.set_as_windows_menu_for_nsapp();
                }
                let _ = menu.append(&submenu);
            }
            menu.init_for_nsapp();
            Self {
                _menu: menu,
                lines,
                by_id,
                quit: quit_id,
                seen: None,
            }
        }

        /// Bring the lines' labels, ticks and greying up to date, when
        /// the studio says something they show has changed.
        pub fn refresh(&mut self, studio: &Studio) {
            let revision = studio.menu_revision();
            if self.seen == Some(revision) {
                return;
            }
            self.seen = Some(revision);
            for (line, action, label, _) in &self.lines {
                let state = studio.menu_state(action, label);
                match line {
                    Line::Plain(item) => {
                        if item.text() != state.label {
                            item.set_text(&state.label);
                        }
                        item.set_enabled(state.enabled);
                    }
                    Line::Check(item) => {
                        item.set_enabled(state.enabled);
                        item.set_checked(state.checked);
                    }
                }
            }
        }

        /// What the line the system named asks for.
        pub fn asked(&self, chosen: &Chosen) -> Option<Asked> {
            if chosen.id == self.quit {
                return Some(Asked::Quit);
            }
            let (_, action, _, shortcut) = &self.lines[*self.by_id.get(&chosen.id)?];
            Some(match shortcut {
                Some(key) if chosen.by_key => Asked::Key(*key),
                _ => Asked::Run(action.clone()),
            })
        }
    }

    /// Send every line chosen to `send`, noting whether a key chose it: the
    /// event being handled when a menu answers a key equivalent is that
    /// key's.
    pub fn listen(send: impl Fn(Chosen) + Send + Sync + 'static) {
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let by_key = objc2::MainThreadMarker::new()
                .and_then(|mtm| objc2_app_kit::NSApplication::sharedApplication(mtm).currentEvent())
                .is_some_and(|e| e.r#type() == objc2_app_kit::NSEventType::KeyDown);
            send(Chosen {
                id: event.id.0,
                by_key,
            });
        }));
    }

    /// A shortcut as the system's accelerator.
    fn accelerator(shortcut: Shortcut) -> Option<Accelerator> {
        let mut mods = Modifiers::empty();
        for (on, m) in [
            (shortcut.command, Modifiers::META),
            (shortcut.control, Modifiers::CONTROL),
            (shortcut.shift, Modifiers::SHIFT),
            (shortcut.alt, Modifiers::ALT),
        ] {
            if on {
                mods |= m;
            }
        }
        Some(Accelerator::new(mods, code(shortcut.key)?))
    }

    fn code(key: Key) -> Option<Code> {
        use Code as C;
        use Key::*;
        Some(match key {
            A => C::KeyA,
            B => C::KeyB,
            C => C::KeyC,
            D => C::KeyD,
            E => C::KeyE,
            F => C::KeyF,
            G => C::KeyG,
            H => C::KeyH,
            I => C::KeyI,
            J => C::KeyJ,
            K => C::KeyK,
            L => C::KeyL,
            M => C::KeyM,
            N => C::KeyN,
            O => C::KeyO,
            P => C::KeyP,
            Q => C::KeyQ,
            R => C::KeyR,
            S => C::KeyS,
            T => C::KeyT,
            U => C::KeyU,
            V => C::KeyV,
            W => C::KeyW,
            X => C::KeyX,
            Y => C::KeyY,
            Z => C::KeyZ,
            Digit0 => C::Digit0,
            Digit1 => C::Digit1,
            Digit2 => C::Digit2,
            Digit3 => C::Digit3,
            Digit4 => C::Digit4,
            Digit5 => C::Digit5,
            Digit6 => C::Digit6,
            Digit7 => C::Digit7,
            Digit8 => C::Digit8,
            Digit9 => C::Digit9,
            Escape => C::Escape,
            Space => C::Space,
            Enter => C::Enter,
            Tab => C::Tab,
            Backspace => C::Backspace,
            Delete => C::Delete,
            Insert => C::Insert,
            Home => C::Home,
            End => C::End,
            PageUp => C::PageUp,
            PageDown => C::PageDown,
            Left => C::ArrowLeft,
            Right => C::ArrowRight,
            Up => C::ArrowUp,
            Down => C::ArrowDown,
            F1 => C::F1,
            F2 => C::F2,
            F3 => C::F3,
            F4 => C::F4,
            F5 => C::F5,
            F6 => C::F6,
            F7 => C::F7,
            F8 => C::F8,
            F9 => C::F9,
            F10 => C::F10,
            F11 => C::F11,
            F12 => C::F12,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(command: bool, control: bool, shift: bool, alt: bool, key: Key) -> Shortcut {
        Shortcut {
            command,
            control,
            shift,
            alt,
            key,
        }
    }

    #[test]
    fn a_shortcut_reads_as_either_platform_writes_it() {
        let parse = |t| Shortcut::parse(t);
        assert_eq!(parse("⌘Z"), Some(keys(true, false, false, false, Key::Z)));
        assert_eq!(parse("⇧⌘Z"), Some(keys(true, false, true, false, Key::Z)));
        assert_eq!(parse("⌥⌘P"), Some(keys(true, false, false, true, Key::P)));
        assert_eq!(
            parse("⇧Space"),
            Some(keys(false, false, true, false, Key::Space))
        );
        assert_eq!(
            parse("Ctrl+Z"),
            Some(keys(false, true, false, false, Key::Z))
        );
        assert_eq!(
            parse("Ctrl+Shift+P"),
            Some(keys(false, true, true, false, Key::P))
        );
        assert_eq!(
            parse("Ctrl+Alt+P"),
            Some(keys(false, true, false, true, Key::P))
        );
        assert_eq!(parse("F2"), Some(keys(false, false, false, false, Key::F2)));
        assert_eq!(
            parse("F12"),
            Some(keys(false, false, false, false, Key::F12))
        );
        assert_eq!(
            parse("Delete"),
            Some(keys(false, false, false, false, Key::Delete))
        );
        assert_eq!(
            parse("Esc"),
            Some(keys(false, false, false, false, Key::Escape))
        );
        assert_eq!(
            parse("End"),
            Some(keys(false, false, false, false, Key::End))
        );
        assert_eq!(parse("f"), Some(keys(false, false, false, false, Key::F)));
        assert_eq!(
            parse("⌘1"),
            Some(keys(true, false, false, false, Key::Digit1))
        );
        assert_eq!(parse("F13"), None);
        assert_eq!(parse("Hyper+Z"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn a_key_that_types_is_the_fields_unless_command_or_ctrl_holds() {
        let native = |t| Shortcut::parse(t).unwrap().native();
        assert!(native("⌘C"));
        assert!(native("Ctrl+C"));
        assert!(native("F2"));
        assert!(native("Delete"));
        assert!(native("Esc"));
        assert!(!native("F"), "F frames, and types an f");
        assert!(!native("⇧Space"));
        assert!(!native("⇧H"));
    }

    /// Every menu's key reads, so none is lost on the way to the bar.
    #[test]
    fn every_key_in_the_menus_reads() {
        for (title, items) in crate::menu::menu_bar() {
            for item in items {
                if let Some(key) = item.shortcut {
                    assert!(
                        Shortcut::parse(key).is_some(),
                        "{title} › {}: {key:?}",
                        item.label
                    );
                }
            }
        }
    }

    #[test]
    fn the_bar_is_the_app_menu_then_the_toolbars_menus() {
        let bar = crate::menu::menu_bar();
        let tree = tree(bar.clone());
        assert_eq!(tree[0].title, APP);
        assert_eq!(tree[0].entries.last(), Some(&Entry::Quit));
        assert!(tree[0].entries.contains(&Entry::Standard(Standard::Hide)));
        let titles: Vec<&str> = tree[1..].iter().map(|m| m.title.as_str()).collect();
        let expected: Vec<&str> = bar.iter().map(|(t, _)| *t).collect();
        assert_eq!(titles, expected);
        for (menu, (_, items)) in tree[1..].iter().zip(&bar) {
            assert_eq!(menu.entries.len(), items.len(), "{}", menu.title);
            for (entry, item) in menu.entries.iter().zip(items) {
                match (entry, &item.action) {
                    (Entry::Separator, None) => {}
                    (Entry::Item { label, action, .. }, Some(a)) => {
                        assert_eq!(label, &item.label);
                        assert_eq!(action, a);
                    }
                    other => panic!("{}: {other:?}", menu.title),
                }
            }
        }
        // Ids are unique: the system names a line back by one.
        let mut ids: Vec<&str> = tree
            .iter()
            .flat_map(|m| &m.entries)
            .filter_map(|e| match e {
                Entry::Item { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    #[test]
    fn keys_and_ticks_carry_over() {
        let tree = tree(crate::menu::menu_bar());
        let find = |menu: &str, name: &str| {
            tree.iter()
                .find(|m| m.title == menu)
                .and_then(|m| {
                    m.entries.iter().find_map(|e| match e {
                        Entry::Item {
                            label,
                            shortcut,
                            check,
                            ..
                        } if label == name => Some((*shortcut, *check)),
                        _ => None,
                    })
                })
                .unwrap_or_else(|| panic!("no {menu} › {name}"))
        };
        let undo = find("Edit", "Undo").0.expect("Undo has its key");
        assert_eq!(undo.key, Key::Z);
        assert!(undo.command || undo.control);
        assert!(!undo.shift);
        let redo = find("Edit", "Redo").0.expect("Redo has its key");
        assert!(redo.command && redo.shift || redo.control && redo.key == Key::Y);
        assert_eq!(find("Edit", "Rename").0.map(|s| s.key), Some(Key::F2));
        assert_eq!(
            find("Edit", "Frame Selected").0,
            None,
            "F would take an f from a field"
        );
        assert!(find("View", "Grid").1, "a toggle shows its tick");
        assert!(!find("File", "New Scene").1);
        assert!(find("Window", "Maximize the View").1);
    }
}
