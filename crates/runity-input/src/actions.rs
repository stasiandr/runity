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
//!
//! The rest of the Input System, when a game wants it (DNA, "Ввод — как
//! Unity Input System"; the file above is its simplest case):
//!
//! ```text
//! (
//!     actions: { "pause": [Key(Escape), Pad(Start)] },  // always on
//!     maps: {                                          // Action Maps
//!         "walking": (actions: { "jump": [Key(Space), Pad(South)] }),
//!         "menu": (actions: { "back": [Key(Backspace), Pad(East)] }),
//!     },
//!     schemes: {                                       // Control Schemes
//!         "keyboard": [Keyboard, Mouse],
//!         "gamepad": [Gamepad],
//!     },
//! )
//! ```
//!
//! A map is switched on and off by the game ([`Actions::disable_map`]: the
//! menu opens, walking stops answering); its actions answer by name, or as
//! `"menu/back"`. A scheme is which devices count: with schemes, the one
//! the player last touched a device of is on ([`Actions::update`]), and a
//! binding of another counts for nothing — Unity's PlayerInput switching
//! between keyboard and pad.

use std::collections::{BTreeMap, BTreeSet};
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

impl Binding {
    /// The device it is on.
    pub fn device(self) -> Device {
        match self {
            Binding::Key(_) => Device::Keyboard,
            Binding::Mouse(_) => Device::Mouse,
            Binding::Pad(_) => Device::Gamepad,
        }
    }
}

/// A kind of device a control scheme is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Device {
    Keyboard,
    Mouse,
    Gamepad,
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

/// What `input.ron` holds: the actions and axes that are always on, the
/// maps a game switches, and the control schemes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ActionMap {
    #[serde(default)]
    pub actions: BTreeMap<String, Vec<Binding>>,
    #[serde(default)]
    pub axes: BTreeMap<String, Axis>,
    /// Unity's Action Maps, by name: actions and axes the game switches on
    /// and off together. One level: a map has no maps of its own.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub maps: BTreeMap<String, ActionMap>,
    /// Unity's Control Schemes, by name: the devices each is made of.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schemes: BTreeMap<String, Vec<Device>>,
}

impl ActionMap {
    /// What does not hold together, in words: a map inside a map.
    pub fn problems(&self) -> Vec<String> {
        self.maps
            .iter()
            .filter(|(_, m)| !m.maps.is_empty())
            .map(|(name, _)| format!("map `{name}` has maps of its own; a map is one level"))
            .collect()
    }
}

/// The project's actions, and where they came from.
#[derive(Debug, Clone, Default)]
pub struct Actions {
    pub map: ActionMap,
    path: Option<PathBuf>,
    stamp: Option<SystemTime>,
    /// A player's own bindings, laid over the project's after every reload.
    player: Option<PathBuf>,
    /// Maps the game switched off.
    disabled: BTreeSet<String>,
    /// The control scheme on: `None` counts every device.
    scheme: Option<String>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    runity_core::files::modified(path)
}

impl Actions {
    pub fn new(map: ActionMap) -> Self {
        Self {
            map,
            ..Self::default()
        }
    }

