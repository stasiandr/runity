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
    /// A box drawn over the keys, window coordinates from where it was
    /// pressed to where the pointer is; `kept` the keys chosen before it,
    /// with Shift.
    Box { x0: f32, y0: f32, x1: f32, y1: f32, kept: Vec<KeyRef> },
}

pub struct Animation {
    pub root: NodeId,
    title: NodeId,
    rec: NodeId,
    rec_badge: NodeId,
    play: NodeId,
    first: NodeId,
    back: NodeId,
    on: NodeId,
    last: NodeId,
    time_field: NodeId,
    frame_label: NodeId,
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
    /// Past the clip's end, shaded: over the ruler, Curves and each lane.
    ruler_shade: NodeId,
    /// What the ruler's numbers are written in, under the playhead's flag.
    ruler_marks: NodeId,
    curves_shade: NodeId,
    shades: Vec<NodeId>,
    /// The playhead's frame, over the ruler.
    flag: NodeId,
    flag_text: NodeId,
    rows: NodeId,
    curves: NodeId,
    curve_names: NodeId,
    /// The values written along Curves' grid, and the values they say.
    curve_labels: Vec<(NodeId, f32)>,
    /// What the window says when there is nothing to show.
    empty: NodeId,
    empty_text: NodeId,
    normalize: NodeId,
    /// Curves each drawn over the same height, -1 to 1, as Blender's
    /// Normalize: a turn of 90° and a lift of 2 cm side by side.
    normalized: bool,
    adds: NodeId,
    add_filter: NodeId,
    add_toggle: NodeId,
    adding: bool,
    /// The thing or bone the add panel shows the numbers of.
    add_path: Option<String>,
    /// What each thing's shader calls the slots of its material's
    /// `params`, by the thing's path: read when a clip opens.
    // Paths with their named slots.
    #[allow(clippy::type_complexity)]
    param_names: Vec<(String, Vec<(usize, String, Option<&'static str>)>)>,
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
    /// A track row's value at the playhead, by its text.
    row_values: Vec<(NodeId, String, Property)>,
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
    recorded: Vec<(String, Property, f32)>,
    /// The document's revision `recorded` was read at.
    recorded_at: Option<u64>,
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
    /// The document's revision when the window last changed the clip:
    /// while it is still that, Undo is the window's (as the Configs
    /// window's is), from the menu, the keys or the toolbar alike.
    revision: Option<u64>,
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
        let root = ui.add(parent, Style::column().fill().full_width());
        ui.set_name(root, "animation");
        // The transport, the time and the view.
        let bar = ui.add(root, Style::row().full_width().height(34.0).fixed().gap(SPACE_3).padding_x(SPACE_3).center_items());
        let transport = segmented(ui, bar);
        let rec = icon_cell(ui, transport, "animation record", "record");
        word_divider(ui, transport);
        let first = icon_cell(ui, transport, "animation first", "skip-back");
        let back = icon_cell(ui, transport, "animation back", "chevron-left");
        let play = icon_cell(ui, transport, "animation play", "play");
        let on = icon_cell(ui, transport, "animation on", "chevron-right");
        let last = icon_cell(ui, transport, "animation last", "skip-forward");
        let clock = ui.add(bar, Style::row().gap(SPACE_1).center_items());
        let time_field = small_field(ui, clock, "animation time", 54.0);
        let frame_label = ui.add_text(clock, caption().width(34.0).fixed(), "");
        ui.set_name(frame_label, "animation frame");
        ui.add_text(clock, caption(), "of");
        let length_field = small_field(ui, clock, "animation length", 46.0);
        ui.add_text(clock, caption(), "s");
        let modes = segmented(ui, bar);
        let sheet_button = word_toggle(ui, modes, "animation dopesheet", "Dopesheet");
        word_divider(ui, modes);
        let curves_button = word_toggle(ui, modes, "animation curves", "Curves");
        let normalize_group = segmented(ui, bar);
        let normalize = word_toggle(ui, normalize_group, "animation normalize", "Normalize");
        ui.restyle(normalize_group, |s| s.hidden());
        ui.add(bar, Style::row().fill());
        let rec_badge = ui.add(bar, Style::row().height(18.0).fixed().padding_x(7.0).center().radius(6.0).background(ERROR.alpha(22)).hidden());
        ui.add_text(rec_badge, Style::default().text_size(10.5).text_color(ERROR).nowrap(), "● RECORDING");
        let title = ui.add_text(bar, text().text_color(LABEL), "");
        ui.set_name(title, "animation title");
        let undo_button = icon_button(ui, bar, "animation undo", "undo-2", false);
        let stop = icon_button(ui, bar, "animation stop", "x", false);

        // Keys: make one, shape them, blend them.
        let keys = ui.add(root, Style::row().full_width().height(32.0).fixed().gap(SPACE_3).padding_x(SPACE_3).center_items());
        let key_group = segmented(ui, keys);
        let key_button = ui.add(key_group, word_toggle_style(false).gap(5.0));
        ui.set_name(key_button, "animation key");
        icon(ui, key_button, "diamond", LABEL);
        ui.add_text(key_button, Style::default().text_size(11.5).text_color(LABEL).nowrap(), "Key");
        let shape_group = segmented(ui, keys);
        let mut shapes = Vec::new();
        for (i, (name, glyph, t)) in [
            ("tangent auto", "tangent-auto", Tangent::Auto),
            ("tangent flat", "tangent-flat", Tangent::Flat),
            ("tangent linear", "tangent-linear", Tangent::Linear),
            ("tangent step", "tangent-step", Tangent::Step),
        ]
        .into_iter()
        .enumerate()
        {
            if i > 0 {
                word_divider(ui, shape_group);
            }
            shapes.push((icon_cell(ui, shape_group, name, glyph), t));
        }
        let blend = ui.add(keys, Style::row().gap(SPACE_2).center_items());
        ui.add_text(blend, caption(), "BLEND");
        let blend_group = segmented(ui, blend);
        let mut tween_modes = Vec::new();
        for (i, (label, how)) in [("Tween", Blend::Tween), ("Neighbor", Blend::Neighbor), ("Push", Blend::Push), ("Default", Blend::Default)]
            .into_iter()
            .enumerate()
        {
            if i > 0 {
                word_divider(ui, blend_group);
            }
            let b = word_toggle(ui, blend_group, &format!("blend {}", label.to_lowercase()), label);
            set_word_toggle(ui, b, how == Blend::Tween);
            tween_modes.push((b, how));
        }
        let tween = ui.add(blend, Style::row().width(220.0).fixed().height(22.0).clickable().draggable());
        ui.set_name(tween, "tween slider");
        let tween_label = ui.add_text(blend, text().width(44.0).fixed().text_color(ACCENT).mono(), "");
        ui.set_name(tween_label, "tween amount");
        ui.add(root, Style::row().full_width().height(1.0).fixed().background(DIVIDER));

        let body = ui.add(root, Style::row().fill().full_width().min_height(0.0));
        // The clips.
        let left = ui.add(body, Style::column().width(190.0).fixed().full_height().min_height(0.0).padding(SPACE_2).gap(SPACE_2));
        let clips_head = ui.add(left, Style::row().full_width().height(20.0).fixed().padding_x(SPACE_1).center_items());
        ui.add_text(clips_head, caption().fill(), "CLIPS");
        let clips_list = ui.add(left, Style::column().fill().full_width().min_height(48.0).gap(1.0).clip());
        ui.set_name(clips_list, "animation clips");
        let new_name = ui.add_field(left, field_style().full_width(), "");
        ui.set_name(new_name, "new clip name");
        ui.set_placeholder(new_name, "New clip…");
        let new_row = segmented(ui, left);
        ui.restyle(new_row, |s| s.full_width());
        let new_clip = word_toggle(ui, new_row, "new clip", "+ Things");
        ui.restyle(new_clip, |s| s.fill());
        word_divider(ui, new_row);
        let new_rig = word_toggle(ui, new_row, "new rig clip", "+ Bones");
        ui.restyle(new_rig, |s| s.fill());
        ui.add(body, Style::row().width(1.0).fixed().full_height().background(DIVIDER));

        // The tracks.
        let main = ui.add(body, Style::column().fill().full_height().min_height(0.0).clip());
        let head = ui.add(main, Style::row().full_width().height(RULER_H).fixed());
        let corner = ui.add(head, Style::row().width(NAMES_W).fixed().full_height().padding_x(SPACE_3).gap(SPACE_1).center_items());
        ui.add_text(corner, caption().fill(), "TRACKS");
        let add_toggle = ui.add(corner, word_toggle_style(false).height(18.0).radius(RADIUS_SM).gap(3.0));
        ui.set_name(add_toggle, "animation add");
        icon(ui, add_toggle, "plus", LABEL);
        ui.add_text(add_toggle, Style::default().text_size(11.0).text_color(LABEL).nowrap(), "Track");
        let ruler = ui.add(head, Style::row().fill().full_height().background(BG).clickable().draggable().clip());
        ui.set_name(ruler, "animation ruler");
        let ruler_shade = ui.add(ruler, shade_style(0.0));
        let ruler_marks = ui.add(ruler, Style::row().absolute(0.0, 0.0).full());
        let flag = ui.add(ruler, Style::row().absolute(0.0, 3.0).size(30.0, 16.0).fixed().center().radius(4.0).background(ACCENT));
        let flag_text = ui.add_text(flag, Style::default().text_size(10.0).text_color(BG).nowrap().mono(), "0");
        let rows = ui.add(main, Style::column().fill().full_width().min_height(0.0).clip());
        ui.set_name(rows, "animation rows");
        let curves_row = ui.add(main, Style::row().fill().full_width().hidden());
        let curve_names = ui.add(curves_row, Style::column().width(NAMES_W).fixed().full_height().clip());
        let curves = ui.add(curves_row, Style::column().fill().full_height().background(BG).clickable().draggable().clip());
        ui.set_name(curves, "animation curve view");
        let curves_shade = ui.add(curves, shade_style(0.0));
        let empty = ui.add(main, Style::column().fill().full_width().center().gap(SPACE_2).hidden());
        icon(ui, empty, "play", MUTED);
        let empty_text = ui.add_text(empty, text().text_color(MUTED), "");
        ui.set_name(empty_text, "animation hint");
        // Adding a track.
        let adds = ui.add(main, Style::column().full_width().max_height(170.0).min_height(0.0).padding(SPACE_2).gap(1.0).background(BG).clip().hidden());
        let add_bar = ui.add(adds, Style::row().full_width().gap(SPACE_2).center_items());
        ui.add_text(add_bar, caption(), "ADD A TRACK TO");
        let add_filter = ui.add_field(add_bar, field_style().width(200.0).fixed(), "");
        ui.set_name(add_filter, "animation add filter");
        ui.set_placeholder(add_filter, "thing or bone…");
        let me = Self {
            root,
            title,
            rec,
            rec_badge,
            play,
            first,
            back,
            on,
            last,
            time_field,
            frame_label,
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
            ruler_shade,
            ruler_marks,
            curves_shade,
            shades: Vec::new(),
            flag,
            flag_text,
            rows,
            curves,
            curve_names,
            curve_labels: Vec::new(),
            empty,
            empty_text,
            normalize,
            normalized: false,
            adds,
            add_filter,
            add_toggle,
            adding: false,
            add_path: None,
            param_names: Vec::new(),
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
            row_values: Vec::new(),
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
            recorded_at: None,
            mode: Mode::Dopesheet,
            open_groups: BTreeSet::new(),
            chosen_rows: BTreeSet::new(),
            chosen: Vec::new(),
            view: (0.0, 200.0),
            range: None,
            drag: None,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: None,
            posed: None,
            version: 0,
            built: None,
            clipboard: Vec::new(),
        };
        me.show_time(ui);
        me.mark(ui);
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
            ui.set_text(self.title, "");
            return;
        };
        for (clip, kind, length) in session.clip_list(id) {
            let row = ui.add(
                self.clips_list,
                Style::row().full_width().height(24.0).fixed().padding_x(SPACE_2).gap(SPACE_2).center_items().radius(RADIUS_SM).hover(HOVER).clickable(),
            );
            ui.set_name(row, format!("clip {clip}"));
            let (glyph, color) = match kind {
                ClipKind::Things => ("box", INFO),
                ClipKind::Rig => ("bone", SUCCESS),
                ClipKind::Model => ("file-box", WARNING),
            };
            icon(ui, row, glyph, color);
            ui.add_text(row, text().fill(), &clip);
            ui.add_text(row, caption().mono(), &format!("{length:.2}"));
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
                let mut paths: Vec<String> = doc.tracks.iter().map(|t| t.path.clone()).collect();
                paths.dedup();
                self.param_names = paths.into_iter().map(|p| (p.clone(), session.shader_params(id, &p))).collect();
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
                    out.push(RowKind::Track(p.to_string(), t.what.clone()));
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
        self.row_values.clear();
        self.shades.clear();
        let curves = self.mode == Mode::Curves;
        let nothing = self.doc.is_none();
        ui.restyle(self.rows, |s| if curves || nothing { s.hidden() } else { s.shown() });
        if let Some(row) = ui.parent(self.curves) {
            ui.restyle(row, |s| if curves && !nothing { s.shown() } else { s.hidden() });
        }
        ui.restyle(self.empty, |s| if nothing { s.shown() } else { s.hidden() });
        let bones = self.doc.as_ref().is_some_and(|d| d.kind != ClipKind::Things);
        for kind in kinds {
            let parent = if curves { self.curve_names } else { self.rows };
            let band = match kind {
                RowKind::Track(..) => Color::TRANSPARENT,
                _ => TEXT.alpha(3),
            };
            let row = ui.add(parent, Style::row().full_width().height(ROW_H).fixed().background(band).clickable().hover(HOVER));
            let name = ui.add(row, Style::row().width(NAMES_W).fixed().full_height().padding_x(SPACE_3).gap(SPACE_2).center_items());
            match &kind {
                RowKind::Summary => {
                    icon(ui, name, "diamond", MUTED);
                    ui.add_text(name, text().fill().text_color(LABEL), "Summary");
                    ui.set_name(row, "row summary");
                }
                RowKind::Group(p) => {
                    let open = self.open_groups.contains(p);
                    icon(ui, name, if open { "chevron-down" } else { "chevron-right" }, MUTED);
                    let shown = if p.is_empty() { "(itself)".to_string() } else { p.rsplit('/').next().unwrap_or(p).to_string() };
                    let depth = if bones { 0 } else { p.matches('/').count() + usize::from(!p.is_empty()) };
                    ui.restyle(name, |s| s.padding_left(SPACE_3 + 14.0 * depth as f32));
                    icon(ui, name, if bones { "bone" } else { "box" }, if bones { SUCCESS } else { INFO });
                    ui.add_text(name, text().fill(), &shown);
                    ui.set_name(row, format!("row {p}"));
                }
                RowKind::Track(p, what) => {
                    ui.restyle(name, |s| s.padding_left(SPACE_3 + 26.0));
                    ui.add(name, Style::row().size(3.0, 12.0).fixed().radius(1.5).background(axis_color(what)));
                    ui.add_text(name, text().fill().text_color(LABEL), &self.label_of(p, what));
                    let value = ui.add_text(name, caption().mono(), "");
                    self.row_values.push((value, p.clone(), what.clone()));
                    ui.set_name(row, format!("row {p} {what:?}"));
                }
            }
            let lane = if curves {
                row
            } else {
                let lane_bg = match kind {
                    RowKind::Track(..) => BG,
                    _ => NEUTRAL_900,
                };
                let lane = ui.add(row, Style::row().fill().full_height().background(lane_bg).clickable().draggable().clip());
                ui.set_name(lane, format!("lane {}", row_label(&kind)));
                self.shades.push(ui.add(lane, shade_style(0.0)));
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
            ui.restyle(self.flag, |s| s.hidden());
            return;
        };
        let ruler = ui.rect(self.ruler);
        // The ruler: a tick a frame where they fit, a second taller.
        let mut lines = Vec::new();
        let end = self.x_of(doc.length);
        ui.set_style(self.ruler_shade, shade_style(end));
        let step = self.tick_step();
        let first = (self.view.0 / step).floor() as i64;
        let last = (self.t_of(ruler.width) / step).ceil() as i64;
        for i in first..=last.min(first + 2000) {
            let t = i as f32 * step;
            let x = self.x_of(t);
            let whole = (t - t.round()).abs() < 1e-4;
            let h = if whole { RULER_H * 0.45 } else { RULER_H * 0.22 };
            lines.push(line(x, RULER_H - h, x, RULER_H, if whole { LABEL } else { MUTED }, 1.0));
        }
        let head = self.x_of(self.time);
        lines.push(line(head, RULER_H - 6.0, head, RULER_H, ACCENT, 2.0));
        ui.set_drawing(self.ruler, Drawing { lines, marks: Vec::new() });
        self.label_ruler(ui, ruler.width);
        // A number under the playhead's flag is the flag's to say.
        for (n, t) in self.ruler_labels.clone() {
            let near = (self.x_of(t) + 14.0 - head).abs() < 34.0;
            ui.restyle(n, |s| if near { s.hidden() } else { s.shown() });
        }
        ui.restyle(self.flag, |s| s.shown().absolute(head - 15.0, 3.0));
        ui.set_text(self.flag_text, &format!("{}", (self.time * FPS).round() as i64));
        // Each track's value where the playhead is.
        for (node, path, what) in self.row_values.clone() {
            let v = doc.track(&path, &what).and_then(|t| t.sample(self.time));
            ui.set_text(node, &v.map(|v| value_text(&what, v)).unwrap_or_default());
        }
        if self.mode == Mode::Curves {
            self.draw_curves(ui, &doc);
            for (row, _, kind) in self.row_nodes.clone() {
                let on = self.chosen_rows.contains(&kind);
                ui.restyle(row, |s| s.background(if on { ACCENT_900 } else { Color::TRANSPARENT }));
            }
            return;
        }
        for shade in self.shades.clone() {
            ui.set_style(shade, shade_style(end));
        }
        for (row, lane, kind) in self.row_nodes.clone() {
            let rect = ui.rect(lane);
            let mut marks = Vec::new();
            let mut lines = Vec::new();
            // Past the clip's end, shaded; a line a second.
            let seconds = if self.view.1 >= 60.0 { 1.0 } else { 5.0 };
            let mut t = (self.view.0 / seconds).ceil() * seconds;
            while self.x_of(t) < rect.width && t < 1e4 {
                lines.push(line(self.x_of(t), 0.0, self.x_of(t), ROW_H, TEXT.alpha(5), 1.0));
                t += seconds;
            }
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
                RowKind::Track(p, what) => {
                    let Some(track) = doc.track(p, what) else { continue };
                    // A hold: two keys of one value, a bar between them.
                    for pair in track.keys.windows(2) {
                        if (pair[0].value - pair[1].value).abs() < 1e-5 {
                            lines.push(line(self.x_of(pair[0].time), ROW_H / 2.0, self.x_of(pair[1].time), ROW_H / 2.0, NEUTRAL_800, 4.0));
                        }
                    }
                    track.keys.iter().map(|k| (k.time, self.is_chosen(p, what, k.time))).collect()
                }
            };
            let written = match &kind {
                RowKind::Track(p, w) => doc.kind != ClipKind::Model || doc.is_written(p, w),
                RowKind::Group(p) => doc.kind != ClipKind::Model || doc.motion.tracks.iter().any(|t| t.path == *p),
                RowKind::Summary => true,
            };
            let (size, fill) = match kind {
                RowKind::Track(..) => (10.0, if written { NEUTRAL_300 } else { NEUTRAL_500 }),
                _ => (11.0, if written { NEUTRAL_500 } else { NEUTRAL_800 }),
            };
            for (t, on) in keys {
                let x = self.x_of(t);
                if x < -NEAR || x > rect.width + NEAR {
                    continue;
                }
                marks.push(Mark {
                    x,
                    y: ROW_H / 2.0,
                    size: if on { size + 1.0 } else { size },
                    shape: MarkShape::Diamond,
                    fill: if on { ACCENT } else { fill },
                    border: if on { ACCENT_200 } else { BG },
                });
            }
            lines.push(line(head, 0.0, head, ROW_H, ACCENT, 1.5));
            lines.extend(self.box_lines(ui, lane));
            ui.set_drawing(lane, Drawing { lines, marks });
            let on = self.chosen_rows.contains(&kind);
            let band = match kind {
                RowKind::Track(..) => Color::TRANSPARENT,
                _ => TEXT.alpha(3),
            };
            ui.restyle(row, |s| s.background(if on { ACCENT_900 } else { band }));
        }
    }

    /// The ruler's smallest tick: a frame where frames have room, else a
    /// tenth or a whole second.
    fn tick_step(&self) -> f32 {
        if self.view.1 / FPS >= 6.0 {
            1.0 / FPS
        } else if self.view.1 >= 40.0 {
            0.1
        } else {
            1.0
        }
    }

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
            let n = ui.add_text(self.ruler_marks, caption().absolute(self.x_of(t) + 3.0, 1.0), &label);
            self.ruler_labels.push((n, t));
        }
    }

    /// The keys inside a box, in window coordinates: in Curves where they
    /// are drawn, in the Dopesheet on the rows it crosses, a group's or the
    /// summary's row standing for every key of its tracks at that time.
    fn keys_in_box(&self, ui: &Ui, (left, top, right, bottom): (f32, f32, f32, f32)) -> Vec<KeyRef> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let mut out = Vec::new();
        if self.mode == Mode::Curves {
            let rect = ui.rect(self.curves);
            let range = self.value_range(doc);
            for t in self.curve_tracks(doc) {
                let (a, b) = self.norm(doc, &t);
                for k in &t.keys {
                    let (x, y) = (rect.x + self.x_of(k.time), rect.y + self.y_of(k.value * a + b, range, rect.height));
                    if x >= left && x <= right && y >= top && y <= bottom {
                        out.push(KeyRef { path: t.path.clone(), what: t.what.clone(), time: k.time });
                    }
                }
            }
            return out;
        }
        for (_, lane, kind) in &self.row_nodes {
            let r = ui.rect(*lane);
            if r.y + r.height < top || r.y > bottom {
                continue;
            }
            let (t0, t1) = (self.t_of(left - r.x) - 1e-4, self.t_of(right - r.x) + 1e-4);
            for t in doc.tracks.iter().filter(|t| match kind {
                RowKind::Track(p, w) => t.path == *p && t.what == *w,
                RowKind::Group(p) => t.path == *p,
                RowKind::Summary => true,
            }) {
                for k in t.keys.iter().filter(|k| k.time >= t0 && k.time <= t1) {
                    let key = KeyRef { path: t.path.clone(), what: t.what.clone(), time: k.time };
                    if !out.contains(&key) {
                        out.push(key);
                    }
                }
            }
        }
        out
    }

    /// The box being drawn, as lines in a node's own coordinates.
    fn box_lines(&self, ui: &Ui, node: NodeId) -> Vec<Polyline> {
        let Some(Drag::Box { x0, y0, x1, y1, .. }) = &self.drag else { return Vec::new() };
        let r = ui.rect(node);
        let (l, t, rt, b) = (x0.min(*x1) - r.x, y0.min(*y1) - r.y, x0.max(*x1) - r.x, y0.max(*y1) - r.y);
        if b < 0.0 || t > r.height {
            return Vec::new();
        }
        let (top, bottom) = (t.max(0.0), b.min(r.height));
        let mut out = vec![line(l, top, l, bottom, ACCENT, 1.0), line(rt, top, rt, bottom, ACCENT, 1.0)];
        if t >= 0.0 {
            out.push(line(l, t, rt, t, ACCENT, 1.0));
        }
        if b <= r.height {
            out.push(line(l, b, rt, b, ACCENT, 1.0));
        }
        out
    }

    /// The name a thing's shader gives a slot of its params, and the axis,
    /// for a `Material("params.N")` track.
    fn param_name(&self, path: &str, what: &Property) -> Option<(String, Option<&'static str>)> {
        let Property::Material(m) = what else { return None };
        let slot = m.strip_prefix("params.")?.parse::<usize>().ok()?;
        let (_, names) = self.param_names.iter().find(|(p, _)| p == path)?;
        names.iter().find(|(s, _, _)| *s == slot).map(|(_, n, a)| (n.clone(), *a))
    }

    /// A track's name in its row: a shader's slot by the shader's name for
    /// it (`Glow`, `Tint R`), the rest as [`property_label`] says.
    fn label_of(&self, path: &str, what: &Property) -> String {
        match self.param_name(path, what) {
            Some((name, Some(axis))) => format!("{} {}", words(&name), axis.to_uppercase()),
            Some((name, None)) => words(&name),
            None => property_label(what),
        }
    }

    fn is_chosen(&self, path: &str, what: &Property, time: f32) -> bool {
        self.chosen.iter().any(|c| c.path == path && c.what == *what && (c.time - time).abs() < 1e-4)
    }

    /// The tracks Curves shows: the chosen rows', a group for all of its;
    /// none chosen, every track of the open groups, or all of them.
    fn curve_tracks(&self, doc: &ClipDoc) -> Vec<Track> {
        let wanted = |t: &Track, rows: &BTreeSet<RowKind>| {
            rows.contains(&RowKind::Track(t.path.clone(), t.what.clone())) || rows.contains(&RowKind::Group(t.path.clone())) || rows.contains(&RowKind::Summary)
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
        if self.normalized {
            return (-1.25, 1.25);
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
        let end = self.x_of(doc.length);
        ui.set_style(self.curves_shade, shade_style(end));
        // A line a value step, and one a second.
        let step = nice_step((range.1 - range.0) / 6.0);
        let mut v = (range.0 / step).ceil() * step;
        let mut labels = Vec::new();
        while v <= range.1 && labels.len() < 40 {
            let y = self.y_of(v, range, rect.height);
            lines.push(line(0.0, y, rect.width, y, if v.abs() < step * 1e-3 { DIVIDER } else { TEXT.alpha(5) }, 1.0));
            labels.push(v);
            v += step;
        }
        let mut t = self.view.0.ceil();
        while self.x_of(t) < rect.width && t < 1e4 {
            lines.push(line(self.x_of(t), 0.0, self.x_of(t), rect.height, TEXT.alpha(5), 1.0));
            t += 1.0;
        }
        self.label_curves(ui, &labels, range, rect.height);
        for track in self.curve_tracks(doc) {
            let color = axis_color(&track.what);
            let (a, b) = self.norm(doc, &track);
            let mut points = Vec::new();
            let mut x = 0.0;
            while x <= rect.width {
                if let Some(v) = track.sample(self.t_of(x)) {
                    points.push((x, self.y_of(v * a + b, range, rect.height)));
                }
                x += 2.0;
            }
            lines.push(Polyline { points, color, width: 1.5 });
            for (i, k) in track.keys.iter().enumerate() {
                let (x, y) = (self.x_of(k.time), self.y_of(k.value * a + b, range, rect.height));
                let on = self.is_chosen(&track.path, &track.what, k.time);
                marks.push(Mark { x, y, size: if on { 9.0 } else { 8.0 }, shape: MarkShape::Circle, fill: if on { ACCENT } else { color }, border: if on { ACCENT_200 } else { BG } });
                if on && self.chosen.len() <= 4 {
                    let (s_in, s_out) = scrap::keys::slopes(&track.keys, i);
                    for (s, sign) in [(s_in * a, -1.0f32), (s_out * a, 1.0)] {
                        let (hx, hy) = self.handle_at(x, y, s, sign, range, rect.height);
                        lines.push(line(x, y, hx, hy, NEUTRAL_300, 1.0));
                        marks.push(Mark { x: hx, y: hy, size: 7.0, shape: MarkShape::Square, fill: NEUTRAL_300, border: BG });
                    }
                }
            }
        }
        let head = self.x_of(self.time);
        lines.push(line(head, 0.0, head, rect.height, ACCENT, 1.5));
        lines.extend(self.box_lines(ui, self.curves));
        ui.set_drawing(self.curves, Drawing { lines, marks });
    }

    /// The values along Curves' grid, written at its left.
    fn label_curves(&mut self, ui: &mut Ui, values: &[f32], range: (f32, f32), height: f32) {
        let placed: Vec<(f32, f32)> = values.iter().map(|v| (*v, self.y_of(*v, range, height))).collect();
        let same = self.curve_labels.len() == placed.len()
            && self.curve_labels.iter().zip(&placed).all(|((_, a), (b, _))| a == b);
        if same {
            for ((n, _), (_, y)) in self.curve_labels.iter().zip(&placed) {
                ui.restyle(*n, |s| s.absolute(4.0, y - 14.0));
            }
            return;
        }
        for (n, _) in self.curve_labels.drain(..) {
            ui.remove(n);
        }
        for (v, y) in placed {
            let label = if v.abs() >= 100.0 { format!("{v:.0}") } else { format!("{}", (v * 1000.0).round() / 1000.0) };
            let n = ui.add_text(self.curves, caption().mono().absolute(4.0, y - 14.0), &label);
            self.curve_labels.push((n, v));
        }
    }

    /// How a track is drawn in Curves: its value times `a` plus `b` —
    /// as it is, or with Normalize, its own lowest to highest over -1..1.
    fn norm(&self, doc: &ClipDoc, track: &Track) -> (f32, f32) {
        if !self.normalized {
            return (1.0, 0.0);
        }
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in 0..=64 {
            if let Some(v) = track.sample(doc.length * i as f32 / 64.0) {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        for k in &track.keys {
            lo = lo.min(k.value);
            hi = hi.max(k.value);
        }
        if lo > hi {
            return (1.0, 0.0);
        }
        if hi - lo < 1e-6 {
            return (1.0, -lo);
        }
        let a = 2.0 / (hi - lo);
        (a, -1.0 - lo * a)
    }

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
        ui.set_text(self.frame_label, &format!("f {frame}"));
        let length = self.doc.as_ref().map(|d| format!("{:.2}", d.length)).unwrap_or_default();
        ui.set_text(self.length_field, &length);
    }

    fn mark(&self, ui: &mut Ui) {
        set_icon_cell(ui, self.rec, self.recording, ERROR);
        ui.restyle(self.rec_badge, |s| if self.recording { s.shown() } else { s.hidden() });
        set_icon_cell(ui, self.play, self.playing, ACCENT);
        if let Some(glyph) = ui.children(self.play).first().copied() {
            ui.set_icon(glyph, if self.playing { "pause" } else { "play" });
        }
        set_word_toggle(ui, self.sheet_button, self.mode == Mode::Dopesheet);
        set_word_toggle(ui, self.curves_button, self.mode == Mode::Curves);
        set_word_toggle(ui, self.normalize, self.normalized);
        if let Some(group) = ui.parent(self.normalize) {
            let curves = self.mode == Mode::Curves;
            ui.restyle(group, |s| if curves { s.shown() } else { s.hidden() });
        }
        set_icon_button(ui, self.undo_button, "undo-2", false, !self.undo.is_empty());
        let open = self.doc.as_ref().map(|d| d.name.clone());
        for (row, name, _) in &self.clip_rows {
            let on = Some(name) == open.as_ref();
            ui.restyle(*row, |s| s.background(if on { ACCENT_900 } else { Color::TRANSPARENT }));
        }
        ui.set_text(self.title, open.as_deref().unwrap_or(""));
        ui.set_text(self.empty_text, match (self.showing, self.clip_rows.is_empty()) {
            (None, _) => "Select something to animate",
            (Some(_), true) => "No clips yet: name one on the left and make it of its things or its bones",
            (Some(_), false) => "Choose a clip on the left",
        });
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
        self.revision = Some(session.revision());
        if !was_file && self.doc.as_ref().is_some_and(|d| d.file.is_some()) {
            let name = self.doc.as_ref().map(|d| d.name.clone()).unwrap_or_default();
            session.say(Level::Info, format!("clip `{name}`: changes written over the model's"));
        }
        self.show_time(ui);
        self.mark(ui);
    }

    /// Whether Undo takes back the window's last change: there is one, and
    /// nothing has changed the scene since.
    pub fn takes_undo(&self, session: &Session) -> bool {
        !self.undo.is_empty() && self.revision == Some(session.revision())
    }

    pub fn takes_redo(&self, session: &Session) -> bool {
        !self.redo.is_empty() && self.revision == Some(session.revision())
    }

    /// What Undo and Redo say they would take back.
    pub fn undo_label(&self) -> Option<String> {
        self.doc.as_ref().map(|d| format!("clip {}", d.name))
    }

    /// Undo or Redo, from wherever it was asked.
    pub fn step(&mut self, ui: &mut Ui, session: &mut Session, redo: bool) {
        self.take_back(ui, session, redo);
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
            tracks.into_iter().map(|(p, w)| { let v = session.current_value(id, doc, &p, &w, time); (p, w, v) }).collect();
        self.chosen.clear();
        for (p, w, _) in &values {
            self.chosen.push(KeyRef { path: p.clone(), what: w.clone(), time });
            self.open_groups.insert(p.clone());
        }
        self.change(ui, session, |doc| {
            for (path, what, value) in values {
                let mut track = doc.track(&path, &what).cloned().unwrap_or(Track { path, what, keys: Vec::new(), ease: Default::default() });
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
                RowKind::Track(p, w) => tracks.push((p.clone(), w.clone())),
                RowKind::Group(p) => tracks.extend(doc.tracks.iter().filter(|t| t.path == *p).map(|t| (t.path.clone(), t.what.clone()))),
                RowKind::Summary => tracks.extend(doc.tracks.iter().map(|t| (t.path.clone(), t.what.clone()))),
            }
        }
        if tracks.is_empty() && doc.kind == ClipKind::Things {
            let part = self.part.clone();
            tracks = doc.tracks.iter().filter(|t| t.path == part).map(|t| (t.path.clone(), t.what.clone())).collect();
            if tracks.is_empty() {
                tracks = TRANSFORM[..6].iter().map(|w| (part.clone(), w.clone())).collect();
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
                .filter(|(_, k)| self.is_chosen(&t.path, &t.what, k.time))
                .map(|(i, _)| i)
                .collect();
            if idx.is_empty() {
                continue;
            }
            let defaults: Vec<(f32, f32)> = if self.tween_how == Blend::Default {
                idx.iter().map(|&i| (t.keys[i].time, session.default_value(id, &doc, &t.path, &t.what, t.keys[i].time))).collect()
            } else {
                Vec::new()
            };
            targets.push((t.path.clone(), t.what.clone(), idx, defaults));
        }
        self.tweening = Some((doc, targets));
        let amount = self.tween_amount(ui, x);
        self.tween_to(ui, amount);
    }

    fn tween_to(&mut self, ui: &mut Ui, amount: f32) {
        let Some((from, targets)) = &self.tweening else { return };
        let mut doc = from.clone();
        for (path, what, idx, defaults) in targets {
            let Some(t) = from.track(path, what) else { continue };
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
                .filter_map(|r| if let RowKind::Track(p, w) = r { Some((p.clone(), w.clone())) } else { None })
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

    /// Every key shown: in Curves, of the curves drawn.
    fn choose_all(&mut self) {
        let Some(doc) = &self.doc else { return };
        let tracks = if self.mode == Mode::Curves { self.curve_tracks(doc) } else { doc.tracks.clone() };
        self.chosen = tracks
            .iter()
            .flat_map(|t| t.keys.iter().map(|k| KeyRef { path: t.path.clone(), what: t.what.clone(), time: k.time }))
            .collect();
    }

    fn copy(&mut self) {
        let Some(doc) = &self.doc else { return };
        let first = self.chosen.iter().map(|c| c.time).fold(f32::MAX, f32::min);
        self.clipboard = self
            .chosen
            .iter()
            .filter_map(|c| {
                let k = doc.track(&c.path, &c.what)?.keys.iter().find(|k| (k.time - c.time).abs() < 1e-4)?;
                Some((c.path.clone(), c.what.clone(), k.time - first, k.value, k.tangent))
            })
            .collect();
    }

    fn paste(&mut self, ui: &mut Ui, session: &mut Session) {
        let keys = self.clipboard.clone();
        if keys.is_empty() {
            return;
        }
        let at = snap(self.time, false);
        self.chosen = keys.iter().map(|(p, w, dt, _, _)| KeyRef { path: p.clone(), what: w.clone(), time: at + dt }).collect();
        self.change(ui, session, |doc| {
            for (path, what, dt, value, tangent) in keys {
                let mut track = doc.track(&path, &what).cloned().unwrap_or(Track { path, what, keys: Vec::new(), ease: Default::default() });
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
        // Only when the scene changed: what is compared is a few dozen
        // numbers a thing, not to be read every frame.
        let revision = session.revision();
        if self.recorded_at == Some(revision) {
            return;
        }
        let now = session.recordable(id);
        let first = self.recorded_at.is_none();
        self.recorded_at = Some(revision);
        if first {
            self.recorded = now;
            return;
        }
        let keys: Vec<(String, Property, f32)> = now
            .iter()
            .filter(|(path, what, v)| {
                self.recorded.iter().any(|(p, w, was)| p == path && w == what && (was - v).abs() > 1e-5)
            })
            .cloned()
            .collect();
        if keys.is_empty() {
            self.recorded = now;
            return;
        }
        // The change goes back out of the scene: it was a key, not an edit.
        let _ = session.undo();
        let time = snap(self.time, false);
        self.chosen = keys.iter().map(|(p, w, _)| KeyRef { path: p.clone(), what: w.clone(), time }).collect();
        for (p, _, _) in &keys {
            self.open_groups.insert(p.clone());
        }
        self.change(ui, session, |doc| {
            for (path, what, value) in keys {
                let mut track = doc.track(&path, &what).cloned().unwrap_or(Track { path, what, keys: Vec::new(), ease: Default::default() });
                put_key(&mut track, ClipKey::shaped(time, value, Tangent::Auto));
                doc.set_track(track);
            }
        });
        self.recorded = session.recordable(id);
        self.recorded_at = Some(session.revision());
        self.posed = None;
    }

    // --- adding tracks -------------------------------------------------------

    /// The panel that adds tracks: the things or bones by name, and under
    /// the one chosen, what of it can move — its place, turn and scale,
    /// its material's numbers, its light's, its components' — a chip each,
    /// keyed at the playhead when pressed.
    fn list_adds(&mut self, ui: &mut Ui, session: &Session) {
        for (row, _, _) in self.add_rows.drain(..) {
            ui.remove(row);
        }
        let (Some(id), Some(doc)) = (self.showing, &self.doc) else { return };
        let filter = ui.text(self.add_filter).unwrap_or_default().to_lowercase();
        let things = doc.kind == ClipKind::Things;
        let names = if things { session.animatable_paths(id) } else { session.joints(id) };
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
            let open = self.add_path.as_deref() == Some(name.as_str());
            if open && !self.param_names.iter().any(|(p, _)| *p == name) {
                self.param_names.push((name.clone(), session.shader_params(id, &name)));
            }
            let row = ui.add(self.adds, Style::row().full_width().height(22.0).fixed().gap(SPACE_2).padding_x(SPACE_1).center_items().radius(RADIUS_SM).hover(HOVER).clickable());
            ui.set_name(row, format!("add to {label}"));
            icon(ui, row, if open { "chevron-down" } else { "chevron-right" }, MUTED);
            icon(ui, row, if things { "box" } else { "bone" }, if things { INFO } else { SUCCESS });
            ui.add_text(row, text().fill(), &label);
            self.add_rows.push((row, name.clone(), Vec::new()));
            if !open {
                continue;
            }
            // What of it can move, in groups: a chip adds every axis of it.
            // Groups of chips of properties, built and read right here.
            #[allow(clippy::type_complexity)]
            let mut groups: Vec<(String, Vec<(String, Vec<Property>)>)> = vec![(
                "Transform".into(),
                vec![
                    ("Position".into(), TRANSFORM[0..3].to_vec()),
                    ("Rotation".into(), TRANSFORM[3..6].to_vec()),
                    ("Scale".into(), TRANSFORM[6..9].to_vec()),
                ],
            )];
            if things {
                groups[0].1.push(("Active".into(), vec![Property::Active]));
                for (what, _) in session.animatable_numbers(id, &name) {
                    let (group, chip) = match self.param_name(&name, &what) {
                        Some((param, _)) => ("Shader".to_string(), words(&param)),
                        None => chip_of(&what),
                    };
                    let at = match groups.iter().position(|(g, _)| *g == group) {
                        Some(i) => i,
                        None => {
                            groups.push((group, Vec::new()));
                            groups.len() - 1
                        }
                    };
                    match groups[at].1.iter_mut().find(|(c, _)| *c == chip) {
                        Some((_, list)) => list.push(what),
                        None => groups[at].1.push((chip, vec![what])),
                    }
                }
            }
            for (group, chips) in groups {
                let line = ui.add(self.adds, Style::row().full_width().padding_left(46.0).gap(SPACE_1).center_items().wrap());
                ui.add_text(line, caption().width(80.0).fixed(), &group.to_uppercase());
                self.add_rows.push((line, name.clone(), Vec::new()));
                for (chip, props) in chips {
                    let b = ui.add(line, Style::row().height(20.0).fixed().padding_x(SPACE_2).center().radius(10.0).border(1.0, DIVIDER).hover(HOVER).clickable());
                    ui.set_name(b, format!("add {name} {chip}"));
                    ui.add_text(b, Style::default().text_size(11.0).text_color(TEXT).nowrap(), &chip);
                    self.add_rows.push((b, name.clone(), props));
                }
            }
        }
    }

    // --- input ---------------------------------------------------------------

    pub fn event(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, event: &Event) {
        let (shift, ctrl, alt, command) = ui.modifiers();
        let _ = ctrl;
        match event {
            Event::Submit(text) if node == self.time_field => {
                // The keyboard back to the window: Cmd-Z and the keys are
                // the clip's again, not the field's.
                ui.focus(None);
                if let Ok(t) = text.trim().parse::<f32>() {
                    self.time = t.max(0.0);
                }
                self.show_time(ui);
            }
            Event::Submit(text) if node == self.length_field => {
                ui.focus(None);
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
            Event::Press { x, y, button } => {
                // Curves pan with Alt, or the middle or right button.
                let pan = alt || *button != scrap::input::MouseButton::Left;
                self.press(ui, node, *x, *y, shift, pan)
            }
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
            self.recorded_at = None;
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
                set_word_toggle(ui, *b, *h == how);
            }
        } else if node == self.normalize {
            self.normalized = !self.normalized;
            self.range = None;
        } else if node == self.add_toggle {
            self.adding = !self.adding;
            let adding = self.adding;
            ui.restyle(self.adds, |s| if adding { s.shown() } else { s.hidden() });
            ui.set_style(self.add_toggle, word_toggle_style(adding).height(18.0).radius(RADIUS_SM).gap(3.0));
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
        } else if let Some((_, name, _)) = self.add_rows.iter().find(|(b, _, _)| *b == node).cloned() {
            self.add_path = if self.add_path.as_deref() == Some(name.as_str()) { None } else { Some(name) };
            self.list_adds(ui, session);
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
            RowKind::Track(p, w) => vec![(p.clone(), w.clone())],
            RowKind::Group(p) => doc.tracks.iter().filter(|t| t.path == *p).map(|t| (t.path.clone(), t.what.clone())).collect(),
            RowKind::Summary => doc.tracks.iter().map(|t| (t.path.clone(), t.what.clone())).collect(),
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
            RowKind::Track(p, w) => doc.track(p, w).into_iter().collect(),
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
            .map(|t| KeyRef { path: t.path.clone(), what: t.what.clone(), time })
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
                let Some(t) = tracks.iter().find(|t| t.path == c.path && t.what == c.what.clone()) else { continue };
                let Some(i) = t.keys.iter().position(|k| (k.time - c.time).abs() < 1e-4) else { continue };
                let k = t.keys[i];
                let (a, b) = self.norm(doc, t);
                let (kx, ky) = (self.x_of(k.time), self.y_of(k.value * a + b, range, rect.height));
                let (s_in, s_out) = scrap::keys::slopes(&t.keys, i);
                for (s, sign, out) in [(s_in * a, -1.0, false), (s_out * a, 1.0, true)] {
                    let (hx, hy) = self.handle_at(kx, ky, s, sign, range, rect.height);
                    if (hx - x).hypot(hy - y) <= NEAR {
                        return Some((c.clone(), Some(out)));
                    }
                }
            }
        }
        let mut best: Option<(f32, KeyRef)> = None;
        for t in &tracks {
            let (a, b) = self.norm(doc, t);
            for k in &t.keys {
                let d = (self.x_of(k.time) - x).hypot(self.y_of(k.value * a + b, range, rect.height) - y);
                if d <= NEAR && best.as_ref().is_none_or(|(b, _)| d < *b) {
                    best = Some((d, KeyRef { path: t.path.clone(), what: t.what.clone(), time: k.time }));
                }
            }
        }
        best.map(|(_, k)| (k, None))
    }

    fn press(&mut self, ui: &mut Ui, node: NodeId, x: f32, y: f32, shift: bool, pan: bool) {
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
            let hit = if pan { None } else { self.curve_hit(ui, lx, ly) };
            match hit {
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
                None if pan => self.drag = Some(Drag::Pan),
                None => {
                    if !shift {
                        self.chosen.clear();
                    }
                    self.drag = Some(Drag::Box { x0: x, y0: y, x1: x, y1: y, kept: self.chosen.clone() });
                }
            }
            return;
        }
        let Some((_, lane, kind)) = self.row_nodes.iter().find(|(_, l, _)| *l == node).cloned() else { return };
        let rect = ui.rect(lane);
        let lx = x - rect.x;
        let hit = self.keys_at(&kind, lx);
        if hit.is_empty() {
            // An empty place: a box to choose keys with, as in Unity; the
            // playhead is the ruler's.
            if !shift {
                self.chosen.clear();
            }
            self.drag = Some(Drag::Box { x0: x, y0: y, x1: x, y1: y, kept: self.chosen.clone() });
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
            Some(Drag::Box { x0, y0, kept, .. }) => {
                self.drag = Some(Drag::Box { x0, y0, x1: x, y1: y, kept: kept.clone() });
                let mut chosen = kept;
                for k in self.keys_in_box(ui, (x0.min(x), y0.min(y), x0.max(x), y0.max(y))) {
                    if !chosen.contains(&k) {
                        chosen.push(k);
                    }
                }
                self.chosen = chosen;
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
                    let mine: Vec<&KeyRef> = chosen.iter().filter(|c| c.path == t.path && c.what == t.what.clone()).collect();
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
                let old_v = from.track(&key.path, &key.what).and_then(|t| t.keys.iter().find(|k| (k.time - key.time).abs() < 1e-4)).map(|k| k.value).unwrap_or(0.0);
                let a = from.track(&key.path, &key.what).map_or(1.0, |t| self.norm(&from, t).0);
                let new_v = old_v + (self.v_of(ly, range, rect.height) - self.v_of(y0, range, rect.height)) / a;
                let mut doc = from.clone();
                if let Some(mut t) = from.track(&key.path, &key.what).cloned() {
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
                let Some(t) = from.track(&key.path, &key.what).cloned() else { return };
                let Some(i) = t.keys.iter().position(|k| (k.time - key.time).abs() < 1e-4) else { return };
                let k = t.keys[i];
                let (a, b) = self.norm(&from, &t);
                let (kx, ky) = (self.x_of(k.time), self.y_of(k.value * a + b, range, rect.height));
                let (hx, hy) = (lx - kx, ly - ky);
                if (hx.abs() < 1.0) || (out && hx <= 0.0) || (!out && hx >= 0.0) {
                    return;
                }
                let per_v = (range.1 - range.0) / rect.height.max(1.0);
                let slope = -(hy * per_v) / (hx / self.view.1) / a;
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
        let (_, ctrl, _, command) = ui.modifiers();
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
            K::A if cmd => self.choose_all(),
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

fn axis_color(what: &Property) -> Color {
    let last = |p: &str| p.rsplit('.').next().unwrap_or("").to_string();
    let axis = match what {
        Property::X | Property::TurnX | Property::ScaleX => "x".to_string(),
        Property::Y | Property::TurnY | Property::ScaleY => "y".to_string(),
        Property::Z | Property::TurnZ | Property::ScaleZ => "z".to_string(),
        Property::Material(p) | Property::Field(p) => last(p),
        _ => String::new(),
    };
    match axis.as_str() {
        "x" | "r" | "0" => AXIS_X,
        "y" | "g" | "1" => AXIS_Y,
        "z" | "b" | "2" => AXIS_Z,
        _ => match what {
            Property::Material(_) => WARNING,
            Property::Field(_) => INFO,
            _ => ACCENT,
        },
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

/// Past the clip's end, shaded over.
const PAST_END: Color = Color::hex(0x000000).alpha(28);

/// A property as the window names it: `Position X`, `Emission R`,
/// `Light · Intensity`, `Player · Speed`.
fn property_label(what: &Property) -> String {
    match what {
        Property::X => "Position X".into(),
        Property::Y => "Position Y".into(),
        Property::Z => "Position Z".into(),
        Property::TurnX => "Rotation X".into(),
        Property::TurnY => "Rotation Y".into(),
        Property::TurnZ => "Rotation Z".into(),
        Property::ScaleX => "Scale X".into(),
        Property::ScaleY => "Scale Y".into(),
        Property::ScaleZ => "Scale Z".into(),
        Property::Active => "Active".into(),
        Property::Volume => "Volume".into(),
        Property::ParticleRate => "Particle rate".into(),
        Property::Material(name) => words(name),
        Property::Field(path) => {
            let path = path.strip_prefix("components.").unwrap_or(path);
            match path.split_once('.') {
                Some((owner, rest)) => format!("{} · {}", words(owner), words(rest)),
                None => words(path),
            }
        }
    }
}

/// A path as words: `base_color.r` → `Base color R`, `params.2` →
/// `Params 2`.
fn words(path: &str) -> String {
    let mut out = String::new();
    for (i, part) in path.split(['.', '_']).filter(|p| !p.is_empty()).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut c = part.chars();
        if let Some(first) = c.next() {
            if i == 0 || part.len() == 1 {
                out.extend(first.to_uppercase());
            } else {
                out.push(first);
            }
            out.push_str(c.as_str());
        }
    }
    out
}

/// Which group and which chip of the add panel a number goes under: its
/// owner, and its name less the axis — `Material`, `Emission` for
/// `emission.g`; `Light`, `Color` for `light.color.1`.
fn chip_of(what: &Property) -> (String, String) {
    let (group, path) = match what {
        Property::Material(name) => ("Material".to_string(), name.as_str()),
        Property::Field(path) => {
            let path = path.strip_prefix("components.").unwrap_or(path);
            let (owner, rest) = path.split_once('.').unwrap_or((path, ""));
            (words(owner), rest)
        }
        other => ("Transform".to_string(), if matches!(other, Property::Active) { "active" } else { "" }),
    };
    let chip = match path.rsplit_once('.') {
        Some((head, last)) if last.len() == 1 || last.parse::<usize>().is_ok() && head != "params" => head,
        _ => path,
    };
    (group, words(chip))
}

/// A track's value as the row shows it.
fn value_text(what: &Property, v: f32) -> String {
    match what {
        Property::TurnX | Property::TurnY | Property::TurnZ => format!("{v:.1}°"),
        Property::Active => if v > 0.5 { "on".into() } else { "off".into() },
        _ if v.abs() >= 100.0 => format!("{v:.0}"),
        _ => format!("{v:.2}"),
    }
}

/// A round step for a grid that has about `rough` between its lines:
/// 1, 2 or 5 times a power of ten.
fn nice_step(rough: f32) -> f32 {
    if !rough.is_finite() || rough <= 0.0 {
        return 1.0;
    }
    let ten = 10f32.powf(rough.log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * ten).find(|s| *s >= rough).unwrap_or(10.0 * ten)
}

/// Past the clip's end, from `x` on: a shade over what is under it.
fn shade_style(x: f32) -> Style {
    Style::row().absolute(x.max(0.0), 0.0).width(100_000.0).full_height().background(PAST_END)
}
