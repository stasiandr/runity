//! The editor, without its window.
//!
//! `Studio` owns the session, the UI tree and the routing between them. A
//! window (winit, `crate::window`) feeds it events and shows what it draws;
//! a test or the `shot` example does the same with no screen. Everything a
//! person can do in the editor goes through here, and everything that
//! changes the document goes through the session — the same calls an agent
//! makes over MCP (DNA, postulate 5).
//!
//! The layout is Unity's: the menu bar and toolbar on top, the Hierarchy on
//! the left, the Inspector on the right, the Scene view in the middle over
//! Project and Console, a status line at the bottom. The borders between
//! them drag. The look is Nocturne (`crate::theme`).
//!
//! The Scene view is a node whose picture is the session's frame texture,
//! on the same GPU device: no copy, on every platform (docs/ui.md).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use scrap::edit::Face;
use scrap::gizmo::Tool;
use scrap::glam::Vec3;
use scrap::input::{Input, InputEvent, Key, MouseButton};
use scrap::EntityId;
use scrap_editor::console::Level;
use scrap_editor::{Pivot, Session, Space};
use scrap_ui::render::UiRenderer;
use scrap_ui::{Event, ImageId, NodeId, Style, Ui};

use scrap_ui::Clipboard as _;

use crate::animator::Animator;
use crate::bottom::{Asset, Bottom};
use crate::clipboard::SystemClipboard;
use crate::dock::{Arrangement, Docked, Docks, Panel};
use crate::layouts::{self, Layout};
use crate::hierarchy::Hierarchy;
use crate::inspector::Inspector;
use crate::menu::{self, Action, MenuItem};
use crate::native_menu::Shortcut;
use crate::screens::{self, Screens};
use crate::theme::*;
use crate::tools::{Animation, FrameCost, Profiler, Settings};

/// The engine's reference scene: every builtin, no import step.
pub const REFERENCE_SCENE: &str = "examples/valley/scenes/first-light.ron";

/// The Scene view's picture, as the renderer knows it.
const SCENE: ImageId = ImageId(0);
/// A selected camera's view, in the Scene view's corner.
const CAMERA_PREVIEW: ImageId = ImageId(2);

/// The pointer's look, as [`Studio::cursor`] asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Default,
    Text,
    ResizeColumn,
    ResizeRow,
    /// Over the view with the terrain brush on.
    Brush,
}

/// What a panel asks the studio to do after an event.
#[derive(Default)]
pub struct Requests {
    /// The document changed: bring the panels up to date now.
    pub refresh: bool,
    /// Open a menu at a point.
    pub menu: Option<(Vec<MenuItem>, f32, f32)>,
    /// Give the keyboard back to the Scene view's shortcuts.
    pub keyboard_to_scene: bool,
    /// Put the keyboard in the Inspector's box for this field.
    pub focus_named: Option<String>,
    pub action: Option<Action>,
    /// A Project entry was dragged and let go.
    pub dropped: Option<Asset>,
    /// A Project entry was clicked: show it in the Inspector.
    pub inspect: Option<Asset>,
    /// Show this entry in the Project: an Inspector field pointed at it.
    pub ping: Option<Asset>,
}

/// A panel torn off into a window of its own.
///
/// One UI tree serves every window: the panel moves into a frame far to
/// the right of the main window, and the panel's window draws the tree
/// from that frame's corner and feeds its pointer in shifted by as much.
/// Focus, drags, menus and the panel's state are the one tree's, so
/// nothing is copied between windows.
struct Float {
    panel: Panel,
    frame: NodeId,
    dock_button: NodeId,
    origin: (f32, f32),
}

/// How far to the right of the main window floating frames begin, and how
/// far apart they are.
const FLOAT_X: f32 = 100_000.0;
const FLOAT_STEP: f32 = 10_000.0;

/// A face being dragged in face mode.
struct FaceDrag {
    id: EntityId,
    face: Face,
    /// Where the press was, in the view's pixels.
    from: (f32, f32),
    /// Where one metre out along the face goes on screen, in pixels.
    per_metre: (f32, f32),
    /// Metres pushed so far, in the one undo step the drag makes.
    pushed: f32,
}

/// What a question asked in a dialog is for.
#[derive(Debug, Clone, PartialEq)]
enum Ask {
    RenameAsset(String),
    DuplicateAsset(String),
    SaveMaterial,
    NewComponent,
    NewSystem,
    Variant,
    NewScene,
    Snap,
    SaveLayout,
    DeleteLayout,
}

/// A dialog asking for a name: Nocturne's `.dialog` over its backdrop.
struct Prompt {
    overlay: NodeId,
    field: NodeId,
    ok: NodeId,
    cancel: NodeId,
    ask: Ask,
}

/// What a Quick Search line leads to.
#[derive(Debug, Clone)]
enum Hit {
    Entity(scrap::EntityId),
    Asset(Asset),
    Action(Action),
}

/// Quick Search: one field over the scene, the project and the menus.
struct Search {
    overlay: NodeId,
    field: NodeId,
    list: NodeId,
    hits: Vec<(NodeId, Hit)>,
}

/// An open menu: its overlay, and what each line does.
struct Popup {
    overlay: NodeId,
    items: Vec<(NodeId, Action)>,
}

/// The toolbar's controls that change with the session.
struct Toolbar {
    menus: Vec<(NodeId, Vec<MenuItem>)>,
    scene_name: NodeId,
    modified: NodeId,
    play: NodeId,
    pause: NodeId,
    step: NodeId,
    play_group: NodeId,
    undo: NodeId,
    redo: NodeId,
    save: NodeId,
    /// The Layout dropdown: presets, Save, Delete.
    layout: NodeId,
}

struct Status {
    /// The Console's newest line: its box (a click shows the Console),
    /// its level's glyph, its text.
    console: NodeId,
    console_icon: NodeId,
    console_text: NodeId,
    entities: NodeId,
    selected: NodeId,
    problems: NodeId,
    mode: NodeId,
}

/// The Console line the status bar shows: which saying it was, its level
/// and text, when it came, the width it was cut to, and whether an info
/// has faded yet.
struct StatusLine {
    said: u64,
    level: Level,
    text: String,
    at: Instant,
    width: f32,
    faded: bool,
}

/// How long an info line stays bright in the status bar.
const STATUS_FADE: Duration = Duration::from_secs(10);

/// The Scene view's tools, in the strip in its corner, in Unity's order:
/// the hand, then the gizmo's five.
const TOOLS: [(&str, &str, Option<Tool>); 6] = [
    ("Hand", "hand", None),
    ("Move", "move-3d", Some(Tool::Move)),
    ("Rotate", "rotate-3d", Some(Tool::Rotate)),
    ("Scale", "scale-3d", Some(Tool::Scale)),
    ("Rect", "scan", Some(Tool::Rect)),
    ("Transform", "locate-fixed", Some(Tool::Transform)),
];

/// What a line of a menu shows now: its label (Undo says what it would
/// take back), whether it can be chosen, whether its tick is on.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuState {
    pub label: String,
    pub enabled: bool,
    pub checked: bool,
}

/// Everything the menus' lines show, in brief: the menu bar is brought up
/// to date when it changes.
#[derive(PartialEq)]
struct MenuStamp {
    undo: Option<Option<String>>,
    redo: Option<Option<String>>,
    selected: bool,
    playing: bool,
    paused: bool,
    grid: bool,
    snap: bool,
    colliders: bool,
    navigation: bool,
    game_view: bool,
    panels: [bool; 3],
    maximized: bool,
    /// The keymap's generation: a key rebound shows in the menus.
    keys: u64,
}

/// What the panels show, in brief: when it is the same as last frame's
/// they are left alone.
#[derive(PartialEq)]
struct Stamp {
    selection: Vec<scrap::EntityId>,
    revision: u64,
    console: (usize, usize, usize),
    lines: usize,
    tool: Tool,
    hand: bool,
    playing: bool,
    /// The game started with Play: the button turns back when it ends.
    game: bool,
    paused: bool,
    hidden: usize,
    isolated: usize,
    grid: bool,
    space: Space,
    pivot: Pivot,
    path: Option<std::path::PathBuf>,
    game_view: bool,
}

impl Stamp {
    fn of(session: &Session) -> Self {
        Self {
            selection: session.selection(),
            revision: session.revision(),
            console: session.console_counts(),
            lines: session.console().iter().map(|l| l.count as usize).sum(),
            tool: session.tool(),
            hand: session.hand(),
            playing: session.is_playing(),
            game: session.is_game_running(),
            paused: session.is_paused(),
            hidden: session.hidden().len(),
            isolated: session.isolated().len(),
            grid: session.show_grid(),
            space: session.space(),
            pivot: session.pivot(),
            path: session.scene_path().map(Path::to_path_buf),
            game_view: session.is_game_view(),
        }
    }
}

pub struct Studio {
    pub session: Session,
    pub ui: Ui,
    scene_input: Input,
    viewport: NodeId,
    view_frame: NodeId,
    tab_scene: NodeId,
    tab_game: NodeId,
    /// Buttons that are one action each: the view's corner, the snap.
    buttons: Vec<(NodeId, Action)>,
    /// The tool strip in the Scene view's corner, and its buttons in
    /// [`TOOLS`]' order.
    tool_strip: NodeId,
    tools: [NodeId; 6],
    /// The Scene view's bar: where the handles sit (Pivot, Center), which
    /// way they point (Global, Local), and the grid.
    pivot_button: NodeId,
    space_button: NodeId,
    grid_button: NodeId,
    /// The menus are the system's (macOS's menu bar), not the toolbar's.
    native_menu: bool,
    /// What the menus showed when last looked at, and a number that
    /// changes whenever that does.
    menu_seen: Option<MenuStamp>,
    menu_revision: u64,
    /// The Console line in the status bar.
    status_line: Option<StatusLine>,

    snap: NodeId,
    colliders_button: NodeId,
    sculpt_button: NodeId,
    docks: Docks,
    settings: Settings,
    configs: crate::configs::Configs,
    profiler: Profiler,
    animation: Animation,
    screens: Screens,
    animator: Animator,
    dialogues: crate::dialogues::Dialogues,
    /// The network inspector, the world diff, the saves, the systems.
    /// What the last draw cost, for the Profiler.
    last_draw_ms: f32,
    /// The sound device, opened the first time a sound is listened to, and
    /// what is playing on it.
    audio: Option<scrap::audio::Audio>,
    listening: Option<scrap::audio::Playing>,
    /// The Game view's width to height, or free.
    aspect: Option<(u32, u32)>,
    aspect_button: NodeId,
    /// The camera preview's box, its caption, its picture, and the size the
    /// renderer was last shown it at.
    cam_holder: NodeId,
    cam_label: NodeId,
    cam_image: NodeId,
    cam_registered: Option<(u32, u32)>,
    /// The orientation gizmo in the view's corner: its six axes and its
    /// middle, and the camera it was last turned for.
    compass: Compass,
    /// When the last event came in: the window draws at full rate for a
    /// while after one.
    last_input: Instant,
    /// Which of the Hierarchy, the Inspector and the lower panel show, and
    /// what has the whole window, if anything does.
    panels: [bool; 3],
    maximized: Option<Zoom>,
    /// The terrain brush is on: a left drag in the view shapes the ground.
    sculpt: bool,
    /// A long background import has been announced.
    import_said: bool,
    /// The foliage brush (docs/artist.md): on, what it paints, how big it
    /// is, and the stroke under way — when the last dab was and how many
    /// undo steps the stroke has made, to be one.
    foliage: bool,
    foliage_button: NodeId,
    foliage_what: Option<String>,
    foliage_radius: f32,
    foliage_stroke: Option<(Instant, usize)>,
    /// A new seed per dab, so two dabs on one spot do not place the same.
    foliage_seed: u64,
    /// The selection's spline points, as handles over the view, and the
    /// one being dragged: which entity and point, and how many undo steps
    /// the drag has made, to be one.
    /// An asset to show in the Inspector once the action that made it is
    /// done: a material instance just created.
    show_next: Option<Asset>,
    spline_handles: Vec<NodeId>,
    spline_drag: Option<(EntityId, usize, usize)>,
    /// Face mode: ProBuilder's face selection over the faces of boxes.
    faces: bool,
    faces_button: NodeId,
    /// The face under the pointer, and the one being dragged: which, where
    /// the drag began, the screen's pixels per metre out along the face,
    /// and how far it has been pushed so far.
    face_hover: Option<(EntityId, Face)>,
    face_drag: Option<FaceDrag>,
    face_box: NodeId,
    /// Panels in windows of their own.
    floats: Vec<Float>,
    /// Every picture given by pixels, kept for a window opened later, and
    /// a number that changes whenever any picture does.
    pictures: HashMap<ImageId, (u32, Vec<u8>)>,
    picture_generation: u64,
    /// A stroke in progress: when the last dab landed, and for a flatten
    /// the height it flattens to.
    stroke: Option<(Instant, f32)>,
    /// The tooltip on show, and what the pointer has rested on since when.
    tooltip: Option<NodeId>,
    resting: Option<(NodeId, Instant)>,
    /// When the disk was last looked at for a changed scene or asset.
    polled: Instant,
    toolbar: Toolbar,
    hierarchy: Hierarchy,
    /// What git has not got yet: the Hierarchy's and the Project's dots.
    git: crate::git_marks::GitMarks,
    inspector: Inspector,
    bottom: Bottom,
    status: Status,
    splits: [NodeId; 3],
    /// The column of the view and the lower dock.
    center: NodeId,
    view_slot: NodeId,
    /// The lower dock's height before the UI Builder went wide.
    lower_before_wide: Option<f32>,
    /// The colours: the person's choice and the project's file.
    theme: crate::appearance::Theme,
    /// Which key does what: the one table keys are answered from.
    keymap: crate::keymap::Keymap,
    /// The person's preferences that are not colours, keys or layouts,
    /// and whether they changed since written (a window moved).
    personal: crate::preferences::Personal,
    personal_dirty: bool,
    /// A floating panel's window to bring to the front.
    raise: Option<String>,
    /// SCRAP_BLENDER is the Blender chosen in Preferences.
    blender_set: bool,
    /// The Preferences window's content (Appearance is its first page).
    preferences: crate::preferences::Preferences,
    left: NodeId,
    right: NodeId,
    lower: NodeId,
    popup: Option<Popup>,
    prompt: Option<Prompt>,
    search: Option<Search>,
    /// Where a walker can go is shown.
    navigation: bool,
    seen: Option<Stamp>,
    /// Buttons that went down in the Scene view: their release goes there
    /// too, wherever the pointer is by then.
    scene_buttons: HashSet<MouseButton>,
    /// Buttons pressed over the game in the view: their release is the
    /// game's too, wherever the pointer is by then.
    game_buttons: HashSet<MouseButton>,
    /// The session frame's size when the renderer was last shown it.
    registered: Option<(u32, u32)>,
    drawn: Instant,
    frame_times: Vec<f32>,
    fps: NodeId,
    /// A scene someone asked to open over unsaved work, once.
    discard_asked: Option<std::path::PathBuf>,
    colliders: bool,
    conflict_said: bool,
    /// Pictures made on the CPU waiting for the renderer: previews.
    pending_images: Vec<(ImageId, u32, Vec<u8>)>,
    /// The layout as last written to disk.
    saved_layout: String,
    /// The layout preset last chosen or saved, for the Layout button.
    layout_name: Option<String>,
    /// The per-user config folder, where saved layouts are kept.
    config_dir: Option<std::path::PathBuf>,
    /// A build running in the background: what it says when it is done.
    job: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    /// The scene to go back to from prefab mode.
    scene_before_prefab: Option<std::path::PathBuf>,
    /// The banner over the view in prefab mode, and its Back button.
    prefab_bar: NodeId,
    prefab_name: NodeId,
    prefab_back: NodeId,
    clipboard: SystemClipboard,
}

impl Studio {
    /// The editor over a session with a document open, laid out for a
    /// window `width` × `height` logical pixels at `scale`.
    pub fn new(mut session: Session, width: f32, height: f32, scale: f32) -> Self {
        let mut ui = Ui::new();
        ui.set_viewport(width, height, scale);
        let root = ui.root();
        ui.set_style(root, Style::column().full().background(BG));

        let (toolbar, _bar) = build_toolbar(&mut ui, root);

        let main = ui.add(root, Style::row().fill().full_width().padding_x(5.0));
        let left = ui.add(
            main,
            Style::column()
                .width(270.0)
                .full_height()
                .fixed()
                .padding(3.0),
        );
        let hierarchy = Hierarchy::new(&mut ui, left);
        let split_left = splitter(&mut ui, main, true, "split left");
        let center = ui.add(main, Style::column().fill().full_height());
        let view_slot = ui.add(
            center,
            Style::column().fill().full_width().padding(3.0).gap(3.0),
        );
        // Scene | Game, as Unity's tabs over the view.
        let view_tabs = ui.add(
            view_slot,
            Style::row()
                .height(26.0)
                .fixed()
                .gap(SPACE_1)
                .center_items(),
        );
        let tab_scene = view_tab(&mut ui, view_tabs, "view scene", "grid-3x3", "Scene", true);
        let tab_game = view_tab(&mut ui, view_tabs, "view game", "camera", "Game", false);
        let aspect_button = ui.add(
            view_tabs,
            Style::row()
                .height(24.0)
                .padding_x(SPACE_2)
                .gap(4.0)
                .center_items()
                .radius(6.0)
                .hover(HOVER),
        );
        ui.set_name(aspect_button, "aspect");
        ui.add_text(
            aspect_button,
            Style::default().text_size(11.5).text_color(LABEL).nowrap(),
            "Free Aspect",
        );
        icon(&mut ui, aspect_button, "chevron-down", MUTED);
        spacer(&mut ui, view_tabs);
        // What the handles do, in words, as Unity's bar says them: where
        // they sit, which way they point; and the grid.
        let pivot_button = word_toggle(&mut ui, view_tabs, "handles at", "Pivot");
        let space_button = word_toggle(&mut ui, view_tabs, "handles along", "Global");
        let grid_button = word_toggle(&mut ui, view_tabs, "grid", "Grid");
        separator(&mut ui, view_tabs);
        let mut buttons = Vec::new();
        let snap = icon_button(&mut ui, view_tabs, "snap", "magnet", false);
        buttons.push((snap, Action::ToggleSnap));
        let colliders = icon_button(&mut ui, view_tabs, "colliders", "box", false);
        buttons.push((colliders, Action::ToggleColliders));
        let sculpt = icon_button(&mut ui, view_tabs, "sculpt", "mountain", false);
        buttons.push((sculpt, Action::ToggleSculpt));
        let faces = icon_button(&mut ui, view_tabs, "faces", "square", false);
        buttons.push((faces, Action::ToggleFaces));
        let foliage = icon_button(&mut ui, view_tabs, "foliage", "sparkles", false);
        buttons.push((foliage, Action::ToggleFoliage));
        // Prefab mode's banner: what is open, and the way back.
        let prefab_bar = ui.add(
            view_slot,
            Style::row()
                .height(28.0)
                .fixed()
                .full_width()
                .padding_x(SPACE_3)
                .gap(SPACE_2)
                .center_items()
                .radius(RADIUS_MD)
                .background(ACCENT_900)
                .border(1.0, ACCENT.alpha(40))
                .hidden(),
        );
        icon(&mut ui, prefab_bar, "package", ACCENT);
        let prefab_name = ui.add_text(prefab_bar, text().text_color(ACCENT_200), "");
        spacer(&mut ui, prefab_bar);
        let prefab_back = button(&mut ui, prefab_bar, "prefab back", "‹ Back to scene", true);
        let view_frame = ui.add(
            view_slot,
            Style::column()
                .fill()
                .full_width()
                .radius(RADIUS_MD)
                .border(1.0, scrap_ui::Color::TRANSPARENT)
                .padding(0.0),
        );
        let viewport = ui.add_image(
            view_frame,
            Style::default()
                .fill()
                .full_width()
                .radius(RADIUS_MD)
                .draggable()
                .focusable(),
            SCENE,
        );
        ui.set_name(viewport, "scene view");
        // Face mode's outline: over the view, but not in the way of it.
        let face_box = ui.add(
            view_frame,
            Style::row()
                .absolute(0.0, 0.0)
                .size(0.0, 0.0)
                .radius(2.0)
                .border(2.0, ACCENT)
                .background(ACCENT.alpha(28))
                .padding(3.0)
                .hidden(),
        );
        ui.set_name(face_box, "face outline");
        ui.add_text(
            face_box,
            Style::default()
                .text_size(11.0)
                .text_color(ACCENT_100)
                .nowrap(),
            "",
        );
        let compass = build_compass(&mut ui, view_frame);
        let (tool_strip, tools) = build_tool_strip(&mut ui, view_frame);
        // Unity's Camera Preview: what a selected camera sees, in the corner.
        let cam_holder = ui.add(view_frame, Style::row().absolute(0.0, 0.0).full().hidden());
        ui.add(cam_holder, Style::row().fill());
        let cam_col = ui.add(cam_holder, Style::column().fill().full_height());
        ui.add(cam_col, Style::row().fill());
        let cam_card = ui.add(
            cam_col,
            Style::column()
                .width(248.0)
                .padding(4.0)
                .gap(2.0)
                .margin(10.0)
                .radius(RADIUS_MD)
                .background(SURFACE)
                .border(1.0, NEUTRAL_800),
        );
        ui.set_layer(cam_card, true);
        ui.set_name(cam_card, "camera preview");
        let cam_label = ui.add_text(
            cam_card,
            Style::default().text_size(11.0).text_color(LABEL).nowrap(),
            "Camera Preview",
        );
        let cam_image = ui.add_image(
            cam_card,
            Style::default().size(240.0, 135.0).radius(RADIUS_SM),
            CAMERA_PREVIEW,
        );
        ui.set_name(cam_image, "camera preview image");
        let split_lower = splitter(&mut ui, center, false, "split lower");
        let lower = ui.add(
            center,
            Style::column()
                .height(210.0)
                .full_width()
                .fixed()
                .padding(3.0),
        );
        let bottom = Bottom::new(&mut ui, lower);
        let split_right = splitter(&mut ui, main, true, "split right");
        let right = ui.add(
            main,
            Style::column()
                .width(340.0)
                .full_height()
                .fixed()
                .padding(3.0),
        );
        let inspector = Inspector::new(&mut ui, right);
        let (status, fps) = build_status(&mut ui, root);
        // The panels are content; the docks at the three edges hold them,
        // as tabs, where the layout says.
        let mut roots = std::collections::HashMap::new();
        roots.insert(Panel::Hierarchy, hierarchy.card);
        roots.insert(Panel::Inspector, inspector.root);
        for (panel, root) in [Panel::Project, Panel::Console, Panel::History, Panel::Git]
            .into_iter()
            .zip(bottom.roots)
        {
            roots.insert(panel, root);
        }
        let settings = Settings::new(&mut ui, lower);
        let config = crate::appearance::config_dir();
        let (keymap, key_errors) = crate::keymap::Keymap::load(config.as_deref());
        let (personal, personal_error) = crate::preferences::Personal::load(config.as_deref());
        let preferences = crate::preferences::Preferences::new(&mut ui, lower, &keymap);
        roots.insert(Panel::Preferences, preferences.root);
        let profiler = Profiler::new(&mut ui, lower);
        roots.insert(Panel::Settings, settings.root);
        roots.insert(Panel::Profiler, profiler.root);
        let configs = crate::configs::Configs::new(&mut ui, lower);
        roots.insert(Panel::Configs, configs.root);
        let animation = Animation::new(&mut ui, lower);
        roots.insert(Panel::Animation, animation.root);
        let screens = Screens::new(&mut ui, lower);
        roots.insert(Panel::Screens, screens.root);
        let animator = Animator::new(&mut ui, lower);
        roots.insert(Panel::Animator, animator.root);
        let dialogues = crate::dialogues::Dialogues::new(&mut ui, lower);
        roots.insert(Panel::Dialogues, dialogues.root);
        let docks = Docks::new(
            &mut ui,
            [left, right, lower],
            roots,
            Arrangement::default_layout(),
        );

        session.set_readback(false);
        let mut studio = Self {
            session,
            ui,
            scene_input: Input::new(),
            viewport,
            view_frame,
            tab_scene,
            tab_game,
            buttons,
            tool_strip,
            tools,
            pivot_button,
            space_button,
            grid_button,
            native_menu: false,
            menu_seen: None,
            menu_revision: 0,
            status_line: None,
            snap,

            colliders_button: colliders,
            sculpt_button: sculpt,
            sculpt: false,
            import_said: false,
            foliage: false,
            foliage_button: foliage,
            foliage_what: None,
            foliage_radius: 4.0,
            foliage_stroke: None,
            foliage_seed: 0,
            show_next: None,
            spline_handles: Vec::new(),
            spline_drag: None,
            faces: false,
            faces_button: faces,
            face_hover: None,
            face_drag: None,
            face_box,
            floats: Vec::new(),
            pictures: HashMap::new(),
            picture_generation: 0,
            panels: [true; 3],
            last_input: Instant::now(),
            docks,
            settings,
            configs,
            profiler,
            animation,
            screens,
            animator,
            dialogues,
            last_draw_ms: 0.0,
            aspect: None,
            audio: None,
            listening: None,
            aspect_button,
            cam_holder,
            cam_label,
            cam_image,
            cam_registered: None,
            compass,
            maximized: None,
            stroke: None,
            tooltip: None,
            resting: None,
            polled: Instant::now(),
            toolbar,
            hierarchy,
            git: crate::git_marks::GitMarks::new(),
            inspector,
            bottom,
            status,
            splits: [split_left, split_lower, split_right],
            center,
            view_slot,
            lower_before_wide: None,
            theme: crate::appearance::Theme::new(config.clone()),
            keymap,
            personal,
            personal_dirty: false,
            raise: None,
            blender_set: false,
            preferences,
            left,
            right,
            lower,
            popup: None,
            prompt: None,
            search: None,
            navigation: false,
            seen: None,
            scene_buttons: HashSet::new(),
            game_buttons: HashSet::new(),
            registered: None,
            drawn: Instant::now(),
            frame_times: Vec::new(),
            fps,
            discard_asked: None,
            colliders: false,
            conflict_said: false,
            job: None,
            saved_layout: String::new(),
            layout_name: Some("Default".into()),
            config_dir: config,
            pending_images: Vec::new(),
            scene_before_prefab: None,
            prefab_bar,
            prefab_name,
            prefab_back,
            clipboard: SystemClipboard::new(),
        };
        studio.ui.set_clipboard(Box::new(SystemClipboard::new()));
        studio.ui.focus(Some(viewport));
        studio.restore_layout();
        studio.update_layout_button();
        studio.sync_visible();
        for e in key_errors.into_iter().chain(personal_error) {
            studio.session.say(Level::Error, e);
        }
        studio.apply_personal();
        studio.show_preferences();
        studio.poll_theme();
        studio.show_theme();
        studio.refresh();
        studio
    }

