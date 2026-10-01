//! Animation: a clip's tracks over time, to look at and to change — Unity's
//! Animation window, Dopesheet and Curves.
//!
//! The window keeps to the entity with the `animator` at or above the
//! selection, as Unity's keeps to the Animator: selecting a door's hinge
//! shows the door's clips, with the hinge's tracks the ones a key goes to.
//! On the left are the clips — the model's, and the ones made here —
//! and buttons to make one; over the tracks, the time ruler, pressed and
//! dragged to scrub. The entity in the view is posed where the playhead is
//! (in the world only: [`scrap_editor::clips`]).
//!
//! **Dopesheet**: a row a track, under a row for each thing or bone that
//! sums its tracks; a key is a diamond. Click a key to choose it, Shift for
//! more, drag along the row to move what is chosen (to whole frames; Alt
//! for between), double-click a row to key it there. **Curves**: the chosen
//! rows' values over time, keys as dots to drag in time and value, the
//! chosen key's slopes as handles. The buttons over the tracks shape the
//! chosen keys — Auto, Flat, Linear, Step — as Unity's tangent menu does.
//!
//! **Key** (K) keys the chosen rows at the playhead; with no row chosen,
//! every track of the selected thing — and a thing with none yet, its
//! place and turn. **Record** keys what is moved in the view: while it is
//! on, moving a thing under the entity is not an edit of the scene but
//! keys at the playhead, as Unity's red record button does.
//!
//! Every change is written at once to the clip's file, only where it
//! changed; Cmd-Z over the window takes back the window's last change.

use std::collections::BTreeSet;
use std::time::Instant;

use scrap::keys::Blend;
use scrap::motion::{Key as ClipKey, Property, Tangent, Track};
use scrap::EntityId;
use scrap_editor::clips::{ClipDoc, ClipKind};
use scrap_editor::console::Level;
use scrap_editor::Session;
use scrap_ui::{Color, Drawing, Event, Mark, MarkShape, NodeId, Polyline, Style, Ui};

use crate::theme::*;

/// The frames keys snap to, a second.
const FPS: f32 = 30.0;
const NAMES_W: f32 = 210.0;
const ROW_H: f32 = 20.0;
const RULER_H: f32 = 22.0;
/// How near, in pixels, a press must be to a key to take it.
const NEAR: f32 = 7.0;
/// The tween slider's room past each end, for the overshoot.
const TWEEN_PAD: f32 = 46.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Dopesheet,
    Curves,
}

/// What a row of the dopesheet is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RowKind {
    /// Every key of the clip.
    Summary,
    /// A thing's or a bone's tracks, folded or not.
    Group(String),
    Track(String, Property),
}

/// A key, by its track and its time: what stays true of it as keys around
/// it come and go.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
struct KeyRef {
    path: String,
    what: Property,
    time: f32,
}

#[derive(Debug, Clone)]
enum Drag {
    Scrub,
    /// Keys moved in time from where the drag began, in the clip as it was.
    Keys { from: ClipDoc, chosen: Vec<KeyRef>, x0: f32, moved: bool },
    /// A key in Curves, in time and value.
    Point { from: ClipDoc, key: KeyRef, x0: f32, y0: f32 },
    /// A chosen key's slope, by its handle: `out` the right one.
    Handle { from: ClipDoc, key: KeyRef, out: bool },
    Pan,
}

pub struct Animation {
    pub root: NodeId,
    title: NodeId,
    rec: NodeId,
    play: NodeId,
    first: NodeId,
    back: NodeId,
    on: NodeId,
    last: NodeId,
    time_field: NodeId,
    length_field: NodeId,
    sheet_button: NodeId,
    curves_button: NodeId,
    key_button: NodeId,
    shapes: Vec<(NodeId, Tangent)>,
    undo_button: NodeId,
    stop: NodeId,
    clips_list: NodeId,
    new_name: NodeId,
    new_clip: NodeId,
    new_rig: NodeId,
    ruler: NodeId,
    rows: NodeId,
    curves: NodeId,
    curve_names: NodeId,
    adds: NodeId,
    add_filter: NodeId,
    add_toggle: NodeId,
    adding: bool,
    /// The seconds written over the ruler, and the times they say.
    ruler_labels: Vec<(NodeId, f32)>,
    tween: NodeId,
    tween_label: NodeId,
    tween_modes: Vec<(NodeId, Blend)>,
    tween_how: Blend,
    /// While the slider is held: the clip as it was, and per track the
    /// chosen keys' places (and, for Default, the values without the
    /// track at their times).
    #[allow(clippy::type_complexity)]
    tweening: Option<(ClipDoc, Vec<(String, Property, Vec<usize>, Vec<(f32, f32)>)>)>,
    tween_at: f32,
    /// The press made the keys it moves: their making is the step back.
    tween_made: bool,
    /// The clips shown on the left, by row.
    clip_rows: Vec<(NodeId, String, ClipKind)>,
    /// The dopesheet's rows: the row, its lane, what it is.
    row_nodes: Vec<(NodeId, NodeId, RowKind)>,
    add_rows: Vec<(NodeId, String, Vec<Property>)>,
    /// The entity shown, and the selection's path under it.
    showing: Option<EntityId>,
    part: String,
    selected: Option<EntityId>,
    doc: Option<ClipDoc>,
    time: f32,
    playing: bool,
    ticked: Option<Instant>,
    recording: bool,
    /// The places of the things under the entity when recording began,
    /// or since the last key it made.
    recorded: Vec<(String, scrap::Transform)>,
    mode: Mode,
    open_groups: BTreeSet<String>,
    chosen_rows: BTreeSet<RowKind>,
    chosen: Vec<KeyRef>,
    /// Seconds at the lane's left edge and pixels a second.
    view: (f32, f32),
    /// Curves' value at the bottom and at the top; `None` frames them.
    range: Option<(f32, f32)>,
    drag: Option<Drag>,
    undo: Vec<ClipDoc>,
    redo: Vec<ClipDoc>,
    /// Shown in the view as the window has it: posed again when not.
    posed: Option<(f32, u64)>,
    version: u64,
    /// What the rows were built for.
    built: Option<(Vec<RowKind>, Mode)>,
    clipboard: Vec<(String, Property, f32, f32, Tangent)>,
}

fn small_field(ui: &mut Ui, parent: NodeId, name: &str, width: f32) -> NodeId {
    let f = ui.add_field(parent, field_style().width(width).fixed(), "");
    ui.set_name(f, name);
    f
}

impl Animation {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let root = ui.add(parent, Style::column().fill().full_width().gap(SPACE_1));
        ui.set_name(root, "animation");
        // The toolbar.
        let bar = ui.add(root, Style::row().full_width().height(30.0).fixed().gap(SPACE_1).padding_x(SPACE_2).center_items());
        let rec = icon_button(ui, bar, "animation record", "circle-dot", false);
        separator(ui, bar);
        let first = icon_button(ui, bar, "animation first", "skip-back", false);
        let back = icon_button(ui, bar, "animation back", "chevron-left", false);
        let play = icon_button(ui, bar, "animation play", "play", false);
        let on = icon_button(ui, bar, "animation on", "chevron-right", false);
        let last = icon_button(ui, bar, "animation last", "skip-forward", false);
        let time_field = small_field(ui, bar, "animation time", 64.0);
        ui.add_text(bar, caption(), "of");
        let length_field = small_field(ui, bar, "animation length", 56.0);
        separator(ui, bar);
        let sheet_button = button(ui, bar, "animation dopesheet", "Dopesheet", true);
        let curves_button = button(ui, bar, "animation curves", "Curves", false);
        separator(ui, bar);
        let key_button = button(ui, bar, "animation key", "Key", false);
        let mut shapes = Vec::new();
        for (label, t) in [("Auto", Tangent::Auto), ("Flat", Tangent::Flat), ("Linear", Tangent::Linear), ("Step", Tangent::Step)] {
            let b = button(ui, bar, &format!("tangent {}", label.to_lowercase()), label, false);
            shapes.push((b, t));
        }
        let undo_button = icon_button(ui, bar, "animation undo", "undo-2", false);
        let title = ui.add_text(bar, text().fill().text_color(LABEL), "");
        ui.set_name(title, "animation title");
        let stop = button(ui, bar, "animation stop", "Stop", false);

