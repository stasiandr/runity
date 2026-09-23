//! The Inspector: the selection's fields, to read and to type into.
//!
//! What the fields are and what typing into one does is
//! `Session::inspect_all` and `Session::set_field_all` — the calls an agent
//! makes, one undo step each, with `—` where several selected things
//! disagree. This lays them out as Unity does: the name on top, the
//! transform as three numbers a line, the rest in groups, each as the RON
//! the scene file holds (a form built from the game's own types waits on
//! how modules reach the editor — DNA, open question 2). Material and
//! model have a picker next to them. Empty fields are offered as chips.
//!
//! A field commits on Enter, Tab, or when the keyboard leaves it changed;
//! what does not parse is said in the Console and the box goes back to
//! what the scene holds.

use std::collections::{BTreeSet, HashMap};

use runity::EntityId;
use runity_editor::console::Level;
use runity_editor::panels::{Field, MIXED};
use runity_editor::Session;
use runity_ui::{Event, NodeId, Style, Ui};

use crate::bottom::Asset;
use crate::menu::{Action, MenuItem};

/// The picture the Inspector previews an asset in.
pub const PREVIEW: runity_ui::ImageId = runity_ui::ImageId(1);
use crate::studio::Requests;
use crate::theme::*;

const OBJECT: [&str; 4] = ["model", "material", "prefab", "layer"];
const TRANSFORM: [&str; 3] = ["position", "rotation", "scale"];
const PHYSICS: [&str; 4] = ["body", "collider", "physics", "joint"];
const PARTS: [&str; 4] = ["camera", "light", "particles", "route"];

/// A field that says nothing: not shown, offered as a chip.
fn is_empty(value: &str) -> bool {
    matches!(value, "" | "None" | "r#None" | "()" | "\"\"")
}

/// A vector field's three numbers, when its text is one.
fn axes(value: &str) -> Option<[String; 3]> {
    let inner = value.trim().strip_prefix('(')?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 3 || parts.iter().any(|p| p.parse::<f32>().is_err()) {
        return None;
    }
    Some([0, 1, 2].map(|i| trim_number(parts[i])))
}

