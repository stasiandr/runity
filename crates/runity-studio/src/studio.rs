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
use crate::hierarchy::Hierarchy;
use crate::inspector::Inspector;
use crate::menu::{self, Action, MenuItem};
use crate::theme::*;

/// The engine's reference scene: every builtin, no import step.
pub const REFERENCE_SCENE: &str = "examples/valley/scenes/first-light.ron";

/// The Scene view's picture, as the renderer knows it.
const SCENE: ImageId = ImageId(0);

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
    steps: usize,
    redo: Option<String>,
    entities: usize,
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
            steps: session.undo_steps().len(),
            redo: session.redo_label(),
            entities: session.entity_count(),
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

        session.set_readback(false);
        let mut studio = Self {
            session,
            ui,
            scene_input: Input::new(),
            viewport,
            view_frame,
            tab_scene,
            tab_game,
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
            clipboard: SystemClipboard::new(),
        };
        studio.ui.set_clipboard(Box::new(SystemClipboard::new()));
        studio.ui.focus(Some(viewport));
        studio.refresh();
        studio
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
        self.ui.handle(event);
        let over_view = self.popup.is_none() && self.ui.hovered() == Some(self.viewport);
        let typing = self.ui.focused().is_some_and(|f| self.ui.is_field(f));
        let scale = self.ui.viewport().2;
        let view = self.ui.rect(self.viewport);
        let to_view = |x: f32, y: f32| ((x - view.x) * scale, (y - view.y) * scale);
        match event {
            InputEvent::MouseMoved { x, y } => {
                let (x, y) = to_view(*x, *y);
                self.scene_input.handle(&InputEvent::MouseMoved { x, y });
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
        let _ = self.session.scene_view(&self.scene_input, dt);
        self.scene_input.begin_frame();
        self.session.render();

        let stamp = Stamp::of(&self.session);
        let moving = self.session.is_dragging() || self.session.is_playing();
        if self.seen.as_ref() != Some(&stamp) || moving {
            let errors_before = self.seen.as_ref().map_or(0, |s| s.console.2);
            if stamp.console.2 > errors_before {
                self.bottom.show_console(&mut self.ui);
            }
            self.seen = Some(stamp);
            self.update_panels(moving);
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
        if !moving {
            self.hierarchy.update(&mut self.ui, s);
            self.bottom.update(&mut self.ui, s);
        }
        self.inspector.update(&mut self.ui, s);
        self.update_toolbar();
        self.update_status();
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
        if node == self.tab_scene || node == self.tab_game {
            requests.action = Some(Action::GameView(node == self.tab_game));
            requests.keyboard_to_scene = true;
            return true;
        }
        let t = &self.toolbar;
        if let Some((_, items)) = t.menus.iter().find(|(n, _)| *n == node) {
            let r = self.ui.rect(node);
            requests.menu = Some((items.clone(), r.x, r.y + r.height + 4.0));
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
            Asset::Model(name) | Asset::Prefab(name) => s.drop_asset(&name, x, y).map(|_| ()),
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
                Action::Save => {
                    s.save_scene(None).map_err(e)?;
                    s.say(Level::Info, "saved");
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
                Action::SetField(field, value) => {
                    self.inspector.set_field(s, &field, &value);
                }
                Action::Place(name) => {
                    let (w, h) = s.size();
                    s.drop_asset(&name, w / 2, h / 2).map_err(e)?;
                }
                Action::ClearConsole => s.clear_console(),
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