    /// Whether the next frame should come soon: something is moving, the
    /// person is doing something, or the UI has changes to show. When not,
    /// the window can wait — an editor left open on a scene draws a few
    /// frames a second instead of a hundred.
    pub fn wants_frame(&self) -> bool {
        self.last_input.elapsed().as_secs_f32() < 1.0
            || self.ui.is_dirty()
            || self.session.is_playing()
            || self.session.is_game_running()
            || self.session.is_dragging()
            || self.stroke.is_some()
            || self.job.is_some()
            || !self.scene_buttons.is_empty()
            || !self.session.previewing().is_empty()
            || !self.bottom.wanted_pictures(1).is_empty()
    }

    /// How many Project pictures are still to draw.
    pub fn bottom_pictures_pending(&self) -> usize {
        self.bottom.wanted_pictures(usize::MAX).len()
    }

    /// Whether a text field has the keyboard: the window turns the input
    /// method on only then, so W, E and R stay the tools elsewhere.
    pub fn typing(&self) -> bool {
        self.ui.focused().is_some_and(|f| self.ui.is_field(f))
    }

    /// What an input method is composing, shown in the focused field.
    pub fn ime_preedit(&mut self, text: &str) {
        self.last_input = Instant::now();
        self.ui.ime_preedit(text);
    }

    /// Where the input method's candidates go: by the caret.
    pub fn ime_area(&self) -> Option<scrap_ui::Rect> {
        self.ui.caret_rect()
    }

    /// What the pointer should look like where it is: an I-beam over a
    /// field, a resize arrow over a border between panels.
    /// Whether the game in the view is being played: the Game view is up
    /// and the game Play started draws in it.
    fn game_in_view(&self) -> bool {
        self.session.is_game_view() && self.session.is_game_in_view() && self.popup.is_none()
    }

    /// Whether the pointer is the game's: captured — hidden, held, only its
    /// motion counting — as the game asked.
    pub fn captures_cursor(&self) -> bool {
        self.game_in_view() && self.session.game_captures_cursor()
    }

    /// Hand what falls on the game in the view to it, as its own window
    /// would: the pointer over it, the buttons pressed on it, the keys
    /// while the view has the keyboard — except the editor's shortcuts, so
    /// Ctrl/Cmd P still stops. `true` when the event was the game's alone.
    fn to_game(
        &mut self,
        event: &InputEvent,
        over_view: bool,
        typing: bool,
        to_view: impl Fn(f32, f32) -> (f32, f32),
    ) -> bool {
        if !self.game_in_view() {
            self.game_buttons.clear();
            return false;
        }
        let focused = self.ui.focused() == Some(self.viewport);
        let (_, ctrl, _, command) = self.ui.modifiers();
        let shortcut = if cfg!(target_os = "macos") { command } else { ctrl };
        let modifier = |key: &Key| {
            matches!(
                key,
                Key::LeftShift
                    | Key::RightShift
                    | Key::LeftControl
                    | Key::RightControl
                    | Key::LeftAlt
                    | Key::RightAlt
                    | Key::LeftSuper
                    | Key::RightSuper
            )
        };
        match event {
            InputEvent::MouseMoved { x, y } => {
                let (x, y) = to_view(*x, *y);
                self.session.send_to_game(InputEvent::MouseMoved { x, y });
                false
            }
            InputEvent::MouseMotion { .. } => {
                self.session.send_to_game(event.clone());
                true
            }
            InputEvent::MouseDown(button) if over_view => {
                self.game_buttons.insert(*button);
                self.ui.focus(Some(self.viewport));
                self.session.send_to_game(event.clone());
                true
            }
            InputEvent::MouseUp(button) if self.game_buttons.remove(button) => {
                self.session.send_to_game(event.clone());
                true
            }
            InputEvent::Scroll { .. } if over_view => {
                self.session.send_to_game(event.clone());
                true
            }
            InputEvent::KeyDown(key) if focused && !typing && (modifier(key) || !shortcut) => {
                self.session.send_to_game(event.clone());
                !modifier(key)
            }
            // Every release: a key held into the game comes up in it.
            InputEvent::KeyUp(_) => {
                self.session.send_to_game(event.clone());
                false
            }
            InputEvent::Text(_) if focused && !typing && !shortcut => {
                self.session.send_to_game(event.clone());
                true
            }
            InputEvent::FocusLost => {
                self.game_buttons.clear();
                self.session.send_to_game(event.clone());
                false
            }
            _ => false,
        }
    }

    pub fn cursor(&self) -> Cursor {
        let node = self.ui.dragging().or(self.ui.hovered());
        match node {
            Some(n) if self.ui.is_field(n) => Cursor::Text,
            Some(n) if n == self.splits[1] => Cursor::ResizeRow,
            Some(n) if self.splits.contains(&n) => Cursor::ResizeColumn,
            Some(n) if n == self.viewport && self.sculpt => Cursor::Brush,
            _ => Cursor::Default,
        }
    }