    /// Read an `input.ron`.
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let text = runity_core::files::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let map = ron::from_str(&text).map_err(|e| anyhow::anyhow!("{}:{e}", path.display()))?;
        Ok(Self {
            map,
            stamp: modified(&path),
            path: Some(path),
            ..Self::default()
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

    /// The maps an action or axis called `name` is looked for in, with
    /// the name there: the always-on one, then every map switched on — or,
    /// for `"menu/back"`, the map `menu` alone.
    fn looked_in(&self, name: &str) -> Vec<(&ActionMap, String)> {
        if let Some((map, action)) = name.split_once('/') {
            return self
                .map
                .maps
                .get(map)
                .filter(|_| self.map_enabled(map))
                .map(|m| (m, action.to_string()))
                .into_iter()
                .collect();
        }
        std::iter::once(&self.map)
            .chain(
                self.map
                    .maps
                    .iter()
                    .filter(|(n, _)| self.map_enabled(n))
                    .map(|(_, m)| m),
            )
            .map(|m| (m, name.to_string()))
            .collect()
    }

    /// Whether the scheme on counts `device`.
    fn counts(&self, device: Device) -> bool {
        match self.scheme.as_ref().and_then(|s| self.map.schemes.get(s)) {
            Some(devices) => devices.contains(&device),
            None => true,
        }
    }

    fn bindings(&self, name: &str) -> Vec<Binding> {
        self.looked_in(name)
            .into_iter()
            .filter_map(|(map, name)| map.actions.get(&name))
            .flatten()
            .copied()
            .filter(|b| self.counts(b.device()))
            .collect()
    }

    /// Switch a map's actions on: they answer again.
    pub fn enable_map(&mut self, map: &str) {
        self.disabled.remove(map);
    }

    /// Switch a map's actions off: the menu opens, and walking stops
    /// answering.
    pub fn disable_map(&mut self, map: &str) {
        self.disabled.insert(map.to_string());
    }

    pub fn map_enabled(&self, map: &str) -> bool {
        !self.disabled.contains(map)
    }

    /// The control scheme on, when the file has schemes.
    pub fn scheme(&self) -> Option<&str> {
        self.scheme.as_deref()
    }

    /// Put a scheme on by name, or `None` for every device.
    pub fn set_scheme(&mut self, scheme: Option<&str>) {
        self.scheme = scheme.map(str::to_string);
    }

    /// Once a frame, before asking: with schemes, the one the player just
    /// touched a device of goes on — a key, and the keyboard's scheme is
    /// on; a pad's button, the pad's. Returns the scheme when it changed.
    pub fn update(&mut self, input: &Input) -> Option<&str> {
        let touched = input
            .pressed_keys()
            .next()
            .map(|_| Device::Keyboard)
            .or_else(|| input.pressed_buttons().next().map(|_| Device::Mouse))
            .or_else(|| input.pressed_pad_buttons().next().map(|_| Device::Gamepad))?;
        if self.scheme.is_some() && self.counts(touched) {
            return None;
        }
        let scheme = self
            .map
            .schemes
            .iter()
            .find(|(_, devices)| devices.contains(&touched))
            .map(|(name, _)| name.clone())?;
        self.scheme = Some(scheme);
        self.scheme.as_deref()
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
        let axes: Vec<Axis> = self
            .looked_in(name)
            .into_iter()
            .filter_map(|(map, name)| map.axes.get(&name).cloned())
            .collect();
        let counted = |bindings: &[Binding]| -> Vec<Binding> {
            bindings
                .iter()
                .copied()
                .filter(|b| self.counts(b.device()))
                .collect()
        };
        let positive: Vec<Binding> = axes.iter().flat_map(|a| counted(&a.positive)).collect();
        let negative: Vec<Binding> = axes.iter().flat_map(|a| counted(&a.negative)).collect();
        let side = |bindings: &[Binding]| f32::from(bindings.iter().any(|b| held(input, *b)));
        let pressed = positive.iter().chain(&negative).any(|b| held(input, *b));
        if pressed {
            return side(&positive) - side(&negative);
        }
        if !self.counts(Device::Gamepad) {
            return 0.0;
        }
        axes.iter()
            .flat_map(|a| &a.analog)
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
        let Ok(text) = runity_core::files::read_to_string(path) else {
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
        let mut known: Vec<String> = self
            .map
            .actions
            .keys()
            .chain(self.map.axes.keys())
            .cloned()
            .collect();
        for (map, inner) in &self.map.maps {
            for name in inner.actions.keys().chain(inner.axes.keys()) {
                known.push(name.clone());
                known.push(format!("{map}/{name}"));
            }
        }
        let known: Vec<&str> = known.iter().map(String::as_str).collect();
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
    fn maps_switch_and_the_scheme_follows_the_device_touched_last() {
        use crate::input::InputEvent;
        let map: ActionMap = ron::from_str(
            r#"(
                actions: { "pause": [Key(Escape), Pad(Start)] },
                maps: {
                    "walking": (actions: { "jump": [Key(Space), Pad(South)] }),
                    "menu": (actions: { "back": [Key(Backspace), Pad(East)] }),
                },
                schemes: { "keyboard": [Keyboard, Mouse], "gamepad": [Gamepad] },
            )"#,
        )
        .unwrap();
        assert!(map.problems().is_empty());
        let mut actions = Actions::new(map);
        let mut input = Input::new();
        input.handle(&InputEvent::KeyDown(Key::Space));
        assert!(actions.pressed(&input, "jump") && actions.pressed(&input, "walking/jump"));
        actions.disable_map("walking");
        assert!(!actions.pressed(&input, "jump"), "the menu is open");
        actions.enable_map("walking");
        assert!(actions.missing(&["menu/back", "back", "pause"]).is_empty());

        // A key: the keyboard's scheme goes on, and the pad counts for nothing.
        assert_eq!(actions.update(&input), Some("keyboard"));
        input.begin_frame();
        input.handle(&InputEvent::KeyUp(Key::Space));
        input.begin_frame();
        input.handle(&InputEvent::PadDown(PadButton::South));
        assert!(
            !actions.held(&input, "jump"),
            "the pad is not in the keyboard's scheme"
        );
        // Until the pad is touched: then it is the pad's.
        assert_eq!(actions.update(&input), Some("gamepad"));
        assert!(actions.held(&input, "jump"));
        assert!(!actions.held(&input, "pause"));
    }

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
