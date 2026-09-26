//! A game project: one folder, with a few things fixed at its root and
//! the rest wherever a person put it (docs/layout.md).
//!
//! DNA, postulate 7: one standard — how a file says what it is (its
//! extension, [`crate::layout`]), how it is named (an ID and a name), and
//! what lies at the root — so that neither a person nor an agent has to
//! guess, and every tool finds the same things. Where the rest lies is the
//! project's: a tomato's model, material and prefab together, code by
//! feature. What `scrap new` writes is the default, Unreal's scheme:
//!
//! ```text
//! <project>/
//!   scrap.ron        this file says it is a project: name, engine version
//!   config/          the game's settings as a whole          (Unreal Config/)
//!     input.ron      actions by name, and their keys
//!     layers.ron     collision layers
//!   content/         what artists and designers make         (Unreal Content/)
//!     <name>/        the project's own folder (Allar 2.2)
//!       maps/        levels: *.scene.ron (2.4)
//!       core/        what everything else stands on (2.5)
//!       <feature>/   model, texture, material, prefab together
//!       ui/screens/  *.screen.ron
//!     localization/  the game's words, one file per language
//!     developers/    sandboxes: read, never shipped (2.3)
//!   library/         built .scrasset — derived, never committed
//!   Cargo.toml       the game crate, its own workspace
//!   build.rs         finds the components and systems in src/; not edited
//!   src/main.rs      the game: a window on its first scene, reloading live
//!   src/<feature>/   code by feature: a component is a file named for it
//!   CLAUDE.md        what an agent needs to work here
//! ```
//!
//! A component is a file rather than a line in a list, because two people
//! each adding one should not both edit the same line: git merges two new
//! files cleanly and two lines appended at one place as a conflict. The
//! file's name is the name a scene uses (`cooking/pot.rs` is `"pot"`,
//! struct `Pot`), and `build.rs` writes the registration — so there is no
//! list to forget either. Systems are files for the same reason, but the
//! order they run in is one place on purpose: two people changing it should
//! see each other's change.
//!
//! A project laid out before (`scenes/`, `prefabs/`, `input.ron` at the
//! root) still opens, until 2026-10-31; see [`crate::layout`].
//!
//! The game is its own crate, at the root, and its own workspace: editing
//! it rebuilds the game and never the engine (postulate 1).

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file that makes a folder a project.
pub const FILE: &str = "scrap.ron";

/// Where a window opens and how big, as `x,y,width,height` in logical
/// pixels: set for each player's window when several play from the editor
/// (`scrap run --players N`), so they lie side by side rather than on top
/// of each other. The desktop shell reads it; it overrides the size the
/// game asked for.
pub const WINDOW_VAR: &str = "SCRAP_WINDOW";
/// The game's settings as a whole: input, layers (Unreal's `Config/`).
pub const CONFIG: &str = "config";
/// Everything artists and designers make (Unreal's `Content/`). Where in
/// it is theirs: the engine finds a file by its kind ([`crate::layout`]).
pub const CONTENT: &str = "content";
/// Where levels go by default, in the project's own folder in
/// [`CONTENT`] (Allar 2.4).
pub const MAPS: &str = "maps";
/// Where built assets go. Derived from everything above, and never
/// committed: a clone builds it.
pub const LIBRARY: &str = "library";
/// Where the game's code lives: by feature, a component or a system a file
/// anywhere in it.
pub const SRC: &str = "src";
/// What the player does, by name, and which keys that is.
pub const INPUT: &str = "config/input.ron";
/// The game's numbers, as RON a designer turns while it runs: see
/// [`crate::Tuned`]. Tables and tuned files lie with the feature that
/// reads them; `configs/` is only where a project laid out before put them.
pub const CONFIGS: &str = "configs";

/// Where a project laid out before 2026-09-26 kept each thing: at the root,
/// a folder per kind. Read until 2026-10-31 ([`crate::layout`]).
pub mod legacy {
    pub const SCENES: &str = "scenes";
    pub const PREFABS: &str = "prefabs";
    pub const MATERIALS: &str = "materials";
    pub const ASSETS: &str = "assets";
    pub const UI: &str = "ui";
    pub const ANIMATORS: &str = "animators";
    pub const SHADERS: &str = "shaders";
    pub const INPUT: &str = "input.ron";
    pub const COMPONENTS: &str = "src/components";
    pub const SYSTEMS: &str = "src/systems";
}

/// What the game's components look like, written by the game from its own
/// types: what the editor's Inspector and `scrap check` know them by
/// without linking the game.
pub const SHAPES: &str = "library/components.ron";

/// What the game reads each file of `tuning/` as, written beside
/// [`SHAPES`] by the same call: the columns of the editor's Table and what
/// `scrap check` holds the files to.
pub const TUNING_SHAPES: &str = "library/tuning.ron";
/// [`TUNING_SHAPES`]'s file name, beside [`SHAPES`].
pub const TUNING_SHAPES_FILE: &str = "tuning.ron";
/// What the game's tables hold, written by the game from its own types:
/// what the Configs window and `scrap check` know them by
/// (`scrap_core::table::TableShape`).
pub const TABLE_SHAPES: &str = "library/tables.ron";

/// Where a built game keeps its project data, beside the executable.
pub const DATA: &str = "data";

/// A file of the game's project, found the way a game has to find it: in a
/// build, in `data/` beside the executable (what `scrap build` makes); in
/// development, in the project the game crate sits in — `dev_root` is the
/// crate's `env!("CARGO_MANIFEST_DIR")`.
///
/// The build is looked for first, so a shipped game never reaches for a
/// path that only existed on the machine it was compiled on.
pub fn data_file(dev_root: &str, relative: &str) -> PathBuf {
    if let Some(dir) = DATA_DIR.get() {
        return dir.join(relative);
    }
    let shipped = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(DATA).join(relative)))
        .filter(|path| path.exists());
    shipped.unwrap_or_else(|| Path::new(dev_root).join(relative))
}

/// The folder a game's data is in: its project in development, `data/`
/// beside it in a build ([`data_file`]).
pub fn data_root(dev_root: &str) -> PathBuf {
    data_file(dev_root, "")
}

/// The scene called `name` in a game's data, wherever it lies
/// ([`crate::layout`]): what `scrap run --scene cave` and the start scene
/// open. A path from the root (`content/game/maps/cave`) is taken as it is.
pub fn data_scene(dev_root: &str, name: &str) -> PathBuf {
    let root = data_root(dev_root);
    crate::layout::find(&root, crate::layout::Kind::Scene, name)
        .unwrap_or_else(|| root.join(format!("{name}{}", crate::layout::Kind::Scene.extension())))
}

/// Where the host put the project's data, when it is not beside the
/// executable: on Android, the app's own files, unpacked from the APK.
static DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Tell [`data_file`] where the data is, once, before the game reads any:
/// a host whose executable has no folder of its own (Android's).
pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

/// Where a new project's game crate gets the engine from.
#[derive(Debug, Clone, PartialEq)]
pub enum Engine {
    /// A git repository — the default, until the engine is published.
    Git(String),
    /// A checkout on this machine: the `crates/scrap` folder. Written into
    /// `Cargo.toml` relative to the project when it can be, so the project
    /// and the engine can move together.
    Path(PathBuf),
}

impl Engine {
    /// Where a game crate in `root` gets the engine, as `Cargo.toml` says
    /// it: `git = "…"` or `path = "…"`, relative when it can be.
    pub fn cargo_source(&self, root: &Path) -> String {
        match self {
            Engine::Git(url) => format!("git = \"{url}\""),
            Engine::Path(path) => {
                let path = relative_path(root, path)
                    .unwrap_or_else(|| path.to_string_lossy().replace('\\', "/"));
                format!("path = \"{path}\"")
            }
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Engine::Git("https://github.com/stasiandr/runity".into())
    }
}

/// What `scrap.ron` holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    /// The engine version the project was made with. Recorded now so that
    /// the day a format changes, there is something to migrate from.
    #[serde(default)]
    pub engine: String,
    /// How the game starts: Unity's Player and Build Settings, as a few
    /// lines anyone can read in a diff.
    #[serde(default)]
    pub game: GameSettings,
    /// The project's modules by name (docs/modules.md): the one list of
    /// what the game is built with. The engine's cargo features in the
    /// game's `Cargo.toml` follow from it (`scrap modules`), and `scrap
    /// check` says where they part. Absent in a project made before
    /// modules were listed, empty: the engine's default set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<String>,
}

/// The game's window, clock, first scene and language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameSettings {
    /// The scene the game opens, by name: the `.scene.ron` of that name,
    /// wherever it lies.
    pub start_scene: String,
    /// The window's title; the project's name when empty.
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// Fixed simulation steps a second: physics, saves, the network.
    pub steps_per_second: u32,
    /// The table in `strings/` the game speaks.
    pub language: String,
    /// How big the player is and how far it jumps: what the Scene view's
    /// reference shows, navigation bakes for and the game's controller
    /// reads (docs/player.md).
    pub player: crate::player::PlayerMetrics,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            start_scene: "main".into(),
            title: String::new(),
            width: 1280,
            height: 720,
            steps_per_second: 60,
            language: "en".into(),
            player: Default::default(),
        }
    }
}

impl GameSettings {
    /// A game's settings, from `scrap.ron` beside it in a build or in its
    /// project in development ([`data_file`]). What does not read is an
    /// error in words, not the defaults quietly.
    pub fn load(dev_root: &str) -> Result<(String, GameSettings), String> {
        let path = data_file(dev_root, FILE);
        let text =
            crate::files::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let manifest: Manifest =
            ron::from_str(&text).map_err(|e| format!("{}:{e}", path.display()))?;
        Ok((manifest.name, manifest.game))
    }

    /// Seconds per fixed step.
    pub fn fixed_delta(&self) -> f32 {
        1.0 / self.steps_per_second.max(1) as f32
    }
}

/// An open project.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    root: PathBuf,
    manifest: Manifest,
}

