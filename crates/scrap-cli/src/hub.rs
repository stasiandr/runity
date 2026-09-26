//! The projects this person works on: the list the editor's Projects
//! screen shows and `scrap projects` prints (docs/hub.md).
//!
//! The list is the person's, not a project's: `projects.ron` in the
//! settings folder, beside the editor's preferences. It keeps only paths,
//! pins and when each was opened; everything else — the name, the branch,
//! the engine the game is built against — is read from the project each
//! time, so it is never stale.
//!
//! The engine is not a version yet: while it moves this fast a project
//! names a checkout of it (`path = …` in Cargo.toml, or a git source), and
//! the hub shows that checkout's commit.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The list, in the settings folder.
pub const FILE: &str = "projects.ron";

/// Stands in for the person's settings folder when set: a test keeps its
/// list out of the real one.
pub const CONFIG_DIR_VAR: &str = "SCRAP_CONFIG_DIR";

/// Where the editor keeps what is the person's rather than a project's:
/// `~/Library/Application Support/scrap` on macOS, `%APPDATA%\scrap` on
/// Windows, `$XDG_CONFIG_HOME/scrap` or `~/.config/scrap` elsewhere — or
/// `SCRAP_CONFIG_DIR`. None with no home folder to find.
pub fn config_dir() -> Option<PathBuf> {
    let env = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = env(CONFIG_DIR_VAR) {
        return Some(dir);
    }
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    };
    base.map(|b| b.join("scrap"))
}

/// What `projects.ron` holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Known {
    pub projects: Vec<KnownProject>,
}

/// One project on the list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnownProject {
    /// The folder with `scrap.ron`, absolute.
    pub path: PathBuf,
    #[serde(default)]
    pub pinned: bool,
    /// When it was last opened, in seconds since 1970; 0 for never.
    #[serde(default)]
    pub opened: u64,
}

impl Known {
    /// The list in `dir`; empty when there is none yet. A file that does
    /// not read is an error, so that a save does not quietly drop it.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let file = dir.join(FILE);
        match std::fs::read_to_string(&file) {
            Ok(text) => scrap::ron::from_str(&text)
                .map_err(|e| format!("{}: {e}", file.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", file.display())),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let text = scrap::ron::ser::to_string_pretty(self, Default::default())
            .map_err(|e| e.to_string())?;
        let file = dir.join(FILE);
        std::fs::write(&file, text + "\n").map_err(|e| format!("{}: {e}", file.display()))
    }

    fn find(&mut self, root: &Path) -> Option<&mut KnownProject> {
        let root = canonical(root);
        self.projects.iter_mut().find(|p| canonical(&p.path) == root)
    }

    /// Puts the project on the list, or keeps it there, as opened now.
    pub fn opened(&mut self, root: &Path) {
        let now = now();
        match self.find(root) {
            Some(known) => known.opened = now,
            None => self.projects.push(KnownProject {
                path: canonical(root),
                pinned: false,
                opened: now,
            }),
        }
    }

    /// Puts the project on the list without calling it opened.
    pub fn add(&mut self, root: &Path) {
        if self.find(root).is_none() {
            self.projects.push(KnownProject {
                path: canonical(root),
                pinned: false,
                opened: 0,
            });
        }
    }

    /// Takes it off the list; the folder stays as it is. False when it was
    /// not there.
    pub fn forget(&mut self, root: &Path) -> bool {
        let root = canonical(root);
        let before = self.projects.len();
        self.projects.retain(|p| canonical(&p.path) != root);
        self.projects.len() != before
    }

    pub fn pin(&mut self, root: &Path, pinned: bool) -> bool {
        match self.find(root) {
            Some(known) => {
                known.pinned = pinned;
                true
            }
            None => false,
        }
    }
}

/// Changes the list in the settings folder: loads it, calls `change`, saves.
pub fn update(dir: &Path, change: impl FnOnce(&mut Known)) -> Result<(), String> {
    let mut known = Known::load(dir)?;
    change(&mut known);
    known.save(dir)
}

/// A project as the hub shows it, read from its folder now.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    pub path: PathBuf,
    /// From `scrap.ron`; the folder's name when it does not read.
    pub name: String,
    /// Why it cannot be opened: the folder is gone, or `scrap.ron` does
    /// not read.
    pub problem: Option<String>,
    pub pinned: bool,
    pub opened: u64,
    /// The project's own git branch, when it is in a repository.
    pub branch: Option<String>,
    pub engine: EngineRef,
    /// The scene opening the project opens: the game's start scene, else
    /// the first one.
    pub scene: Option<PathBuf>,
}

/// Which engine a project's game is built against.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct EngineRef {
    /// What Cargo.toml says: a path, as written, or a git URL.
    pub source: String,
    /// The engine's checkout on this machine: the root of the repository
    /// (or worktree) the path is in. None for a git source.
    pub checkout: Option<PathBuf>,
    /// Short commit hash: the checkout's HEAD, or the one Cargo.lock
    /// pinned for a git source.
    pub commit: Option<String>,
    /// The checkout's branch.
    pub branch: Option<String>,
    /// The checkout has changes not committed.
    pub dirty: bool,
}

