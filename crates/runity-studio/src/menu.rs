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
    Save,
    ReloadAssets,
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
    ToggleGrid,
    ToggleColliders,
    Tool(Tool),
    ToggleSpace,
    TogglePivot,
    Play,
    /// The Game view (`true`) or the Scene view.
    GameView(bool),
    Pause,
    Step,
    SetField(String, String),
    /// A prefab or model placed in front of the view.
    Place(String),
    ClearConsole,
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
                item("Save", Action::Save).key(key!("⌘S", "Ctrl+S")),
                MenuItem::separator(),
                item("Reload Assets", Action::ReloadAssets),
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
        ("GameObject", create_items(true)),
        (
            "View",
            vec![
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
        item("Apply Overrides", Action::ApplyOverrides),
        item("Revert Overrides", Action::RevertOverrides),
        item("Unpack Prefab", Action::Unpack),
        MenuItem::separator(),
        item("Create Empty", Action::CreateEmpty),
        item("Cube", Action::Create("builtin:cube")),
    ]
}