/// `2.0` as `2`, `0.30000001` as `0.3`: what a person would have typed.
fn trim_number(text: &str) -> String {
    let Ok(n) = text.parse::<f32>() else {
        return text.to_string();
    };
    let s = format!("{:.4}", n);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

fn title(field: &str) -> String {
    let name = field.strip_prefix("components.").unwrap_or(field);
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// What a node of the panel stands for.
#[derive(Debug, Clone, PartialEq)]
enum Part {
    /// A box of a field; `axis` for one number of a vector.
    Slot {
        field: String,
        axis: Option<usize>,
    },
    /// The dot on an overridden field: revert it.
    Revert(String),
    /// A chip offering an empty field.
    Reveal(String),
    /// The «…» next to a field with a list to pick from.
    Pick(String),
    AddComponent,
    /// The list of the game's components.
    PickComponent,
    /// A field's label: a click opens its menu — Reset, Copy, Paste,
    /// Remove — as Unity's ⋮ on a component.
    Label(String),
    /// The material's colour as `#rrggbb`.
    Hex,
    /// One of the colour's hue, saturation and value, 0 to 1.
    Slider(usize),
    /// An import setting of the asset shown, by name: a box.
    Import(String),
    /// An import setting that is on or off: a switch.
    ImportToggle(String, bool),
    /// One field of a game component, laid out by the shape the game wrote
    /// down (`library/components.ron`).
    Sub {
        component: String,
        key: String,
        kind: SubKind,
    },
    /// The scene's sun or fog, as RON.
    Environment(String),
    /// The sun's hour, 0 to 24, as a track.
    Hour,
}

/// What a component's field is, for the box it gets.
#[derive(Debug, Clone, PartialEq)]
enum SubKind {
    Bool(bool),
    Number,
    Text,
    Enum(Vec<String>),
    /// A link to another entity: a picker, as Unity's object field.
    Entity,
    Raw,
}

/// A struct's fields as RON writes them — `(open_angle: 90.0, locked:
/// true)` — split at the top level, brackets and strings respected.
fn struct_fields(text: &str) -> Option<Vec<(String, String)>> {
    let inner = text.trim().strip_prefix('(')?.strip_suffix(')')?;
    let mut out = Vec::new();
    let (mut depth, mut quoted, mut start) = (0i32, false, 0usize);
    let bytes: Vec<(usize, char)> = inner.char_indices().collect();
    let mut parts = Vec::new();
    for (i, c) in &bytes {
        match c {
            '"' => quoted = !quoted,
            '(' | '[' | '{' if !quoted => depth += 1,
            ')' | ']' | '}' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => {
                parts.push(&inner[start..*i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (key, value) = part.split_once(':')?;
        out.push((key.trim().to_string(), value.trim().to_string()));
    }
    Some(out)
}

/// sRGB bytes of a linear colour.
fn to_srgb(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| (runity::material::linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0).round() as u8)
}

fn hsv_of(rgb: [u8; 3]) -> [f32; 3] {
    let [r, g, b] = rgb.map(|v| v as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    [h, s, max]
}

fn rgb_of(hsv: [f32; 3]) -> [u8; 3] {
    let [h, s, v] = hsv;
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [r, g, b].map(|c| (c * 255.0).round() as u8)
}

fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let t = text.trim().trim_start_matches('#');
    if t.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(t, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

pub struct Inspector {
    /// The panel's content, for a dock to hold.
    pub root: NodeId,
    body: NodeId,
    /// Whose fields are shown, and the shape they were laid out in.
    showing: Vec<EntityId>,
    shape: Vec<(String, Option<usize>)>,
    parts: HashMap<NodeId, Part>,
    /// Slots by field and axis, and what each was last given.
    slots: Vec<(String, Option<usize>, NodeId, String)>,
    revealed: BTreeSet<String>,
    playing: bool,
    /// The material editor's colour while it is being dragged, and its
    /// nodes: the swatch, the hex box, the three tracks.
    hsv: [f32; 3],
    swatch: Option<NodeId>,
    hex: Option<NodeId>,
    tracks: [Option<NodeId>; 3],
    /// An asset from the Project shown instead of the selection, with its
    /// source file.
    asset: Option<(Asset, Option<String>)>,
    /// What the game's components look like, as it last wrote them.
    shapes: std::collections::BTreeMap<String, runity::shape::Shape>,
    /// Each component's fields as last shown, to write one back changed.
    component_values: HashMap<String, Vec<(String, String)>>,
    /// The hour being dragged to.
    hour: f32,
    /// Built at least once: an empty selection at the start is still a
    /// panel to build.
    built: bool,
    /// Unity's padlock: the entities shown whatever is selected after.
    locked: Option<Vec<EntityId>>,
    lock_button: NodeId,
}

impl Inspector {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let card = ui.add(parent, Style::column().fill().full_width());
        ui.set_name(card, "inspector");
        // A strip over the fields for the padlock.
        let strip = ui.add(
            card,
            Style::row()
                .full_width()
                .height(22.0)
                .fixed()
                .padding_x(SPACE_2)
                .center_items(),
        );
        spacer(ui, strip);
        let lock_button = icon_button(ui, strip, "inspector lock", "lock-open", false);
        let body = ui.add(card, Style::column().fill().full_width().clip());
        Self {
            locked: None,
            lock_button,
            root: card,
            body,
            showing: Vec::new(),
            shape: Vec::new(),
            parts: HashMap::new(),
            slots: Vec::new(),
            revealed: BTreeSet::new(),
            playing: false,
            built: false,
            hour: 12.0,
            shapes: Default::default(),
            component_values: HashMap::new(),
            asset: None,
            hsv: [0.0; 3],
            swatch: None,
            hex: None,
            tracks: [None; 3],
        }
    }

    pub fn owns(&self, node: NodeId) -> bool {
        node == self.lock_button || self.parts.contains_key(&node)
    }

    /// What the Inspector edits: the locked entities that still exist, or
    /// the selection.
    pub fn targets(&self, session: &Session) -> Vec<EntityId> {
        match &self.locked {
            Some(ids) => ids
                .iter()
                .copied()
                .filter(|id| session.inspect(*id).is_some())
                .collect(),
            None => session.selection(),
        }
    }

    /// Lock on what is shown, or let go.
    pub fn toggle_lock(&mut self, ui: &mut Ui) {
        self.locked = match self.locked {
            Some(_) => None,
            None if !self.showing.is_empty() => Some(self.showing.clone()),
            None => None,
        };
        let on = self.locked.is_some();
        set_icon_button(
            ui,
            self.lock_button,
            if on { "lock" } else { "lock-open" },
            on,
            true,
        );
    }

    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        if self.asset.is_some() {
            return;
        }
        let mut ids = self.targets(session);
        if self.locked.is_some() && ids.is_empty() {
            // What it was locked on is gone: back to the selection.
            self.toggle_lock(ui);
            ids = session.selection();
        }
        let fields = session.inspect_all(&ids).unwrap_or_default();
        let playing = session.is_playing();
        if ids != self.showing {
            self.revealed.clear();
        }
        // A material means nothing without a model to wear it: an empty,
        // a camera, a light.
        let no_model = fields
            .iter()
            .any(|f| f.name == "model" && f.value.is_empty());
        let shown = |f: &Field| {
            (!is_empty(&f.value) && !(no_model && f.name == "material"))
                || self.revealed.contains(&f.name)
        };
        let mut shape = Vec::new();
        for f in &fields {
            if f.name.starts_with("game")
                || !(TRANSFORM.contains(&f.name.as_str()) || f.name == "name" || shown(f))
            {
                continue;
            }
            if TRANSFORM.contains(&f.name.as_str()) && axes(&f.value).is_some() {
                for i in 0..3 {
                    shape.push((f.name.clone(), Some(i)));
                }
            } else {
                shape.push((f.name.clone(), None));
            }
        }
        if !self.built || ids != self.showing || shape != self.shape || playing != self.playing {
            self.built = true;
            self.showing = ids.clone();
            self.shape = shape;
            self.playing = playing;
            self.build(ui, session, &ids, &fields);
            return;
        }
        // Same boxes: new text in the ones nobody is typing into.
        for (field, axis, node, given) in &mut self.slots {
            let Some(f) = fields.iter().find(|f| f.name == *field) else {
                continue;
            };
            let text = match axis {
                Some(i) => axes(&f.value).map(|a| a[*i].clone()).unwrap_or_default(),
                None => f.value.clone(),
            };
            if *given != text && ui.focused() != Some(*node) {
                ui.set_text(*node, &text);
                *given = text;
            }
        }
    }

    fn build(&mut self, ui: &mut Ui, session: &Session, ids: &[EntityId], fields: &[Field]) {
        ui.clear(self.body);
        self.parts.clear();
        self.slots.clear();
        self.swatch = None;
        self.hex = None;
        self.tracks = [None; 3];
        if ids.is_empty() {
            let empty = ui.add(self.body, Style::column().padding(SPACE_4).gap(SPACE_2));
            ui.add_text(empty, text().text_color(MUTED), "Nothing selected");
            ui.add_text(
                empty,
                Style::default().text_size(12.0).text_color(TEXT.alpha(40)),
                "Click something in the Scene view or the Hierarchy.",
            );
            self.environment(ui, session);
            return;
        }
        self.shapes = session.component_shapes();
        self.component_values.clear();
        let find = |name: &str| fields.iter().find(|f| f.name == name);
        let prefab = find("prefab")
            .map(|f| f.value.clone())
            .filter(|p| !p.is_empty() && p != MIXED);

        // The header: what it is, its name, how many.
        let head = ui.add(
            self.body,
            Style::row()
                .full_width()
                .padding_x(SPACE_4)
                .padding_y(SPACE_1)
                .gap(SPACE_2)
                .center_items(),
        );
        icon(
            ui,
            head,
            if prefab.is_some() { "package" } else { "box" },
            ACCENT,
        );
        if let Some(name) = find("name") {
            let f = ui.add_field(
                head,
                field_style().fill().height(26.0).text_size(13.0),
                &name.value,
            );
            ui.set_name(f, "inspector name");
            self.slot(f, "name", None, &name.value);
        }
        if ids.len() > 1 {
            tag(
                ui,
                head,
                &format!("{} selected", ids.len()),
                ACCENT_800,
                ACCENT_100,
            );
        }
        if let Some(p) = &prefab {
            tag(ui, head, p, ACCENT_900, ACCENT_300);
        }
        if self.playing {
            let note = ui.add(
                self.body,
                Style::row()
                    .margin(SPACE_4)
                    .padding(SPACE_3)
                    .radius(RADIUS_MD)
                    .border(1.0, ACCENT.alpha(40)),
            );
            ui.add_text(
                note,
                Style::default().text_size(11.5).text_color(ACCENT_300),
                "Playing: this is where things are now. Stop brings the scene back.",
            );
        }

        let no_model = fields
            .iter()
            .any(|f| f.name == "model" && f.value.is_empty());
        let shown = |f: &&Field| {
            (!is_empty(&f.value) && !(no_model && f.name == "material"))
                || self.revealed.contains(&f.name)
        };
        let transform: Vec<&Field> = TRANSFORM.iter().filter_map(|n| find(n)).collect();
        let object: Vec<&Field> = OBJECT
            .iter()
            .filter_map(|n| find(n))
            .filter(shown)
            .collect();
        let physics: Vec<&Field> = PHYSICS
            .iter()
            .filter_map(|n| find(n))
            .filter(shown)
            .collect();
        let mut parts: Vec<&Field> = PARTS.iter().filter_map(|n| find(n)).filter(shown).collect();
        parts.extend(fields.iter().filter(|f| f.name.starts_with("components.")));
        let game: Vec<&Field> = fields
            .iter()
            .filter(|f| f.name.starts_with("game"))
            .collect();

        for (name, group) in [
            ("Transform", transform),
            ("Object", object),
            ("Physics", physics),
            ("Components", parts),
        ] {
            if group.is_empty() {
                continue;
            }
            self.heading(ui, name);
            for f in group {
                self.line(ui, session, f);
                if f.name == "material" && f.value != MIXED {
                    if let Some(m) = session.material(ids[0]) {
                        self.color_editor(ui, m.base_color);
                    }
                }
            }
        }
        if !game.is_empty() {
            self.heading(ui, "Running game");
            for f in game {
                let line = ui.add(
                    self.body,
                    Style::row().full_width().padding_x(SPACE_4).gap(SPACE_2),
                );
                ui.add_text(
                    line,
                    Style::default()
                        .width(84.0)
                        .fixed()
                        .text_size(11.5)
                        .text_color(LABEL)
                        .nowrap(),
                    &title(f.name.strip_prefix("game.").unwrap_or(&f.name)),
                );
                ui.add_text(
                    line,
                    Style::default()
                        .fill()
                        .text_size(11.5)
                        .text_color(MUTED)
                        .mono(),
                    &f.value,
                );
            }
        }

        // What could be added: the empty fields as outlined chips, and a
        // component by name.
        let foot = ui.add(
            self.body,
            Style::column()
                .full_width()
                .padding_x(SPACE_4)
                .padding_y(SPACE_3)
                .gap(SPACE_2),
        );
        let chips = ui.add(foot, Style::row().wrap().gap(SPACE_1).full_width());
        for name in OBJECT.iter().chain(PHYSICS.iter()).chain(PARTS.iter()) {
            if *name == "prefab" {
                continue;
            }
            let Some(f) = find(name) else { continue };
            if !is_empty(&f.value) || self.revealed.contains(*name) {
                continue;
            }
            let chip = ui.add(
                chips,
                Style::row()
                    .height(22.0)
                    .padding_x(SPACE_2)
                    .gap(4.0)
                    .center_items()
                    .radius(6.0)
                    .border(1.0, DIVIDER)
                    .hover(HOVER)
                    .hover_border(ACCENT),
            );
            ui.set_name(chip, format!("add {name}"));
            icon(ui, chip, "plus", MUTED);
            ui.add_text(
                chip,
                Style::default().text_size(11.5).text_color(LABEL).nowrap(),
                &title(name),
            );
            self.parts.insert(chip, Part::Reveal(name.to_string()));
        }
        let add_row = ui.add(foot, Style::row().full_width().gap(SPACE_1).center_items());
        let add = ui.add_field(add_row, field_style().fill(), "");
        // The components the game has: a list to pick from, as Unity's Add
        // Component button.
        let pick = ui.add(
            add_row,
            Style::row()
                .size(22.0, 22.0)
                .fixed()
                .center()
                .radius(6.0)
                .hover(HOVER),
        );
        ui.set_name(pick, "pick component");
        icon(ui, pick, "plus", LABEL);
        self.parts.insert(pick, Part::PickComponent);
        ui.set_name(add, "add component");
        ui.set_placeholder(add, "Add component by name, Enter");
        self.parts.insert(add, Part::AddComponent);
        let _ = session;
    }

    fn slot(&mut self, node: NodeId, field: &str, axis: Option<usize>, value: &str) {
        self.parts.insert(
            node,
            Part::Slot {
                field: field.to_string(),
                axis,
            },
        );
        self.slots
            .push((field.to_string(), axis, node, value.to_string()));
    }

    /// A group's heading with the rule after it fading out.
    fn heading(&mut self, ui: &mut Ui, name: &str) {
        let h = ui.add(
            self.body,
            Style::row()
                .full_width()
                .padding_x(SPACE_4)
                .padding_y(SPACE_2)
                .gap(SPACE_2)
                .center_items(),
        );
        ui.add_text(
            h,
            Style::default()
                .text_size(12.0)
                .text_color(NEUTRAL_300)
                .nowrap(),
            name,
        );
        ui.add(h, Style::row().fill().height(1.0).background(DIVIDER));
    }

    /// One field: its label (with the override dot) and its boxes.
    fn line(&mut self, ui: &mut Ui, session: &Session, f: &Field) {
        if self.component_form(ui, session, f) {
            return;
        }
        let line = ui.add(
            self.body,
            Style::row()
                .full_width()
                .padding_x(SPACE_4)
                .padding_y(2.0)
                .gap(SPACE_2)
                .center_items(),
        );
        let label = ui.add(
            line,
            Style::row()
                .width(84.0)
                .fixed()
                .gap(SPACE_2)
                .center_items()
                .radius(RADIUS_SM)
                .hover(HOVER),
        );
        ui.set_name(label, format!("label {}", f.name));
        self.parts.insert(label, Part::Label(f.name.clone()));
        if f.overridden {
            let dot = ui.add(
                label,
                Style::row()
                    .size(8.0, 8.0)
                    .radius(4.0)
                    .background(ACCENT)
                    .clickable(),
            );
            self.parts.insert(dot, Part::Revert(f.name.clone()));
        }
        ui.add_text(
            label,
            Style::default()
                .text_size(12.0)
                .text_color(if f.overridden { ACCENT_300 } else { LABEL })
                .nowrap(),
            &title(&f.name),
        );
        match TRANSFORM
            .contains(&f.name.as_str())
            .then(|| axes(&f.value))
            .flatten()
        {
            Some(three) => {
                let boxes = ui.add(line, Style::row().fill().gap(SPACE_1));
                for (i, value) in three.iter().enumerate() {
                    let b = ui.add(boxes, Style::row().fill().gap(2.0).center_items());
                    ui.add_text(
                        b,
                        Style::default()
                            .text_size(10.5)
                            .text_color(TEXT.alpha(40))
                            .nowrap(),
                        ["X", "Y", "Z"][i],
                    );
                    let slot = ui.add_field(b, field_style().fill(), value);
                    ui.set_name(slot, format!("{} {}", f.name, ["x", "y", "z"][i]));
                    self.slot(slot, &f.name, Some(i), value);
                }
            }
            None => {
                // A long value — a light, a route, a component with no
                // shape to go by — gets room: several lines, wrapping.
                let long = f.value.len() > 36 || f.name.starts_with("components.");
                let slot = if long {
                    ui.add_textarea(
                        line,
                        field_style()
                            .fill()
                            .auto_height()
                            .min_height(22.0)
                            .padding_y(3.0)
                            .mono()
                            .text_size(11.5),
                        &f.value,
                    )
                } else {
                    ui.add_field(line, field_style().fill().mono().text_size(11.5), &f.value)
                };
                ui.set_name(slot, f.name.clone());
                self.slot(slot, &f.name, None, &f.value);
                if matches!(f.name.as_str(), "material" | "model" | "prefab") {
                    let pick = ui.add(
                        line,
                        Style::row()
                            .size(22.0, 22.0)
                            .fixed()
                            .center()
                            .radius(6.0)
                            .hover(HOVER),
                    );
                    ui.set_name(pick, format!("pick {}", f.name));
                    icon(ui, pick, "ellipsis-vertical", LABEL);
                    self.parts.insert(pick, Part::Pick(f.name.clone()));
                }
            }
        }
    }

    /// A game component laid out by its shape: a line per field, each with
    /// the box its type wants. `false` when there is no shape to go by —
    /// the component is then one line of RON, as before.
    fn component_form(&mut self, ui: &mut Ui, session: &Session, f: &Field) -> bool {
        use runity::shape::Shape;
        let Some(component) = f.name.strip_prefix("components.") else {
            return false;
        };
        let Some(Shape::Struct(shape)) = self.shapes.get(component).cloned() else {
            return false;
        };
        let Some(values) = struct_fields(&f.value) else {
            return false;
        };
        let head = ui.add(
            self.body,
            Style::row()
                .full_width()
                .padding_x(SPACE_4)
                .padding_y(2.0)
                .gap(SPACE_2)
                .center_items(),
        );
        icon(ui, head, "component", ACCENT);
        self.parts.insert(head, Part::Label(f.name.clone()));
        ui.restyle(head, |s| s.hover(HOVER));
        ui.set_name(head, format!("label {}", f.name));
        ui.add_text(
            head,
            Style::default()
                .text_size(12.0)
                .text_color(if f.overridden { ACCENT_300 } else { TEXT })
                .nowrap(),
            &title(&f.name),
        );
        for (key, kind) in &shape {
            let value = values
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| kind.example());
            let line = ui.add(
                self.body,
                Style::row()
                    .full_width()
                    .padding_left(SPACE_4 + 18.0)
                    .padding_x(SPACE_4)
                    .padding_left(SPACE_4 + 18.0)
                    .padding_y(2.0)
                    .gap(SPACE_2)
                    .center_items(),
            );
            ui.add_text(
                line,
                Style::default()
                    .width(84.0 - 18.0)
                    .fixed()
                    .text_size(12.0)
                    .text_color(LABEL)
                    .nowrap(),
                &title(key),
            );
            let sub = match kind {
                Shape::Bool => SubKind::Bool(value == "true"),
                Shape::Int | Shape::Float => SubKind::Number,
                Shape::Text => SubKind::Text,
                Shape::Enum(variants) => SubKind::Enum(variants.clone()),
                Shape::Entity => SubKind::Entity,
                _ => SubKind::Raw,
            };
            let node = match &sub {
                SubKind::Bool(on) => {
                    let switch = ui.add(
                        line,
                        Style::row()
                            .size(34.0, 18.0)
                            .radius(9.0)
                            .padding(2.0)
                            .border(1.0, if *on { ACCENT } else { DIVIDER })
                            .background(if *on {
                                ACCENT.alpha(25)
                            } else {
                                runity_ui::Color::TRANSPARENT
                            })
                            .clickable(),
                    );
                    if *on {
                        ui.add(switch, Style::row().fill());
                    }
                    ui.add(
                        switch,
                        Style::row()
                            .size(12.0, 12.0)
                            .radius(6.0)
                            .background(if *on { ACCENT } else { MUTED }),
                    );
                    switch
                }
                SubKind::Enum(_) => {
                    let pick = ui.add(
                        line,
                        Style::row()
                            .fill()
                            .height(22.0)
                            .padding_x(6.0)
                            .gap(SPACE_2)
                            .center_items()
                            .radius(6.0)
                            .border(1.0, DIVIDER)
                            .hover(HOVER),
                    );
                    ui.add_text(pick, text().fill(), &value);
                    icon(ui, pick, "chevron-down", MUTED);
                    pick
                }
                SubKind::Entity => {
                    // The linked entity by name, or None; a menu to pick.
                    let target = runity::EntityRef::find_in(&value).first().copied();
                    let (label, known) = match target {
                        Some(id) => match session.entity_name(id) {
                            Some(name) => (name, true),
                            None => (format!("missing {id}"), false),
                        },
                        None => ("None (entity)".to_string(), true),
                    };
                    let pick = ui.add(
                        line,
                        Style::row()
                            .fill()
                            .height(22.0)
                            .padding_x(6.0)
                            .gap(SPACE_2)
                            .center_items()
                            .radius(6.0)
                            .border(1.0, if known { DIVIDER } else { ERROR })
                            .hover(HOVER)
                            .clickable(),
                    );
                    icon(
                        ui,
                        pick,
                        "crosshair",
                        if target.is_some() { ACCENT } else { MUTED },
                    );
                    ui.add_text(
                        pick,
                        text()
                            .fill()
                            .nowrap()
                            .text_color(if known { TEXT } else { ERROR }),
                        &label,
                    );
                    icon(ui, pick, "chevron-down", MUTED);
                    pick
                }
                SubKind::Text => {
                    let shown = value.trim_matches('"').to_string();
                    ui.add_field(line, field_style().fill(), &shown)
                }
                SubKind::Number => ui.add_field(
                    line,
                    field_style().fill().mono().text_size(11.5),
                    &trim_number(&value),
                ),
                SubKind::Raw => {
                    ui.add_field(line, field_style().fill().mono().text_size(11.5), &value)
                }
            };
            ui.set_name(node, format!("{component} {key}"));
            self.parts.insert(
                node,
                Part::Sub {
                    component: component.to_string(),
                    key: key.clone(),
                    kind: sub,
                },
            );
        }
        self.component_values.insert(component.to_string(), values);
        true
    }

    /// Write one field of a component back: the whole value again, that
    /// field changed, in the shape's order.
    fn set_sub(&mut self, session: &mut Session, component: &str, key: &str, value: &str) {
        let Some(runity::shape::Shape::Struct(shape)) = self.shapes.get(component).cloned() else {
            return;
        };
        let values = self
            .component_values
            .get(component)
            .cloned()
            .unwrap_or_default();
        let text = format!(
            "({})",
            shape
                .iter()
                .map(|(k, kind)| {
                    let v = if k == key {
                        value.to_string()
                    } else {
                        values
                            .iter()
                            .find(|(vk, _)| vk == k)
                            .map(|(_, v)| v.clone())
                            .unwrap_or_else(|| kind.example())
                    };
                    format!("{k}: {v}")
                })
                .collect::<Vec<_>>()
                .join(", ")
        );
        if let Err(e) =
            session.set_field_all(&self.showing, &format!("components.{component}"), &text)
        {
            session.say(Level::Error, e.to_string());
        }
        self.built = false;
    }

    /// The scene's own settings, shown when nothing is selected: the time
    /// of day as a track, the sun and the fog as the file writes them —
    /// Unity's Lighting window, where it is at hand.
    fn environment(&mut self, ui: &mut Ui, session: &Session) {
        self.heading(ui, "Scene");
        let env = session.environment();
        let hour = env
            .iter()
            .find(|(f, _)| *f == "sun")
            .and_then(|(_, v)| {
                let at = v.find("hour:")? + 5;
                v[at..].split([',', ')']).next()?.trim().parse::<f32>().ok()
            })
            .unwrap_or(12.0);
        let line = ui.add(
            self.body,
            Style::row()
                .full_width()
                .padding_x(SPACE_4)
                .padding_y(2.0)
                .gap(SPACE_2)
                .center_items(),
        );
        ui.add_text(
            line,
            Style::default()
                .width(84.0)
                .fixed()
                .text_size(12.0)
                .text_color(LABEL)
                .nowrap(),
            "Time of day",
        );
        let track = ui.add(
            line,
            Style::row()
                .fill()
                .height(6.0)
                .radius(3.0)
                .background(DIVIDER)
                .draggable()
                .clickable(),
        );
        ui.set_name(track, "sun hour");
        ui.add(
            track,
            Style::row()
                .full_height()
                .radius(3.0)
                .background(ACCENT.alpha(70))
                .width_fraction(hour / 24.0),
        );
        ui.add_text(
            line,
            Style::default()
                .width(40.0)
                .fixed()
                .text_size(11.5)
                .text_color(MUTED)
                .nowrap(),
            &format!("{:02}:{:02}", hour as u32, ((hour.fract()) * 60.0) as u32),
        );
        self.parts.insert(track, Part::Hour);
        for (field, value) in env {
            let line = ui.add(
                self.body,
                Style::row()
                    .full_width()
                    .padding_x(SPACE_4)
                    .padding_y(2.0)
                    .gap(SPACE_2)
                    .center_items(),
            );
            ui.add_text(
                line,
                Style::default()
                    .width(84.0)
                    .fixed()
                    .text_size(12.0)
                    .text_color(LABEL)
                    .nowrap(),
                &title(field),
            );
            let f = ui.add_field(line, field_style().fill().mono().text_size(11.5), &value);
            ui.set_name(f, format!("scene {field}"));
            self.parts.insert(f, Part::Environment(field.to_string()));
        }
    }

    /// Go back to showing the selection.
    pub fn clear_asset(&mut self) {
        if self.asset.take().is_some() {
            self.built = false;
        }
    }

    /// Show an asset from the Project: its picture, how it is imported,
    /// where it is used — Unity's Inspector on a selected asset. Returns
    /// the preview's pixels (256 square) for the renderer, when it has one.
    pub fn show_asset(
        &mut self,
        ui: &mut Ui,
        session: &mut Session,
        asset: Asset,
    ) -> Option<Vec<u8>> {
        ui.clear(self.body);
        self.parts.clear();
        self.slots.clear();
        self.swatch = None;
        self.hex = None;
        self.tracks = [None; 3];
        self.showing.clear();
        let (name, kind, file) = match &asset {
            Asset::Model(n, f) => (n.clone(), "model", f.clone()),
            Asset::Prefab(n) => (n.clone(), "prefab", Some(format!("prefabs/{n}.prefab"))),
            Asset::Material(n) => (n.clone(), "material", None),
            Asset::Sound(n, f) => (n.clone(), "sound", Some(f.clone())),
            Asset::Scene(p) => (
                p.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                "scene",
                None,
            ),
        };
        let head = ui.add(
            self.body,
            Style::row()
                .full_width()
                .padding_x(SPACE_4)
                .padding_y(SPACE_1)
                .gap(SPACE_2)
                .center_items(),
        );
        icon(
            ui,
            head,
            match kind {
                "prefab" => "package",
                "material" => "sparkles",
                "scene" => "mountain",
                _ => "box",
            },
            ACCENT,
        );
        ui.add_text(head, text().fill().text_size(13.0), &name);
        tag(ui, head, kind, ACCENT_900, ACCENT_300);

        let pixels = match kind {
            "model" | "prefab" => session.thumbnail(&name, 256).ok(),
            _ => None,
        };
        if pixels.is_some() {
            let frame = ui.add(
                self.body,
                Style::row()
                    .full_width()
                    .padding_x(SPACE_4)
                    .padding_y(SPACE_2),
            );
            let img = ui.add_image(
                frame,
                Style::default().size(200.0, 200.0).radius(RADIUS_MD),
                PREVIEW,
            );
            ui.set_name(img, "asset preview");
        }
        if kind == "material" {
            if let Some((_, m)) = session.palette().into_iter().find(|(n, _)| *n == name) {
                let rgb = to_srgb(m.base_color);
                let line = ui.add(
                    self.body,
                    Style::row()
                        .full_width()
                        .padding_x(SPACE_4)
                        .gap(SPACE_2)
                        .center_items(),
                );
                ui.add(
                    line,
                    Style::row()
                        .size(40.0, 40.0)
                        .radius(RADIUS_MD)
                        .border(1.0, DIVIDER)
                        .background(runity_ui::Color::rgba(rgb[0], rgb[1], rgb[2], 255)),
                );
                ui.add_text(
                    line,
                    text().mono(),
                    &format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]),
                );
            }
        }
        // How it is imported: a model with a source in the project.
        if let (Some(source), "model") = (&file, kind) {
            if let Ok(settings) = session.import_settings(source) {
                self.heading(ui, "Import");
                let line = ui.add(
                    self.body,
                    Style::row()
                        .full_width()
                        .padding_x(SPACE_4)
                        .padding_y(2.0)
                        .gap(SPACE_2)
                        .center_items(),
                );
                ui.add_text(
                    line,
                    Style::default()
                        .width(120.0)
                        .fixed()
                        .text_size(12.0)
                        .text_color(LABEL)
                        .nowrap(),
                    "Scale",
                );
                let f = ui.add_field(
                    line,
                    field_style().fill().mono(),
                    &trim_number(&settings.scale.to_string()),
                );
                ui.set_name(f, "import scale");
                self.parts.insert(f, Part::Import("scale".into()));
                for (field, label, on) in [
                    (
                        "recompute_normals",
                        "Recompute normals",
                        settings.recompute_normals,
                    ),
                    ("srgb", "Colour is sRGB", settings.srgb),
                    (
                        "origin_to_base",
                        "Origin at the base",
                        settings.origin_to_base,
                    ),
                ] {
                    let line = ui.add(
                        self.body,
                        Style::row()
                            .full_width()
                            .padding_x(SPACE_4)
                            .padding_y(2.0)
                            .gap(SPACE_2)
                            .center_items(),
                    );
                    ui.add_text(
                        line,
                        Style::default()
                            .width(120.0)
                            .fixed()
                            .text_size(12.0)
                            .text_color(LABEL)
                            .nowrap(),
                        label,
                    );
                    let switch = ui.add(
                        line,
                        Style::row()
                            .size(34.0, 18.0)
                            .radius(9.0)
                            .padding(2.0)
                            .border(1.0, if on { ACCENT } else { DIVIDER })
                            .background(if on {
                                ACCENT.alpha(25)
                            } else {
                                runity_ui::Color::TRANSPARENT
                            })
                            .clickable(),
                    );
                    ui.set_name(switch, format!("import {field}"));
                    if on {
                        ui.add(switch, Style::row().fill());
                    }
                    ui.add(
                        switch,
                        Style::row().size(12.0, 12.0).radius(6.0).background(if on {
                            ACCENT
                        } else {
                            MUTED
                        }),
                    );
                    self.parts
                        .insert(switch, Part::ImportToggle(field.into(), on));
                }
            }
        }
        // Where it is used.
        if let Some(f) = &file {
            if let Ok(usages) = session.asset_usages(f) {
                self.heading(ui, &format!("Used in ({})", usages.len()));
                for u in usages.iter().take(30) {
                    let line = ui.add(
                        self.body,
                        Style::row().full_width().padding_x(SPACE_4).padding_y(1.0),
                    );
                    ui.add_text(
                        line,
                        Style::default().text_size(11.5).text_color(MUTED).nowrap(),
                        &u.to_string(),
                    );
                }
            }
        }
        self.asset = Some((asset, file));
        self.built = true;
        pixels
    }

    /// The material's colour: a swatch, `#rrggbb`, and hue, saturation
    /// and value tracks. Unity's colour field, flattened into the panel.
    fn color_editor(&mut self, ui: &mut Ui, linear: [f32; 3]) {
        let rgb = to_srgb(linear);
        self.hsv = hsv_of(rgb);
        let line = ui.add(
            self.body,
            Style::row()
                .full_width()
                .padding_x(SPACE_4)
                .padding_y(2.0)
                .gap(SPACE_2)
                .center_items(),
        );
        ui.add(line, Style::row().width(84.0).fixed());
        let swatch = ui.add(
            line,
            Style::row()
                .size(22.0, 22.0)
                .fixed()
                .radius(6.0)
                .border(1.0, DIVIDER)
                .background(runity_ui::Color::rgba(rgb[0], rgb[1], rgb[2], 255)),
        );
        ui.set_name(swatch, "material swatch");
        let hex = ui.add_field(
            line,
            field_style().width(86.0).mono().text_size(11.5),
            &format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]),
        );
        ui.set_name(hex, "material hex");
        self.parts.insert(hex, Part::Hex);
        let tracks = ui.add(line, Style::column().fill().gap(3.0));
        for (i, name) in ["hue", "saturation", "value"].into_iter().enumerate() {
            let track = ui.add(
                tracks,
                Style::row()
                    .full_width()
                    .height(6.0)
                    .radius(3.0)
                    .background(DIVIDER)
                    .draggable()
                    .clickable(),
            );
            ui.set_name(track, format!("material {name}"));
            ui.add(
                track,
                Style::row()
                    .full_height()
                    .radius(3.0)
                    .background(ACCENT.alpha(70)),
            );
            self.parts.insert(track, Part::Slider(i));
            self.tracks[i] = Some(track);
        }
        self.swatch = Some(swatch);
        self.hex = Some(hex);
        self.show_color(ui);
    }

    /// The swatch, the hex and the tracks, for the colour being edited.
    fn show_color(&mut self, ui: &mut Ui) {
        let rgb = rgb_of(self.hsv);
        if let Some(sw) = self.swatch {
            ui.restyle(sw, |s| {
                s.background(runity_ui::Color::rgba(rgb[0], rgb[1], rgb[2], 255))
            });
        }
        if let Some(hex) = self.hex {
            if ui.focused() != Some(hex) {
                ui.set_text(hex, &format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]));
            }
        }
        for (i, track) in self.tracks.iter().enumerate() {
            let Some(track) = track else { continue };
            if let Some(fill) = ui.children(*track).first().copied() {
                let v = self.hsv[i];
                ui.restyle(fill, |s| s.width_fraction(v.max(0.04)));
            }
        }
    }

    /// Give the selection the colour being edited: one undo step.
    fn apply_color(&mut self, session: &mut Session) {
        let [r, g, b] = rgb_of(self.hsv);
        let material = runity::Material::from_srgb(r, g, b);
        for id in self.showing.clone() {
            let mut m = session.material(id).unwrap_or(material);
            m.base_color = material.base_color;
            if let Err(e) = session.set_material(id, m) {
                session.say(Level::Error, e.to_string());
                break;
            }
        }
    }

    pub fn event(
        &mut self,
        ui: &mut Ui,
        session: &mut Session,
        node: NodeId,
        event: &Event,
        requests: &mut Requests,
    ) {
        if node == self.lock_button {
            if let Event::Click { .. } = event {
                self.toggle_lock(ui);
                requests.refresh = true;
            }
            return;
        }
        let Some(part) = self.parts.get(&node).cloned() else {
            return;
        };
        match (part, event) {
            (Part::Slot { field, axis }, Event::Submit(_)) => {
                self.commit(ui, session, &field, axis);
                requests.refresh = true;
            }
            (Part::Slot { .. }, Event::Cancel) => {
                requests.refresh = true;
            }
            (Part::Revert(field), Event::Click { .. }) => {
                for id in self.showing.clone() {
                    let _ = session.revert_field(id, &field);
                }
                requests.refresh = true;
            }
            (Part::Reveal(field), Event::Click { .. }) => {
                self.revealed.insert(field.clone());
                self.shape.clear();
                requests.refresh = true;
                requests.focus_named = Some(field);
            }
            (Part::Pick(field), Event::Click { .. }) => {
                let items = self.choices(session, &field);
                let r = ui.rect(node);
                requests.menu = Some((items, r.x - 180.0, r.y + r.height));
            }
            (Part::Slider(i), Event::Press { x, .. } | Event::Drag { x, .. }) => {
                let r = ui.rect(node);
                self.hsv[i] = ((x - r.x) / r.width.max(1.0)).clamp(0.0, 1.0);
                self.show_color(ui);
            }
            (Part::Slider(_), Event::Release { .. }) => {
                self.apply_color(session);
                requests.refresh = true;
            }
            (Part::Hex, Event::Submit(text)) => {
                match parse_hex(text) {
                    Some(rgb) => {
                        self.hsv = hsv_of(rgb);
                        self.apply_color(session);
                    }
                    None => session.say(Level::Error, format!("{text:?} is not a colour: #rrggbb")),
                }
                requests.refresh = true;
            }
            (Part::Import(field), Event::Submit(value)) => {
                if let Some((asset, Some(source))) = self.asset.clone() {
                    match session.set_import_setting(&source, &field, value.trim()) {
                        Ok(()) => session.say(
                            Level::Info,
                            format!("{source}: {field} = {value}, reimported"),
                        ),
                        Err(e) => session.say(Level::Error, e.to_string()),
                    }
                    requests.inspect = Some(asset);
                }
            }
            (Part::ImportToggle(field, on), Event::Click { .. }) => {
                if let Some((asset, Some(source))) = self.asset.clone() {
                    match session.set_import_setting(
                        &source,
                        &field,
                        if on { "false" } else { "true" },
                    ) {
                        Ok(()) => session.say(
                            Level::Info,
                            format!("{source}: {field} = {}, reimported", !on),
                        ),
                        Err(e) => session.say(Level::Error, e.to_string()),
                    }
                    requests.inspect = Some(asset);
                }
            }
            (
                Part::Sub {
                    component,
                    key,
                    kind: SubKind::Bool(on),
                },
                Event::Click { .. },
            ) => {
                self.set_sub(session, &component, &key, if on { "false" } else { "true" });
                requests.refresh = true;
            }
            (
                Part::Sub {
                    component,
                    key,
                    kind: SubKind::Entity,
                },
                Event::Click { .. },
            ) => {
                // Unity's object picker: None, then every entity of the
                // scene but the ones being edited, by the hierarchy's order.
                let link = |id: Option<runity::EntityId>| {
                    let value = format!(
                        "{}({:?})",
                        runity::EntityRef::NAME,
                        id.map(|id| id.to_string()).unwrap_or_default()
                    );
                    Action::SetSub(component.clone(), key.clone(), value)
                };
                let mut items = vec![MenuItem::new("None", link(None)), MenuItem::separator()];
                for row in session.hierarchy() {
                    if self.showing.contains(&row.id) {
                        continue;
                    }
                    let indent = "  ".repeat(row.depth);
                    items.push(MenuItem::new(
                        &format!("{indent}{}", row.name),
                        link(Some(row.id)),
                    ));
                }
                let r = ui.rect(node);
                requests.menu = Some((items, r.x, r.y + r.height));
            }
            (
                Part::Sub {
                    component,
                    key,
                    kind: SubKind::Enum(variants),
                },
                Event::Click { .. },
            ) => {
                let r = ui.rect(node);
                let items = variants
                    .iter()
                    .map(|v| {
                        MenuItem::new(v, Action::SetSub(component.clone(), key.clone(), v.clone()))
                    })
                    .collect();
                requests.menu = Some((items, r.x, r.y + r.height));
            }
            (
                Part::Sub {
                    component,
                    key,
                    kind,
                },
                Event::Submit(value),
            ) => {
                let value = match kind {
                    SubKind::Number => match eval(value.trim()) {
                        Some(n) => format!("{n:?}"),
                        None => {
                            session.say(Level::Error, format!("{component}.{key}: a number"));
                            self.built = false;
                            requests.refresh = true;
                            return;
                        }
                    },
                    SubKind::Text => format!("{:?}", value),
                    _ => value.trim().to_string(),
                };
                self.set_sub(session, &component, &key, &value);
                requests.refresh = true;
            }
            (Part::Environment(field), Event::Submit(value)) => {
                if let Err(e) = session.set_environment(&field, value.trim()) {
                    session.say(Level::Error, e.to_string());
                }
                self.built = false;
                requests.refresh = true;
            }
            (Part::Hour, Event::Press { x, .. } | Event::Drag { x, .. }) => {
                let r = ui.rect(node);
                let hour = ((x - r.x) / r.width.max(1.0)).clamp(0.0, 0.999) * 24.0;
                if let Some(fill) = ui.children(node).first().copied() {
                    ui.restyle(fill, |s| s.width_fraction(hour / 24.0));
                }
                self.hour = hour;
            }
            (Part::Hour, Event::Release { .. }) => {
                let sun = session
                    .environment()
                    .into_iter()
                    .find(|(f, _)| *f == "sun")
                    .map(|(_, v)| v)
                    .unwrap_or_default();
                // The hour replaced in what the sun says, the rest kept.
                let text = match sun.find("hour:") {
                    Some(at) => {
                        let rest = &sun[at + 5..];
                        let end = rest.find([',', ')']).unwrap_or(rest.len());
                        format!("{}hour:{:.2}{}", &sun[..at], self.hour, &rest[end..])
                    }
                    None => format!("(hour: {:.2})", self.hour),
                };
                if let Err(e) = session.set_environment("sun", &text) {
                    session.say(Level::Error, e.to_string());
                }
                self.built = false;
                requests.refresh = true;
            }
            (Part::Label(field), Event::Click { .. }) => {
                let mut items = vec![
                    MenuItem::new("Reset", Action::FieldReset(field.clone())),
                    MenuItem::new("Copy Value", Action::FieldCopy(field.clone())),
                    MenuItem::new("Paste Value", Action::FieldPaste(field.clone())),
                ];
                let removable = field.starts_with("components.")
                    || [
                        "camera",
                        "light",
                        "particles",
                        "route",
                        "joint",
                        "collider",
                        "body",
                        "physics",
                    ]
                    .contains(&field.as_str());
                if removable {
                    items.push(MenuItem::separator());
                    items.push(MenuItem::new("Remove", Action::FieldRemove(field.clone())));
                }
                let (x, y) = ui.pointer();
                requests.menu = Some((items, x, y));
            }
            (Part::PickComponent, Event::Click { .. }) => {
                let mut names: Vec<String> = session.component_shapes().into_keys().collect();
                // Components written but not yet described: their files.
                if let Some(project) = session.project() {
                    if let Ok(read) =
                        std::fs::read_dir(project.root().join(runity::project::COMPONENTS))
                    {
                        for e in read.flatten() {
                            let p = e.path();
                            if p.extension().is_some_and(|x| x == "rs") {
                                let n = p.file_stem().unwrap().to_string_lossy().into_owned();
                                if n != "mod" && !names.contains(&n) {
                                    names.push(n);
                                }
                            }
                        }
                    }
                }
                names.sort();
                let mut items: Vec<MenuItem> = names
                    .into_iter()
                    .map(|n| MenuItem::new(&n, Action::AddComponent(n.clone())))
                    .collect();
                if items.is_empty() {
                    items.push(MenuItem::new("Create Component…", Action::NewComponent));
                } else {
                    items.push(MenuItem::separator());
                    items.push(MenuItem::new("Create Component…", Action::NewComponent));
                }
                let r = ui.rect(node);
                requests.menu = Some((items, r.x - 200.0, r.y + r.height));
            }
            (Part::AddComponent, Event::Submit(name)) => {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    for id in self.showing.clone() {
                        if let Err(e) = session.add_component(id, &name) {
                            session.say(Level::Error, e.to_string());
                            break;
                        }
                    }
                }
                ui.set_text(node, "");
                requests.refresh = true;
            }
            _ => {}
        }
    }

    /// What a picker offers for `field`.
    fn choices(&self, session: &Session, field: &str) -> Vec<MenuItem> {
        let names: Vec<String> = match field {
            "material" => session.palette().into_iter().map(|(n, _)| n).collect(),
            "model" => {
                let mut v: Vec<String> = runity::builtin::NAMES
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                if let Ok(assets) = session.assets() {
                    v.extend(
                        assets
                            .into_iter()
                            .filter(|a| a.kind == "model")
                            .map(|a| a.name),
                    );
                }
                v
            }
            "prefab" => session.prefab_names(),
            _ => Vec::new(),
        };
        names
            .into_iter()
            .map(|n| MenuItem::new(&n, Action::SetField(field.to_string(), n.clone())))
            .collect()
    }

    /// Hand what was typed into `field` to the session.
    fn commit(&mut self, ui: &mut Ui, session: &mut Session, field: &str, axis: Option<usize>) {
        let text = match axis {
            None => {
                let Some((_, _, node, _)) = self.slots.iter().find(|s| s.0 == field) else {
                    return;
                };
                let t = ui.text(*node).unwrap_or_default().to_string();
                if field == "material" && !t.starts_with('"') && !t.starts_with('(') {
                    // A material by name, typed as a person types it.
                    format!("{t:?}")
                } else {
                    t
                }
            }
            Some(_) => {
                let typed: Vec<String> = self
                    .slots
                    .iter()
                    .filter(|s| s.0 == field)
                    .map(|s| ui.text(s.2).unwrap_or_default().trim().to_string())
                    .collect();
                let numbers: Option<Vec<f32>> = typed.iter().map(|t| eval(t)).collect();
                match numbers {
                    Some(n) if n.len() == 3 => format!("({:?}, {:?}, {:?})", n[0], n[1], n[2]),
                    _ => {
                        session.say(Level::Error, format!("{field}: every box takes a number"));
                        self.shape.clear();
                        return;
                    }
                }
            }
        };
        if text == MIXED {
            return;
        }
        if let Err(e) = session.set_field_all(&self.showing, field, &text) {
            session.say(Level::Error, e.to_string());
        }
        self.revealed.remove(field);
        // Whatever the scene holds now is what the boxes show — including
        // the old text, when it refused.
        for s in self.slots.iter_mut().filter(|s| s.0 == field) {
            s.3.clear();
        }
    }

    /// A variant picked from an enum field's menu.
    pub fn pick_sub(&mut self, session: &mut Session, component: &str, key: &str, value: &str) {
        self.set_sub(session, component, key, value);
    }

    pub fn set_field(&mut self, session: &mut Session, field: &str, value: &str) {
        let text = if field == "material" {
            format!("{value:?}")
        } else {
            value.to_string()
        };
        if let Err(e) = session.set_field_all(&self.showing, field, &text) {
            session.say(Level::Error, e.to_string());
        }
    }

    /// The slot for a field, to put the keyboard in after revealing it.
    pub fn slot_of(&self, field: &str) -> Option<NodeId> {
        self.slots.iter().find(|s| s.0 == field).map(|s| s.2)
    }
}

