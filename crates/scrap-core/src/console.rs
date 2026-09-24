//! Commands typed into a running game: Unreal's console and CheatManager.
//!
//! A game registers what can be typed — `god`, `give sword`, `wolves 0` —
//! as plain functions of the world and the words after the name, each with
//! a line of help ([`Commands::add`]). The engine's own come with it:
//!
//! * `help` — every command and what it does;
//! * `get world.gravity` — a number from `tuning/`, as the file has it;
//! * `set world.gravity -3` — the same number changed **in the file**, only
//!   that value's text, so the game's [`crate::Tuned`] picks it up at its
//!   next poll, as it would a save from a text editor. The file stays the
//!   one truth (DNA, postulates 1 and 2): a change worth keeping is already
//!   a one-line diff, one not worth keeping is `git checkout`. There is no
//!   `set` that changes only the running game.
//!
//! The words come from the in-game console (`scrap::console::Console`), from
//! the editor's Console over the embed socket
//! ([`crate::embed::ToGame::Command`]) or from an agent through the editor
//! — the same line, the same answer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hecs::World;

/// A command: what it does to the world with the words after its name, and
/// what it answers — or why it could not.
pub type Command = fn(&mut World, &[&str]) -> Result<String, String>;

/// What can be typed, by name.
#[derive(Debug, Clone, Default)]
pub struct Commands {
    by_name: BTreeMap<String, (Command, String)>,
    /// The project's `tuning/`, for `get` and `set`.
    tuning: Option<PathBuf>,
}

/// The engine's own commands: what `help` lists first.
const BUILT_IN: [(&str, &str); 3] = [
    ("help", "every command and what it does"),
    (
        "get",
        "get FILE.FIELD: a number from tuning/, as the file has it (get world.gravity)",
    ),
    (
        "set",
        "set FILE.FIELD VALUE: change it in tuning/FILE.ron; the game reloads it (set world.gravity -3)",
    ),
];

impl Commands {
    /// The built-in commands; `get` and `set` answer that there is no
    /// `tuning/` until [`Commands::with_tuning`] says where it is.
    pub fn new() -> Self {
        Self::default()
    }

    /// Where the tuning files are, for `get` and `set`.
    pub fn with_tuning(mut self, dir: impl Into<PathBuf>) -> Self {
        self.tuning = Some(dir.into());
        self
    }

    /// Say that `name` runs `command`. A name taken before is replaced —
    /// a game's own `help` included.
    pub fn add(&mut self, name: &str, help: &str, command: Command) -> &mut Self {
        self.by_name
            .insert(name.to_string(), (command, help.to_string()));
        self
    }

    /// Every name there is to type, the engine's and the game's, sorted.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = BUILT_IN.iter().map(|(n, _)| *n).collect();
        names.extend(self.by_name.keys().map(String::as_str));
        names.sort_unstable();
        names.dedup();
        names
    }

    /// The names starting with what was typed: what Tab completes to.
    pub fn complete(&self, typed: &str) -> Vec<&str> {
        self.names()
            .into_iter()
            .filter(|n| n.starts_with(typed))
            .collect()
    }

    /// Run a typed line. `Ok` with what it answers, `Err` with why not —
    /// in words either way, for a person or an agent to read.
    pub fn run(&self, world: &mut World, line: &str) -> Result<String, String> {
        let line = line.trim();
        let (name, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        if name.is_empty() {
            return Ok(String::new());
        }
        if let Some((command, _)) = self.by_name.get(name) {
            let words = split(rest);
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            return command(world, &words);
        }
        match name {
            "help" => Ok(self.help()),
            "get" => get(self.tuning()?, rest),
            "set" => {
                let (target, value) = rest
                    .split_once(char::is_whitespace)
                    .ok_or("set FILE.FIELD VALUE, as: set world.gravity -3")?;
                set(self.tuning()?, target, value.trim())
            }
            _ => {
                let near = crate::spelling::closest(name, self.names());
                Err(match near {
                    Some(near) => format!("no command `{name}`; `{near}`? (help lists them)"),
                    None => format!("no command `{name}` (help lists them)"),
                })
            }
        }
    }

    fn tuning(&self) -> Result<&Path, String> {
        self.tuning
            .as_deref()
            .ok_or_else(|| "this game has no tuning/ to get and set from".to_string())
    }

    fn help(&self) -> String {
        let mut lines: Vec<String> = BUILT_IN
            .iter()
            .filter(|(n, _)| !self.by_name.contains_key(*n))
            .map(|(n, h)| format!("{n} — {h}"))
            .collect();
        lines.extend(self.by_name.iter().map(|(n, (_, h))| format!("{n} — {h}")));
        lines.join("\n")
    }
}