    /// What the window's title says, as Unity's does: the scene, the
    /// project, the editor — `first-light — valley — scrap` — with a dot
    /// in front while there are unsaved edits (the order an editor with
    /// tabs uses; macOS also marks the close button, `window.rs`).
    pub fn title(&self) -> String {
        let s = &self.session;
        let name = s
            .scene_path()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());
        let dot = if s.is_modified() { "• " } else { "" };
        let prefab = if s.is_prefab() { " (prefab)" } else { "" };
        match s.project() {
            Some(project) => format!("{dot}{name}{prefab} — {} — scrap", project.name()),
            None => format!("{dot}{name}{prefab} — scrap"),
        }
    }

    /// Menus in the system's menu bar (`true`, macOS's window does this)
    /// or in the toolbar, drawn by the studio (the tests, other systems).
    pub fn set_native_menu(&mut self, on: bool) {
        self.native_menu = on;
        for (m, _) in &self.toolbar.menus {
            self.ui
                .restyle(*m, |s| if on { s.hidden() } else { s.shown() });
        }
    }

    pub fn native_menu(&self) -> bool {
        self.native_menu
    }

    /// A number that changes whenever something a menu line shows does —
    /// what the menu bar checks before bringing its lines up to date.
    pub fn menu_revision(&self) -> u64 {
        self.menu_revision
    }

    /// What the menu line for `action`, called `label`, shows now. The
    /// toolbar's menus and the system's both ask.
    pub fn menu_state(&self, action: &Action, label: &str) -> MenuState {
        let s = &self.session;
        let selected = !s.selection().is_empty();
        let playing = s.is_playing();
        let (enabled, checked) = match action {
            Action::Editor("undo") => (s.can_undo(), false),
            Action::Editor("redo") => (s.can_redo(), false),
            Action::Editor("duplicate_entity" | "delete_entity" | "drop_to_ground")
            | Action::Copy
            | Action::Rename
            | Action::Frame
            | Action::Group
            | Action::Hide
            | Action::SnapToGrid
            | Action::SaveMaterial => (selected, false),
            Action::Pause => (playing, s.is_paused()),
            Action::Step | Action::KeepSimulation => (playing, false),
            Action::Play => (true, playing),
            Action::ToggleGrid => (true, s.show_grid()),
            Action::ToggleSnap => (true, s.snap().meters > 0.0),
            Action::ToggleColliders => (true, self.colliders),
            Action::ToggleNavigation => (true, self.navigation),
            Action::GameView(game) => (true, s.is_game_view() == *game),
            Action::TogglePanel(i) => (true, self.panels.get(*i).copied().unwrap_or(false)),
            Action::Maximize => (true, self.maximized == Some(Zoom::View)),
            _ => (true, false),
        };
        // Undo and Redo say what they would do, as Unity's Edit menu.
        let label = match action {
            Action::Editor("undo") => s
                .undo_label()
                .map_or_else(|| label.to_string(), |l| format!("Undo {l}")),
            Action::Editor("redo") => s
                .redo_label()
                .map_or_else(|| label.to_string(), |l| format!("Redo {l}")),
            _ => label.to_string(),
        };
        MenuState {
            label,
            enabled,
            checked,
        }
    }

    fn menu_stamp(&self) -> MenuStamp {
        let s = &self.session;
        MenuStamp {
            undo: s.can_undo().then(|| s.undo_label()),
            redo: s.can_redo().then(|| s.redo_label()),
            selected: !s.selection().is_empty(),
            playing: s.is_playing(),
            paused: s.is_paused(),
            grid: s.show_grid(),
            snap: s.snap().meters > 0.0,
            colliders: self.colliders,
            navigation: self.navigation,
            game_view: s.is_game_view(),
            panels: self.panels,
            maximized: self.maximized.is_some(),
            keys: self.keymap.generation,
        }
    }

    /// The window changed size or moved to a screen with another scale.
    pub fn resize(&mut self, width: f32, height: f32, scale: f32) {
        self.ui.set_viewport(width, height, scale);
    }

    /// Where the Scene view is, in logical pixels.
    pub fn scene_rect(&mut self) -> scrap_ui::Rect {
        self.ui.paint();
        self.ui.rect(self.viewport)
    }

    // --- input -----------------------------------------------------------

    /// One event from the window, in logical pixels.
    pub fn handle(&mut self, event: &InputEvent) {
        self.last_input = Instant::now();
        self.ui.handle(event);
        let over_view = self.popup.is_none() && self.ui.hovered() == Some(self.viewport);
        let typing = self.ui.focused().is_some_and(|f| self.ui.is_field(f));
        let scale = self.ui.viewport().2;
        let view = self.ui.rect(self.viewport);
        let to_view = |x: f32, y: f32| ((x - view.x) * scale, (y - view.y) * scale);
        if let InputEvent::MouseDown(_) = event {
            // A click on the Project gives it the arrows; anywhere else,
            // takes them back.
            let on = self
                .ui
                .hovered()
                .is_some_and(|h| self.bottom.owns_project(&self.ui, h));
            self.bottom.set_active(on);
        }
        if self.to_game(event, over_view, typing, to_view) {
            return;
        }
        match event {
            InputEvent::MouseMoved { x, y } => {
                let (vx, vy) = to_view(*x, *y);
                self.scene_input
                    .handle(&InputEvent::MouseMoved { x: vx, y: vy });
                if self.stroke.is_some() {
                    self.dab(vx, vy, false);
                }
                if self.faces {
                    self.face_move(vx, vy, over_view);
                }
                if self.foliage_stroke.is_some() {
                    self.foliage_dab(vx, vy);
                }
            }
            // An Inspector field's eyedropper takes the next click in the
            // view: what is under it goes in the field.
            InputEvent::MouseDown(MouseButton::Left)
                if over_view && self.inspector.picking_in_scene() =>
            {
                let (px, py) = self.ui.pointer();
                let (vx, vy) = to_view(px, py);
                let hit = self.session.pick(vx as u32, vy as u32);
                self.inspector.pick_in_scene(&mut self.session, hit);
                self.refresh();
            }
            InputEvent::KeyDown(Key::Escape) if self.inspector.picking_in_scene() => {
                self.inspector.cancel_pick();
                self.refresh();
            }
            // The foliage brush takes the left button; Alt still orbits.
            InputEvent::MouseDown(MouseButton::Left)
                if over_view && self.foliage && !self.ui.modifiers().2 =>
            {
                let (px, py) = self.ui.pointer();
                let (vx, vy) = to_view(px, py);
                self.foliage_stroke = Some((Instant::now() - Duration::from_secs(1), 0));
                self.foliage_dab(vx, vy);
            }
            InputEvent::MouseUp(MouseButton::Left) if self.foliage_stroke.is_some() => {
                if let Some((_, steps)) = self.foliage_stroke.take() {
                    if steps > 1 {
                        self.session.squash_last(steps);
                    }
                }
                self.refresh();
            }
            // Face mode takes the left button on a face; Alt still orbits.
            InputEvent::MouseDown(MouseButton::Left)
                if over_view
                    && self.faces
                    && !self.ui.modifiers().2
                    && self.face_hover.is_some() =>
            {
                let (px, py) = self.ui.pointer();
                let (vx, vy) = to_view(px, py);
                self.face_press(vx, vy);
            }
            InputEvent::MouseUp(MouseButton::Left) if self.face_drag.is_some() => {
                self.face_drag = None;
                self.refresh();
            }
            // The brush takes the left button; Alt still orbits.
            InputEvent::MouseDown(MouseButton::Left)
                if over_view && self.sculpt && !self.ui.modifiers().2 =>
            {
                let (px, py) = self.ui.pointer();
                let (vx, vy) = to_view(px, py);
                self.dab(vx, vy, true);
            }
            InputEvent::MouseUp(MouseButton::Left) if self.stroke.is_some() => {
                self.stroke = None;
                self.refresh();
            }
            InputEvent::MouseDown(_)
                if !self
                    .hierarchy
                    .owns(&self.ui, self.ui.hovered().unwrap_or(self.viewport))
                    && self.ui.hovered().is_some() =>
            {
                // A click anywhere but the Hierarchy gives the arrows back
                // to the Scene view.
                self.hierarchy.set_active(false);
                if over_view && !self.session.is_game_view() {
                    if let InputEvent::MouseDown(button) = event {
                        self.scene_buttons.insert(*button);
                    }
                    self.scene_input.handle(event);
                }
            }
            InputEvent::MouseDown(button) if over_view && !self.session.is_game_view() => {
                self.scene_buttons.insert(*button);
                self.scene_input.handle(event);
            }
            InputEvent::MouseUp(button) => {
                if self.scene_buttons.remove(button) {
                    self.scene_input.handle(event);
                }
            }
            InputEvent::Scroll { .. } if over_view => self.scene_input.handle(event),
            // Preferences › Keys is listening: the key pressed is the new
            // one (with what is held), whatever it did before.
            InputEvent::KeyDown(key)
                if self.preferences.capturing().is_some() && !Shortcut::is_modifier(*key) =>
            {
                self.capture_key(*key);
            }
            InputEvent::KeyDown(key) if !typing => {
                let mut requests = Requests::default();
                if self
                    .bottom
                    .key(&mut self.ui, &self.session, *key, &mut requests)
                {
                    self.apply(requests);
                    return;
                }
                if matches!(key, Key::Up | Key::Down | Key::Left | Key::Right) {
                    let shift = self.ui.modifiers().0;
                    if self.hierarchy.key(&mut self.session, *key, shift) {
                        self.refresh();
                        return;
                    }
                }
                if *key == Key::Escape && self.popup.is_some() {
                    self.close_popup();
                    return;
                }
                let (_, ctrl, _, command) = self.ui.modifiers();
                // Flying (the right button held in the view), a plain key
                // steers — W A S D, Q E — and does not pick a tool.
                let steering = self.scene_buttons.contains(&MouseButton::Right) && !ctrl && !command;
                if !steering && !Shortcut::is_modifier(*key) {
                    let chord = Shortcut::held(*key, self.ui.modifiers());
                    if let Some(action) = self.keymap.action(chord).cloned() {
                        if self.answer_key(action) {
                            return;
                        }
                    }
                }
                // The view hears only the keys it holds — the modifiers
                // (Ctrl steps a drag), V (vertex snapping), the flying keys
                // — and never answers one as a shortcut of its own: what a
                // key does is the keymap's, so a rebound W is not the Move
                // tool still.
                let plain_v = *key == Key::V && !ctrl && !command;
                if Shortcut::is_modifier(*key) || plain_v || steering {
                    self.scene_input.handle(event);
                }
            }
            // The foliage brush's size: [ and ], as typed, since the
            // engine's keys have no brackets.
            InputEvent::Text(text) if self.foliage && !typing && (text == "[" || text == "]") => {
                let grow = if text == "]" { 1.25 } else { 0.8 };
                self.foliage_radius = (self.foliage_radius * grow).clamp(0.5, 50.0);
                self.session.say(
                    Level::Info,
                    format!("foliage brush: {:.1} m across", self.foliage_radius * 2.0),
                );
            }
            // Always: a key let go while typing must not stay held.
            InputEvent::KeyUp(_) | InputEvent::FocusLost => self.scene_input.handle(event),
            _ => {}
        }
    }

    /// A file dropped on the window: import it into the project.
    pub fn drop_file(&mut self, path: &Path) {
        match self.session.import(path) {
            Ok(notes) => {
                self.session
                    .say(Level::Info, format!("imported {}", path.display()));
                for n in notes {
                    self.session.say(Level::Warning, n);
                }
            }
            Err(e) => self.session.say(Level::Error, e.to_string()),
        }
        self.refresh();
    }

    // --- the frame -------------------------------------------------------

    /// Everything that happens once a frame: what the panels heard, what
    /// the Scene view does with its input, the session's render, and the
    /// panels brought up to date if the document changed.
    pub fn frame(&mut self) {
        let dt = self.drawn.elapsed().as_secs_f32().min(0.1);
        self.drawn = Instant::now();
        self.frame_times.push(dt);

        let timing = std::env::var_os("SCRAP_STUDIO_TIMING").is_some();
        let t0 = Instant::now();
        let mut requests = Requests::default();
        for (node, event) in self.ui.events() {
            self.dispatch(node, &event, &mut requests);
        }
        self.apply(requests);

        // The Game view at a chosen shape: the view letterboxed in its frame.
        self.fit_aspect();
        // The session draws at the Scene view's size, in pixels.
        let view = self.scene_rect();
        let scale = self.ui.viewport().2;
        let size = (
            ((view.width * scale).round() as u32).max(1),
            ((view.height * scale).round() as u32).max(1),
        );
        if self.session.size() != size {
            self.session.resize(size.0, size.1);
        }
        // How thick outlines are and how big a turn's angle is written.
        self.session.set_ui_scale(scale);
        let t1 = Instant::now();
        // Play runs on the frame's time: the simulation takes as many fixed
        // steps as the frame took (paused, none).
        if self.session.is_playing() {
            self.session.step(dt);
        }
        let _ = self.session.scene_view(&self.scene_input, dt);
        self.scene_input.begin_frame();
        let t2 = Instant::now();
        self.session.render();
        self.camera_preview();
        self.place_spline_handles();
        let t3 = Instant::now();

        self.poll_disk();
        // An open Blender: saves and objects being moved, every frame, so
        // a drag there moves here while it happens.
        self.session.poll_blender();
        self.bottom.update_git(&mut self.ui, &mut self.session);
        // The Project's pictures, in frames nobody is waiting on: none
        // while the person is doing something or the document or the
        // selection just changed, and a few milliseconds' worth at most —
        // each is drawn and read back whole, and a click must not wait
        // for a picture of a scene.
        let calm = self.last_input.elapsed() > Duration::from_millis(250)
            && self.seen.as_ref().is_some_and(|seen| *seen == Stamp::of(&self.session));
        if calm {
            let start = Instant::now();
            for (name, image) in self.bottom.wanted_pictures(4) {
                if start.elapsed() > Duration::from_millis(6) {
                    break;
                }
                let pixels = match name.strip_prefix(crate::bottom::SCENE_PICTURE) {
                    Some(path) => self
                        .session
                        .scene_thumbnail(std::path::Path::new(path), 128, 128),
                    None => self.session.thumbnail(&name, 128),
                };
                if let Ok(pixels) = pixels {
                    self.pending_images.push((image, 128, pixels));
                }
                self.bottom.picture_ready(&mut self.ui, &name);
            }
        }
        if let Some(job) = &self.job {
            if let Ok(result) = job.try_recv() {
                self.job = None;
                match result {
                    Ok(message) => self.session.say(Level::Info, message),
                    Err(message) => self.session.say(Level::Error, message),
                }
            }
        }
        self.update_tooltip();
        self.hierarchy.hover(&mut self.ui);
        if self.git.poll(&self.session) {
            self.hierarchy.set_marks(&mut self.ui, &self.git.entities);
            let mut files = self.git.files.clone();
            // The open scene, edited and not saved, is not committed either.
            if !self.git.entities.is_empty() {
                if let Some(p) = self.session.scene_path().and_then(|p| p.canonicalize().ok()) {
                    files.insert(p);
                }
            }
            self.bottom.set_marks(&mut self.ui, files);
        }
        self.turn_compass();
        // The status bar's line: an info fading, the bar resized.
        if self.status_line.is_some() {
            self.update_status_line();
        }

        let t4 = Instant::now();
        let stamp = Stamp::of(&self.session);
        let t5 = Instant::now();
        let moving = self.session.is_dragging() || self.session.is_playing();
        if self.seen.as_ref() != Some(&stamp) || moving {
            if self
                .seen
                .as_ref()
                .is_none_or(|s| s.selection != stamp.selection)
            {
                self.inspector.clear_asset();
            }
            self.seen = Some(stamp);
            self.update_panels(moving);
        }
        // The colour picker's pictures, as it makes them.
        let images = self.inspector.take_images();
        self.pending_images.extend(images);
        // An asset an action made, shown once the panels have caught up
        // (catching up clears what the Inspector showed).
        if let Some(asset) = self.show_next.take() {
            if let Some(pixels) = self
                .inspector
                .show_asset(&mut self.ui, &mut self.session, asset)
            {
                self.pending_images
                    .push((crate::inspector::PREVIEW, 256, pixels));
            }
        }
        {
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f32() * 1e3;
            self.profiler.record(FrameCost {
                input: ms(t1, t2),
                render: ms(t2, t3),
                panels: ms(t5, Instant::now()) + ms(t0, t1),
                ui: self.last_draw_ms,
            });
            let live = self.wants_frame();
            if live && self.docks.is_showing(Panel::Profiler) {
                self.profiler.update(&mut self.ui);
            }
            if self.docks.is_showing(Panel::Animation) {
                self.animation.update(&mut self.ui, &self.session);
            }
            if self.docks.is_showing(Panel::Screens) {
                self.screens.update(&mut self.ui, &self.session);
                self.screens.draw(&self.session);
            }
            if self.docks.is_showing(Panel::Animator) {
                self.animator.update(&mut self.ui, &self.session);
            }
            if self.docks.is_showing(Panel::Dialogues) {
                self.dialogues.update(&mut self.ui, &self.session);
            }
            self.fit_wide();
            if self.docks.is_showing(Panel::Settings) {
                self.settings.update(&mut self.ui, &mut self.session);
            }
            if self.docks.is_showing(Panel::Configs) {
                self.configs.update(&mut self.ui, &self.session);
            }
        }
        if timing {
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1e3;
            eprintln!(
                "events+layout {:.1} scene_view {:.1} render {:.1} disk+tip {:.1} stamp {:.1} panels {:.1} ms",
                ms(t0, t1), ms(t1, t2), ms(t2, t3), ms(t3, t4), ms(t4, t5), ms(t5, Instant::now())
            );
        }
        if self.frame_times.len() >= 30 {
            let mean = self.frame_times.iter().sum::<f32>() / self.frame_times.len() as f32;
            let fps = 1.0 / mean.max(1e-4);
            self.ui.set_text(self.fps, &format!("{fps:.0} fps"));
            if std::env::var_os("SCRAP_STUDIO_FPS").is_some() {
                eprintln!("{fps:.0} fps, frame {:.1} ms", mean * 1e3);
            }
            self.frame_times.clear();
        }
    }

    /// The UI Builder over the whole window below the toolbar, or back in
    /// its dock.
    fn fit_wide(&mut self) {
        let under = |panel| self.docks.is_active(panel) && self.docks.region_of(panel) == Some(2);
        let wide = if self.screens.wide && under(Panel::Screens) {
            Some(Panel::Screens)
        } else if self.animator.wide && under(Panel::Animator) {
            Some(Panel::Animator)
        } else {
            None
        };
        // Its stack has the lower area to itself, split or not.
        if wide.is_some() && self.lower_before_wide.is_none() {
            self.docks.set_zoom(&mut self.ui, wide);
        }
        let wide = wide.is_some();
        let split = self.splits[1];
        match (wide, self.lower_before_wide) {
            (true, _) => {
                if self.lower_before_wide.is_none() {
                    self.lower_before_wide = Some(self.ui.rect(self.lower).height);
                    // The side docks go too: the canvas wants the width.
                    let [left, _, right] = self.splits;
                    for n in [self.view_slot, split, self.left, left, self.right, right] {
                        self.ui.restyle(n, |s| s.hidden());
                    }
                }
                let room = self.ui.parent(self.lower).map(|c| self.ui.rect(c).height);
                if let Some(room) =
                    room.filter(|r| (self.ui.rect(self.lower).height - r).abs() > 0.5)
                {
                    self.ui.restyle(self.lower, |s| s.height(room));
                }
            }
            (false, Some(height)) => {
                self.lower_before_wide = None;
                for n in [self.view_slot, split] {
                    self.ui.restyle(n, |s| s.shown());
                }
                self.ui.restyle(self.lower, |s| s.height(height));
                self.show_panels();
            }
            (false, None) => {}
        }
    }

    /// Show the panels that are on, or only what is maximized. A dock with
    /// no panels folds away and gives the view its room, except while a tab
    /// is dragged: then it shows, to be dropped on.
    fn show_panels(&mut self) {
        // The UI Builder gone wide lays the window out itself.
        if self.lower_before_wide.is_some() {
            return;
        }
        let [left, lower, right] = self.splits;
        let slots = [(self.left, left), (self.right, right), (self.lower, lower)];
        let dragging = self.docks.dragging();
        // A maximized stack whose panel has gone (floated, closed) gives
        // the window back.
        if let Some(Zoom::Stack(p)) = self.maximized {
            if self.docks.region_of(p).is_none() {
                self.maximized = None;
            }
        }
        let zoomed_region = match self.maximized {
            Some(Zoom::Stack(p)) => self.docks.region_of(p),
            _ => None,
        };
        let stack = match self.maximized {
            Some(Zoom::Stack(p)) => Some(p),
            _ => None,
        };
        self.docks.set_zoom(&mut self.ui, stack);
        for (i, (slot, split)) in slots.into_iter().enumerate() {
            let zoomed = zoomed_region == Some(i);
            let on = match self.maximized {
                None => self.panels[i] && (dragging || !self.docks.is_empty(i)),
                Some(_) => zoomed,
            };
            let split_on = on && self.maximized.is_none();
            self.ui.restyle(slot, |s| {
                let s = if on { s.shown() } else { s.hidden() };
                if zoomed {
                    s.fill()
                } else {
                    s.unfilled()
                }
            });
            self.ui
                .restyle(split, |s| if split_on { s.shown() } else { s.hidden() });
        }
        let side = matches!(zoomed_region, Some(0 | 1));
        let lower_zoomed = zoomed_region == Some(2);
        self.ui
            .restyle(self.center, |s| if side { s.hidden() } else { s.shown() });
        self.ui.restyle(self.view_slot, |s| {
            if lower_zoomed {
                s.hidden()
            } else {
                s.shown()
            }
        });
    }

    /// Give `zoom` the whole window, or, when it has it, give the window
    /// back.
    fn toggle_zoom(&mut self, zoom: Zoom) {
        self.maximized = if self.maximized == Some(zoom) {
            None
        } else {
            Some(zoom)
        };
        self.show_panels();
        self.refresh();
    }

    /// Draw the selected camera's view into the corner, or hide the box.
    fn camera_preview(&mut self) {
        let camera = self
            .session
            .selected()
            .filter(|_| !self.session.is_game_view())
            .filter(|id| self.session.camera_of(*id).is_some());
        let shown = match camera {
            Some(id) => self.session.render_camera_preview(id),
            None => false,
        };
        self.ui.restyle(
            self.cam_holder,
            |s| if shown { s.shown() } else { s.hidden() },
        );
        if let Some(id) = camera.filter(|_| shown) {
            let name = self.session.entity_name(id).unwrap_or_default();
            self.ui
                .set_text(self.cam_label, &format!("{name} — Camera Preview"));
            let (w, h) = self.session.size();
            let ratio = h as f32 / w.max(1) as f32;
            self.ui
                .restyle(self.cam_image, |s| s.size(240.0, (240.0 * ratio).round()));
        }
    }

    fn fit_aspect(&mut self) {
        let wanted = if self.session.is_game_view() {
            self.aspect
        } else {
            None
        };
        let label = match self.aspect {
            None => "Free Aspect".to_string(),
            Some((w, h)) => format!("{w}:{h}"),
        };
        if let Some(t) = self.ui.children(self.aspect_button).first().copied() {
            self.ui.set_text(t, &label);
        }
        self.ui.paint();
        let frame = self.ui.rect(self.view_frame);
        let style = match wanted {
            None => Style::default()
                .fill()
                .full_width()
                .radius(RADIUS_MD)
                .draggable()
                .focusable(),
            Some((w, h)) => {
                let ratio = w as f32 / h as f32;
                let (fw, fh) = (frame.width.max(1.0), frame.height.max(1.0));
                let (vw, vh) = if fw / fh > ratio {
                    (fh * ratio, fh)
                } else {
                    (fw, fw / ratio)
                };
                Style::default()
                    .size(vw.floor(), vh.floor())
                    .fixed()
                    .radius(RADIUS_MD)
                    .draggable()
                    .focusable()
            }
        };
        self.ui.set_style(self.viewport, style);
        let centred = wanted.is_some();
        self.ui.restyle(self.view_frame, |s| {
            if centred {
                s.center()
            } else {
                s.center_items_reset()
            }
        });
    }

    /// Turn the compass to the camera: each axis where it points on screen,
    /// the ones pointing away drawn faint.
    fn turn_compass(&mut self) {
        let camera = self.session.camera();
        let key = (camera.position, camera.target, camera.ortho.is_some());
        if self.compass.seen == Some(key) {
            return;
        }
        self.compass.seen = Some(key);

        let forward = (camera.target - camera.position).normalize_or_zero();
        let right = forward.cross(camera.up).normalize_or_zero();
        let up = right.cross(forward);
        let (c, reach) = (COMPASS / 2.0, COMPASS / 2.0 - 11.0);
        let axes = [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X, -Vec3::Y, -Vec3::Z];
        let at = |axis: Vec3| (c + axis.dot(right) * reach, c - axis.dot(up) * reach);
        // Each positive axis's line, from the middle out to its dot.
        for (i, beads) in self.compass.beads.clone().into_iter().enumerate() {
            let (x, y) = at(axes[i]);
            let away = axes[i].dot(forward) > 0.1;
            for (k, bead) in beads.into_iter().enumerate() {
                let t = (k as f32 + 1.0) / (BEADS as f32 + 1.0) * 0.8;
                let (bx, by) = (c + (x - c) * t, c + (y - c) * t);
                self.ui.restyle(bead, |s| {
                    s.absolute(bx - 1.5, by - 1.5)
                        .opacity(if away { 0.35 } else { 0.9 })
                });
            }
        }
        // The dots, the nearer over the farther: put back in the dial from
        // the farthest.
        let mut order: Vec<usize> = (0..6).collect();
        order.sort_by(|a, b| axes[*b].dot(forward).total_cmp(&axes[*a].dot(forward)));
        for i in order {
            let (node, _) = self.compass.axes[i];
            let depth = axes[i].dot(forward);
            let (x, y) = at(axes[i]);
            let size = if i < 3 { 18.0 } else { 12.0 };
            // An axis along the view sits in the middle, on the switch:
            // the view already looks from it, so it goes.
            let along = depth.abs() > 0.95;
            self.ui.restyle(node, |s| {
                let s = s
                    .absolute(x - size / 2.0, y - size / 2.0)
                    .opacity(if depth > 0.1 { 0.5 } else { 1.0 });
                if along {
                    s.hidden()
                } else {
                    s.shown()
                }
            });
            self.ui.move_to(node, self.compass.dial);
        }
        // The switch over every axis: always there to click.
        self.ui.move_to(self.compass.middle, self.compass.dial);
        let label = if camera.ortho.is_some() {
            "Iso"
        } else {
            "Persp"
        };
        self.ui.set_text(self.compass.label, label);
    }

    // --- keys and preferences -------------------------------------------

    /// Do what a key of the keymap does. False when it does nothing now
    /// and the key should go on as a key: K outside play.
    fn answer_key(&mut self, action: Action) -> bool {
        match action {
            Action::KeepSimulation if !self.session.is_playing() => false,
            // Shift Space maximizes what is under the pointer, as in
            // Unity: a dock, or else the view.
            Action::Maximize => {
                let (x, y) = self.ui.pointer();
                let zoom = match self.maximized {
                    Some(z) => z,
                    None => self
                        .docks
                        .stack_at(&self.ui, x, y)
                        .map_or(Zoom::View, Zoom::Stack),
                };
                self.toggle_zoom(zoom);
                true
            }
            action => {
                self.run(action);
                true
            }
        }
    }

    /// The key Preferences › Keys was listening for: the command's now,
    /// taken from whichever had it. Esc keeps the old one.
    fn capture_key(&mut self, key: Key) {
        let Some(id) = self.preferences.capturing() else {
            return;
        };
        self.preferences.stop_capture();
        if key == Key::Escape {
            self.preferences.note(&mut self.ui, "Kept as it was.", false);
            self.preferences.show_keys(&mut self.ui, &self.keymap);
            return;
        }
        let chord = Shortcut::held(key, self.ui.modifiers());
        let label = chord.label();
        let name = self.command_label(id);
        let (note, warn) = match self.keymap.set(id, chord) {
            Some(other) => (
                format!("{label} is {name} now — {other} has no key; its ↶ gives it back its own."),
                true,
            ),
            None => (format!("{label} is {name} now."), false),
        };
        self.preferences.note(&mut self.ui, &note, warn);
        self.keys_changed();
    }

    /// A command's label, by its keymap name.
    fn command_label(&self, id: &'static str) -> &'static str {
        self.keymap
            .commands()
            .iter()
            .find(|c| c.id == id)
            .map_or(id, |c| c.label)
    }

    /// The keymap changed: written down, shown, and in the menus.
    fn keys_changed(&mut self) {
        if let Some(dir) = &self.config_dir {
            if let Err(e) = self.keymap.save(dir) {
                self.session.say(Level::Error, e);
            }
        }
        self.preferences.show_keys(&mut self.ui, &self.keymap);
    }

    /// What Preferences asked for.
    fn preference(&mut self, asked: crate::preferences::Asked) {
        use crate::preferences::Asked;
        match asked {
            Asked::Theme(change) => self.change_theme(change),
            Asked::Capture(id) => {
                let name = self.command_label(id);
                self.preferences.note(
                    &mut self.ui,
                    &format!("Press the new key for {name}, with what it needs held. Esc cancels."),
                    false,
                );
            }
            Asked::ResetKey(id) => {
                self.keymap.reset(id);
                self.preferences
                    .note(&mut self.ui, "Back to its own key.", false);
                self.keys_changed();
            }
            Asked::ResetAllKeys => {
                self.keymap.reset_all();
                self.preferences
                    .note(&mut self.ui, "Every key is the default again.", false);
                self.keys_changed();
            }
            Asked::CodeEditor(command) => {
                self.personal.code_editor = command;
                self.personal_changed();
            }
            Asked::Blender(path) => {
                self.personal.blender = path;
                self.personal_changed();
            }
            Asked::ToggleOpenLastScene => {
                self.personal.open_last_scene = !self.personal.open_last_scene;
                self.personal_changed();
            }
            Asked::IdleFps(fps) => {
                self.personal.idle_fps = fps.clamp(1, 60);
                self.personal_changed();
            }
            Asked::ApplyLayout(name) => self.run(Action::Layout(name)),
            Asked::RenameLayout(old, new) => {
                let renamed = self
                    .config_dir
                    .as_deref()
                    .ok_or_else(|| "no settings folder".to_string())
                    .and_then(|dir| layouts::rename(dir, &old, &new));
                match renamed {
                    Ok(()) => {
                        if self.layout_name.as_deref() == Some(old.as_str()) {
                            self.layout_name = Some(new.clone());
                            self.update_layout_button();
                        }
                        self.session
                            .say(Level::Info, format!("layout {old} is {new} now"));
                    }
                    Err(e) => self.session.say(Level::Error, e),
                }
                self.preferences.forget_layouts();
                self.show_preferences();
            }
            Asked::DeleteLayout(name) => {
                let deleted = self
                    .config_dir
                    .as_deref()
                    .ok_or_else(|| "no settings folder".to_string())
                    .and_then(|dir| layouts::delete(dir, &name));
                match deleted {
                    Ok(()) => self
                        .session
                        .say(Level::Info, format!("deleted layout {name}")),
                    Err(e) => self.session.say(Level::Error, e),
                }
                self.preferences.forget_layouts();
                self.show_preferences();
            }
        }
    }

    /// The person's preferences changed: written down, and in force.
    fn personal_changed(&mut self) {
        self.save_personal();
        self.apply_personal();
        self.preferences.show_personal(&mut self.ui, &self.personal);
    }

    fn save_personal(&mut self) {
        self.personal_dirty = false;
        if let Some(dir) = &self.config_dir {
            if let Err(e) = self.personal.save(dir) {
                self.session.say(Level::Error, e);
            }
        }
    }

    /// Put the preferences in force where they are read: the Console's
    /// editor command, the Blender every `.blend` is read with.
    fn apply_personal(&mut self) {
        self.bottom.editor_command = self.personal.editor().to_string();
        // Blender is found through SCRAP_BLENDER (`scrap_import::blend`):
        // a path chosen here is that, and choosing none puts back what the
        // environment said.
        static ORIGINAL: std::sync::OnceLock<Option<std::ffi::OsString>> =
            std::sync::OnceLock::new();
        let original = ORIGINAL.get_or_init(|| std::env::var_os("SCRAP_BLENDER"));
        let chosen = self.personal.blender.trim();
        if !chosen.is_empty() {
            std::env::set_var("SCRAP_BLENDER", chosen);
            self.blender_set = true;
        } else if self.blender_set {
            match original {
                Some(v) => std::env::set_var("SCRAP_BLENDER", v),
                None => std::env::remove_var("SCRAP_BLENDER"),
            }
            self.blender_set = false;
        }
    }

    /// Every page of Preferences as things stand.
    fn show_preferences(&mut self) {
        self.preferences
            .show_where(&mut self.ui, self.config_dir.as_deref());
        self.preferences.show_keys(&mut self.ui, &self.keymap);
        self.preferences.show_personal(&mut self.ui, &self.personal);
        let saved = self
            .config_dir
            .as_deref()
            .map(layouts::saved)
            .unwrap_or_default();
        self.preferences.show_layouts(&mut self.ui, saved);
    }

    /// Preferences in its window (or its tab, when someone docked it), on
    /// `page`, or where it was.
    fn open_preferences(&mut self, page: Option<crate::preferences::Page>) {
        if let Some(page) = page {
            self.preferences.set_page(&mut self.ui, page);
        }
        self.preferences.forget_layouts();
        self.show_preferences();
        if self.docks.region_of(Panel::Preferences).is_some() {
            self.show_panel(Panel::Preferences);
        } else if self.floats.iter().any(|f| f.panel == Panel::Preferences) {
            self.raise = Some(Panel::Preferences.name().to_string());
        } else {
            self.float(Panel::Preferences);
        }
    }

    /// A floating panel asked for while its window is open: the window
    /// code brings it to the front.
    pub fn take_raise(&mut self) -> Option<String> {
        self.raise.take()
    }

    /// The page Preferences shows.
    pub fn preferences_page(&self) -> crate::preferences::Page {
        self.preferences.page()
    }

    /// Which key does what.
    pub fn keymap(&self) -> &crate::keymap::Keymap {
        &self.keymap
    }

    /// The person's preferences.
    pub fn personal(&self) -> &crate::preferences::Personal {
        &self.personal
    }

    /// The menu bar with the person's keys: what macOS's bar is built from.
    pub fn menu_bar(&self) -> Vec<(&'static str, Vec<MenuItem>)> {
        self.keymap.label_bar(menu::bare_menu_bar())
    }

    /// The key the menu line for `action` shows now: `Some(None)` for a
    /// command with no key, `None` for a line the keymap has no say in.
    pub fn menu_shortcut(&self, action: &Action) -> Option<Option<Shortcut>> {
        self.keymap.shortcut_for(action)
    }

    // --- floating windows ---------------------------------------------

    /// Tear a panel off into a window of its own.
    pub(crate) fn float(&mut self, panel: Panel) {
        if self.floats.iter().any(|f| f.panel == panel) {
            return;
        }
        let Some(root) = self.docks.take(&mut self.ui, panel) else {
            return;
        };
        let used: Vec<f32> = self.floats.iter().map(|f| f.origin.0).collect();
        let x = (0..)
            .map(|i| FLOAT_X + i as f32 * FLOAT_STEP)
            .find(|x| !used.contains(x))
            .expect("there is always a free place");
        let [width, height] = self.float_size(panel);
        let ui = &mut self.ui;
        let frame = ui.add(
            ui.root(),
            Style::column()
                .absolute(x, 0.0)
                .size(width, height)
                .background(BG)
                .padding(3.0),
        );
        ui.set_name(frame, format!("float {}", panel.name()));
        let card = ui.add(
            frame,
            Style::column()
                .full()
                .background(SURFACE)
                .radius(RADIUS_MD)
                .clip(),
        );
        let strip = ui.add(
            card,
            Style::row()
                .height(32.0)
                .fixed()
                .full_width()
                .padding_x(SPACE_3)
                .gap(SPACE_2)
                .center_items(),
        );
        ui.add_text(strip, text(), panel.label());
        spacer(ui, strip);
        let dock_button = button(
            ui,
            strip,
            &format!("dock {}", panel.name()),
            "Dock back",
            false,
        );
        // A panel that lives in a window has no dock to go back to.
        if panel.floats() {
            ui.restyle(dock_button, |s| s.hidden());
        }
        let body = ui.add(card, Style::column().fill().full_width());
        ui.move_to(root, body);
        ui.restyle(root, |s| s.shown());
        self.floats.push(Float {
            panel,
            frame,
            dock_button,
            origin: (x, 0.0),
        });
        self.sync_visible();
        self.refresh();
    }

    /// Bring a panel up wherever it is: its tab on top, back in a dock if
    /// it was closed, its area shown.
    fn show_panel(&mut self, panel: Panel) {
        if self.floats.iter().any(|f| f.panel == panel) {
            return;
        }
        if self.docks.region_of(panel).is_none() {
            self.docks.give_back(&mut self.ui, panel);
        }
        self.docks.activate(&mut self.ui, panel);
        if let Some(r) = self.docks.region_of(panel) {
            self.panels[r] = true;
        }
        if self.maximized.is_some() && self.maximized != Some(Zoom::Stack(panel)) {
            self.maximized = None;
        }
        self.sync_visible();
    }

    /// Put a floating panel back where it was docked, and close its window.
    pub(crate) fn dock_back(&mut self, panel: Panel) {
        let Some(at) = self.floats.iter().position(|f| f.panel == panel) else {
            return;
        };
        let float = self.floats.remove(at);
        self.docks.give_back(&mut self.ui, panel);
        self.ui.remove(float.frame);
        self.sync_visible();
        self.refresh();
    }

    /// The panels in windows of their own, with their titles: what the
    /// window code keeps a window open for.
    pub fn floating(&self) -> Vec<(String, String)> {
        self.floats
            .iter()
            .map(|f| {
                (
                    f.panel.name().to_string(),
                    format!("{} — scrap", f.panel.label()),
                )
            })
            .collect()
    }

    fn float_of(&self, name: &str) -> Option<&Float> {
        self.floats.iter().find(|f| f.panel.name() == name)
    }

    /// A floating panel's window was resized, in logical pixels.
    /// A panel by its name (`settings`, `inspector`…) into a window of its
    /// own, as its tab's right-click menu does.
    pub fn float_panel(&mut self, name: &str) {
        if let Some(panel) = Panel::from_name(name) {
            self.float(panel);
        }
    }

    pub fn resize_float(&mut self, name: &str, width: f32, height: f32) {
        let Some(frame) = self.float_of(name).map(|f| f.frame) else {
            return;
        };
        self.ui.restyle(frame, |s| s.size(width, height));
        let entry = self
            .personal
            .windows
            .entry(name.to_string())
            .or_insert([f32::NAN, f32::NAN, width, height]);
        entry[2] = width;
        entry[3] = height;
        self.personal_dirty = true;
    }

    /// An event from a floating panel's window, pointer in its logical
    /// pixels.
    pub fn handle_float(&mut self, name: &str, event: &InputEvent) {
        let Some((ox, oy)) = self.float_of(name).map(|f| f.origin) else {
            return;
        };
        match event {
            InputEvent::MouseMoved { x, y } => self.handle(&InputEvent::MouseMoved {
                x: x + ox,
                y: y + oy,
            }),
            other => self.handle(other),
        }
    }

    /// Its window closed: the panel goes back to a dock — or, one that
    /// lives in a window (Preferences), away until opened again.
    pub fn close_float(&mut self, name: &str) {
        let Some(panel) = self.float_of(name).map(|f| f.panel) else {
            return;
        };
        if !panel.floats() {
            self.dock_back(panel);
            return;
        }
        let Some(at) = self.floats.iter().position(|f| f.panel == panel) else {
            return;
        };
        let float = self.floats.remove(at);
        self.docks.put_away(&mut self.ui, panel);
        self.ui.remove(float.frame);
        self.preferences.stop_capture();
        self.save_personal();
        self.sync_visible();
        self.refresh();
    }

    /// How big a panel's window is: as it was left, or its first size.
    fn float_size(&self, panel: Panel) -> [f32; 2] {
        match self.personal.windows.get(panel.name()) {
            Some([_, _, w, h]) if *w > 100.0 && *h > 100.0 => [*w, *h],
            _ if panel == Panel::Preferences => [780.0, 560.0],
            _ => [420.0, 560.0],
        }
    }

    /// Where a floating panel's window goes when it opens: its size, and
    /// where it was left on the screen, if it was — logical pixels.
    pub fn float_place(&self, name: &str) -> ([f32; 2], Option<[f32; 2]>) {
        let size = Panel::from_name(name).map_or([420.0, 560.0], |p| self.float_size(p));
        let at = self
            .personal
            .windows
            .get(name)
            .filter(|[x, y, _, _]| x.is_finite() && y.is_finite())
            .map(|[x, y, _, _]| [*x, *y]);
        (size, at)
    }

    /// A floating panel's window moved on the screen, in logical pixels:
    /// it opens there next time.
    pub fn moved_float(&mut self, name: &str, x: f32, y: f32) {
        let size = self.float_place(name).0;
        let entry = self
            .personal
            .windows
            .entry(name.to_string())
            .or_insert([x, y, size[0], size[1]]);
        entry[0] = x;
        entry[1] = y;
        self.personal_dirty = true;
    }


    /// Draw a floating panel's window. `seen` is the picture generation
    /// this renderer has: pictures are given to it again when it is old.
    pub fn draw_float(
        &mut self,
        name: &str,
        renderer: &mut UiRenderer,
        seen: &mut u64,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let Some((ox, oy)) = self.float_of(name).map(|f| f.origin) else {
            return;
        };
        if *seen != self.picture_generation {
            *seen = self.picture_generation;
            let gpu = self.session.gpu();
            renderer.set_image(gpu, SCENE, self.session.frame_target().view());
            if let Some(target) = self.session.preview_target() {
                renderer.set_image(gpu, CAMERA_PREVIEW, target.view());
            }
            if let Some(target) = self.screens.target() {
                renderer.set_image(gpu, screens::CANVAS, target.view());
            }
            for (image, (size, pixels)) in &self.pictures {
                renderer.set_image_rgba(gpu, *image, *size, *size, pixels);
            }
        }
        renderer.set_origin(ox, oy);
        let ground = self.ui.tint(BG);
        let gpu = self.session.gpu();
        renderer.draw(gpu, view, width, height, &mut self.ui, Some(ground));
    }

    /// One dab of the foliage brush where the view's pixel shows the
    /// ground: at most ten a second, Shift erasing.
    fn foliage_dab(&mut self, vx: f32, vy: f32) {
        let Some((last, steps)) = self.foliage_stroke else {
            return;
        };
        if last.elapsed() < Duration::from_millis(100) {
            return;
        }
        let Some(what) = self.foliage_what.clone() else {
            self.session.say(
                Level::Warning,
                "foliage brush: click a model or prefab in Project to paint it",
            );
            self.foliage_stroke = None;
            return;
        };
        let Some(at) = self
            .session
            .point_under(vx.max(0.0) as u32, vy.max(0.0) as u32)
        else {
            return;
        };
        let erase = self.ui.modifiers().0;
        self.foliage_seed = self.foliage_seed.wrapping_add(1);
        match self.session.paint_foliage(
            &what,
            at,
            self.foliage_radius,
            0.4,
            false,
            erase,
            self.foliage_seed,
        ) {
            Ok(0) => self.foliage_stroke = Some((Instant::now(), steps)),
            Ok(_) => self.foliage_stroke = Some((Instant::now(), steps + 1)),
            Err(e) => {
                self.session.say(Level::Error, e.to_string());
                self.foliage_stroke = None;
            }
        }
    }

    /// The selection's spline: its points as handles over the view, one
    /// per point, where the camera sees them now.
    fn place_spline_handles(&mut self) {
        let points: Vec<Vec3> = self
            .session
            .selected()
            .filter(|_| !self.session.is_playing() && !self.session.is_game_view())
            .and_then(|id| {
                let text = self
                    .session
                    .inspect(id)?
                    .into_iter()
                    .find(|f| f.name == "spline")?
                    .value;
                let spline: scrap::Spline = scrap::ron::from_str(&text).ok()?;
                let world = self.session.world_matrix(id)?;
                Some(
                    spline
                        .points
                        .iter()
                        .map(|p| world.transform_point3(*p))
                        .collect(),
                )
            })
            .unwrap_or_default();
        let Some(frame) = self.ui.parent(self.face_box) else {
            return;
        };
        while self.spline_handles.len() > points.len() {
            let handle = self.spline_handles.pop().expect("more than none");
            self.ui.remove(handle);
        }
        while self.spline_handles.len() < points.len() {
            let i = self.spline_handles.len();
            let handle = self.ui.add(
                frame,
                Style::row()
                    .absolute(0.0, 0.0)
                    .size(14.0, 14.0)
                    .radius(3.0)
                    .background(ACCENT)
                    .border(2.0, TEXT)
                    .draggable(),
            );
            self.ui.set_name(handle, format!("spline point {i}"));
            self.spline_handles.push(handle);
        }
        let scale = self.ui.viewport().2;
        for (handle, point) in self.spline_handles.clone().into_iter().zip(points) {
            match self.session.screen_of(point) {
                Some((x, y)) => {
                    let (x, y) = (x / scale - 7.0, y / scale - 7.0);
                    self.ui.restyle(handle, |s| s.shown().absolute(x, y));
                }
                None => self.ui.restyle(handle, |s| s.hidden()),
            }
        }
    }

    /// A spline point dragged: to the ground under the pointer, the fence's
    /// own posts not counting. One drag is one undo step.
    fn spline_handle_event(&mut self, i: usize, event: &Event) {
        match event {
            Event::Drag { x, y, .. } => {
                let Some(id) = self.session.selected() else {
                    return;
                };
                let view = self.ui.rect(self.viewport);
                let scale = self.ui.viewport().2;
                let (vx, vy) = ((x - view.x) * scale, (y - view.y) * scale);
                let Some(at) =
                    self.session
                        .point_under_without(vx.max(0.0) as u32, vy.max(0.0) as u32, id)
                else {
                    return;
                };
                let (Some(world), Some(field)) = (
                    self.session.world_matrix(id),
                    self.session
                        .inspect(id)
                        .and_then(|f| f.into_iter().find(|f| f.name == "spline")),
                ) else {
                    return;
                };
                let Ok(mut spline) = scrap::ron::from_str::<scrap::Spline>(&field.value) else {
                    return;
                };
                let Some(point) = spline.points.get_mut(i) else {
                    return;
                };
                *point = world.inverse().transform_point3(at);
                let Ok(text) = scrap::ron::to_string(&spline) else {
                    return;
                };
                if self.session.set_field(id, "spline", &text).is_ok() {
                    let steps = match self.spline_drag {
                        Some((who, point, steps)) if who == id && point == i => {
                            self.session.squash_last(2);
                            steps
                        }
                        _ => 1,
                    };
                    self.spline_drag = Some((id, i, steps));
                }
            }
            Event::DragEnd { .. } => {
                self.spline_drag = None;
                self.refresh();
            }
            _ => {}
        }
    }

    /// The pointer over the view in face mode: outline the face under it,
    /// or push the face being dragged by how far the pointer has gone
    /// along it on screen, in steps of 5 cm.
    fn face_move(&mut self, vx: f32, vy: f32, over_view: bool) {
        if let Some(drag) = self.face_drag.as_mut() {
            let (ox, oy) = drag.per_metre;
            let along = ((vx - drag.from.0) * ox + (vy - drag.from.1) * oy) / (ox * ox + oy * oy);
            let total = (along / 0.05).round() * 0.05;
            let step = total - drag.pushed;
            if step.abs() > 1e-4 {
                // Pulled in past the opposite face, it stays where it last
                // could be.
                if self.session.push_face(drag.id, drag.face, step).is_ok() {
                    if drag.pushed != 0.0 {
                        self.session.squash_last(2);
                    }
                    drag.pushed = total;
                }
            }
        } else {
            let hover = over_view
                .then(|| {
                    self.session
                        .face_under(vx.max(0.0) as u32, vy.max(0.0) as u32)
                })
                .flatten();
            if hover == self.face_hover {
                return;
            }
            self.face_hover = hover;
        }
        self.show_face();
    }

    fn face_press(&mut self, vx: f32, vy: f32) {
        let Some((id, face)) = self.face_hover else {
            return;
        };
        let Some((_, per_metre)) = self.session.face_on_screen(id, face) else {
            return;
        };
        if per_metre.0.abs() + per_metre.1.abs() < 1.0 {
            // Seen edge on: dragging it would be a guess.
            return;
        }
        let _ = self.session.select(Some(id));
        self.face_drag = Some(FaceDrag {
            id,
            face,
            from: (vx, vy),
            per_metre,
            pushed: 0.0,
        });
        self.refresh();
    }

    /// Outline the face under the pointer or being dragged: the box its
    /// corners make on screen, named.
    fn show_face(&mut self) {
        let shown = self
            .face_drag
            .as_ref()
            .map(|d| (d.id, d.face))
            .or(self.face_hover)
            .filter(|_| self.faces);
        let place = shown.and_then(|(id, face)| {
            let (corners, _) = self.session.face_on_screen(id, face)?;
            Some((id, face, corners))
        });
        let Some((id, face, corners)) = place else {
            self.ui.restyle(self.face_box, |s| s.hidden());
            return;
        };
        let scale = self.ui.viewport().2;
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (x, y) in corners {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let (x, y) = (x0 / scale, y0 / scale);
        let (w, h) = (((x1 - x0) / scale).max(4.0), ((y1 - y0) / scale).max(4.0));
        self.ui
            .restyle(self.face_box, |s| s.shown().absolute(x, y).size(w, h));
        let name = self.session.entity_name(id).unwrap_or_default();
        let pushed = self
            .face_drag
            .as_ref()
            .filter(|d| d.pushed != 0.0)
            .map(|d| format!("  {:+.2} m", d.pushed))
            .unwrap_or_default();
        if let Some(label) = self.ui.children(self.face_box).first().copied() {
            self.ui.set_text(label, &format!("{name} {face:?}{pushed}"));
        }
    }

    /// One dab of the terrain brush where the view's pixel shows the ground:
    /// raise, Shift lower, Ctrl (Cmd) flatten to where the stroke began. Dabs are
    /// at most ten a second — each is a line in the terrain's file.
    fn dab(&mut self, x: f32, y: f32, first: bool) {
        let (x, y) = (x.max(0.0) as u32, y.max(0.0) as u32);
        let Some(at) = self.session.point_under(x, y) else {
            return;
        };
        if !first {
            if let Some((when, _)) = self.stroke {
                if when.elapsed().as_secs_f32() < 0.1 {
                    return;
                }
            }
        }
        let terrain = self
            .session
            .selected()
            .filter(|id| self.session.entity_model(*id).is_some())
            .or_else(|| {
                self.session.entities().into_iter().find(|id| {
                    self.session.entity_model(*id).is_some_and(|m| {
                        self.session
                            .project()
                            .is_some_and(|p| p.assets().join(format!("{m}.scrterrain")).is_file())
                    })
                })
            });
        let Some(terrain) = terrain else {
            self.session.say(
                Level::Warning,
                "no terrain to sculpt: Entity › Terrain makes one",
            );
            self.sculpt = false;
            self.refresh();
            return;
        };
        let (shift, ctrl, _, command) = self.ui.modifiers();
        let flatten = ctrl || command;
        let flatten_to = match self.stroke {
            Some((_, h)) if !first => h,
            _ => at.y,
        };
        let result = if flatten {
            self.session.sculpt(terrain, at, 3.0, flatten_to, true)
        } else {
            self.session
                .sculpt(terrain, at, 3.0, if shift { -0.3 } else { 0.3 }, false)
        };
        if let Err(e) = result {
            self.session.say(Level::Error, e.to_string());
            self.sculpt = false;
            self.stroke = None;
            self.refresh();
            return;
        }
        self.stroke = Some((Instant::now(), flatten_to));
    }

    /// Where the studio keeps its own layout: next to the session's view
    /// settings, in the project's `.scrap/` (not in git).
    fn layout_file(&self) -> Option<std::path::PathBuf> {
        self.session
            .project()
            .map(|p| p.root().join(".scrap").join("studio.ron"))
    }

    /// Where the panels are and how big the areas are, as a preset holds
    /// it (`crate::layouts`).
    fn layout(&self) -> Layout {
        // The sizes asked for, not the ones laid out: a folded dock is
        // laid out at none, a maximized one at the whole window.
        let w = |n: NodeId| self.ui.style(n).layout.size.width.value().round();
        let lower = self
            .lower_before_wide
            .unwrap_or(self.ui.style(self.lower).layout.size.height.value())
            .round();
        Layout {
            sizes: [Some(w(self.left)), Some(w(self.right)), Some(lower)],
            name: self.layout_name.clone(),
            arrangement: Some(self.docks.arrangement().clone()),
            clear_on_play: None,
            project: None,
        }
    }

    /// The layout file's text: the layout, and the Console's Clear on Play.
    fn layout_text(&self) -> String {
        Layout {
            clear_on_play: Some(self.bottom.clear_on_play),
            project: Some(self.bottom.modes()),
            ..self.layout()
        }
        .write()
    }

    /// Put the panels back where they were last time.
    fn restore_layout(&mut self) {
        let Some(text) = self
            .layout_file()
            .and_then(|f| std::fs::read_to_string(f).ok())
        else {
            return;
        };
        let layout = Layout::read(&text);
        if let Some(on) = layout.clear_on_play {
            self.bottom.clear_on_play = on;
        }
        self.apply_layout(layout);
        self.saved_layout = self.layout_text_after_paint();
    }

    /// Put the panels where `layout` says, at its sizes. Panels in windows
    /// of their own are docked first, as Unity's layouts do.
    fn apply_layout(&mut self, layout: Layout) {
        let project_modes = layout.project.clone();
        for panel in self.floats.iter().map(|f| f.panel).collect::<Vec<_>>() {
            self.dock_back(panel);
        }
        let [left, right, lower] = layout.sizes;
        if let Some(v) = left {
            self.ui
                .restyle(self.left, |s| s.width(v.clamp(140.0, 900.0)));
        }
        if let Some(v) = right {
            self.ui
                .restyle(self.right, |s| s.width(v.clamp(140.0, 900.0)));
        }
        if let Some(v) = lower {
            self.ui
                .restyle(self.lower, |s| s.height(v.clamp(60.0, 900.0)));
        }
        if let Some(arrangement) = layout.arrangement {
            self.docks.set_arrangement(&mut self.ui, arrangement);
        }
        self.layout_name = layout.name;
        self.maximized = None;
        self.panels = [true; 3];
        if let Some(modes) = &project_modes {
            self.bottom.set_modes(&mut self.ui, modes);
        }
        self.sync_visible();
        self.update_layout_button();
    }

    /// The Layout button names the preset last chosen or saved.
    fn update_layout_button(&mut self) {
        let name = self.layout_name.clone().unwrap_or_else(|| "Layout".into());
        if let Some(t) = self.ui.children(self.toolbar.layout).first().copied() {
            self.ui.set_text(t, &name);
        }
    }

    /// The presets to choose from: the built-in ones, then this person's.
    fn layout_names(&self) -> Vec<String> {
        let mut names: Vec<String> = layouts::BUILT_IN.iter().map(|n| n.to_string()).collect();
        if let Some(dir) = &self.config_dir {
            names.extend(layouts::saved(dir));
        }
        names
    }

    /// The Layout button's list: every preset, the one in use checked,
    /// and Save and Delete.
    fn layout_menu(&self) -> Vec<MenuItem> {
        let mut items: Vec<MenuItem> = self
            .layout_names()
            .into_iter()
            .map(|n| {
                let on = self.layout_name.as_deref() == Some(n.as_str());
                MenuItem::new(&n, Action::Layout(n.clone())).checked(on)
            })
            .collect();
        items.push(MenuItem::separator());
        items.push(MenuItem::new("Save Layout As…", Action::SaveLayoutAs));
        items.push(MenuItem::new("Delete Layout…", Action::DeleteLayout));
        items
    }

    /// Where the person's own things are kept — their layouts and their
    /// colours: the per-user config folder (`layouts::config_dir`). A test
    /// points it elsewhere, and what is there is drawn at once.
    pub fn set_config_dir(&mut self, dir: impl Into<std::path::PathBuf>) {
        let dir = dir.into();
        self.config_dir = Some(dir.clone());
        self.theme.set_dir(dir.clone());
        let (keymap, errors) = crate::keymap::Keymap::load(Some(&dir));
        // A new generation all the same: the menus show these keys.
        let generation = self.keymap.generation + 1;
        self.keymap = keymap;
        self.keymap.generation = generation;
        let (personal, error) = crate::preferences::Personal::load(Some(&dir));
        self.personal = personal;
        for e in errors.into_iter().chain(error) {
            self.session.say(Level::Error, e);
        }
        self.apply_personal();
        self.preferences.forget_layouts();
        self.show_preferences();
        self.poll_theme();
        self.show_theme();
    }

    /// A panel's menu — its tab's right click and its strip's ⋮: what it
    /// offers of its own, then what every panel does.
    fn panel_menu(&self, panel: Panel) -> Vec<MenuItem> {
        let mut items = match panel {
            Panel::Inspector => {
                let debug = self.session.inspector_debug();
                vec![
                    MenuItem::new("Normal", Action::InspectorDebug(false)).checked(!debug),
                    MenuItem::new("Debug", Action::InspectorDebug(true)).checked(debug),
                    MenuItem::separator(),
                ]
            }
            Panel::Console => vec![
                MenuItem::new("Clear on Play", Action::ToggleClearOnPlay)
                    .checked(self.bottom.clear_on_play),
                MenuItem::new("Clear", Action::ClearConsole),
                MenuItem::separator(),
            ],
            _ => Vec::new(),
        };
        let maximized = self.maximized == Some(Zoom::Stack(panel));
        items.extend([
            MenuItem::new("Float in its own window", Action::Float(panel)),
            MenuItem::new("Maximize", Action::MaximizePanel(panel)).checked(maximized),
            MenuItem::new("Close Tab", Action::CloseTab(panel)),
        ]);
        items
    }

    /// Show the Inspector's padlock in its strip as it is.
    fn sync_lock(&mut self) {
        let on = self.inspector.is_locked();
        self.docks.set_locked(&mut self.ui, Panel::Inspector, on);
    }

    /// Tell the lower panels which of them are on top.
    fn sync_visible(&mut self) {
        self.show_panels();
        let on = |p| self.docks.is_showing(p);
        self.bottom.set_visible([
            on(Panel::Project),
            on(Panel::Console),
            on(Panel::History),
            on(Panel::Git),
        ]);
    }

    fn layout_text_after_paint(&mut self) -> String {
        self.ui.paint();
        self.layout_text()
    }

    /// Write the layout down when it changed.
    fn save_layout(&mut self) {
        let text = self.layout_text_after_paint();
        if text == self.saved_layout {
            return;
        }
        if let Some(file) = self.layout_file() {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(file, &text);
        }
        self.saved_layout = text;
    }

    /// Twice a second, pick up what changed on disk: the scene edited in a
    /// text editor or by git, an asset re-exported (DNA, postulate 1).
    /// The person's colours and the project's `.scrap/theme.ron`, when
    /// either changed: Nocturne's colours drawn as they say, with the
    /// editor running. A file that does not read says why in the Console
    /// and the last good colours stay.
    fn poll_theme(&mut self) {
        let root = self.session.project().map(|p| p.root().to_path_buf());
        let (changed, errors) = self.theme.poll(root.as_deref());
        for e in errors {
            self.session.say(Level::Error, e);
        }
        if changed {
            self.show_theme();
        }
    }

    /// Draw the tree in the theme's colours, and the Appearance page as it
    /// stands.
    fn show_theme(&mut self) {
        self.ui.set_palette(self.theme.palette());
        self.preferences.appearance.show(&mut self.ui, &self.theme);
    }

    /// Change the person's colours: at once here, and in their file.
    pub fn change_theme(&mut self, change: crate::appearance::Change) {
        if let Err(e) = self.theme.change(change) {
            self.session.say(Level::Error, e);
        }
        self.show_theme();
    }

    /// The colours as chosen and as drawn.
    pub fn theme(&self) -> &crate::appearance::Theme {
        &self.theme
    }


    fn poll_disk(&mut self) {
        if self.polled.elapsed().as_secs_f32() < 0.5 {
            return;
        }
        self.polled = Instant::now();
        self.save_layout();
        if self.personal_dirty {
            self.save_personal();
        }
        self.poll_theme();
        match self.session.reload_scene() {
            Ok(scrap_editor::SceneReload::Reloaded) => {
                self.session.say(
                    Level::Info,
                    "the scene changed on disk and was reloaded (undo takes it back)",
                );
            }
            Ok(scrap_editor::SceneReload::Conflict) => {
                if !self.conflict_said {
                    self.conflict_said = true;
                    self.session.say(
                        Level::Warning,
                        "the scene changed on disk and here too: saving keeps these edits, opening it again takes the file's",
                    );
                }
            }
            Ok(_) => {}
            Err(e) => self.session.say(Level::Error, e.to_string()),
        }
        if let Some(n) = self.session.poll_assets() {
            self.import_said = false;
            if n > 0 {
                self.session.say(
                    Level::Info,
                    format!("{n} assets changed on disk and were reloaded"),
                );
            }
        } else if !self.import_said
            && self
                .session
                .importing_for()
                .is_some_and(|t| t.as_secs_f32() > 1.0)
        {
            // A .blend being read by Blender: the view keeps going.
            self.import_said = true;
            self.session
                .say(Level::Info, "importing changed assets in the background…");
        }
    }

    /// A control's name, said when the pointer rests on it.
    fn update_tooltip(&mut self) {
        let hovered = self.ui.hovered();
        let now = Instant::now();
        match self.resting {
            Some((n, _)) if Some(n) == hovered => {}
            _ => {
                self.resting = hovered.map(|n| (n, now));
                if let Some(t) = self.tooltip.take() {
                    self.ui.remove(t);
                }
            }
        }
        let Some((node, since)) = self.resting else {
            return;
        };
        if self.tooltip.is_some()
            || self.popup.is_some()
            || now - since < std::time::Duration::from_millis(600)
        {
            return;
        }
        let Some(name) = self.ui.name(node) else {
            return;
        };
        let Some(tip) = tooltip(name) else {
            return;
        };
        // The key, as the keymap has it now: a rebound key reads here too.
        let key = tooltip_key(name)
            .and_then(|id| self.keymap.keys(id).first())
            .map(|k| k.label());
        let tip = match key {
            Some(key) => format!("{tip} ({key})"),
            None => tip.to_string(),
        };
        let tip = tip.as_str();
        let r = self.ui.rect(node);
        let root = self.ui.root();
        let (w, h, _) = self.ui.viewport();
        let x = r.x.min(w - 320.0).max(4.0);
        let y = if r.y + r.height + 34.0 > h {
            r.y - 30.0
        } else {
            r.y + r.height + 6.0
        };
        // The tool strip's run down: beside a button, not over the next.
        let (x, y) = if self.tools.contains(&node) {
            (r.x + r.width + 8.0, r.y + (r.height - 24.0) / 2.0)
        } else {
            (x, y)
        };

        let t = self.ui.add(
            root,
            Style::row()
                .absolute(x, y)
                .height(24.0)
                .padding_x(SPACE_3)
                .center_items()
                .radius(6.0)
                .background(NEUTRAL_900)
                .border(1.0, NEUTRAL_800),
        );
        self.ui.set_layer(t, true);
        self.ui.add_text(
            t,
            Style::default().text_size(11.5).text_color(TEXT).nowrap(),
            tip,
        );
        self.tooltip = Some(t);
    }

    /// Draw into `view` (the plain-bytes view of the window's frame or of an
    /// off-screen target), `width` × `height` pixels.
    pub fn draw(
        &mut self,
        renderer: &mut UiRenderer,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let size = self.session.size();
        if self.registered != Some(size) {
            renderer.set_image(
                self.session.gpu(),
                SCENE,
                self.session.frame_target().view(),
            );
            self.registered = Some(size);
            self.picture_generation += 1;
        }
        if let Some(target) = self.session.preview_target() {
            let size = (target.width, target.height);
            if self.cam_registered != Some(size) {
                renderer.set_image(self.session.gpu(), CAMERA_PREVIEW, target.view());
                self.cam_registered = Some(size);
                self.picture_generation += 1;
            }
        }
        if self.screens.new_target {
            if let Some(target) = self.screens.target() {
                renderer.set_image(self.session.gpu(), screens::CANVAS, target.view());
                self.screens.new_target = false;
                self.picture_generation += 1;
            }
        }
        for (image, size, pixels) in self.pending_images.drain(..) {
            renderer.set_image_rgba(self.session.gpu(), image, size, size, &pixels);
            self.pictures.insert(image, (size, pixels));
            self.picture_generation += 1;
        }
        let started = Instant::now();
        let gpu = self.session.gpu();
        let ground = self.ui.tint(BG);
        renderer.draw(gpu, view, width, height, &mut self.ui, Some(ground));
        self.last_draw_ms = started.elapsed().as_secs_f32() * 1e3;
    }

    /// The renderer for this studio's frames, targets of `format`.
    pub fn renderer(&self, format: wgpu::TextureFormat) -> UiRenderer {
        UiRenderer::new(self.session.gpu(), format.remove_srgb_suffix())
    }

    /// Bring the Project on top, go to an asset there, choose it and
    /// scroll to it: see [`Bottom::show_asset`] for what `file_or_name`
    /// may be. `false` when nothing in the project matches.
    pub fn show_in_project(&mut self, file_or_name: &str) -> bool {
        self.docks.activate(&mut self.ui, Panel::Project);
        self.sync_visible();
        self.bottom
            .show_asset(&mut self.ui, &self.session, file_or_name)
    }

    /// Bring every panel up to date now, whatever the stamp says.
    pub fn refresh(&mut self) {
        self.seen = None;
        self.update_panels(false);
    }

    fn update_panels(&mut self, moving: bool) {
        let s = &self.session;
        // While a drag or a game runs only what moves is redone: the
        // Inspector's numbers and the status line.
        let t0 = Instant::now();
        if !moving {
            self.hierarchy.update(&mut self.ui, s);
        }
        let t1 = Instant::now();
        if !moving {
            self.bottom.update(&mut self.ui, s);
        }
        let t2 = Instant::now();
        // The Inspector's object fields: the Project's pictures, and the
        // one a dragged entry would land in lit.
        self.inspector.set_pictures(self.bottom.pictures());
        self.inspector.update(&mut self.ui, s);
        let dragged = self
            .bottom
            .dragged(&self.ui)
            .map(crate::inspector::Dragged::Asset)
            .or_else(|| {
                let line = self.ui.dragging()?;
                self.hierarchy
                    .line_entity(line)
                    .map(crate::inspector::Dragged::Entity)
            });
        self.inspector.hover_drop(&mut self.ui, dragged.as_ref());
        // The Inspector lets go of an entity that went: its padlock opens.
        self.sync_lock();
        let t3 = Instant::now();
        self.update_toolbar();
        self.update_status();
        if std::env::var_os("SCRAP_STUDIO_TIMING").is_some() {
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1e3;
            eprintln!(
                "  hierarchy {:.1} bottom {:.1} inspector {:.1} bars {:.1} ms",
                ms(t0, t1),
                ms(t1, t2),
                ms(t2, t3),
                ms(t3, Instant::now())
            );
        }
    }

    fn update_toolbar(&mut self) {
        let s = &self.session;
        let t = &self.toolbar;
        let ui = &mut self.ui;
        let name = s
            .scene_path()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());
        ui.set_text(t.scene_name, &name);
        let modified = s.is_modified();
        ui.restyle(t.modified, |st| {
            st.opacity(if modified { 1.0 } else { 0.0 })
        });
        let game = s.is_game_view();
        for (i, (_, _, tool)) in TOOLS.iter().enumerate() {
            let on = match tool {
                None => s.hand(),
                Some(tool) => !s.hand() && s.tool() == *tool,
            };
            set_tool_button(ui, self.tools[i], on);
        }
        // The tools are the Scene view's: the Game view has none.
        ui.restyle(
            self.tool_strip,
            |st| {
                if game {
                    st.hidden()
                } else {
                    st.shown()
                }
            },
        );
        let center = s.pivot() == Pivot::Center;
        set_word(
            ui,
            self.pivot_button,
            if center { "Center" } else { "Pivot" },
        );
        let local = s.space() == Space::Local;
        set_word(
            ui,
            self.space_button,
            if local { "Local" } else { "Global" },
        );
        set_word_toggle(ui, self.grid_button, s.show_grid());
        let playing = s.is_playing() || s.is_game_running();
        set_icon_button(
            ui,
            t.play,
            if playing { "square" } else { "play" },
            playing,
            true,
        );
        set_icon_button(ui, t.pause, "pause", s.is_paused(), true);
        set_icon_button(ui, t.step, "step-forward", false, true);
        ui.restyle(t.play_group, |st| {
            st.border(1.0, if playing { ACCENT.alpha(60) } else { DIVIDER })
        });
        set_icon_button(ui, t.undo, "undo-2", false, s.can_undo());
        set_icon_button(ui, t.redo, "redo-2", false, s.can_redo());
        set_button_primary(ui, t.save, modified);
        set_icon_button(ui, self.snap, "magnet", s.snap().meters > 0.0, true);
        set_icon_button(ui, self.colliders_button, "box", self.colliders, true);
        set_icon_button(ui, self.sculpt_button, "mountain", self.sculpt, true);
        set_icon_button(ui, self.faces_button, "square", self.faces, true);
        set_icon_button(ui, self.foliage_button, "sparkles", self.foliage, true);
        let prefab = s.is_prefab();
        ui.restyle(self.prefab_bar, |st| {
            if prefab {
                st.shown()
            } else {
                st.hidden()
            }
        });
        if prefab {
            let name = s
                .scene_path()
                .and_then(|p| p.file_stem())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.set_text(self.prefab_name, &format!("Prefab: {name}"));
        }
        set_view_tab(ui, self.tab_scene, !game);
        set_view_tab(ui, self.tab_game, game);
        ui.restyle(self.view_frame, |st| {
            st.border(
                1.0,
                if playing {
                    ACCENT
                } else {
                    scrap_ui::Color::TRANSPARENT
                },
            )
        });
        let menus = self.menu_stamp();
        if self.menu_seen.as_ref() != Some(&menus) {
            self.menu_seen = Some(menus);
            self.menu_revision += 1;
        }
    }

    fn update_status(&mut self) {
        let s = &self.session;
        let ui = &mut self.ui;
        ui.set_text(
            self.status.entities,
            &format!("{} entities", s.entity_count()),
        );
        let n = s.selection().len();
        ui.set_text(
            self.status.selected,
            &if n == 0 {
                String::new()
            } else {
                format!("{n} selected")
            },
        );
        let (_, w, e) = s.console_counts();
        ui.set_text(
            self.status.problems,
            &match (w, e) {
                (0, 0) => String::new(),
                (w, 0) => format!("{w} warnings"),
                (0, e) => format!("{e} errors"),
                (w, e) => format!("{e} errors, {w} warnings"),
            },
        );
        ui.restyle(self.status.problems, |st| {
            st.text_color(if e > 0 { ERROR } else { WARNING })
        });
        let mode = match (s.is_playing(), s.is_paused()) {
            (true, true) => "simulation paused",
            (true, false) => "simulating",
            _ if s.is_game_running() => "playing",
            _ if s.is_prefab() => "prefab mode",
            _ => "editing",
        };
        ui.set_text(self.status.mode, mode);
        ui.restyle(self.status.mode, |st| {
            st.text_color(if s.is_playing() || s.is_game_running() { ACCENT } else { MUTED })
        });
        self.update_status_line();
    }

    /// The Console's newest line at the status bar's left, in its level's
    /// colour: a warning or an error stays until a newer line, an info
    /// goes muted after a while. One line, cut to the room there is.
    fn update_status_line(&mut self) {
        let said = self
            .session
            .last_said()
            .map(|(line, n)| (n, line.level, line.text.clone()));
        let Some((n, level, text)) = said else {
            if self.status_line.take().is_some() {
                self.ui.set_text(self.status.console_text, "");
                self.ui.restyle(self.status.console_icon, |s| s.hidden());
            }
            return;
        };
        let width = self.ui.rect(self.status.console).width;
        let now = Instant::now();
        let fade = |l: &StatusLine| l.level == Level::Info && now - l.at >= STATUS_FADE;
        match &mut self.status_line {
            Some(l) if l.said == n && (l.width - width).abs() < 1.0 && l.faded == fade(l) => {
                return;
            }
            Some(l) if l.said == n => l.width = width,
            _ => {
                self.status_line = Some(StatusLine {
                    said: n,
                    level,
                    text: text.lines().next().unwrap_or_default().to_string(),
                    at: now,
                    width,
                    faded: false,
                })
            }
        }
        let line = self.status_line.as_mut().expect("just set");
        line.faded = fade(line);
        let (glyph, color) = match line.level {
            Level::Error => ("circle-alert", ERROR),
            Level::Warning => ("triangle-alert", WARNING),
            Level::Info if line.faded => ("info", MUTED),
            Level::Info => ("info", LABEL),
        };
        // About six pixels a character at this size; the box clips what
        // the guess lets past.
        let room = ((width - 24.0) / 6.0).max(0.0) as usize;
        let shown = if width <= 0.0 || line.text.chars().count() <= room {
            line.text.clone()
        } else {
            let cut: String = line.text.chars().take(room.saturating_sub(1)).collect();
            format!("{}…", cut.trim_end())
        };
        self.ui.set_text(self.status.console_text, &shown);
        self.ui
            .restyle(self.status.console_text, |s| s.text_color(color));
        self.ui.set_icon(self.status.console_icon, glyph);
        self.ui
            .restyle(self.status.console_icon, |s| s.shown().text_color(color));
    }

    // --- events ----------------------------------------------------------

    fn dispatch(&mut self, node: NodeId, event: &Event, requests: &mut Requests) {
        if let Some(i) = self.spline_handles.iter().position(|h| *h == node) {
            self.spline_handle_event(i, event);
            return;
        }
        // Quick Search takes what is aimed at it.
        if let Some(q) = &self.search {
            let (field, overlay) = (q.field, q.overlay);
            let hit = q
                .hits
                .iter()
                .find(|(n, _)| *n == node)
                .map(|(_, h)| h.clone());
            match event {
                Event::Changed(text) if node == field => {
                    let text = text.clone();
                    self.fill_search(&text);
                    return;
                }
                Event::Submit(_) if node == field => {
                    let first = self
                        .search
                        .as_ref()
                        .and_then(|q| q.hits.first())
                        .map(|(_, h)| h.clone());
                    self.close_search();
                    if let Some(h) = first {
                        self.go_to(h, requests);
                    }
                    return;
                }
                Event::Cancel if node == field => {
                    self.close_search();
                    return;
                }
                Event::Click { .. } if hit.is_some() => {
                    self.close_search();
                    self.go_to(hit.expect("checked"), requests);
                    return;
                }
                Event::Click { .. } if node == overlay => {
                    self.close_search();
                    return;
                }
                _ if node == field => return,
                _ => {}
            }
        }
        // A dialog takes everything aimed at it.
        if let Some(p) = &self.prompt {
            let (field, ok, cancel, overlay) = (p.field, p.ok, p.cancel, p.overlay);
            match event {
                Event::Submit(_) if node == field => {
                    self.answer();
                    return;
                }
                Event::Click { .. } if node == ok => {
                    self.answer();
                    return;
                }
                Event::Cancel if node == field => {
                    self.close_prompt();
                    return;
                }
                Event::Click { .. } if node == cancel || node == overlay => {
                    self.close_prompt();
                    return;
                }
                _ if node == field => return,
                _ => {}
            }
        }
        // An open menu takes the click first.
        if let Some(popup) = &self.popup {
            if let Event::Click { .. } = event {
                if let Some((_, action)) = popup.items.iter().find(|(n, _)| *n == node) {
                    requests.action = Some(action.clone());
                    self.close_popup();
                    return;
                }
                if node == popup.overlay {
                    self.close_popup();
                    return;
                }
            }
        }
        match self.docks.event(&mut self.ui, node, event) {
            Some(Docked::Handled) => {
                self.sync_visible();
                if !matches!(event, Event::Drag { .. }) {
                    requests.refresh = true;
                }
                return;
            }
            Some(Docked::Maximize(panel)) => {
                self.toggle_zoom(Zoom::Stack(panel));
                self.sync_visible();
                return;
            }
            Some(Docked::Menu(panel, x, y)) => {
                requests.menu = Some((self.panel_menu(panel), x, y));
                return;
            }
            Some(Docked::Lock(panel)) => {
                if panel == Panel::Inspector {
                    self.inspector.toggle_lock();
                    self.sync_lock();
                    requests.refresh = true;
                }
                return;
            }
            None => {}
        }
        if let Some(panel) = self
            .floats
            .iter()
            .find(|f| f.dock_button == node)
            .map(|f| f.panel)
        {
            if let Event::Click { .. } = event {
                self.dock_back(panel);
                requests.refresh = true;
            }
            return;
        }
        if let Some(i) = self.splits.iter().position(|s| *s == node) {
            if let Event::Drag { dx, dy, .. } = event {
                self.drag_split(i, *dx, *dy);
            }
            return;
        }
        if node == self.viewport {
            return;
        }
        if self.toolbar_event(node, event, requests) {
            return;
        }
        if self.hierarchy.owns(&self.ui, node) {
            // A line let go on an Inspector field that links an entity:
            // linked there, and not moved in the tree.
            if let (Event::DragEnd { .. }, Some(id)) = (event, self.hierarchy.line_entity(node)) {
                if self.inspector.drop_on(
                    &mut self.ui,
                    &mut self.session,
                    &crate::inspector::Dragged::Entity(id),
                ) {
                    requests.refresh = true;
                    return;
                }
            }
            self.hierarchy
                .event(&mut self.ui, &mut self.session, node, event, requests);
        } else if self.inspector.owns(node) {
            self.inspector
                .event(&mut self.ui, &mut self.session, node, event, requests);
        } else if self.preferences.owns(&self.ui, node) {
            if let Some(asked) = self
                .preferences
                .event(&mut self.ui, node, event, &self.keymap)
            {
                self.preference(asked);
            }
        } else if self.settings.owns(&self.ui, node) {
            self.settings
                .event(&mut self.ui, &mut self.session, node, event);
        } else if self.configs.owns(&self.ui, node) {
            self.configs.event(&mut self.ui, &self.session, node, event);
        } else if self.animator.owns(&self.ui, node) {
            self.animator
                .event(&mut self.ui, &mut self.session, node, event);
        } else if self.dialogues.owns(&self.ui, node) {
            self.dialogues
                .event(&mut self.ui, &mut self.session, node, event);
        } else if self.screens.owns(&self.ui, node) {
            self.screens
                .event(&mut self.ui, &mut self.session, node, event);
        } else if self.animation.owns(&self.ui, node) {
            self.animation
                .event(&mut self.ui, &mut self.session, node, event);
        } else if self.profiler.owns(&self.ui, node) {
        } else if self.bottom.owns(&self.ui, node) {
            self.bottom
                .event(&mut self.ui, &mut self.session, node, event, requests);
        }
    }

    fn toolbar_event(&mut self, node: NodeId, event: &Event, requests: &mut Requests) -> bool {
        let Event::Click { .. } = event else {
            return false;
        };
        if let Some((_, side)) = self.compass.axes.iter().find(|(n, _)| *n == node) {
            requests.action = Some(Action::View(*side));
            requests.keyboard_to_scene = true;
            return true;
        }
        if node == self.compass.middle {
            requests.action = Some(Action::ToggleOrtho);
            requests.keyboard_to_scene = true;
            return true;
        }
        if node == self.status.problems || node == self.status.console {
            self.docks.activate(&mut self.ui, Panel::Console);
            self.sync_visible();
            requests.refresh = true;
            return true;
        }
        if node == self.prefab_back {
            requests.action = Some(Action::ExitPrefab);
            return true;
        }
        if let Some((_, action)) = self.buttons.iter().find(|(n, _)| *n == node) {
            requests.action = Some(action.clone());
            requests.keyboard_to_scene = true;
            return true;
        }
        if let Some(i) = self.tools.iter().position(|n| *n == node) {
            requests.action = Some(match TOOLS[i].2 {
                None => Action::Hand,
                Some(tool) => Action::Tool(tool),
            });
            requests.keyboard_to_scene = true;
            return true;
        }
        // The bar's words: each says what is so, a click the other.
        let word = if node == self.pivot_button {
            Some(Action::TogglePivot)
        } else if node == self.space_button {
            Some(Action::ToggleSpace)
        } else if node == self.grid_button {
            Some(Action::ToggleGrid)
        } else {
            None
        };
        if let Some(action) = word {
            requests.action = Some(action);
            requests.keyboard_to_scene = true;
            return true;
        }
        if node == self.aspect_button {
            let r = self.ui.rect(node);
            let items = [
                ("Free Aspect", None),
                ("16:9", Some((16, 9))),
                ("16:10", Some((16, 10))),
                ("4:3", Some((4, 3))),
                ("1:1", Some((1, 1))),
                ("9:16 (portrait)", Some((9, 16))),
            ]
            .into_iter()
            .map(|(l, a)| MenuItem::new(l, Action::Aspect(a)))
            .collect();
            requests.menu = Some((items, r.x, r.y + r.height + 4.0));
            return true;
        }
        if node == self.tab_scene || node == self.tab_game {
            if matches!(event, Event::Click { count: 2, .. }) {
                self.toggle_zoom(Zoom::View);
                return true;
            }
            requests.action = Some(Action::GameView(node == self.tab_game));
            requests.keyboard_to_scene = true;
            return true;
        }
        if node == self.toolbar.layout {
            let r = self.ui.rect(node);
            requests.menu = Some((self.layout_menu(), r.x + r.width - 236.0, r.y + r.height + 4.0));
            return true;
        }
        let t = &self.toolbar;
        if let Some((_, items)) = t.menus.iter().find(|(n, _)| *n == node) {
            let r = self.ui.rect(node);
            // The labels as they stand (Undo says what it would undo).
            let mut items = items.clone();
            for item in &mut items {
                if let Some(action) = &item.action {
                    item.label = self.menu_state(action, &item.label).label;
                }
            }
            // The Window menu lists the layouts this person saved after
            // the built-in ones.
            if let Some(at) = items
                .iter()
                .position(|i| i.action == Some(Action::Layout("Tall".into())))
            {
                let saved = self.config_dir.as_deref().map(layouts::saved).unwrap_or_default();
                for (k, name) in saved.into_iter().enumerate() {
                    items.insert(
                        at + 1 + k,
                        MenuItem::new(&format!("Layout: {name}"), Action::Layout(name)),
                    );
                }
            }
            requests.menu = Some((items, r.x, r.y + r.height + 4.0));
            return true;
        }
        let action = if node == t.play {
            Action::Play
        } else if node == t.pause {
            Action::Pause
        } else if node == t.step {
            Action::Step
        } else if node == t.undo {
            Action::Editor("undo")
        } else if node == t.redo {
            Action::Editor("redo")
        } else if node == t.save {
            Action::Editor("save_scene")
        } else {
            return false;
        };
        requests.action = Some(action);
        requests.keyboard_to_scene = true;
        true
    }

    fn apply(&mut self, requests: Requests) {
        if let Some(action) = requests.action {
            self.run(action);
        }

        if let Some(asset) = requests.inspect {
            if self.foliage {
                if let Asset::Model(name, _) | Asset::Prefab(name) = &asset {
                    self.foliage_what = Some(name.clone());
                    self.session
                        .say(Level::Info, format!("foliage brush: painting {name}"));
                }
            }
            if let Some(pixels) = self
                .inspector
                .show_asset(&mut self.ui, &mut self.session, asset)
            {
                self.pending_images
                    .push((crate::inspector::PREVIEW, 256, pixels));
            }
        }
        if let Some(asset) = requests.dropped {
            self.drop_asset(asset);
        }
        if let Some(asset) = requests.ping {
            self.bottom.ping(&mut self.ui, &self.session, asset);
            self.refresh();
        }
        if let Some((items, x, y)) = requests.menu {
            self.open_popup(items, x, y);
        }
        if requests.keyboard_to_scene {
            // Clicking a line of the Hierarchy or a toolbar button leaves
            // the keyboard where F, Delete and Ctrl Z act on the selection.
            if !self.ui.focused().is_some_and(|f| self.ui.is_field(f)) {
                self.ui.focus(Some(self.viewport));
            }
        }
        if requests.refresh {
            self.refresh();
        }
        if let Some(field) = requests.focus_named {
            if let Some(slot) = self.inspector.slot_of(&field) {
                self.ui.focus(Some(slot));
            }
        }
    }

    /// A Project entry let go somewhere: in the Scene view, placed where
    /// it landed; a material, onto what it landed on.
    fn drop_asset(&mut self, asset: Asset) {
        // Onto an Inspector field that names an asset: set there if it fits.
        if self.inspector.drop_on(
            &mut self.ui,
            &mut self.session,
            &crate::inspector::Dragged::Asset(asset.clone()),
        ) {
            self.refresh();
            return;
        }
        let (px, py) = self.ui.pointer();
        // Onto the Hierarchy: under the line it lands on, or at the top.
        if let Some(parent) = self.hierarchy.drop_target(&self.ui) {
            let s = &mut self.session;
            let made = match &asset {
                Asset::Prefab(name) => s.add_instance(parent, name).map(Some),
                Asset::Model(name, _) => s.add(parent, name).map(Some),
                _ => Ok(None),
            };
            match made {
                Ok(Some(id)) => {
                    // Named after what it is, as a drop into the view names it.
                    let name = match &asset {
                        Asset::Prefab(n) | Asset::Model(n, _) => {
                            n.rsplit(':').next().unwrap_or(n).to_string()
                        }
                        _ => String::new(),
                    };
                    if !name.is_empty() {
                        let _ = s.rename(id, &name);
                        s.squash_last(2);
                    }
                    let _ = s.select(Some(id));
                    if let Some(p) = parent {
                        s.set_open(p, true);
                    }
                }
                Ok(None) => {}
                Err(e) => s.say(Level::Error, e.to_string()),
            }
            self.refresh();
            return;
        }
        let view = self.ui.rect(self.viewport);
        if !view.contains(px, py) {
            return;
        }
        let scale = self.ui.viewport().2;
        let (x, y) = (
            ((px - view.x) * scale) as u32,
            ((py - view.y) * scale) as u32,
        );
        let s = &mut self.session;
        let result = match asset {
            Asset::Model(name, _) | Asset::Prefab(name) => s.drop_asset(&name, x, y).map(|_| ()),
            Asset::Sound(..) => Ok(()),
            Asset::Material(name) => match s.pick(x, y) {
                Some(id) => s.set_material_name(id, &name),
                None => Ok(()),
            },
            Asset::Scene(path) => {
                self.run(Action::OpenScene(path));
                Ok(())
            }
        };
        if let Err(e) = result {
            self.session.say(Level::Error, e.to_string());
        }
        self.ui.focus(Some(self.viewport));
        self.refresh();
    }

    /// Do what a menu entry, a button or a context menu asks.
    pub fn run(&mut self, action: Action) {
        if let Err(message) = self.run_inner(action) {
            self.session.say(Level::Error, message);
        }
        self.refresh();
    }

    fn run_inner(&mut self, action: Action) -> Result<(), String> {
        let s = &mut self.session;
        let e = |e: scrap_editor::EditError| e.to_string();
        {
            match action {
                Action::OpenScene(path) => {
                    if s.is_modified() && self.discard_asked.as_ref() != Some(&path) {
                        self.discard_asked = Some(path);
                        return Err("the open scene has unsaved changes: save first, or open the other one again to discard them".into());
                    }
                    self.discard_asked = None;
                    let missing = s.open_scene(&path).map_err(e)?;
                    s.say(Level::Info, format!("opened {}", path.display()));
                    for m in missing {
                        s.say(Level::Warning, m);
                    }
                }
                Action::NewScene => {
                    self.ask("New scene", "", Ask::NewScene);
                }
                Action::AssetRename(file) => {
                    let stem = std::path::Path::new(&file)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    self.ask(&format!("Rename {file}"), &stem, Ask::RenameAsset(file));
                }
                Action::AssetDuplicate(file) => {
                    let stem = std::path::Path::new(&file)
                        .file_stem()
                        .map(|s| format!("{}_copy", s.to_string_lossy()))
                        .unwrap_or_default();
                    self.ask(&format!("Copy {file} as"), &stem, Ask::DuplicateAsset(file));
                }
                Action::AssetDelete(file) => {
                    s.delete_asset(&file).map_err(e)?;
                    s.say(Level::Info, format!("deleted {file}"));
                }
                Action::AssetReveal(file) => {
                    let path = s
                        .project()
                        .map(|p| p.root().join(&file))
                        .ok_or("no project")?;
                    reveal(&path)
                        .map_err(|err| format!("cannot show {}: {err}", path.display()))?;
                }
                Action::NewComponent => {
                    self.ask("New component (snake_case)", "", Ask::NewComponent)
                }
                Action::NewSystem => self.ask("New system (snake_case)", "", Ask::NewSystem),
                Action::SaveMaterial => {
                    if s.selected().is_none() {
                        return Err("select something to take the colour of".into());
                    }
                    self.ask("Save material as", "", Ask::SaveMaterial);
                }
                Action::MakeVariant => {
                    let id = s.selected().ok_or("select a prefab instance")?;
                    let base = s
                        .entity_prefab(id)
                        .ok_or("the selection is not a prefab instance")?;
                    self.ask(
                        "Prefab variant name",
                        &format!("{base}_variant"),
                        Ask::Variant,
                    );
                }
                Action::SnapSettings => {
                    let snap = s.snap();
                    let now = if snap.meters > 0.0 {
                        snap
                    } else {
                        scrap_editor::Snap::INCREMENT
                    };
                    self.ask(
                        "Snap: metres, degrees, scale",
                        &format!("{}, {}, {}", now.meters, now.degrees, now.scale),
                        Ask::Snap,
                    );
                }
                Action::ToggleNavigation => {
                    self.navigation = !self.navigation;
                    s.set_show_navigation(
                        self.navigation
                            .then(scrap::navigation::NavSettings::default),
                    );
                }
                Action::Search => self.open_search(),
                Action::PlaySound(name) => {
                    let sound = s
                        .sound(&name)
                        .ok_or_else(|| format!("no sound called {name} in the library"))?;
                    if self.audio.is_none() {
                        self.audio = Some(
                            scrap::audio::Audio::new()
                                .map_err(|e| format!("no audio device: {e}"))?,
                        );
                    }
                    let audio = self.audio.as_mut().expect("made above");
                    if let Some(mut old) = self.listening.take() {
                        old.stop();
                    }
                    self.listening = Some(audio.play(sound, 1.0)?);
                }
                Action::StopSound => {
                    if let Some(mut old) = self.listening.take() {
                        old.stop();
                    }
                }
                Action::FieldReset(field) => {
                    for id in self.inspector.targets(s) {
                        s.reset_field(id, &field).map_err(e)?;
                    }
                }
                Action::FieldCopy(field) => {
                    let id = *self
                        .inspector
                        .targets(s)
                        .first()
                        .ok_or("nothing selected")?;
                    let value = s
                        .inspect(id)
                        .and_then(|f| f.into_iter().find(|f| f.name == field))
                        .map(|f| f.value)
                        .ok_or("no such field")?;
                    self.clipboard.set(value);
                }
                Action::FieldPaste(field) => {
                    let value = self.clipboard.get().unwrap_or_default();
                    let ids = self.inspector.targets(s);
                    s.set_field_all(&ids, &field, value.trim()).map_err(e)?;
                }
                Action::FieldRemove(field) => {
                    for id in self.inspector.targets(s) {
                        match field.strip_prefix("components.") {
                            Some(name) => s.set_component(id, name, None).map_err(e)?,
                            None => s.reset_field(id, &field).map_err(e)?,
                        }
                    }
                }
                Action::Aspect(aspect) => {
                    self.aspect = aspect;
                    s.set_game_view(true);
                }
                Action::AddComponent(name) => {
                    // Described by the game: its shape's example. Written
                    // but not built yet: `()`, until the game says more.
                    let described = s.component_shapes().contains_key(&name);
                    for id in s.selection() {
                        if described {
                            s.add_component(id, &name).map_err(e)?;
                        } else {
                            s.set_component(id, &name, Some("()")).map_err(e)?;
                        }
                    }
                    if !described {
                        s.say(
                            Level::Info,
                            format!("`{name}` is on as (): its fields show here once the game is built and has described it"),
                        );
                    }
                }
                Action::OpenSceneDialog => {
                    let mut dialog = rfd::FileDialog::new().add_filter("scene", &["ron"]);
                    if let Some(dir) = s.project().map(|p| p.root().join("scenes")) {
                        dialog = dialog.set_directory(dir);
                    }
                    if let Some(path) = dialog.pick_file() {
                        return self.run_inner(Action::OpenScene(path));
                    }
                }
                Action::SaveAs => {
                    let mut dialog = rfd::FileDialog::new().add_filter("scene", &["ron"]);
                    if let Some(dir) = s.project().map(|p| p.root().join("scenes")) {
                        dialog = dialog.set_directory(dir);
                    }
                    if let Some(path) = dialog.save_file() {
                        s.save_scene(Some(&path)).map_err(e)?;
                        let missing = s.open_scene(&path).map_err(e)?;
                        for m in missing {
                            s.say(Level::Warning, m);
                        }
                        s.say(Level::Info, format!("saved as {}", path.display()));
                    }
                }
                Action::ReloadScene => {
                    if s.revert_to_disk().map_err(e)? {
                        s.say(Level::Info, "reloaded from disk (undo takes it back)");
                    }
                }
                Action::ShowInProject(path) => {
                    // Its folder open, its tile outlined and scrolled to.
                    self.docks.activate(&mut self.ui, Panel::Project);
                    self.sync_visible();
                    self.bottom
                        .ping(&mut self.ui, &self.session, crate::bottom::Asset::Scene(path));
                    self.refresh();
                }
                Action::Import => {
                    if let Some(paths) = rfd::FileDialog::new()
                        .add_filter(
                            "model, texture, sound",
                            &[
                                "gltf", "glb", "obj", "png", "jpg", "jpeg", "wav", "ogg", "mp3",
                                "flac", "scrterrain", "scrpoly",
                            ],
                        )
                        .pick_files()
                    {
                        for path in paths {
                            match s.import(&path) {
                                Ok(notes) => {
                                    s.say(Level::Info, format!("imported {}", path.display()));
                                    for n in notes {
                                        s.say(Level::Warning, n);
                                    }
                                }
                                Err(err) => s.say(Level::Error, err.to_string()),
                            }
                        }
                    }
                }
                Action::OpenPrefab(name) => {
                    if !s.is_prefab() {
                        self.scene_before_prefab = s.scene_path().map(Path::to_path_buf);
                    }
                    if s.is_modified() {
                        s.save_scene(None).map_err(e)?;
                    }
                    let missing = s.open_prefab(&name).map_err(e)?;
                    for m in missing {
                        s.say(Level::Warning, m);
                    }
                    s.say(
                        Level::Info,
                        format!("editing prefab {name}; Back returns to the scene"),
                    );
                }
                Action::ExitPrefab => {
                    if s.is_prefab() && s.is_modified() {
                        s.save_scene(None).map_err(e)?;
                        s.say(Level::Info, "saved the prefab: every instance has it");
                    }
                    if let Some(scene) = self.scene_before_prefab.take() {
                        let missing = s.open_scene(&scene).map_err(e)?;
                        for m in missing {
                            s.say(Level::Warning, m);
                        }
                    }
                }
                // The registry's actions: what the agent's tools run too.
                Action::Editor(name) => {
                    let said = scrap_editor::actions::run(s, name, &Default::default())?;
                    if name == "save_scene" {
                        s.say(Level::Info, said);
                    }
                }
                Action::CheckProject => {
                    let project = s.project().ok_or("no project is open")?;
                    let findings = scrap_cli::check(project);
                    if findings.is_empty() {
                        s.say(Level::Info, "check: nothing wrong");
                    }
                    for f in findings {
                        let level = match f.severity {
                            scrap_cli::Severity::Error => Level::Error,
                            scrap_cli::Severity::Warning => Level::Warning,
                        };
                        s.say(level, f.to_string());
                    }
                }
                Action::Build(run) => {
                    if self.job.is_some() {
                        return Err("a build is already running".into());
                    }
                    let project = s.project().ok_or("no project is open")?.clone();
                    let out = project.root().join("build");
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let result = scrap_cli::build::build(&project, &out, true)
                            .map_err(|e| format!("{e:#}"))
                            .and_then(|built| {
                                if !built.stale.is_empty() {
                                    // Built anyway, with the last good versions.
                                    eprintln!("stale: {}", built.stale.join("; "));
                                }
                                if run {
                                    std::process::Command::new(&built.executable)
                                        .current_dir(&built.folder)
                                        .spawn()
                                        .map_err(|e| format!("built, but it did not start: {e}"))?;
                                }
                                Ok(format!("built {}", built.executable.display()))
                            });
                        let _ = tx.send(result);
                    });
                    self.job = Some(rx);
                    s.say(Level::Info, "building (release)…");
                }
                Action::ReloadAssets => {
                    let n = s.reload_assets();
                    s.say(Level::Info, format!("reloaded {n} assets"));
                }
                Action::StartGame => s.start_game().map_err(e)?,
                Action::StopGame => {
                    s.stop_game();
                }
                Action::Copy => {
                    let text = s.copy_selection();
                    self.clipboard.set(text);
                }
                Action::Paste => {
                    let text = self.clipboard.get().unwrap_or_default();
                    s.paste(&text, None).map_err(e)?;
                }
                Action::SelectAll => {
                    s.select_everything().map_err(e)?;
                }
                Action::SelectNone => s.select(None).map_err(e)?,
                Action::Rename => {
                    self.hierarchy.rename_selected(&mut self.ui, s);
                }
                Action::Frame => {
                    s.focus_selected();
                }
                Action::SnapToGrid => {
                    s.snap_selection().map_err(e)?;
                }
                Action::CreateEmpty => {
                    s.create_empty("Empty").map_err(e)?;
                }
                Action::CreateChild => {
                    let parent = s.selected().ok_or("select what the child goes under")?;
                    let child = s.create_empty("Child").map_err(e)?;
                    s.reparent(child, Some(parent)).map_err(e)?;
                    s.squash_last(2);
                    s.set_open(parent, true);
                    s.select(Some(child)).map_err(e)?;
                }
                Action::Group => {
                    s.group_selection("Group").map_err(e)?;
                }
                Action::Create(model) => {
                    let parent = None;
                    let id = s.add(parent, model).map_err(e)?;
                    place_in_front(s, id);
                    s.select(Some(id)).map_err(e)?;
                }
                Action::CreateLight | Action::CreateCamera => {
                    let (name, field, value) = if action == Action::CreateLight {
                        (
                            "Light",
                            "light",
                            "(color: (1.0, 0.9, 0.8), intensity: 3.0, range: 8.0)",
                        )
                    } else {
                        ("Camera", "camera", "()")
                    };
                    let id = s.create_empty(name).map_err(e)?;
                    s.set_field(id, field, value).map_err(e)?;
                    s.select(Some(id)).map_err(e)?;
                }
                Action::MakePrefab => {
                    let id = s.selected().ok_or("select something to make a prefab of")?;
                    let name = s.entity_name(id).unwrap_or_else(|| "prefab".into());
                    let name: String = name
                        .chars()
                        .map(|c| {
                            if c.is_alphanumeric() {
                                c.to_ascii_lowercase()
                            } else {
                                '-'
                            }
                        })
                        .collect();
                    s.make_prefab(id, &name).map_err(e)?;
                    s.say(Level::Info, format!("made prefab {name}"));
                }
                Action::ApplyOverrides => {
                    let id = s.selected().ok_or("select an instance")?;
                    s.apply_overrides(id).map_err(e)?;
                }
                Action::RevertOverrides => {
                    let id = s.selected().ok_or("select an instance")?;
                    s.revert_overrides(id).map_err(e)?;
                }
                Action::Unpack => {
                    let id = s.selected().ok_or("select an instance")?;
                    s.unpack_prefab(id).map_err(e)?;
                }
                Action::Hide => {
                    s.toggle_hidden().map_err(e)?;
                }
                Action::ShowAll => s.show_all(),
                Action::View(side) => s.look_from(side),
                Action::Perspective => s.set_orthographic(false),
                Action::ToggleOrtho => {
                    let ortho = s.is_orthographic();
                    s.set_orthographic(!ortho);
                }
                Action::ToggleGrid => {
                    let on = s.show_grid();
                    s.set_show_grid(!on);
                }
                Action::ToggleSnap => {
                    let on = s.snap().meters > 0.0;
                    s.set_snap(if on {
                        scrap_editor::Snap {
                            meters: 0.0,
                            degrees: 0.0,
                            scale: 0.0,
                        }
                    } else {
                        scrap_editor::Snap::INCREMENT
                    });
                }
                Action::ToggleColliders => {
                    self.colliders = !self.colliders;
                    s.set_show_colliders(self.colliders);
                }
                Action::Tool(tool) => s.set_tool(tool),
                Action::Hand => s.set_hand(),

                Action::ToggleSpace => {
                    let next = if s.space() == Space::Local {
                        Space::Global
                    } else {
                        Space::Local
                    };
                    s.set_space(next);
                }
                Action::TogglePivot => {
                    let next = if s.pivot() == Pivot::Center {
                        Pivot::Pivot
                    } else {
                        Pivot::Center
                    };
                    s.set_pivot(next);
                }
                Action::Play => {
                    // Play is the game: its own code, input and camera, its
                    // own process, drawing in the Game view. Pressed while
                    // something plays — the game, or the physics simulated
                    // here — it stops that.
                    if s.is_playing() {
                        s.stop();
                        s.set_game_view(false);
                    } else if s.is_game_running() {
                        s.stop_game();
                        s.set_game_view(false);
                        s.say(Level::Info, "stopped the game");
                    } else {
                        if self.bottom.clear_on_play {
                            s.clear_console();
                        }
                        s.start_game().map_err(|err| {
                            format!("{err} — Play → Simulate Physics Here runs the scene's physics in the view instead")
                        })?;
                        // It builds, then draws here; the Game view shows
                        // the scene through its camera until it does.
                        s.set_game_view(true);
                        self.ui.focus(Some(self.viewport));
                    }
                }
                Action::Simulate => {
                    // The simulation looks through the game's eyes, as
                    // Unity's Play brings up the Game view; stop goes back.
                    if s.is_playing() {
                        s.stop();
                        s.set_game_view(false);
                    } else {
                        if self.bottom.clear_on_play {
                            s.clear_console();
                        }
                        s.play();
                        s.set_game_view(true);
                    }
                }
                Action::GameView(game) => s.set_game_view(game),
                Action::Pause => {
                    if !s.is_playing() {
                        if self.bottom.clear_on_play {
                            s.clear_console();
                        }
                        s.play();
                    }
                    let now = s.is_paused();
                    s.pause(!now);
                }
                Action::Step => {
                    if !s.is_playing() {
                        if self.bottom.clear_on_play {
                            s.clear_console();
                        }
                        s.play();
                    }
                    s.step_once();
                }
                Action::SetSub(component, key, value) => {
                    self.inspector.pick_sub(s, &component, &key, &value);
                }
                Action::SetLeaf(place, value) => {
                    self.inspector.set_leaf(s, &place, &value);
                }
                Action::SetField(field, value) => {
                    self.inspector.set_field(s, &field, &value);
                }
                Action::Place(name) => {
                    let (w, h) = s.size();
                    s.drop_asset(&name, w / 2, h / 2).map_err(e)?;
                }
                Action::ClearConsole => s.clear_console(),
                Action::TogglePanel(i) => {
                    self.maximized = None;
                    self.panels[i] = !self.panels[i];
                    self.show_panels();
                }
                Action::Maximize => self.toggle_zoom(Zoom::View),
                Action::NewTerrain => {
                    let mut name = "terrain".to_string();
                    let mut n = 1;
                    while s.has_model(&name) {
                        n += 1;
                        name = format!("terrain_{n}");
                    }
                    s.new_terrain(&name, 40.0).map_err(e)?;
                    self.sculpt = true;
                    s.say(
                        Level::Info,
                        "sculpt with the left button: Shift lowers, Ctrl/Cmd flattens",
                    );
                }
                Action::Float(panel) => self.float(panel),
                Action::Layout(name) => {
                    let layout = layouts::load(self.config_dir.as_deref(), &name)?;
                    self.apply_layout(layout);
                }
                Action::SaveLayoutAs => {
                    let initial = self
                        .layout_name
                        .clone()
                        .filter(|n| Layout::built_in(n).is_none())
                        .unwrap_or_else(|| "My Layout".into());
                    self.ask("Save layout as", &initial, Ask::SaveLayout);
                }
                Action::DeleteLayout => {
                    let saved = self.config_dir.as_deref().map(layouts::saved).unwrap_or_default();
                    let initial = self
                        .layout_name
                        .clone()
                        .filter(|n| saved.contains(n))
                        .or_else(|| saved.first().cloned())
                        .ok_or("no saved layouts to delete: built-in ones stay")?;
                    self.ask("Delete layout", &initial, Ask::DeleteLayout);
                }
                Action::ShowPanel(panel) => self.show_panel(panel),
                Action::MaximizePanel(panel) => {
                    self.docks.activate(&mut self.ui, panel);
                    self.toggle_zoom(Zoom::Stack(panel));
                    self.sync_visible();
                }
                Action::CloseTab(panel) => {
                    self.docks.close(&mut self.ui, panel);
                    self.sync_visible();
                }
                Action::InspectorDebug(debug) => {
                    self.inspector.set_debug(&mut self.ui, &mut self.session, debug);
                }
                Action::ToggleClearOnPlay => {
                    self.bottom.clear_on_play = !self.bottom.clear_on_play;
                }
                Action::Theme(name) => {
                    self.change_theme(crate::appearance::Change::Preset(name.into()))
                }
                Action::Preferences(page) => self.open_preferences(page),
                Action::ProjectSettings => self.show_panel(Panel::Settings),
                Action::MoveToView => {
                    if !s.move_to_view().map_err(e)? {
                        return Err("select something to move to the view".into());
                    }
                }
                Action::AlignWithView => {
                    if !s.align_with_view().map_err(e)? {
                        return Err("select something to align with the view".into());
                    }
                }
                Action::MaterialInstance(parent) => {
                    let name = s.new_material_instance(&parent).map_err(e)?;
                    s.say(
                        Level::Info,
                        format!("{name}: {parent} until something on it says otherwise"),
                    );
                    self.show_next = Some(Asset::Material(name));
                }
                Action::InstallBlenderPlugin => {
                    let blender = scrap_import::blend::blender()
                        .ok_or("Blender was not found: install it, or set SCRAP_BLENDER to it")?;
                    if scrap_import::blend::blender_open() {
                        return Err(
                            "Blender is open: quit it and install again. Blender saves its preferences when it quits, over the ones that turn the plugin on".into(),
                        );
                    }
                    let folder =
                        scrap_import::blend::install(&blender).map_err(|e| format!("{e:#}"))?;
                    s.say(
                        Level::Info,
                        format!(
                            "the scrap plugin is in Blender and on ({}): open Blender, the scrap tab is in the 3D view's sidebar (N)",
                            folder.display()
                        ),
                    );
                }
                Action::ToggleFoliage => {
                    self.foliage = !self.foliage;
                    self.foliage_stroke = None;
                    if self.foliage {
                        self.sculpt = false;
                        self.faces = false;
                        // What to paint: the model or prefab looked at in
                        // the Inspector, or the selection's.
                        if self.foliage_what.is_none() {
                            self.foliage_what = self
                                .inspector
                                .asset_name()
                                .or_else(|| s.selected().and_then(|id| s.entity_model(id)))
                                .or_else(|| s.selected().and_then(|id| s.entity_prefab(id)));
                        }
                        let what = self.foliage_what.clone();
                        s.say(
                            Level::Info,
                            match what {
                                Some(w) => format!("foliage brush: painting {w}; Shift erases, [ and ] change the size; click another model in Project to paint it"),
                                None => "foliage brush: click a model or prefab in Project to paint it".into(),
                            },
                        );
                    }
                }
                Action::KeepSimulation => {
                    if !s.is_playing() {
                        return Err("Keep Simulation Changes works while playing".into());
                    }
                    let n = s.keep_simulation();
                    s.say(
                        Level::Info,
                        format!("{n} kept where the simulation puts them when play stops"),
                    );
                }
                Action::DockAll => {
                    for panel in self.floats.iter().map(|f| f.panel).collect::<Vec<_>>() {
                        self.dock_back(panel);
                    }
                }
                Action::ToggleFaces => {
                    self.faces = !self.faces;
                    self.face_hover = None;
                    self.face_drag = None;
                    if self.faces {
                        self.sculpt = false;
                        s.say(
                            Level::Info,
                            "face mode: point at a face to outline it, drag it to push it out or in",
                        );
                    }
                    self.show_face();
                }
                Action::ToggleSculpt => {
                    self.sculpt = !self.sculpt;
                    if self.sculpt {
                        self.faces = false;
                        s.say(
                            Level::Info,
                            "sculpt with the left button: Shift lowers, Ctrl/Cmd flattens",
                        );
                    }
                }
            }
            Ok(())
        }
    }

    // --- menus -----------------------------------------------------------

    /// Quick Search: a field over the window, results as it is typed.
    fn open_search(&mut self) {
        self.close_search();
        self.close_popup();
        let root = self.ui.root();
        let (w, h, _) = self.ui.viewport();
        let overlay = self.ui.add(
            root,
            Style::column()
                .absolute(0.0, 0.0)
                .size(w, h)
                .padding(80.0)
                .center_items()
                .background(NEUTRAL_900.alpha(40))
                .clickable(),
        );
        self.ui.set_layer(overlay, true);
        self.ui.set_name(overlay, "search overlay");
        let panel = self.ui.add(
            overlay,
            Style::column()
                .width(560.0)
                .padding(SPACE_3)
                .gap(SPACE_2)
                .radius(RADIUS_LG)
                .background(SURFACE)
                .border(1.0, NEUTRAL_500)
                .clickable(),
        );
        let field = self.ui.add_field(
            panel,
            field_style().full_width().height(32.0).text_size(14.0),
            "",
        );
        self.ui.set_name(field, "search field");
        self.ui
            .set_placeholder(field, "Search the scene, the project and the menus");
        let list = self.ui.add(panel, Style::column().full_width().gap(2.0));
        self.ui.set_name(list, "search results");
        self.ui.focus(Some(field));
        self.search = Some(Search {
            overlay,
            field,
            list,
            hits: Vec::new(),
        });
        self.fill_search("");
    }

    fn close_search(&mut self) {
        if let Some(q) = self.search.take() {
            self.ui.remove(q.overlay);
            self.ui.focus(Some(self.viewport));
        }
    }

    /// The results for what is typed: entities (by the Hierarchy's search,
    /// `c:` and `m:` and all), then assets, then menu entries, a dozen at most.
    fn fill_search(&mut self, typed: &str) {
        let Some(q) = &self.search else { return };
        let list = q.list;
        let query = typed.trim().to_lowercase();
        let mut hits: Vec<(String, &'static str, Hit)> = Vec::new();
        if !query.is_empty() {
            let ids = self.session.search(typed.trim()).unwrap_or_default();
            for id in ids.into_iter().take(6) {
                let name = self.session.entity_name(id).unwrap_or_default();
                hits.push((name, "box", Hit::Entity(id)));
            }
            for asset in crate::bottom::all_assets(&self.session) {
                if asset.label().to_lowercase().contains(&query) {
                    hits.push((asset.label(), asset.icon(), Hit::Asset(asset)));
                }
                if hits.len() >= 9 {
                    break;
                }
            }
            for (_, items) in menu::menu_bar() {
                for item in items {
                    if let Some(action) = item.action {
                        if item.label.to_lowercase().contains(&query) {
                            hits.push((item.label.clone(), "chevron-right", Hit::Action(action)));
                        }
                    }
                }
            }
        }
        hits.truncate(12);
        self.ui.clear(list);
        let mut nodes = Vec::new();
        for (i, (label, glyph, hit)) in hits.into_iter().enumerate() {
            let row = self.ui.add(
                list,
                Style::row()
                    .full_width()
                    .height(28.0)
                    .fixed()
                    .padding_x(SPACE_3)
                    .gap(SPACE_2)
                    .center_items()
                    .radius(RADIUS_SM)
                    .hover(ACCENT.alpha(16))
                    .background(if i == 0 {
                        ACCENT_900
                    } else {
                        scrap_ui::Color::TRANSPARENT
                    }),
            );
            self.ui.set_name(row, format!("result {label}"));
            icon(
                &mut self.ui,
                row,
                glyph,
                if i == 0 { ACCENT } else { MUTED },
            );
            self.ui.add_text(row, text().fill(), &label);
            let kind = match &hit {
                Hit::Entity(_) => "in the scene",
                Hit::Asset(_) => "in the project",
                Hit::Action(_) => "menu",
            };
            self.ui.add_text(
                row,
                Style::default().text_size(11.0).text_color(MUTED).nowrap(),
                kind,
            );
            nodes.push((row, hit));
        }
        if let Some(q) = &mut self.search {
            q.hits = nodes;
        }
    }

    /// Go where a result leads.
    fn go_to(&mut self, hit: Hit, requests: &mut Requests) {
        match hit {
            Hit::Entity(id) => {
                let _ = self.session.select(Some(id));
                self.session.focus_selected();
                requests.refresh = true;
            }
            Hit::Asset(Asset::Scene(path)) => requests.action = Some(Action::OpenScene(path)),
            Hit::Asset(Asset::Prefab(name)) => requests.action = Some(Action::OpenPrefab(name)),
            Hit::Asset(asset) => requests.inspect = Some(asset),
            Hit::Action(action) => requests.action = Some(action),
        }
    }

    /// Ask for a line of text in a dialog; what it is for says what the
    /// answer does.
    fn ask(&mut self, title: &str, initial: &str, ask: Ask) {
        self.close_prompt();
        self.close_popup();
        let root = self.ui.root();
        let (w, h, _) = self.ui.viewport();
        let overlay = self.ui.add(
            root,
            Style::column()
                .absolute(0.0, 0.0)
                .size(w, h)
                .center()
                .background(NEUTRAL_900.alpha(50))
                .clickable(),
        );
        self.ui.set_layer(overlay, true);
        self.ui.set_name(overlay, "dialog overlay");
        let dialog = self.ui.add(
            overlay,
            Style::column()
                .width(420.0)
                .padding(SPACE_4)
                .gap(SPACE_3)
                .radius(RADIUS_LG)
                .background(SURFACE)
                .border(1.0, NEUTRAL_500)
                .clickable(),
        );
        self.ui.set_name(dialog, "dialog");
        self.ui.add_text(
            dialog,
            Style::default()
                .text_size(16.0)
                .weight(500)
                .text_color(TEXT)
                .nowrap(),
            title,
        );
        let field = self.ui.add_field(
            dialog,
            field_style().full_width().height(28.0).text_size(13.0),
            initial,
        );
        self.ui.set_name(field, "dialog field");
        let row = self.ui.add(dialog, Style::row().full_width().gap(SPACE_2));
        spacer(&mut self.ui, row);
        let cancel = button(&mut self.ui, row, "dialog cancel", "Cancel", false);
        let ok = button(&mut self.ui, row, "dialog ok", "OK", true);
        self.ui.focus(Some(field));
        self.ui.click(field);
        // All of it selected, so typing replaces it.
        use scrap::input::InputEvent as E;
        let cmd = if cfg!(target_os = "macos") {
            Key::LeftSuper
        } else {
            Key::LeftControl
        };
        for e in [
            E::KeyDown(cmd),
            E::KeyDown(Key::A),
            E::KeyUp(Key::A),
            E::KeyUp(cmd),
        ] {
            self.ui.handle(&e);
        }
        self.ui.events();
        self.prompt = Some(Prompt {
            overlay,
            field,
            ok,
            cancel,
            ask,
        });
    }

    fn close_prompt(&mut self) {
        if let Some(p) = self.prompt.take() {
            self.ui.remove(p.overlay);
            self.ui.focus(Some(self.viewport));
        }
    }

    /// The dialog's answer, done.
    fn answer(&mut self) {
        let Some(p) = &self.prompt else { return };
        let text = self.ui.text(p.field).unwrap_or_default().trim().to_string();
        let ask = p.ask.clone();
        self.close_prompt();
        if text.is_empty() {
            return;
        }
        if let Err(message) = self.do_answer(ask, &text) {
            self.session.say(Level::Error, message);
        }
        self.refresh();
    }

    fn do_answer(&mut self, ask: Ask, text: &str) -> Result<(), String> {
        let e = |e: scrap_editor::EditError| e.to_string();
        let s = &mut self.session;
        match ask {
            Ask::RenameAsset(file) => {
                let to = sibling(&file, text);
                s.rename_asset(&file, &to).map_err(e)?;
                s.say(Level::Info, format!("{file} is now {to}"));
            }
            Ask::DuplicateAsset(file) => {
                let to = sibling(&file, text);
                s.duplicate_asset(&file, &to).map_err(e)?;
                s.say(Level::Info, format!("copied {file} to {to}"));
            }
            Ask::SaveMaterial => {
                let id = s
                    .selected()
                    .ok_or("select something to take the colour of")?;
                s.save_material(id, text).map_err(e)?;
                s.say(Level::Info, format!("saved material {text}"));
            }
            Ask::NewComponent => {
                let project = s.project().ok_or("no project is open")?;
                let path =
                    scrap_cli::add::component(project, text).map_err(|e| format!("{e:#}"))?;
                s.say(Level::Info, format!("wrote {}", path.display()));
            }
            Ask::NewSystem => {
                let project = s.project().ok_or("no project is open")?;
                let added = scrap_cli::add::system(project, text).map_err(|e| format!("{e:#}"))?;
                s.say(Level::Info, format!("wrote {}", added.file.display()));
                if !added.called {
                    s.say(
                        Level::Warning,
                        format!("src/main.rs has no `// systems, in order` line: call {} from step yourself", scrap_cli::add::system_call(text)),
                    );
                }
            }
            Ask::Variant => {
                let id = s.selected().ok_or("select a prefab instance")?;
                s.make_variant(id, text).map_err(e)?;
                s.say(Level::Info, format!("made variant {text}"));
            }
            Ask::NewScene => {
                let path = s.new_scene(text).map_err(e)?;
                s.say(Level::Info, format!("new scene {}", path.display()));
            }
            Ask::Snap => {
                let n: Vec<f32> = text
                    .split([',', ' '])
                    .filter(|t| !t.is_empty())
                    .map(|t| t.parse::<f32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| "three numbers: metres, degrees, scale — 0.25, 15, 0.1")?;
                let [meters, degrees, scale] = n[..] else {
                    return Err("three numbers: metres, degrees, scale — 0.25, 15, 0.1".into());
                };
                s.set_snap(scrap_editor::Snap {
                    meters,
                    degrees,
                    scale,
                });
            }
            Ask::SaveLayout => {
                let dir = self.config_dir.clone().ok_or("no config folder to keep layouts in")?;
                let name = layouts::check_name(text)?.to_string();
                let layout = Layout {
                    name: Some(name.clone()),
                    ..self.layout()
                };
                let path = layouts::save(&dir, &name, &layout)?;
                self.layout_name = Some(name.clone());
                self.update_layout_button();
                self.session
                    .say(Level::Info, format!("saved layout {name} ({})", path.display()));
            }
            Ask::DeleteLayout => {
                let dir = self.config_dir.clone().ok_or("no config folder to keep layouts in")?;
                layouts::delete(&dir, text)?;
                if self.layout_name.as_deref() == Some(text) {
                    self.layout_name = None;
                    self.update_layout_button();
                }
                self.session
                    .say(Level::Info, format!("deleted layout {text}"));
            }
        }
        Ok(())
    }

    fn open_popup(&mut self, mut items: Vec<MenuItem>, x: f32, y: f32) {
        self.close_popup();
        // The keys as the keymap has them now, rebound ones too.
        self.keymap.label(&mut items);
        let root = self.ui.root();
        let (w, h, _) = self.ui.viewport();
        let overlay = self.ui.add(
            root,
            Style::column().absolute(0.0, 0.0).size(w, h).clickable(),
        );
        self.ui.set_name(overlay, "menu overlay");
        self.ui.set_layer(overlay, true);
        let rows = items.iter().filter(|i| i.action.is_some()).count() as f32;
        let seps = items.len() as f32 - rows;
        let height = rows * 26.0 + seps * 9.0 + 12.0;
        let (x, y) = (x.min(w - 240.0).max(4.0), y.min(h - height - 4.0).max(4.0));
        let menu = self.ui.add(
            overlay,
            Style::column()
                .absolute(x, y)
                .width(236.0)
                .padding(6.0)
                .background(SURFACE)
                .radius(RADIUS_MD)
                .border(1.0, NEUTRAL_800.alpha(100))
                .clip(),
        );
        self.ui.set_name(menu, "menu");
        let checks = items.iter().any(|i| i.checked.is_some());
        let mut lines = Vec::new();
        for item in items {
            match item.action {
                None => {
                    self.ui.add(
                        menu,
                        Style::row()
                            .full_width()
                            .height(1.0)
                            .margin(4.0)
                            .background(DIVIDER),
                    );
                }
                Some(action) => {
                    let style = Style::row()
                        .full_width()
                        .height(26.0)
                        .fixed()
                        .padding_x(SPACE_3)
                        .gap(SPACE_2)
                        .center_items()
                        .radius(RADIUS_SM);
                    let line = self.ui.add(
                        menu,
                        if item.disabled {
                            style
                        } else {
                            style.hover(ACCENT.alpha(16))
                        },
                    );
                    self.ui.set_name(line, format!("menu {}", item.label));
                    let ink = if item.disabled { MUTED } else { TEXT };
                    // A choice that is on or off: a check when on. Every
                    // line of a menu with one keeps room for it, so the
                    // labels line up.
                    if checks {
                        let mark = self.ui.add(line, Style::row().size(14.0, 14.0).fixed());
                        if item.checked == Some(true) {
                            icon(&mut self.ui, mark, "check", ACCENT);
                        }
                    }
                    self.ui
                        .add_text(line, text().text_color(ink).fill(), &item.label);
                    if item.disabled {
                        // Shown, and nothing when clicked.
                        continue;
                    }
                    if let Some(k) = &item.shortcut {
                        self.ui.add_text(
                            line,
                            Style::default().text_size(11.0).text_color(MUTED).nowrap(),
                            k,
                        );
                    }
                    lines.push((line, action));
                }
            }
        }
        self.popup = Some(Popup {
            overlay,
            items: lines,
        });
    }

    fn close_popup(&mut self) {
        if let Some(popup) = self.popup.take() {
            self.ui.remove(popup.overlay);
        }
    }

    fn drag_split(&mut self, which: usize, dx: f32, dy: f32) {
        let (node, delta, horizontal) = match which {
            0 => (self.left, dx, true),
            1 => (self.lower, -dy, false),
            _ => (self.right, -dx, true),
        };
        let r = self.ui.rect(node);
        let now = if horizontal { r.width } else { r.height };
        let next = (now + delta).clamp(140.0, 900.0);
        self.ui.restyle(node, |s| {
            if horizontal {
                s.width(next)
            } else {
                s.height(next)
            }
        });
    }
}