impl EngineRef {
    /// `abc1234 main*` — commit, branch, a star for changes not committed.
    pub fn label(&self) -> String {
        self.label_within(usize::MAX)
    }

    /// `label`, its branch cut to `branch` characters with an ellipsis.
    pub fn label_within(&self, branch: usize) -> String {
        if self.source.is_empty() {
            return "no game crate".into();
        }
        let mut label = self.commit.clone().unwrap_or_else(|| "?".into());
        if let Some(name) = &self.branch {
            label.push(' ');
            if name.chars().count() > branch {
                label.extend(name.chars().take(branch.saturating_sub(1)));
                label.push('…');
            } else {
                label.push_str(name);
            }
        }
        if self.dirty {
            label.push('*');
        }
        label
    }
}

/// The list with each project read now: pinned first, then the most
/// recently opened.
pub fn entries(known: &Known) -> Vec<Entry> {
    let mut entries: Vec<Entry> = known
        .projects
        .iter()
        .map(|k| {
            let mut entry = describe(&k.path);
            entry.pinned = k.pinned;
            entry.opened = k.opened;
            entry
        })
        .collect();
    entries.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.opened.cmp(&a.opened))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

/// One project, read from its folder.
pub fn describe(root: &Path) -> Entry {
    let folder_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let mut entry = Entry {
        path: root.to_path_buf(),
        name: folder_name,
        problem: None,
        pinned: false,
        opened: 0,
        branch: None,
        engine: EngineRef::default(),
        scene: None,
    };
    if !root.is_dir() {
        entry.problem = Some("the folder is gone".into());
        return entry;
    }
    match scrap::Project::open(root) {
        Ok(project) => {
            entry.name = project.name().to_string();
            let start = &project.manifest().game.start_scene;
            entry.scene = project
                .scene(start)
                .filter(|p| p.is_file())
                .or_else(|| {
                    project
                        .scene_names()
                        .first()
                        .and_then(|n| project.scene(n))
                });
        }
        Err(e) => entry.problem = Some(e.to_string()),
    }
    entry.branch = branch(root);
    entry.engine = engine_of(root);
    entry
}

/// The engine the game crate in `root` depends on, from its Cargo.toml.
pub fn engine_of(root: &Path) -> EngineRef {
    let Ok(manifest) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        return EngineRef::default();
    };
    let Some(line) = manifest
        .lines()
        .find(|l| l.contains("\"scrap-engine\"") && !l.trim_start().starts_with('#'))
    else {
        return EngineRef::default();
    };
    if let Some(path) = quoted_after(line, "path") {
        let crate_dir = root.join(&path);
        let checkout = git(&crate_dir, &["rev-parse", "--show-toplevel"]).map(PathBuf::from);
        let dirty = checkout.as_ref().is_some_and(|c| {
            git(c, &["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|s| !s.is_empty())
        });
        return EngineRef {
            source: path,
            commit: git(&crate_dir, &["rev-parse", "--short", "HEAD"]),
            branch: branch(&crate_dir),
            checkout,
            dirty,
        };
    }
    let Some(url) = quoted_after(line, "git") else {
        return EngineRef::default();
    };
    let commit = locked_commit(root).or_else(|| quoted_after(line, "rev"));
    EngineRef {
        branch: quoted_after(line, "branch").or_else(|| quoted_after(line, "tag")),
        source: url,
        checkout: None,
        commit: commit.map(|c| c.chars().take(7).collect()),
        dirty: false,
    }
}

/// The commit Cargo.lock pinned the engine's git source at.
fn locked_commit(root: &Path) -> Option<String> {
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).ok()?;
    let mut in_engine = false;
    for line in lock.lines() {
        if line.starts_with("name = ") {
            in_engine = line == "name = \"scrap-engine\"";
        } else if in_engine && line.starts_with("source = \"git+") {
            return line.rsplit_once('#').map(|(_, h)| h.trim_end_matches('"').to_string());
        }
    }
    None
}