        // The tween slider and what it does.
        let tweens = ui.add(root, Style::row().full_width().height(26.0).fixed().gap(SPACE_1).padding_x(SPACE_2).center_items());
        ui.add_text(tweens, caption(), "BLEND");
        let mut tween_modes = Vec::new();
        for (label, how) in [("Tween", Blend::Tween), ("Neighbor", Blend::Neighbor), ("Push", Blend::Push), ("Default", Blend::Default)] {
            let b = button(ui, tweens, &format!("blend {}", label.to_lowercase()), label, how == Blend::Tween);
            ui.restyle(b, |s| s.height(22.0));
            tween_modes.push((b, how));
        }
        let tween = ui.add(tweens, Style::row().width(280.0).fixed().height(22.0).clickable().draggable());
        ui.set_name(tween, "tween slider");
        let tween_label = ui.add_text(tweens, text().width(50.0).fixed().text_color(ACCENT), "");
        ui.set_name(tween_label, "tween amount");
        let body = ui.add(root, Style::row().fill().full_width());
        // The clips.
        let left = ui.add(body, Style::column().width(170.0).fixed().full_height().padding(SPACE_2).gap(SPACE_1));
        ui.add_text(left, caption(), "CLIPS");
        let clips_list = ui.add(left, Style::column().fill().full_width().gap(1.0).clip());
        ui.set_name(clips_list, "animation clips");
        let new_name = small_field(ui, left, "new clip name", 150.0);
        ui.set_placeholder(new_name, "name of a new clip");
        let new_row = ui.add(left, Style::row().full_width().gap(SPACE_1));
        let new_clip = button(ui, new_row, "new clip", "New clip", false);
        let new_rig = button(ui, new_row, "new rig clip", "Bones", false);

