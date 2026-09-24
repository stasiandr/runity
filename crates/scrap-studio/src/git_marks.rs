//! Dots for what is not committed yet: on a Project tile whose file git
//! sees as changed or new, on a Hierarchy line whose entity is not as the
//! last commit has it — unsaved edits included (DNA, postulate 2).
//!
//! Git is asked on a thread of its own every couple of seconds, so a slow
//! `git status` in a big repository never costs a frame. The open scene's
//! committed text is asked for once per commit and scene; comparing the
//! document with it is the session's (`Session::changed_since`) and runs
//! when the document changes.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use scrap::EntityId;
use scrap_editor::history;
use scrap_editor::Session;

/// How often git is asked.
const EVERY: Duration = Duration::from_secs(2);

/// What one asking found.
struct Found {
    files: HashSet<PathBuf>,
    /// The scene and commit the text is of, and the text (`None`: the scene
    /// is not in that commit). Absent when the cached one still holds.
    committed: Option<(PathBuf, String, Option<String>)>,
}

pub struct GitMarks {
    asking: Option<Receiver<Option<Found>>>,
    asked: Option<Instant>,
    /// Files not as committed, absolute.
    pub files: HashSet<PathBuf>,
    committed: Option<(PathBuf, String, Option<String>)>,
    /// Entities of the open document not as committed.
    pub entities: HashSet<EntityId>,
    /// The document revision and scene the entities were worked out for.
    compared: Option<(u64, Option<PathBuf>)>,
    /// Git answered: there is something to mark, or nothing any more.
    pub known: bool,
}

impl GitMarks {
    pub fn new() -> Self {
        Self {
            asking: None,
            asked: None,
            files: HashSet::new(),
            committed: None,
            entities: HashSet::new(),
            compared: None,
            known: false,
        }
    }

    /// Ask git again when it is time, take its answer when it has come,
    /// and compare the document when either it or the answer changed.
    /// Whether the marks changed.
    pub fn poll(&mut self, session: &Session) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.asking {
            match rx.try_recv() {
                Ok(found) => {
                    self.asking = None;
                    match found {
                        Some(found) => {
                            if found.files != self.files {
                                self.files = found.files;
                                changed = true;
                            }
                            if found.committed.is_some() {
                                self.committed = found.committed;
                                self.compared = None;
                            }
                            self.known = true;
                        }
                        // Not a repository, or no git: no dots at all.
                        None => {
                            changed |= !self.files.is_empty() || !self.entities.is_empty();
                            self.files.clear();
                            self.entities.clear();
                            self.committed = None;
                            self.known = false;
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.asking = None,
            }
        }
        let due = self.asked.is_none_or(|at| at.elapsed() >= EVERY);
        if self.asking.is_none() && due {
            if let Some(root) = session.project().map(|p| p.root().to_path_buf()) {
                self.asked = Some(Instant::now());
                let scene = session.scene_path().map(|p| p.to_path_buf());
                let cached = self
                    .committed
                    .as_ref()
                    .map(|(path, head, _)| (path.clone(), head.clone()));
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    let _ = tx.send(ask(&root, scene, cached));
                });
                self.asking = Some(rx);
            }
        }
        let now = (session.revision(), session.scene_path().map(|p| p.to_path_buf()));
        if self.known && self.compared.as_ref() != Some(&now) {
            let text = self
                .committed
                .as_ref()
                .filter(|(path, _, _)| Some(path) == now.1.as_ref())
                .map(|(_, _, text)| text.as_deref());
            // Until the committed text of this scene is known, nothing is
            // marked rather than everything.
            let entities = match text {
                Some(text) => session.changed_since(text),
                None => HashSet::new(),
            };
            if entities != self.entities {
                self.entities = entities;
                changed = true;
            }
            self.compared = Some(now);
        }
        changed
    }
}

/// One asking, off the frame: `None` outside a repository.
fn ask(
    root: &std::path::Path,
    scene: Option<PathBuf>,
    cached: Option<(PathBuf, String)>,
) -> Option<Found> {
    let files = history::uncommitted(root).ok()?;
    let head = history::head(root).unwrap_or_default();
    let committed = scene.and_then(|scene| {
        if cached.as_ref() == Some(&(scene.clone(), head.clone())) {
            return None;
        }
        let text = history::show(&scene, "HEAD").ok();
        Some((scene, head, text))
    });
    Some(Found { files, committed })
}
