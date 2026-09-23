//! The game, run from the editor, talking into the Console.
//!
//! Unity's Console shows what the game logs while it plays; here the game
//! is its own process (DNA, open question 1: the viewport stays the
//! editor's), so what it prints — and what cargo says building it, compile
//! errors first — comes back line by line into [`Session::console`]. The
//! window calls [`Session::scene_view`] every frame, which collects it; a
//! tool without a window calls [`Session::poll_game`].
//!
//! The same poll keeps the game's scene up with the editor: the game
//! watches `.runity/live/<scene>.ron` (named to it by `RUNITY_SCENE_FILE`),
//! and every edit of the open scene is written there — the running game
//! patches itself from it, keeping its state, without anyone saving. What
//! is on disk under `scenes/` changes only when the person saves.
//!
//! And back: the game writes what its world is like to the file named in
//! `RUNITY_STATE_FILE` a few times a second (`LiveScene::report`), and the
//! Inspector shows it as `game.` fields beside the document's.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};

use crate::console::Level;
use crate::{EditError, EditResult, Session};

/// The variable naming the scene file the game watches.
pub const LIVE_VAR: &str = "RUNITY_SCENE_FILE";

/// Write the document where the game watches it, whole or not at all: a
/// game reading halfway through a write would see a broken scene.
pub(crate) fn write_live(scene: &runity::Scene, file: &Path) -> EditResult<()> {
    let io = |e: std::io::Error| EditError::Io(format!("{}: {e}", file.display()));
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let pretty = runity::ron::ser::PrettyConfig::new().depth_limit(4);
    let text = runity::ron::ser::to_string_pretty(scene, pretty)
        .map_err(|e| EditError::Scene(e.to_string()))?;
    let part = file.with_extension("ron.part");
    std::fs::write(&part, text + "\n").map_err(io)?;
    std::fs::rename(&part, file).map_err(io)
}

/// Which document the game shows, where it reads it, and what it was last
/// given.
struct Mirror {
    document: PathBuf,
    file: PathBuf,
    given: runity::Scene,
}

/// A game started from the editor.
pub(crate) struct Running {
    child: Child,
    lines: Receiver<String>,
    mirror: Option<Mirror>,
    /// Where the game says what its world is like.
    state: Option<PathBuf>,
    /// The entry being put together: a message whose details — a stack
    /// trace, where a compile error is — may still be arriving.
    pending: Option<(String, std::time::Instant)>,
}

/// How long an entry waits for more of itself before it is said.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(200);

impl Drop for Running {
    /// Closing the editor does not leave its game behind.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// How much a printed line matters, from how Rust and cargo word it.
fn level_of(line: &str) -> Level {
    let lower = line.trim_start().to_lowercase();
    if lower.starts_with("error") || lower.contains("panicked at") {
        Level::Error
    } else if lower.starts_with("warning") {
        Level::Warning
    } else {
        Level::Info
    }
}

/// Whether a line is more of the one before rather than a line of its own:
/// the frames of a backtrace, the `-->` and `|` of a compile error. Cargo's
/// own progress (`   Compiling game`) is indented too, but is its own line.
fn continues(line: &str) -> bool {
    const CARGO: [&str; 12] = [
        "Compiling",
        "Checking",
        "Finished",
        "Running",
        "Updating",
        "Downloading",
        "Downloaded",
        "Locking",
        "Adding",
        "Blocking",
        "Building",
        "Fresh",
    ];
    let trimmed = line.trim_start();
    if trimmed.len() == line.len() {
        return trimmed.starts_with("Stack backtrace:")
            || trimmed.starts_with("stack backtrace:")
            || trimmed.starts_with("note: ")
            || trimmed.starts_with("Caused by:");
    }
    let first = trimmed.split_whitespace().next().unwrap_or("");
    !CARGO.contains(&first)
}

fn forward(stream: impl Read + Send + 'static, to: Sender<String>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            if to.send(line).is_err() {
                break;
            }
        }
    });
}

impl Session {
    /// Play with the game's own code, its output in the Console:
    /// [`Session::game_command`], started. One game at a time; starting
    /// again stops the one running.
    pub fn start_game(&mut self) -> EditResult<()> {
        let command = self.game_command()?;
        self.run_in_console(command)
    }

