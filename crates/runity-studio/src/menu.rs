//! Menus: what the editor can be asked to do, by name.
//!
//! Every menu entry, toolbar button and context-menu line is an [`Action`],
//! and `Studio::run` is the one place an action turns into session calls —
//! so the menu bar, a right click and a test all do the same thing.
//! Shortcuts are shown next to entries; the Scene view's own keyboard
//! handling (`Session::scene_view`) is what answers them, so the label and
//! the key cannot disagree about what happens.

use std::path::PathBuf;

use runity::gizmo::Tool;
use runity_editor::Side;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    NewScene,
    OpenScene(PathBuf),
    /// Ask for a scene file and open it.
    OpenSceneDialog,
    /// Ask where, and save the scene there.
    SaveAs,
    /// Ask for files and import them into the project.
    Import,
    OpenPrefab(String),
    /// Save the prefab and go back to the scene it was opened from.
    ExitPrefab,
    Save,
    ReloadAssets,
    /// `runity check`, its findings in the Console.
    CheckProject,
    /// `runity build`, in the background; `true` runs what it built.
    Build(bool),
    StartGame,
    StopGame,
    Undo,
    Redo,
    Copy,
    Paste,
    Duplicate,
    Delete,
    SelectAll,
    SelectNone,
    Rename,
    Frame,
    DropToGround,
    SnapToGrid,
    CreateEmpty,
    /// An empty entity under the selection.
    CreateChild,
    Group,
    Create(&'static str),
    CreateLight,
    CreateCamera,
    MakePrefab,
    ApplyOverrides,
    RevertOverrides,
    Unpack,
    Hide,
    Isolate,
    ShowAll,
    View(Side),
    Perspective,
    /// Perspective or orthographic, whichever it is not.
    ToggleOrtho,
    ToggleGrid,
    ToggleColliders,
    ToggleSnap,
    Tool(Tool),
    ToggleSpace,
    TogglePivot,
    Play,
    /// The Game view (`true`) or the Scene view.
    GameView(bool),
    Pause,
    Step,
    SetField(String, String),
    /// One field of a game component, by component, field and RON value.
    SetSub(String, String, String),
    /// A prefab or model placed in front of the view.
    Place(String),
    ClearConsole,
    /// A Project entry, by its project-relative file: renamed or copied
    /// under a name asked for, deleted, shown in the file manager.
    AssetRename(String),
    AssetDuplicate(String),
    AssetDelete(String),
    AssetReveal(String),
    /// Ask for a name, then write `src/components/NAME.rs` or
    /// `src/systems/NAME.rs` (`runity add`).
    NewComponent,
    NewSystem,
    /// Ask for a name and save the selection's colour as a material.
    SaveMaterial,
    /// Ask for a name and save the selected instance as a prefab variant.
    MakeVariant,
    /// Ask for the snap steps.
    SnapSettings,
    /// Show where a walker can go, or stop.
    ToggleNavigation,
    /// One field of the selection: back to a new entity's, its value to
    /// the clipboard, the clipboard's value into it, gone.
    FieldReset(String),
    FieldCopy(String),
    FieldPaste(String),
    FieldRemove(String),
    /// The Game view's shape: free, or a width to height.
    Aspect(Option<(u32, u32)>),
    /// Open Quick Search.
    Search,
    /// Listen to a sound asset, or stop.
    PlaySound(String),
    StopSound,
    /// A game component by name onto the selection.
    AddComponent(String),
    /// Blockout: a floor or a wall drawn as a Poly Shape in front of the view.
    PolyFloor,
    PolyWall,
    /// Push one face of the selection out (or in) by metres.
    PushFace(runity::edit::Face, f32),
    /// Copies of the selection in a row along X, its own width apart.
    Array(usize),
    /// The selection's model or prefab scattered around the view's centre.
    Scatter,
    /// Show or hide a dock: 0 the left, 1 the right, 2 the one under the
    /// view.
    TogglePanel(usize),
    /// The Scene view over the whole window, or back (Shift Space).
    Maximize,
    /// A flat terrain to sculpt, and the brush on it.
    NewTerrain,
    ToggleSculpt,
    /// Face mode: point at a face of a box to outline it, drag to push it.
    ToggleFaces,
    /// Line the selection up along an axis.
    Align(usize, runity_editor::Align),
}