/// `path = "../runity/crates/scrap"` → `../runity/crates/scrap`.
fn quoted_after(line: &str, key: &str) -> Option<String> {
    let mut rest = line;
    while let Some(at) = rest.find(key) {
        let before = rest[..at].chars().last();
        let after = rest[at + key.len()..].trim_start();
        rest = &rest[at + key.len()..];
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            continue;
        }
        let Some(value) = after.strip_prefix('=') else { continue };
        let value = value.trim_start().strip_prefix('"')?;
        return value.split('"').next().map(str::to_string);
    }
    None
}

/// The branch checked out, None when HEAD is detached.
fn branch(dir: &Path) -> Option<String> {
    git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).filter(|b| b != "HEAD")
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Projects in `folder`: itself when it is one, else those up to two
/// levels down — what Add finds in a folder of games.
pub fn discover(folder: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
        if dir.join(scrap::project::FILE).is_file() {
            found.push(dir.to_path_buf());
            return;
        }
        if depth == 0 {
            return;
        }
        let Ok(read) = std::fs::read_dir(dir) else { return };
        let mut dirs: Vec<PathBuf> = read
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter(|p| {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                !name.starts_with('.') && name != "target" && name != "node_modules"
            })
            .collect();
        dirs.sort();
        for d in dirs {
            walk(&d, depth - 1, found);
        }
    }
    let mut found = Vec::new();
    walk(folder, 2, &mut found);
    found
}

/// What a new project starts from: an empty one with a set of modules, or
/// a copy of one of the engine's example projects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// `bare`, `basic` or `full` (`scrap::modules::SETS`).
    Set(String),
    /// An example's folder name in the engine's `examples/`.
    Template(String),
}

/// One choice on the New screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub start: Start,
    pub title: String,
    pub about: String,
}

/// What a new project can start from: the three sets, then every example
/// with a `scrap.ron` (`scrap new --template`), each said in a line.
pub fn choices() -> Vec<Choice> {
    let mut out = vec![
        Choice {
            start: Start::Set("basic".into()),
            title: "Empty".into(),
            about: "A window, the picture, input, a score on screen, sound, collisions".into(),
        },
        Choice {
            start: Start::Set("full".into()),
            title: "Empty, every module".into(),
            about: "All the official modules, to take out what the game does not need".into(),
        },
        Choice {
            start: Start::Set("bare".into()),
            title: "Empty, the core alone".into(),
            about: "No window: a server or a simulation".into(),
        },
    ];
    for name in crate::template::names() {
        let root = crate::template::examples().join(&name);
        let Ok(project) = scrap::Project::open(&root) else { continue };
        let manifest = project.manifest();
        let scenes = project.scene_names().len();
        let title = if manifest.game.title.is_empty() {
            project.name().to_string()
        } else {
            manifest.game.title.clone()
        };
        out.push(Choice {
            start: Start::Template(name.clone()),
            title,
            about: format!(
                "The engine's example `{name}`: {scenes} scene{}, {} module{}",
                if scenes == 1 { "" } else { "s" },
                manifest.modules.len(),
                if manifest.modules.len() == 1 { "" } else { "s" },
            ),
        });
    }
    out
}

/// A new project in `folder`, as `scrap new --engine-path` makes it: from
/// `start`, its game built against this checkout of the engine (or the
/// engine from git when this one has moved). Returns the scene to open.
pub fn create(folder: &Path, start: &Start) -> Result<PathBuf, String> {
    let name = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("the folder has no name")?;
    if folder.exists() {
        return Err(format!("{} is there already", folder.display()));
    }
    let engine = match this_engine() {
        Some(root) => scrap::project::Engine::Path(root.join("crates/scrap")),
        None => scrap::project::Engine::default(),
    };
    let project = match start {
        Start::Set(set) => {
            let listed = scrap::modules::set(set).ok_or_else(|| format!("no set `{set}`"))?;
            let features = scrap::modules::features(&listed, &scrap::modules::official());
            scrap::Project::create_with_modules(folder, &name, &engine, Some((&listed, &features)))
                .map_err(|e| e.to_string())?
        }
        Start::Template(template) => crate::template::create(template, folder, &name, &engine)
            .map_err(|e| format!("{e:#}"))?,
    };
    describe(project.root())
        .scene
        .ok_or_else(|| format!("{} has no scene to open", project.root().display()))
}

