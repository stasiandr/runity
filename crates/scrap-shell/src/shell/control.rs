//! A running game driven from outside its window: `SCRAP_CONTROL` (or
//! [`super::WindowConfig::control`]) opens a socket an agent's tool talks
//! to, a line of JSON a request — the editor's live MCP (docs/live-mcp.md)
//! for a game playing in a window of its own, where there is no studio.
//!
//! The shell answers what any game has: `hello` (the window, the game),
//! `shot` (the frame exactly as the window showed it, PNG, cut and scaled,
//! now or after so many frames, one or several in a row), `input` (events
//! as a device would send them, each when due), `pause` / `resume` /
//! `step` (fixed steps while paused) / `frames` (wait so many) /
//! `time_scale`, `perf` (the last frames' times), `quit`. Anything else is
//! the game's ([`super::Game::control`]): asked during its frame with the
//! renderer at hand, it answers at once, fails, or says "not yet" and is
//! asked again next frame until it answers or the request's `timeout`
//! (seconds, 60 by default) runs out — how a tool waits for a state
//! without polling.
//!
//! Wire: a Unix socket (owner-only, as the studio's: a TCP port would be
//! open to every process of every user of the machine), a line
//! `{"token": "…", "id": 1, "cmd": "shot", "args": {…}}` in, a line
//! `{"id": 1, "ok": true, "result": …}` (or `"ok": false, "error": "…"`)
//! out. The socket's path and a random token are written beside it in
//! `<socket>.json` (0600) and said on stderr; a request without the token
//! is refused. Off unless asked for; never in a browser.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use scrap_core::web_time::Instant;

pub use serde_json as json;
pub use serde_json::Value;

use crate::input::InputEvent;

pub use scrap_core::control::Answer;

/// The variable that turns control on: `1` (a socket in the temporary
/// folder, named by the process) or the socket's path.
pub const CONTROL_VAR: &str = "SCRAP_CONTROL";

/// The socket for a value of [`CONTROL_VAR`].
pub fn socket_for(value: &str) -> PathBuf {
    if value == "1" || value.is_empty() {
        std::env::temp_dir().join(format!("scrap-control-{}.sock", std::process::id()))
    } else {
        PathBuf::from(value)
    }
}

/// A request waiting for its answer.
pub(super) struct Request {
    pub id: Value,
    pub cmd: String,
    pub args: Value,
    pub(super) reply: Sender<String>,
    pub began: Instant,
    /// The loop's turn it arrived on, and what it waits for.
    pub turn: u64,
    pub until: Option<Until>,
    /// Frames drawn when it was taken (a shot that waits on a window
    /// showing nothing gives up).
    pub drawn_then: u64,
}

pub(super) enum Until {
    /// The game's: asked after each frame until it answers.
    Game,
    /// The script under way ends.
    Script,
    Turn(u64),
    Step(u64),
    Shots { want: usize, files: Vec<Value> },
}

/// The shell's own commands (the rest are the game's: `help` lists all).
pub const SHELL_COMMANDS: [[&str; 2]; 11] = [
    ["script", "source=… | path=file.rhai [lib=helpers.rhai] [timeout=600]: a Rhai program of these commands, its cases and asserts (one at a time)"],
    ["script_cancel", "the script under way stops at its next step"],
    ["hello", "the game, the drawable's size, paused or not, the turn and the step"],
    ["shot", "path=… [half=p1|p2 | crop=[x,y,w,h]] [width=…] [frames=N] [after=N]: the frame as shown, PNG"],
    ["input", "events=[\"KeyDown(Space)\", [6, \"KeyUp(Space)\"]]: as a device sends them, each when due"],
    ["pause", "the fixed steps stop (frames go on)"],
    ["resume", "they go on"],
    ["step", "steps=N: so many fixed steps, paused"],
    ["frames", "frames=N: answered N frames later"],
    ["time_scale", "scale=0.5"],
    ["perf", "[frames=240]: the last frames' fps, avg, p50, p99, longest, steps a frame, GPU ms, size"],
];

/// The wire's version.
pub const VERSION: u32 = 1;

static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl Request {
    /// Answered once: `{"v":1,"re":id,"ok":…,"seq":n,"frame":turn,"result"|"error","code"}`.
    pub fn answer(self, answer: Result<Value, String>) {
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let frame = super::LOOP_TURN.load(std::sync::atomic::Ordering::Relaxed);
        let line = match answer {
            Ok(result) => json::json!({ "v": VERSION, "re": self.id, "ok": true, "seq": seq, "frame": frame, "result": result }),
            Err(error) => {
                let code = if error.starts_with("unknown command") {
                    "unknown_command"
                } else if error.contains("timed out") {
                    "timeout"
                } else {
                    "failed"
                };
                json::json!({ "v": VERSION, "re": self.id, "ok": false, "seq": seq, "frame": frame, "code": code, "error": error })
            }
        };
        let _ = self.reply.send(line.to_string());
    }

    /// Seconds it may wait (`timeout` in its args, 60 by default).
    pub fn timeout(&self) -> f32 {
        self.args.get("timeout").and_then(Value::as_f64).unwrap_or(60.0) as f32
    }
}

