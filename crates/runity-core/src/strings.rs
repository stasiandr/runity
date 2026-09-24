//! The game's words in every language it speaks.
//!
//! Unity's Localization tables, as one RON file per language in
//! `strings/`, key to text:
//!
//! ```text
//! // strings/en.ron
//! {
//!     "menu.play": "Play",
//!     "menu.quit": "Quit",
//!     "hud.score": "Score: {0}",
//! }
//! ```
//!
//! A screen says `Text("@menu.play")` and shows the current language's
//! words; code asks `strings.get("hud.score")`. A key a language lacks shows
//! as the key itself — visible on screen, not blank — and `runity check`
//! lists every one. A translator edits a file and the running game shows
//! it, like every other data file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::Tuned;

/// Where the language files are, in a project.
pub const DIR: &str = "strings";

/// One language's table.
pub type Table = BTreeMap<String, String>;

/// The current language's words, kept up with its file.
pub struct Strings {
    table: Tuned<Table>,
    language: String,
}

impl Strings {
    /// The table for `language` (`"en"`, `"ru"`) in a project's `strings/`.
    pub fn load(dir: impl AsRef<Path>, language: &str) -> Result<Self, String> {
        let path = dir.as_ref().join(format!("{language}.ron"));
        Ok(Self {
            table: Tuned::load(&path)?,
            language: language.to_string(),
        })
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    /// Switch language while running; the old one stays if the new file is
    /// not there or does not read.
    pub fn switch(&mut self, dir: impl AsRef<Path>, language: &str) -> Result<(), String> {
        *self = Self::load(dir, language)?;
        Ok(())
    }

    /// Reread the file when a translator saves it.
    pub fn poll(&mut self, delta: f32) -> Option<Result<(), String>> {
        self.table.poll(delta)
    }

    /// A key's words, or the key itself when this language lacks it.
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.table.get(key).map_or(key, String::as_str)
    }

    /// `{0}`, `{1}`, … replaced by `args`: "Score: {0}" with 12 is
    /// "Score: 12". Word order is the translation's to choose.
    pub fn format(&self, key: &str, args: &[&dyn std::fmt::Display]) -> String {
        let mut text = self.get(key).to_string();
        for (i, arg) in args.iter().enumerate() {
            text = text.replace(&format!("{{{i}}}"), &arg.to_string());
        }
        text
    }

    /// A screen's text: `@key` is looked up, anything else is itself.
    pub fn resolve<'a>(&'a self, text: &'a str) -> &'a str {
        match text.strip_prefix('@') {
            Some(key) => self.get(key),
            None => text,
        }
    }
}

/// Every language file in a directory, by language.
pub fn tables(dir: impl AsRef<Path>) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir.as_ref())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ron"))
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p)))
        .collect();
    out.sort();
    out
}

/// Keys some language has and another lacks, in words: what a translator
/// has left to do. `used` are keys the game's files ask for (`@key` in a
/// screen), which every language must have.
pub fn missing(tables: &[(String, Table)], used: &[String]) -> Vec<String> {
    let mut all: Vec<&String> = tables.iter().flat_map(|(_, t)| t.keys()).collect();
    all.extend(used.iter());
    all.sort();
    all.dedup();
    let mut out = Vec::new();
    for (language, table) in tables {
        for key in &all {
            if !table.contains_key(*key) {
                out.push(format!("{language}: no `{key}`"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runity-strings-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("en.ron"),
            r#"{"menu.play": "Play", "hud.score": "Score: {0}"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("ru.ron"),
            r#"{"menu.play": "Играть", "hud.score": "Счёт: {0}"}"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn words_come_from_the_current_language_and_a_missing_key_shows_itself() {
        let dir = dir("get");
        let mut strings = Strings::load(&dir, "en").unwrap();
        assert_eq!(strings.resolve("@menu.play"), "Play");
        assert_eq!(strings.resolve("plain words"), "plain words");
        assert_eq!(strings.format("hud.score", &[&12]), "Score: 12");
        assert_eq!(strings.get("menu.quit"), "menu.quit", "visible, not blank");
        strings.switch(&dir, "ru").unwrap();
        assert_eq!(strings.resolve("@menu.play"), "Играть");
        assert!(strings.switch(&dir, "fr").is_err());
        assert_eq!(strings.language(), "ru", "the old one stays");
    }

    #[test]
    fn what_a_translator_has_left_is_listed() {
        let tables = vec![
            (
                "en".to_string(),
                Table::from([("a".into(), "A".into()), ("b".into(), "B".into())]),
            ),
            ("ru".to_string(), Table::from([("a".into(), "А".into())])),
        ];
        assert_eq!(
            missing(&tables, &["c".to_string()]),
            ["en: no `c`", "ru: no `b`", "ru: no `c`"]
        );
    }
}