/// Put a new thing where the view looks, on the ground.
fn place_in_front(s: &mut Session, id: scrap::EntityId) {
    let (w, h) = s.size();
    let _ = s.select(Some(id));
    let _ = s.place_on_surface(w / 2, h / 2);
}

fn splitter(ui: &mut Ui, parent: NodeId, vertical: bool, name: &str) -> NodeId {
    let s = if vertical {
        Style::row().width(4.0).full_height().fixed()
    } else {
        Style::row().height(4.0).full_width().fixed()
    };
    let n = ui.add(parent, s.radius(2.0).hover(ACCENT.alpha(30)).draggable());
    ui.set_name(n, name);
    n
}

fn build_toolbar(ui: &mut Ui, root: NodeId) -> (Toolbar, NodeId) {
    let bar = ui.add(
        root,
        Style::row()
            .height(42.0)
            .fixed()
            .full_width()
            .padding_x(SPACE_3)
            .gap(SPACE_2)
            .center_items(),
    );
    ui.set_name(bar, "toolbar");
    // Three parts: Play, Pause and Step in the middle of the window, the
    // two sides sharing what is left evenly. A side wider than its half
    // pushes the middle over instead of running under it.
    let side = |ui: &mut Ui| {
        ui.add(
            bar,
            Style::row().share().full_height().gap(SPACE_2).center_items(),
        )
    };
    let left = side(ui);
    let brand = ui.add(
        left,
        Style::row().gap(SPACE_2).center_items().padding_x(SPACE_1),
    );
    ui.add(
        brand,
        Style::row().size(8.0, 8.0).radius(4.0).background(ACCENT),
    );
    ui.add_text(
        brand,
        Style::default()
            .text_size(15.0)
            .weight(500)
            .text_color(TEXT)
            .nowrap(),
        "scrap",
    );
    let mut menus = Vec::new();
    for (title, items) in menu::menu_bar() {
        let m = ui.add(
            left,
            Style::row()
                .height(26.0)
                .padding_x(SPACE_2)
                .center()
                .radius(6.0)
                .hover(HOVER)
                .pressed(PRESSED),
        );
        ui.set_name(m, format!("menu bar {title}"));
        ui.add_text(m, text().text_color(LABEL), title);
        menus.push((m, items));
    }
    let play_group = ui.add(
        bar,
        Style::row()
            .fixed()
            .gap(SPACE_1)
            .padding(2.0)
            .radius(RADIUS_MD)
            .border(1.0, DIVIDER)
            .center_items(),
    );
    let play = icon_button(ui, play_group, "play", "play", false);
    let pause = icon_button(ui, play_group, "pause", "pause", false);
    let step = icon_button(ui, play_group, "step", "step-forward", false);
    let right = side(ui);
    spacer(ui, right);
    // The document beside what saves it: its name, and a dot while it
    // has changes.
    let doc = ui.add(
        right,
        Style::row().gap(SPACE_2).center_items(),
    );
    let scene_name = ui.add_text(
        doc,
        Style::default().text_size(13.0).text_color(MUTED).nowrap(),
        "",
    );
    ui.set_name(scene_name, "scene name");
    let modified = ui.add(
        doc,
        Style::row()
            .size(6.0, 6.0)
            .radius(3.0)
            .background(ACCENT_400),
    );
    separator(ui, right);
    // Unity's Layout dropdown, beside Undo and Redo.
    let layout = ui.add(
        right,
        Style::row()
            .height(26.0)
            .padding_x(SPACE_2)
            .gap(4.0)
            .center_items()
            .radius(6.0)
            .hover(HOVER)
            .pressed(PRESSED),
    );
    ui.set_name(layout, "layout");
    ui.add_text(layout, text().text_color(LABEL), "Default");
    icon(ui, layout, "chevron-down", MUTED);
    separator(ui, right);
    let undo = icon_button(ui, right, "undo", "undo-2", false);
    let redo = icon_button(ui, right, "redo", "redo-2", false);
    separator(ui, right);
    let save = button(ui, right, "save", "Save", false);
    (
        Toolbar {
            menus,
            scene_name,
            modified,
            play,
            pause,
            step,
            play_group,
            undo,
            redo,
            save,
            layout,
        },
        bar,
    )
}

