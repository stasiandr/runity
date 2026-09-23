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

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use runity::edit::Face;
use runity::gizmo::Tool;
use runity::input::{Input, InputEvent, Key, MouseButton};
use runity::EntityId;
use runity_editor::console::Level;
use runity_editor::{Pivot, Session, Space};
use runity_ui::render::UiRenderer;
use runity_ui::{Event, ImageId, NodeId, Style, Ui};

use runity_ui::Clipboard as _;

use crate::animator::Animator;
use crate::bottom::{Asset, Bottom};
use crate::clipboard::SystemClipboard;
use crate::dock::{Docked, Docks, Panel};
use crate::hierarchy::Hierarchy;
use crate::inspector::Inspector;
use crate::menu::{self, Action, MenuItem};
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
}

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
    Entity(runity::EntityId),
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
    tools: [NodeId; 3],
    space: NodeId,
    pivot: NodeId,
    grid: NodeId,
    play: NodeId,
    pause: NodeId,
    step: NodeId,
    play_group: NodeId,
    undo: NodeId,
    redo: NodeId,
    save: NodeId,
}

struct Status {
    entities: NodeId,
    selected: NodeId,
    last: NodeId,
    problems: NodeId,
    mode: NodeId,
}

/// What the panels show, in brief: when it is the same as last frame's
/// they are left alone.
#[derive(PartialEq)]
struct Stamp {
    selection: Vec<runity::EntityId>,
    revision: u64,
    console: (usize, usize, usize),
    lines: usize,
    tool: Tool,
    playing: bool,
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
            playing: session.is_playing(),
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
    snap: NodeId,
    colliders_button: NodeId,
    sculpt_button: NodeId,
    docks: Docks,
    settings: Settings,
    profiler: Profiler,
    animation: Animation,
    screens: Screens,
    animator: Animator,
    /// What the last draw cost, for the Profiler.
    last_draw_ms: f32,
    /// The sound device, opened the first time a sound is listened to, and
    /// what is playing on it.
    audio: Option<runity::audio::Audio>,
    listening: Option<runity::audio::Playing>,
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
    /// whether the view has the whole window.
    panels: [bool; 3],
    maximized: bool,
    /// The terrain brush is on: a left drag in the view shapes the ground.
    sculpt: bool,
    /// Face mode: ProBuilder's face selection over the faces of boxes.
    faces: bool,
    faces_button: NodeId,
    /// The face under the pointer, and the one being dragged: which, where
    /// the drag began, the screen's pixels per metre out along the face,
    /// and how far it has been pushed so far.
    face_hover: Option<(EntityId, Face)>,
    face_drag: Option<FaceDrag>,
    face_box: NodeId,
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
    inspector: Inspector,
    bottom: Bottom,
    status: Status,
    splits: [NodeId; 3],
    view_slot: NodeId,
    /// The lower dock's height before the UI Builder went wide.
    lower_before_wide: Option<f32>,
    /// When the theme file was last read.
    theme_stamp: Option<std::time::SystemTime>,
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
        let tab_scene = view_tab(&mut ui, view_tabs, "view scene", "hand", "Scene", true);
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
        let mut buttons = Vec::new();
        for (name, label, action) in [
            ("view persp", "Persp", Action::Perspective),
            ("view top", "Top", Action::View(runity_editor::Side::Top)),
            (
                "view front",
                "Front",
                Action::View(runity_editor::Side::Front),
            ),
            (
                "view right",
                "Right",
                Action::View(runity_editor::Side::Right),
            ),
        ] {
            let b = ui.add(
                view_tabs,
                Style::row()
                    .height(24.0)
                    .padding_x(SPACE_2)
                    .center()
                    .radius(6.0)
                    .hover(HOVER)
                    .pressed(PRESSED),
            );
            ui.set_name(b, name);
            ui.add_text(
                b,
                Style::default().text_size(11.5).text_color(LABEL).nowrap(),
                label,
            );
            buttons.push((b, action));
        }
        separator(&mut ui, view_tabs);
        let snap = icon_button(&mut ui, view_tabs, "snap", "magnet", false);
        buttons.push((snap, Action::ToggleSnap));
        let colliders = icon_button(&mut ui, view_tabs, "colliders", "box", false);
        buttons.push((colliders, Action::ToggleColliders));
        let sculpt = icon_button(&mut ui, view_tabs, "sculpt", "mountain", false);
        buttons.push((sculpt, Action::ToggleSculpt));
        let faces = icon_button(&mut ui, view_tabs, "faces", "square", false);
        buttons.push((faces, Action::ToggleFaces));
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
                .border(1.0, runity_ui::Color::TRANSPARENT)
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
        let profiler = Profiler::new(&mut ui, lower);
        roots.insert(Panel::Settings, settings.root);
        roots.insert(Panel::Profiler, profiler.root);
        let animation = Animation::new(&mut ui, lower);
        roots.insert(Panel::Animation, animation.root);
        let screens = Screens::new(&mut ui, lower);
        roots.insert(Panel::Screens, screens.root);
        let animator = Animator::new(&mut ui, lower);
        roots.insert(Panel::Animator, animator.root);
        let docks = Docks::new(
            &mut ui,
            [left, right, lower],
            roots,
            Docks::default_layout(),
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
            snap,
            colliders_button: colliders,
            sculpt_button: sculpt,
            sculpt: false,
            faces: false,
            faces_button: faces,
            face_hover: None,
            face_drag: None,
            face_box,
            panels: [true; 3],
            last_input: Instant::now(),
            docks,
            settings,
            profiler,
            animation,
            screens,
            animator,
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
            maximized: false,
            stroke: None,
            tooltip: None,
            resting: None,
            polled: Instant::now(),
            toolbar,
            hierarchy,
            inspector,
            bottom,
            status,
            splits: [split_left, split_lower, split_right],
            view_slot,
            lower_before_wide: None,
            theme_stamp: None,
            left,
            right,
            lower,
            popup: None,
            prompt: None,
            search: None,
            navigation: false,
            seen: None,
            scene_buttons: HashSet::new(),
            registered: None,
            drawn: Instant::now(),
            frame_times: Vec::new(),
            fps,
            discard_asked: None,
            colliders: false,
            conflict_said: false,
            job: None,
            saved_layout: String::new(),
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
        studio.sync_visible();
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
    pub fn ime_area(&self) -> Option<runity_ui::Rect> {
        self.ui.caret_rect()
    }

    /// What the pointer should look like where it is: an I-beam over a
    /// field, a resize arrow over a border between panels.
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

    /// What the window's title says: the document, and a dot when it has
    /// unsaved edits.
    pub fn title(&self) -> String {
        let name = self
            .session
            .scene_path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());
        let dot = if self.session.is_modified() {
            " ●"
        } else {
            ""
        };
        let prefab = if self.session.is_prefab() {
            " (prefab)"
        } else {
            ""
        };
        format!("runity — {name}{prefab}{dot}")
    }