/// The control desk: requests in from the socket's threads, what they
/// asked the loop for.
pub(super) struct Desk {
    inbox: Receiver<Request>,
    pub waiting: Vec<Request>,
    pub paused: bool,
    /// Fixed steps owed while paused (`step`).
    pub steps_owed: u32,
    /// Input to play: the turn it is due on, the event.
    pub due: Vec<(u64, InputEvent)>,
    /// Shots asked for: the first turn wanted, how many, how cut.
    pub shots: Vec<ShotJob>,
    socket: PathBuf,
    /// Where a script's commands come in, as a client's.
    pub to_loop: Sender<Request>,
    /// The script under way (one at a time).
    pub script: Option<super::script::Running>,
}

pub(super) struct ShotJob {
    pub from_turn: u64,
    pub left: u32,
    /// How many were asked, and whether the window was told to keep them.
    pub armed_count: u32,
    pub armed: bool,
    pub path: PathBuf,
    pub crop: Option<(u32, u32, u32, u32)>,
    pub width: Option<u32>,
    pub request: u64,
}

impl Desk {
    /// Listen at `socket`; the token and the path said.
    pub fn open(socket: &Path, title: &str) -> std::io::Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        let _ = std::fs::remove_file(socket);
        let listener = UnixListener::bind(socket)?;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
        let token = token();
        let card = socket.with_extension("sock.json");
        let about = json::json!({ "socket": socket, "token": token, "pid": std::process::id(), "game": title });
        std::fs::write(&card, about.to_string())?;
        std::fs::set_permissions(&card, std::fs::Permissions::from_mode(0o600))?;
        eprintln!("control: {} (token in {})", socket.display(), card.display());
        let (to_loop, inbox) = channel::<Request>();
        let for_scripts = to_loop.clone();
        std::thread::Builder::new().name("scrap-control".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                let to_loop = to_loop.clone();
                let token = token.clone();
                let _ = std::thread::Builder::new().name("scrap-control-client".into()).spawn(move || serve(stream, &token, &to_loop));
            }
        })?;
        Ok(Self { inbox, waiting: Vec::new(), paused: false, steps_owed: 0, due: Vec::new(), shots: Vec::new(), socket: socket.to_path_buf(), to_loop: for_scripts, script: None })
    }

    /// The requests that came since the last turn.
    pub fn take_new(&mut self, turn: u64) {
        while let Ok(mut r) = self.inbox.try_recv() {
            r.turn = turn;
            self.waiting.push(r);
        }
    }

    /// The input due by `turn`, in the order asked.
    pub fn due_by(&mut self, turn: u64) -> Vec<InputEvent> {
        let mut out = Vec::new();
        self.due.retain(|(at, e)| {
            if *at <= turn {
                out.push(e.clone());
                false
            } else {
                true
            }
        });
        out
    }
}

impl Drop for Desk {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_file(self.socket.with_extension("sock.json"));
    }
}

/// One connection: lines in, each handed to the loop and its answer
/// written back, in order.
fn serve(stream: std::os::unix::net::UnixStream, token: &str, to_loop: &Sender<Request>) {
    let Ok(mut out) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let parsed: Result<Value, _> = json::from_str(&line);
        let answer = match parsed {
            Err(e) => json::json!({ "v": VERSION, "re": null, "ok": false, "code": "bad_request", "error": format!("not JSON: {e}") }).to_string(),
            Ok(v) if v.get("token").and_then(Value::as_str) != Some(token) => {
                json::json!({ "v": VERSION, "re": v.get("id"), "ok": false, "code": "unauthorized", "error": "wrong or missing token" }).to_string()
            }
            Ok(v) => {
                let (reply, answer) = channel();
                let request = Request {
                    id: v.get("id").cloned().unwrap_or(Value::Null),
                    cmd: v.get("cmd").and_then(Value::as_str).unwrap_or_default().to_string(),
                    args: v.get("args").cloned().unwrap_or(Value::Null),
                    reply,
                    began: Instant::now(),
                    turn: 0,
                    until: None,
                    drawn_then: 0,
                };
                if to_loop.send(request).is_err() {
                    return;
                }
                match answer.recv() {
                    Ok(line) => line,
                    Err(_) => return,
                }
            }
        };
        if writeln!(out, "{answer}").is_err() {
            return;
        }
    }
}