fn build_status(ui: &mut Ui, root: NodeId) -> (Status, NodeId) {
    let bar = ui.add(
        root,
        Style::row()
            .height(24.0)
            .fixed()
            .full_width()
            .padding_x(SPACE_4)
            .gap(SPACE_4)
            .center_items(),
    );
    ui.set_name(bar, "status");
    let small = || Style::default().text_size(11.0).text_color(MUTED).nowrap();
    // The Console's newest line, as Unity's status bar has it: all the
    // room the numbers leave, and a click shows the Console.
    let console = ui.add(
        bar,
        Style::row()
            .fill()
            .full_height()
            .gap(6.0)
            .center_items()
            .clip()
            .clickable(),
    );
    ui.set_name(console, "status console");
    let console_icon = ui.add_icon(
        console,
        Style::default()
            .size(12.0, 12.0)
            .fixed()
            .text_color(MUTED)
            .hidden(),
        "info",
    );
    let console_text = ui.add_text(console, small(), "");
    ui.set_name(console_text, "status console line");
    let selected = ui.add_text(bar, small(), "");
    let entities = ui.add_text(bar, small(), "");
    // Warnings and errors: a click shows the Console, wherever it is.
    let problems = ui.add_text(bar, small().clickable(), "");
    ui.set_name(problems, "status problems");
    let fps = ui.add_text(bar, small(), "");
    let mode = ui.add_text(bar, small(), "");
    (
        Status {
            console,
            console_icon,
            console_text,
            entities,
            selected,
            problems,
            mode,
        },
        fps,
    )
}

