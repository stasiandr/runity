//! The game, run from the editor, talking into the Console.
//!
//! Unity's Console shows what the game logs while it plays; here the game
//! is its own process — the editor does not link its code — so what it
//! prints, and what cargo says building it (compile errors first), comes
//! back line by line into [`Session::console`]. The
//! window calls [`Session::scene_view`] every frame, which collects it; a
//! tool without a window calls [`Session::poll_game`].
//!
//! The same poll keeps the game's scene up with the editor: the game
//! watches `.scrap/live/<scene>.ron` (named to it by `SCRAP_SCENE_FILE`),
//! and every edit of the open scene is written there — the running game
//! patches itself from it, keeping its state, without anyone saving. What
//! is on disk under `scenes/` changes only when the person saves.
//!
//! It draws in the editor's Game view rather than a window of its own
//! ([`crate::embedded`]): its frames come over a local socket, and what the
//! person does over the view goes back.
//!
//! And back: the game writes what its world is like to the file named in
//! `SCRAP_STATE_FILE` a few times a second (`LiveScene::report`), and the
//! Inspector shows it as `game.` fields beside the document's.
//!
//! **Several players** ([`Session::set_players`], Unity's Multiplayer Play
//! Mode): Play starts a game per player on this machine, playing together
//! (DNA, postulate 4). The first is the host, in the Game view — `cargo run`, as alone, with
//! `SCRAP_NET=host:…`; once it is up, the others start from the same
//! build with `join:…`, so nothing is compiled twice and nobody knocks
//! before the door is there. Each of the others has its own window, laid
//! out side by side, its own player folder (`.scrap/players/N/`: its own prefs and
//! saves), and its lines in the Console under its name. The others can be
//! made to play over a bad link ([`Session::set_link`]) — the dacha
//! simulator's Bad Link window. All of them watch
//! the editor's document; the Inspector shows the host's world. Stop ends
//! every one.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};

use crate::console::Level;
use crate::{EditError, EditResult, Session};

/// The variable naming the scene file the game watches.
pub const LIVE_VAR: &str = "SCRAP_SCENE_FILE";

/// Write the document where the game watches it, whole or not at all: a
/// game reading halfway through a write would see a broken scene.
pub(crate) fn write_live(scene: &scrap::Scene, file: &Path) -> EditResult<()> {
    let io = |e: std::io::Error| EditError::Io(format!("{}: {e}", file.display()));
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let pretty = scrap::ron::ser::PrettyConfig::new().depth_limit(4);
    let text = scrap::ron::ser::to_string_pretty(scene, pretty)
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
    given: scrap::Scene,
}

/// The most windows that play together from the editor, as in Unity.
pub const MAX_PLAYERS: u32 = 4;

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
    /// "player 2", when several play: what its Console lines start with.
    label: Option<String>,
    /// The other players, once started; the host's own `Running` has them.
    guests: Vec<Running>,
    /// The players still to start, when the host is up.
    waiting: Option<Waiting>,
    /// Drawing into the Game view rather than a window.
    pub(crate) embed: Option<crate::embedded::Embedded>,
}

/// Players who start once the host is: from the build the host ran, each
/// with what makes it that player.
struct Waiting {
    root: PathBuf,
    players: Vec<(u32, Vec<(OsString, OsString)>)>,
}

impl Running {
    fn start(command: &mut Command) -> EditResult<Self> {
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
        let state = command
            .get_envs()
            .find(|(k, _)| *k == scrap::live::STATE_VAR)
            .and_then(|(_, v)| v)
            .map(PathBuf::from);
        Ok(Self {
            child,
            lines,
            mirror: None,
            state,
            pending: None,
            label: None,
            guests: Vec::new(),
            waiting: None,
            embed: None,
        })
    }