/// Why a project could not be opened or made.
#[derive(Debug)]
pub enum ProjectError {
    /// No `scrap.ron` in the folder, or in any folder above the path given.
    NotAProject(PathBuf),
    /// `scrap.ron` is there and does not read.
    BadManifest(PathBuf, String),
    /// Making a project where one already is.
    AlreadyAProject(PathBuf),
    Io(std::io::Error),
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::NotAProject(path) => write!(
                f,
                "{} is not in a scrap project — no {FILE} there or in any folder above it",
                path.display()
            ),
            ProjectError::BadManifest(path, e) => write!(f, "{}:{e}", path.display()),
            ProjectError::AlreadyAProject(path) => {
                write!(f, "{} is already a project", path.display())
            }
            ProjectError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProjectError {}

impl From<std::io::Error> for ProjectError {
    fn from(e: std::io::Error) -> Self {
        ProjectError::Io(e)
    }
}

impl Project {
    /// Open the project whose root is `root`.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let root = root.as_ref().to_path_buf();
        let file = root.join(FILE);
        let text = match crate::files::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ProjectError::NotAProject(root));
            }
            Err(e) => return Err(e.into()),
        };
        let manifest: Manifest =
            ron::from_str(&text).map_err(|e| ProjectError::BadManifest(file, e.to_string()))?;
        Ok(Self { root, manifest })
    }

    /// The project a path is in: the nearest folder at or above it with a
    /// `scrap.ron`.
    ///
    /// What every tool does with the scene it was handed, so that
    /// `scene_shot scenes/camp.ron` and the editor opening the same file
    /// find the same prefabs, materials and library without being told.
    pub fn find(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref();
        let start = if crate::files::is_dir(path) {
            path.to_path_buf()
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
        };
        // Made absolute first: walking up from a relative path stops at the
        // empty path, one folder short of where a project usually is.
        let start = std::path::absolute(&start).unwrap_or(start);
        let mut here = Some(start.as_path());
        while let Some(dir) = here {
            if crate::files::is_file(dir.join(FILE)) {
                return Self::open(dir);
            }
            here = dir.parent();
        }
        Err(ProjectError::NotAProject(path.to_path_buf()))
    }

    /// Make a new project, laid out to the standard, with one scene to open
    /// and a game crate that opens it, getting the engine from git.
    ///
    /// Refuses a folder that is already a project rather than overwriting
    /// its files: the generator is for starting, not for resetting.
    pub fn create(root: impl AsRef<Path>, name: &str) -> Result<Self, ProjectError> {
        Self::create_with(root, name, &Engine::default())
    }

    /// [`Project::create`], with the engine the game crate depends on.
    pub fn create_with(
        root: impl AsRef<Path>,
        name: &str,
        engine: &Engine,
    ) -> Result<Self, ProjectError> {
        Self::create_with_modules(root, name, engine, None)
    }

    /// [`Project::create_with`] for a set of modules (DNA, postulate 8:
    /// `scrap new` offers the bare core, the basic set, the full one): the
    /// modules `scrap.ron` lists and the engine's features they need, which
    /// the engine's manifests say — the core knows no module by name. The
    /// game is written for the set: what a module not in it would run is
    /// left out, and with no module at all it is a game on the bare core,
    /// with no window. `None` writes every module's code and lists none.
    pub fn create_with_modules(
        root: impl AsRef<Path>,
        name: &str,
        engine: &Engine,
        modules: Option<(&[String], &[String])>,
    ) -> Result<Self, ProjectError> {
        let root = root.as_ref().to_path_buf();
        if root.join(FILE).exists() {
            return Err(ProjectError::AlreadyAProject(root));
        }
        std::fs::create_dir_all(root.join(SRC))?;
        let manifest = Manifest {
            name: name.to_string(),
            engine: env!("CARGO_PKG_VERSION").to_string(),
            game: GameSettings::default(),
            modules: modules
                .map(|(listed, _)| listed.to_vec())
                .unwrap_or_default(),
        };
        let pretty = ron::ser::PrettyConfig::new();
        let text = ron::ser::to_string_pretty(&manifest, pretty)
            .map_err(|e| ProjectError::BadManifest(root.join(FILE), e.to_string()))?;
        std::fs::write(root.join(FILE), text + "\n")?;
        std::fs::write(root.join(".gitignore"), GITIGNORE)?;
        std::fs::write(root.join(".gitattributes"), GITATTRIBUTES)?;
        std::fs::write(
            root.join("CLAUDE.md"),
            CLAUDE_MD
                .replace("{name}", name)
                .replace("{folder}", &crate_name(name)),
        )?;
        let write = |relative: &Path, text: &str| -> std::io::Result<()> {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, text)
        };
        let own_relative = Path::new(CONTENT).join(crate_name(name));
        write(
            &own_relative.join(MAPS).join("main.scene.ron"),
            &starter_scene(),
        )?;
        write(Path::new(INPUT), INPUT_RON)?;
        write(Path::new(crate::layers::FILE), LAYERS_RON)?;
        write(
            &own_relative.join("ui/screens/hud.screen.ron"),
            &HUD_RON.replace("{name}", name),
        )?;
        write(&Path::new(crate::strings::DIR).join("en.ron"), STRINGS_EN)?;
        write(&own_relative.join("core/world.ron"), WORLD_RON)?;
        // Git keeps no empty folders, and a layout that disappears on the
        // first clone is not a layout.
        for dir in ["ui/components", "props"] {
            write(&own_relative.join(dir).join(".gitkeep"), "")?;
        }
        write(
            &Path::new(CONTENT)
                .join(crate::layout::DEVELOPERS)
                .join(".gitkeep"),
            "",
        )?;
        let bare = modules.is_some_and(|(listed, _)| listed.is_empty());
        let features = modules.map(|(_, features)| features);
        std::fs::write(
            root.join("Cargo.toml"),
            cargo_toml(&root, name, engine, features),
        )?;
        std::fs::write(root.join("build.rs"), BUILD_RS)?;
        let game = if bare { GAME_BARE } else { GAME };
        let listed = modules.map(|(listed, _)| listed);
        std::fs::write(
            root.join(SRC).join("main.rs"),
            for_modules(game, listed)
                .replace("{name}", name)
                .replace("{folder}", &crate_name(name)),
        )?;
        std::fs::create_dir_all(root.join(SRC).join("spin"))?;
        std::fs::write(root.join(SRC).join("spin/spin.rs"), SPIN_COMPONENT)?;
        std::fs::write(root.join(SRC).join("spin/turn.rs"), SPIN_SYSTEM)?;

        Self::open(root)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        &self.manifest.name
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Whether the project is laid out as before 2026-09-26: a folder per
    /// kind at the root, no `content/`. Read until 2026-10-31.
    pub fn is_legacy(&self) -> bool {
        !crate::files::is_dir(self.root.join(CONTENT))
    }

    /// The project's own folder in `content/` (Allar 2.2): `content/<name>`.
    /// The root, in a project laid out before.
    pub fn content(&self) -> PathBuf {
        if self.is_legacy() {
            self.root.clone()
        } else {
            self.root
                .join(CONTENT)
                .join(crate_name(&self.manifest.name))
        }
    }

    /// Where a new file of a kind goes when nobody said: levels in
    /// `maps/`, screens in `ui/screens/`, the rest in the project's own
    /// folder. In a project laid out before, the kind's old folder.
    pub fn default_dir(&self, kind: crate::layout::Kind) -> PathBuf {
        use crate::layout::Kind;
        if self.is_legacy() {
            let folder = match kind {
                Kind::Prefab => legacy::PREFABS,
                Kind::Material => legacy::MATERIALS,
                Kind::Shader => legacy::SHADERS,
                Kind::Cases => legacy::ANIMATORS,
                _ => kind.legacy_folder().unwrap_or(legacy::SCENES),
            };
            return self.root.join(folder);
        }
        match kind {
            Kind::Scene => self.content().join(MAPS),
            Kind::Screen => self.content().join("ui/screens"),
            _ => self.content(),
        }
    }

    /// The path a new file of a kind called `name` gets: in
    /// [`Project::default_dir`], with the kind's extension — or plain
    /// `.ron` in an old folder, where the folder says the kind.
    pub fn new_file(&self, kind: crate::layout::Kind, name: &str) -> PathBuf {
        let extension = if self.is_legacy() && kind.legacy_folder().is_some() {
            ".ron"
        } else {
            kind.extension()
        };
        self.default_dir(kind).join(format!("{name}{extension}"))
    }

    /// Every file of a kind in the project, wherever it lies, sorted by
    /// path.
    pub fn files(&self, kind: crate::layout::Kind) -> Vec<PathBuf> {
        crate::layout::files(&self.root, kind)
    }

    /// The file of a kind a name — or a path from the root — names.
    pub fn file(&self, kind: crate::layout::Kind, name: &str) -> Option<PathBuf> {
        crate::layout::find(&self.root, kind, name)
    }

    /// The scene called `name`, wherever it lies.
    pub fn scene(&self, name: &str) -> Option<PathBuf> {
        self.file(crate::layout::Kind::Scene, name)
    }

    /// Where new scenes go: `content/<name>/maps/`.
    pub fn scenes(&self) -> PathBuf {
        self.default_dir(crate::layout::Kind::Scene)
    }

    /// Where new prefabs go when nobody said.
    pub fn prefabs(&self) -> PathBuf {
        self.default_dir(crate::layout::Kind::Prefab)
    }

    /// Make a scene called NAME in `maps/`: Unity's File → New Scene, for a
    /// greybox — a ground with the metre grid on it, solid, and the default
    /// sun and view. Refuses a name already taken rather than overwrite a
    /// level.
    pub fn new_scene(&self, name: &str) -> Result<PathBuf, String> {
        valid_name(name)?;
        if let Some(there) = self.scene(name) {
            return Err(format!(
                "{} is already there",
                self.relative(&there).unwrap_or_default()
            ));
        }
        let path = self.new_file(crate::layout::Kind::Scene, name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, blank_scene()).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    /// The project's scenes by name, sorted: `main`, `level_2`.
    pub fn scene_names(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .files(crate::layout::Kind::Scene)
            .iter()
            .map(crate::layout::name_of)
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Where new materials go when nobody said.
    pub fn materials(&self) -> PathBuf {
        self.default_dir(crate::layout::Kind::Material)
    }

    /// Where a new source — a model, a texture — goes when nobody said:
    /// the project's own folder, or `assets/` in a project laid out
    /// before.
    pub fn assets(&self) -> PathBuf {
        if self.is_legacy() {
            self.root.join(legacy::ASSETS)
        } else {
            self.content()
        }
    }

    /// Where content is looked for: every folder but the derived ones and
    /// code ([`crate::layout::walk`]). The root.
    pub fn sources(&self) -> PathBuf {
        self.root.clone()
    }

    /// `config/input.ron`, or `input.ron` at the root of a project laid out
    /// before.
    pub fn input_file(&self) -> PathBuf {
        pick(&self.root, INPUT, legacy::INPUT)
    }

    /// `config/layers.ron`, or `layers.ron` at the root of a project laid
    /// out before.
    pub fn layers_file(&self) -> PathBuf {
        pick(&self.root, crate::layers::FILE, crate::layers::LEGACY_FILE)
    }

    /// `content/localization/`, or `strings/` in a project laid out before.
    pub fn strings_dir(&self) -> PathBuf {
        pick(&self.root, crate::strings::DIR, crate::strings::LEGACY_DIR)
    }

    pub fn library(&self) -> PathBuf {
        self.root.join(LIBRARY)
    }

    /// A path as the project names it: relative to the root, with forward
    /// slashes on every platform. `None` for a path outside the project.
    ///
    /// Forward slashes because this is written into files that are
    /// committed, and a sidecar that says `assets\trees\pine.obj` on one
    /// machine and `assets/trees/pine.obj` on the next is a diff about
    /// nothing.
    pub fn relative(&self, path: impl AsRef<Path>) -> Option<String> {
        let path = path.as_ref();
        let absolute = std::path::absolute(path).ok()?;
        let root = std::path::absolute(&self.root).ok()?;
        let inside = absolute.strip_prefix(&root).ok()?;
        let parts: Vec<String> = inside
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        Some(parts.join("/"))
    }

    /// The game's components, by the name scenes use — the files in
    /// `src/` that declare the struct of their name ([`component_files`]),
    /// sorted. `None` for a project with no `src/`, whose names only its
    /// code knows.
    pub fn component_names(&self) -> Option<Vec<String>> {
        let src = self.root.join(SRC);
        if !src.is_dir() {
            return None;
        }
        let mut names: Vec<String> = component_files(&src)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        names.sort();
        names.dedup();
        Some(names)
    }

    /// The file a project-relative path names.
    pub fn resolve(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

const GITIGNORE: &str = "\
# Built from the sources and their .scrimport sidecars; a clone rebuilds it.
/library/
/target/
# What `scrap build` makes.
/build/
# The editor's memory for whoever uses it: views, the last scene.
/.scrap/
# Keys for the services editor modules call (FAL_KEY, …): a key in git is a
# key on GitHub.
.env
";

/// The `.gitattributes` lines that send scenes, prefabs and configs to
/// `scrap merge`, for a project made before they were in the template.
pub const MERGE_ATTRIBUTES: &str = "\
# Scenes and prefabs merge by entity and field, the game's data by record
# and field, not by line — wherever they lie. The driver is `scrap merge`;
# `scrap git-setup` turns it on in a clone.
*.scene.ron merge=scrap
*.prefab merge=scrap
*.ron merge=scrap
";

/// `new` under `root` when it is there or `old` is not: where a file
/// that moved with the layout is, in a project laid out either way.
pub fn pick(root: &Path, new: &str, old: &str) -> PathBuf {
    let new = root.join(new);
    let old = root.join(old);
    if !crate::files::exists(&new) && crate::files::exists(&old) {
        old
    } else {
        new
    }
}

/// The files `build.rs` looks at under `src`, sorted: in its folders, not
/// right in it, and not in a folder with a `mod.rs` — those are modules
/// main.rs declares.
fn rust_files(src: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut folders: Vec<PathBuf> = std::fs::read_dir(src)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    while let Some(folder) = folders.pop() {
        if folder.join("mod.rs").is_file() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Whether the text of `name.rs` declares the component `name`: a
/// `pub struct` of its name in PascalCase — the rule `build.rs` goes by.
pub fn declares_component(name: &str, text: &str) -> bool {
    let decl = format!("pub struct {}", type_name(name));
    text.match_indices(&decl).any(|(at, _)| {
        !text[at + decl.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    })
}

/// Whether a file's text is a system: it has `pub fn run(`.
pub fn declares_system(text: &str) -> bool {
    text.contains("pub fn run(")
}

/// The game's components under `src`, by name, with their files: a file
/// in `src/components/` (a project laid out before), or anywhere in `src/`
/// declaring the struct of its name.
pub fn component_files(src: &Path) -> Vec<(String, PathBuf)> {
    rust_files(src)
        .into_iter()
        .filter_map(|path| {
            let name = path.file_stem()?.to_string_lossy().into_owned();
            let in_components = path.parent().is_some_and(|p| p.ends_with("components"));
            let in_systems = path.parent().is_some_and(|p| p.ends_with("systems"));
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            (in_components || (!in_systems && declares_component(&name, &text)))
                .then_some((name, path))
        })
        .collect()
}

/// The name Cargo will accept for a project called `name`.
/// A project's name as a crate's: lowercase, `_` for the rest, never
/// starting with a digit.
pub fn crate_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if !out.starts_with(|c: char| c.is_ascii_alphabetic()) {
        out.insert_str(0, "game_");
    }
    out
}

/// `to` as seen from `from`, both folders: `../engine/crates/scrap`.
/// `None` when there is no relative way, such as another drive.
fn relative_path(from: &Path, to: &Path) -> Option<String> {
    let from = std::path::absolute(from).ok()?;
    let to = std::path::absolute(to).ok()?;
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let shared = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    // Sharing only the root (or drive) is not being near each other: a
    // path that climbs to `/` and back down says less than the absolute one.
    let root = from
        .iter()
        .take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
        .count();
    if shared <= root {
        return None;
    }
    let mut parts: Vec<String> = vec!["..".into(); from.len() - shared];
    parts.extend(
        to[shared..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    Some(if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    })
}

fn cargo_toml(root: &Path, name: &str, engine: &Engine, features: Option<&[String]>) -> String {
    let source = engine.cargo_source(root);
    format!(
        "\
[package]
name = \"{crate_name}\"
version = \"0.1.0\"
edition = \"2021\"
publish = false

# The game is its own workspace: editing it rebuilds the game, never the
# engine.
[workspace]

[dependencies]
anyhow = \"1\"
{scrap}
serde = {{ version = \"1\", features = [\"derive\"] }}

# The engine and every other dependency optimised even in a dev build, the
# game itself not: a frame that runs at speed, a rebuild that takes seconds.
# Line tables only: panics still name the line, and the link — most of a
# rebuild — is a third of the time. `scrap rebuild-time` measures it.
[profile.dev]
debug = \"line-tables-only\"

[profile.dev.package.\"*\"]
opt-level = 3
",
        crate_name = crate_name(name),
        scrap = scrap_dependency(&source, features),
    )
}

/// The game's `scrap` line: the engine with the set's features, or — for
/// a game on the bare core — the core itself under the engine's name, so
/// `scrap::` means the same in its code and in every file `scrap add`
/// writes. `None`: the engine's defaults, with the window and sound.
fn scrap_dependency(source: &str, features: Option<&[String]>) -> String {
    match features {
        None => format!("scrap = {{ package = \"scrap-engine\", {source}, features = [\"desktop-shell\", \"audio\"] }}"),
        Some([]) => {
            let source = source.replace("crates/scrap\"", "crates/scrap-core\"");
            format!("scrap = {{ package = \"scrap-core\", {source} }}")
        }
        Some(features) => {
            let list: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
            format!(
                "scrap = {{ package = \"scrap-engine\", {source}, default-features = false, features = [{}] }}",
                list.join(", ")
            )
        }
    }
}

/// A template with what the modules not in `listed` would run left out:
/// the lines between `// @module {` and `// @module }` go when `module` is
/// not listed, and the marks go either way. `None` keeps every module's.
fn for_modules(template: &str, listed: Option<&[String]>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut skipping: Option<String> = None;
    for line in template.split_inclusive('\n') {
        let mark = line.trim().strip_prefix("// @");
        match (mark, &skipping) {
            (Some(mark), None) if mark.ends_with(" {") => {
                let module = mark.trim_end_matches(" {");
                if listed.is_some_and(|l| !l.iter().any(|m| m == module)) {
                    skipping = Some(module.to_string());
                }
            }
            (Some(mark), Some(module)) if mark == format!("{module} }}") => skipping = None,
            (Some(mark), None) if mark.ends_with(" }") => {}
            (_, Some(_)) => {}
            (_, None) => out.push_str(line),
        }
    }
    out
}

/// The numbers a new project starts with.
const WORLD_RON: &str = "\
// The world's numbers. Change one and save while the game runs.
(
    gravity: -9.81,
)
";

/// The bindings a new project starts with.
const HUD_RON: &str = "\
// The heads-up display, drawn over the scene. Each element hangs from an
// anchor (TopLeft, Top, Center, BottomRight, ...), in pixels of a 1280x720
// screen, scaled with the window. Move something here and the running game
// shows it there. See scrap::screen.
(
    elements: [
        (id: \"title\", anchor: TopLeft, at: (20, 16), size: (400, 30), kind: Text(\"{name}\"), text_size: 22),
        (id: \"quit\", anchor: TopRight, at: (-20, 16), size: (120, 36), kind: Button(\"@hud.quit\")),
    ],
)
";

/// A new project's words: one language, to show where a second one goes.
const STRINGS_EN: &str = "\
// The game's words in English. A screen shows one with `@hud.quit`; add
// ru.ron beside this for Russian, with the same keys. scrap check lists
// every key a language lacks.
{
    \"hud.quit\": \"Quit\",
}
";

const LAYERS_RON: &str = "\
// Collision layers. A scene line puts a body on one with `layer: \"debris\"`;
// a line without one is on \"default\". Pairs in `ignore` pass through each
// other. Up to 32. A running game picks up a change here.
(
    layers: [\"default\", \"player\", \"debris\"],
    ignore: [(\"debris\", \"player\")],
)
";

const INPUT_RON: &str = "\
// What the player does, by name. The game asks for \"jump\", not Space;
// rebind here, and a running game picks it up. Pad buttons are named by
// where they are: South is A on Xbox, cross on PlayStation.
(
    actions: {
        \"quit\": [Key(Escape)],
        \"profile\": [Key(F3)],
        \"console\": [Key(Backquote)],
        \"debug\": [Key(Quote)],
        \"jump\": [Key(Space), Pad(South)],
    },
    axes: {
        \"walk\": (negative: [Key(S), Key(Down)], positive: [Key(W), Key(Up)], analog: [LeftY]),
        \"strafe\": (negative: [Key(A), Key(Left)], positive: [Key(D), Key(Right)], analog: [LeftX]),
    },
)
";

/// The game a new project starts with: a window on its `main` scene that
/// keeps up with the files.
const GAME: &str = r#"//! {name}.
//!
//! `cargo run` opens a window on the `main` scene. Save the scene, a
//! prefab, or re-import an asset while it runs, and the change is in the
//! next frames without the game losing its state. Run under
//! `dx serve --hotpatch` and a rebuilt system, `step` or `frame` takes
//! effect without closing the window; a component given a new field is
//! carried across too (`patched` below), keeping what a save keeps.
//!
//! Components are files in `src/components/`, systems files in
//! `src/systems/` (`scrap add component NAME`, `scrap add system NAME`);
//! `build.rs` finds them, and `step` below runs the systems in order.
//!
//! The game is played together by default: `party` is who else is in it.
//! Alone it is a party of one. Started with several players — from the
//! editor, or `scrap run --players 4` — each window is one of them, and
//! what each owns moves in the others' windows too.

use scrap::hecs::World;
use scrap::party::{Event, Party};
use scrap::player_loop::{Phase, PlayerLoop};
use scrap::prelude::*;
use scrap::physics::PhysicsWorld;
use scrap::render::Frame;
use scrap::shell::{self, run, Context, StepContext, WindowConfig};
use scrap::screen::Screen;
use scrap::ui::{TextRun, Ui};
use scrap::widgets::Widgets;
use scrap::{Actions, Components, LiveScene, Tables, Tuned};
use serde::Deserialize;

/// Every file in src/components/, registered by its file name.
mod components {
    include!(concat!(env!("OUT_DIR"), "/components.rs"));
}

/// Every file in src/systems/.
mod systems {
    include!(concat!(env!("OUT_DIR"), "/systems.rs"));
}

/// Numbers from `core/world.ron`, reloaded while the game runs.
#[derive(Deserialize)]
struct WorldNumbers {
    gravity: f32,
}

struct Game {
    live: LiveScene,
    actions: Actions,
    tuning: Tuned<WorldNumbers>,
    layers: Tuned<scrap::layers::Layers>,
    hud: Screen,
    strings: scrap::strings::Strings,
    profile: scrap::perf::Profiler,
    show_profile: bool,
    /// ` : the game's console — `help`, `set world.gravity -3`, the cheats
    /// in [`cheats`]; the editor's Console types into it too.
    console: scrap::console::Console,
    /// ' : what the thing looked at is doing, written over it.
    debug: scrap::debug_overlay::DebugOverlay,
    widgets: Widgets,
    /// Materials' own shaders, put in and reloaded as they are saved.
    shaders: scrap::render::MaterialShaders,
    ui: Ui,
    world: World,
    physics: PhysicsWorld,
    /// The modules' systems, by phase (Unity's PlayerLoop).
    modules: PlayerLoop,
    party: Party,
    components: Components,
    /// The mixer; `None` on a machine with no sound device.
    audio: Option<scrap::audio::Audio>,
    /// The scene's `sound`s, played.
    sounds: scrap::audio::Sources,
    // @animation {
    /// The graphs and clips that lines' `animator`s play.
    motions: scrap::motion::Motions,
    // @animation }
}

impl Game {
    /// Bodies from the world as it is: at the start, and after a patch.
    fn start_physics(&mut self, ctx: &mut Context) {
        self.physics = PhysicsWorld::new(ctx.time.settings().fixed_delta);
        self.physics.set_layers((*self.layers).clone(), &self.world);
        // The scene's wind carries what it says is `blown`.
        self.physics.wind = self.live.scene().wind().unwrap_or_default();
    }
}

/// One fixed step of the game (Unity's FixedUpdate): the game's systems in
/// order, then the modules' — platforms on their routes, clips and
/// characters' animation, everything placed — then physics. The window
/// runs it, and so does the play-mode test below — the same step with
/// nothing drawn.
fn tick(world: &mut World, physics: &mut PhysicsWorld, modules: &mut PlayerLoop, profile: &mut scrap::perf::Profiler, seconds: f32) {
    // systems, in order
    profile.time("turn", || systems::turn::run(world, seconds));
    // @soft {
    // Ropes' ends held by bodies take the bodies' weight and speed.
    scrap::netsim::anchor_ropes(world, physics);
    // @soft }
    // The modules' systems of the fixed step (`scrap::player_loop`).
    modules.run(Phase::FixedUpdate, world, seconds, Some(profile));
    // @soft {
    // What the ropes did to the bodies holding them, for the coming step.
    scrap::netsim::pull_bodies(world, physics);
    // @soft }
    // @fluid {
    // What floats is held up by the water under it, for the coming step.
    scrap::fluid::float(world, physics);
    // @fluid }
    // @character {
    // Ragdolls' muscles pull toward their poses.
    scrap::character::step(world, physics, seconds);
    // @character }
    // Physics is a system too: bodies from the scene, a fixed step, and
    // where the dynamic ones went written back.
    profile.time("physics", || physics.run(world));
    // @destruction {
    // What the step brought together hard enough breaks or dents.
    profile.time("destruction", || scrap::destruction::step(world, physics, seconds));
    // @destruction }
}

/// The game's console commands besides the engine's `help`, `get` and
/// `set` — its cheats. Each is a function of the world and the words typed
/// after its name (Unreal's CheatManager); add yours here. The console is
/// on in a debug build, and in a release one run with SCRAP_CONSOLE=1.
fn cheats() -> scrap::console::Commands {
    let tuning = scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "tuning");
    let mut commands = scrap::console::Commands::new().with_tuning(tuning);
    commands.add("spin", "spin DEGREES: everything that spins turns this fast", |world, words| {
        let speed: f32 = words.first().and_then(|w| w.parse().ok()).ok_or("spin DEGREES, as: spin 90")?;
        let mut turned = 0;
        for spin in world.query_mut::<&mut components::Spin>() {
            spin.degrees_per_second = speed;
            turned += 1;
        }
        Ok(format!("{turned} spinning at {speed} degrees a second"))
    });
    commands
}

/// In the project, write what the components and the tables look like,
/// for the editor's Inspector and Configs window and `scrap check`
/// (library/components.ron, library/tables.ron). Nothing in a build.
fn write_shapes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    if root.join("scrap.ron").is_file() {
        if let Err(e) = game_components().write_shapes(root.join(scrap::project::SHAPES)) {
            eprintln!("{e}");
        }
        if let Err(e) = game_tables().write_shapes(root.join(scrap::project::TABLE_SHAPES)) {
            eprintln!("{e}");
        }
    }
}

/// Every table the game reads, and what its records are: a
/// `Wolf` that is a `scrap::Record`, read with
/// `scrap::Table::<Wolf>::load(...)`, is registered here as
/// `tables.register::<Wolf>("content/{folder}/wolves/wolves.ron")` — and then the editor
/// knows its fields and `scrap check` its links.
fn game_tables() -> Tables {
    Tables::new()
}

/// Every component the game has, by name.
fn game_components() -> Components {
    let mut components = Components::new();
    components::register(&mut components);
    // What core/world.ron is read as: the editor's Table shows its columns and
    // `scrap check` its misspelt fields.
    components.register_tuning::<WorldNumbers>("world");
    components
}

impl shell::Game for Game {
    fn start(&mut self, ctx: &mut Context) {
        for line in self.live.spawn(&mut self.world, ctx.gpu, ctx.renderer).lines() {
            eprintln!("{line}");
        }
        // Play from Here in the editor: what the scene marks
        // `player_start: true` stands where the editor was looking.
        for line in self.live.start_here(&mut self.world) {
            eprintln!("{line}");
        }
        self.start_physics(ctx);
    }

    /// A hot patch landed: the component types may have new fields, so the
    /// world starts again from the scene under the new code, keeping every
    /// transform and every component registered as saved.
    fn patched(&mut self, ctx: &mut Context) {
        let restored = self.live.reinstance(&mut self.world, game_components(), ctx.gpu, ctx.renderer);
        for problem in &restored.problems {
            eprintln!("{problem}");
        }
        self.start_physics(ctx);
    }

    /// Fixed-step game logic: [`tick`].
    fn step(&mut self, ctx: &mut StepContext) {
        let seconds = ctx.time.settings().fixed_delta;
        self.physics.gravity.y = self.tuning.gravity;
        tick(&mut self.world, &mut self.physics, &mut self.modules, &mut self.profile, seconds);
        // Slow motion or a hit-stop the step's systems asked for
        // (`scrap::time::hit_stop`), to the clock.
        ctx.ask_time(scrap::time::sync(&mut self.world, ctx.time));
    }

    fn frame(&mut self, ctx: &mut Context) -> Frame {
        if let Some(Err(problem)) = self.actions.reload_if_changed() {
            eprintln!("{problem}");
        }
        if let Some(Err(problem)) = self.tuning.poll(ctx.time.unscaled_delta()) {
            eprintln!("{problem}");
        }
        match self.layers.poll(ctx.time.unscaled_delta()) {
            Some(Ok(())) => self.physics.set_layers((*self.layers).clone(), &self.world),
            Some(Err(problem)) => eprintln!("{problem}"),
            None => {}
        }
        if let Some(Err(problem)) = self.hud.poll(ctx.time.unscaled_delta()) {
            eprintln!("{problem}");
        }
        self.ui.clear();
        let size = scrap::glam::Vec2::new(ctx.size.0 as f32, ctx.size.1 as f32);
        if let Some(Err(problem)) = self.strings.poll(ctx.time.unscaled_delta()) {
            eprintln!("{problem}");
        }
        // The pad's moves between the screen's widgets, before they draw.
        self.widgets.begin_frame(ctx.input);
        let done = self.hud.draw_localized(&mut self.widgets, &mut self.ui, ctx.input, size, &self.strings);
        // The console: typed into here, or sent from the editor's Console.
        // While it has the keyboard, the game's keys are not the game's.
        let toggled = self.actions.pressed(ctx.input, "console");
        self.console.frame(&mut self.world, ctx.input, toggled, ctx.commands, &mut self.ui, size);
        let keys = !self.console.has_keyboard();
        if done.clicked("quit") || (keys && self.actions.pressed(ctx.input, "quit")) {
            ctx.quit();
        }
        let reload = self.live.poll(ctx.time.unscaled_delta(), &mut self.world, ctx.gpu, ctx.renderer);
        for line in reload.lines() {
            eprintln!("{line}");
        }
        // @animation {
        // Lines that came in with an `animator` start their graphs.
        let library = self.live.library();
        let skins = |model: &scrap::AssetLink| library?.mesh_by_name(model)?.skin_owned();
        for problem in scrap::motion::attach(&mut self.world, &self.motions, skins) {
            eprintln!("{problem}");
        }
        // @animation }
        for (shader, result) in self.shaders.poll(ctx.renderer, ctx.gpu) {
            match result {
                Ok(()) => eprintln!("shader {shader}: in"),
                Err(problem) => eprintln!("{problem}"),
            }
        }
        // Played together: what the others own comes in, what this player
        // owns goes out, and whatever they spawn is spawned here too.
        let (live, gpu, renderer) = (&mut self.live, ctx.gpu, &mut *ctx.renderer);
        let events = self.party.update(&mut self.world, &self.components, ctx.time.unscaled_delta(), |world, prefab, at| {
            live.spawn_prefab(prefab, at, None, world, gpu, renderer).ok().map(|i| i.root)
        });
        for event in events {
            match event {
                // The session plays another scene: go there, then say so.
                Event::SceneRequired { scene } => {
                    match self.live.switch(&scene, &mut self.world, ctx.gpu, ctx.renderer) {
                        Ok((spawned, problems)) => {
                            for line in spawned.lines().into_iter().chain(problems) {
                                eprintln!("{line}");
                            }
                            self.start_physics(ctx);
                        }
                        Err(e) => eprintln!("{e}"),
                    }
                    self.party.ready_in(&scene);
                }
                Event::Welcomed => eprintln!("in the game as {}", self.party.name()),
                Event::Joined { name, .. } => eprintln!("{} joined", name),
                Event::Left { name, clean, .. } => {
                    eprintln!("{} {}", name, if clean { "left" } else { "went quiet, and is gone" })
                }
                Event::HostLost { quit } => eprintln!(
                    "the host {}; waiting for them to come back",
                    if quit { "left" } else { "is not answering" }
                ),
                Event::HostBack => eprintln!("the host is back"),
                Event::Rejected(why) => eprintln!("could not join: {why}"),
                Event::ClaimLost { .. } | Event::Message { .. } => {}
                Event::Silent { id, owner } => eprintln!("{id}: player {} stopped saying where it is", owner.0 + 1),
                Event::Problem(why) => eprintln!("{why}"),
            }
        }
        // Started from the editor: tell it where things are, who this is
        // and what the systems cost.
        self.live.note(self.party.me().0, &self.profile);
        if let Err(problem) = self.live.report(&self.world, ctx.time.unscaled_delta()) {
            eprintln!("{problem}");
        }
        // F3: what each part costs, over the game.
        if keys && self.actions.pressed(ctx.input, "profile") {
            self.show_profile = !self.show_profile;
        }
        if keys && self.actions.pressed(ctx.input, "debug") {
            self.debug.toggle();
        }
        if self.show_profile {
            // The game's parts, then the loop's own: steps, frame, drawing and
            // the wait for the screen.
            let lines = self.profile.lines().into_iter().chain(ctx.loop_times.lines().into_iter().map(|l| format!("loop {l}")));
            for (i, line) in lines.enumerate() {
                let at = 60.0 + 20.0 * i as f32;
                self.ui.text(TextRun::new(20.0, at, 16.0, scrap::glam::Vec4::ONE, line));
            }
        }
        // The clock into the world — the real delta a camera's blend and
        // shake run on — and what the systems asked of it, to the clock.
        ctx.ask_time(scrap::time::sync(&mut self.world, ctx.time));
        // The modules' late systems: cameras that follow keep after their
        // targets and blend and shake; sparks and dust move on the frame's
        // time.
        let delta = ctx.time.delta();
        for phase in [Phase::Update, Phase::LateUpdate, Phase::PostLateUpdate] {
            self.modules.run(phase, &mut self.world, delta, Some(&mut self.profile));
        }
        // A camera on an entity — a child of the player follows the player —
        // or the scene's view when there is none.
        // What the fixed steps move is drawn between the last two of
        // them, by how far this frame is into the next.
        scrap::world::interpolate(&mut self.world, ctx.time.interpolation());
        let camera = scrap::world::camera_of(&self.world)
            .unwrap_or_else(|| scrap::scene_camera(&self.live.scene().view()));
        // The world's streamed regions, in and out by where it looks from.
        for event in self.live.stream(&mut self.world, camera.position, ctx.gpu, ctx.renderer) {
            if let scrap::streaming::StreamEvent::Failed(scene, why) = event {
                eprintln!("stream {scene}: {why}");
            }
        }
        let scene = self.live.scene();
        // Everything the scene says about how it looks: sun, fog, sky and
        // post-processing.
        let mut frame = self.profile.time("frame", || scrap::world::scene_frame(&self.world, camera, scene));
        // ' : the state of the thing in the middle of the view, over it.
        self.debug.draw(&self.world, &self.components, &camera, size, &mut self.ui, &mut frame, ctx.gpu, ctx.renderer);
        // The scene's sounds, heard from where the camera is.
        if let (Some(audio), Some(library)) = (self.audio.as_mut(), self.live.library()) {
            audio.set_listener(camera.position);
            for problem in self.sounds.update(audio, &self.world, |link| library.sound_of(link)) {
                eprintln!("{problem}");
            }
        }
        frame
    }

    fn overlay(&mut self) -> &Ui {
        &self.ui
    }
}

fn main() -> anyhow::Result<()> {
    // `data/` beside the executable in a build, the project in development.
    // scrap.ron's `game`: window, clock, first scene, language.
    let (project_name, settings) =
        scrap::project::GameSettings::load(env!("CARGO_MANIFEST_DIR")).map_err(anyhow::Error::msg)?;
    // A panic is written down in the player's folder: scrap::crash::pending
    // finds it on the next start.
    scrap::crash::install(&project_name, env!("CARGO_PKG_VERSION"));
    // `scrap run --scene cave` plays the scene called `cave`, wherever it lies.
    let playing = std::env::var("SCRAP_SCENE").unwrap_or_else(|_| settings.start_scene.clone());
    // Started from the editor, the game watches the editor's document as it
    // stands (SCRAP_SCENE_FILE), so an edit shows here without a save.
    let scene = match std::env::var_os("SCRAP_SCENE_FILE") {
        Some(file) => std::path::PathBuf::from(file),
        None => scrap::project::data_scene(env!("CARGO_MANIFEST_DIR"), &playing),
    };
    let (live, problems) = LiveScene::open(&scene)?;
    let live = live.with_components(game_components());
    write_shapes();
    for problem in &problems {
        eprintln!("{problem}");
    }
    // SCRAP_NET: host or join a game — alone without it, which is the
    // same thing with nobody else in it.
    let party = Party::from_env(&playing, &game_components()).map_err(anyhow::Error::msg)?;
    let mut title = if settings.title.is_empty() { project_name } else { settings.title.clone() };
    if !party.is_alone() {
        title = format!("{title} — {}", party.name());
    }
    let config = WindowConfig {
        title,
        width: settings.width,
        height: settings.height,
        time: scrap::TimeSettings {
            fixed_delta: settings.fixed_delta(),
            ..Default::default()
        },
        ..Default::default()
    };
    // @animation {
    let (motions, problems) = scrap::motion::Motions::load(scrap::project::data_root(env!("CARGO_MANIFEST_DIR")));
    for problem in &problems {
        eprintln!("{problem}");
    }
    // @animation }
    let actions = Actions::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "config/input.ron"))?;
    for problem in actions.missing(&["quit"]) {
        eprintln!("{problem}");
    }
    let tuning = Tuned::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "content/{folder}/core/world.ron"))
        .map_err(anyhow::Error::msg)?;
    let layers = Tuned::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "config/layers.ron"))
        .map_err(anyhow::Error::msg)?;
    let hud = Screen::load(scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), "content/{folder}/ui/screens/hud.screen.ron"))
        .map_err(anyhow::Error::msg)?;
    let strings = scrap::strings::Strings::load(
        scrap::project::data_file(env!("CARGO_MANIFEST_DIR"), scrap::strings::DIR),
        &settings.language,
    )
    .map_err(anyhow::Error::msg)?;
    let game = Game {
        live,
        actions,
        tuning,
        layers,
        hud,
        strings,
        profile: scrap::perf::Profiler::new(600),
        show_profile: false,
        console: scrap::console::Console::for_build(cheats(), cfg!(debug_assertions)),
        debug: scrap::debug_overlay::DebugOverlay::new(),
        widgets: Widgets::new(),
        shaders: scrap::render::MaterialShaders::new(scrap::project::data_root(env!("CARGO_MANIFEST_DIR"))),
        ui: Ui::new(),
        world: World::new(),
        physics: PhysicsWorld::default(),
        modules: scrap::player_loop::modules(),
        party,
        components: game_components(),
        audio: scrap::audio::Audio::new().map_err(|e| eprintln!("no sound: {e}")).ok(),
        sounds: scrap::audio::Sources::new(),
        // @animation {
        motions,
        // @animation }
    };

    run(config, game)
}

/// Play mode without a window: `scrap test` (or `cargo test`).
#[cfg(test)]
mod tests {
    use super::*;

    /// The start scene opens with nothing unresolved and plays two seconds
    /// of the game's own steps — systems and physics — with everything
    /// still somewhere real at the end.
    #[test]
    fn the_start_scene_plays() {
        write_shapes();
        let (_, settings) = scrap::project::GameSettings::load(env!("CARGO_MANIFEST_DIR")).unwrap();
        let scene = scrap::project::data_scene(env!("CARGO_MANIFEST_DIR"), &settings.start_scene);
        let (live, problems) = LiveScene::open(&scene).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let mut live = live.with_components(game_components());
        let mut world = World::new();
        let spawned = live.spawn_headless(&mut world);
        assert!(spawned.lines().is_empty(), "{:?}", spawned.lines());

        let seconds = settings.fixed_delta();
        let mut physics = PhysicsWorld::new(seconds);
        let mut modules = scrap::player_loop::modules();
        let mut profile = scrap::perf::Profiler::new(8);
        for _ in 0..(2.0 / seconds) as usize {
            tick(&mut world, &mut physics, &mut modules, &mut profile, seconds);
        }
        for (_, transform) in world.query::<(scrap::hecs::Entity, &scrap::Transform)>().iter() {
            assert!(transform.position.is_finite(), "something flew off: {transform:?}");
        }
    }

    /// Every table loads into its type, keeps its records' rules, and links
    /// only to records that are there.
    #[test]
    fn the_tables_hold() {
        let problems = game_tables().problems(std::path::Path::new(env!("CARGO_MANIFEST_DIR")));
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
"#;

/// The game a project on the bare core starts with: no window, no picture,
/// no sound — a server, a simulation, a test.
const GAME_BARE: &str = r#"//! {name}, on the bare core: no window, no picture, no sound —
//! `scrap.ron` lists no modules. It plays its start scene headless, the
//! systems in order at the fixed step: a server, a simulation, a test.
//! `scrap modules` shows what there is to add to scrap.ron's `modules`;
//! `scrap modules sync` then brings the engine in.
//!
//! Components are files in `src/components/`, systems files in
//! `src/systems/` (`scrap add component NAME`, `scrap add system NAME`);
//! `build.rs` finds them, and `tick` below runs the systems in order.

use scrap::hecs::World;
use scrap::player_loop::{Phase, PlayerLoop};
use scrap::{Components, Scene};

/// Every file in src/components/, registered by its file name.
mod components {
    include!(concat!(env!("OUT_DIR"), "/components.rs"));
}

/// Every file in src/systems/.
mod systems {
    include!(concat!(env!("OUT_DIR"), "/systems.rs"));
}

/// One fixed step of the game (Unity's FixedUpdate): the game's systems in
/// order, then the core's — everything placed.
fn tick(world: &mut World, modules: &mut PlayerLoop, seconds: f32) {
    // systems, in order
    systems::turn::run(world, seconds);
    modules.run(Phase::FixedUpdate, world, seconds, None);
}

/// The systems of the modules the game has, by phase: on the bare core,
/// the core's own.
fn modules() -> PlayerLoop {
    let mut modules = PlayerLoop::new();
    scrap::player_loop::systems(&mut modules);
    modules
}

/// Every component the game has, by name.
fn game_components() -> Components {
    let mut components = Components::new();
    components::register(&mut components);
    components
}

/// The scene at `path`, its prefabs expanded, in a new world with the
/// game's components on it. Alone, this player simulates everything.
fn start(path: &std::path::Path) -> anyhow::Result<World> {
    let scene = Scene::load(path)?;
    let (prefabs, problems) =
        scrap::Prefabs::open(scrap::project::data_root(env!("CARGO_MANIFEST_DIR")))?;
    for (path, problem) in problems {
        eprintln!("{}: {problem}", path.display());
    }
    let scene = scrap::prefab::instantiate_with(&scene, &prefabs, |_| {}).scene;
    let mut world = World::new();
    scrap::world::spawn_scene_dressed(&scene, &mut world, &mut []);
    for problem in game_components().apply(&scene, &mut world) {
        eprintln!("{problem}");
    }
    let everything: Vec<_> = world.iter().map(|e| e.entity()).collect();
    for entity in everything {
        let _ = world.insert_one(entity, scrap::world::Owned);
    }
    Ok(world)
}

fn main() -> anyhow::Result<()> {
    // `data/` beside the executable in a build, the project in development.
    let (project_name, settings) =
        scrap::project::GameSettings::load(env!("CARGO_MANIFEST_DIR")).map_err(anyhow::Error::msg)?;
    scrap::crash::install(&project_name, env!("CARGO_PKG_VERSION"));
    let playing = std::env::var("SCRAP_SCENE").unwrap_or_else(|_| settings.start_scene.clone());
    let scene = scrap::project::data_scene(env!("CARGO_MANIFEST_DIR"), &playing);
    let mut world = start(&scene)?;
    let seconds = settings.fixed_delta();
    eprintln!("{project_name}: {playing}, {} steps a second", settings.steps_per_second);
    let step = std::time::Duration::from_secs_f32(seconds);
    let mut modules = modules();
    loop {
        tick(&mut world, &mut modules, seconds);
        std::thread::sleep(step);
    }
}

/// Play without anything else: `scrap test` (or `cargo test`).
#[cfg(test)]
mod tests {
    use super::*;

    /// The start scene plays two seconds of the game's own steps with
    /// everything still somewhere real at the end.
    #[test]
    fn the_start_scene_plays() {
        let (_, settings) = scrap::project::GameSettings::load(env!("CARGO_MANIFEST_DIR")).unwrap();
        let scene = scrap::project::data_scene(env!("CARGO_MANIFEST_DIR"), &settings.start_scene);
        let mut world = start(&scene).unwrap();
        let seconds = settings.fixed_delta();
        let mut modules = modules();
        for _ in 0..(2.0 / seconds) as usize {
            tick(&mut world, &mut modules, seconds);
        }
        for (_, transform) in world.query::<(scrap::hecs::Entity, &scrap::Transform)>().iter() {
            assert!(transform.position.is_finite(), "something flew off: {transform:?}");
        }
    }
}
"#;

/// The component a new project starts with.
const SPIN_COMPONENT: &str = r#"//! Turning in place. A scene line gives it as
//! `components: { "spin": (degrees_per_second: 45.0) }` — the file's name is
//! the name — and changing the number while the game runs changes the speed.

use serde::Deserialize;

#[derive(Deserialize)]
pub struct Spin {
    pub degrees_per_second: f32,
}
"#;

/// The system a new project starts with.
const SPIN_SYSTEM: &str = r#"//! Turns everything that has a `Spin` and this player drives: what someone
//! else drives turns on their machine and is shown turning here. Every
//! system that simulates asks for `Owned`; alone, everything is.

use scrap::hecs::World;
use scrap::world::Owned;
use scrap::Transform;

use crate::components::Spin;

pub fn run(world: &mut World, seconds: f32) {
    for (transform, spin) in world.query_mut::<(&mut Transform, &Spin)>().with::<&Owned>() {
        transform.rotation_deg.y += spin.degrees_per_second * seconds;
    }
}
"#;

/// A new component's file: `scrap add component NAME`.
pub fn component_file(name: &str) -> String {
    format!(
        "//! `{name}`: what it means for an entity to have one. A scene line gives it\n\
         //! as `components: {{ \"{name}\": (…) }}`. Derive `Serialize` too and\n\
         //! its value is kept — in a save game, and across a hot patch.\n\
         \n\
         use serde::Deserialize;\n\
         \n\
         #[derive(Deserialize)]\n\
         pub struct {ty} {{}}\n",
        ty = type_name(name)
    )
}

/// A new system's file: `scrap add system NAME`.
pub fn system_file(name: &str) -> String {
    format!(
        "//! `{name}`. What it changes, it changes only on what this player\n\
         //! drives: query with `.with::<&scrap::net::Owned>()` — alone,\n\
         //! that is everything; together, the rest is someone else's to run.\n\
         \n\
         use scrap::hecs::World;\n\
         \n\
         pub fn run(world: &mut World, seconds: f32) {{\n\
         \x20   let _ = (world, seconds);\n\
         }}\n"
    )
}

/// The Rust type a component file declares: `front_door` is `FrontDoor`.
pub fn type_name(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

/// Whether a name can be a component's or system's file: a Rust module
/// name, snake_case, not a keyword.
pub fn valid_name(name: &str) -> Result<(), String> {
    let ok = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !ok {
        return Err(format!(
            "`{name}` cannot be a file here: snake_case, starting with a letter, like `front_door`"
        ));
    }
    if [
        "mod", "self", "super", "crate", "type", "fn", "struct", "use", "match", "loop", "move",
        "ref", "static", "trait", "impl", "where", "async", "await", "dyn", "box", "yield", "let",
        "if", "else", "for", "in", "while", "return", "break", "continue", "const", "enum",
        "extern", "pub", "mut", "true", "false", "as", "unsafe", "register",
    ]
    .contains(&name)
    {
        return Err(format!("`{name}` is a Rust keyword or taken; pick another"));
    }
    Ok(())
}

/// The game's `build.rs`: the module list and the registration, written
/// from what files there are.
const BUILD_RS: &str = r##"//! Finds the game's components and systems. Written by `scrap new`; not
//! edited.
//!
//! Code lies by feature: `src/cooking/pot.rs`, `src/cooking/cook.rs`. A file
//! in a folder under `src/` that declares the struct of its name
//! (`pot.rs`, `pub struct Pot`) is a component, and its file name is the
//! name scenes use for it. One with `pub fn run(` is a system; `step` in
//! main.rs runs them in order. The rest are the game's own modules. A
//! folder with a `mod.rs` is an ordinary module main.rs declares, and so
//! are the files right in `src/`: neither is looked into.

use std::fmt::Write;
use std::path::{Path, PathBuf};

fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let src = Path::new(&root).join("src");
    let (components, systems) = find(&src);

    let mut text = String::new();
    for (name, path) in &components {
        let _ = writeln!(text, "#[path = {path:?}]
pub mod {name};
pub use {name}::{};", type_name(name));
    }
    text.push_str("
/// Every component above, by its file name.
pub fn register(components: &mut scrap::Components) {
    let _ = &components;
    // The engine's simulations' network states (docs/netsim.md).
    scrap::netsim::register(components);
");
    for (name, path) in &components {
        // A component that can be written down is kept: by a save game,
        // and across a hot patch.
        let how = if serializable(path) { "register_saved" } else { "register" };
        let _ = writeln!(text, "    components.{how}::<{}>({name:?});", type_name(name));
        // One whose file says `pub const NETWORKED: bool = true;` goes to
        // the other players from whoever owns the entity.
        if networked(path) {
            let _ = writeln!(text, "    let _ = {name}::NETWORKED;
    components.register_networked::<{}>({name:?});", type_name(name));
        }
    }
    text.push_str("}
");
    std::fs::write(Path::new(&out).join("components.rs"), text).unwrap();

    let mut text = String::new();
    for (name, path) in &systems {
        let _ = writeln!(text, "#[path = {path:?}]
pub mod {name};");
    }
    std::fs::write(Path::new(&out).join("systems.rs"), text).unwrap();
}

/// Whether a component's file derives `Serialize` (not only `Deserialize`).
fn serializable(path: &str) -> bool {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.match_indices("Serialize").any(|(at, _)| !text[..at].ends_with("De"))
}

/// Whether a component's file marks it networked.
fn networked(path: &str) -> bool {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .contains("pub const NETWORKED: bool = true;")
}

/// The components and the systems under `src`, as `(module name, absolute
/// path)`, sorted. Two of either with one name is an error: a scene could
/// not tell the components apart, and `step` the systems.
fn find(src: &Path) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let mut components = Vec::new();
    let mut systems = Vec::new();
    let mut folders: Vec<PathBuf> = Vec::new();
    println!("cargo::rerun-if-changed={}", src.display());
    for entry in std::fs::read_dir(src).into_iter().flatten().flatten() {
        if entry.path().is_dir() {
            folders.push(entry.path());
        }
    }
    while let Some(folder) = folders.pop() {
        println!("cargo::rerun-if-changed={}", folder.display());
        if folder.join("mod.rs").is_file() {
            continue;
        }
        // A project laid out before: the folder says which.
        let only = folder.file_name().and_then(|n| n.to_str()).filter(|n| *n == "components" || *n == "systems").map(str::to_string);
        for path in std::fs::read_dir(&folder).into_iter().flatten().flatten().map(|e| e.path()) {
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let component = match only.as_deref() {
                Some(which) => which == "components",
                None => declares(&name, &text),
            };
            let system = match only.as_deref() {
                Some(which) => which == "systems",
                None => !component && text.contains("pub fn run("),
            };
            if !(component || system) {
                continue;
            }
            if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
                panic!("{}: a component or system file is named in snake_case, like front_door.rs", path.display());
            }
            let found = (name, path.to_string_lossy().replace('\\', "/"));
            if component { components.push(found) } else { systems.push(found) }
        }
    }
    for list in [&mut components, &mut systems] {
        list.sort();
        for pair in list.windows(2) {
            if pair[0].0 == pair[1].0 {
                panic!("two files are called {}.rs: {} and {} — a scene or `step` could not tell them apart; rename one", pair[0].0, pair[0].1, pair[1].1);
            }
        }
    }
    (components, systems)
}

/// Whether `name.rs` declares the struct of its name.
fn declares(name: &str, text: &str) -> bool {
    let decl = format!("pub struct {}", type_name(name));
    text.match_indices(&decl).any(|(at, _)| !text[at + decl.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_'))
}

/// `front_door` is `FrontDoor`.
fn type_name(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
        })
        .collect()
}
"##;

/// Binary sources go to LFS and are lockable from the first commit
/// (DNA, postulate 2). Two people editing the same texture cannot merge it,
/// so the second one should find out before they start, not after.
///
/// Text is checked out with LF everywhere, because sidecars store a hash of
/// the source's bytes: a Windows checkout that turned every `.obj` into
/// CRLF would rebuild the whole library and rewrite every sidecar.
const GITATTRIBUTES: &str = "\
# Text as LF on every machine: .scrimport stores a hash of the source's bytes.
* text=auto eol=lf

# Scenes and prefabs merge by entity and field, the game's data by record
# and field, not by line — wherever they lie. The driver is `scrap merge`;
# `scrap git-setup` turns it on in a clone.
*.scene.ron merge=scrap
*.prefab merge=scrap
*.ron merge=scrap

# Binary sources: stored in LFS, lockable. Text sources (.ron, .prefab,
# .scrmat, .scrimport, .obj, .gltf) stay in git, where a diff means something.
*.png   filter=lfs diff=lfs merge=lfs -text lockable
*.jpg   filter=lfs diff=lfs merge=lfs -text lockable
*.jpeg  filter=lfs diff=lfs merge=lfs -text lockable
*.tga   filter=lfs diff=lfs merge=lfs -text lockable
*.bmp   filter=lfs diff=lfs merge=lfs -text lockable
*.exr   filter=lfs diff=lfs merge=lfs -text lockable
*.hdr   filter=lfs diff=lfs merge=lfs -text lockable
*.psd   filter=lfs diff=lfs merge=lfs -text lockable
*.wav   filter=lfs diff=lfs merge=lfs -text lockable
*.ogg   filter=lfs diff=lfs merge=lfs -text lockable
*.glb   filter=lfs diff=lfs merge=lfs -text lockable
*.bin   filter=lfs diff=lfs merge=lfs -text lockable
*.fbx   filter=lfs diff=lfs merge=lfs -text lockable
*.blend filter=lfs diff=lfs merge=lfs -text lockable
";

const CLAUDE_MD: &str = "\
# {name}

A scrap project. What a file is, its extension says — not the folder it
lies in — so every tool, and every agent, finds a file wherever it is
(docs/layout.md in the engine). Keep what belongs together together: a
tomato's model, texture, material and prefab in one folder; code by
feature. The layout `scrap new` wrote is Unreal's:

```
scrap.ron     the project file: name, modules, the start scene by name
config/       settings for the whole game
  input.ron   actions by name (\"jump\"), and the keys for each
  layers.ron  collision layers, and which pairs pass through each other
content/      everything a person makes
  {folder}/   the game's own folder
    maps/     levels
    core/     what everything else stands on; world.ron, the game's numbers
    props/    a feature: its model, texture, material and prefab together
    ui/screens/     the screens
    ui/components/  pieces of screens
  localization/     the game's words, one file per language; a screen says `@key`
  developers/ sandboxes: the editor reads them, `scrap build` does not ship them
library/      built assets — derived, never committed
Cargo.toml    the game crate; build.rs finds its components and systems
src/main.rs   the game: the window, and `step`, which runs the systems in order
src/<feature>/  code by feature: `spin/spin.rs` is the component `spin`, `spin/turn.rs` a system
```

Kinds, by extension — find them anywhere with a glob (`**/*.prefab`):

```
*.scene.ron     scenes, RON — one entity per block, `id` first
*.prefab        one entity subtree per file; a scene places it with `prefab: \"name\"`
*.scrmat        materials: `(color: \"#rrggbb\")`, sRGB hex
*.screen.ron    the game's screens: elements anchored in a 1280x720 frame (scrap::screen)
*.animator.ron  which animation plays when: states and transitions (scrap::animgraph)
*.clip.ron      clips that move things, not bones — a line's `animator` plays them (scrap::motion)
*.dialogue.ron  conversations: lines, answers and the flags they set (scrap::dialogue)
*.quest.ron     rows of stages done by the dialogues' flags (scrap::quest)
*.cases.ron     beside a graph or a dialogue: the cases `scrap check` plays
*.wgsl, *.graph.ron  materials' own looks: `shader: \"water\"` is water.wgsl
*.vfx.ron       particle effect graphs
*.post.ron      fullscreen graphs, over the whole picture: a scene says `fullscreen: (graph: \"name\")`
*.ron           anything else is the game's data: one struct is scrap::Tuned, records by name scrap::Table
models, textures, sounds  sources; each gets a .scrimport beside it
```

A name is the file's name without that extension: `maps/cave.scene.ron`
is the scene `cave`, wherever it lies. Two files of one kind with one name
are ambiguous to a line that names only the name; `scrap check` says so.

* `scrap run` (or `cargo run`) plays the scene `main`. It keeps running
  while you edit: saved scenes, prefabs and rebuilt assets show up in the
  window. `scrap run --hot` patches the game's own Rust in too, under
  `dx serve --hotpatch` (`cargo install dioxus-cli`).
* `scrap check` says what does not resolve — a model, a material or a
  prefab nobody has, a repeated id, a stale sidecar — with the file and the
  entity. Run it after editing scenes; it exits non-zero on errors.
* `scrap git-setup`, once per clone, turns on the merge driver: scenes
  and prefabs then merge by entity and field, and a real conflict is
  reported in words (\"both changed the position of `tree`\") with ours
  kept and the file still loading.
* `scrap build` makes a folder to ship: the game in release, and `data/`
  beside it with the scenes, prefabs, screens, data and built library at
  the same paths — no sources, and nothing from `developers/`.
* Scenes link assets by ID and name (`model: (\"rock\", \"3f9a…\")`), so
  a file moved to another folder — with its `.scrimport` beside it — is
  still found. `scrap rename FROM TO` renames or moves a model, texture,
  sound, material or prefab and its .scrimport together, and freshens the
  name in every line that named it; `scrap uses FILE` lists those lines
  first. `scrap assets` lists every asset and how much it is used;
  `scrap delete FILE` removes one only when nothing names it;
  `scrap duplicate FROM TO` copies one as a new asset.
* `scrap sync` builds `library/` from the sources. After adding, changing
  or moving a source, run it and commit the `.scrimport` it writes beside the
  source. Never edit a sidecar's `hash` or `id` by hand: the hash is how a
  moved file is found, the id is what the library refers to.
* Every entity has an `id`: sixteen hex digits, unique in its file. When
  writing one by hand, leave it out and the engine assigns one on load;
  never copy an existing one.
* The editor is also an MCP server: with scrap's `scrap-mcp` on the PATH
  (`cargo install --path crates/scrap-mcp` in the engine),
  `claude mcp add scrap -- scrap-mcp` gives every editor operation as a
  tool — open, add, move, undo, render a frame to look at, check, simulate.
* Game logic is Rust on the ECS (`scrap::hecs`): components are plain
  structs, systems are functions over the world, and an entity spawned from
  a scene carries `SceneId`. `scrap add component doors/door` writes
  `src/doors/door.rs` (struct `Door`), and build.rs registers it as
  `\"door\"` — a scene line then gives it:
  `components: { \"door\": (locked: true) }`. A component the other
  players must see derives `Serialize` too and says
  `pub const NETWORKED: bool = true;` in its file: whoever owns the entity
  sends it. `scrap add system guards/patrol`
  writes `src/guards/patrol.rs` and adds its call last in `step`; move the
  line to change the order. build.rs finds them in every folder under
  `src/`: a file declaring the struct of its name is a component, one with
  `pub fn run(` a system; the files right in `src/`, and folders with a
  `mod.rs`, are the game's own modules and are left alone — so never
  declare a component's file with `mod` yourself. Never register
  components by hand or keep a list of them: the files are the list.
  `scrap check` reports a component name no file answers to. Editing a value while the game runs changes that
  component and nothing else.
* The player's size is `game: (player: (height, radius, step, slope,
  jump_height, speed, gravity))` in `scrap.ron`: navigation bakes for it,
  the editor draws it (View › Player), and a controller should read the
  same numbers (`scrap::project::GameSettings::load`). The line the player
  starts as — or an empty the game spawns it at — says
  `player_start: true`; Play › Play from Here moves it where the editor
  looks (`LiveScene::start_here` in `start`).
* Everything a person makes is text and is committed; `library/` and
  `target/` are not.
* Binary sources are in Git LFS and lockable — lock before editing one.
";

/// The scene a new project opens to: ground to stand on and one thing on
/// it, so the first frame shows that everything works.
/// A new scene: somewhere to stand, and nothing else.
fn blank_scene() -> String {
    format!(
        "(
    entities: [
        (id: \"{}\", name: \"ground\", model: \"builtin:plane\", transform: (scale: (40.0, 1.0, 40.0)), material: \"grid\", body: Static, collider: Box(half: (0.5, 0.05, 0.5))),
    ],
)
",
        crate::EntityId::fresh()
    )
}

fn starter_scene() -> String {
    let (ground, cube, crate_) = (
        crate::EntityId::fresh(),
        crate::EntityId::fresh(),
        crate::EntityId::fresh(),
    );
    format!(
        "(
    entities: [
        (id: \"{ground}\", name: \"ground\", model: \"builtin:plane\", transform: (scale: (40.0, 1.0, 40.0)), material: \"grid\", body: Static, collider: Box(half: (0.5, 0.05, 0.5))),
        (id: \"{cube}\", name: \"cube\", model: \"builtin:cube\", transform: (position: (0.0, 0.5, 0.0)), material: \"earth\", components: {{ \"spin\": (degrees_per_second: 45.0) }}),
        (id: \"{crate_}\", name: \"crate\", model: \"builtin:cube\", transform: (position: (1.5, 4.0, 0.0), scale: (0.6, 0.6, 0.6)), material: \"bark\", body: Dynamic, collider: Box(half: (0.5, 0.5, 0.5))),
    ],
)
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scrap-project-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_new_scene_is_ground_to_stand_on_and_never_overwrites_a_level() {
        let root = temp("new-scene");
        let project = Project::create(&root, "moss").unwrap();
        let path = project.new_scene("cave").unwrap();
        let scene = crate::Scene::load(&path).unwrap();
        assert_eq!(scene.entities.len(), 1);
        assert_eq!(scene.entities[0].name, "ground");
        assert_eq!(project.scene_names(), ["cave", "main"]);
        assert_eq!(path, root.join("content/moss/maps/cave.scene.ron"));
        // The game picks the scene by name at run time, wherever it lies,
        // not the project's name baked in by the template.
        let main = std::fs::read_to_string(root.join("src/main.rs")).unwrap();
        assert!(
            main.contains("scrap::project::data_scene(env!(\"CARGO_MANIFEST_DIR\"), &playing)"),
            "{main}"
        );
        let e = project.new_scene("main").unwrap_err();
        assert!(e.contains("already there"), "{e}");
        assert!(project.new_scene("Cave Two").is_err());
    }

    #[test]
    fn a_new_project_is_laid_out_to_the_standard_and_opens() {
        let root = temp("new");
        let project = Project::create(&root, "moss").unwrap();
        assert_eq!(project.name(), "moss");
        // Unreal's scheme: settings in config/, what people make in
        // content/, the project's own folder in it (docs/layout.md).
        assert!(!project.is_legacy());
        assert_eq!(project.content(), root.join("content/moss"));
        for file in [
            FILE,
            ".gitignore",
            ".gitattributes",
            "CLAUDE.md",
            "Cargo.toml",
            "src/main.rs",
            "config/input.ron",
            "config/layers.ron",
            "content/moss/maps/main.scene.ron",
            "content/moss/core/world.ron",
            "content/moss/ui/screens/hud.screen.ron",
            "content/localization/en.ron",
            "content/developers/.gitkeep",
        ] {
            assert!(root.join(file).is_file(), "{file}");
        }
        for old in [
            "scenes",
            "prefabs",
            "materials",
            "assets",
            "input.ron",
            "configs",
        ] {
            assert!(!root.join(old).exists(), "{old}");
        }
        assert_eq!(project.input_file(), root.join("config/input.ron"));
        assert_eq!(project.layers_file(), root.join("config/layers.ron"));
        assert_eq!(project.strings_dir(), root.join("content/localization"));
        let main = std::fs::read_to_string(root.join("src/main.rs")).unwrap();
        assert!(main.contains("\"content/moss/core/world.ron\""), "{main}");
        assert!(!main.contains("{folder}"), "{main}");
        let attributes = std::fs::read_to_string(root.join(".gitattributes")).unwrap();
        assert!(
            attributes.contains("*.scene.ron merge=scrap"),
            "{attributes}"
        );
        // The library is derived: ignored, and not made until something is
        // built into it.
        assert!(std::fs::read_to_string(root.join(".gitignore"))
            .unwrap()
            .contains("/library/"));

        // And the scene it starts with opens, with every entity named.
        let scene = crate::Scene::load(root.join("content/moss/maps/main.scene.ron")).unwrap();
        assert_eq!(scene.entities.len(), 3);
        let text = std::fs::read_to_string(root.join("content/moss/maps/main.scene.ron")).unwrap();
        assert_eq!(
            text.matches("id: ").count(),
            3,
            "ids written, not minted on load"
        );

        assert_eq!(Project::open(&root).unwrap(), project);
    }

    #[test]
    fn the_game_crate_names_the_engine_relative_to_the_project() {
        let root = temp("engine-path");
        let engine = std::env::temp_dir().join("scrap-project-engine/crates/scrap");
        Project::create_with(&root, "My Game", &Engine::Path(engine)).unwrap();
        let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(cargo.contains("name = \"my_game\""), "{cargo}");
        assert!(
            cargo.contains("path = \"../scrap-project-engine/crates/scrap\""),
            "relative, so the two can move together: {cargo}"
        );
        assert!(cargo.contains("[workspace]"), "its own workspace: {cargo}");
        // The window's title is the project's name, from scrap.ron.
        let manifest = std::fs::read_to_string(root.join(FILE)).unwrap();
        assert!(manifest.contains("name: \"My Game\""), "{manifest}");

        let default = Project::create(temp("engine-git"), "7 seas").unwrap();
        let cargo = std::fs::read_to_string(default.root().join("Cargo.toml")).unwrap();
        assert!(cargo.contains("git = \"https://"), "{cargo}");
        assert!(cargo.contains("name = \"game_7_seas\""), "{cargo}");
    }

    #[test]
    fn making_a_project_where_one_is_refuses_rather_than_overwriting() {
        let root = temp("twice");
        Project::create(&root, "once").unwrap();
        std::fs::write(
            root.join("content/once/maps/main.scene.ron"),
            "(entities: [])",
        )
        .unwrap();
        assert!(matches!(
            Project::create(&root, "twice"),
            Err(ProjectError::AlreadyAProject(_))
        ));
        assert_eq!(
            std::fs::read_to_string(root.join("content/once/maps/main.scene.ron")).unwrap(),
            "(entities: [])",
            "and nothing was touched"
        );
    }

    #[test]
    fn a_project_is_found_from_anything_inside_it() {
        let root = temp("find");
        let project = Project::create(&root, "found").unwrap();
        std::fs::create_dir_all(root.join("content/found/maps/caves")).unwrap();
        let deep = root.join("content/found/maps/caves/deep.scene.ron");
        std::fs::write(&deep, "(entities: [])").unwrap();

        assert_eq!(Project::find(&deep).unwrap(), project);
        assert_eq!(Project::find(root.join("content")).unwrap(), project);
        assert_eq!(
            project.scene("deep").unwrap(),
            deep,
            "a scene in a subfolder is found by its name"
        );

        // And outside one, it says so in a sentence.
        let lost = std::env::temp_dir().join("scrap-project-nowhere/scene.ron");
        let _ = std::fs::create_dir_all(lost.parent().unwrap());
        if !lost.parent().unwrap().join(FILE).exists() {
            let err = Project::find(&lost).unwrap_err();
            assert!(err.to_string().contains(FILE), "{err}");
        }
    }

    #[test]
    fn a_path_is_named_relative_to_the_project_with_forward_slashes() {
        let root = temp("relative");
        let project = Project::create(&root, "paths").unwrap();
        let file = root.join("assets").join("trees").join("pine.obj");
        assert_eq!(
            project.relative(&file).as_deref(),
            Some("assets/trees/pine.obj")
        );
        assert_eq!(project.resolve("assets/trees/pine.obj"), file);
        assert_eq!(
            project.relative(std::env::temp_dir().join("elsewhere.obj")),
            None
        );
    }

    #[test]
    fn a_manifest_that_does_not_read_says_where() {
        let root = temp("bad");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(FILE), "(name: ").unwrap();
        let err = Project::open(&root).unwrap_err();
        assert!(err.to_string().contains(FILE), "{err}");
    }

    #[test]
    fn a_new_project_lays_its_code_out_one_file_per_component_and_system() {
        let root = std::env::temp_dir().join("scrap-project-code-layout");
        let _ = std::fs::remove_dir_all(&root);
        let project = Project::create(&root, "layout").unwrap();
        // By feature: the component and its system together.
        assert_eq!(project.component_names(), Some(vec!["spin".to_string()]));
        assert!(root.join("src/spin/spin.rs").is_file());
        assert!(root.join("src/spin/turn.rs").is_file());
        let main = std::fs::read_to_string(root.join("src/main.rs")).unwrap();
        assert!(main.contains("components::register(&mut components);"));
        assert!(
            main.contains(
                "// systems, in order\n    profile.time(\"turn\", || systems::turn::run("
            ),
            "{main}"
        );
        // The build script is compiled only in the game; a stray escape in
        // this template is a game that does not build.
        let build = std::fs::read_to_string(root.join("build.rs")).unwrap();
        assert!(build.contains(r#".replace('\\', "/")"#), "{build}");
    }

    #[test]
    fn a_component_file_name_is_its_scene_name_and_its_type() {
        assert_eq!(type_name("front_door"), "FrontDoor");
        assert_eq!(type_name("spin"), "Spin");
        assert!(valid_name("front_door").is_ok());
        assert!(valid_name("Front Door").unwrap_err().contains("snake_case"));
        assert!(valid_name("mod").is_err());
        assert!(component_file("front_door").contains("pub struct FrontDoor {}"));
        assert!(system_file("patrol").contains("pub fn run(world: &mut World, seconds: f32)"));
    }
}
