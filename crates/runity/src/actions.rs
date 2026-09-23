//! What the player does, by name: `"jump"`, not `Space`.
//!
//! Unity's Input System actions, as a text file the project keeps beside
//! `runity.ron`:
//!
//! ```text
//! // input.ron
//! (
//!     actions: {
//!         "jump": [Key(Space)],
//!         "fire": [Mouse(Left), Key(LeftControl)],
//!     },
//!     axes: {
//!         "walk": (negative: [Key(S), Key(Down)], positive: [Key(W), Key(Up)]),
//!     },
//! )
//! ```
//!
//! The game asks for `"jump"`; which keys that is lives in the file, so a
//! rebinding is a one-line diff, and — polled like scenes are — takes
//! effect while the game runs. A file that does not parse is reported and
//! the bindings stay as they were.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::input::{Input, Key, MouseButton};

/// One thing that can trigger an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Binding {
    Key(Key),
    Mouse(MouseButton),
}

/// A value from −1 to 1 made of two sets of bindings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Axis {
    #[serde(default)]
    pub negative: Vec<Binding>,
    #[serde(default)]
    pub positive: Vec<Binding>,
}

/// What `input.ron` holds.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ActionMap {
    #[serde(default)]
    pub actions: BTreeMap<String, Vec<Binding>>,
    #[serde(default)]
    pub axes: BTreeMap<String, Axis>,
}

/// The project's actions, and where they came from.
#[derive(Debug, Clone, Default)]
pub struct Actions {
    pub map: ActionMap,
    path: Option<PathBuf>,
    stamp: Option<SystemTime>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Actions {
    pub fn new(map: ActionMap) -> Self {
        Self {
            map,
            path: None,
            stamp: None,
        }
    }

    /// Read an `input.ron`.
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let text = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let map = ron::from_str(&text).map_err(|e| anyhow::anyhow!("{}:{e}", path.display()))?;
        Ok(Self {
            map,
            stamp: modified(&path),
            path: Some(path),
        })
    }

    /// Read the file again if it changed. `None` when it did not; a file
    /// that does not parse comes back as the error, with the old bindings
    /// kept.
    pub fn reload_if_changed(&mut self) -> Option<Result<(), String>> {
        let path = self.path.clone()?;
        let now = modified(&path);
        if now == self.stamp {
            return None;
        }
        self.stamp = now;
        Some(match Self::load(&path) {
            Ok(fresh) => {
                self.map = fresh.map;
                Ok(())
            }
            Err(e) => Err(format!("{e:#}")),
        })
    }

    fn bindings(&self, name: &str) -> &[Binding] {
        self.map.actions.get(name).map_or(&[], Vec::as_slice)
    }

    pub fn held(&self, input: &Input, name: &str) -> bool {
        self.bindings(name).iter().any(|b| held(input, *b))
    }

    /// Pressed this frame by any of its bindings.
    pub fn pressed(&self, input: &Input, name: &str) -> bool {
        self.bindings(name).iter().any(|b| match *b {
            Binding::Key(key) => input.pressed(key),
            Binding::Mouse(button) => input.mouse_pressed(button),
        })
    }

    pub fn released(&self, input: &Input, name: &str) -> bool {
        self.bindings(name).iter().any(|b| match *b {
            Binding::Key(key) => input.released(key),
            Binding::Mouse(button) => input.mouse_released(button),
        })
    }

    /// −1, 0 or 1 — or 0 when both sides are held.
    pub fn axis(&self, input: &Input, name: &str) -> f32 {
        let Some(axis) = self.map.axes.get(name) else {
            return 0.0;
        };
        let side = |bindings: &[Binding]| f32::from(bindings.iter().any(|b| held(input, *b)));
        side(&axis.positive) - side(&axis.negative)
    }

    /// The names the game asks for that the file does not define, each with
    /// the closest one it does: call it once at start, and a typo is a
    /// sentence rather than a key that silently does nothing.
    pub fn missing(&self, names: &[&str]) -> Vec<String> {
        let known: Vec<&str> = self
            .map
            .actions
            .keys()
            .chain(self.map.axes.keys())
            .map(String::as_str)
            .collect();
        names
            .iter()
            .filter(|name| !known.contains(name))
            .map(|name| {
                let hint = crate::spelling::closest(name, known.iter().copied())
                    .map(|n| format!(" — did you mean `{n}`?"))
                    .unwrap_or_default();
                let file = self
                    .path
                    .as_ref()
                    .map_or("input.ron".to_string(), |p| p.display().to_string());
                format!("{file}: no action or axis `{name}`{hint}")
            })
            .collect()
    }
}

fn held(input: &Input, binding: Binding) -> bool {
    match binding {
        Binding::Key(key) => input.held(key),
        Binding::Mouse(button) => input.mouse_held(button),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::InputEvent;

    const FILE: &str = r#"(
        actions: { "jump": [Key(Space)], "fire": [Mouse(Left), Key(LeftControl)] },
        axes: { "walk": (negative: [Key(S)], positive: [Key(W), Key(Up)]) },
    )"#;

    fn actions() -> Actions {
        Actions::new(ron::from_str(FILE).unwrap())
    }

    #[test]
    fn an_action_answers_to_any_of_its_bindings() {
        let actions = actions();
        let mut input = Input::new();
        input.handle(&InputEvent::MouseDown(MouseButton::Left));
        assert!(actions.pressed(&input, "fire") && actions.held(&input, "fire"));
        assert!(!actions.held(&input, "jump"));
        input.begin_frame();
        input.handle(&InputEvent::KeyDown(Key::Up));
        assert_eq!(actions.axis(&input, "walk"), 1.0);
        input.handle(&InputEvent::KeyDown(Key::S));
        assert_eq!(actions.axis(&input, "walk"), 0.0, "both sides: still");
    }

    #[test]
    fn a_name_the_file_lacks_is_said_with_the_closest_it_has() {
        let missing = actions().missing(&["jump", "jmup", "walk"]);
        assert_eq!(missing.len(), 1);
        assert!(missing[0].contains("did you mean `jump`?"), "{missing:?}");
    }

    #[test]
    fn a_rebinding_on_disk_takes_effect_and_a_broken_one_keeps_the_old() {
        let dir = std::env::temp_dir().join("runity-actions");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("input.ron");
        std::fs::write(&path, FILE).unwrap();
        let mut actions = Actions::load(&path).unwrap();
        assert!(actions.reload_if_changed().is_none());

        let later = |path: &Path, text: &str, secs: u64| {
            std::fs::write(path, text).unwrap();
            let t = SystemTime::now() + std::time::Duration::from_secs(secs);
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(t)
                .unwrap();
        };
        later(&path, &FILE.replace("Key(Space)", "Key(J)"), 2);
        assert_eq!(actions.reload_if_changed(), Some(Ok(())));
        let mut input = Input::new();
        input.handle(&InputEvent::KeyDown(Key::J));
        assert!(actions.held(&input, "jump"));

        later(&path, "(actions: {", 4);
        assert!(matches!(actions.reload_if_changed(), Some(Err(_))));
        assert!(actions.held(&input, "jump"), "the old bindings stay");
    }
}
