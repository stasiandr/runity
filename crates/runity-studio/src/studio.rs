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

use runity::gizmo::Tool;
use runity::input::{Input, InputEvent, Key, MouseButton};
use runity_editor::console::Level;
use runity_editor::{Pivot, Session, Space};
use runity_ui::render::UiRenderer;
use runity_ui::{Event, ImageId, NodeId, Style, Ui};

use runity_ui::Clipboard as _;

use crate::bottom::{Asset, Bottom};
use crate::clipboard::SystemClipboard;
use crate::dock::{Docked, Docks, Panel};
use crate::hierarchy::Hierarchy;
use crate::inspector::Inspector;
use crate::menu::{self, Action, MenuItem};
use crate::theme::*;

/// The engine's reference scene: every builtin, no import step.
pub const REFERENCE_SCENE: &str = "examples/valley/scenes/first-light.ron";

/// The Scene view's picture, as the renderer knows it.
const SCENE: ImageId = ImageId(0);

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
    /// When the last event came in: the window draws at full rate for a
    /// while after one.
    last_input: Instant,
    /// Which of the Hierarchy, the Inspector and the lower panel show, and
    /// whether the view has the whole window.
    panels: [bool; 3],
    maximized: bool,
    /// The terrain brush is on: a left drag in the view shapes the ground.
    sculpt: bool,
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
    left: NodeId,
    right: NodeId,
    lower: NodeId,
    popup: Option<Popup>,
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
            panels: [true; 3],
            last_input: Instant::now(),
            docks,
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
            left,
            right,
            lower,
            popup: None,
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
        let t3 = Instant::now();

        self.poll_disk();
        self.bottom.update_git(&mut self.ui, &mut self.session);
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

        let t4 = Instant::now();
        let stamp = Stamp::of(&self.session);
        let t5 = Instant::now();
        let moving = self.session.is_dragging() || self.session.is_playing();
        if self.seen.as_ref() != Some(&stamp) || moving {
            let errors_before = self.seen.as_ref().map_or(0, |s| s.console.2);
            if stamp.console.2 > errors_before {
                self.docks.activate(&mut self.ui, Panel::Console);
                self.sync_visible();
            }
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
            self.ui.rect(self.lower).height.round(),
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
    fn poll_disk(&mut self) {
        if self.polled.elapsed().as_secs_f32() < 0.5 {
            return;
        }
        self.polled = Instant::now();
        self.save_layout();
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
        for (image, size, pixels) in self.pending_images.drain(..) {
            renderer.set_image_rgba(self.session.gpu(), image, size, size, &pixels);
        }
        let gpu = self.session.gpu();
        renderer.draw(gpu, view, width, height, &mut self.ui, Some(BG));
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
        } else if self.bottom.owns(&self.ui, node) {
            self.bottom
                .event(&mut self.ui, &mut self.session, node, event, requests);
        }
    }

    fn toolbar_event(&mut self, node: NodeId, event: &Event, requests: &mut Requests) -> bool {
        let Event::Click { .. } = event else {
            return false;
        };
        if node == self.prefab_back {
            requests.action = Some(Action::ExitPrefab);
            return true;
        }
        if let Some((_, action)) = self.buttons.iter().find(|(n, _)| *n == node) {
            requests.action = Some(action.clone());
            requests.keyboard_to_scene = true;
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
                Action::NewScene => {
                    let name = format!(
                        "scene-{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs())
                            % 100_000
                    );
                    let path = s.new_scene(&name).map_err(e)?;
                    s.say(Level::Info, format!("new scene {}", path.display()));
                }
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
                Action::ToggleSculpt => {
                    self.sculpt = !self.sculpt;
                    if self.sculpt {
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
    let problems = ui.add_text(bar, small(), "");
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
        "view persp" => "Perspective view",
        "view top" => "Look down from above",
        "view front" => "Look from the front",
        "view right" => "Look from the right",
        "view scene" => "The Scene view: edit",
        "view game" => "The Game view: what the game's camera sees",
        "console clear" => "Clear the Console",
        _ => return None,
    })
}