/// The checkout this code was built from — the engine of the editor that is
/// running — when it is still where it was built.
pub fn this_engine() -> Option<PathBuf> {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = crate_dir.parent()?.parent()?;
    root.join("crates/scrap").is_dir().then(|| canonical(root))
}

/// When, as `opened` counts it: `3 min ago`, `yesterday`, `12 Sep`.
pub fn ago(opened: u64) -> String {
    if opened == 0 {
        return "never".into();
    }
    let seconds = now().saturating_sub(opened);
    match seconds {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86_399 => format!("{} h ago", seconds / 3600),
        86_400..=172_799 => "yesterday".into(),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_is_read_from_a_path_or_a_git_line() {
        let line = r#"scrap = { package = "scrap-engine", path = "../runity/crates/scrap", default-features = false }"#;
        assert_eq!(quoted_after(line, "path").as_deref(), Some("../runity/crates/scrap"));
        assert_eq!(quoted_after(line, "git"), None);
        let line = r#"scrap = { package = "scrap-engine", git = "https://github.com/stasiandr/runity", rev = "abc" }"#;
        assert_eq!(quoted_after(line, "git").as_deref(), Some("https://github.com/stasiandr/runity"));
        assert_eq!(quoted_after(line, "rev").as_deref(), Some("abc"));
        // `features` has no `path` in it to be fooled by, but `xpath` would.
        assert_eq!(quoted_after(r#"{ xpath = "no", path = "yes" }"#, "path").as_deref(), Some("yes"));
    }

    #[test]
    fn the_list_keeps_one_entry_a_project_and_sorts_pinned_then_recent() {
        let dir = std::env::temp_dir().join(format!("scrap-hub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (a, b) = (dir.join("a"), dir.join("b"));
        for p in [&a, &b] {
            std::fs::create_dir_all(p).unwrap();
        }
        let mut known = Known::default();
        known.add(&a);
        known.opened(&b);
        known.opened(&b);
        known.add(&a);
        assert_eq!(known.projects.len(), 2);
        assert_eq!(entries(&known)[0].path, canonical(&b), "opened beats never");
        known.pin(&a, true);
        assert_eq!(entries(&known)[0].path, canonical(&a), "pinned beats opened");

        known.save(&dir).unwrap();
        assert_eq!(Known::load(&dir).unwrap(), known);
        assert!(known.forget(&a));
        assert!(!known.forget(&a));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_project_starts_from_a_set_or_from_an_example() {
        let choices = choices();
        assert_eq!(choices[0].start, Start::Set("basic".into()), "the plain one first");
        assert!(choices.iter().any(|c| c.start == Start::Template("valley".into())));

        let dir = std::env::temp_dir().join(format!("scrap-hub-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let empty = create(&dir.join("moss"), &Start::Set("basic".into())).unwrap();
        assert!(empty.is_file(), "an empty project has a scene to open");
        let copy = create(&dir.join("meadow"), &Start::Template("valley".into())).unwrap();
        assert!(copy.is_file());
        assert_eq!(describe(&dir.join("meadow")).name, "meadow", "renamed, not valley");
        assert!(create(&dir.join("moss"), &Start::Set("basic".into())).is_err(), "not over one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_gone_folder_is_listed_with_why() {
        let entry = describe(Path::new("/nowhere/at/all"));
        assert_eq!(entry.name, "all");
        assert_eq!(entry.problem.as_deref(), Some("the folder is gone"));
    }

    #[test]
    fn the_example_names_this_checkout_as_its_engine() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/valley");
        let entry = describe(&root);
        assert_eq!(entry.problem, None);
        assert_eq!(entry.name, "valley");
        assert!(entry.scene.is_some(), "valley has a start scene");
        assert_eq!(discover(&root.join("..")).len() >= 1, true);
    }
}