    /// What it printed since last asked, as whole entries, and how it
    /// ended if it has.
    fn collect(&mut self) -> (Vec<String>, Option<ExitStatus>) {
        let exited = self.child.try_wait().ok().flatten();
        let mut said = Vec::new();
        if exited.is_some() {
            // Ended: what is left in the pipes is all there will be — unless
            // something it started holds them open, which is not waited on
            // for long.
            let give_up = std::time::Instant::now() + std::time::Duration::from_millis(500);
            while let Ok(line) = self
                .lines
                .recv_timeout(give_up.saturating_duration_since(std::time::Instant::now()))
            {
                said.push(line);
            }
        } else {
            while let Ok(line) = self.lines.try_recv() {
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
            match &mut self.pending {
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
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, since)| exited.is_some() || since.elapsed() >= SETTLE)
        {
            entries.extend(self.pending.take().map(|(text, _)| text));
        }
        (entries, exited)
    }

    /// An entry as the Console has it: under the player's name, when
    /// several play.
    fn entry(&self, text: String) -> (Level, String) {
        let first = text.lines().next().unwrap_or_default().to_string();
        let text = match &self.label {
            Some(label) => format!("{label}: {text}"),
            None => text,
        };
        (level_of(&first), text)
    }

    /// Start whoever waits for the host, once the host is up — it says
    /// what its world is like as soon as it runs a frame.
    fn start_guests(&mut self) -> Vec<(Level, String)> {
        let up = self.state.as_ref().is_some_and(|s| s.is_file());
        if !up {
            return Vec::new();
        }
        let Some(waiting) = self.waiting.take() else {
            return Vec::new();
        };
        let exe = match executable(&waiting.root) {
            Ok(exe) => exe,
            Err(e) => {
                return vec![(
                    Level::Error,
                    format!("the other players could not start: {e}"),
                )]
            }
        };
        let mut said = Vec::new();
        for (number, envs) in waiting.players {
            let mut command = Command::new(&exe);
            command.current_dir(&waiting.root).envs(envs);
            match Running::start(&mut command) {
                Ok(mut guest) => {
                    guest.label = Some(format!("player {number}"));
                    self.guests.push(guest);
                }
                Err(e) => said.push((Level::Error, format!("player {number}: {e}"))),
            }
        }
        said
    }
}

/// The game's executable, as `cargo run` built it for the host: from
/// `cargo metadata`, which knows the target folder wherever it is.
fn executable(root: &Path) -> Result<PathBuf, String> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("cargo metadata said something unreadable: {e}"))?;
    let target = meta["target_directory"]
        .as_str()
        .ok_or("cargo metadata named no target folder")?;
    let bin = meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|p| p["targets"].as_array().into_iter().flatten())
        .find(|t| {
            t["kind"]
                .as_array()
                .is_some_and(|k| k.iter().any(|k| k == "bin"))
        })
        .and_then(|t| t["name"].as_str())
        .ok_or("the game crate has no executable")?;
    let exe = Path::new(target)
        .join("debug")
        .join(format!("{bin}{}", std::env::consts::EXE_SUFFIX));
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(format!("{} is not built", exe.display()))
    }
}