/// A number, or a little arithmetic on numbers: `1.5*2`, `10/4`, `-3+1`,
/// as Unity's fields take.
fn eval(text: &str) -> Option<f32> {
    if let Ok(n) = text.parse() {
        return Some(n);
    }
    // Last operator wins, so `a-b-c` is `(a-b)-c`; a leading sign is part
    // of the number.
    for ops in [['+', '-'], ['*', '/']] {
        if let Some((i, op)) = text.char_indices().rev().find(|(i, c)| {
            ops.contains(c) && *i > 0 && !text[..*i].ends_with(['e', 'E', '*', '/', '+', '-'])
        }) {
            let (a, b) = (eval(text[..i].trim())?, eval(text[i + 1..].trim())?);
            return Some(match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                _ => a / b,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_takes_a_little_arithmetic() {
        assert_eq!(eval("2.5"), Some(2.5));
        assert_eq!(eval("1.5*2"), Some(3.0));
        assert_eq!(eval("10 - 4 - 1"), Some(5.0));
        assert_eq!(eval("-3+1"), Some(-2.0));
        assert_eq!(eval("2+3*4"), Some(14.0));
        assert_eq!(eval("abc"), None);
    }

    #[test]
    fn a_components_fields_split_at_the_top() {
        assert_eq!(
            struct_fields("(a: 1.0, name: \"x, y\", list: [1, 2], inner: (b: true))"),
            Some(vec![
                ("a".into(), "1.0".into()),
                ("name".into(), "\"x, y\"".into()),
                ("list".into(), "[1, 2]".into()),
                ("inner".into(), "(b: true)".into()),
            ])
        );
        assert_eq!(struct_fields("()"), Some(vec![]));
    }

    #[test]
    fn colours_go_round_hsv() {
        for rgb in [
            [0x91, 0x84, 0xd9],
            [255, 0, 0],
            [12, 200, 90],
            [0, 0, 0],
            [255, 255, 255],
        ] {
            assert_eq!(rgb_of(hsv_of(rgb)), rgb);
        }
        assert_eq!(parse_hex("#9184d9"), Some([0x91, 0x84, 0xd9]));
        assert_eq!(parse_hex("nope"), None);
    }

    #[test]
    fn numbers_read_as_typed() {
        assert_eq!(
            axes("(2.6,0.5,2.0)"),
            Some(["2.6".into(), "0.5".into(), "2".into()])
        );
        assert_eq!(trim_number("-0.0"), "0");
    }
}