        // The tracks.
        let main = ui.add(body, Style::column().fill().full_height().clip());
        let head = ui.add(main, Style::row().full_width().height(RULER_H).fixed());
        let corner = ui.add(head, Style::row().width(NAMES_W).fixed().full_height().padding_x(SPACE_2).center_items());
        ui.add_text(corner, caption().fill(), "TRACKS");
        let add_toggle = button(ui, corner, "animation add", "+ Track", false);
        ui.restyle(add_toggle, |s| s.height(18.0));
        let ruler = ui.add(head, Style::row().fill().full_height().background(BG).clickable().draggable().clip());
        ui.set_name(ruler, "animation ruler");
        let rows = ui.add(main, Style::column().fill().full_width().clip());
        ui.set_name(rows, "animation rows");
        let curves_row = ui.add(main, Style::row().fill().full_width().hidden());
        let curve_names = ui.add(curves_row, Style::column().width(NAMES_W).fixed().full_height().clip());
        let curves = ui.add(curves_row, Style::column().fill().full_height().background(BG).clickable().draggable().clip());
        ui.set_name(curves, "animation curve view");
        // Adding a track.
        let adds = ui.add(main, Style::column().full_width().max_height(160.0).padding_x(SPACE_2).gap(1.0).clip().hidden());
        let add_bar = ui.add(adds, Style::row().full_width().gap(SPACE_2).center_items());
        ui.add_text(add_bar, caption(), "ADD");
        let add_filter = small_field(ui, add_bar, "animation add filter", 160.0);
        ui.set_placeholder(add_filter, "thing or bone");
        let me = Self {
            root,
            title,
            rec,
            play,
            first,
            back,
            on,
            last,
            time_field,
            length_field,
            sheet_button,
            curves_button,
            key_button,
            shapes,
            undo_button,
            stop,
            clips_list,
            new_name,
            new_clip,
            new_rig,
            ruler,
            rows,
            curves,
            curve_names,
            adds,
            add_filter,
            add_toggle,
            adding: false,
            ruler_labels: Vec::new(),
            tween,
            tween_label,
            tween_modes,
            tween_how: Blend::Tween,
            tweening: None,
            tween_at: 0.0,
            tween_made: false,
            clip_rows: Vec::new(),
            row_nodes: Vec::new(),
            add_rows: Vec::new(),
            showing: None,
            part: String::new(),
            selected: None,
            doc: None,
            time: 0.0,
            playing: false,
            ticked: None,
            recording: false,
            recorded: Vec::new(),
            mode: Mode::Dopesheet,
            open_groups: BTreeSet::new(),
            chosen_rows: BTreeSet::new(),
            chosen: Vec::new(),
            view: (0.0, 200.0),
            range: None,
            drag: None,
            undo: Vec::new(),
            redo: Vec::new(),
            posed: None,
            version: 0,
            built: None,
            clipboard: Vec::new(),
        };
        me.show_time(ui);
        me
    }

    pub fn owns(&self, ui: &Ui, node: NodeId) -> bool {
        let mut at = Some(node);
        while let Some(n) = at {
            if n == self.root {
                return true;
            }
            at = ui.parent(n);
        }
        false
    }

    // --- every frame -----------------------------------------------------

    pub fn update(&mut self, ui: &mut Ui, session: &mut Session) {
        let selected = session.selected();
        if selected != self.selected {
            self.selected = selected;
            let root = selected.map(|id| session.clip_root(id));
            let entity = root.as_ref().map(|r| r.0);
            self.part = root.map(|r| r.1).unwrap_or_default();
            if entity != self.showing {
                self.show_entity(ui, session, entity);
            }
            self.list_adds(ui, session);
        }
        if self.playing {
            let now = Instant::now();
            let dt = self.ticked.map_or(0.0, |t| now.duration_since(t).as_secs_f32());
            self.ticked = Some(now);
            let length = self.doc.as_ref().map_or(1.0, |d| d.length).max(1e-3);
            self.time = (self.time + dt).rem_euclid(length);
            self.show_time(ui);
        }
        if self.recording {
            self.record(ui, session);
        }
        self.pose(session);
        self.draw(ui);
    }

    fn show_entity(&mut self, ui: &mut Ui, session: &mut Session, entity: Option<EntityId>) {
        session.stop_posing();
        self.showing = entity;
        self.doc = None;
        self.chosen.clear();
        self.chosen_rows.clear();
        self.open_groups.clear();
        self.undo.clear();
        self.redo.clear();
        self.playing = false;
        self.recording = false;
        self.time = 0.0;
        self.built = None;
        self.list_clips(ui, session);
        if let Some((_, name, _)) = self.clip_rows.first().cloned() {
            self.open_clip(ui, session, &name);
        }
        self.show_time(ui);
        self.mark(ui);
    }

    fn list_clips(&mut self, ui: &mut Ui, session: &Session) {
        ui.clear(self.clips_list);
        self.clip_rows.clear();
        let Some(id) = self.showing else {
            ui.set_text(self.title, "Select something to animate.");
            return;
        };
        let name = session.entity_name(id).unwrap_or_default();
        ui.set_text(self.title, &name);
        for (clip, kind) in session.clip_list(id) {
            let row = ui.add(
                self.clips_list,
                Style::row().full_width().height(22.0).fixed().padding_x(SPACE_2).gap(SPACE_2).center_items().radius(RADIUS_SM).hover(HOVER),
            );
            ui.set_name(row, format!("clip {clip}"));
            let glyph = match kind {
                ClipKind::Things => "box",
                ClipKind::Rig => "bone",
                ClipKind::Model => "file-box",
            };
            icon(ui, row, glyph, MUTED);
            ui.add_text(row, text().fill(), &clip);
            self.clip_rows.push((row, clip, kind));
        }
    }

    fn open_clip(&mut self, ui: &mut Ui, session: &mut Session, name: &str) {
        let Some(id) = self.showing else { return };
        match session.open_clip(id, name) {
            Ok(doc) => {
                if self.doc.as_ref().map(|d| &d.name) != Some(&doc.name) {
                    self.undo.clear();
                    self.redo.clear();
                    self.chosen.clear();
                    self.chosen_rows.clear();
                    self.range = None;
                    self.time = self.time.min(doc.length);
                }
                self.doc = Some(doc);
                self.version += 1;
                self.built = None;
                self.frame_all(ui);
            }
            Err(e) => session.say(Level::Error, e.to_string()),
        }
        self.show_time(ui);
        self.mark(ui);
    }

    /// The entity posed where the playhead is, when it is not already.
    fn pose(&mut self, session: &mut Session) {
        let (Some(id), Some(doc)) = (self.showing, &self.doc) else { return };
        if session.is_playing() {
            return;
        }
        let shown = session.previewing().contains(&id);
        if shown && self.posed == Some((self.time, self.version)) {
            return;
        }
        if let Err(e) = session.pose_clip(id, doc, self.time) {
            session.say(Level::Error, e.to_string());
            self.doc = None;
            return;
        }
        self.posed = Some((self.time, self.version));
    }

    // --- drawing -----------------------------------------------------------

    fn x_of(&self, t: f32) -> f32 {
        (t - self.view.0) * self.view.1
    }

    fn t_of(&self, x: f32) -> f32 {
        x / self.view.1 + self.view.0
    }

    /// The rows the clip has, as they are folded.
    fn row_kinds(&self) -> Vec<RowKind> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let mut out = vec![RowKind::Summary];
        let mut paths: Vec<&str> = Vec::new();
        for t in &doc.tracks {
            if !paths.contains(&t.path.as_str()) {
                paths.push(&t.path);
            }
        }
        for p in paths {
            out.push(RowKind::Group(p.to_string()));
            if self.open_groups.contains(p) {
                for t in doc.tracks.iter().filter(|t| t.path == p) {
                    out.push(RowKind::Track(p.to_string(), t.what));
                }
            }
        }
        out
    }

    fn build_rows(&mut self, ui: &mut Ui) {
        let kinds = self.row_kinds();
        if self.built.as_ref() == Some(&(kinds.clone(), self.mode)) {
            return;
        }
        self.built = Some((kinds.clone(), self.mode));
        ui.clear(self.rows);
        ui.clear(self.curve_names);
        self.row_nodes.clear();
        let curves = self.mode == Mode::Curves;
        ui.restyle(self.rows, |s| if curves { s.hidden() } else { s.shown() });
        if let Some(row) = ui.parent(self.curves) {
            ui.restyle(row, |s| if curves { s.shown() } else { s.hidden() });
        }
        for kind in kinds {
            let parent = if curves { self.curve_names } else { self.rows };
            let row = ui.add(parent, Style::row().full_width().height(ROW_H).fixed().clickable().hover(HOVER));
            let name = ui.add(row, Style::row().width(NAMES_W).fixed().full_height().padding_x(SPACE_2).gap(SPACE_1).center_items());
            let (label, indent, color, row_name) = match &kind {
                RowKind::Summary => ("Summary".to_string(), 0.0, LABEL, "row summary".to_string()),
                RowKind::Group(p) => {
                    let open = self.open_groups.contains(p);
                    icon(ui, name, if open { "chevron-down" } else { "chevron-right" }, MUTED);
                    let shown = if p.is_empty() { "(itself)".to_string() } else { p.clone() };
                    (shown, 0.0, TEXT, format!("row {p}"))
                }
                RowKind::Track(p, what) => (format!("{what:?}"), 18.0, axis_color(*what), format!("row {p} {what:?}")),
            };
            ui.restyle(name, |s| s.padding_left(SPACE_2 + indent));
            ui.add_text(name, text().fill().text_color(color), &label);
            ui.set_name(row, row_name);
            let lane = if curves {
                row
            } else {
                let lane = ui.add(row, Style::row().fill().full_height().clickable().draggable().clip());
                ui.set_name(lane, format!("lane {}", row_label(&kind)));
                lane
            };
            self.row_nodes.push((row, lane, kind));
        }
    }

    fn draw(&mut self, ui: &mut Ui) {
        self.build_rows(ui);
        self.draw_tween(ui);
        let Some(doc) = self.doc.clone() else {
            for (_, lane, _) in &self.row_nodes {
                ui.set_drawing(*lane, Drawing::default());
            }
            ui.set_drawing(self.ruler, Drawing::default());
            ui.set_drawing(self.curves, Drawing::default());
            return;
        };
        let ruler = ui.rect(self.ruler);
        // The ruler: a tick a frame where they fit, a second taller.
        let mut lines = Vec::new();
        let step = if self.view.1 / FPS >= 6.0 { 1.0 / FPS } else if self.view.1 >= 40.0 { 0.1 } else { 1.0 };
        let first = (self.view.0 / step).floor() as i64;
        let last = (self.t_of(ruler.width) / step).ceil() as i64;
        for i in first..=last.min(first + 2000) {
            let t = i as f32 * step;
            let x = self.x_of(t);
            let whole = (t - t.round()).abs() < 1e-4;
            let h = if whole { RULER_H } else { RULER_H * 0.35 };
            lines.push(line(x, RULER_H - h, x, RULER_H, if whole { LABEL } else { MUTED }, 1.0));
        }
        // Past the end, dimmed.
        let end = self.x_of(doc.length);
        lines.push(line(end, 0.0, end, RULER_H, WARNING, 1.0));
        let head = self.x_of(self.time);
        lines.push(line(head, 0.0, head, RULER_H, ACCENT, 2.0));
        ui.set_drawing(self.ruler, Drawing { lines, marks: Vec::new() });
        self.label_ruler(ui, ruler.width);
        if self.mode == Mode::Curves {
            self.draw_curves(ui, &doc);
            for (row, _, kind) in self.row_nodes.clone() {
                let on = self.chosen_rows.contains(&kind);
                ui.restyle(row, |s| s.background(if on { ACCENT_900 } else { Color::TRANSPARENT }));
            }
            return;
        }
        for (row, lane, kind) in self.row_nodes.clone() {
            let rect = ui.rect(lane);
            let mut marks = Vec::new();
            let keys: Vec<(f32, bool)> = match &kind {
                RowKind::Summary => {
                    let mut times: Vec<f32> = doc.tracks.iter().flat_map(|t| t.keys.iter().map(|k| k.time)).collect();
                    times.sort_by(f32::total_cmp);
                    times.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
                    times.into_iter().map(|t| (t, self.chosen.iter().any(|c| (c.time - t).abs() < 1e-4))).collect()
                }
                RowKind::Group(p) => {
                    let mut times: Vec<f32> = doc.tracks.iter().filter(|t| t.path == *p).flat_map(|t| t.keys.iter().map(|k| k.time)).collect();
                    times.sort_by(f32::total_cmp);
                    times.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
                    times.into_iter().map(|t| (t, self.chosen.iter().any(|c| c.path == *p && (c.time - t).abs() < 1e-4))).collect()
                }
                RowKind::Track(p, what) => doc
                    .track(p, *what)
                    .map(|t| t.keys.iter().map(|k| (k.time, self.is_chosen(p, *what, k.time))).collect())
                    .unwrap_or_default(),
            };
            let written = match &kind {
                RowKind::Track(p, w) => doc.kind != ClipKind::Model || doc.is_written(p, *w),
                RowKind::Group(p) => doc.kind != ClipKind::Model || doc.motion.tracks.iter().any(|t| t.path == *p),
                RowKind::Summary => true,
            };
            let fill = if written { NEUTRAL_300 } else { NEUTRAL_500 };
            for (t, on) in keys {
                let x = self.x_of(t);
                if x < -NEAR || x > rect.width + NEAR {
                    continue;
                }
                marks.push(Mark {
                    x,
                    y: ROW_H / 2.0,
                    size: if matches!(kind, RowKind::Track(..)) { 10.0 } else { 9.0 },
                    shape: MarkShape::Diamond,
                    fill: if on { ACCENT } else { fill },
                    border: BG,
                });
            }
            let mut lines = vec![line(self.x_of(self.time), 0.0, self.x_of(self.time), ROW_H, ACCENT, 1.5)];
            lines.push(line(end, 0.0, end, ROW_H, WARNING.alpha(60), 1.0));
            ui.set_drawing(lane, Drawing { lines, marks });
            let on = self.chosen_rows.contains(&kind);
            ui.restyle(row, |s| s.background(if on { ACCENT_900 } else { Color::TRANSPARENT }));
        }
    }

    /// A second's number over the ruler where there is room for it.
    fn label_ruler(&mut self, ui: &mut Ui, width: f32) {
        let step = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0].into_iter().find(|s| s * self.view.1 >= 48.0).unwrap_or(30.0);
        let first = (self.view.0 / step).ceil() as i64;
        let wanted: Vec<f32> = (first..).map(|i| i as f32 * step).take_while(|t| self.x_of(*t) < width - 20.0).take(40).collect();
        if self.ruler_labels.iter().map(|(_, t)| *t).collect::<Vec<_>>() == wanted {
            return;
        }
        for (n, _) in self.ruler_labels.drain(..) {
            ui.remove(n);
        }
        for t in wanted {
            let label = if step < 1.0 { format!("{t:.2}") } else { format!("{t:.0}") };
            let n = ui.add_text(self.ruler, caption().absolute(self.x_of(t) + 3.0, 1.0), &label);
            self.ruler_labels.push((n, t));
        }
    }

    fn is_chosen(&self, path: &str, what: Property, time: f32) -> bool {
        self.chosen.iter().any(|c| c.path == path && c.what == what && (c.time - time).abs() < 1e-4)
    }

    /// The tracks Curves shows: the chosen rows', a group for all of its;
    /// none chosen, every track of the open groups, or all of them.
    fn curve_tracks(&self, doc: &ClipDoc) -> Vec<Track> {
        let wanted = |t: &Track, rows: &BTreeSet<RowKind>| {
            rows.contains(&RowKind::Track(t.path.clone(), t.what)) || rows.contains(&RowKind::Group(t.path.clone())) || rows.contains(&RowKind::Summary)
        };
        let mut out: Vec<Track> = doc.tracks.iter().filter(|t| wanted(t, &self.chosen_rows)).cloned().collect();
        if out.is_empty() {
            out = doc.tracks.iter().filter(|t| self.open_groups.contains(&t.path)).cloned().collect();
        }
        if out.is_empty() {
            out = doc.tracks.clone();
        }
        out.truncate(48);
        out
    }

    fn value_range(&self, doc: &ClipDoc) -> (f32, f32) {
        if let Some(r) = self.range {
            return r;
        }
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for t in self.curve_tracks(doc) {
            for i in 0..=64 {
                if let Some(v) = t.sample(doc.length * i as f32 / 64.0) {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            for k in &t.keys {
                lo = lo.min(k.value);
                hi = hi.max(k.value);
            }
        }
        if lo > hi {
            return (-1.0, 1.0);
        }
        let pad = ((hi - lo) * 0.12).max(0.05);
        (lo - pad, hi + pad)
    }

    fn y_of(&self, v: f32, range: (f32, f32), height: f32) -> f32 {
        height - (v - range.0) / (range.1 - range.0).max(1e-6) * height
    }

    fn v_of(&self, y: f32, range: (f32, f32), height: f32) -> f32 {
        range.0 + (height - y) / height * (range.1 - range.0)
    }

    fn draw_curves(&mut self, ui: &mut Ui, doc: &ClipDoc) {
        let rect = ui.rect(self.curves);
        let range = self.value_range(doc);
        let mut lines = Vec::new();
        let mut marks = Vec::new();
        // Zero, and the clip's ends.
        let zero = self.y_of(0.0, range, rect.height);
        lines.push(line(0.0, zero, rect.width, zero, DIVIDER, 1.0));
        for t in [0.0, doc.length] {
            let x = self.x_of(t);
            lines.push(line(x, 0.0, x, rect.height, if t == 0.0 { DIVIDER } else { WARNING.alpha(60) }, 1.0));
        }
        for track in self.curve_tracks(doc) {
            let color = axis_color(track.what);
            let mut points = Vec::new();
            let mut x = 0.0;
            while x <= rect.width {
                if let Some(v) = track.sample(self.t_of(x)) {
                    points.push((x, self.y_of(v, range, rect.height)));
                }
                x += 2.0;
            }
            lines.push(Polyline { points, color, width: 1.5 });
            for (i, k) in track.keys.iter().enumerate() {
                let (x, y) = (self.x_of(k.time), self.y_of(k.value, range, rect.height));
                let on = self.is_chosen(&track.path, track.what, k.time);
                marks.push(Mark { x, y, size: 8.0, shape: MarkShape::Circle, fill: if on { ACCENT } else { color }, border: BG });
                if on && self.chosen.len() <= 4 {
                    let (s_in, s_out) = scrap::keys::slopes(&track.keys, i);
                    for (s, sign) in [(s_in, -1.0f32), (s_out, 1.0)] {
                        let (hx, hy) = self.handle_at(x, y, s, sign, range, rect.height);
                        lines.push(line(x, y, hx, hy, NEUTRAL_300, 1.0));
                        marks.push(Mark { x: hx, y: hy, size: 7.0, shape: MarkShape::Square, fill: NEUTRAL_300, border: BG });
                    }
                }
            }
        }
        let head = self.x_of(self.time);
        lines.push(line(head, 0.0, head, rect.height, ACCENT, 1.5));
        ui.set_drawing(self.curves, Drawing { lines, marks });
    }

    /// Where a slope's handle is drawn: 40 px out from its key, along it.
    fn handle_at(&self, x: f32, y: f32, slope: f32, sign: f32, range: (f32, f32), height: f32) -> (f32, f32) {
        let per_px_v = (range.1 - range.0) / height.max(1.0);
        let dx = 1.0;
        let dy = -slope / self.view.1 / per_px_v.max(1e-9);
        let len = (dx * dx + dy * dy).sqrt();
        (x + sign * 40.0 * dx / len, y + sign * 40.0 * dy / len)
    }

    fn show_time(&self, ui: &mut Ui) {
        let frame = (self.time * FPS).round() as i64;
        ui.set_text(self.time_field, &format!("{:.2}", self.time));
        let _ = frame;
        let length = self.doc.as_ref().map(|d| format!("{:.2}", d.length)).unwrap_or_default();
        ui.set_text(self.length_field, &length);
    }

    /// Buttons lit as the state is.
    fn mark(&self, ui: &mut Ui) {
        set_icon_button(ui, self.rec, "circle-dot", self.recording, self.doc.is_some());
        set_icon_button(ui, self.play, if self.playing { "pause" } else { "play" }, self.playing, self.doc.is_some());
        set_button_primary(ui, self.sheet_button, self.mode == Mode::Dopesheet);
        set_button_primary(ui, self.curves_button, self.mode == Mode::Curves);
        set_icon_button(ui, self.undo_button, "undo-2", false, !self.undo.is_empty());
        let open = self.doc.as_ref().map(|d| d.name.clone());
        for (row, name, _) in &self.clip_rows {
            let on = Some(name) == open.as_ref();
            ui.restyle(*row, |s| s.background(if on { ACCENT_900 } else { Color::TRANSPARENT }));
        }
        if self.recording {
            ui.restyle(self.rec, |s| s.background(ERROR.alpha(60)));
        }
    }

    fn frame_all(&mut self, ui: &Ui) {
        let width = ui.rect(self.ruler).width.max(200.0);
        let length = self.doc.as_ref().map_or(1.0, |d| d.length).max(0.1);
        self.view = (-length * 0.04, width / (length * 1.08));
        self.range = None;
    }

    // --- changes -----------------------------------------------------------

    /// Change the open clip as one step that can be taken back, and write
    /// it.
    fn change(&mut self, ui: &mut Ui, session: &mut Session, edit: impl FnOnce(&mut ClipDoc)) {
        let Some(doc) = self.doc.as_mut() else { return };
        let before = doc.clone();
        edit(doc);
        if *doc == before {
            return;
        }
        self.undo.push(before);
        self.redo.clear();
        self.write(ui, session);
    }

    fn write(&mut self, ui: &mut Ui, session: &mut Session) {
        let Some(doc) = self.doc.as_mut() else { return };
        let was_file = doc.file.is_some();
        if let Err(e) = session.save_clip(doc) {
            session.say(Level::Error, e.to_string());
        }
        self.version += 1;
        if !was_file && self.doc.as_ref().is_some_and(|d| d.file.is_some()) {
            let name = self.doc.as_ref().map(|d| d.name.clone()).unwrap_or_default();
            session.say(Level::Info, format!("clip `{name}`: changes written over the model's"));
        }
        self.show_time(ui);
        self.mark(ui);
    }

    fn take_back(&mut self, ui: &mut Ui, session: &mut Session, redo: bool) {
        let (from, to) = if redo { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        let Some(prev) = from.pop() else { return };
        if let Some(now) = self.doc.replace(prev) {
            to.push(now);
        }
        // The file's name is the clip's, whichever version: keep it.
        self.chosen.clear();
        self.write(ui, session);
    }

    /// Key `tracks` (path, property) at the playhead, each at the value it
    /// has there now.
    fn key_tracks(&mut self, ui: &mut Ui, session: &mut Session, tracks: Vec<(String, Property)>) {
        let (Some(id), Some(doc)) = (self.showing, &self.doc) else { return };
        let time = snap(self.time, false);
        let values: Vec<(String, Property, f32)> =
            tracks.into_iter().map(|(p, w)| { let v = session.current_value(id, doc, &p, w, time); (p, w, v) }).collect();
        self.chosen.clear();
        for (p, w, _) in &values {
            self.chosen.push(KeyRef { path: p.clone(), what: *w, time });
            self.open_groups.insert(p.clone());
        }
        self.change(ui, session, |doc| {
            for (path, what, value) in values {
                let mut track = doc.track(&path, what).cloned().unwrap_or(Track { path, what, keys: Vec::new(), ease: Default::default() });
                put_key(&mut track, ClipKey::shaped(time, value, Tangent::Auto));
                doc.set_track(track);
            }
        });
    }

    /// What a press of Key keys: the chosen rows, or the selected thing's
    /// tracks, or its place and turn.
    /// What Key keys: the chosen rows, or the selected thing's tracks, or
    /// its place and turn.
    fn key_targets(&self) -> Vec<(String, Property)> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let mut tracks: Vec<(String, Property)> = Vec::new();
        for row in &self.chosen_rows {
            match row {
                RowKind::Track(p, w) => tracks.push((p.clone(), *w)),
                RowKind::Group(p) => tracks.extend(doc.tracks.iter().filter(|t| t.path == *p).map(|t| (t.path.clone(), t.what))),
                RowKind::Summary => tracks.extend(doc.tracks.iter().map(|t| (t.path.clone(), t.what))),
            }
        }
        if tracks.is_empty() && doc.kind == ClipKind::Things {
            let part = self.part.clone();
            tracks = doc.tracks.iter().filter(|t| t.path == part).map(|t| (t.path.clone(), t.what)).collect();
            if tracks.is_empty() {
                tracks = TRANSFORM[..6].iter().map(|w| (part.clone(), *w)).collect();
            }
        }
        tracks.dedup();
        tracks
    }

    fn key_now(&mut self, ui: &mut Ui, session: &mut Session) {
        let tracks = self.key_targets();
        if tracks.is_empty() {
            session.say(Level::Info, "choose the rows to key, or add a track below");
            return;
        }
        self.key_tracks(ui, session, tracks);
    }

    // --- the tween slider ----------------------------------------------------

    /// The slider's amount under a point of it: its middle 0, its ends -1
    /// and 1, and past them up to 2 to overshoot.
    fn tween_amount(&self, ui: &Ui, x: f32) -> f32 {
        let r = ui.rect(self.tween);
        let half = (r.width / 2.0 - TWEEN_PAD).max(1.0);
        ((x - r.x - r.width / 2.0) / half).clamp(-2.0, 2.0)
    }

    /// The slider pressed: what it moves is the chosen keys, or — none
    /// chosen — a key it makes at the playhead on each track Key would key:
    /// a breakdown where there was none, as Tween Machine makes one.
    fn tween_start(&mut self, ui: &mut Ui, session: &mut Session, x: f32) {
        let (Some(id), Some(_)) = (self.showing, &self.doc) else { return };
        if self.chosen.is_empty() {
            let targets = self.key_targets();
            if targets.is_empty() {
                session.say(Level::Info, "choose keys, or the rows to make a breakdown in");
                return;
            }
            let before = self.undo.len();
            self.key_tracks(ui, session, targets);
            // The made keys and the slide are one step back.
            self.tween_made = self.undo.len() > before;
        } else {
            self.tween_made = false;
        }
        let Some(doc) = self.doc.clone() else { return };
        let mut targets = Vec::new();
        for t in &doc.tracks {
            let idx: Vec<usize> = t
                .keys
                .iter()
                .enumerate()
                .filter(|(_, k)| self.is_chosen(&t.path, t.what, k.time))
                .map(|(i, _)| i)
                .collect();
            if idx.is_empty() {
                continue;
            }
            let defaults: Vec<(f32, f32)> = if self.tween_how == Blend::Default {
                idx.iter().map(|&i| (t.keys[i].time, session.default_value(id, &doc, &t.path, t.what, t.keys[i].time))).collect()
            } else {
                Vec::new()
            };
            targets.push((t.path.clone(), t.what, idx, defaults));
        }
        self.tweening = Some((doc, targets));
        let amount = self.tween_amount(ui, x);
        self.tween_to(ui, amount);
    }

    fn tween_to(&mut self, ui: &mut Ui, amount: f32) {
        let Some((from, targets)) = &self.tweening else { return };
        let mut doc = from.clone();
        for (path, what, idx, defaults) in targets {
            let Some(t) = from.track(path, *what) else { continue };
            let default = |time: f32| defaults.iter().find(|(at, _)| (*at - time).abs() < 1e-4).map_or(0.0, |d| d.1);
            let keys = scrap::keys::blend(&t.keys, idx, self.tween_how, amount, &default);
            doc.set_track(Track { keys, ..t.clone() });
        }
        self.doc = Some(doc);
        self.version += 1;
        self.tween_at = amount;
        ui.set_text(self.tween_label, &format!("{:+.0}%", amount * 100.0));
    }

    /// Let go: one step back takes it all, the keys it made with it.
    fn tween_end(&mut self, ui: &mut Ui, session: &mut Session) {
        let Some((from, _)) = self.tweening.take() else { return };
        self.tween_at = 0.0;
        ui.set_text(self.tween_label, "");
        if self.doc.as_ref() == Some(&from) {
            return;
        }
        if !self.tween_made {
            self.undo.push(from);
        }
        self.redo.clear();
        self.write(ui, session);
    }

    fn draw_tween(&self, ui: &mut Ui) {
        let r = ui.rect(self.tween);
        let (mid, half, y) = (r.width / 2.0, (r.width / 2.0 - TWEEN_PAD).max(1.0), r.height / 2.0);
        let mut lines = vec![line(TWEEN_PAD / 2.0, y, r.width - TWEEN_PAD / 2.0, y, NEUTRAL_800, 2.0)];
        for t in [-1.0, -0.5, 0.0, 0.5, 1.0f32] {
            let x = mid + t * half;
            let h = if t == 0.0 { 7.0 } else { 4.0 };
            lines.push(line(x, y - h, x, y + h, if t.abs() == 1.0 { LABEL } else { MUTED }, 1.0));
        }
        let on = self.tweening.is_some();
        let x = mid + self.tween_at.clamp(-2.0, 2.0) * half;
        if on {
            lines.push(line(mid, y, x, y, ACCENT, 2.0));
        }
        let marks = vec![Mark { x, y, size: 11.0, shape: MarkShape::Circle, fill: if on { ACCENT } else { NEUTRAL_300 }, border: BG }];
        ui.set_drawing(self.tween, Drawing { lines, marks });
    }

    fn delete_chosen(&mut self, ui: &mut Ui, session: &mut Session) {
        let chosen = std::mem::take(&mut self.chosen);
        if chosen.is_empty() {
            // A chosen track row with no key chosen: the track goes.
            let rows: Vec<(String, Property)> = self
                .chosen_rows
                .iter()
                .filter_map(|r| if let RowKind::Track(p, w) = r { Some((p.clone(), *w)) } else { None })
                .collect();
            if rows.is_empty() {
                return;
            }
            self.chosen_rows.clear();
            self.change(ui, session, |doc| {
                for (p, w) in rows {
                    doc.set_track(Track { path: p, what: w, keys: Vec::new(), ease: Default::default() });
                }
            });
            return;
        }
        self.change(ui, session, |doc| {
            let tracks = doc.tracks.clone();
            for mut t in tracks {
                let before = t.keys.len();
                t.keys.retain(|k| !chosen.iter().any(|c| c.path == t.path && c.what == t.what && (c.time - k.time).abs() < 1e-4));
                if t.keys.len() != before {
                    doc.set_track(t);
                }
            }
        });
    }

    fn shape_chosen(&mut self, ui: &mut Ui, session: &mut Session, shape: Tangent) {
        let chosen = self.chosen.clone();
        if chosen.is_empty() {
            session.say(Level::Info, "choose keys to shape");
            return;
        }
        self.change(ui, session, |doc| {
            for t in doc.tracks.clone() {
                let mut t2 = t.clone();
                for k in &mut t2.keys {
                    if chosen.iter().any(|c| c.path == t.path && c.what == t.what && (c.time - k.time).abs() < 1e-4) {
                        k.tangent = shape;
                    }
                }
                if t2 != t {
                    doc.set_track(t2);
                }
            }
        });
    }

    fn copy(&mut self) {
        let Some(doc) = &self.doc else { return };
        let first = self.chosen.iter().map(|c| c.time).fold(f32::MAX, f32::min);
        self.clipboard = self
            .chosen
            .iter()
            .filter_map(|c| {
                let k = doc.track(&c.path, c.what)?.keys.iter().find(|k| (k.time - c.time).abs() < 1e-4)?;
                Some((c.path.clone(), c.what, k.time - first, k.value, k.tangent))
            })
            .collect();
    }

    fn paste(&mut self, ui: &mut Ui, session: &mut Session) {
        let keys = self.clipboard.clone();
        if keys.is_empty() {
            return;
        }
        let at = snap(self.time, false);
        self.chosen = keys.iter().map(|(p, w, dt, _, _)| KeyRef { path: p.clone(), what: *w, time: at + dt }).collect();
        self.change(ui, session, |doc| {
            for (path, what, dt, value, tangent) in keys {
                let mut track = doc.track(&path, what).cloned().unwrap_or(Track { path, what, keys: Vec::new(), ease: Default::default() });
                put_key(&mut track, ClipKey::shaped(at + dt, value, tangent));
                doc.set_track(track);
            }
        });
    }

    // --- record ----------------------------------------------------------

    /// While recording, a thing under the entity moved in the document is
    /// keys at the playhead, and the move is taken back from the scene.
    fn record(&mut self, ui: &mut Ui, session: &mut Session) {
        let (Some(id), Some(doc)) = (self.showing, &self.doc) else { return };
        if doc.kind != ClipKind::Things || session.is_dragging() {
            return;
        }
        let now = session.part_places(id);
        if self.recorded.is_empty() {
            self.recorded = now;
            return;
        }
        let mut keys: Vec<(String, Property, f32)> = Vec::new();
        for (path, place) in &now {
            let Some((_, was)) = self.recorded.iter().find(|(p, _)| p == path) else { continue };
            for (i, what) in TRANSFORM.iter().enumerate() {
                let (a, b) = (component(was, i), component(place, i));
                if (a - b).abs() > 1e-5 {
                    keys.push((path.clone(), *what, b));
                }
            }
        }
        if keys.is_empty() {
            return;
        }
        // The move goes back out of the scene: it was a key, not an edit.
        let _ = session.undo();
        let time = snap(self.time, false);
        self.chosen = keys.iter().map(|(p, w, _)| KeyRef { path: p.clone(), what: *w, time }).collect();
        for (p, _, _) in &keys {
            self.open_groups.insert(p.clone());
        }
        self.change(ui, session, |doc| {
            for (path, what, value) in keys {
                let mut track = doc.track(&path, what).cloned().unwrap_or(Track { path, what, keys: Vec::new(), ease: Default::default() });
                put_key(&mut track, ClipKey::shaped(time, value, Tangent::Auto));
                doc.set_track(track);
            }
        });
        self.recorded = session.part_places(id);
        self.posed = None;
    }

    // --- adding tracks -------------------------------------------------------

    fn list_adds(&mut self, ui: &mut Ui, session: &Session) {
        for (row, _, _) in self.add_rows.drain(..) {
            ui.remove(row);
        }
        let (Some(id), Some(doc)) = (self.showing, &self.doc) else { return };
        let filter = ui.text(self.add_filter).unwrap_or_default().to_lowercase();
        let (names, props): (Vec<String>, Vec<Property>) = match doc.kind {
            ClipKind::Things => (session.animatable_paths(id), TRANSFORM.iter().copied().chain([Property::Active]).collect()),
            _ => (session.joints(id), TRANSFORM.to_vec()),
        };
        let mut shown = 0;
        for name in names {
            let label = if name.is_empty() { "(itself)".to_string() } else { name.clone() };
            if !filter.is_empty() && !label.to_lowercase().contains(&filter) {
                continue;
            }
            if shown >= 40 {
                break;
            }
            shown += 1;
            let row = ui.add(self.adds, Style::row().full_width().height(20.0).fixed().gap(SPACE_1).center_items());
            ui.add_text(row, text().width(180.0).fixed().text_color(LABEL), &label);
            for (group, what) in [("Position", &props[0..3]), ("Turn", &props[3..6]), ("Scale", &props[6..9])] {
                let b = button(ui, row, &format!("add {name} {group}"), group, false);
                ui.restyle(b, |s| s.height(18.0));
                self.add_rows.push((b, name.clone(), what.to_vec()));
            }
            if props.contains(&Property::Active) {
                let b = button(ui, row, &format!("add {name} Active"), "Active", false);
                ui.restyle(b, |s| s.height(18.0));
                self.add_rows.push((b, name.clone(), vec![Property::Active]));
            }
            self.add_rows.push((row, name.clone(), Vec::new()));
        }
    }

    // --- input ---------------------------------------------------------------

    pub fn event(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, event: &Event) {
        let (shift, ctrl, alt, command) = ui.modifiers();
        let _ = ctrl;
        match event {
            Event::Submit(text) if node == self.time_field => {
                if let Ok(t) = text.trim().parse::<f32>() {
                    self.time = t.max(0.0);
                }
                self.show_time(ui);
            }
            Event::Submit(text) if node == self.length_field => {
                if let Ok(t) = text.trim().parse::<f32>() {
                    self.change(ui, session, |doc| doc.set_length(t));
                }
                self.show_time(ui);
            }
            Event::Changed(_) if node == self.add_filter => self.list_adds(ui, session),
            Event::Click { count, .. } => self.click(ui, session, node, *count, shift, alt, command),
            Event::Press { x, .. } if node == self.tween => self.tween_start(ui, session, *x),
            Event::Drag { x, .. } if node == self.tween => {
                let amount = self.tween_amount(ui, *x);
                self.tween_to(ui, amount);
            }
            Event::DragEnd { .. } if node == self.tween => self.tween_end(ui, session),
            Event::Press { x, y, .. } => self.press(ui, node, *x, *y, shift),
            Event::Drag { dx, dy, x, y } => self.drag_to(ui, session, node, *x, *y, *dx, *dy, alt),
            Event::DragEnd { .. } => self.drag_end(ui, session),
            Event::Wheel { x, y } if node == self.curves || node == self.ruler || self.is_lane(node) => {
                let rect = ui.rect(node);
                let (px, _) = ui.pointer();
                let at = self.t_of(px - rect.x);
                let zoom = (1.0 + y.clamp(-50.0, 50.0) * 0.01).max(0.2);
                if ctrl && node == self.curves {
                    let doc = self.doc.clone();
                    if let Some(doc) = doc {
                        let (lo, hi) = self.value_range(&doc);
                        let mid = (lo + hi) / 2.0;
                        self.range = Some((mid - (mid - lo) / zoom, mid + (hi - mid) / zoom));
                    }
                } else {
                    self.view.1 = (self.view.1 * zoom).clamp(10.0, 4000.0);
                    self.view.0 = at - (px - rect.x) / self.view.1;
                    self.view.0 -= x / self.view.1;
                }
            }
            _ => {}
        }
    }

    fn is_lane(&self, node: NodeId) -> bool {
        self.row_nodes.iter().any(|(_, lane, _)| *lane == node)
    }

    fn make_clip(&mut self, ui: &mut Ui, session: &mut Session, name: String, rig: bool) {
        let Some(id) = self.showing else { return };
        let name = if name.is_empty() { format!("clip-{}", self.clip_rows.len() + 1) } else { name };
        match session.new_clip(id, &name, rig) {
            Ok(_) => {
                ui.set_text(self.new_name, "");
                self.list_clips(ui, session);
                self.open_clip(ui, session, &name);
                self.list_adds(ui, session);
                session.say(Level::Info, format!("clip `{name}` made"));
            }
            Err(e) => session.say(Level::Error, e.to_string()),
        }
        self.mark(ui);
    }

    #[allow(clippy::too_many_arguments)]
    fn click(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, count: u32, shift: bool, _alt: bool, _command: bool) {
        if node == self.tween {
            // Pressed and let go without sliding: the amount where it was
            // pressed, a preset.
            self.tween_end(ui, session);
        } else if node == self.play {
            self.playing = !self.playing && self.doc.is_some();
            self.ticked = None;
        } else if node == self.rec {
            self.recording = !self.recording && self.doc.as_ref().is_some_and(|d| d.kind == ClipKind::Things);
            self.recorded.clear();
            if self.doc.as_ref().is_some_and(|d| d.kind != ClipKind::Things) {
                session.say(Level::Info, "recording keys what is moved in the view; bones are keyed with Key");
            }
        } else if node == self.first {
            self.time = 0.0;
        } else if node == self.last {
            self.time = self.doc.as_ref().map_or(0.0, |d| d.length);
        } else if node == self.back {
            self.step_frame(-1);
        } else if node == self.on {
            self.step_frame(1);
        } else if node == self.sheet_button {
            self.mode = Mode::Dopesheet;
        } else if node == self.curves_button {
            self.mode = Mode::Curves;
            self.range = None;
        } else if let Some((_, how)) = self.tween_modes.iter().find(|(b, _)| *b == node).copied() {
            self.tween_how = how;
            for (b, h) in &self.tween_modes {
                set_button_primary(ui, *b, *h == how);
            }
        } else if node == self.add_toggle {
            self.adding = !self.adding;
            let adding = self.adding;
            ui.restyle(self.adds, |s| if adding { s.shown() } else { s.hidden() });
            set_button_primary(ui, self.add_toggle, adding);
            self.list_adds(ui, session);
        } else if node == self.key_button {
            self.key_now(ui, session);
        } else if node == self.undo_button {
            self.take_back(ui, session, false);
        } else if node == self.stop {
            self.playing = false;
            self.recording = false;
            session.stop_posing();
            self.posed = None;
            self.doc = None;
            self.built = None;
            self.mark(ui);
            return;
        } else if node == self.new_clip || node == self.new_rig {
            let name = ui.text(self.new_name).unwrap_or_default();
            self.make_clip(ui, session, name.trim().to_string(), node == self.new_rig);
            return;
        } else if let Some((_, shape)) = self.shapes.iter().find(|(b, _)| *b == node).copied() {
            self.shape_chosen(ui, session, shape);
        } else if let Some((_, name, _)) = self.clip_rows.iter().find(|(r, _, _)| *r == node).cloned() {
            self.playing = false;
            self.open_clip(ui, session, &name);
            self.list_adds(ui, session);
        } else if let Some((_, name, props)) = self.add_rows.iter().find(|(b, _, p)| *b == node && !p.is_empty()).cloned() {
            self.key_tracks(ui, session, props.into_iter().map(|w| (name.clone(), w)).collect());
        } else if let Some((row, lane, kind)) = self.row_nodes.iter().find(|(r, l, _)| *r == node || *l == node).cloned() {
            let on_name = node == row && lane != row;
            if count >= 2 && node == lane && self.mode == Mode::Dopesheet {
                // Double-click on a lane: keys it there.
                let (px, _) = ui.pointer();
                self.time = snap(self.t_of(px - ui.rect(lane).x), false);
                let tracks = self.row_tracks(&kind);
                self.key_tracks(ui, session, tracks);
            } else if on_name || node == lane {
                if let RowKind::Group(p) = &kind {
                    if on_name || self.mode == Mode::Curves {
                        // Folded and unfolded by its name.
                        if !self.open_groups.remove(p) {
                            self.open_groups.insert(p.clone());
                        }
                    }
                }
                if !shift {
                    self.chosen_rows.clear();
                }
                if !self.chosen_rows.remove(&kind) {
                    self.chosen_rows.insert(kind);
                }
                self.range = None;
            }
        }
        self.show_time(ui);
        self.mark(ui);
    }

    fn row_tracks(&self, kind: &RowKind) -> Vec<(String, Property)> {
        let Some(doc) = &self.doc else { return Vec::new() };
        match kind {
            RowKind::Track(p, w) => vec![(p.clone(), *w)],
            RowKind::Group(p) => doc.tracks.iter().filter(|t| t.path == *p).map(|t| (t.path.clone(), t.what)).collect(),
            RowKind::Summary => doc.tracks.iter().map(|t| (t.path.clone(), t.what)).collect(),
        }
    }

    fn step_frame(&mut self, by: i32) {
        let length = self.doc.as_ref().map_or(0.0, |d| d.length);
        let frame = (self.time * FPS).round() as i32 + by;
        self.time = (frame as f32 / FPS).clamp(0.0, length);
        self.playing = false;
    }

    /// The keys under a point of a lane: (path, property, time).
    fn keys_at(&self, kind: &RowKind, x: f32) -> Vec<KeyRef> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let mut best: Option<f32> = None;
        let tracks: Vec<&Track> = match kind {
            RowKind::Track(p, w) => doc.track(p, *w).into_iter().collect(),
            RowKind::Group(p) => doc.tracks.iter().filter(|t| t.path == *p).collect(),
            RowKind::Summary => doc.tracks.iter().collect(),
        };
        for t in &tracks {
            for k in &t.keys {
                let d = (self.x_of(k.time) - x).abs();
                if d <= NEAR && best.is_none_or(|b| (self.x_of(b) - x).abs() > d) {
                    best = Some(k.time);
                }
            }
        }
        let Some(time) = best else { return Vec::new() };
        tracks
            .iter()
            .filter(|t| t.keys.iter().any(|k| (k.time - time).abs() < 1e-4))
            .map(|t| KeyRef { path: t.path.clone(), what: t.what, time })
            .collect()
    }

    /// The key of Curves under a point, and whether it is a handle (and
    /// which).
    fn curve_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<(KeyRef, Option<bool>)> {
        let doc = self.doc.as_ref()?;
        let rect = ui.rect(self.curves);
        let range = self.value_range(doc);
        let tracks = self.curve_tracks(doc);
        if self.chosen.len() <= 4 {
            for c in &self.chosen {
                let Some(t) = tracks.iter().find(|t| t.path == c.path && t.what == c.what) else { continue };
                let Some(i) = t.keys.iter().position(|k| (k.time - c.time).abs() < 1e-4) else { continue };
                let k = t.keys[i];
                let (kx, ky) = (self.x_of(k.time), self.y_of(k.value, range, rect.height));
                let (s_in, s_out) = scrap::keys::slopes(&t.keys, i);
                for (s, sign, out) in [(s_in, -1.0, false), (s_out, 1.0, true)] {
                    let (hx, hy) = self.handle_at(kx, ky, s, sign, range, rect.height);
                    if (hx - x).hypot(hy - y) <= NEAR {
                        return Some((c.clone(), Some(out)));
                    }
                }
            }
        }
        let mut best: Option<(f32, KeyRef)> = None;
        for t in &tracks {
            for k in &t.keys {
                let d = (self.x_of(k.time) - x).hypot(self.y_of(k.value, range, rect.height) - y);
                if d <= NEAR && best.as_ref().is_none_or(|(b, _)| d < *b) {
                    best = Some((d, KeyRef { path: t.path.clone(), what: t.what, time: k.time }));
                }
            }
        }
        best.map(|(_, k)| (k, None))
    }

    fn press(&mut self, ui: &mut Ui, node: NodeId, x: f32, y: f32, shift: bool) {
        let Some(doc) = self.doc.clone() else { return };
        if node == self.ruler {
            let rect = ui.rect(node);
            self.time = snap(self.t_of(x - rect.x), false).clamp(0.0, doc.length.max(0.0));
            self.playing = false;
            self.drag = Some(Drag::Scrub);
            self.show_time(ui);
            return;
        }
        if node == self.curves {
            let rect = ui.rect(node);
            let (lx, ly) = (x - rect.x, y - rect.y);
            match self.curve_hit(ui, lx, ly) {
                Some((key, Some(out))) => self.drag = Some(Drag::Handle { from: doc, key, out }),
                Some((key, None)) => {
                    if !shift && !self.chosen.contains(&key) {
                        self.chosen.clear();
                    }
                    if !self.chosen.contains(&key) {
                        self.chosen.push(key.clone());
                    }
                    self.drag = Some(Drag::Point { from: doc, key, x0: lx, y0: ly });
                }
                None => {
                    if !shift {
                        self.chosen.clear();
                    }
                    self.drag = Some(Drag::Pan);
                }
            }
            return;
        }
        let Some((_, lane, kind)) = self.row_nodes.iter().find(|(_, l, _)| *l == node).cloned() else { return };
        let rect = ui.rect(lane);
        let lx = x - rect.x;
        let hit = self.keys_at(&kind, lx);
        if hit.is_empty() {
            if !shift {
                self.chosen.clear();
            }
            // An empty place: the playhead goes there.
            self.time = snap(self.t_of(lx), false).clamp(0.0, doc.length);
            self.drag = Some(Drag::Scrub);
            self.show_time(ui);
            return;
        }
        let already = hit.iter().all(|h| self.chosen.contains(h));
        if !shift && !already {
            self.chosen.clear();
        }
        for h in hit {
            if !self.chosen.contains(&h) {
                self.chosen.push(h);
            }
        }
        self.drag = Some(Drag::Keys { from: doc, chosen: self.chosen.clone(), x0: lx, moved: false });
    }

    #[allow(clippy::too_many_arguments)]
    fn drag_to(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, x: f32, y: f32, dx: f32, dy: f32, alt: bool) {
        let rect = ui.rect(node);
        let (lx, ly) = (x - rect.x, y - rect.y);
        let length = self.doc.as_ref().map_or(0.0, |d| d.length);
        let _ = session;
        match self.drag.clone() {
            Some(Drag::Scrub) => {
                self.time = snap(self.t_of(lx), alt).clamp(0.0, length);
                self.show_time(ui);
            }
            Some(Drag::Pan) => {
                self.view.0 -= dx / self.view.1;
                if let Some(doc) = self.doc.clone() {
                    let (lo, hi) = self.value_range(&doc);
                    let per = (hi - lo) / rect.height.max(1.0);
                    self.range = Some((lo + dy * per, hi + dy * per));
                }
            }
            Some(Drag::Keys { from, chosen, x0, .. }) => {
                let shift_t = snap((lx - x0) / self.view.1, alt);
                let mut doc = from.clone();
                for t in from.tracks.iter() {
                    let mine: Vec<&KeyRef> = chosen.iter().filter(|c| c.path == t.path && c.what == t.what).collect();
                    if mine.is_empty() {
                        continue;
                    }
                    let mut t2 = t.clone();
                    let moving: Vec<ClipKey> = t.keys.iter().filter(|k| mine.iter().any(|c| (c.time - k.time).abs() < 1e-4)).copied().collect();
                    t2.keys.retain(|k| !mine.iter().any(|c| (c.time - k.time).abs() < 1e-4));
                    for mut k in moving {
                        k.time = (k.time + shift_t).max(0.0);
                        put_key(&mut t2, k);
                    }
                    doc.set_track(t2);
                }
                self.chosen = chosen.iter().map(|c| KeyRef { time: (c.time + shift_t).max(0.0), ..c.clone() }).collect();
                self.doc = Some(doc);
                self.version += 1;
                self.drag = Some(Drag::Keys { from, chosen, x0, moved: true });
            }
            Some(Drag::Point { from, key, x0, y0 }) => {
                let range = self.value_range(&from);
                let new_t = snap(key.time + (lx - x0) / self.view.1, alt).max(0.0);
                let old_v = from.track(&key.path, key.what).and_then(|t| t.keys.iter().find(|k| (k.time - key.time).abs() < 1e-4)).map(|k| k.value).unwrap_or(0.0);
                let new_v = old_v + (self.v_of(ly, range, rect.height) - self.v_of(y0, range, rect.height));
                let mut doc = from.clone();
                if let Some(mut t) = from.track(&key.path, key.what).cloned() {
                    if let Some(i) = t.keys.iter().position(|k| (k.time - key.time).abs() < 1e-4) {
                        let mut k = t.keys.remove(i);
                        k.time = new_t;
                        k.value = new_v;
                        put_key(&mut t, k);
                        doc.set_track(t);
                    }
                }
                self.chosen = vec![KeyRef { time: new_t, ..key.clone() }];
                self.doc = Some(doc);
                self.version += 1;
                self.range = Some(range);
            }
            Some(Drag::Handle { from, key, out }) => {
                let range = self.value_range(&from);
                let Some(t) = from.track(&key.path, key.what).cloned() else { return };
                let Some(i) = t.keys.iter().position(|k| (k.time - key.time).abs() < 1e-4) else { return };
                let k = t.keys[i];
                let (kx, ky) = (self.x_of(k.time), self.y_of(k.value, range, rect.height));
                let (hx, hy) = (lx - kx, ly - ky);
                if (hx.abs() < 1.0) || (out && hx <= 0.0) || (!out && hx >= 0.0) {
                    return;
                }
                let per_v = (range.1 - range.0) / rect.height.max(1.0);
                let slope = -(hy * per_v) / (hx / self.view.1);
                let (s_in, s_out) = scrap::keys::slopes(&t.keys, i);
                let shape = match k.tangent {
                    Tangent::Broken(..) => if out { Tangent::Broken(s_in, slope) } else { Tangent::Broken(slope, s_out) },
                    _ if alt => if out { Tangent::Broken(s_in, slope) } else { Tangent::Broken(slope, s_out) },
                    _ => Tangent::Free(slope),
                };
                let mut t2 = t.clone();
                t2.keys[i].tangent = shape;
                let mut doc = from.clone();
                doc.set_track(t2);
                self.doc = Some(doc);
                self.version += 1;
                self.range = Some(range);
            }
            None => {}
        }
    }

    fn drag_end(&mut self, ui: &mut Ui, session: &mut Session) {
        let Some(drag) = self.drag.take() else { return };
        let from = match drag {
            Drag::Keys { from, moved: true, .. } | Drag::Point { from, .. } | Drag::Handle { from, .. } => from,
            _ => return,
        };
        if self.doc.as_ref() != Some(&from) {
            self.undo.push(from);
            self.redo.clear();
            self.write(ui, session);
        }
    }

    /// A key, while the pointer is over the window. Whether it was the
    /// window's.
    pub fn key(&mut self, ui: &mut Ui, session: &mut Session, key: scrap::input::Key) -> bool {
        use scrap::input::Key as K;
        let over = ui.hovered().is_some_and(|h| self.owns(ui, h));
        if !over || self.doc.is_none() {
            return false;
        }
        let (shift, ctrl, _, command) = ui.modifiers();
        let cmd = ctrl || command;
        match key {
            K::Space => {
                self.playing = !self.playing;
                self.ticked = None;
            }
            K::K => self.key_now(ui, session),
            K::Delete | K::Backspace => self.delete_chosen(ui, session),
            K::Left => self.step_frame(-1),
            K::Right => self.step_frame(1),
            K::Z if cmd => self.take_back(ui, session, shift),
            K::C if cmd => self.copy(),
            K::V if cmd => self.paste(ui, session),
            K::F => {
                self.frame_all(ui);
            }
            _ => return false,
        }
        self.show_time(ui);
        self.mark(ui);
        true
    }
}