    /// The window changed size or moved to a screen with another scale.
    pub fn resize(&mut self, width: f32, height: f32, scale: f32) {
        self.ui.set_viewport(width, height, scale);
    }

    /// Where the Scene view is, in logical pixels.
    pub fn scene_rect(&mut self) -> runity_ui::Rect {
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
            InputEvent::KeyDown(key) if !typing => {
                if matches!(key, Key::Up | Key::Down | Key::Left | Key::Right) {
                    let shift = self.ui.modifiers().0;
                    if self.hierarchy.key(&mut self.session, *key, shift) {
                        self.refresh();
                        return;
                    }
                }
                let (_, ctrl, _, command) = self.ui.modifiers();
                if *key == Key::K && (ctrl || command) {
                    self.open_search();
                    return;
                }
                if *key == Key::Space && self.ui.modifiers().0 {
                    self.run(Action::Maximize);
                    return;
                }
                if *key == Key::F2 {
                    self.hierarchy.rename_selected(&mut self.ui, &self.session);
                    return;
                }
                if *key == Key::Escape && self.popup.is_some() {
                    self.close_popup();
                    return;
                }
                self.scene_input.handle(event);
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

        let timing = std::env::var_os("RUNITY_STUDIO_TIMING").is_some();
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
        let t1 = Instant::now();
        let _ = self.session.scene_view(&self.scene_input, dt);
        self.scene_input.begin_frame();
        let t2 = Instant::now();
        self.session.render();
        self.camera_preview();
        let t3 = Instant::now();

        self.poll_disk();
        self.bottom.update_git(&mut self.ui, &mut self.session);
        // Two of the Project's pictures a frame, until it has them all.
        for (name, image) in self.bottom.wanted_pictures(2) {
            if let Ok(pixels) = self.session.thumbnail(&name, 128) {
                self.pending_images.push((image, 128, pixels));
            }
            self.bottom.picture_ready(&mut self.ui, &name);
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
        self.turn_compass();

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
        {
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f32() * 1e3;
            self.profiler.record(FrameCost {
                input: ms(t1, t2),
                render: ms(t2, t3),
                panels: ms(t5, Instant::now()) + ms(t0, t1),
                ui: self.last_draw_ms,
            });
            let live = self.wants_frame();
            if live && self.docks.is_active(Panel::Profiler) {
                self.profiler.update(&mut self.ui);
            }
            if self.docks.is_active(Panel::Animation) {
                self.animation.update(&mut self.ui, &self.session);
            }
            if self.docks.is_active(Panel::Screens) {
                self.screens.update(&mut self.ui, &self.session);
                self.screens.draw(&self.session);
            }
            if self.docks.is_active(Panel::Animator) {
                self.animator.update(&mut self.ui, &self.session);
            }
            self.fit_wide();
            if self.docks.is_active(Panel::Settings) {
                self.settings.update(&mut self.ui, &self.session);
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
            if std::env::var_os("RUNITY_STUDIO_FPS").is_some() {
                eprintln!("{fps:.0} fps, frame {:.1} ms", mean * 1e3);
            }
            self.frame_times.clear();
        }
    }

    /// The UI Builder over the whole window below the toolbar, or back in
    /// its dock.
    fn fit_wide(&mut self) {
        let under = |panel| self.docks.is_active(panel) && self.docks.dock_of(panel) == Some(2);
        let wide = (self.screens.wide && under(Panel::Screens))
            || (self.animator.wide && under(Panel::Animator));
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

    /// Show the panels that are on, or only the view when it is maximized.
    fn show_panels(&mut self) {
        let [left, lower, right] = self.splits;
        let slots = [(self.left, left), (self.right, right), (self.lower, lower)];
        for (i, (slot, split)) in slots.into_iter().enumerate() {
            let on = self.panels[i] && !self.maximized;
            for n in [slot, split] {
                self.ui
                    .restyle(n, |s| if on { s.shown() } else { s.hidden() });
            }
        }
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
        use runity::glam::Vec3;
        let forward = (camera.target - camera.position).normalize_or_zero();
        let right = forward.cross(camera.up).normalize_or_zero();
        let up = right.cross(forward);
        let (c, reach) = (COMPASS / 2.0, COMPASS / 2.0 - 11.0);
        for (i, (node, _)) in self.compass.axes.clone().into_iter().enumerate() {
            let axis = [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X, -Vec3::Y, -Vec3::Z][i];
            let x = c + axis.dot(right) * reach;
            let y = c - axis.dot(up) * reach;
            let away = axis.dot(forward) > 0.1;
            let size = if i < 3 { 20.0 } else { 14.0 };
            self.ui.restyle(node, |s| {
                s.absolute(x - size / 2.0, y - size / 2.0)
                    .opacity(if away { 0.45 } else { 1.0 })
            });
        }
        let label = if camera.ortho.is_some() {
            "iso"
        } else {
            "persp"
        };
        if let Some(t) = self.ui.children(self.compass.middle).first().copied() {
            self.ui.set_text(t, label);
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
                            .is_some_and(|p| p.assets().join(format!("{m}.rterrain")).is_file())
                    })
                })
            });
        let Some(terrain) = terrain else {
            self.session.say(
                Level::Warning,
                "no terrain to sculpt: GameObject › Terrain makes one",
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
    /// settings, in the project's `.runity/` (not in git).
    fn layout_file(&self) -> Option<std::path::PathBuf> {
        self.session
            .project()
            .map(|p| p.root().join(".runity").join("studio.ron"))
    }

    /// The panels' sizes and the bottom tab, as RON.
    fn layout_text(&self) -> String {
        let w = |n: NodeId| self.ui.rect(n).width.round();
        let (docks, active) = self.docks.layout();
        format!(
            "(left: {:.0}, right: {:.0}, lower: {:.0}, docks: {docks:?}, active: {active:?})\n",
            w(self.left),
            w(self.right),
            self.lower_before_wide
                .unwrap_or(self.ui.rect(self.lower).height)
                .round(),
        )
    }

    /// Put the panels back where they were last time.
    fn restore_layout(&mut self) {
        let Some(text) = self
            .layout_file()
            .and_then(|f| std::fs::read_to_string(f).ok())
        else {
            return;
        };
        let number = |key: &str| -> Option<f32> {
            let at = text.find(&format!("{key}:"))? + key.len() + 1;
            text[at..].split([',', ')']).next()?.trim().parse().ok()
        };
        if let Some(v) = number("left") {
            self.ui
                .restyle(self.left, |s| s.width(v.clamp(140.0, 900.0)));
        }
        if let Some(v) = number("right") {
            self.ui
                .restyle(self.right, |s| s.width(v.clamp(140.0, 900.0)));
        }
        if let Some(v) = number("lower") {
            self.ui
                .restyle(self.lower, |s| s.height(v.clamp(60.0, 900.0)));
        }
        let quoted = |key: &str| -> Option<String> {
            let at = text.find(&format!("{key}:"))? + key.len() + 1;
            let rest = text[at..].trim().strip_prefix('"')?;
            Some(rest.split('"').next()?.to_string())
        };
        if let Some(layout) = quoted("docks").as_deref().and_then(Docks::parse) {
            for (i, panels) in layout.iter().enumerate() {
                for panel in panels {
                    self.docks.move_panel(&mut self.ui, *panel, i);
                }
            }
        }
        if let Some(active) = quoted("active") {
            for name in active.split('|') {
                if let Some(panel) = Panel::from_name(name) {
                    self.docks.activate(&mut self.ui, panel);
                }
            }
        }
        self.sync_visible();
        self.saved_layout = self.layout_text_after_paint();
    }

    /// Tell the lower panels which of them are on top.
    fn sync_visible(&mut self) {
        let on = |p| self.docks.is_active(p);
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
    /// `.runity/theme.ron`, when it changed: Nocturne's colours drawn as
    /// the file says, with the editor running. A file that does not read
    /// says why in the Console and the last good colours stay.
    fn poll_theme(&mut self) {
        let Some(path) = self
            .session
            .project()
            .map(|p| p.root().join(".runity").join("theme.ron"))
        else {
            return;
        };
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if stamp == self.theme_stamp {
            return;
        }
        self.theme_stamp = stamp;
        if stamp.is_none() {
            self.ui.set_palette(Default::default());
            return;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        match crate::theme::palette(&text) {
            Ok(palette) => self.ui.set_palette(palette),
            Err(e) => self
                .session
                .say(Level::Error, format!("{}: {e}", path.display())),
        }
    }

    fn poll_disk(&mut self) {
        if self.polled.elapsed().as_secs_f32() < 0.5 {
            return;
        }
        self.polled = Instant::now();
        self.save_layout();
        self.poll_theme();
        match self.session.reload_scene() {
            Ok(runity_editor::SceneReload::Reloaded) => {
                self.session.say(
                    Level::Info,
                    "the scene changed on disk and was reloaded (undo takes it back)",
                );
            }
            Ok(runity_editor::SceneReload::Conflict) => {
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
        let n = self.session.reload_assets();
        if n > 0 {
            self.session.say(
                Level::Info,
                format!("{n} assets changed on disk and were reloaded"),
            );
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
        let Some(tip) = self.ui.name(node).and_then(tooltip) else {
            return;
        };
        let r = self.ui.rect(node);
        let root = self.ui.root();
        let (w, h, _) = self.ui.viewport();
        let x = r.x.min(w - 320.0).max(4.0);
        let y = if r.y + r.height + 34.0 > h {
            r.y - 30.0
        } else {
            r.y + r.height + 6.0
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
        }
        if let Some(target) = self.session.preview_target() {
            let size = (target.width, target.height);
            if self.cam_registered != Some(size) {
                renderer.set_image(self.session.gpu(), CAMERA_PREVIEW, target.view());
                self.cam_registered = Some(size);
            }
        }
        if self.screens.new_target {
            if let Some(target) = self.screens.target() {
                renderer.set_image(self.session.gpu(), screens::CANVAS, target.view());
                self.screens.new_target = false;
            }
        }
        for (image, size, pixels) in self.pending_images.drain(..) {
            renderer.set_image_rgba(self.session.gpu(), image, size, size, &pixels);
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
        self.inspector.update(&mut self.ui, s);
        let t3 = Instant::now();
        self.update_toolbar();
        self.update_status();
        if std::env::var_os("RUNITY_STUDIO_TIMING").is_some() {
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
        for (i, tool) in [Tool::Move, Tool::Rotate, Tool::Scale]
            .into_iter()
            .enumerate()
        {
            set_segment(ui, t.tools[i], s.tool() == tool);
        }
        set_icon_button(
            ui,
            t.space,
            if s.space() == Space::Local {
                "box"
            } else {
                "globe"
            },
            false,
            true,
        );
        set_icon_button(
            ui,
            t.pivot,
            if s.pivot() == Pivot::Center {
                "circle-dot"
            } else {
                "crosshair"
            },
            false,
            true,
        );
        set_icon_button(ui, t.grid, "grid-3x3", s.show_grid(), true);
        let playing = s.is_playing();
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
        let game = s.is_game_view();
        set_view_tab(ui, self.tab_scene, !game);
        set_view_tab(ui, self.tab_game, game);
        ui.restyle(self.view_frame, |st| {
            st.border(
                1.0,
                if playing {
                    ACCENT
                } else {
                    runity_ui::Color::TRANSPARENT
                },
            )
        });
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
                "nothing selected".to_string()
            } else {
                format!("{n} selected")
            },
        );
        ui.set_text(
            self.status.last,
            &s.undo_label()
                .map(|l| format!("last: {l}"))
                .unwrap_or_default(),
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
            (true, true) => "paused",
            (true, false) => "playing",
            _ if s.is_prefab() => "prefab mode",
            _ => "editing",
        };
        ui.set_text(self.status.mode, mode);
        ui.restyle(self.status.mode, |st| {
            st.text_color(if s.is_playing() { ACCENT } else { MUTED })
        });
    }

    // --- events ----------------------------------------------------------

    fn dispatch(&mut self, node: NodeId, event: &Event, requests: &mut Requests) {
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
        if let Some(Docked::Handled) = self.docks.event(&mut self.ui, node, event) {
            self.sync_visible();
            if !matches!(event, Event::Drag { .. }) {
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
            self.hierarchy
                .event(&mut self.ui, &mut self.session, node, event, requests);
        } else if self.inspector.owns(node) {
            self.inspector
                .event(&mut self.ui, &mut self.session, node, event, requests);
        } else if self.settings.owns(&self.ui, node) {
            self.settings
                .event(&mut self.ui, &mut self.session, node, event);
        } else if self.animator.owns(&self.ui, node) {
            self.animator
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
        if node == self.status.problems {
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
            requests.action = Some(Action::GameView(node == self.tab_game));
            requests.keyboard_to_scene = true;
            return true;
        }
        let t = &self.toolbar;
        if let Some((_, items)) = t.menus.iter().find(|(n, _)| *n == node) {
            let r = self.ui.rect(node);
            // Undo and Redo say what they would do, as Unity's Edit menu.
            let mut items = items.clone();
            for item in &mut items {
                match item.action {
                    Some(Action::Undo) => {
                        item.label = match self.session.undo_label() {
                            Some(l) => format!("Undo {l}"),
                            None => "Undo".into(),
                        }
                    }
                    Some(Action::Redo) => {
                        item.label = match self.session.redo_label() {
                            Some(l) => format!("Redo {l}"),
                            None => "Redo".into(),
                        }
                    }
                    _ => {}
                }
            }
            requests.menu = Some((items, r.x, r.y + r.height + 4.0));
            return true;
        }
        let action = if let Some(i) = t.tools.iter().position(|n| *n == node) {
            Action::Tool([Tool::Move, Tool::Rotate, Tool::Scale][i])
        } else if node == t.space {
            Action::ToggleSpace
        } else if node == t.pivot {
            Action::TogglePivot
        } else if node == t.grid {
            Action::ToggleGrid
        } else if node == t.play {
            Action::Play
        } else if node == t.pause {
            Action::Pause
        } else if node == t.step {
            Action::Step
        } else if node == t.undo {
            Action::Undo
        } else if node == t.redo {
            Action::Redo
        } else if node == t.save {
            Action::Save
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
        let e = |e: runity_editor::EditError| e.to_string();
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
                        runity_editor::Snap::INCREMENT
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
                            .then(runity::navigation::NavSettings::default),
                    );
                }
                Action::Search => self.open_search(),
                Action::PlaySound(name) => {
                    let sound = s
                        .sound(&name)
                        .ok_or_else(|| format!("no sound called {name} in the library"))?;
                    if self.audio.is_none() {
                        self.audio = Some(
                            runity::audio::Audio::new()
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
                Action::Import => {
                    if let Some(paths) = rfd::FileDialog::new()
                        .add_filter(
                            "model, texture, sound",
                            &[
                                "gltf", "glb", "obj", "png", "jpg", "jpeg", "wav", "ogg",
                                "rterrain", "rpoly",
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
                Action::Save => {
                    s.save_scene(None).map_err(e)?;
                    s.say(Level::Info, "saved");
                }
                Action::CheckProject => {
                    let project = s.project().ok_or("no project is open")?;
                    let findings = runity_cli::check(project);
                    if findings.is_empty() {
                        s.say(Level::Info, "check: nothing wrong");
                    }
                    for f in findings {
                        let level = match f.severity {
                            runity_cli::Severity::Error => Level::Error,
                            runity_cli::Severity::Warning => Level::Warning,
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
                        let result = runity_cli::build::build(&project, &out, true)
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
                Action::Undo => {
                    s.undo().map_err(e)?;
                }
                Action::Redo => {
                    s.redo().map_err(e)?;
                }
                Action::Copy => {
                    let text = s.copy_selection();
                    self.clipboard.set(text);
                }
                Action::Paste => {
                    let text = self.clipboard.get().unwrap_or_default();
                    s.paste(&text, None).map_err(e)?;
                }
                Action::Duplicate => {
                    s.duplicate_selection().map_err(e)?;
                }
                Action::Delete => {
                    s.delete_selection().map_err(e)?;
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
                Action::DropToGround => {
                    s.drop_to_ground().map_err(e)?;
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
                Action::Isolate => {
                    let ids = s.selection();
                    s.isolate(&ids).map_err(e)?;
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
                        runity_editor::Snap {
                            meters: 0.0,
                            degrees: 0.0,
                            scale: 0.0,
                        }
                    } else {
                        runity_editor::Snap::INCREMENT
                    });
                }
                Action::ToggleColliders => {
                    self.colliders = !self.colliders;
                    s.set_show_colliders(self.colliders);
                }
                Action::Tool(tool) => s.set_tool(tool),
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
                    // Play looks through the game's eyes, as Unity's Play
                    // brings up the Game view; stop goes back.
                    if s.is_playing() {
                        s.stop();
                        s.set_game_view(false);
                    } else {
                        s.play();
                        s.set_game_view(true);
                    }
                }
                Action::GameView(game) => s.set_game_view(game),
                Action::Pause => {
                    if !s.is_playing() {
                        s.play();
                    }
                    let now = s.is_paused();
                    s.pause(!now);
                }
                Action::Step => {
                    if !s.is_playing() {
                        s.play();
                    }
                    s.step_once();
                }
                Action::SetSub(component, key, value) => {
                    self.inspector.pick_sub(s, &component, &key, &value);
                }
                Action::SetField(field, value) => {
                    self.inspector.set_field(s, &field, &value);
                }
                Action::Place(name) => {
                    let (w, h) = s.size();
                    s.drop_asset(&name, w / 2, h / 2).map_err(e)?;
                }
                Action::ClearConsole => s.clear_console(),
                Action::PolyFloor | Action::PolyWall => {
                    let standing = action == Action::PolyWall;
                    let base = if standing { "wall" } else { "floor" };
                    // A name no asset has yet: floor, floor_2, floor_3…
                    let mut name = base.to_string();
                    let mut n = 1;
                    while s.has_model(&name) {
                        n += 1;
                        name = format!("{base}_{n}");
                    }
                    let source = runity_import::poly::PolySource {
                        points: if standing {
                            vec![(-2.0, 0.0), (2.0, 0.0), (2.0, 3.0), (-2.0, 3.0)]
                        } else {
                            vec![(-2.0, -2.0), (2.0, -2.0), (2.0, 2.0), (-2.0, 2.0)]
                        },
                        height: 0.2,
                        standing,
                        holes: Vec::new(),
                    };
                    let at = s.camera().target;
                    s.poly_shape(&name, &source, at).map_err(e)?;
                }
                Action::PushFace(face, metres) => {
                    let ids = s.selection();
                    if ids.is_empty() {
                        return Err("select something to push a face of".into());
                    }
                    for id in ids {
                        s.push_face(id, face, metres).map_err(e)?;
                    }
                }
                Action::Array(count) => {
                    let id = s.selected().ok_or("select something to repeat")?;
                    let width = s
                        .world_bounds(id)
                        .map(|(lo, hi)| (hi.x - lo.x).max(0.1))
                        .unwrap_or(1.0);
                    s.array(id, count, runity::glam::Vec3::new(width, 0.0, 0.0))
                        .map_err(e)?;
                }
                Action::Scatter => {
                    let id = s
                        .selected()
                        .ok_or("select something to scatter copies of")?;
                    let what = s
                        .entity_prefab(id)
                        .or_else(|| s.entity_model(id))
                        .ok_or("the selection has no model or prefab to scatter")?;
                    let centre = s.camera().target;
                    s.scatter(None, &what, centre, &runity::edit::Scatter::default())
                        .map_err(e)?;
                }
                Action::TogglePanel(i) => {
                    self.maximized = false;
                    self.panels[i] = !self.panels[i];
                    self.show_panels();
                }
                Action::Maximize => {
                    self.maximized = !self.maximized;
                    self.show_panels();
                }
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
                Action::Align(axis, to) => {
                    s.align_selection(axis, to).map_err(e)?;
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
                        runity_ui::Color::TRANSPARENT
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
        use runity::input::InputEvent as E;
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
        let e = |e: runity_editor::EditError| e.to_string();
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
                    runity_cli::add::component(project, text).map_err(|e| format!("{e:#}"))?;
                s.say(Level::Info, format!("wrote {}", path.display()));
            }
            Ask::NewSystem => {
                let project = s.project().ok_or("no project is open")?;
                let added = runity_cli::add::system(project, text).map_err(|e| format!("{e:#}"))?;
                s.say(Level::Info, format!("wrote {}", added.file.display()));
                if !added.called {
                    s.say(
                        Level::Warning,
                        format!("src/main.rs has no `// systems, in order` line: call {} from step yourself", runity_cli::add::system_call(text)),
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
                s.set_snap(runity_editor::Snap {
                    meters,
                    degrees,
                    scale,
                });
            }
        }
        Ok(())
    }

    fn open_popup(&mut self, items: Vec<MenuItem>, x: f32, y: f32) {
        self.close_popup();
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
                    let line = self.ui.add(
                        menu,
                        Style::row()
                            .full_width()
                            .height(26.0)
                            .fixed()
                            .padding_x(SPACE_3)
                            .gap(SPACE_2)
                            .center_items()
                            .radius(RADIUS_SM)
                            .hover(ACCENT.alpha(16)),
                    );
                    self.ui.set_name(line, format!("menu {}", item.label));
                    self.ui.add_text(line, text().fill(), &item.label);
                    if let Some(k) = item.shortcut {
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
fn place_in_front(s: &mut Session, id: runity::EntityId) {
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

fn segment_style(on: bool, first: bool) -> Style {
    let mut s = Style::row()
        .full_height()
        .padding_x(SPACE_3)
        .gap(6.0)
        .center_items()
        .clickable();
    if !first {
        s = s.border(0.0, DIVIDER);
    }
    if on {
        s.background(ACCENT.alpha(12)).hover(ACCENT.alpha(16))
    } else {
        s.hover(HOVER)
    }
}

fn set_segment(ui: &mut Ui, seg: NodeId, on: bool) {
    ui.set_style(seg, segment_style(on, false));
    for child in ui.children(seg) {
        ui.restyle(child, |s| s.text_color(if on { ACCENT } else { LABEL }));
    }
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
    let brand = ui.add(
        bar,
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
        "runity",
    );
    let mut menus = Vec::new();
    for (title, items) in menu::menu_bar() {
        let m = ui.add(
            bar,
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
    separator(ui, bar);
    let doc = ui.add(
        bar,
        Style::row().gap(SPACE_2).center_items().min_width(120.0),
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

    let seg = ui.add(
        bar,
        Style::row()
            .height(26.0)
            .fixed()
            .radius(RADIUS_MD)
            .border(1.0, DIVIDER)
            .clip(),
    );
    let mut tools = [seg; 3];
    for (i, (glyph, word)) in [
        ("move-3d", "Move"),
        ("rotate-3d", "Rotate"),
        ("scale-3d", "Scale"),
    ]
    .into_iter()
    .enumerate()
    {
        let opt = ui.add(seg, segment_style(i == 0, i == 0));
        ui.set_name(opt, format!("tool {word}"));
        icon(ui, opt, glyph, LABEL);
        ui.add_text(
            opt,
            Style::default().text_size(12.0).text_color(LABEL).nowrap(),
            word,
        );
        tools[i] = opt;
    }
    separator(ui, bar);
    let space = icon_button(ui, bar, "space", "globe", false);
    let pivot = icon_button(ui, bar, "pivot", "crosshair", false);
    let grid = icon_button(ui, bar, "grid", "grid-3x3", true);
    spacer(ui, bar);
    let play_group = ui.add(
        bar,
        Style::row()
            .gap(SPACE_1)
            .padding(2.0)
            .radius(RADIUS_MD)
            .border(1.0, DIVIDER)
            .center_items(),
    );
    let play = icon_button(ui, play_group, "play", "play", false);
    let pause = icon_button(ui, play_group, "pause", "pause", false);
    let step = icon_button(ui, play_group, "step", "step-forward", false);
    spacer(ui, bar);
    let undo = icon_button(ui, bar, "undo", "undo-2", false);
    let redo = icon_button(ui, bar, "redo", "redo-2", false);
    separator(ui, bar);
    let save = button(ui, bar, "save", "Save", false);
    (
        Toolbar {
            menus,
            scene_name,
            modified,
            tools,
            space,
            pivot,
            grid,
            play,
            pause,
            step,
            play_group,
            undo,
            redo,
            save,
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
    let entities = ui.add_text(bar, small(), "");
    let selected = ui.add_text(bar, small(), "");
    let last = ui.add_text(bar, small(), "");
    spacer(ui, bar);
    // Warnings and errors: a click shows the Console, wherever it is.
    let problems = ui.add_text(bar, small().clickable(), "");
    ui.set_name(problems, "status problems");
    let fps = ui.add_text(bar, small(), "");
    let mode = ui.add_text(bar, small(), "");
    (
        Status {
            entities,
            selected,
            last,
            problems,
            mode,
        },
        fps,
    )
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

/// What a control does, for its tooltip — by the control's name.
fn tooltip(name: &str) -> Option<&'static str> {
    Some(match name {
        "tool Move" => "Move (W)",
        "tool Rotate" => "Rotate (E)",
        "tool Scale" => "Scale (R)",
        "space" => "Handles along the world's axes or the entity's own (X)",
        "pivot" => "Handles on the pivot or the selection's centre (Z)",
        "grid" => "Show the grid",
        "play" => "Play / Stop (Ctrl/Cmd P)",
        "pause" => "Pause (Ctrl/Cmd Shift P)",
        "step" => "One step (Ctrl/Cmd Alt P)",
        "undo" => "Undo (Ctrl/Cmd Z)",
        "redo" => "Redo (Ctrl/Cmd Shift Z)",
        "save" => "Save the scene (Ctrl/Cmd S)",
        "snap" => "Snap moves to ¼ m, turns to 15°, scale to 0.1",
        "colliders" => "Show colliders",
        "scene view" => "Shift Space: the view over the whole window",
        "sculpt" => "Terrain brush: left raises, Shift lowers, Ctrl/Cmd flattens; Alt still orbits",
        "faces" => "Face mode: drag a face of a box to push it; Alt still orbits",
        "view persp" => "Perspective view",
        "view top" => "Look down from above",
        "view front" => "Look from the front",
        "view right" => "Look from the right",
        "view scene" => "The Scene view: edit",
        "view game" => "The Game view: what the game's camera sees",
        "console clear" => "Clear the Console",
        "status problems" => "Show the Console",
        _ => return None,
    })
}

/// How big the orientation gizmo is, a side.
const COMPASS: f32 = 84.0;

struct Compass {
    /// +X, +Y, +Z, −X, −Y, −Z, and the side each looks from.
    axes: Vec<(NodeId, runity_editor::Side)>,
    middle: NodeId,
    seen: Option<(runity::glam::Vec3, runity::glam::Vec3, bool)>,
}

/// Unity's scene gizmo: the axes as the camera sees them, in the view's
/// top right corner. A click on an axis looks from it; the label under it
/// switches perspective and orthographic.
fn build_compass(ui: &mut Ui, frame: NodeId) -> Compass {
    use runity_editor::Side;
    let holder = ui.add(
        frame,
        Style::row()
            .absolute(0.0, 0.0)
            .full_width()
            .height(COMPASS + 30.0),
    );
    let pad = ui.add(holder, Style::row().fill());
    let _ = pad;
    let dial = ui.add(
        holder,
        Style::row().size(COMPASS, COMPASS + 22.0).margin(4.0),
    );
    ui.set_layer(dial, true);
    ui.set_name(dial, "compass");
    let colors = [
        runity_ui::Color::hex(0xe5736f),
        runity_ui::Color::hex(0x8cc26b),
        runity_ui::Color::hex(0x6f9be5),
    ];
    let middle = ui.add(
        dial,
        Style::row()
            .absolute(COMPASS / 2.0 - 18.0, COMPASS + 2.0)
            .size(36.0, 18.0)
            .radius(9.0)
            .center()
            .background(NEUTRAL_900.alpha(80))
            .hover(NEUTRAL_800),
    );
    ui.set_name(middle, "compass middle");
    // Under the dial, as Unity's Persp label: an axis pointing at the
    // camera sits in the middle and would cover it there.
    ui.add_text(
        middle,
        Style::default().text_size(9.5).text_color(LABEL).nowrap(),
        "persp",
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
        let size = if positive { 20.0 } else { 14.0 };
        let color = colors[i % 3];
        let dot = ui.add(
            dial,
            Style::row()
                .absolute(0.0, 0.0)
                .size(size, size)
                .radius(size / 2.0)
                .center()
                .background(if positive { color } else { color.alpha(35) })
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
        middle,
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
