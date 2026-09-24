//! Draft models from a neural network, for prototyping.
//!
//! Words or a picture go to a [`Provider`]; a mesh comes back and becomes an
//! ordinary asset in `assets/drafts/`, and — when the request names a
//! greybox entity — takes that entity's place: the cube's box is the
//! brief, so the well comes out the size of the cube it replaces, standing
//! where it stood. One undo step, as any edit.
//!
//! **A draft is a draft.** Today's 3D networks are good enough to make a
//! greybox level read as a place and not good enough to ship. So what they
//! make is marked by where it lives: `assets/drafts/<name>.glb`, with
//! `<name>.draft.ron` beside it saying what made it. Scenes name models by
//! file stem, so the real model later takes the draft's name and every line
//! that used the draft uses it — nothing to re-point. [`drafts`] lists what
//! is still waiting for that.
//!
//! Generation takes tens of seconds and never blocks the editor: a
//! [`Generator`] runs each request on its own thread, and [`Generator::poll`],
//! called from the editor's loop (or before each tool call, in the MCP
//! server), brings finished ones into the session.

mod color;
pub mod mcp;
pub mod provider;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[allow(unused_imports)]
use runity::prelude::*;
use runity::glam::Vec3;
use runity::scene::{Collider, MaterialRef};
use runity::{EntityId, Material};
use runity_editor::console::Level;
use runity_editor::Session;
use serde::{Deserialize, Serialize};

pub use provider::{secret, Fal, Input, Output, Provider};

/// Where drafts live, under the project's `assets/`.
pub const DRAFTS: &str = "drafts";

/// What to make.
#[derive(Debug, Clone)]
pub struct Request {
    /// The model's name: its file stem, so what scenes will call it.
    pub name: String,
    pub input: Input,
    /// Words that describe the input, kept with the draft: the prompt, or
    /// the picture's path.
    pub about: String,
    /// A greybox entity to replace: the mesh is fitted into its box and put
    /// on it as its model.
    pub fit: Option<EntityId>,
}

impl Request {
    /// A request from words.
    pub fn text(name: impl Into<String>, prompt: impl Into<String>) -> Self {
        let prompt = prompt.into();
        Self {
            name: name.into(),
            about: prompt.clone(),
            input: Input::Text(prompt),
            fit: None,
        }
    }

    /// A request from a picture on disk.
    pub fn image(name: impl Into<String>, path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let mime = match path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .as_deref()
        {
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            _ => anyhow::bail!("{}: a picture is .png, .jpg or .webp", path.display()),
        };
        Ok(Self {
            name: name.into(),
            about: path.display().to_string(),
            input: Input::Image {
                bytes,
                mime: mime.into(),
            },
            fit: None,
        })
    }

    pub fn fit(mut self, id: EntityId) -> Self {
        self.fit = Some(id);
        self
    }
}

/// What made a draft: `assets/drafts/<name>.draft.ron`. Text, so a diff
/// says "the well was regenerated from a different prompt" and not only
/// "a binary changed".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftRecord {
    /// The prompt, or the picture it was made from.
    pub from: String,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// The colour the draft is painted, `#rrggbb`: the average of the
    /// texture it came with.
    pub color: String,
}

/// A draft that is in the project.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub name: String,
    pub file: PathBuf,
    pub record: Option<DraftRecord>,
}