    /// Start any command with its output going to the Console, as the game
    /// does: a build, a test run, a tool.
    pub fn run_in_console(&mut self, mut command: Command) -> EditResult<()> {
        self.stop_game();
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| EditError::Io(format!("could not start {command:?}: {e}")))?;
        let (to, lines) = channel();
        if let Some(out) = child.stdout.take() {
            forward(out, to.clone());
        }
        if let Some(err) = child.stderr.take() {
            forward(err, to);
        }
        // A game told where to watch is kept up with the open document.
        let mirror = command
            .get_envs()
            .find(|(k, _)| *k == LIVE_VAR)
            .and_then(|(_, v)| v)
            .zip(self.scene_path.clone())
            .map(|(file, document)| Mirror {
                document,
                file: PathBuf::from(file),
                given: self.history.scene().clone(),
            });
        let state = command
            .get_envs()
            .find(|(k, _)| *k == runity::live::STATE_VAR)
            .and_then(|(_, v)| v)
            .map(PathBuf::from);
        self.game = Some(Running {
            child,
            lines,
            mirror,
            state,
            pending: None,
        });
        Ok(())
    }

    /// Bring what the game printed into the Console. `Some(code)` the call
    /// it is seen to have ended — its last lines are in by then.
    pub fn poll_game(&mut self) -> Option<i32> {
        self.mirror_to_game();
        let running = self.game.as_mut()?;
        let exited = running.child.try_wait().ok().flatten();
        let mut said = Vec::new();
        if exited.is_some() {
            // Ended: what is left in the pipes is all there will be — unless
            // something it started holds them open, which is not waited on
            // for long.
            let give_up = std::time::Instant::now() + std::time::Duration::from_millis(500);
            while let Ok(line) = running
                .lines
                .recv_timeout(give_up.saturating_duration_since(std::time::Instant::now()))
            {
                said.push(line);
            }
        } else {
            while let Ok(line) = running.lines.try_recv() {
                said.push(line);
            }
        }
        // Lines into entries: a message and what continues it are one.
        let mut entries = Vec::new();
        let now = std::time::Instant::now();
        for line in said {
            if line.trim().is_empty() {
                continue;
            }
            match &mut running.pending {
                Some((text, since)) if continues(&line) => {
                    text.push('\n');
                    text.push_str(&line);
                    *since = now;
                }
                pending => {
                    if let Some((text, _)) = pending.replace((line, now)) {
                        entries.push(text);
                    }
                }
            }
        }
        // Said once it has been quiet a moment, or the game is gone.
        if running
            .pending
            .as_ref()
            .is_some_and(|(_, since)| exited.is_some() || since.elapsed() >= SETTLE)
        {
            entries.extend(running.pending.take().map(|(text, _)| text));
        }
        for text in entries {
            let first = text.lines().next().unwrap_or_default().to_string();
            self.say(level_of(&first), text);
        }
        let status = exited?;
        self.game = None;
        let code = status.code().unwrap_or(-1);
        if status.success() {
            self.say(Level::Info, "the game ended");
        } else {
            self.say(Level::Error, format!("the game ended with {status}"));
        }
        Some(code)
    }

    /// Give the running game the open document if it changed since it was
    /// last given — while the document open is the one the game plays.
    fn mirror_to_game(&mut self) {
        let Some(mirror) = self.game.as_mut().and_then(|g| g.mirror.as_mut()) else {
            return;
        };
        let scene = self.history.scene();
        if self.scene_path.as_ref() != Some(&mirror.document) || *scene == mirror.given {
            return;
        }
        let problem = match write_live(scene, &mirror.file) {
            Ok(()) => {
                mirror.given = scene.clone();
                None
            }
            Err(e) => Some(e.to_string()),
        };
        if let Some(problem) = problem {
            self.say(
                Level::Warning,
                format!("the game could not be given the edit: {problem}"),
            );
        }
    }

    /// What the running game last said its world is like: where each of
    /// the scene's entities is, their saved components, what is gone and
    /// what was spawned. `None` without a running game, or before it has
    /// said anything.
    pub fn game_state(&self) -> Option<runity::save::SaveGame> {
        let path = self.game.as_ref()?.state.as_ref()?;
        runity::save::SaveGame::read(path).ok()
    }

    /// Whether a game started from here is still running.
    pub fn game_running(&mut self) -> bool {
        self.poll_game();
        self.game.is_some()
    }

    /// Stop the game started from here. `false` when there was none.
    pub fn stop_game(&mut self) -> bool {
        self.poll_game();
        let Some(mut running) = self.game.take() else {
            return false;
        };
        // Its last entry, even if it was still arriving.
        if let Some((text, _)) = running.pending.take() {
            let first = text.lines().next().unwrap_or_default().to_string();
            self.say(level_of(&first), text);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trace_belongs_to_its_message_and_cargo_progress_does_not() {
        assert!(continues("   0: anyhow::error::from"));
        assert!(continues("  --> src/main.rs:3:5"));
        assert!(continues("   |"));
        assert!(continues("Stack backtrace:"));
        assert!(continues("note: run with `RUST_BACKTRACE=1`"));
        assert!(!continues("   Compiling game v0.1.0 (/tmp/game)"));
        assert!(!continues("    Finished `dev` profile"));
        assert!(!continues("error: could not compile `game`"));
        assert!(!continues("the door opened"));
    }

    #[test]
    fn rust_and_cargo_say_how_much_a_line_matters() {
        assert_eq!(
            level_of("error[E0425]: cannot find value `x`"),
            Level::Error
        );
        assert_eq!(level_of("error: could not compile `game`"), Level::Error);
        assert_eq!(
            level_of("thread 'main' panicked at src/main.rs:3:5:"),
            Level::Error
        );
        assert_eq!(level_of("warning: unused variable: `y`"), Level::Warning);
        assert_eq!(level_of("   Compiling game v0.1.0"), Level::Info);
        assert_eq!(level_of("the door opened"), Level::Info);
    }
}
