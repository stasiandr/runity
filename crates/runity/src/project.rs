//! A game project: one folder, laid out one way.
//!
//! DNA, postulate 7: one standard for where everything lives — scenes,
//! prefabs, materials, sources, the built library — so that neither a
//! person nor an agent has to guess, and every tool finds the same things.
//! Before this there was a convention per tool ("`prefabs/` beside the
//! scene"), which is the kind of rule that holds until the second scene
//! lives in a subfolder.
//!
//! ```text
//! <project>/
//!   runity.ron      this file says it is a project: name, engine version
//!   scenes/         *.ron
//!   prefabs/        *.prefab
//!   materials/      *.rmat
//!   assets/         sources: models, textures, sounds
//!   library/        built .rasset — derived, never committed
//!   Cargo.toml      the game crate, its own workspace
//!   src/main.rs     the game: a window on scenes/main.ron, reloading live
//!   CLAUDE.md       what an agent needs to work here
//! ```
//!
//! Not configurable, on purpose. A layout with settings is a layout every
//! tool has to read the settings of, and a project that moved `scenes/`
//! somewhere clever is a project the next tool gets wrong. Custom is done by
//! replacing a module, not by a hundred options (postulate 7).
//!
//! The game is its own crate, at the root, and its own workspace: editing
//! it rebuilds the game and never the engine (postulate 1).

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file that makes a folder a project.
pub const FILE: &str = "runity.ron";
/// Where scenes live.
pub const SCENES: &str = "scenes";
/// Where prefabs live.
pub const PREFABS: &str = "prefabs";
/// Where material sources live.
pub const MATERIALS: &str = "materials";
/// Where every other source lives: models, textures, sounds.
pub const ASSETS: &str = "assets";
/// Where built assets go. Derived from everything above, and never
/// committed: a clone builds it.
pub const LIBRARY: &str = "library";
/// Where the game's code lives.
pub const SRC: &str = "src";

/// Where a built game keeps its project data, beside the executable.
pub const DATA: &str = "data";

/// A file of the game's project, found the way a game has to find it: in a
/// build, in `data/` beside the executable (what `runity build` makes); in
/// development, in the project the game crate sits in — `dev_root` is the
/// crate's `env!("CARGO_MANIFEST_DIR")`.
///
/// The build is looked for first, so a shipped game never reaches for a
/// path that only existed on the machine it was compiled on.
pub fn data_file(dev_root: &str, relative: &str) -> PathBuf {
    let shipped = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(DATA).join(relative)))
        .filter(|path| path.exists());
    shipped.unwrap_or_else(|| Path::new(dev_root).join(relative))
}

/// Where a new project's game crate gets the engine from.
#[derive(Debug, Clone, PartialEq)]
pub enum Engine {
    /// A git repository — the default, until the engine is published.
    Git(String),
    /// A checkout on this machine: the `crates/runity` folder. Written into
    /// `Cargo.toml` relative to the project when it can be, so the project
    /// and the engine can move together.
    Path(PathBuf),
}

impl Default for Engine {
    fn default() -> Self {
        Engine::Git("https://github.com/stasiandr/runity".into())
    }
}

