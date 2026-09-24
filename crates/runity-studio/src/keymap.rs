//! Keys: which key does what, one table the whole editor answers from.
//!
//! **One keymap.** Every shortcut the editor has — the registry's
//! (`runity_editor::actions`: Save, Undo, Delete…), the menus' (Copy,
//! Play, Search…) and the Scene view's (Q W E R, F, H, Shift Space) — is a
//! [`Command`] here, with its default keys. The studio answers a key by
//! looking it up here and running the command's [`Action`], the same
//! action its menu line runs; the in-window menus and macOS's menu bar
//! show the key from here. So a key rebound in Preferences › Keys works in
//! the window, reads in the menus and is the menu bar's key equivalent,
//! and nothing else holds a copy of it that could disagree.
//!
//! Keys a panel answers only while it has the keyboard — the arrows in the
//! Hierarchy, Enter in a field, Delete on a Project tile — stay the
//! panel's: they are what the panel's own list means, not the editor's.
//!
//! **Whose.** The person's, in every project, as the colours are:
//! `keys.ron` in their settings folder (`appearance::config_dir`), holding
//! only what differs from the defaults, by the command's name —
//!
//! ```ron
//! {
//!     "move": ["M"],
//!     "frame": [],
//! }
//! ```
//!
//! — so a default changed in a later editor reaches everyone who did not
//! change that one. An empty list is a command with no key.
//!
//! **Postulates.** 7 (one opinionated default) bends here as it does for
//! the colours, and on purpose: the defaults are Unity's, so hands that
//! know Unity work on day one, and a rebinding is the editor's chrome —
//! no project, game or teammate sees it. 5 (AI): a registry command's
//! name here is the agent's tool of the same name (`undo`, `save_scene`).
//!
//! **Conflicts.** A key does one thing. Given to a second command, it is
//! taken from the first ([`Keymap::set`] says whose it was); Reset on the
//! first gives it back its default.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use runity::gizmo::Tool;

use crate::menu::{Action, MenuItem};
use crate::native_menu::Shortcut;

/// The file, in the settings folder.
pub const FILE: &str = "keys.ron";

/// One thing a key can do.
#[derive(Debug, Clone)]
pub struct Command {
    /// Its name in `keys.ron`: `move`, `undo`.
    pub id: &'static str,
    pub label: &'static str,
    /// Where it is listed: the menu it is in, or `Tools`.
    pub group: &'static str,
    pub action: Action,
    /// Its keys out of the box, as this platform writes them.
    pub defaults: Vec<Shortcut>,
}

/// A command's keys as the Mac and the other systems write them.
type Keys = &'static [(&'static str, &'static str)];

/// The shortcut key, as the platform writes it.
fn keys(keys: &[(&str, &str)]) -> Vec<Shortcut> {
    let mut out = Vec::new();
    for (mac, other) in keys {
        let key = Shortcut::parse(if cfg!(target_os = "macos") {
            mac
        } else {
            other
        });
        if let Some(key) = key.filter(|k| !out.contains(k)) {
            out.push(key);
        }
    }
    out
}

