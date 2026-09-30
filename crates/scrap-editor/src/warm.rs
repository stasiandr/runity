//! Play without waiting for the game to start.
//!
//! Started cold, Play is `cargo run`, the game's `main` reading the
//! library, a device and a renderer made, the scene spawned and its
//! pipelines built — seconds before the first frame for a big game. So the
//! editor starts the game ahead, as soon as there is a scene to play and
//! no game playing: the same command Play would give, with
//! [`scrap::embed::HOLD_VAR`] set, which makes the game do all of that and
//! then wait, drawing nothing into the view, running no step and heard by
//! nobody (a mute file of its own says so). Pressing Play lets it go: a
//! frame later it plays.
//!
//! It is the game Play would start only while nothing it was started from
//! changed. The scene may: the editor writes the document where the game
//! watches it as Play does, and the game takes the change as it takes any
//! edit while it plays. What else it was started from may not — another
//! scene, the optimized-code setting, the players, or a source the build
//! reads (the game's and the engine's crates) newer than when it started.
//! Then it is stopped and started again for what is there now, and Play
//! pressed before it is ready starts cold, as without it.
//!
//! Only where a window asked for it ([`Session::set_play_ahead`]): a tool
//! polling a session headless would otherwise build and start games no one
//! plays.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::console::Level;
use crate::game::{write_live, Mirror, Running, Waiting};
use crate::{EditResult, Session};

/// How often the sources are looked at for a change.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/// A game started ahead of Play, held.
pub(crate) struct Warm {
    running: Running,
    /// The document it plays.
    document: PathBuf,
    /// What else Play would start it with: optimized code, how many play
    /// and over what link.
    fast: bool,
    players: u32,
    link: String,
    /// The others, when several play: started once it plays and is up.
    waiting: Option<(Waiting, String)>,
    started: Instant,
    /// Set by the watcher once a source is newer than the start.
    stale: Arc<AtomicBool>,
    roots: Vec<PathBuf>,
    since: SystemTime,
}

impl Warm {
    /// Whether a source the build reads changed since it started: looked
    /// at now, not only when the watcher last did.
    fn stale_now(&self) -> bool {
        self.stale.load(Ordering::Relaxed) || newest(&self.roots) > self.since
    }
}

impl Session {
    /// Start the game ahead of Play from now on ([`crate::warm`]), or stop
    /// the one started and start no more.
    pub fn set_play_ahead(&mut self, on: bool) {
        self.play_ahead = on;
        if !on {
            self.warm = None;
        }
    }

    /// Whether a game started ahead is held, ready or getting ready.
    pub fn is_warm(&self) -> bool {
        self.warm.is_some()
    }

    /// Whether the game started ahead is ready: started, its scene
    /// spawned and drawn once, waiting for Play.
    pub fn is_warm_ready(&self) -> bool {
        self.warm
            .as_ref()
            .and_then(|w| w.running.embed.as_ref())
            .is_some_and(|e| e.held)
    }

    /// Part of [`Session::poll_game`]: the game started ahead kept up with
    /// what Play would start — dropped when it no longer is, or when it
    /// ended, and started when there is none.
    pub(crate) fn keep_warm(&mut self) {
        let view = self.size();
        if let Some(warm) = self.warm.as_mut() {
            if let Some(embed) = warm.running.embed.as_mut() {
                embed.poll(view);
            }
        }
        let fits = self.warm.as_mut().is_some_and(|w| !w.running.has_ended())
            && self.warm.as_ref().is_some_and(|w| self.still_fits(w) && !w.stale.load(Ordering::Relaxed));
        if !fits {
            self.warm = None;
        }
        if self.warm.is_some() || !self.play_ahead || self.game.is_some() {
            return;
        }
        let Some(document) = self.scene_path.clone() else {
            return;
        };
        // Once for a scene that failed to start ahead: Play starts it cold,
        // and says why.
        if self.warm_failed.as_ref() == Some(&document) {
            return;
        }
        match self.start_warm(&document) {
            Ok(warm) => self.warm = Some(warm),
            Err(e) => {
                self.say(Level::Info, format!("the game could not be started ahead of Play: {e}"));
                self.warm_failed = Some(document);
            }
        }
    }