/// A word saying which of two is so, as Unity's Pivot and Global
/// buttons: a click switches it.
fn set_word(ui: &mut Ui, b: NodeId, word: &str) {
    if let Some(text) = ui.children(b).first().copied() {
        if ui.text(text) != Some(word) {
            ui.set_text(text, word);
        }
    }
}

fn word_toggle_style(on: bool) -> Style {
    let s = Style::row()
        .height(22.0)
        .fixed()
        .padding_x(SPACE_3)
        .center()
        .radius(6.0)
        .clickable();
    if on {
        s.border(1.0, ACCENT.alpha(60))
            .background(ACCENT.alpha(12))
            .hover(ACCENT.alpha(16))
    } else {
        s.border(1.0, DIVIDER).hover(HOVER)
    }
}

/// A word that is on or off: the grid.
fn word_toggle(ui: &mut Ui, parent: NodeId, name: &str, word: &str) -> NodeId {
    let b = ui.add(parent, word_toggle_style(false));
    ui.set_name(b, name);
    ui.add_text(
        b,
        Style::default().text_size(11.5).text_color(LABEL).nowrap(),
        word,
    );
    b
}

fn set_word_toggle(ui: &mut Ui, b: NodeId, on: bool) {
    ui.set_style(b, word_toggle_style(on));
    for child in ui.children(b) {
        ui.restyle(child, |s| s.text_color(if on { ACCENT } else { LABEL }));
    }
}

