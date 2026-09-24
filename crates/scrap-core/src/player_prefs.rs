//! What a player chose, kept between runs: Unity's PlayerPrefs and
//! `Application.persistentDataPath`.
//!
//! [`user_dir`] is where a game keeps the player's own files — saves,
//! bindings, these prefs — outside the install, which a patch replaces.
//! [`PlayerPrefs`] is a small key-value file there: volume, language, the
//! last save used. RON, sorted, so a player (or a support request) can read
//! it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Stands in for the player's folder when set: each player a game is
/// played by from the editor has one of their own (`.scrap/players/N/`),
/// so two windows on one machine do not share one person's saves and
/// choices — as Unity's virtual players each have their own.
pub const USER_DIR_VAR: &str = "SCRAP_USER_DIR";

/// Where a game keeps the player's own files, made if missing: the
/// platform's per-user data folder with the game's name under it.
/// `$XDG_DATA_HOME` or `~/.local/share` on Linux, `~/Library/Application
/// Support` on macOS, `%APPDATA%` on Windows — or `SCRAP_USER_DIR`.
pub fn user_dir(game: &str) -> std::io::Result<PathBuf> {
    // A browser has no folders: the player's files are kept in memory
    // under this path, and the web host keeps them between visits.
    if cfg!(target_arch = "wasm32") {
        return Ok(PathBuf::from("/user").join(game));
    }
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    if let Some(dir) = env(USER_DIR_VAR).filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(&dir)?;
        return Ok(dir);
    }
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share")))
    }
    .ok_or_else(|| std::io::Error::other("no home folder to keep the player's files in"))?;
    let dir = base.join(game);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// One value a player chose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Pref {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

/// A player's choices, by key, and the file they live in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerPrefs {
    values: BTreeMap<String, Pref>,
    path: Option<PathBuf>,
}

impl PlayerPrefs {
    /// Read them from a file — none there yet is none chosen yet. A file
    /// that does not read is an error, not a quiet reset of every setting.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let values = match crate::files::read_to_string(&path) {
            Ok(text) => ron::from_str(&text).map_err(|e| format!("{}:{e}", path.display()))?,
            Err(_) => BTreeMap::new(),
        };
        Ok(Self {
            values,
            path: Some(path),
        })
    }

    /// `prefs.ron` in [`user_dir`] for the game.
    pub fn of_game(game: &str) -> Result<Self, String> {
        let dir = user_dir(game).map_err(|e| e.to_string())?;
        Self::open(dir.join("prefs.ron"))
    }

    /// Write them back.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let text = ron::ser::to_string_pretty(&self.values, ron::ser::PrettyConfig::new())
            .map_err(|e| e.to_string())?;
        crate::files::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn set(&mut self, key: &str, value: Pref) {
        self.values.insert(key.to_string(), value);
    }

    pub fn has(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    pub fn remove(&mut self, key: &str) {
        self.values.remove(key);
    }

    pub fn bool(&self, key: &str, otherwise: bool) -> bool {
        match self.values.get(key) {
            Some(Pref::Bool(v)) => *v,
            _ => otherwise,
        }
    }

    pub fn int(&self, key: &str, otherwise: i64) -> i64 {
        match self.values.get(key) {
            Some(Pref::Int(v)) => *v,
            _ => otherwise,
        }
    }

    /// A number, whole or not.
    pub fn float(&self, key: &str, otherwise: f64) -> f64 {
        match self.values.get(key) {
            Some(Pref::Float(v)) => *v,
            Some(Pref::Int(v)) => *v as f64,
            _ => otherwise,
        }
    }

    pub fn text<'a>(&'a self, key: &str, otherwise: &'a str) -> &'a str {
        match self.values.get(key) {
            Some(Pref::Text(v)) => v,
            _ => otherwise,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_s_choices_come_back_next_run_and_a_missing_one_is_the_default() {
        let dir = std::env::temp_dir().join("scrap-player-prefs");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prefs.ron");
        let mut prefs = PlayerPrefs::open(&path).unwrap();
        assert_eq!(prefs.float("volume", 0.8), 0.8, "nothing chosen yet");
        prefs.set("volume", Pref::Float(0.3));
        prefs.set("language", Pref::Text("ru".into()));
        prefs.set("subtitles", Pref::Bool(true));
        prefs.save().unwrap();

        let again = PlayerPrefs::open(&path).unwrap();
        assert_eq!(again.float("volume", 0.8), 0.3);
        assert_eq!(again.text("language", "en"), "ru");
        assert!(again.bool("subtitles", false));
        assert_eq!(again.int("volume", 5), 5, "a float is not an int");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.find("language") < text.find("volume"),
            "sorted: {text}"
        );

        std::fs::write(&path, "{ \"volume\": ").unwrap();
        assert!(
            PlayerPrefs::open(&path).is_err(),
            "a broken file is said, not reset"
        );
        let home = user_dir("scrap-test-game").unwrap();
        assert!(home.is_dir() && home.ends_with("scrap-test-game"));
    }
}
