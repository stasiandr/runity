//! The live studio, for an agent: the window a person has open serves the
//! same protocol as `scrap-mcp`, so an agent edits what they see.
//!
//! A studio with a project open listens on a Unix socket in the editor's
//! own folder (`scrap_cli::hub::config_dir`, `~/Library/Application
//! Support/scrap` on macOS): `studio-<key>.sock`, where the key is a hash of
//! the project's root, only its owner may open (0600), and beside it
//! `studio-<key>.json` says which project and which process. Messages come
//! in a line each, as over stdio, on a thread per connection; each is handed
//! to the studio's own frame loop ([`Listener::serve`]) and answered there,
//! on the session the person sees — the same document, undo history, Game
//! view and Console — and the reply goes back on the connection's thread.
//!
//! `scrap-mcp` looks here first ([`find`]): when a studio serves the project
//! it was asked about, it passes every line on ([`Attached`]); otherwise it
//! is its own headless editor, as before.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// A studio serving a project, as its registry file says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Studio {
    pub project: PathBuf,
    pub pid: u32,
    pub socket: PathBuf,
}

/// The key of a project in file names: FNV-1a of its canonical root, so
/// the studio and every `scrap-mcp` build agree on it (std's hasher is not
/// promised to be the same across builds).
pub fn key(project: &Path) -> String {
    let root = canonical(project);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in root.to_string_lossy().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Where the studio on `project` listens.
pub fn socket_path(dir: &Path, project: &Path) -> PathBuf {
    dir.join(format!("studio-{}.sock", key(project)))
}

/// Where it says which project and process it is.
pub fn registry_path(dir: &Path, project: &Path) -> PathBuf {
    dir.join(format!("studio-{}.json", key(project)))
}

/// The studios serving a project now. A registry whose socket nobody
/// answers on is a studio that went without tidying up: removed.
#[cfg(unix)]
pub fn studios(dir: &Path) -> Vec<Studio> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !(name.starts_with("studio-") && name.ends_with(".json")) {
            continue;
        }
        let Some(studio) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|v| {
                Some(Studio {
                    project: PathBuf::from(v.get("project")?.as_str()?),
                    pid: v.get("pid")?.as_u64()? as u32,
                    socket: PathBuf::from(v.get("socket")?.as_str()?),
                })
            })
        else {
            continue;
        };
        if std::os::unix::net::UnixStream::connect(&studio.socket).is_ok() {
            out.push(studio);
        } else {
            let _ = std::fs::remove_file(&studio.socket);
            let _ = std::fs::remove_file(&path);
        }
    }
    out.sort_by(|a, b| a.project.cmp(&b.project));
    out
}

/// The studio to attach to: the one on `project` when it is named;
/// otherwise the one on the project `cwd` is in, or the only one with its
/// project under `cwd` (an agent started in a repository whose game is in
/// a folder of it). `Err` says why none, in words for the `attached` tool.
#[cfg(unix)]
pub fn find(dir: &Path, project: Option<&Path>, cwd: &Path) -> Result<Studio, String> {
    let open = studios(dir);
    let listed = || {
        open.iter()
            .map(|s| s.project.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let none = |about: String| {
        if open.is_empty() {
            format!("no scrap studio is open ({about})")
        } else {
            format!(
                "no scrap studio serves {about}; open ones serve: {}",
                listed()
            )
        }
    };
    if let Some(project) = project {
        let root = scrap::Project::find(project)
            .map(|p| p.root().to_path_buf())
            .unwrap_or_else(|_| project.to_path_buf());
        let root = canonical(&root);
        return open
            .iter()
            .find(|s| canonical(&s.project) == root)
            .cloned()
            .ok_or_else(|| none(root.display().to_string()));
    }
    let cwd = canonical(cwd);
    if let Ok(found) = scrap::Project::find(&cwd) {
        let root = canonical(found.root());
        if let Some(studio) = open.iter().find(|s| canonical(&s.project) == root) {
            return Ok(studio.clone());
        }
    }
    let under: Vec<&Studio> = open
        .iter()
        .filter(|s| canonical(&s.project).starts_with(&cwd))
        .collect();
    match under.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(none(format!("a project at or under {}", cwd.display()))),
        _ => Err(format!(
            "several scrap studios serve projects under {}: {}; start scrap-mcp with --project PATH",
            cwd.display(),
            listed()
        )),
    }
}

/// `scrap-mcp`'s end of a studio: each line goes on, and a request's reply
/// comes back.
#[cfg(unix)]
pub struct Attached {
    pub studio: Studio,
    writer: std::os::unix::net::UnixStream,
    reader: BufReader<std::os::unix::net::UnixStream>,
}

#[cfg(unix)]
impl Attached {
    pub fn connect(studio: Studio) -> std::io::Result<Self> {
        let writer = std::os::unix::net::UnixStream::connect(&studio.socket)?;
        let reader = BufReader::new(writer.try_clone()?);
        Ok(Self {
            studio,
            writer,
            reader,
        })
    }

    /// Pass one line on; the reply's line when it is a request (it has an
    /// `id`), `None` for a notification, which gets none.
    pub fn exchange(&mut self, line: &str) -> std::io::Result<Option<String>> {
        let request = serde_json::from_str::<Value>(line)
            .map(|v| v.get("id").is_some())
            .unwrap_or(true);
        writeln!(self.writer, "{}", line.trim())?;
        self.writer.flush()?;
        if !request {
            return Ok(None);
        }
        let mut reply = String::new();
        if self.reader.read_line(&mut reply)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "the studio closed",
            ));
        }
        Ok(Some(reply.trim_end().to_string()))
    }
}