/// One line of a menu: a label, the key that does the same, what it does.
/// No action: a separator.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub shortcut: Option<&'static str>,
    pub action: Option<Action>,
}

impl MenuItem {
    pub fn new(label: &str, action: Action) -> Self {
        Self {
            label: label.to_string(),
            shortcut: None,
            action: Some(action),
        }
    }

    pub fn key(mut self, shortcut: &'static str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    pub fn separator() -> Self {
        Self {
            label: String::new(),
            shortcut: None,
            action: None,
        }
    }
}

fn item(label: &str, action: Action) -> MenuItem {
    MenuItem::new(label, action)
}

/// The shortcut key, as the platform writes it.
const CMD: bool = cfg!(target_os = "macos");

macro_rules! key {
    ($mac:expr, $other:expr) => {
        if CMD {
            $mac
        } else {
            $other
        }
    };
}

/// The menu bar: its titles and what each opens.
pub fn menu_bar() -> Vec<(&'static str, Vec<MenuItem>)> {
    vec![
        (
            "File",
            vec![
                item("New Scene", Action::NewScene),
                item("Open Scene…", Action::OpenSceneDialog),
                item("Save", Action::Save).key(key!("⌘S", "Ctrl+S")),
                item("Save As…", Action::SaveAs),
                MenuItem::separator(),
                item("Import…", Action::Import),
                item("Reload Assets", Action::ReloadAssets),
                item("Check Project", Action::CheckProject),
                MenuItem::separator(),
                item("Build", Action::Build(false)),
                item("Build and Run", Action::Build(true)),
                MenuItem::separator(),
                item("Run Game", Action::StartGame),
                item("Stop Game", Action::StopGame),
            ],
        ),
        (
            "Edit",
            vec![
                item("Undo", Action::Undo).key(key!("⌘Z", "Ctrl+Z")),
                item("Redo", Action::Redo).key(key!("⇧⌘Z", "Ctrl+Y")),
                MenuItem::separator(),
                item("Copy", Action::Copy).key(key!("⌘C", "Ctrl+C")),
                item("Paste", Action::Paste).key(key!("⌘V", "Ctrl+V")),
                item("Duplicate", Action::Duplicate).key(key!("⌘D", "Ctrl+D")),
                item("Delete", Action::Delete).key("Delete"),
                item("Rename", Action::Rename).key("F2"),
                MenuItem::separator(),
                item("Select All", Action::SelectAll).key(key!("⌘A", "Ctrl+A")),
                item("Select None", Action::SelectNone).key("Esc"),
                MenuItem::separator(),
                item("Frame Selected", Action::Frame).key("F"),
                item("Drop to Ground", Action::DropToGround).key("End"),
                item("Snap to Grid", Action::SnapToGrid),
            ],
        ),
        (
            "Assets",
            vec![
                item("Create Component…", Action::NewComponent),
                item("Create System…", Action::NewSystem),
                item("Save Material from Selection…", Action::SaveMaterial),
                MenuItem::separator(),
                item("Import…", Action::Import),
                item("Refresh", Action::ReloadAssets),
                item("Check Project", Action::CheckProject),
            ],
        ),
        ("GameObject", create_items(true)),
        (
            "View",
            vec![
                item("Search…", Action::Search).key(key!("⌘K", "Ctrl+K")),
                MenuItem::separator(),
                item("Scene View", Action::GameView(false)),
                item("Game View", Action::GameView(true)),
                MenuItem::separator(),
                item("Perspective", Action::Perspective),
                item("Top", Action::View(Side::Top)),
                item("Front", Action::View(Side::Front)),
                item("Right", Action::View(Side::Right)),
                item("Back", Action::View(Side::Back)),
                item("Left", Action::View(Side::Left)),
                MenuItem::separator(),
                item("Hide Selection", Action::Hide).key("H"),
                item("Isolate Selection", Action::Isolate).key("⇧H"),
                item("Show All", Action::ShowAll),
                MenuItem::separator(),
                item("Grid", Action::ToggleGrid),
                item("Colliders", Action::ToggleColliders),
                item("Snap", Action::ToggleSnap),
                item("Snap Settings…", Action::SnapSettings),
                item("Navigation", Action::ToggleNavigation),
            ],
        ),
        (
            "Tools",
            vec![
                item("Poly Shape: Floor", Action::PolyFloor),
                item("Poly Shape: Wall", Action::PolyWall),
                item("Sculpt Terrain (brush)", Action::ToggleSculpt),
                item("Face Mode (drag a face)", Action::ToggleFaces),
                MenuItem::separator(),
                item(
                    "Push Top +0.5",
                    Action::PushFace(runity::edit::Face::PosY, 0.5),
                ),
                item(
                    "Pull Top −0.5",
                    Action::PushFace(runity::edit::Face::PosY, -0.5),
                ),
                item(
                    "Push Right +0.5",
                    Action::PushFace(runity::edit::Face::PosX, 0.5),
                ),
                item(
                    "Push Left +0.5",
                    Action::PushFace(runity::edit::Face::NegX, 0.5),
                ),
                item(
                    "Push Front +0.5",
                    Action::PushFace(runity::edit::Face::PosZ, 0.5),
                ),
                item(
                    "Push Back +0.5",
                    Action::PushFace(runity::edit::Face::NegZ, 0.5),
                ),
                MenuItem::separator(),
                item("Array: 4 copies along X", Action::Array(4)),
                item("Scatter 20 around the view", Action::Scatter),
                MenuItem::separator(),
                item(
                    "Align X centres",
                    Action::Align(0, runity_editor::Align::Center),
                ),
                item("Align bottoms", Action::Align(1, runity_editor::Align::Min)),
                item(
                    "Align Z centres",
                    Action::Align(2, runity_editor::Align::Center),
                ),
            ],
        ),
        (
            "Window",
            vec![
                item("Left Dock", Action::TogglePanel(0)),
                item("Right Dock", Action::TogglePanel(1)),
                item("Bottom Dock", Action::TogglePanel(2)),
                MenuItem::separator(),
                item("Maximize the View", Action::Maximize).key("⇧Space"),
            ],
        ),
        (
            "Play",
            vec![
                item("Play / Stop", Action::Play).key(key!("⌘P", "Ctrl+P")),
                item("Pause", Action::Pause).key(key!("⇧⌘P", "Ctrl+Shift+P")),
                item("Step", Action::Step).key(key!("⌥⌘P", "Ctrl+Alt+P")),
            ],
        ),
    ]
}