/// Every command, in the order Preferences › Keys lists them.
pub fn commands() -> Vec<Command> {
    let mut out = Vec::new();
    // The registry's, with the keys it gives them: what the agent's tool
    // of the same name is called.
    for a in runity_editor::actions::registry() {
        let mut defaults: Vec<(&'static str, &'static str)> = a.shortcut.into_iter().collect();
        // A Mac's Delete key is Backspace: ⌘⌫ deletes, as in Finder.
        if a.name == "delete_entity" {
            defaults.push(("⌘⌫", "Ctrl+Backspace"));
        }
        // Ctrl+Shift+Z redoes too where Ctrl+Y is the rule.
        if a.name == "redo" {
            defaults.push(("⇧⌘Z", "Ctrl+Shift+Z"));
        }
        out.push(Command {
            id: a.name,
            label: a.label,
            group: a.menu,
            action: Action::Editor(a.name),
            defaults: keys(&defaults),
        });
    }
    let mut add = |id, label, group, action, k: Keys| {
        out.push(Command {
            id,
            label,
            group,
            action,
            defaults: keys(k),
        })
    };
    add("copy", "Copy", "Edit", Action::Copy, &[("⌘C", "Ctrl+C")]);
    add("paste", "Paste", "Edit", Action::Paste, &[("⌘V", "Ctrl+V")]);
    add("rename", "Rename", "Edit", Action::Rename, &[("F2", "F2")]);
    add(
        "select_all",
        "Select All",
        "Edit",
        Action::SelectAll,
        &[("⌘A", "Ctrl+A")],
    );
    add(
        "select_none",
        "Select None",
        "Edit",
        Action::SelectNone,
        &[("Esc", "Esc")],
    );
    add(
        "frame",
        "Frame Selected",
        "Edit",
        Action::Frame,
        &[("F", "F")],
    );
    add(
        "move_to_view",
        "Move to View",
        "Edit",
        Action::MoveToView,
        &[("⌥⌘F", "Ctrl+Alt+F")],
    );
    add(
        "align_with_view",
        "Align with View",
        "Edit",
        Action::AlignWithView,
        &[("⇧⌘F", "Ctrl+Shift+F")],
    );
    add(
        "project_settings",
        "Project Settings…",
        "Edit",
        Action::ProjectSettings,
        &[],
    );
    add(
        "preferences",
        "Preferences…",
        "Edit",
        Action::Preferences(None),
        &[("⌘,", "Ctrl+,")],
    );
    add(
        "create_empty",
        "Create Empty",
        "Entity",
        Action::CreateEmpty,
        &[("⇧⌘N", "Ctrl+Shift+N")],
    );
    add(
        "group",
        "Group Selection",
        "Entity",
        Action::Group,
        &[("⇧⌘G", "Ctrl+Shift+G")],
    );
    add(
        "search",
        "Search…",
        "View",
        Action::Search,
        &[("⌘K", "Ctrl+K")],
    );
    add(
        "hide",
        "Hide Selection",
        "View",
        Action::Hide,
        &[("H", "H")],
    );
    add("hand", "Hand", "Tools", Action::Hand, &[("Q", "Q")]);
    add(
        "move",
        "Move",
        "Tools",
        Action::Tool(Tool::Move),
        &[("W", "W")],
    );
    add(
        "rotate",
        "Rotate",
        "Tools",
        Action::Tool(Tool::Rotate),
        &[("E", "E")],
    );
    add(
        "scale",
        "Scale",
        "Tools",
        Action::Tool(Tool::Scale),
        &[("R", "R")],
    );
    add(
        "rect",
        "Rect",
        "Tools",
        Action::Tool(Tool::Rect),
        &[("T", "T")],
    );
    add(
        "transform",
        "Transform",
        "Tools",
        Action::Tool(Tool::Transform),
        &[("Y", "Y")],
    );
    add(
        "space",
        "Global / Local",
        "Tools",
        Action::ToggleSpace,
        &[("X", "X")],
    );
    add(
        "pivot",
        "Pivot / Center",
        "Tools",
        Action::TogglePivot,
        &[("Z", "Z")],
    );
    add(
        "maximize",
        "Maximize What Is Under the Pointer",
        "Window",
        Action::Maximize,
        &[("⇧Space", "Shift+Space")],
    );
    add(
        "play",
        "Play / Stop",
        "Play",
        Action::Play,
        &[("⌘P", "Ctrl+P")],
    );
    add(
        "pause",
        "Pause",
        "Play",
        Action::Pause,
        &[("⇧⌘P", "Ctrl+Shift+P")],
    );
    add(
        "step",
        "Step",
        "Play",
        Action::Step,
        &[("⌥⌘P", "Ctrl+Alt+P")],
    );
    add(
        "keep_simulation",
        "Keep Simulation Changes",
        "Play",
        Action::KeepSimulation,
        &[("K", "K")],
    );
    out
}