/// Sixteen random bytes as hex, from the system's source.
fn token() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut bytes);
    }
    if bytes == [0; 16] {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        bytes.copy_from_slice(&(t ^ (std::process::id() as u128) << 64).to_le_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Events from a request's `events`: each a script's event (`"KeyDown(Space)"`,
/// `"PadDownOn(pad: 1, button: South)"`) or `[frames_later, "…"]`.
pub(super) fn events_of(args: &Value) -> Result<Vec<(u64, InputEvent)>, String> {
    let list = args.get("events").and_then(Value::as_array).ok_or("input wants events: [\"KeyDown(Space)\", [6, \"KeyUp(Space)\"]]")?;
    let mut out = Vec::new();
    for item in list {
        let (after, text) = match item {
            Value::String(s) => (0, s.as_str()),
            Value::Array(pair) if pair.len() == 2 => (pair[0].as_u64().unwrap_or(0), pair[1].as_str().unwrap_or_default()),
            other => return Err(format!("not an event: {other}")),
        };
        let event: InputEvent = ron::from_str(text).map_err(|e| format!("{text}: {e}"))?;
        out.push((after, event));
    }
    Ok(out)
}

/// A shot written: cut to `crop` (x, y, w, h in the drawable's pixels),
/// scaled to `width` (its height in proportion).
pub(super) fn write_shot(shot: &super::WindowShot, job: &ShotJob, index: u32) -> Result<Value, String> {
    let mut picture = image::RgbaImage::from_raw(shot.width, shot.height, shot.rgba.clone()).ok_or("a shot of the wrong size")?;
    if let Some((x, y, w, h)) = job.crop {
        let (x, y) = (x.min(shot.width - 1), y.min(shot.height - 1));
        let (w, h) = (w.min(shot.width - x).max(1), h.min(shot.height - y).max(1));
        picture = image::imageops::crop_imm(&picture, x, y, w, h).to_image();
    }
    if let Some(width) = job.width.filter(|w| *w > 0 && *w < picture.width()) {
        let height = (picture.height() as u64 * width as u64 / picture.width() as u64).max(1) as u32;
        picture = image::imageops::resize(&picture, width, height, image::imageops::FilterType::Triangle);
    }
    let path = if job.left > 0 || index > 0 {
        let stem = job.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "shot".into());
        job.path.with_file_name(format!("{stem}_{index}.png"))
    } else {
        job.path.clone()
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    picture.save(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(json::json!({ "path": path, "width": picture.width(), "height": picture.height(), "turn": shot.turn }))
}

/// The loop's turn now (the frame a request's answer is tagged with):
/// what a game's own multi-frame command counts by.
pub fn frame() -> u64 {
    super::LOOP_TURN.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    /// The socket: a request without the token refused there, one with it
    /// handed to the loop and its answer back on the line, versioned.
    #[test]
    fn a_request_needs_the_token_and_is_answered_by_the_loop() {
        let socket = std::env::temp_dir().join(format!("scrap-control-test-{}.sock", std::process::id()));
        let mut desk = Desk::open(&socket, "test").unwrap();
        let card: Value = json::from_str(&std::fs::read_to_string(socket.with_extension("sock.json")).unwrap()).unwrap();
        let token = card["token"].as_str().unwrap().to_string();
        let ask = |line: String| {
            let stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
            let mut out = stream.try_clone().unwrap();
            writeln!(out, "{line}").unwrap();
            std::thread::spawn(move || {
                let mut answer = String::new();
                BufReader::new(stream).read_line(&mut answer).unwrap();
                json::from_str::<Value>(&answer).unwrap()
            })
        };
        let refused = ask(json::json!({ "token": "nope", "id": 1, "cmd": "hello" }).to_string()).join().unwrap();
        assert_eq!(refused["code"], "unauthorized");
        let pending = ask(json::json!({ "token": token, "id": 2, "cmd": "ping", "args": { "n": 3 } }).to_string());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while desk.waiting.is_empty() {
            assert!(std::time::Instant::now() < deadline, "the request never came");
            std::thread::sleep(std::time::Duration::from_millis(5));
            desk.take_new(0);
        }
        let r = desk.waiting.pop().unwrap();
        assert_eq!((r.cmd.as_str(), r.args["n"].as_i64()), ("ping", Some(3)));
        r.answer(Ok(json::json!("pong")));
        let answer = pending.join().unwrap();
        assert_eq!((answer["v"].as_u64(), answer["re"].as_u64(), answer["ok"].as_bool()), (Some(1), Some(2), Some(true)));
        assert_eq!(answer["result"], "pong");
        drop(desk);
        assert!(!socket.exists(), "the socket goes with the game");
    }
}