/// A key into a track, in time order, in place of one at the same time.
fn put_key(track: &mut Track, key: ClipKey) {
    track.keys.retain(|k| (k.time - key.time).abs() > 1e-4);
    let at = track.keys.partition_point(|k| k.time < key.time);
    track.keys.insert(at, key);
}

/// To a whole frame, or as it is with `free`.
fn snap(t: f32, free: bool) -> f32 {
    if free {
        t
    } else {
        (t * FPS).round() / FPS
    }
}

const TRANSFORM: [Property; 9] = [
    Property::X,
    Property::Y,
    Property::Z,
    Property::TurnX,
    Property::TurnY,
    Property::TurnZ,
    Property::ScaleX,
    Property::ScaleY,
    Property::ScaleZ,
];

fn component(t: &scrap::Transform, i: usize) -> f32 {
    match i {
        0..=2 => t.position[i],
        3..=5 => t.rotation_deg[i - 3],
        _ => t.scale[i - 6],
    }
}

fn axis_color(what: Property) -> Color {
    match what {
        Property::X | Property::TurnX | Property::ScaleX => AXIS_X,
        Property::Y | Property::TurnY | Property::ScaleY => AXIS_Y,
        Property::Z | Property::TurnZ | Property::ScaleZ => AXIS_Z,
        _ => ACCENT,
    }
}

fn row_label(kind: &RowKind) -> String {
    match kind {
        RowKind::Summary => "summary".into(),
        RowKind::Group(p) => p.clone(),
        RowKind::Track(p, w) => format!("{p} {w:?}"),
    }
}

fn line(x0: f32, y0: f32, x1: f32, y1: f32, color: Color, width: f32) -> Polyline {
    Polyline { points: vec![(x0, y0), (x1, y1)], color, width }
}