/// Which keys every command has now.
#[derive(Debug, Clone)]
pub struct Keymap {
    commands: Vec<Command>,
    keys: Vec<Vec<Shortcut>>,
    /// Changes whenever a key does: the menu bar checks it.
    pub generation: u64,
}

impl Default for Keymap {
    fn default() -> Self {
        let commands = commands();
        let keys = commands.iter().map(|c| c.defaults.clone()).collect();
        Self {
            commands,
            keys,
            generation: 0,
        }
    }
}

impl Keymap {
    /// The defaults with the person's `keys.ron` over them. What does not
    /// read is said, and the rest stands.
    pub fn load(dir: Option<&Path>) -> (Self, Vec<String>) {
        let mut this = Self::default();
        let mut errors = Vec::new();
        let Some(file) = dir.map(|d| d.join(FILE)) else {
            return (this, errors);
        };
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (this, errors),
            Err(e) => {
                errors.push(format!("{}: {e}", file.display()));
                return (this, errors);
            }
        };
        let map: BTreeMap<String, Vec<String>> = match runity::ron::from_str(&text) {
            Ok(map) => map,
            Err(e) => {
                errors.push(format!("{}: {e}", file.display()));
                return (this, errors);
            }
        };
        for (id, written) in map {
            let Some(i) = this.index(&id) else {
                errors.push(format!("{}: no command {id:?}", file.display()));
                continue;
            };
            let mut keys = Vec::new();
            for w in written {
                match Shortcut::parse(&w) {
                    Some(k) => keys.push(k),
                    None => errors.push(format!("{}: {id}: {w:?} is not a key", file.display())),
                }
            }
            this.keys[i] = keys;
        }
        (this, errors)
    }

    /// Write what differs from the defaults to `dir/keys.ron`.
    pub fn save(&self, dir: &Path) -> Result<PathBuf, String> {
        let mut out = String::from(
            "// Your keys, in every project: Preferences › Keys. Only what differs from the defaults.\n{\n",
        );
        for (c, keys) in self.commands.iter().zip(&self.keys) {
            if *keys != c.defaults {
                let written: Vec<String> =
                    keys.iter().map(|k| format!("{:?}", k.label())).collect();
                out.push_str(&format!("    {:?}: [{}],\n", c.id, written.join(", ")));
            }
        }
        out.push_str("}\n");
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let file = dir.join(FILE);
        std::fs::write(&file, out).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(file)
    }

    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.commands.iter().position(|c| c.id == id)
    }

    /// A command's keys now.
    pub fn keys(&self, id: &str) -> &[Shortcut] {
        self.index(id).map_or(&[], |i| &self.keys[i])
    }

    /// Whether a command has its default keys.
    pub fn is_default(&self, id: &str) -> bool {
        self.index(id)
            .is_none_or(|i| self.keys[i] == self.commands[i].defaults)
    }

    /// What a key does, if anything.
    pub fn action(&self, key: Shortcut) -> Option<&Action> {
        self.keys
            .iter()
            .position(|ks| ks.contains(&key))
            .map(|i| &self.commands[i].action)
    }

    /// The command a key belongs to.
    pub fn owner(&self, key: Shortcut) -> Option<&Command> {
        self.keys
            .iter()
            .position(|ks| ks.contains(&key))
            .map(|i| &self.commands[i])
    }

    /// Give a command this key, and only this one. A command that had it
    /// loses it: its label is returned, to say so.
    pub fn set(&mut self, id: &str, key: Shortcut) -> Option<&'static str> {
        let i = self.index(id)?;
        let mut taken = None;
        for (j, keys) in self.keys.iter_mut().enumerate() {
            if j != i && keys.contains(&key) {
                keys.retain(|k| *k != key);
                taken = Some(self.commands[j].label);
            }
        }
        self.keys[i] = vec![key];
        self.generation += 1;
        taken
    }

    /// A command with no key.
    pub fn clear(&mut self, id: &str) {
        if let Some(i) = self.index(id) {
            self.keys[i].clear();
            self.generation += 1;
        }
    }

    /// A command's default keys back; one who has one of them since loses
    /// it, so a key still does one thing.
    pub fn reset(&mut self, id: &str) {
        let Some(i) = self.index(id) else { return };
        let defaults = self.commands[i].defaults.clone();
        for (j, keys) in self.keys.iter_mut().enumerate() {
            if j != i {
                keys.retain(|k| !defaults.contains(k));
            }
        }
        self.keys[i] = defaults;
        self.generation += 1;
    }

    /// Every command's default keys.
    pub fn reset_all(&mut self) {
        for (keys, c) in self.keys.iter_mut().zip(&self.commands) {
            *keys = c.defaults.clone();
        }
        self.generation += 1;
    }

    /// The key a menu line for `action` shows: `Some(None)` for a command
    /// with no key, `None` for an action that is not a command here.
    pub fn shortcut_for(&self, action: &Action) -> Option<Option<Shortcut>> {
        let i = self.commands.iter().position(|c| c.action == *action)?;
        Some(self.keys[i].first().copied())
    }

    /// Menu lines with the keys from here.
    pub fn label(&self, items: &mut [MenuItem]) {
        for item in items {
            if let Some(key) = item.action.as_ref().and_then(|a| self.shortcut_for(a)) {
                item.shortcut = key.map(|k| k.label());
            }
        }
    }

    /// The menu bar with the keys from here.
    pub fn label_bar(
        &self,
        mut bar: Vec<(&'static str, Vec<MenuItem>)>,
    ) -> Vec<(&'static str, Vec<MenuItem>)> {
        for (_, items) in &mut bar {
            self.label(items);
        }
        bar
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity::input::Key;

    fn plain(key: Key) -> Shortcut {
        Shortcut::held(key, (false, false, false, false))
    }

    #[test]
    fn every_command_has_one_name_and_every_default_key_one_command() {
        let map = Keymap::default();
        let all = map.commands();
        for (i, c) in all.iter().enumerate() {
            assert!(all[i + 1..].iter().all(|d| d.id != c.id), "{} twice", c.id);
            for k in &c.defaults {
                assert_eq!(map.owner(*k).map(|o| o.id), Some(c.id), "{}", k.label());
            }
        }
        assert_eq!(map.action(plain(Key::W)), Some(&Action::Tool(Tool::Move)));
    }

    #[test]
    fn a_key_taken_is_lost_by_its_owner_and_reset_gives_it_back() {
        let mut map = Keymap::default();
        assert_eq!(map.set("move", plain(Key::F)), Some("Frame Selected"));
        assert_eq!(map.keys("frame"), &[]);
        assert_eq!(map.action(plain(Key::F)), Some(&Action::Tool(Tool::Move)));
        assert_eq!(map.action(plain(Key::W)), None);
        map.reset("frame");
        assert_eq!(map.action(plain(Key::F)), Some(&Action::Frame));
        assert_eq!(map.keys("move"), &[], "move lost F back");
        map.reset_all();
        assert!(map.commands().iter().all(|c| map.is_default(c.id)));
    }

    #[test]
    fn only_what_differs_is_written_and_it_reads_back() {
        let dir = std::env::temp_dir().join(format!("runity-keymap-{}", std::process::id()));
        let mut map = Keymap::default();
        map.set("move", plain(Key::M));
        map.clear("hide");
        let file = map.save(&dir).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"move\": [\"M\"]"), "{text}");
        assert!(text.contains("\"hide\": []"), "{text}");
        assert!(!text.contains("rotate"), "{text}");
        let (back, errors) = Keymap::load(Some(&dir));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(back.keys("move"), &[plain(Key::M)]);
        assert_eq!(back.keys("hide"), &[]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
