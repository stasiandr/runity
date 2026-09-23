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
//!         "walk": (negative: [Key(S), Key(Down)], positive: [Key(W), Key(Up)], analog: [LeftY]),
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

use crate::input::{Input, Key, MouseButton, PadAxis, PadButton};

/// One thing that can trigger an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Binding {
    Key(Key),
    Mouse(MouseButton),
    Pad(PadButton),
}

/// A value from −1 to 1 made of two sets of bindings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Axis {
    #[serde(default)]
    pub negative: Vec<Binding>,
    #[serde(default)]
    pub positive: Vec<Binding>,
    /// Sticks and triggers that drive it directly, −1..1. When a key or
    /// button of the axis is held it wins; otherwise the largest of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub analog: Vec<PadAxis>,
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
    /// A player's own bindings, laid over the project's after every reload.
    player: Option<PathBuf>,
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
            player: None,
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
            player: None,
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
                match self.player.clone() {
                    Some(player) => self.lay_over(&player),
                    None => Ok(()),
                }
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
            Binding::Pad(button) => input.pad_pressed(button),
        })
    }

    pub fn released(&self, input: &Input, name: &str) -> bool {
        self.bindings(name).iter().any(|b| match *b {
            Binding::Key(key) => input.released(key),
            Binding::Mouse(button) => input.mouse_released(button),
            Binding::Pad(button) => input.pad_released(button),
        })
    }

    /// −1..1: −1, 0 or 1 from keys and buttons — 0 when both sides are
    /// held — or, when none is, from its sticks.
    pub fn axis(&self, input: &Input, name: &str) -> f32 {
        let Some(axis) = self.map.axes.get(name) else {
            return 0.0;
        };
        let side = |bindings: &[Binding]| f32::from(bindings.iter().any(|b| held(input, *b)));
        let digital = side(&axis.positive) - side(&axis.negative);
        let pressed = axis
            .positive
            .iter()
            .chain(&axis.negative)
            .any(|b| held(input, *b));
        if pressed {
            return digital;
        }
        axis.analog
            .iter()
            .map(|a| input.pad_axis(*a))
            .fold(
                0.0,
                |best: f32, v| if v.abs() > best.abs() { v } else { best },
            )
    }

    /// Whatever the player pressed this frame, as a binding: "press the
    /// key for Jump". Escape is not offered — it is how a player backs out
    /// of choosing — and nothing is, on a frame with more than one press.
    pub fn listen(input: &Input) -> Option<Binding> {
        let mut found = input
            .pressed_keys()
            .filter(|k| *k != Key::Escape)
            .map(Binding::Key)
            .chain(input.pressed_buttons().map(Binding::Mouse))
            .chain(input.pressed_pad_buttons().map(Binding::Pad));
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// Put `binding` on `action` in place of the one at `slot` (or add it
    /// when there are fewer), and take it off any other action that had it
    /// — one key, one action. Returns the actions it was taken from.
    pub fn rebind(&mut self, action: &str, slot: usize, binding: Binding) -> Vec<String> {
        let mut taken = Vec::new();
        for (name, bindings) in self.map.actions.iter_mut() {
            if name != action && bindings.contains(&binding) {
                bindings.retain(|b| *b != binding);
                taken.push(name.clone());
            }
        }
        let bindings = self.map.actions.entry(action.to_string()).or_default();
        bindings.retain(|b| *b != binding);
        if slot < bindings.len() {
            bindings[slot] = binding;
        } else {
            bindings.push(binding);
        }
        taken
    }

    /// Write the bindings as a player's own file — beside their saves, not
    /// the project's `input.ron`, which stays the defaults.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        let text = ron::ser::to_string_pretty(&self.map, ron::ser::PrettyConfig::new())
            .map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The project's bindings with a player's file over them: every action
    /// the player's file names is theirs, the rest — new ones a patch
    /// added included — the project's. No player file is no change.
    pub fn with_player(mut self, path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        self.lay_over(&path)?;
        self.player = Some(path);
        Ok(self)
    }

    fn lay_over(&mut self, path: &Path) -> Result<(), String> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(());
        };
        let theirs: ActionMap =
            ron::from_str(&text).map_err(|e| format!("{}:{e}", path.display()))?;
        self.map.actions.extend(theirs.actions);
        self.map.axes.extend(theirs.axes);
        Ok(())
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
        Binding::Pad(button) => input.pad_held(button),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_rebinds_a_key_and_it_is_theirs_over_the_project() {
        use crate::input::InputEvent;
        let dir = std::env::temp_dir().join("runity-rebind");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("input.ron");
        std::fs::write(
            &project,
            r#"(actions: { "jump": [Key(Space)], "fire": [Key(F)], "quit": [Key(Escape)] })"#,
        )
        .unwrap();
        let mut actions = Actions::load(&project).unwrap();

        // "Press the key for Jump": F, which Fire had.
        let mut input = Input::new();
        input.handle(&InputEvent::KeyDown(Key::F));
        let pressed = Actions::listen(&input).unwrap();
        assert_eq!(pressed, Binding::Key(Key::F));
        assert_eq!(actions.rebind("jump", 0, pressed), ["fire"]);
        assert_eq!(actions.map.actions["jump"], [Binding::Key(Key::F)]);
        assert!(actions.map.actions["fire"].is_empty());
        input.begin_frame();
        input.handle(&InputEvent::KeyDown(Key::Escape));
        assert_eq!(Actions::listen(&input), None, "Escape backs out");

        // Saved as the player's, laid over the project's next time.
        let player = dir.join("player/bindings.ron");
        actions.save(&player).unwrap();
        let again = Actions::load(&project)
            .unwrap()
            .with_player(&player)
            .unwrap();
        assert_eq!(again.map.actions["jump"], [Binding::Key(Key::F)]);
        assert_eq!(again.map.actions["quit"], [Binding::Key(Key::Escape)]);
        assert!(Actions::load(&project)
            .unwrap()
            .with_player(dir.join("none.ron"))
            .is_ok());
    }
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

    #[test]
    fn a_pad_binds_like_a_key_and_a_stick_drives_an_axis_past_its_dead_zone() {
        use crate::input::{PadAxis, PadButton};
        let actions = Actions::new(
            ron::from_str(
                r#"(
                actions: { "jump": [Key(Space), Pad(South)] },
                axes: { "walk": (negative: [Key(S)], positive: [Key(W)], analog: [LeftY]) },
            )"#,
            )
            .unwrap(),
        );
        let mut input = Input::new();
        input.handle(&InputEvent::PadDown(PadButton::South));
        assert!(actions.pressed(&input, "jump"));
        input.begin_frame();
        assert!(actions.held(&input, "jump") && !actions.pressed(&input, "jump"));

        let stick = |input: &mut Input, value: f32| {
            input.handle(&InputEvent::PadMoved {
                axis: PadAxis::LeftY,
                value,
            })
        };
        stick(&mut input, 0.1);
        assert_eq!(
            actions.axis(&input, "walk"),
            0.0,
            "resting off centre is resting"
        );
        stick(&mut input, 1.0);
        assert_eq!(actions.axis(&input, "walk"), 1.0);
        stick(&mut input, -0.575);
        assert!(
            (actions.axis(&input, "walk") + 0.5).abs() < 1e-4,
            "stretched past the zone"
        );
        input.handle(&InputEvent::KeyDown(Key::W));
        assert_eq!(actions.axis(&input, "walk"), 1.0, "a held key wins");
        input.handle(&InputEvent::FocusLost);
        input.handle(&InputEvent::KeyUp(Key::W));
        assert_eq!(
            actions.axis(&input, "walk"),
            0.0,
            "focus lost lets go of the pad too"
        );
    }
}