/// The words of a line: split at spaces, a `"quoted phrase"` one word
/// without its quotes.
pub fn split(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            c => {
                word.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(word);
    }
    words
}

/// `world.gravity` as the file `tuning/world.ron` and the path of fields
/// in it; `enemies/wolf.speed` is `tuning/enemies/wolf.ron`.
fn target<'a>(tuning: &Path, target: &'a str) -> Result<(PathBuf, Vec<&'a str>), String> {
    let (file, path) = target
        .split_once('.')
        .ok_or_else(|| format!("`{target}` is FILE.FIELD, as world.gravity"))?;
    let fields: Vec<&str> = path.split('.').collect();
    // Inside tuning/, and nowhere else.
    let outside = Path::new(file).is_absolute() || file.split(['/', '\\']).any(|p| p == "..");
    if file.is_empty() || outside || fields.iter().any(|f| f.is_empty()) {
        return Err(format!("`{target}` is FILE.FIELD, as world.gravity"));
    }
    Ok((tuning.join(format!("{file}.ron")), fields))
}

fn read(path: &Path) -> Result<String, String> {
    crate::files::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// What the file says at `target`, as it is written.
pub fn get(tuning: &Path, what: &str) -> Result<String, String> {
    let (path, fields) = target(tuning, what)?;
    let text = read(&path)?;
    let span = crate::ron_edit::value_at(&text, &fields)
        .ok_or_else(|| format!("{} has no `{}`", path.display(), fields.join(".")))?;
    Ok(format!("{what} = {}", &text[span]))
}

/// The kind of a RON value, to refuse a word where a number was.
fn kind(value: &ron::Value) -> &'static str {
    match value {
        ron::Value::Bool(_) => "true or false",
        ron::Value::Char(_) | ron::Value::String(_) => "text",
        ron::Value::Number(_) => "a number",
        ron::Value::Seq(_) => "a list",
        ron::Value::Map(_) => "a map",
        _ => "",
    }
}

