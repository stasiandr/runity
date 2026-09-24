//! Menus: what the editor can be asked to do, by name.
//!
//! Every menu entry, toolbar button and context-menu line is an [`Action`],
//! and `Studio::run` is the one place an action turns into session calls —
//! so the menu bar, a right click and a test all do the same thing.
//! Shortcuts are shown next to entries, and they come from the keymap
//! (`crate::keymap`) the studio answers keys from, so the label and the
//! key cannot disagree about what happens — in macOS's menu bar too
//! (`crate::native_menu`), and after a key is rebound in Preferences.

use std::path::PathBuf;

use scrap::gizmo::Tool;
use scrap_editor::Side;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// An editor action by its name (`scrap_editor::actions`): the one
    /// the agent calls as a tool, with its label and key from there.
    Editor(&'static str),
    NewScene,
    OpenScene(PathBuf),
    /// Ask for a scene file and open it.
    OpenSceneDialog,
    /// Ask where, and save the scene there.
    SaveAs,
    /// Throw the edits away: the scene as its file has it (one undo step).
    ReloadScene,
    /// Show a file's tile in the Project panel.
    ShowInProject(PathBuf),
    /// Ask for files and import them into the project.
    Import,
    OpenPrefab(String),
    /// Save the prefab and go back to the scene it was opened from.
    ExitPrefab,
    ReloadAssets,
    /// `scrap check`, its findings in the Console.
    CheckProject,
    /// `scrap build`, in the background; `true` runs what it built.
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
    /// The hand (Q): the view moves, nothing is picked.
    Hand,
    ToggleSpace,
    TogglePivot,
    /// Play the game — its own code, in its own window — or stop it.
    Play,
    /// Simulate the scene's physics right here in the view, without the
    /// game's code: what falls, falls. Stop puts it all back.
    Simulate,
    /// The Game view (`true`) or the Scene view.
    GameView(bool),
    Pause,
    Step,
    SetField(String, String),
    /// One field of a game component, by component, field and RON value.
    SetSub(String, String, String),
    /// One place in a field's value, picked from a form's list: the RON
    /// written there.
    SetLeaf(crate::inspector::Place, String),
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
    /// `src/systems/NAME.rs` (`scrap add`).
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
    /// Put the scrap add-on into Blender, turned on (docs/blender.md).
    InstallBlenderPlugin,
    /// Apply a layout preset, built in or saved, by name.
    Layout(String),
    SaveLayoutAs,
    DeleteLayout,
    /// Bring a panel up: its tab on top, or back in a dock when closed.
    ShowPanel(crate::dock::Panel),
    /// A panel's stack over the whole window, or back.
    MaximizePanel(crate::dock::Panel),
    CloseTab(crate::dock::Panel),
    /// The Inspector's Normal (`false`) or Debug (`true`) mode.
    InspectorDebug(bool),
    /// The Console's Clear on Play, on or off.
    ToggleClearOnPlay,
    /// A colour preset by name (`theme::PRESETS`), as the person's choice.
    Theme(&'static str),
    /// Preferences — the person's, in every project — in their window, on
    /// a page or where they were last.
    Preferences(Option<crate::preferences::Page>),
    /// Project Settings: the project's own files, in git.
    ProjectSettings,
    /// The selection to where the view is looking from, or turned to look
    /// the way the view does (Unity's GameObject menu).
    MoveToView,
    AlignWithView,
}

/// One line of a menu: a label, the key that does the same, what it does.
/// No action: a separator.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    /// As this platform writes it: "⇧⌘Z", "Ctrl+Shift+Z".
    pub shortcut: Option<String>,
    pub action: Option<Action>,
    /// Shown, greyed, and not clickable: what is coming.
    pub disabled: bool,
    /// A choice that is on or off: `Some(true)` draws a check by it.
    pub checked: Option<bool>,
}

impl MenuItem {
    pub fn new(label: &str, action: Action) -> Self {
        Self {
            label: label.to_string(),
            shortcut: None,
            action: Some(action),
            disabled: false,
            checked: None,
        }
    }

    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    /// A choice that is on or off.
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }

    pub fn key(mut self, shortcut: &str) -> Self {
        self.shortcut = Some(shortcut.to_string());
        self
    }

    pub fn separator() -> Self {
        Self {
            label: String::new(),
            shortcut: None,
            action: None,
            disabled: false,
            checked: None,
        }
    }
}

fn item(label: &str, action: Action) -> MenuItem {
    MenuItem::new(label, action)
}

/// An editor action's item, as the registry has it: its label is the one
/// the agent's tool of the same name answers to, and its key the keymap's.
fn editor(name: &'static str) -> MenuItem {
    let action = scrap_editor::actions::find(name).expect("a registered editor action");
    MenuItem::new(action.label, Action::Editor(name))
}

/// The menu bar: its titles and what each opens, with the default keys.
/// The studio shows the person's (`Studio::menu_bar`).
pub fn menu_bar() -> Vec<(&'static str, Vec<MenuItem>)> {
    crate::keymap::Keymap::default().label_bar(bare_menu_bar())
}

