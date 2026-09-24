//! The live link with Blender (docs/blender.md).
//!
//! While a project is open, the editor listens on a port of this machine
//! and writes the number into `library/blender-link` — derived, like the
//! rest of the library. The scrap plugin in an open Blender finds the
//! project above its `.blend`, reads the port, and sends:
//!
//! * **on every save**, the whole scene, in the stream the importer reads
//!   from a background Blender — so the saved file is imported without
//!   starting a second Blender, a second or so sooner;
//! * **while something is being moved**, the placements of the objects
//!   that moved, which the editor applies to the file's prefabs in memory:
//!   a preview. The truth is still the `.blend`, and the next save brings
//!   it in, as stopping play brings the scene back.
//!
//! A message is `SCRAP_LK`, a `u32` kind (1 a scene, 2 placements), a
//! `u32` length and the source's project-relative path, then the payload.
//! One message per connection. Reading and importing happen on the
//! listener's thread; the frame only picks up what they did.

use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use scrap::{EntityId, Transform};
use serde_json::Value;

/// The file in the library that holds the port.
pub const PORT_FILE: &str = "blender-link";

const MAGIC: &[u8; 8] = b"SCRAP_LK";

/// What came from Blender.
pub(crate) enum Message {
    /// A saved `.blend`, imported.
    Imported {
        source: PathBuf,
        result: Result<scrap::AssetId, String>,
    },
    /// Objects of a `.blend` moved, not yet saved.
    Moved(Vec<(EntityId, Transform)>),
}

/// A listening link, for one project.
pub(crate) struct Link {
    pub project: scrap::Project,
    pub port: u16,
    messages: Receiver<Message>,
    stop: Arc<AtomicBool>,
    /// When the port file was last seen to hold this link's port.
    checked: Instant,
}

impl Link {
    pub fn start(project: scrap::Project) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let library = project.library();
        std::fs::create_dir_all(&library)?;
        std::fs::write(library.join(PORT_FILE), format!("{port}\n"))?;
        let (send, messages) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let root = project.clone();
        std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(15));
                    continue;
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
                let mut bytes = Vec::new();
                if stream.read_to_end(&mut bytes).is_err() {
                    continue;
                }
                if let Some(message) = handle(&root, &bytes) {
                    if send.send(message).is_err() {
                        break;
                    }
                }
            }
        });
        Ok(Self {
            project,
            port,
            messages,
            stop,
            checked: Instant::now(),
        })
    }

    /// What arrived since the last call.
    pub fn drain(&self) -> Vec<Message> {
        self.messages.try_iter().collect()
    }

    /// Put the port back where the plugin looks, if it is gone: another
    /// editor on the same project — a second window, a screenshot tool —
    /// writes its own and takes it away when it closes, and Blender would
    /// no longer find this one. Once a second, not every frame.
    pub fn keep_port(&mut self) {
        if self.checked.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.checked = Instant::now();
        let file = self.project.library().join(PORT_FILE);
        if !file.is_file() {
            let _ = std::fs::write(file, format!("{}\n", self.port));
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The port file, if it is still ours: another editor on the same
        // project may have written its own since.
        let file = self.project.library().join(PORT_FILE);
        if std::fs::read_to_string(&file).is_ok_and(|t| t.trim() == self.port.to_string()) {
            let _ = std::fs::remove_file(file);
        }
    }
}

fn handle(project: &scrap::Project, bytes: &[u8]) -> Option<Message> {
    if bytes.len() < 16 || &bytes[..8] != MAGIC {
        return None;
    }
    let kind = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    let length = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
    let path = std::str::from_utf8(bytes.get(16..16 + length)?).ok()?;
    let payload = &bytes[16 + length..];
    let source = project.resolve(path);
    match kind {
        1 => Some(Message::Imported {
            result: scrap_import::blend::import_stream(project, &source, payload)
                .map_err(|e| format!("{e:#}")),
            source,
        }),
        2 => {
            let moves: Value = serde_json::from_slice(payload).ok()?;
            let scale = scrap_import::ImportSettings::load(scrap_import::sidecar_for(&source))
                .map_or(1.0, |s| s.scale);
            let numbers = |v: &Value, key: &str| -> Option<Vec<f32>> {
                v.get(key)?
                    .as_array()?
                    .iter()
                    .map(|n| n.as_f64().map(|n| n as f32))
                    .collect()
            };
            Some(Message::Moved(
                moves
                    .get("moves")?
                    .as_array()?
                    .iter()
                    .filter_map(|m| {
                        let id: EntityId = m.get("id")?.as_str()?.parse().ok()?;
                        let l = numbers(m, "location")?;
                        let r = numbers(m, "rotation")?;
                        let s = numbers(m, "scale")?;
                        let mut t = scrap_import::blend::placement(
                            [*l.first()?, *l.get(1)?, *l.get(2)?],
                            [*r.first()?, *r.get(1)?, *r.get(2)?, *r.get(3)?],
                            [*s.first()?, *s.get(1)?, *s.get(2)?],
                        );
                        t.position *= scale;
                        Some((id, t))
                    })
                    .collect(),
            ))
        }
        _ => None,
    }
}