/// A message from a connection, waiting for the studio's frame loop, and
/// where its reply goes.
#[cfg(unix)]
struct Request {
    message: Value,
    reply: std::sync::mpsc::Sender<Option<Value>>,
}

/// The studio's end: the socket, the thread accepting on it, and the
/// messages waiting to be answered on the studio's own session.
#[cfg(unix)]
pub struct Listener {
    project: PathBuf,
    socket: PathBuf,
    registry: PathBuf,
    requests: std::sync::mpsc::Receiver<Request>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Agents connected now.
    connected: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    server: crate::Server,
}

#[cfg(unix)]
impl Listener {
    /// Listen for agents on `project`'s socket in `dir`. `wake` is called
    /// from a connection's thread when a message arrives, to bring the
    /// frame loop round to [`Listener::serve`] it. `Err` when another live
    /// studio already serves the project, or the socket cannot be made.
    pub fn open(
        dir: &Path,
        project: &Path,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::{UnixListener, UnixStream};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let project = canonical(project);
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let socket = socket_path(dir, &project);
        let registry = registry_path(dir, &project);
        if socket.exists() {
            if UnixStream::connect(&socket).is_ok() {
                return Err(format!(
                    "another scrap studio already serves agents on {}",
                    project.display()
                ));
            }
            // A studio that ended without tidying up.
            let _ = std::fs::remove_file(&socket);
        }
        let listener = UnixListener::bind(&socket)
            .map_err(|e| format!("cannot listen on {}: {e}", socket.display()))?;
        // Only this user may talk to the editor.
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("{}: {e}", socket.display()))?;
        let entry = json!({
            "project": project.to_string_lossy(),
            "pid": std::process::id(),
            "socket": socket.to_string_lossy(),
        });
        std::fs::write(&registry, format!("{entry:#}\n"))
            .map_err(|e| format!("{}: {e}", registry.display()))?;
        let _ = std::fs::set_permissions(&registry, std::fs::Permissions::from_mode(0o600));

        let (tx, requests) = std::sync::mpsc::channel::<Request>();
        let stop = Arc::new(AtomicBool::new(false));
        let connected = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let stopping = stop.clone();
        let counting = connected.clone();
        std::thread::Builder::new()
            .name("scrap-studio-agents".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if stopping.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let tx = tx.clone();
                    let wake = wake.clone();
                    let counting = counting.clone();
                    let _ = std::thread::Builder::new()
                        .name("scrap-studio-agent".into())
                        .spawn(move || {
                            counting.fetch_add(1, Ordering::SeqCst);
                            // The studio hears of the agent before its first message.
                            wake();
                            converse(stream, tx, wake.clone());
                            counting.fetch_sub(1, Ordering::SeqCst);
                            wake();
                        });
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            server: crate::Server::live(&project),
            project,
            socket,
            registry,
            requests,
            stop,
            connected,
        })
    }

    /// Answer every message waiting, on `session` — the studio's own. On
    /// the thread that owns it, in its frame loop. How many were answered.
    pub fn serve(&mut self, session: &mut scrap_editor::Session) -> usize {
        let mut served = 0;
        while let Ok(request) = self.requests.try_recv() {
            let reply = self.server.handle_on(session, &request.message);
            let _ = request.reply.send(reply);
            served += 1;
        }
        served
    }

    /// How many agents are connected now.
    pub fn connections(&self) -> usize {
        self.connected.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The project this studio serves agents on.
    pub fn project(&self) -> &Path {
        &self.project
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

#[cfg(unix)]
impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        // Brings the accepting thread round to see it is stopped.
        let _ = std::os::unix::net::UnixStream::connect(&self.socket);
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_file(&self.registry);
    }
}

/// One agent's connection: a line in, handed to the frame loop, its reply
/// out — in order, one at a time, as over stdio.
#[cfg(unix)]
fn converse(
    stream: std::os::unix::net::UnixStream,
    to_studio: std::sync::mpsc::Sender<Request>,
    wake: std::sync::Arc<dyn Fn() + Send + Sync>,
) {
    let Ok(reading) = stream.try_clone() else {
        return;
    };
    let mut writer = stream;
    for line in BufReader::new(reading).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => {
                let (tx, rx) = std::sync::mpsc::channel();
                if to_studio.send(Request { message, reply: tx }).is_err() {
                    break;
                }
                wake();
                match rx.recv() {
                    Ok(reply) => reply,
                    // The studio closed the project, or itself.
                    Err(_) => break,
                }
            }
            Err(e) => Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("not JSON: {e}") },
            })),
        };
        if let Some(reply) = reply {
            if writeln!(writer, "{reply}")
                .and_then(|_| writer.flush())
                .is_err()
            {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_s_key_is_its_root_whichever_way_it_is_named() {
        let dir = std::env::temp_dir().join(format!("scrap-live-key-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("game")).unwrap();
        assert_eq!(key(&dir.join("game")), key(&dir.join("game/../game")));
        assert_ne!(key(&dir.join("game")), key(&dir));
        assert_eq!(key(&dir).len(), 16);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
