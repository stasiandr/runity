//! Numbers a designer turns while the game runs.
//!
//! Unity's ScriptableObject, for data: a struct of the game's, kept as RON
//! in `tuning/` — how high the player jumps, what a wolf costs, how strong
//! gravity is — and read into the game typed. [`Tuned::poll`] rereads it
//! when the file changes, so balancing is: save, look, save again, with the
//! game running (DNA, postulate 1: balance numbers reload too). A save that
//! does not parse, or does not fit the type, is reported and the game
//! keeps the last good values.
//!
//! ```
//! # use serde::Deserialize;
//! #[derive(Deserialize, Default)]
//! struct Player { jump: f32, speed: f32 }
//! # let dir = std::env::temp_dir().join("runity-tuned-doc");
//! # std::fs::create_dir_all(&dir).unwrap();
//! # std::fs::write(dir.join("player.ron"), "(jump: 1.2, speed: 4.0)").unwrap();
//! let mut player = runity_core::Tuned::<Player>::load(dir.join("player.ron")).unwrap();
//! let height = player.jump; // it derefs to the values
//! # assert_eq!(height, 1.2);
//! ```

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::de::DeserializeOwned;

/// A value read from a RON file, and kept up with it.
#[derive(Debug, Clone)]
pub struct Tuned<T> {
    value: T,
    path: PathBuf,
    stamp: Option<SystemTime>,
    since_poll: f32,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn read<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    ron::from_str(&text).map_err(|e| format!("{}:{e}", path.display()))
}

impl<T: DeserializeOwned> Tuned<T> {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        Ok(Self {
            value: read(&path)?,
            stamp: modified(&path),
            path,
            since_poll: 0.0,
        })
    }

    /// Reread the file if it changed: `None` when it did not, the error in
    /// words when the new text does not fit — with the old values kept.
    pub fn reload_if_changed(&mut self) -> Option<Result<(), String>> {
        let now = modified(&self.path);
        if now == self.stamp {
            return None;
        }
        self.stamp = now;
        Some(read(&self.path).map(|value| self.value = value))
    }

    /// [`Tuned::reload_if_changed`] at most every
    /// [`POLL_SECONDS`]: call it every frame with the delta.
    pub fn poll(&mut self, delta: f32) -> Option<Result<(), String>> {
        self.since_poll += delta;
        if self.since_poll < POLL_SECONDS {
            return None;
        }
        self.since_poll = 0.0;
        self.reload_if_changed()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl<T> Tuned<T> {
    /// Values held as they are, with no file behind them: what an editor
    /// shows while they are being changed and not yet written.
    pub fn fixed(value: T) -> Self {
        Self {
            value,
            path: PathBuf::new(),
            stamp: None,
            since_poll: 0.0,
        }
    }
}

impl<T> std::ops::Deref for Tuned<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Wolf {
        speed: f32,
        #[serde(default)]
        pack: u32,
    }

    fn later(path: &Path, text: &str, secs: u64) {
        std::fs::write(path, text).unwrap();
        let t = SystemTime::now() + std::time::Duration::from_secs(secs);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn a_saved_number_reaches_the_game_and_a_bad_save_keeps_the_last_good_one() {
        let dir = std::env::temp_dir().join("runity-tuned");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("wolf.ron");
        std::fs::write(&path, "(speed: 5.0)").unwrap();
        let mut wolf = Tuned::<Wolf>::load(&path).unwrap();
        assert_eq!(wolf.speed, 5.0);
        assert!(wolf.reload_if_changed().is_none());

        later(&path, "(speed: 7.5, pack: 4)", 2);
        assert_eq!(wolf.reload_if_changed(), Some(Ok(())));
        assert_eq!(
            *wolf,
            Wolf {
                speed: 7.5,
                pack: 4
            }
        );

        later(&path, "(speed: \"fast\")", 4);
        let err = wolf.reload_if_changed().unwrap().unwrap_err();
        assert!(err.contains("wolf.ron"), "names the file: {err}");
        assert_eq!(wolf.speed, 7.5, "the last good value stays");
    }
}

/// How often a watched file is looked at for a change, seconds: often
/// enough that a save shows at once, rarely enough to cost nothing.
pub const POLL_SECONDS: f32 = 0.25;