fn create_items(with_group: bool) -> Vec<MenuItem> {
    let mut v = vec![item("Create Empty", Action::CreateEmpty).key(key!("⇧⌘N", "Ctrl+Shift+N"))];
    if with_group {
        v.push(item("Group Selection", Action::Group).key(key!("⇧⌘G", "Ctrl+Shift+G")));
    }
    v.push(MenuItem::separator());
    for (label, model) in [
        ("Cube", "builtin:cube"),
        ("Sphere", "builtin:sphere"),
        ("Plane", "builtin:plane"),
        ("Cylinder", "builtin:cylinder"),
        ("Cone", "builtin:cone"),
        ("Ramp", "builtin:ramp"),
        ("Stairs", "builtin:stairs"),
    ] {
        v.push(item(label, Action::Create(model)));
    }
    v.push(MenuItem::separator());
    v.push(item("Terrain", Action::NewTerrain));
    v.push(item("Light", Action::CreateLight));
    v.push(item("Camera", Action::CreateCamera));
    v
}

/// A right click on nothing in the Hierarchy: make something.
pub fn create_menu() -> Vec<MenuItem> {
    create_items(false)
}

/// A right click on a line of the Hierarchy.
pub fn context_menu() -> Vec<MenuItem> {
    vec![
        item("Rename", Action::Rename).key("F2"),
        item("Duplicate", Action::Duplicate).key(key!("⌘D", "Ctrl+D")),
        item("Delete", Action::Delete).key("Delete"),
        item("Copy", Action::Copy),
        item("Paste", Action::Paste),
        MenuItem::separator(),
        item("Frame Selected", Action::Frame).key("F"),
        item("Group Selection", Action::Group),
        item("Hide", Action::Hide).key("H"),
        item("Isolate", Action::Isolate),
        MenuItem::separator(),
        item("Make Prefab", Action::MakePrefab),
        item("Make Prefab Variant…", Action::MakeVariant),
        item("Apply Overrides", Action::ApplyOverrides),
        item("Revert Overrides", Action::RevertOverrides),
        item("Unpack Prefab", Action::Unpack),
        MenuItem::separator(),
        item("Create Empty Child", Action::CreateChild),
        item("Create Empty", Action::CreateEmpty),
        item("Cube", Action::Create("builtin:cube")),
    ]
}