/// Every draft in a project: what a release still has to replace.
pub fn drafts(project: &runity::Project) -> Vec<Draft> {
    let dir = project.assets().join(DRAFTS);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<Draft> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "glb"))
        .map(|file| {
            let name = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let record = std::fs::read_to_string(dir.join(format!("{name}.draft.ron")))
                .ok()
                .and_then(|text| ron::from_str(&text).ok());
            Draft { name, file, record }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A request on its way.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub stage: String,
    pub started: Instant,
    pub fit: Option<EntityId>,
    about: String,
}

/// What became of a request.
#[derive(Debug, Clone)]
pub struct Finished {
    pub id: u64,
    pub name: String,
    pub seconds: f32,
    pub result: Result<Applied, String>,
}

/// A draft that arrived.
#[derive(Debug, Clone)]
pub struct Applied {
    pub file: PathBuf,
    pub color: [u8; 3],
    pub size: Vec3,
    /// The entity it replaced, if it was asked to.
    pub fitted: Option<EntityId>,
}

enum Message {
    Stage(u64, String),
    Done(u64, anyhow::Result<Output>),
}

/// Requests running in the background, and what finished.
pub struct Generator {
    provider: Arc<dyn Provider>,
    next: u64,
    running: BTreeMap<u64, Job>,
    finished: Vec<Finished>,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
}

impl Generator {
    pub fn new(provider: impl Provider + 'static) -> Self {
        let (sender, receiver) = channel();
        Self {
            provider: Arc::new(provider),
            next: 1,
            running: BTreeMap::new(),
            finished: Vec::new(),
            sender,
            receiver,
        }
    }

    /// The default provider, from the environment.
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self::new(Fal::from_env()?))
    }

    /// Start a request. Checked now — a bad name or a missing entity is
    /// said before anything is paid for — and run on its own thread.
    pub fn start(&mut self, session: &mut Session, request: Request) -> Result<u64, String> {
        let project = session
            .project()
            .ok_or("open a scene in a project first: a draft is a file in its assets/")?;
        runity::project::valid_name(&request.name)?;
        if let Some(other) = taken(project, &request.name) {
            return Err(format!(
                "`{}` is already {} — a draft would shadow it; pick another name",
                request.name,
                other.display()
            ));
        }
        if let Some(id) = request.fit {
            let desc = session
                .scene()
                .get(id)
                .ok_or_else(|| format!("no entity {id} in the document"))?;
            if !desc.prefab.is_empty() {
                return Err(format!(
                    "{id} is an instance of `{}`; a draft replaces a greybox model, not a prefab",
                    desc.prefab
                ));
            }
        }
        if self.running.values().any(|j| j.name == request.name) {
            return Err(format!("`{}` is already being generated", request.name));
        }

        let id = self.next;
        self.next += 1;
        self.running.insert(
            id,
            Job {
                id,
                name: request.name.clone(),
                stage: "starting".into(),
                started: Instant::now(),
                fit: request.fit,
                about: request.about.clone(),
            },
        );
        session.say(
            Level::Info,
            format!("generating `{}` from {:?}", request.name, request.about),
        );
        let provider = Arc::clone(&self.provider);
        let sender = self.sender.clone();
        let input = request.input;
        std::thread::spawn(move || {
            let mut stage = |line: String| {
                let _ = sender.send(Message::Stage(id, line));
            };
            let result = provider.generate(&input, &mut stage);
            let _ = sender.send(Message::Done(id, result));
        });
        Ok(id)
    }

    /// Bring what finished into the session: the file into `assets/drafts/`,
    /// the asset into the library, the entity onto its new model. What
    /// finished is returned, and kept for [`Generator::finished`].
    pub fn poll(&mut self, session: &mut Session) -> Vec<Finished> {
        let mut done = Vec::new();
        while let Ok(message) = self.receiver.try_recv() {
            match message {
                Message::Stage(id, line) => {
                    if let Some(job) = self.running.get_mut(&id) {
                        job.stage = line;
                    }
                }
                Message::Done(id, result) => {
                    let Some(job) = self.running.remove(&id) else {
                        continue;
                    };
                    let seconds = job.started.elapsed().as_secs_f32();
                    let result = result
                        .map_err(|e| format!("{e:#}"))
                        .and_then(|output| apply(session, &job, self.provider.name(), output));
                    match &result {
                        Ok(applied) => session.say(
                            Level::Info,
                            format!(
                                "`{}` is ready in {seconds:.0} s: {:.1} × {:.1} × {:.1} m{}",
                                job.name,
                                applied.size.x,
                                applied.size.y,
                                applied.size.z,
                                applied
                                    .fitted
                                    .map(|id| format!(", in place of {id}"))
                                    .unwrap_or_default()
                            ),
                        ),
                        Err(e) => session.say(Level::Error, format!("`{}`: {e}", job.name)),
                    }
                    done.push(Finished {
                        id,
                        name: job.name,
                        seconds,
                        result,
                    });
                }
            }
        }
        self.finished.extend(done.iter().cloned());
        done
    }

    /// Wait for one request, bringing in whatever finishes meanwhile.
    pub fn wait(&mut self, session: &mut Session, id: u64, timeout: Duration) -> Option<Finished> {
        let until = Instant::now() + timeout;
        loop {
            self.poll(session);
            if let Some(f) = self.finished.iter().rev().find(|f| f.id == id) {
                return Some(f.clone());
            }
            if Instant::now() > until || !self.running.contains_key(&id) {
                return None;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn running(&self) -> Vec<Job> {
        self.running.values().cloned().collect()
    }

    pub fn finished(&self) -> &[Finished] {
        &self.finished
    }
}

/// A file outside `drafts/` with this stem: a real model, which a draft
/// must not shadow.
fn taken(project: &runity::Project, name: &str) -> Option<PathBuf> {
    let drafts = project.assets().join(DRAFTS);
    let mut found = None;
    walk(&project.assets(), &mut |path| {
        let same = path.file_stem().is_some_and(|s| s == name)
            && !path.starts_with(&drafts)
            && path.extension().is_some_and(|e| e != "rimport");
        if same && found.is_none() {
            found = Some(path.to_path_buf());
        }
    });
    found
}

fn walk(dir: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, visit);
        } else {
            visit(&path);
        }
    }
}