    fn start_warm(&mut self, document: &Path) -> EditResult<Warm> {
        let since = SystemTime::now();
        let (mut command, embed, waiting) = self.play_command(None, true)?;
        let project = self.project.clone().ok_or(crate::EditError::NotInProject)?;
        let root = project.root().to_path_buf();
        let mute = root.join(".scrap/live/mute-ahead");
        if let Some(parent) = mute.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&mute, "true\n");
        command
            .env(scrap::embed::HOLD_VAR, "1")
            .env(scrap::sound::MUTE_VAR, &mute);
        let mut running = Running::start(&mut command)?;
        running.embed = Some(embed);
        running.mute = Some(mute);
        running.mirror = command
            .get_envs()
            .find(|(k, _)| *k == crate::game::LIVE_VAR)
            .and_then(|(_, v)| v)
            .map(|file| Mirror {
                document: document.to_path_buf(),
                file: PathBuf::from(file),
                given: self.history.scene().clone(),
            });
        let roots = sources(&root);
        let stale = Arc::new(AtomicBool::new(false));
        watch(roots.clone(), since, Arc::downgrade(&stale));
        Ok(Warm {
            running,
            document: document.to_path_buf(),
            fast: self.fast_game(),
            players: self.players(),
            link: self.link(),
            waiting,
            started: Instant::now(),
            stale,
            roots,
            since,
        })
    }

    /// Whether Play would start the game it was started as: the same
    /// scene and the same settings.
    fn still_fits(&self, warm: &Warm) -> bool {
        self.scene_path.as_ref() == Some(&warm.document)
            && warm.fast == self.fast_game()
            && warm.players == self.players()
            && warm.link == self.link()
    }

    /// Play with the game started ahead, when it is still the game Play
    /// would start: `false` when there is none, and Play starts cold.
    pub(crate) fn play_warm(&mut self) -> EditResult<bool> {
        self.warm_failed = None;
        let Some(warm) = self.warm.take() else {
            return Ok(false);
        };
        let mut warm = warm;
        if warm.running.has_ended() || !self.still_fits(&warm) || warm.stale_now() {
            return Ok(false);
        }
        if self.game.is_some() {
            self.stop_game();
        }
        // As Play started cold would: the scene saved, the document where
        // the game watches it, the editor's mute.
        if self.is_modified() {
            self.save_scene(None)?;
        }
        // Only when edited since: the game reads it again when it changes.
        if let Some(mirror) = warm.running.mirror.as_mut().filter(|m| m.given != *self.history.scene()) {
            write_live(self.history.scene(), &mirror.file)?;
            mirror.given = self.history.scene().clone();
        }
        if let Some(embed) = warm.running.embed.as_mut() {
            embed.play();
        }
        let ready = warm.started.elapsed();
        let name = self
            .project
            .as_ref()
            .and_then(|p| p.relative(&warm.document))
            .unwrap_or_else(|| scrap::layout::name_of(&warm.document));
        self.game = Some(warm.running);
        self.write_mute_file();
        self.host_waits(warm.waiting.take());
        self.say(
            Level::Info,
            format!(
                "playing {name} in the game (started {:.0} s ahead)",
                ready.as_secs_f32()
            ),
        );
        Ok(true)
    }
}

/// The folders a game's build reads: its own crate's, and — for an engine
/// it takes by path, as a project beside a checkout does — every crate
/// beside the engine's.
fn sources(root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        root.join("src"),
        root.join("build.rs"),
        root.join("Cargo.toml"),
        root.join("Cargo.lock"),
    ];
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    for line in manifest.lines() {
        let Some(rest) = line.split("path = \"").nth(1) else {
            continue;
        };
        let Some(path) = rest.split('"').next() else {
            continue;
        };
        let dir = root.join(path);
        let crates = dir.parent().map(Path::to_path_buf).unwrap_or(dir);
        if !roots.contains(&crates) {
            roots.push(crates);
        }
    }
    roots
}

/// The newest time any source under `roots` was changed.
fn newest(roots: &[PathBuf]) -> SystemTime {
    fn walk(path: &Path, newest: &mut SystemTime) {
        let Ok(meta) = std::fs::metadata(path) else {
            return;
        };
        if meta.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "target" | "tests" | "benches" | "examples" | "node_modules") || name.starts_with('.') {
                return;
            }
            for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
                walk(&entry.path(), newest);
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "rs" | "toml" | "lock" | "wgsl"))
        {
            if let Ok(modified) = meta.modified() {
                *newest = (*newest).max(modified);
            }
        }
    }
    let mut out = SystemTime::UNIX_EPOCH;
    for root in roots {
        walk(root, &mut out);
    }
    out
}

/// Look at the sources every little while until one is newer than `since`
/// — then say the game started ahead is stale — or it is gone.
fn watch(roots: Vec<PathBuf>, since: SystemTime, stale: std::sync::Weak<AtomicBool>) {
    let _ = std::thread::Builder::new()
        .name("scrap play-ahead".into())
        .spawn(move || loop {
            std::thread::sleep(LOOK_EVERY);
            let Some(stale) = stale.upgrade() else { return };
            if newest(&roots) > since {
                stale.store(true, Ordering::Relaxed);
                return;
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_changed_after_the_start_is_seen_and_one_outside_is_not() {
        let dir = std::env::temp_dir().join(format!("scrap-warm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("game/src")).unwrap();
        std::fs::create_dir_all(dir.join("engine/crates/scrap/src")).unwrap();
        std::fs::create_dir_all(dir.join("engine/crates/scrap/target")).unwrap();
        std::fs::write(
            dir.join("game/Cargo.toml"),
            "[dependencies]\nscrap = { path = \"../engine/crates/scrap\" }\n",
        )
        .unwrap();
        std::fs::write(dir.join("game/src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.join("engine/crates/scrap/src/lib.rs"), "").unwrap();
        let roots = sources(&dir.join("game"));
        assert!(roots.contains(&dir.join("game/../engine/crates")));
        std::thread::sleep(Duration::from_millis(20));
        let since = SystemTime::now();
        std::thread::sleep(Duration::from_millis(20));
        assert!(newest(&roots) <= since);
        // What the build does not read: its output, a note.
        std::fs::write(dir.join("engine/crates/scrap/target/x.rs"), "").unwrap();
        std::fs::write(dir.join("game/src/notes.txt"), "").unwrap();
        assert!(newest(&roots) <= since);
        std::fs::write(dir.join("engine/crates/scrap/src/lib.rs"), "// changed").unwrap();
        assert!(newest(&roots) > since);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
