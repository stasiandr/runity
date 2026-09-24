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
    /// An editor action by its name (`runity_editor::actions`): the one
    /// the agent calls as a tool, with its label and key from there.
    Editor(&'static str),
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
    ReloadAssets,
    /// `runity check`, its findings in the Console.
    CheckProject,
    /// `runity build`, in the background; `true` runs what it built.
    Build(bool),
    StartGame,
    StopGame,
    Copy,
    Paste,
    SelectAll,
    SelectNone,
    Rename,
    Frame,
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
    /// A panel into a window of its own (a tab's right-click menu).
    Float(crate::dock::Panel),
    /// Every floating panel back into the docks.
    DockAll,
    /// During play: keep the selection where the simulation puts it.
    KeepSimulation,
    /// The foliage brush: paint the chosen model onto the ground.
    ToggleFoliage,
    /// A material that is another one with nothing changed yet.
    MaterialInstance(String),
    /// Put the runity add-on into Blender, turned on (docs/blender.md).
    InstallBlenderPlugin,
    /// A colour preset by name (`theme::PRESETS`), as the person's choice.
    Theme(&'static str),
    /// Settings, on its Appearance page.
    Appearance,
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

/// An editor action's item, as the registry has it: its label and key are
/// the ones the agent's tool of the same name answers to.
fn editor(name: &'static str) -> MenuItem {
    let action = runity_editor::actions::find(name).expect("a registered editor action");
    let item = MenuItem::new(action.label, Action::Editor(name));
    match action.key() {
        Some(key) => item.key(key),
        None => item,
    }
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
                editor("save_scene"),
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
                editor("undo"),
                editor("redo"),
                MenuItem::separator(),
                item("Copy", Action::Copy).key(key!("⌘C", "Ctrl+C")),
                item("Paste", Action::Paste).key(key!("⌘V", "Ctrl+V")),
                editor("duplicate_entity"),
                editor("delete_entity"),
                item("Rename", Action::Rename).key("F2"),
                MenuItem::separator(),
                item("Select All", Action::SelectAll).key(key!("⌘A", "Ctrl+A")),
                item("Select None", Action::SelectNone).key("Esc"),
                MenuItem::separator(),
                item("Frame Selected", Action::Frame).key("F"),
                editor("drop_to_ground"),
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
                item("Install Blender Plugin…", Action::InstallBlenderPlugin),
                item("Refresh", Action::ReloadAssets),
                item("Check Project", Action::CheckProject),
            ],
        ),
        ("Entity", create_items(true)),
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
                editor("isolate"),
                item("Show All", Action::ShowAll),
                MenuItem::separator(),
                item("Grid", Action::ToggleGrid),
                item("Colliders", Action::ToggleColliders),
                item("Snap", Action::ToggleSnap),
                item("Snap Settings…", Action::SnapSettings),
                item("Navigation", Action::ToggleNavigation),
                MenuItem::separator(),
                // A submenu, «Theme ›», where the menu bar has them: the
                // label's first part is its title.
                item("Theme › Appearance Settings…", Action::Appearance),
            ]
            .into_iter()
            .chain(
                crate::theme::PRESETS
                    .iter()
                    .map(|p| item(&format!("Theme › {}", p.label), Action::Theme(p.name))),
            )
            .collect(),
        ),
        (
            "Window",
            vec![
                item("Left Dock", Action::TogglePanel(0)),
                item("Right Dock", Action::TogglePanel(1)),
                item("Bottom Dock", Action::TogglePanel(2)),
                MenuItem::separator(),
                item("Maximize the View", Action::Maximize).key("⇧Space"),
                MenuItem::separator(),
                item(
                    "Float the Inspector",
                    Action::Float(crate::dock::Panel::Inspector),
                ),
                item(
                    "Float the Project",
                    Action::Float(crate::dock::Panel::Project),
                ),
                item("Dock All Floating Panels", Action::DockAll),
            ],
        ),
        (
            "Play",
            vec![
                item("Play / Stop", Action::Play).key(key!("⌘P", "Ctrl+P")),
                item("Pause", Action::Pause).key(key!("⇧⌘P", "Ctrl+Shift+P")),
                item("Step", Action::Step).key(key!("⌥⌘P", "Ctrl+Alt+P")),
                item("Keep Simulation Changes", Action::KeepSimulation).key("K"),
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
        editor("duplicate_entity"),
        editor("delete_entity"),
        item("Copy", Action::Copy),
        item("Paste", Action::Paste),
        MenuItem::separator(),
        item("Frame Selected", Action::Frame).key("F"),
        item("Group Selection", Action::Group),
        item("Hide", Action::Hide).key("H"),
        item("Isolate", Action::Editor("isolate")),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An action is registered once and seen by the menu and the agent:
    /// every one of the registry's stands in the menu it names.
    #[test]
    fn every_editor_action_is_in_its_menu() {
        let bar = menu_bar();
        for action in runity_editor::actions::registry() {
            let (_, items) = bar
                .iter()
                .find(|(title, _)| *title == action.menu)
                .unwrap_or_else(|| panic!("no menu {} for {}", action.menu, action.name));
            assert!(
                items.iter().any(|i| i.action == Some(Action::Editor(action.name))),
                "{} is not in {}",
                action.name,
                action.menu
            );
        }
    }
}