/// A finished mesh into the project, and onto the entity it replaces.
fn apply(
    session: &mut Session,
    job: &Job,
    provider: &str,
    output: Output,
) -> Result<Applied, String> {
    let project = session
        .project()
        .ok_or("the project was closed while generating")?
        .clone();
    let dir = project.assets().join(DRAFTS);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(format!("{}.glb", job.name));

    // A draft generated again keeps the last one in the cache, not in git:
    // the variant nobody picked is not the project's.
    let cache = project.root().join(".runity").join("gen");
    let _ = std::fs::create_dir_all(&cache);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if file.exists() {
        let _ = std::fs::copy(&file, cache.join(format!("{}-{stamp}.glb", job.name)));
    }
    if let Some(picture) = &output.picture {
        let _ = std::fs::write(cache.join(format!("{}-{stamp}.png", job.name)), picture);
    }

    let color = color::average(&output.glb).map_err(|e| format!("{e:#}"))?;
    std::fs::write(&file, &output.glb).map_err(|e| format!("{}: {e}", file.display()))?;
    let record = DraftRecord {
        from: job.about.clone(),
        provider: provider.into(),
        model: output.model.clone(),
        seed: output.seed,
        color: format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]),
    };
    let text = ron::ser::to_string_pretty(&record, ron::ser::PrettyConfig::default())
        .map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{}.draft.ron", job.name)), text + "\n")
        .map_err(|e| e.to_string())?;

    session.import(&file).map_err(|e| e.to_string())?;
    let (low, high) = session.model_bounds(&job.name).ok_or_else(|| {
        format!(
            "`{}` imported, but the library has no mesh by that name",
            job.name
        )
    })?;

    let mut applied = Applied {
        file,
        color,
        size: high - low,
        fitted: None,
    };
    if let Some(id) = job.fit {
        applied.size = fit(session, id, &job.name, (low, high), color)?;
        applied.fitted = Some(id);
    }
    Ok(applied)
}

/// Put `model` on the entity in place of its greybox: scaled evenly to fit
/// inside the box the entity drew, standing on that box's floor, in its
/// colour. One undo step. Returns the size it came out, in metres before
/// any parent's scale.
///
/// In the entity's own frame, so a turned cube gets a turned well: the
/// floor's middle of the old box, `scale ⊙ c`, and of the new, `s · m`, are
/// made to meet by moving the position along the entity's rotation.
fn fit(
    session: &mut Session,
    id: EntityId,
    model: &str,
    (low, high): (Vec3, Vec3),
    color: [u8; 3],
) -> Result<Vec3, String> {
    let desc = session
        .scene()
        .get(id)
        .ok_or_else(|| format!("{id} is gone from the document; `{model}` is in the project"))?;
    let old = desc.transform;
    let (a, b) = session
        .model_bounds(&desc.model())
        .unwrap_or((Vec3::splat(-0.5), Vec3::splat(0.5)));
    let target = (b - a) * old.scale.abs();
    let size = (high - low).max(Vec3::splat(1e-4));
    let s = (target / size).min_element();
    let floor = |lo: Vec3, hi: Vec3| Vec3::new((lo.x + hi.x) * 0.5, lo.y, (lo.z + hi.z) * 0.5);
    let position = old.position + old.rotation() * (old.scale * floor(a, b) - s * floor(low, high));
    let had_collider = desc.collider() != Collider::None;
    let model = model.to_string();
    session
        .update(id, move |d| {
            d.set_model(model.as_str());
            d.transform.position = position;
            d.transform.scale = Vec3::splat(s);
            d.set_material(MaterialRef::Inline(Material::from_srgb(color[0], color[1], color[2])));
            if had_collider {
                d.set_part(&Collider::Box {
                    half: (high - low) * 0.5,
                    center: (high + low) * 0.5,
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(size * s)
}