/// The menu bar without keys: what they are is the keymap's.
pub fn bare_menu_bar() -> Vec<(&'static str, Vec<MenuItem>)> {
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
                item("Copy", Action::Copy),
                item("Paste", Action::Paste),
                editor("duplicate_entity"),
                editor("delete_entity"),
                item("Rename", Action::Rename),
                MenuItem::separator(),
                item("Select All", Action::SelectAll),
                item("Select None", Action::SelectNone),
                MenuItem::separator(),
                item("Frame Selected", Action::Frame),
                item("Move to View", Action::MoveToView),
                item("Align with View", Action::AlignWithView),
                editor("drop_to_ground"),
                item("Snap to Grid", Action::SnapToGrid),
                MenuItem::separator(),
                // Unity's two: the project's, in git, and the person's,
                // in every project (on a Mac, «scrap › Settings…»).
                item("Project Settings…", Action::ProjectSettings),
                item("Preferences…", Action::Preferences(None)),
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
                item("Search…", Action::Search),
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
                item("Hide Selection", Action::Hide),
                editor("isolate"),
                item("Show All", Action::ShowAll),
                MenuItem::separator(),
                item("Grid", Action::ToggleGrid),
                item("Colliders", Action::ToggleColliders),
                item("Snap", Action::ToggleSnap),
                item("Snap Settings…", Action::SnapSettings),
                item("Navigation", Action::ToggleNavigation),
                MenuItem::separator(),
                // The Scene view's tools, where their keys can be seen.
                item("Tool › Hand", Action::Hand),
                item("Tool › Move", Action::Tool(Tool::Move)),
                item("Tool › Rotate", Action::Tool(Tool::Rotate)),
                item("Tool › Scale", Action::Tool(Tool::Scale)),
                item("Tool › Rect", Action::Tool(Tool::Rect)),
                item("Tool › Transform", Action::Tool(Tool::Transform)),
                item("Tool › Global / Local", Action::ToggleSpace),
                item("Tool › Pivot / Center", Action::TogglePivot),
                MenuItem::separator(),
                // Quick switches between presets; the rest of the colours
                // are Preferences › Appearance.
                item(
                    "Theme › Theme Settings…",
                    Action::Preferences(Some(crate::preferences::Page::Appearance)),
                ),
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
                item("Maximize the View", Action::Maximize),
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
                MenuItem::separator(),
                // The studio lists the layouts a person saved after these.
                item("Layout: Default", Action::Layout("Default".into())),
                item("Layout: Tall", Action::Layout("Tall".into())),
                item("Save Layout As…", Action::SaveLayoutAs),
                item("Delete Layout…", Action::DeleteLayout),
                MenuItem::separator(),
                item("Hierarchy", Action::ShowPanel(crate::dock::Panel::Hierarchy)),
                item("Inspector", Action::ShowPanel(crate::dock::Panel::Inspector)),
                item("Project", Action::ShowPanel(crate::dock::Panel::Project)),
                item("Console", Action::ShowPanel(crate::dock::Panel::Console)),
                item("History", Action::ShowPanel(crate::dock::Panel::History)),
                item("Git", Action::ShowPanel(crate::dock::Panel::Git)),
                item("Animation", Action::ShowPanel(crate::dock::Panel::Animation)),
                item("Animator", Action::ShowPanel(crate::dock::Panel::Animator)),
                item("Dialogues", Action::ShowPanel(crate::dock::Panel::Dialogues)),
                item("UI Builder", Action::ShowPanel(crate::dock::Panel::Screens)),
                item(
                    "Project Settings",
                    Action::ShowPanel(crate::dock::Panel::Settings),
                ),
                item("Profiler", Action::ShowPanel(crate::dock::Panel::Profiler)),
            ],
        ),
        (
            "Play",
            vec![
                item("Play / Stop", Action::Play),
                MenuItem::separator(),
                item("Simulate Physics Here", Action::Simulate),
                item("Pause", Action::Pause),
                item("Step", Action::Step),
                item("Keep Simulation Changes", Action::KeepSimulation),
            ],
        ),
    ]
}

fn create_items(with_group: bool) -> Vec<MenuItem> {
    let mut v = vec![item("Create Empty", Action::CreateEmpty)];
    if with_group {
        v.push(item("Group Selection", Action::Group));
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
    if with_group {
        // Blockout: the first picked, less the rest.
        v.push(editor("carve"));
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

/// The ⋮ of the Hierarchy's scene line: the document as a whole — its
/// file, when it has one, and whether it is a prefab being edited.
pub fn scene_menu(path: Option<PathBuf>, prefab: bool) -> Vec<MenuItem> {
    let mut save = editor("save_scene");
    save.label = format!("Save {}", if prefab { "Prefab" } else { "Scene" });
    let mut v = vec![save];
    if !prefab {
        v.push(item("Save Scene As…", Action::SaveAs));
    }
    if let Some(path) = path {
        v.push(item("Show in Project", Action::ShowInProject(path)));
        v.push(item("Reload from Disk", Action::ReloadScene));
    }
    v.push(MenuItem::separator());
    // Several scenes open at once: not yet, and said so where it will be.
    v.push(item("Add Scene (multi-scene — later)", Action::OpenSceneDialog).disabled());
    v
}

/// A right click on a line of the Hierarchy.
pub fn context_menu() -> Vec<MenuItem> {
    vec![
        item("Rename", Action::Rename),
        editor("duplicate_entity"),
        editor("delete_entity"),
        item("Copy", Action::Copy),
        item("Paste", Action::Paste),
        MenuItem::separator(),
        item("Frame Selected", Action::Frame),
        item("Group Selection", Action::Group),
        item("Hide", Action::Hide),
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
        for action in scrap_editor::actions::registry() {
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