fn free_port() -> EditResult<u16> {
    scrap::party::free_port().map_err(|e| EditError::Io(format!("no free port to host on: {e}")))
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
    /// [`Session::game_command`], started — once per player,
    /// [`Session::players`] of them playing together. One game at a time;
    /// starting again stops the one running.
    pub fn start_game(&mut self) -> EditResult<()> {
        self.start_game_at(None)
    }

    /// [`Session::start_game`], told where the player starts (Play from
    /// Here, [`Session::start_game_from_here`]): `SCRAP_START`, which every
    /// player's game is given, as it is given the scene.
    pub fn start_game_at(&mut self, start: Option<scrap::player::Start>) -> EditResult<()> {
        let mut command = self.game_command_at(start)?;
        // The host plays in the Game view; the others, when several play,
        // each in a window of its own.
        let (embed, address) = crate::embedded::Embedded::listen()?;
        command.env(scrap::embed::EMBED_VAR, address);
        let count = self.players();
        if count == 1 {
            self.run_in_console(command)?;
            if let Some(host) = self.game.as_mut() {
                host.embed = Some(embed);
            }
            return Ok(());
        }
        let project = self.project.clone().ok_or(EditError::NotInProject)?;
        let root = project.root().to_path_buf();
        let size = scrap::project::GameSettings::load(&root.to_string_lossy())
            .map(|(_, s)| (s.width, s.height))
            .unwrap_or((1280, 720));
        let address = format!("127.0.0.1:{}", free_port()?);
        // The others play over the link this person chose; the host's own
        // game is in the server's process and has no link to spoil.
        let link = self.link();
        command
            .env(scrap::party::NET_VAR, format!("host:{address}"))
            .env(scrap::party::PLAYER_VAR, "Player 1")
            .env(
                scrap::party::WINDOW_VAR,
                scrap::party::tile(0, count, size),
            );
        // What every player shares with the host: the scene, the file it
        // watches, where the player starts. What is each one's own: its
        // window, its state, its folder.
        let shared: Vec<(OsString, OsString)> = command
            .get_envs()
            .filter(|(k, _)| {
                *k == "SCRAP_SCENE" || *k == LIVE_VAR || *k == scrap::player::START_VAR
            })
            .filter_map(|(k, v)| Some((k.to_os_string(), v?.to_os_string())))
            .collect();
        let state = command
            .get_envs()
            .find(|(k, _)| *k == scrap::live::STATE_VAR)
            .and_then(|(_, v)| v)
            .map(PathBuf::from)
            .unwrap_or_default();
        let players = (1..count)
            .map(|peer| {
                let number = peer + 1;
                let mut envs = shared.clone();
                let mut set = |k: &str, v: String| envs.push((k.into(), v.into()));
                set(scrap::party::NET_VAR, format!("join:{address}"));
                set(scrap::party::PLAYER_VAR, format!("Player {number}"));
                if !link.is_empty() {
                    set(scrap::party::LINK_VAR, link.clone());
                }
                set(
                    scrap::party::WINDOW_VAR,
                    scrap::party::tile(peer, count, size),
                );
                set(
                    scrap::live::STATE_VAR,
                    state
                        .with_extension(format!("player{number}.ron"))
                        .to_string_lossy()
                        .into_owned(),
                );
                set(
                    scrap::player_prefs::USER_DIR_VAR,
                    root.join(format!(".scrap/players/{number}"))
                        .to_string_lossy()
                        .into_owned(),
                );
                (number, envs)
            })
            .collect();
        self.run_in_console(command)?;
        if let Some(host) = self.game.as_mut() {
            host.embed = Some(embed);
            host.label = Some("player 1".into());
            host.waiting = Some(Waiting { root, players });
        }
        self.say(
            Level::Info,
            format!("{count} players: player 1 hosts on {address}, the others join once it is up"),
        );
        Ok(())
    }

    /// Start any command with its output going to the Console, as the game
    /// does: a build, a test run, a tool.
    pub fn run_in_console(&mut self, mut command: Command) -> EditResult<()> {
        self.stop_game();
        let mut running = Running::start(&mut command)?;
        // A game told where to watch is kept up with the open document.
        running.mirror = command
            .get_envs()
            .find(|(k, _)| *k == LIVE_VAR)
            .and_then(|(_, v)| v)
            .zip(self.scene_path.clone())
            .map(|(file, document)| Mirror {
                document,
                file: PathBuf::from(file),
                given: self.history.scene().clone(),
            });
        self.game = Some(running);
        Ok(())
    }

    /// Bring what the game printed into the Console. `Some(code)` the call
    /// it is seen to have ended — its last lines are in by then.
    pub fn poll_game(&mut self) -> Option<i32> {
        self.mirror_to_game();
        self.poll_embedded();
        let running = self.game.as_mut()?;
        let mut said = running.start_guests();
        let (entries, exited) = running.collect();
        said.extend(entries.into_iter().map(|t| running.entry(t)));
        // The other players: their lines, and their going.
        running.guests.retain_mut(|guest| {
            let (entries, ended) = guest.collect();
            said.extend(entries.into_iter().map(|t| guest.entry(t)));
            match ended {
                Some(status) if status.success() => {
                    said.push((
                        Level::Info,
                        format!("{} ended", guest.label.as_deref().unwrap_or("a player")),
                    ));
                    false
                }
                Some(status) => {
                    said.push((
                        Level::Error,
                        format!(
                            "{} ended with {status}",
                            guest.label.as_deref().unwrap_or("a player")
                        ),
                    ));
                    false
                }
                None => true,
            }
        });
        for (level, text) in said {
            self.say(level, text);
        }
        let status = exited?;
        // The host gone is the game gone: the others go with it, and a
        // Game view it drew in goes back to the scene.
        if self.game.take().is_some_and(|g| g.embed.is_some()) {
            self.game_view = false;
        }
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
    pub fn game_state(&self) -> Option<scrap::save::SaveGame> {
        let path = self.game.as_ref()?.state.as_ref()?;
        scrap::save::SaveGame::read(path).ok()
    }

    /// The animator state the running game says an entity is in: what
    /// the Animator window lights up. `None` without a game, or for an
    /// entity with no animation controller.
    pub fn game_animator(&self, id: scrap::EntityId) -> Option<String> {
        self.game_state()?
            .entities
            .into_iter()
            .find(|s| s.id == id)
            .map(|s| s.animator)
            .filter(|a| !a.is_empty())
    }

    /// Every player's report while a game started from here runs: player
    /// 1 (the host) first, then each other window, as `(player, report)`.
    /// What the network inspector and the world diff read.
    pub fn player_states(&self) -> Vec<(u32, scrap::save::SaveGame)> {
        let Some(host) = self.game.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (i, running) in std::iter::once(host).chain(host.guests.iter()).enumerate() {
            if let Some(state) = running
                .state
                .as_ref()
                .and_then(|p| scrap::save::SaveGame::read(p).ok())
            {
                let player = state.diagnostics.as_ref().map_or(i as u32, |d| d.me) + 1;
                out.push((player, state));
            }
        }
        out
    }

    /// The game's systems, in the order its step calls them: read from
    /// `src/main.rs`, after its `// systems, in order` line. What the
    /// systems list shows before the game runs, and orders timings by.
    pub fn systems(&self) -> Vec<String> {
        let Some(project) = self.project() else {
            return Vec::new();
        };
        let Ok(text) = std::fs::read_to_string(project.root().join("src/main.rs")) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut listing = false;
        for line in text.lines() {
            let line = line.trim();
            if line == "// systems, in order" {
                listing = true;
                continue;
            }
            if !listing {
                continue;
            }
            match line
                .split("systems::")
                .nth(1)
                .and_then(|rest| rest.split("::").next())
            {
                Some(name) => out.push(name.to_string()),
                None if line.starts_with("//") || line.is_empty() => {}
                None => break,
            }
        }
        out
    }

    /// Saved games on disk: every RON file that reads as one, in the game's
    /// own folder for this machine and in each editor player's folder
    /// (`.scrap/players/N/`), newest first.
    pub fn save_files(&self) -> Vec<(PathBuf, scrap::save::SaveGame)> {
        let Some(project) = self.project() else {
            return Vec::new();
        };
        let mut dirs = Vec::new();
        if let Ok(dir) = scrap::player_prefs::user_dir(project.name()) {
            dirs.push(dir);
        }
        for n in 1..=MAX_PLAYERS {
            dirs.push(project.root().join(format!(".scrap/players/{n}")));
        }
        let mut out: Vec<(PathBuf, scrap::save::SaveGame, std::time::SystemTime)> = Vec::new();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for path in entries.flatten().map(|e| e.path()) {
                if path.extension().is_none_or(|e| e != "ron") {
                    continue;
                }
                if let Ok(save) = scrap::save::SaveGame::read(&path) {
                    let when = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::UNIX_EPOCH);
                    out.push((path, save, when));
                }
            }
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.2));
        out.into_iter().map(|(p, s, _)| (p, s)).collect()
    }

    /// Whether a game started from here is still running.
    pub fn game_running(&mut self) -> bool {
        self.poll_game();
        self.game.is_some()
    }

    /// Whether a game started from here was running when last polled —
    /// for what is drawn every frame, which polls anyway.
    pub fn is_game_running(&self) -> bool {
        self.game.is_some()
    }

    /// Stop the game started from here. `false` when there was none.
    pub fn stop_game(&mut self) -> bool {
        self.poll_game();
        let Some(mut running) = self.game.take() else {
            return false;
        };
        // Its last entries, even if they were still arriving.
        let mut last = Vec::new();
        if let Some((text, _)) = running.pending.take() {
            last.push(running.entry(text));
        }
        for guest in &mut running.guests {
            if let Some((text, _)) = guest.pending.take() {
                last.push(guest.entry(text));
            }
        }
        for (level, text) in last {
            self.say(level, text);
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