/// How big a button of the tool strip is, a side.
const TOOL_BUTTON: f32 = 28.0;

fn tool_button_style(on: bool) -> Style {
    let s = Style::row()
        .size(TOOL_BUTTON, TOOL_BUTTON)
        .fixed()
        .center()
        .radius(6.0)
        .clickable();
    if on {
        s.background(ACCENT.alpha(22)).hover(ACCENT.alpha(28))
    } else {
        s.hover(HOVER).pressed(PRESSED)
    }
}

/// Unity's Tools overlay: the hand and the gizmo's three, a card down the
/// Scene view's top left corner. Returns the card and its buttons.
fn build_tool_strip(ui: &mut Ui, frame: NodeId) -> (NodeId, [NodeId; 6]) {
    let card = ui.add(
        frame,
        Style::column()
            .absolute(8.0, 8.0)
            .padding(3.0)
            .gap(2.0)
            .radius(RADIUS_MD)
            .background(SURFACE.alpha(92))
            .border(1.0, NEUTRAL_800),
    );
    ui.set_layer(card, true);
    ui.set_name(card, "tools");
    let buttons = TOOLS.map(|(word, glyph, _)| {
        let b = ui.add(card, tool_button_style(false));
        ui.set_name(b, format!("tool {word}"));
        icon(ui, b, glyph, LABEL);
        b
    });
    (card, buttons)
}