/// What `runity.ron` holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    /// The engine version the project was made with. Recorded now so that
    /// the day a format changes, there is something to migrate from.
    #[serde(default)]
    pub engine: String,
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
    /// No `runity.ron` in the folder, or in any folder above the path given.
    NotAProject(PathBuf),
    /// `runity.ron` is there and does not read.
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
                "{} is not in a runity project — no {FILE} there or in any folder above it",
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
        let text = match std::fs::read_to_string(&file) {
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
    /// `runity.ron`.
    ///
    /// What every tool does with the scene it was handed, so that
    /// `scene_shot scenes/camp.ron` and the editor opening the same file
    /// find the same prefabs, materials and library without being told.
    pub fn find(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref();
        let start = if path.is_dir() {
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
            if dir.join(FILE).is_file() {
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
        let root = root.as_ref().to_path_buf();
        if root.join(FILE).exists() {
            return Err(ProjectError::AlreadyAProject(root));
        }
        for dir in [SCENES, PREFABS, MATERIALS, ASSETS] {
            std::fs::create_dir_all(root.join(dir))?;
        }
        // Git keeps no empty folders, and a layout that disappears on the
        // first clone is not a layout.
        for dir in [PREFABS, MATERIALS, ASSETS] {
            std::fs::write(root.join(dir).join(".gitkeep"), "")?;
        }

        let manifest = Manifest {
            name: name.to_string(),
            engine: env!("CARGO_PKG_VERSION").to_string(),
        };
        let pretty = ron::ser::PrettyConfig::new();
        let text = ron::ser::to_string_pretty(&manifest, pretty)
            .map_err(|e| ProjectError::BadManifest(root.join(FILE), e.to_string()))?;
        std::fs::write(root.join(FILE), text + "\n")?;
        std::fs::write(root.join(".gitignore"), GITIGNORE)?;
        std::fs::write(root.join(".gitattributes"), GITATTRIBUTES)?;
        std::fs::write(root.join("CLAUDE.md"), CLAUDE_MD.replace("{name}", name))?;
        std::fs::write(root.join(SCENES).join("main.ron"), starter_scene())?;
        std::fs::create_dir_all(root.join(SRC))?;
        std::fs::write(root.join("Cargo.toml"), cargo_toml(&root, name, engine))?;
        std::fs::write(root.join(SRC).join("main.rs"), GAME.replace("{name}", name))?;

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

    pub fn scenes(&self) -> PathBuf {
        self.root.join(SCENES)
    }

    pub fn prefabs(&self) -> PathBuf {
        self.root.join(PREFABS)
    }

    pub fn materials(&self) -> PathBuf {
        self.root.join(MATERIALS)
    }

    pub fn assets(&self) -> PathBuf {
        self.root.join(ASSETS)
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

    /// The file a project-relative path names.
    pub fn resolve(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

const GITIGNORE: &str = "\
# Built from the sources and their .rimport sidecars; a clone rebuilds it.
/library/
/target/
# What `runity build` makes.
/build/
";

/// The `.gitattributes` lines that send scenes and prefabs to `runity
/// merge`, for a project made before they were in the template.
pub const MERGE_ATTRIBUTES: &str = "\
# Scenes and prefabs merge by entity and field, not by line. The driver is
# `runity merge`; `runity git-setup` turns it on in a clone.
scenes/**/*.ron merge=runity
prefabs/**/*.prefab merge=runity
";

/// The name Cargo will accept for a project called `name`.
fn crate_name(name: &str) -> String {
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

/// `to` as seen from `from`, both folders: `../engine/crates/runity`.
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

fn cargo_toml(root: &Path, name: &str, engine: &Engine) -> String {
    let source = match engine {
        Engine::Git(url) => format!("git = \"{url}\""),
        Engine::Path(path) => {
            let path = relative_path(root, path)
                .unwrap_or_else(|| path.to_string_lossy().replace('\\', "/"));
            format!("path = \"{path}\"")
        }
    };
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
runity = {{ {source}, features = [\"desktop-shell\"] }}
serde = {{ version = \"1\", features = [\"derive\"] }}

# The engine and every other dependency optimised even in a dev build, the
# game itself not: a frame that runs at speed, a rebuild that takes seconds.
# Line tables only: panics still name the line, and the link — most of a
# rebuild — is a third of the time. `runity rebuild-time` measures it.
[profile.dev]
debug = \"line-tables-only\"

[profile.dev.package.\"*\"]
opt-level = 3
",
        crate_name = crate_name(name),
    )
}

/// The game a new project starts with: a window on `scenes/main.ron` that
/// keeps up with the files.
const GAME: &str = r#"//! {name}.
//!
//! `cargo run` opens a window on `scenes/main.ron`. Save the scene, a
//! prefab, or re-import an asset while it runs, and the change is in the
//! next frames without the game losing its state. Game logic goes in
//! `step`; run under `dx serve --hotpatch` and a rebuilt `step` or `frame`
//! takes effect without closing the window.

use runity::hecs::World;
use runity::physics::PhysicsWorld;
use runity::render::Frame;
use runity::shell::{self, run, Context, WindowConfig};
use runity::{Components, Key, LiveScene, Transform};
use serde::Deserialize;

/// A component: a plain struct. A scene line gives it by the name it is
/// registered under in `main` —
/// `components: { "spin": (degrees_per_second: 45.0) }` — and changing
/// the number in the file while the game runs changes the speed.
#[derive(Deserialize)]
struct Spin {
    degrees_per_second: f32,
}

/// A system: a function over the world.
fn spin(world: &mut World, seconds: f32) {
    for (transform, spin) in world.query_mut::<(&mut Transform, &Spin)>() {
        transform.rotation_deg.y += spin.degrees_per_second * seconds;
    }
    runity::world::apply_hierarchy(world);
}

struct Game {
    live: LiveScene,
    world: World,
    physics: PhysicsWorld,
}

impl shell::Game for Game {
    fn start(&mut self, ctx: &mut Context) {
        self.physics = PhysicsWorld::new(ctx.time.settings().fixed_delta);
        for line in self.live.spawn(&mut self.world, ctx.gpu, ctx.renderer).lines() {
            eprintln!("{line}");
        }
    }

    /// Fixed-step game logic: the systems, in order.
    fn step(&mut self, ctx: &mut Context) {
        let seconds = ctx.time.settings().fixed_delta;
        spin(&mut self.world, seconds);
        // Physics is a system too: bodies from the scene, a fixed step, and
        // where the dynamic ones went written back.
        self.physics.run(&mut self.world);
    }

    fn frame(&mut self, ctx: &mut Context) -> Frame {
        if ctx.input.pressed(Key::Escape) {
            ctx.quit();
        }
        let reload = self.live.poll(ctx.time.delta(), &mut self.world, ctx.gpu, ctx.renderer);
        for line in reload.lines() {
            eprintln!("{line}");
        }
        let scene = self.live.scene();
        runity::build_frame(
            &self.world,
            runity::scene_camera(&scene.view),
            runity::scene_lighting(&scene.sun),
            runity::scene_fog(&scene.fog),
        )
    }
}

fn main() -> anyhow::Result<()> {
    // `data/` beside the executable in a build, the project in development.
    let scene = runity::project::data_file(env!("CARGO_MANIFEST_DIR"), "scenes/main.ron");
    let mut components = Components::new();
    components.register::<Spin>("spin");
    let (live, problems) = LiveScene::open(&scene)?;
    let live = live.with_components(components);
    for problem in &problems {
        eprintln!("{problem}");
    }
    let config = WindowConfig {
        title: "{name}".into(),
        ..Default::default()
    };
    let game = Game {
        live,
        world: World::new(),
        physics: PhysicsWorld::default(),
    };
    run(config, game)
}
"#;

/// Binary sources go to LFS and are lockable from the first commit
/// (DNA, postulate 2). Two people editing the same texture cannot merge it,
/// so the second one should find out before they start, not after.
///
/// Text is checked out with LF everywhere, because sidecars store a hash of
/// the source's bytes: a Windows checkout that turned every `.obj` into
/// CRLF would rebuild the whole library and rewrite every sidecar.
const GITATTRIBUTES: &str = "\
# Text as LF on every machine: .rimport stores a hash of the source's bytes.
* text=auto eol=lf

# Scenes and prefabs merge by entity and field, not by line. The driver is
# `runity merge`; `runity git-setup` turns it on in a clone.
scenes/**/*.ron merge=runity
prefabs/**/*.prefab merge=runity

# Binary sources: stored in LFS, lockable. Text sources (.ron, .prefab,
# .rmat, .rimport, .obj, .gltf) stay in git, where a diff means something.
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

A runity project. The layout is fixed — every tool, and every agent, finds
things in the same place:

```
runity.ron   the project file
scenes/      scenes, RON — one entity per block, `id` first
prefabs/     one entity subtree per file; a scene places it with `prefab: \"name\"`
materials/   .rmat sources: `(color: \"#rrggbb\")`, sRGB hex
assets/      models, textures, sounds; each source gets a .rimport beside it
library/     built assets — derived, never committed
Cargo.toml   the game crate; src/main.rs is the game
```

* `cargo run` plays `scenes/main.ron`. It keeps running while you edit:
  saved scenes, prefabs and rebuilt assets show up in the window.
* `runity check` says what does not resolve — a model, a material or a
  prefab nobody has, a repeated id, a stale sidecar — with the file and the
  entity. Run it after editing scenes; it exits non-zero on errors.
* `runity git-setup`, once per clone, turns on the merge driver: scenes
  and prefabs then merge by entity and field, and a real conflict is
  reported in words (\"both changed the position of `tree`\") with ours
  kept and the file still loading.
* `runity build` makes a folder to ship: the game in release, and `data/`
  beside it with the scenes, prefabs and built library — no sources.
* `runity sync` builds `library/` from the sources. After adding, changing
  or moving a source, run it and commit the `.rimport` it writes beside the
  source. Never edit a sidecar's `hash` or `id` by hand: the hash is how a
  moved file is found, the id is what the library refers to.
* Every entity has an `id`: sixteen hex digits, unique in its file. When
  writing one by hand, leave it out and the engine assigns one on load;
  never copy an existing one.
* The editor is also an MCP server: with runity's `runity-mcp` on the PATH
  (`cargo install --path crates/runity-mcp` in the engine),
  `claude mcp add runity -- runity-mcp` gives every editor operation as a
  tool — open, add, move, undo, render a frame to look at, check, simulate.
* Game logic is Rust in `src/`, on the ECS (`runity::hecs`): components are
  plain structs, systems are functions over the world, and an entity
  spawned from a scene carries `SceneId`. A scene line gives an entity the
  game's components by the name `main` registers them under:
  `components: { \"spin\": (degrees_per_second: 45.0) }`. Editing a value
  while the game runs changes that component and nothing else.
* Everything a person makes is text and is committed; `library/` and
  `target/` are not.
* Binary sources are in Git LFS and lockable — lock before editing one.
";

/// The scene a new project opens to: ground to stand on and one thing on
/// it, so the first frame shows that everything works.
fn starter_scene() -> String {
    let (ground, cube, crate_) = (
        crate::EntityId::fresh(),
        crate::EntityId::fresh(),
        crate::EntityId::fresh(),
    );
    format!(
        "(
    entities: [
        (id: \"{ground}\", name: \"ground\", model: \"builtin:plane\", transform: (scale: (40.0, 1.0, 40.0)), material: \"grass\", body: Static, collider: Box(half: (0.5, 0.05, 0.5))),
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
        let dir = std::env::temp_dir().join(format!("runity-project-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_new_project_is_laid_out_to_the_standard_and_opens() {
        let root = temp("new");
        let project = Project::create(&root, "moss").unwrap();
        assert_eq!(project.name(), "moss");
        for dir in [SCENES, PREFABS, MATERIALS, ASSETS] {
            assert!(root.join(dir).is_dir(), "{dir}/");
        }
        for file in [
            FILE,
            ".gitignore",
            ".gitattributes",
            "CLAUDE.md",
            "Cargo.toml",
            "src/main.rs",
        ] {
            assert!(root.join(file).is_file(), "{file}");
        }
        // The library is derived: ignored, and not made until something is
        // built into it.
        assert!(std::fs::read_to_string(root.join(".gitignore"))
            .unwrap()
            .contains("/library/"));

        // And the scene it starts with opens, with every entity named.
        let scene = crate::Scene::load(root.join("scenes/main.ron")).unwrap();
        assert_eq!(scene.entities.len(), 3);
        let text = std::fs::read_to_string(root.join("scenes/main.ron")).unwrap();
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
        let engine = std::env::temp_dir().join("runity-project-engine/crates/runity");
        Project::create_with(&root, "My Game", &Engine::Path(engine)).unwrap();
        let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(cargo.contains("name = \"my_game\""), "{cargo}");
        assert!(
            cargo.contains("path = \"../runity-project-engine/crates/runity\""),
            "relative, so the two can move together: {cargo}"
        );
        assert!(cargo.contains("[workspace]"), "its own workspace: {cargo}");
        let main = std::fs::read_to_string(root.join("src/main.rs")).unwrap();
        assert!(main.contains("title: \"My Game\""), "{main}");

        let default = Project::create(temp("engine-git"), "7 seas").unwrap();
        let cargo = std::fs::read_to_string(default.root().join("Cargo.toml")).unwrap();
        assert!(cargo.contains("git = \"https://"), "{cargo}");
        assert!(cargo.contains("name = \"game_7_seas\""), "{cargo}");
    }

    #[test]
    fn making_a_project_where_one_is_refuses_rather_than_overwriting() {
        let root = temp("twice");
        Project::create(&root, "once").unwrap();
        std::fs::write(root.join("scenes/main.ron"), "(entities: [])").unwrap();
        assert!(matches!(
            Project::create(&root, "twice"),
            Err(ProjectError::AlreadyAProject(_))
        ));
        assert_eq!(
            std::fs::read_to_string(root.join("scenes/main.ron")).unwrap(),
            "(entities: [])",
            "and nothing was touched"
        );
    }

    #[test]
    fn a_project_is_found_from_anything_inside_it() {
        let root = temp("find");
        let project = Project::create(&root, "found").unwrap();
        std::fs::create_dir_all(root.join("scenes/caves")).unwrap();
        let deep = root.join("scenes/caves/deep.ron");
        std::fs::write(&deep, "(entities: [])").unwrap();

        assert_eq!(Project::find(&deep).unwrap(), project);
        assert_eq!(Project::find(root.join("scenes")).unwrap(), project);

        // And outside one, it says so in a sentence.
        let lost = std::env::temp_dir().join("runity-project-nowhere/scene.ron");
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
}