/// Change the value at `target` in its tuning file to `value` (RON text):
/// only that value's text changes, comments and all else as they were.
/// The file must still read as RON, and a number stays a number; whether
/// it fits the game's type is the game's [`crate::Tuned`] to say, keeping
/// its last good values if not.
pub fn set(tuning: &Path, what: &str, value: &str) -> Result<String, String> {
    let (path, fields) = target(tuning, what)?;
    let new: ron::Value = ron::from_str(value).map_err(|e| format!("`{value}` is not RON: {e}"))?;
    let text = read(&path)?;
    if let Some(span) = crate::ron_edit::value_at(&text, &fields) {
        let old = &text[span];
        if let Ok(old) = ron::from_str::<ron::Value>(old) {
            let (was, is) = (kind(&old), kind(&new));
            if !was.is_empty() && was != is {
                return Err(format!("{what} is {was}, and `{value}` is not"));
            }
        }
    }
    let out = crate::ron_edit::set_at(&text, &fields, value).ok_or_else(|| {
        format!(
            "{} has no `{}` to set",
            path.display(),
            fields[..fields.len() - 1].join(".")
        )
    })?;
    ron::from_str::<ron::Value>(&out)
        .map_err(|e| format!("{} would not read after it: {e}", path.display()))?;
    let before = crate::files::modified(&path);
    crate::files::write(&path, &out).map_err(|e| format!("{}: {e}", path.display()))?;
    // A disk that keeps whole seconds would hide a second change within
    // one from the game's poll.
    #[cfg(not(target_arch = "wasm32"))]
    if before.is_some() && crate::files::modified(&path) == before {
        if let (Some(t), Ok(file)) = (before, std::fs::File::options().write(true).open(&path)) {
            let _ = file.set_modified(t + std::time::Duration::from_secs(1));
        }
    }
    Ok(format!("{what} = {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning(name: &str, text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scrap-console-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("world.ron"), text).unwrap();
        dir
    }

    fn god(world: &mut World, words: &[&str]) -> Result<String, String> {
        world.spawn((words.len() as u32,));
        match words {
            [] => Ok("god mode".into()),
            [who] => Ok(format!("{who} is a god")),
            _ => Err("god [WHO]".into()),
        }
    }

    #[test]
    fn a_game_command_runs_with_its_words_and_help_lists_it() {
        let mut commands = Commands::new();
        commands.add("god", "nothing hurts", god);
        let mut world = World::new();
        assert_eq!(commands.run(&mut world, "god").unwrap(), "god mode");
        assert_eq!(
            commands.run(&mut world, "god  \"Big Bob\" ").unwrap(),
            "Big Bob is a god"
        );
        assert_eq!(world.len(), 2, "it had the world");
        assert!(commands.run(&mut world, "god a b").is_err());
        let help = commands.run(&mut world, "help").unwrap();
        assert!(help.contains("set — set FILE.FIELD VALUE"), "{help}");
        assert!(help.contains("god — nothing hurts"), "{help}");
        let wrong = commands.run(&mut world, "godd").unwrap_err();
        assert!(wrong.contains("`god`?"), "{wrong}");
        assert_eq!(commands.complete("g"), ["get", "god"]);
        assert_eq!(commands.run(&mut world, "  ").unwrap(), "");
    }

    #[test]
    fn words_split_at_spaces_and_quotes_hold_a_phrase() {
        assert_eq!(
            split(r#"give "health potion" 3"#),
            ["give", "health potion", "3"]
        );
        assert_eq!(split(r#"say """#), ["say", ""]);
        assert!(split("   ").is_empty());
    }

    #[test]
    fn set_changes_the_number_in_the_file_and_the_game_reloads_it() {
        #[derive(serde::Deserialize)]
        struct World_ {
            gravity: f32,
            #[serde(default)]
            wind: f32,
        }
        let text = "// The world's numbers.\n(\n    gravity: -9.81, // down\n)\n";
        let dir = tuning("set", text);
        let mut tuned = crate::Tuned::<World_>::load(dir.join("world.ron")).unwrap();
        let commands = Commands::new().with_tuning(&dir);
        let mut world = World::new();

        assert_eq!(
            commands.run(&mut world, "get world.gravity").unwrap(),
            "world.gravity = -9.81"
        );
        assert_eq!(
            commands.run(&mut world, "set world.gravity -3.5").unwrap(),
            "world.gravity = -3.5"
        );
        let written = std::fs::read_to_string(dir.join("world.ron")).unwrap();
        assert_eq!(
            written, "// The world's numbers.\n(\n    gravity: -3.5, // down\n)\n",
            "only the number changed"
        );
        assert_eq!(tuned.reload_if_changed(), Some(Ok(())));
        assert_eq!(tuned.gravity, -3.5);

        // A field with a default, not written yet: added.
        commands.run(&mut world, "set world.wind 2").unwrap();
        assert_eq!(tuned.reload_if_changed(), Some(Ok(())));
        assert_eq!(tuned.wind, 2.0);

        // What would not do is refused, and the file is left alone.
        let before = std::fs::read_to_string(dir.join("world.ron")).unwrap();
        let word = commands
            .run(&mut world, "set world.gravity fast")
            .unwrap_err();
        assert!(word.contains("a number"), "{word}");
        assert!(commands
            .run(&mut world, "set world.gravity (")
            .unwrap_err()
            .contains("not RON"));
        let kind = commands
            .run(&mut world, "set world.gravity \"up\"")
            .unwrap_err();
        assert!(kind.contains("a number"), "{kind}");
        assert!(commands.run(&mut world, "set world.moon.size 1").is_err());
        assert!(commands.run(&mut world, "get sky.blue").is_err());
        assert!(commands.run(&mut world, "set gravity").is_err());
        assert!(commands.run(&mut world, "get /etc/world.gravity").is_err());
        assert!(commands
            .run(&mut world, "get sub/../../world.gravity")
            .is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("world.ron")).unwrap(),
            before
        );
        assert!(Commands::new()
            .run(&mut world, "get world.gravity")
            .unwrap_err()
            .contains("no tuning/"));
    }
}