fn set_tool_button(ui: &mut Ui, b: NodeId, on: bool) {
    ui.set_style(b, tool_button_style(on));
    for child in ui.children(b) {
        ui.restyle(child, |s| s.text_color(if on { ACCENT } else { LABEL }));
    }
}

fn view_tab_style(on: bool) -> Style {
    let s = Style::row()
        .height(24.0)
        .padding_x(SPACE_3)
        .gap(6.0)
        .center_items()
        .radius(6.0)
        .clickable();
    if on {
        s.background(SURFACE).hover(SURFACE)
    } else {
        s.hover(HOVER)
    }
}

fn view_tab(ui: &mut Ui, parent: NodeId, name: &str, glyph: &str, label: &str, on: bool) -> NodeId {
    let t = ui.add(parent, view_tab_style(on));
    ui.set_name(t, name);
    icon(ui, t, glyph, if on { ACCENT } else { LABEL });
    ui.add_text(t, text().text_color(if on { TEXT } else { LABEL }), label);
    t
}

fn set_view_tab(ui: &mut Ui, tab: NodeId, on: bool) {
    ui.set_style(tab, view_tab_style(on));
    let kids = ui.children(tab);
    ui.restyle(kids[0], |s| s.text_color(if on { ACCENT } else { LABEL }));
    ui.restyle(kids[1], |s| s.text_color(if on { TEXT } else { LABEL }));
}

/// What has the whole window below the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zoom {
    /// The Scene or Game view.
    View,
    /// The stack holding this panel, in whichever area.
    Stack(Panel),
}

/// What a control does, for its tooltip — by the control's name.
fn tooltip(name: &str) -> Option<&'static str> {
    Some(match name {
        "tool Hand" => "Hand: drag to pan the view, pick nothing",
        "tool Move" => "Move",
        "tool Rotate" => "Rotate",
        "tool Scale" => "Scale",
        "tool Rect" => "Rect: resize by the bounds' corners and edges",
        "tool Transform" => "Transform: move, rotate and scale at once",
        "handles along" => "Handles along the world's axes or the entity's own",
        "handles at" => "Handles on the entity's pivot or the selection's centre",

        "grid" => "Show the grid",
        "status console" => "The Console's newest line: click to show the Console",
        "play" => "Play the game in the Game view / Stop",
        "pause" => "Simulate physics here, paused",
        "step" => "One step of physics simulated here",
        "undo" => "Undo",
        "redo" => "Redo",
        "save" => "Save the scene",
        "snap" => "Snap moves to ¼ m, turns to 15°, scale to 0.1",
        "colliders" => "Show colliders",
        "scene view" => "The view over the whole window",
        "sculpt" => "Terrain brush: left raises, Shift lowers, Ctrl/Cmd flattens; Alt still orbits",
        "faces" => "Face mode: drag a face of a box to push it; Alt still orbits",
        "foliage" => "Foliage brush: paint the model chosen in Project; Shift erases, [ ] size",
        "compass middle" => "Perspective or isometric",
        "compass right" => "Look from the right (+X)",
        "compass top" => "Look down from above (+Y)",
        "compass front" => "Look from the front (+Z)",
        "compass left" => "Look from the left (−X)",
        "compass bottom" => "Look up from below (−Y)",
        "compass back" => "Look from the back (−Z)",
        "view scene" => "The Scene view: edit (double-click: over the whole window)",
        "view game" => "The Game view: what the game's camera sees (double-click: over the whole window)",
        "hierarchy expand all" => "Expand all (Alt click an arrow: all under it)",
        "hierarchy collapse all" => "Collapse all",
        "console clear" => "Clear the Console",
        "status problems" => "Show the Console",
        _ => return None,
    })
}

/// The keymap's command a control does too, whose key its tooltip shows.
fn tooltip_key(name: &str) -> Option<&'static str> {
    Some(match name {
        "tool Hand" => "hand",
        "tool Move" => "move",
        "tool Rotate" => "rotate",
        "tool Scale" => "scale",
        "tool Rect" => "rect",
        "tool Transform" => "transform",
        "handles along" => "space",
        "handles at" => "pivot",
        "play" => "play",
        "pause" => "pause",
        "step" => "step",
        "undo" => "undo",
        "redo" => "redo",
        "save" => "save_scene",
        "scene view" => "maximize",
        _ => return None,
    })
}

/// How big the orientation gizmo is, a side.
const COMPASS: f32 = 84.0;

struct Compass {
    /// +X, +Y, +Z, −X, −Y, −Z, and the side each looks from.
    axes: Vec<(NodeId, scrap_editor::Side)>,
    /// The dial the axes stand on: what they are put in, back to front.
    dial: NodeId,
    /// Each positive axis's line from the middle, as beads.
    beads: [Vec<NodeId>; 3],
    /// The middle: a click switches perspective and orthographic.
    middle: NodeId,
    /// Which of the two, under the dial.
    label: NodeId,
    seen: Option<(scrap::glam::Vec3, scrap::glam::Vec3, bool)>,
}

/// Beads on each axis's line.
const BEADS: usize = 7;

/// Unity's scene gizmo: the axes as the camera sees them, in the view's
/// top right corner. A click on an axis looks from it; the label under it
/// switches perspective and orthographic.
fn build_compass(ui: &mut Ui, frame: NodeId) -> Compass {
    use scrap_editor::Side;
    let holder = ui.add(
        frame,
        Style::row()
            .absolute(0.0, 0.0)
            .full_width()
            .height(COMPASS + 30.0),
    );
    ui.add(holder, Style::row().fill());
    let column = ui.add(
        holder,
        Style::column().margin(4.0).gap(2.0).center_items(),
    );
    // A disc under the axes, so they read as one thing to turn the view
    // by, lit when the pointer is on it.
    let dial = ui.add(
        column,
        Style::row()
            .size(COMPASS, COMPASS)
            .radius(COMPASS / 2.0)
            .background(NEUTRAL_900.alpha(28))
            .hover(NEUTRAL_900.alpha(60)),
    );
    ui.set_layer(dial, true);
    ui.set_name(dial, "compass");
    let colors = [AXIS_X, AXIS_Y, AXIS_Z];
    let beads = colors.map(|color| {
        (0..BEADS)
            .map(|_| {
                ui.add(
                    dial,
                    Style::row()
                        .absolute(0.0, 0.0)
                        .size(3.0, 3.0)
                        .radius(1.5)
                        .background(color),
                )
            })
            .collect::<Vec<_>>()
    });
    let middle = ui.add(
        dial,
        Style::row()
            .absolute(COMPASS / 2.0 - 7.0, COMPASS / 2.0 - 7.0)
            .size(14.0, 14.0)
            .radius(7.0)
            .background(NEUTRAL_800)
            .border(1.0, NEUTRAL_500)
            .hover_border(TEXT)
            .clickable(),
    );
    ui.set_name(middle, "compass middle");
    let label = ui.add_text(
        column,
        Style::default().text_size(9.5).text_color(LABEL).nowrap(),
        "Persp",
    );
    let sides = [
        Side::Right,
        Side::Top,
        Side::Front,
        Side::Left,
        Side::Bottom,
        Side::Back,
    ];
    let mut axes = Vec::new();
    for (i, side) in sides.into_iter().enumerate() {
        let positive = i < 3;
        let size = if positive { 18.0 } else { 12.0 };
        let color = colors[i % 3];
        let dot = ui.add(
            dial,
            Style::row()
                .absolute(0.0, 0.0)
                .size(size, size)
                .radius(size / 2.0)
                .center()
                .background(if positive { color } else { color.alpha(30) })
                .border(1.0, color)
                .hover_border(TEXT)
                .clickable(),
        );
        ui.set_name(dot, format!("compass {}", side.name()));
        if positive {
            ui.add_text(
                dot,
                Style::default()
                    .text_size(10.0)
                    .weight(600)
                    .text_color(BG)
                    .nowrap(),
                ["X", "Y", "Z"][i],
            );
        }
        axes.push((dot, side));
    }
    Compass {
        axes,
        dial,
        beads,
        middle,
        label,
        seen: None,
    }
}

/// `file` renamed to `name`, in its folder, with its extension.
fn sibling(file: &str, name: &str) -> String {
    let path = std::path::Path::new(file);
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let name = name.strip_suffix(&ext).unwrap_or(name);
    match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(dir) => format!("{}/{name}{ext}", dir.display()),
        None => format!("{name}{ext}"),
    }
}

/// Show a file in the system's file manager.
fn reveal(path: &std::path::Path) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-R");
        c
    } else if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("explorer");
        c.arg("/select,");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    let target = if cfg!(any(target_os = "macos", target_os = "windows")) {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };
    command.arg(target).spawn().map(|_| ())
}
