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

const OBJECT: [&str; 5] = ["model", "material", "prefab", "animator", "bends_grass"];
const TRANSFORM: [&str; 3] = ["position", "rotation", "scale"];
const PHYSICS: [&str; 5] = ["body", "collider", "physics", "joint", "joint_break"];
const PARTS: [&str; 13] = [
    "camera",
    "light",
    "particles",
    "sound",
    "reflection_probe",
    "decal",
    "render_texture",
    "post_volume",
    "footprints",
    "terrain",
    "route",
    "spline",
    "along",
];

/// The pictures the colour picker draws with: the square of saturation and
/// value at its hue, and the strip of hues.
pub const SV_SQUARE: runity_ui::ImageId = runity_ui::ImageId(10);
pub const HUE_STRIP: runity_ui::ImageId = runity_ui::ImageId(11);
const PICTURE: u32 = 128;

/// X, Y and Z as the gizmo colours them.
const AXES: [runity_ui::Color; 3] = [
    runity_ui::Color::hex(0xe58a96),
    runity_ui::Color::hex(0x9fd49a),
    runity_ui::Color::hex(0x8fb4e8),
];

/// What a field that can be added is set to when it is: `None` for one
/// shown empty to be filled in.
fn added_value(field: &str) -> Option<&'static str> {
    Some(match field {
        "collider" => "Box(half: (0.5, 0.5, 0.5))",
        "body" => "Dynamic",
        "joint" => "Ball(anchor: (0.0, 0.0, 0.0))",
        "camera" | "light" | "particles" | "reflection_probe" | "decal" => "()",
        "post_volume" => "(size: (10.0, 10.0, 10.0))",
        "route" => "(points: [(0.0, 0.0, 0.0), (0.0, 2.0, 0.0)])",
        "render_texture" => "(name: \"picture\")",
        "sound" => "(clip: \"\")",
        _ => return None,
    })
}

/// The parts a line can have besides the game's components, as the Add
/// Component list names them.
const ADDABLE: [(&str, &str); 15] = [
    ("model", "Model"),
    ("collider", "Collider"),
    ("body", "Body"),
    ("physics", "Physics"),
    ("joint", "Joint"),
    ("joint_break", "Joint Break"),
    ("camera", "Camera"),
    ("light", "Light"),
    ("particles", "Particles"),
    ("sound", "Sound"),
    ("animator", "Animator"),
    ("reflection_probe", "Reflection Probe"),
    ("decal", "Decal"),
    ("route", "Route"),
    ("post_volume", "Post Volume"),
];

/// Fields a line can be without: what the trash on a field takes off.
const REMOVABLE: [&str; 19] = [
    "model",
    "footprints",
    "terrain",
    "camera",
    "light",
    "particles",
    "sound",
    "animator",
    "render_texture",
    "post_volume",
    "decal",
    "reflection_probe",
    "route",
    "joint",
    "joint_break",
    "collider",
    "body",
    "physics",
    "spline",
];

/// A field that says nothing: not shown, offered by Add Component.
fn is_empty(value: &str) -> bool {
    // `false`: a switch that is off — `inactive` — says nothing either.
    matches!(value, "" | "None" | "r#None" | "()" | "\"\"" | "false")
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
    /// The yellow arrow: set the field back to what a new entity has, or
    /// to the prefab's.
    Reset(String),
    /// A parameter of the material shown from the Project, and its arrow:
    /// back to the parent's.
    MaterialParam(String, String),
    MaterialReset(String, String),
    /// A box of a field; `axis` for one number of a vector.
    Slot {
        field: String,
        axis: Option<usize>,
    },
    /// The dot on an overridden field: revert it.
    Revert(String),
    /// The «…» next to a field with a list to pick from.
    Pick(String),
    /// A field's label: a click opens its menu — Reset, Copy, Paste,
    /// Remove — as Unity's ⋮ on a component.
    Label(String),
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
    /// The switch before the name: on in the world, or off.
    Active(bool),
    /// The layer, as a list to pick from.
    Layer,
    /// The trash on a field: take it off the line.
    Remove(String),
    /// A label to drag sideways: the number changes as it goes — one axis
    /// of a vector, or a component's number.
    Scrub(Scrub),
    /// The button under the fields, and in the list it opens: its search,
    /// one entry, and the ground around it that closes it.
    AddButton,
    AddSearch,
    AddEntry(Addition),
    /// A colour's swatch: opens the picker.
    Swatch(ColorTarget),
    /// The picker: its square, its strip of hues, its hex box.
    PickerSquare,
    PickerHue,
    PickerHex,
    /// Around a list or a picker: a click there closes it.
    Dismiss,
}

/// A number dragged by its label.
#[derive(Debug, Clone, PartialEq)]
enum Scrub {
    Axis(String, usize),
    Sub { component: String, key: String, whole: bool },
}

/// Something Add Component offers.
#[derive(Debug, Clone, PartialEq)]
enum Addition {
    Field(String),
    Component(String),
    NewComponent,
}

/// Whose colour the picker edits.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorTarget {
    /// The selection's material, in linear light.
    Material,
    /// A colour inside a field's value, `key: (r, g, b)`, as a picker says
    /// it (sRGB, 0 to 1): a light's, particles'.
    Field(String, String),
}

/// What is open over the panel.
enum Popover {
    Add {
        root: NodeId,
        list: NodeId,
        hits: Vec<Addition>,
    },
    Color {
        root: NodeId,
        target: ColorTarget,
        hex: NodeId,
        preview: NodeId,
        square_mark: NodeId,
        hue_mark: NodeId,
    },
}

impl Popover {
    fn root(&self) -> NodeId {
        match self {
            Popover::Add { root, .. } | Popover::Color { root, .. } => *root,
        }
    }
}

/// `key: (r, g, b)` in a field's RON, as three numbers.
fn ron_color(text: &str, key: &str) -> Option<[f32; 3]> {
    let at = find_key(text, key)?;
    let rest = text[at..].trim_start().strip_prefix('(')?;
    let inner = &rest[..rest.find(')')?];
    let n: Vec<f32> = inner
        .split(',')
        .map(|v| v.trim().parse().ok())
        .collect::<Option<_>>()?;
    (n.len() == 3).then(|| [n[0], n[1], n[2]])
}

/// Where the value of `key:` starts in RON text, the key whole.
fn find_key(text: &str, key: &str) -> Option<usize> {
    let pattern = format!("{key}:");
    let mut from = 0;
    while let Some(i) = text[from..].find(&pattern) {
        let at = from + i;
        let before = text[..at].chars().next_back();
        if !before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            return Some(at + pattern.len());
        }
        from = at + pattern.len();
    }
    None
}

/// The text with `key: (r, g, b)` set: replaced, or added at the end.
fn with_ron_color(text: &str, key: &str, rgb: [f32; 3]) -> String {
    let value = format!("({:.3}, {:.3}, {:.3})", rgb[0], rgb[1], rgb[2]);
    if let Some(at) = find_key(text, key) {
        if let Some(close) = text[at..].find(')') {
            return format!("{} {value}{}", &text[..at], &text[at + close + 1..]);
        }
    }
    let trimmed = text.trim();
    match trimmed.strip_suffix(')') {
        Some(body) if body.trim_end().ends_with('(') => format!("{body}{key}: {value})"),
        Some(body) => format!("{}, {key}: {value})", body.trim_end().trim_end_matches(',')),
        None => text.to_string(),
    }
}

/// The square of saturation (across) and value (down) at a hue, RGBA.
fn sv_pixels(hue: f32) -> Vec<u8> {
    let mut out = Vec::with_capacity((PICTURE * PICTURE * 4) as usize);
    for y in 0..PICTURE {
        for x in 0..PICTURE {
            let s = x as f32 / (PICTURE - 1) as f32;
            let v = 1.0 - y as f32 / (PICTURE - 1) as f32;
            let [r, g, b] = rgb_of([hue, s, v]);
            out.extend([r, g, b, 255]);
        }
    }
    out
}

/// Every hue, left to right.
fn hue_pixels() -> Vec<u8> {
    let mut out = Vec::with_capacity((PICTURE * PICTURE * 4) as usize);
    for _ in 0..PICTURE {
        for x in 0..PICTURE {
            let [r, g, b] = rgb_of([x as f32 / PICTURE as f32, 1.0, 1.0]);
            out.extend([r, g, b, 255]);
        }
    }
    out
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
    /// A link to an asset of a kind (`model`, `prefab`, `sound`…): a picker
    /// of that kind's assets.
    Asset(String),
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
    /// The colour being picked.
    hsv: [f32; 3],
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
    /// Which fields had a reset arrow at the last build: an arrow that
    /// comes or goes is a new layout.
    resets: Vec<String>,
    /// The Add Component list or the colour picker, over everything.
    popover: Option<Popover>,
    popover_parts: HashMap<NodeId, Part>,
    /// A number being dragged by its label: what it was at the press, and
    /// how far the pointer has gone.
    scrubbing: Option<(f32, f32)>,
    /// Pictures for the renderer to take: the picker's.
    images: Vec<(runity_ui::ImageId, u32, Vec<u8>)>,
    hue_drawn: bool,
    /// Each component field's box, to show a scrubbed number as it goes.
    sub_nodes: HashMap<(String, String), NodeId>,
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
        ui.add(strip, Style::row().fill());
        let lock_button = icon_button(ui, strip, "inspector lock", "lock-open", false);
        let body = ui.add(card, Style::column().fill().full_width().clip());
        Self {
            locked: None,
            lock_button,
            resets: Vec::new(),
            popover: None,
            popover_parts: HashMap::new(),
            scrubbing: None,
            images: Vec::new(),
            hue_drawn: false,
            sub_nodes: HashMap::new(),
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
        }
    }

    /// The model or prefab the Inspector is showing from the Project, if
    /// that is what it shows.
    pub fn asset_name(&self) -> Option<String> {
        match &self.asset {
            Some((Asset::Model(name, _) | Asset::Prefab(name), _)) => Some(name.clone()),
            _ => None,
        }
    }

    pub fn owns(&self, node: NodeId) -> bool {
        node == self.lock_button
            || self.parts.contains_key(&node)
            || self.popover_parts.contains_key(&node)
    }

    /// Pictures made since last asked, for the renderer.
    pub fn take_images(&mut self) -> Vec<(runity_ui::ImageId, u32, Vec<u8>)> {
        std::mem::take(&mut self.images)
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
            if self.scrubbing.is_none() {
                self.close_popover(ui);
            }
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
        let resets: Vec<String> = fields
            .iter()
            .filter(|f| f.resettable)
            .map(|f| f.name.clone())
            .collect();
        // Mid-drag the boxes stay: rebuilt, the handle being dragged would
        // be gone from under the pointer.
        let dragging = self.scrubbing.is_some() && ids == self.showing;
        let changed = !self.built
            || ids != self.showing
            || shape != self.shape
            || playing != self.playing
            || resets != self.resets;
        if changed && !dragging {
            self.resets = resets;
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
        // Unity's checkbox before the name: on in the world, or off.
        if let Some(inactive) = find("inactive") {
            let on = inactive.value != "true";
            let check = ui.add(
                head,
                Style::row()
                    .size(16.0, 16.0)
                    .fixed()
                    .center()
                    .radius(4.0)
                    .border(1.0, if on { ACCENT } else { NEUTRAL_500 })
                    .background(if on { ACCENT } else { runity_ui::Color::TRANSPARENT })
                    .hover_border(ACCENT)
                    .clickable(),
            );
            ui.set_name(check, "inspector active");
            if on {
                icon(ui, check, "check", NEUTRAL_900);
            }
            self.parts.insert(check, Part::Active(on));
        }
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
        // The layer: a list to pick from, as Unity's under the name.
        if let Some(layer) = find("layer") {
            let pick = ui.add(
                head,
                Style::row()
                    .height(22.0)
                    .fixed()
                    .padding_x(6.0)
                    .gap(4.0)
                    .center_items()
                    .radius(6.0)
                    .border(1.0, DIVIDER)
                    .hover(HOVER)
                    .clickable(),
            );
            ui.set_name(pick, "inspector layer");
            icon(ui, pick, "layers-2", MUTED);
            let shown = if layer.value.is_empty() { "Default" } else { &layer.value };
            ui.add_text(pick, text().nowrap().text_size(11.5), shown);
            icon(ui, pick, "chevron-down", MUTED);
            self.parts.insert(pick, Part::Layer);
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
                        let rgb = to_srgb(m.base_color);
                        self.color_line(ui, "Color", "material", rgb, ColorTarget::Material);
                    }
                }
                // A colour inside a light's or particles' value: a swatch.
                if f.value != MIXED {
                    for key in ["color", "end_color"] {
                        if !matches!(f.name.as_str(), "light" | "particles") {
                            break;
                        }
                        if let Some(c) = ron_color(&f.value, key) {
                            let rgb = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
                            self.color_line(
                                ui,
                                &title(key).replace('_', " "),
                                &format!("{} {key}", f.name),
                                rgb,
                                ColorTarget::Field(f.name.clone(), key.to_string()),
                            );
                        }
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

        // Unity's Add Component: one button, a list with a search.
        let foot = ui.add(
            self.body,
            Style::column()
                .full_width()
                .padding_x(SPACE_4)
                .padding_y(SPACE_3),
        );
        let add = ui.add(
            foot,
            Style::row()
                .full_width()
                .height(26.0)
                .gap(SPACE_2)
                .center()
                .radius(6.0)
                .border(1.0, DIVIDER)
                .hover(HOVER)
                .hover_border(ACCENT)
                .clickable(),
        );
        ui.set_name(add, "add component");
        icon(ui, add, "plus", LABEL);
        ui.add_text(add, text().nowrap(), "Add Component");
        self.parts.insert(add, Part::AddButton);
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
                    // The letter is a handle: dragged sideways, the number
                    // goes up or down with it.
                    let handle = ui.add(
                        b,
                        Style::row()
                            .size(12.0, 22.0)
                            .fixed()
                            .center()
                            .radius(RADIUS_SM)
                            .hover(HOVER)
                            .draggable()
                            .clickable(),
                    );
                    ui.set_name(handle, format!("scrub {} {}", f.name, ["x", "y", "z"][i]));
                    ui.add_text(
                        handle,
                        Style::default()
                            .text_size(10.5)
                            .text_color(AXES[i])
                            .nowrap(),
                        ["X", "Y", "Z"][i],
                    );
                    if !self.playing {
                        self.parts
                            .insert(handle, Part::Scrub(Scrub::Axis(f.name.clone(), i)));
                    }
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
        self.remove_button(ui, line, &f.name);
        // Unreal's yellow arrow: this field says something a new entity (or
        // the prefab) does not, and one click takes it back.
        if f.resettable && !self.playing {
            let reset = ui.add(
                line,
                Style::row()
                    .size(18.0, 22.0)
                    .fixed()
                    .center()
                    .radius(6.0)
                    .hover(HOVER)
                    .clickable(),
            );
            ui.set_name(reset, format!("reset {}", f.name));
            icon(ui, reset, "undo-2", WARNING);
            self.parts.insert(reset, Part::Reset(f.name.clone()));
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
                .fill()
                .text_size(12.0)
                .text_color(if f.overridden { ACCENT_300 } else { TEXT })
                .nowrap(),
            &title(&f.name),
        );
        self.remove_button(ui, head, &f.name);
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
            // A number's name is a handle: drag it sideways.
            let number = matches!(kind, Shape::Int | Shape::Float);
            let name = ui.add(
                line,
                Style::row()
                    .width(84.0 - 18.0)
                    .fixed()
                    .height(22.0)
                    .center_items()
                    .radius(RADIUS_SM),
            );
            ui.add_text(
                name,
                Style::default().text_size(12.0).text_color(LABEL).nowrap(),
                &title(key),
            );
            if number && !self.playing {
                ui.restyle(name, |s| s.hover(HOVER).draggable().clickable());
                ui.set_name(name, format!("scrub {component} {key}"));
                self.parts.insert(
                    name,
                    Part::Scrub(Scrub::Sub {
                        component: component.to_string(),
                        key: key.clone(),
                        whole: matches!(kind, Shape::Int),
                    }),
                );
            }
            let sub = match kind {
                Shape::Bool => SubKind::Bool(value == "true"),
                Shape::Int | Shape::Float => SubKind::Number,
                Shape::Text => SubKind::Text,
                Shape::Enum(variants) => SubKind::Enum(variants.clone()),
                Shape::Entity => SubKind::Entity,
                Shape::Asset(kind) => SubKind::Asset(kind.clone()),
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
                SubKind::Asset(kind) => {
                    // The asset by name, or None; red when there is none by
                    // that name or ID.
                    let link = runity::refs::links_in(&value)
                        .into_iter()
                        .next()
                        .map(|(_, l)| l);
                    let (label, known) = match &link {
                        Some(l) if !l.is_empty() => (l.to_string(), session.link_exists(kind, l)),
                        _ => (format!("None ({kind})"), true),
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
                    let glyph = match kind.as_str() {
                        "prefab" => "package",
                        "sound" => "music",
                        "scene" => "mountain",
                        "texture" => "image",
                        "material" => "sparkles",
                        _ => "box",
                    };
                    icon(ui, pick, glyph, if link.is_some() { ACCENT } else { MUTED });
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
            self.sub_nodes
                .insert((component.to_string(), key.clone()), node);
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
        // Its parameters, as an instance of its parent: what it sets itself
        // stands out and has an arrow back to the parent's.
        if kind == "material" {
            if let Ok(layers) = session.material_layers(&name) {
                let title_text = match &layers.parent {
                    Some(parent) => format!("Instance of {}", parent.as_str()),
                    None => "Material".to_string(),
                };
                self.heading(ui, &title_text);
                for (key, value, set) in layers.fields {
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
                            .text_color(if set { ACCENT_300 } else { LABEL })
                            .nowrap(),
                        &title(&key),
                    );
                    let f = ui.add_field(line, field_style().fill().mono().text_size(11.5), &value);
                    ui.set_name(f, format!("material {key}"));
                    self.parts
                        .insert(f, Part::MaterialParam(name.clone(), key.clone()));
                    if set && layers.parent.is_some() {
                        let reset = ui.add(
                            line,
                            Style::row()
                                .size(18.0, 22.0)
                                .fixed()
                                .center()
                                .radius(6.0)
                                .hover(HOVER)
                                .clickable(),
                        );
                        ui.set_name(reset, format!("reset material {key}"));
                        icon(ui, reset, "undo-2", WARNING);
                        self.parts
                            .insert(reset, Part::MaterialReset(name.clone(), key.clone()));
                    }
                }
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

    /// The trash at the end of a field that the line can do without.
    fn remove_button(&mut self, ui: &mut Ui, line: NodeId, field: &str) {
        if self.playing || !(field.starts_with("components.") || REMOVABLE.contains(&field)) {
            return;
        }
        let trash = ui.add(
            line,
            Style::row()
                .size(20.0, 22.0)
                .fixed()
                .center()
                .radius(6.0)
                .hover(HOVER)
                .clickable(),
        );
        ui.set_name(trash, format!("remove {field}"));
        icon(ui, trash, "trash", MUTED);
        self.parts.insert(trash, Part::Remove(field.to_string()));
    }

    /// A colour as a swatch and its `#rrggbb`: a click on either opens the
    /// picker.
    fn color_line(&mut self, ui: &mut Ui, label: &str, name: &str, rgb: [u8; 3], target: ColorTarget) {
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
            label,
        );
        let swatch = ui.add(
            line,
            Style::row()
                .fill()
                .height(22.0)
                .padding_x(6.0)
                .gap(SPACE_2)
                .center_items()
                .radius(6.0)
                .border(1.0, DIVIDER)
                .hover_border(ACCENT)
                .clickable(),
        );
        ui.set_name(swatch, format!("{name} swatch"));
        ui.add(
            swatch,
            Style::row()
                .size(28.0, 14.0)
                .fixed()
                .radius(3.0)
                .border(1.0, NEUTRAL_800)
                .background(runity_ui::Color::rgba(rgb[0], rgb[1], rgb[2], 255)),
        );
        ui.add_text(
            swatch,
            text().mono().text_size(11.5).nowrap(),
            &format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]),
        );
        self.parts.insert(swatch, Part::Swatch(target));
    }

    pub fn event(
        &mut self,
        ui: &mut Ui,
        session: &mut Session,
        node: NodeId,
        event: &Event,
        requests: &mut Requests,
    ) {
        if let Some(part) = self.popover_parts.get(&node).cloned() {
            self.popover_event(ui, session, node, part, event, requests);
            return;
        }
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
            (Part::MaterialParam(material, key), Event::Submit(value)) => {
                if let Err(e) = session.set_material_param(&material, &key, Some(value.trim())) {
                    session.say(Level::Error, e.to_string());
                }
                requests.inspect = Some(Asset::Material(material));
            }
            (Part::MaterialReset(material, key), Event::Click { .. }) => {
                if let Err(e) = session.set_material_param(&material, &key, None) {
                    session.say(Level::Error, e.to_string());
                }
                requests.inspect = Some(Asset::Material(material));
            }
            (Part::Reset(field), Event::Click { .. }) => {
                for id in self.showing.clone() {
                    if let Err(e) = session.reset_field(id, &field) {
                        session.say(Level::Error, e.to_string());
                        break;
                    }
                }
                requests.refresh = true;
            }
            (Part::Revert(field), Event::Click { .. }) => {
                for id in self.showing.clone() {
                    let _ = session.revert_field(id, &field);
                }
                requests.refresh = true;
            }
            (Part::Pick(field), Event::Click { .. }) => {
                let items = self.choices(session, &field);
                let r = ui.rect(node);
                requests.menu = Some((items, r.x - 180.0, r.y + r.height));
            }
            (Part::Active(on), Event::Click { .. }) => {
                let value = if on { "true" } else { "false" };
                if let Err(e) = session.set_field_all(&self.showing, "inactive", value) {
                    session.say(Level::Error, e.to_string());
                }
                self.built = false;
                requests.refresh = true;
            }
            (Part::Layer, Event::Click { .. }) => {
                let mut items = vec![MenuItem::new(
                    "Default",
                    Action::SetField("layer".into(), String::new()),
                )];
                let layers = session.layer_names();
                if !layers.is_empty() {
                    items.push(MenuItem::separator());
                }
                for layer in layers {
                    items.push(MenuItem::new(&layer, Action::SetField("layer".into(), layer.clone())));
                }
                let r = ui.rect(node);
                requests.menu = Some((items, r.x, r.y + r.height));
            }
            (Part::Remove(field), Event::Click { .. }) => {
                // Taken off every line shown: one step.
                session.begin_gesture();
                for id in self.showing.clone() {
                    let done = match field.strip_prefix("components.") {
                        Some(name) => session.set_component(id, name, None),
                        None => session.reset_field(id, &field),
                    };
                    if let Err(e) = done {
                        session.say(Level::Error, e.to_string());
                        break;
                    }
                }
                session.end_gesture();
                self.revealed.remove(&field);
                self.built = false;
                requests.refresh = true;
            }
            (Part::Scrub(scrub), Event::Press { .. }) => {
                let start = match &scrub {
                    Scrub::Axis(field, i) => self
                        .slots
                        .iter()
                        .find(|s| s.0 == *field && s.1 == Some(*i))
                        .and_then(|s| eval(ui.text(s.2).unwrap_or_default().trim())),
                    Scrub::Sub { component, key, .. } => self
                        .component_values
                        .get(component)
                        .and_then(|v| v.iter().find(|(k, _)| k == key))
                        .and_then(|(_, v)| eval(v.trim())),
                };
                if let Some(start) = start {
                    session.begin_gesture();
                    self.scrubbing = Some((start, 0.0));
                }
            }
            (Part::Scrub(scrub), Event::Drag { dx, .. }) => {
                let Some((start, moved)) = self.scrubbing.as_mut() else {
                    return;
                };
                *moved += dx;
                let (start, moved) = (*start, *moved);
                self.scrub_to(ui, session, &scrub, start, moved);
            }
            (Part::Scrub(_), Event::Release { .. }) => {
                if self.scrubbing.take().is_some() {
                    session.end_gesture();
                }
                self.built = false;
                requests.refresh = true;
            }
            (Part::AddButton, Event::Click { .. }) => {
                let r = ui.rect(node);
                self.open_add(ui, session, r);
            }
            (Part::Swatch(target), Event::Click { .. }) => {
                if let Some(rgb) = self.color_of(session, &target) {
                    let r = ui.rect(node);
                    self.open_picker(ui, target, rgb, r);
                }
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
                    kind: SubKind::Asset(kind),
                },
                Event::Click { .. },
            ) => {
                // Unity's object picker for an asset: None, then every asset
                // of the kind, each linked by name and ID.
                let type_name = runity::refs::LINK_KINDS
                    .iter()
                    .find(|(_, k)| *k == kind)
                    .map_or("ModelLink", |(n, _)| n);
                let link = |name: Option<&str>| {
                    let inner = match name {
                        Some(name) => runity::ron::to_string(&session.link_to(&kind, name))
                            .unwrap_or_default(),
                        None => "\"\"".to_string(),
                    };
                    Action::SetSub(
                        component.clone(),
                        key.clone(),
                        format!("{type_name}({inner})"),
                    )
                };
                let mut items = vec![MenuItem::new("None", link(None)), MenuItem::separator()];
                for name in session.assets_of_kind(&kind) {
                    items.push(MenuItem::new(&name, link(Some(&name))));
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
                if matches!(event, Event::Press { .. }) {
                    session.begin_gesture();
                    self.scrubbing = Some((0.0, 0.0));
                }
                let r = ui.rect(node);
                let hour = ((x - r.x) / r.width.max(1.0)).clamp(0.0, 0.999) * 24.0;
                if let Some(fill) = ui.children(node).first().copied() {
                    ui.restyle(fill, |s| s.width_fraction(hour / 24.0));
                }
                // The sun moves as the track does.
                self.hour = hour;
                self.set_hour(session);
            }
            (Part::Hour, Event::Release { .. }) => {
                if self.scrubbing.take().is_some() {
                    session.end_gesture();
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
                let removable =
                    field.starts_with("components.") || REMOVABLE.contains(&field.as_str());
                if removable {
                    items.push(MenuItem::separator());
                    items.push(MenuItem::new("Remove", Action::FieldRemove(field.clone())));
                }
                let (x, y) = ui.pointer();
                requests.menu = Some((items, x, y));
            }
            _ => {}
        }
    }

    /// The sun at the hour being dragged to, the rest of what it says kept.
    fn set_hour(&mut self, session: &mut Session) {
        let sun = session
            .environment()
            .into_iter()
            .find(|(f, _)| *f == "sun")
            .map(|(_, v)| v)
            .unwrap_or_default();
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
    }

    /// A number dragged by its label to `start` and `moved` pixels on.
    fn scrub_to(&mut self, ui: &mut Ui, session: &mut Session, scrub: &Scrub, start: f32, moved: f32) {
        match scrub {
            Scrub::Axis(field, axis) => {
                // A degree a pixel for turning, a centimetre for the rest.
                let step = if field == "rotation" { 0.5 } else { 0.01 };
                let value = start + moved * step;
                let mut three = [0.0f32; 3];
                for s in self.slots.iter().filter(|s| s.0 == *field) {
                    if let Some(i) = s.1 {
                        three[i] = eval(ui.text(s.2).unwrap_or_default().trim()).unwrap_or(0.0);
                    }
                }
                three[*axis] = value;
                let text = format!("({:?}, {:?}, {:?})", three[0], three[1], three[2]);
                if let Err(e) = session.set_field_all(&self.showing, field, &text) {
                    session.say(Level::Error, e.to_string());
                }
                for s in self.slots.iter_mut().filter(|s| s.0 == *field && s.1 == Some(*axis)) {
                    let shown = trim_number(&format!("{value}"));
                    ui.set_text(s.2, &shown);
                    s.3 = shown;
                }
            }
            Scrub::Sub { component, key, whole } => {
                let value = if *whole {
                    format!("{}", (start + moved / 4.0).round() as i64)
                } else {
                    // Faster for big numbers: half a percent of it a pixel.
                    let step = (start.abs() * 0.005).max(0.01);
                    format!("{:?}", start + moved * step)
                };
                self.set_sub(session, component, key, &value);
                if let Some(values) = self.component_values.get_mut(component) {
                    match values.iter_mut().find(|(k, _)| k == key) {
                        Some(slot) => slot.1 = value.clone(),
                        None => values.push((key.clone(), value.clone())),
                    }
                }
                if let Some(node) = self.sub_nodes.get(&(component.clone(), key.clone())) {
                    ui.set_text(*node, &trim_number(&value));
                }
            }
        }
    }

    /// The colour a swatch stands for, as sRGB bytes.
    fn color_of(&self, session: &Session, target: &ColorTarget) -> Option<[u8; 3]> {
        let id = *self.showing.first()?;
        match target {
            ColorTarget::Material => session.material(id).map(|m| to_srgb(m.base_color)),
            ColorTarget::Field(field, key) => {
                let value = session.inspect(id)?.into_iter().find(|f| f.name == *field)?.value;
                ron_color(&value, key).map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
            }
        }
    }

    /// Give what is shown the picker's colour.
    fn apply_color(&mut self, session: &mut Session, target: &ColorTarget) {
        let [r, g, b] = rgb_of(self.hsv);
        match target {
            ColorTarget::Material => {
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
            ColorTarget::Field(field, key) => {
                let rgb = [r, g, b].map(|v| v as f32 / 255.0);
                for id in self.showing.clone() {
                    let Some(value) = session
                        .inspect(id)
                        .and_then(|f| f.into_iter().find(|f| f.name == *field))
                        .map(|f| f.value)
                    else {
                        continue;
                    };
                    if let Err(e) = session.set_field(id, field, &with_ron_color(&value, key, rgb)) {
                        session.say(Level::Error, e.to_string());
                        break;
                    }
                }
            }
        }
    }

    /// Close whatever is open over the panel.
    pub fn close_popover(&mut self, ui: &mut Ui) {
        if let Some(p) = self.popover.take() {
            ui.remove(p.root());
        }
        self.popover_parts.clear();
    }

    /// A layer over everything: the ground, a click on which closes it, and
    /// a card at `x`, `y` for what it holds.
    fn popover_card(&mut self, ui: &mut Ui, x: f32, y: f32, width: f32) -> (NodeId, NodeId) {
        self.close_popover(ui);
        let (w, h, _) = ui.viewport();
        let ground = ui.add(
            ui.root(),
            Style::column().absolute(0.0, 0.0).size(w, h).clickable(),
        );
        ui.set_layer(ground, true);
        ui.set_name(ground, "popover ground");
        self.popover_parts.insert(ground, Part::Dismiss);
        let card = ui.add(
            ground,
            Style::column()
                .absolute(x.min(w - width - 8.0).max(8.0), y.min(h - 120.0))
                .width(width)
                .padding(SPACE_2)
                .gap(SPACE_2)
                .radius(RADIUS_MD)
                .background(SURFACE)
                .border(1.0, NEUTRAL_500)
                .clickable(),
        );
        (ground, card)
    }

    /// Unity's Add Component: a search, and what matches it.
    fn open_add(&mut self, ui: &mut Ui, session: &Session, at: runity_ui::Rect) {
        let (ground, card) = self.popover_card(ui, at.x, at.y + at.height + 4.0, at.width.max(220.0));
        ui.set_name(card, "add component list");
        let field = ui.add_field(card, field_style().full_width().height(26.0), "");
        ui.set_placeholder(field, "Search");
        ui.set_name(field, "add component search");
        self.popover_parts.insert(field, Part::AddSearch);
        let list = ui.add(card, Style::column().full_width().gap(1.0).clip());
        ui.focus(Some(field));
        self.popover = Some(Popover::Add {
            root: ground,
            list,
            hits: Vec::new(),
        });
        self.fill_add(ui, session, "");
    }

    /// What Add Component offers for what is typed: the line's parts it
    /// does not have yet, the game's components, and making a new one.
    fn fill_add(&mut self, ui: &mut Ui, session: &Session, typed: &str) {
        let Some(Popover::Add { list, hits, .. }) = &mut self.popover else {
            return;
        };
        let list = *list;
        let query = typed.trim().to_lowercase();
        let fields = self
            .showing
            .first()
            .and_then(|id| session.inspect(*id))
            .unwrap_or_default();
        let has = |name: &str| {
            fields
                .iter()
                .any(|f| f.name == name && !is_empty(&f.value))
                || self.revealed.contains(name)
        };
        let mut offered: Vec<(Addition, String)> = ADDABLE
            .iter()
            .filter(|(field, _)| !has(field))
            .map(|(field, label)| (Addition::Field(field.to_string()), label.to_string()))
            .collect();
        let mut names: Vec<String> = session.component_shapes().into_keys().collect();
        if let Some(project) = session.project() {
            if let Ok(read) = std::fs::read_dir(project.root().join(runity::project::COMPONENTS)) {
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
        for n in names {
            if !has(&format!("components.{n}")) {
                offered.push((Addition::Component(n.clone()), title(&n)));
            }
        }
        offered.retain(|(_, label)| query.is_empty() || label.to_lowercase().contains(&query));
        offered.push((Addition::NewComponent, "New Component…".into()));
        ui.clear(list);
        hits.clear();
        let mut parts = Vec::new();
        for (i, (addition, label)) in offered.into_iter().enumerate() {
            let row = ui.add(
                list,
                Style::row()
                    .full_width()
                    .height(24.0)
                    .padding_x(SPACE_2)
                    .gap(SPACE_2)
                    .center_items()
                    .radius(RADIUS_SM)
                    .background(if i == 0 && !query.is_empty() { HOVER } else { runity_ui::Color::TRANSPARENT })
                    .hover(HOVER)
                    .clickable(),
            );
            ui.set_name(row, format!("add {label}"));
            let glyph = match &addition {
                Addition::Field(_) => "box",
                Addition::Component(_) => "component",
                Addition::NewComponent => "plus",
            };
            icon(ui, row, glyph, if matches!(addition, Addition::Field(_)) { MUTED } else { ACCENT });
            ui.add_text(row, text().nowrap(), &label);
            hits.push(addition.clone());
            parts.push((row, addition));
        }
        for (row, addition) in parts {
            self.popover_parts.insert(row, Part::AddEntry(addition));
        }
    }

    /// Put what was picked from Add Component on what is shown.
    fn add(&mut self, ui: &mut Ui, session: &mut Session, addition: Addition, requests: &mut Requests) {
        self.close_popover(ui);
        match addition {
            Addition::Field(field) => match added_value(&field) {
                Some(value) => {
                    if let Err(e) = session.set_field_all(&self.showing, &field, value) {
                        session.say(Level::Error, e.to_string());
                    }
                }
                None => {
                    // Shown empty, the keyboard in it, to be filled in.
                    self.revealed.insert(field.clone());
                    self.shape.clear();
                    requests.focus_named = Some(field);
                }
            },
            Addition::Component(name) => requests.action = Some(Action::AddComponent(name)),
            Addition::NewComponent => requests.action = Some(Action::NewComponent),
        }
        self.built = false;
        requests.refresh = true;
    }

    /// The colour picker: a square of saturation and value, a strip of
    /// hues, and the hex. Everything moves the colour as it is dragged.
    fn open_picker(&mut self, ui: &mut Ui, target: ColorTarget, rgb: [u8; 3], at: runity_ui::Rect) {
        self.hsv = hsv_of(rgb);
        let (ground, card) = self.popover_card(ui, at.x, at.y + at.height + 4.0, 236.0);
        ui.set_name(card, "color picker");
        let square = ui.add_image(
            card,
            Style::default()
                .size(220.0, 150.0)
                .radius(RADIUS_SM)
                .draggable()
                .clickable(),
            SV_SQUARE,
        );
        ui.set_name(square, "picker square");
        let square_mark = ui.add(
            square,
            Style::row()
                .absolute(0.0, 0.0)
                .size(10.0, 10.0)
                .radius(5.0)
                .border(2.0, runity_ui::Color::hex(0xffffff)),
        );
        let hue = ui.add_image(
            card,
            Style::default()
                .size(220.0, 12.0)
                .radius(6.0)
                .draggable()
                .clickable(),
            HUE_STRIP,
        );
        ui.set_name(hue, "picker hue");
        let hue_mark = ui.add(
            hue,
            Style::row()
                .absolute(0.0, -2.0)
                .size(6.0, 16.0)
                .radius(3.0)
                .border(2.0, runity_ui::Color::hex(0xffffff)),
        );
        let row = ui.add(card, Style::row().full_width().gap(SPACE_2).center_items());
        let preview = ui.add(
            row,
            Style::row()
                .size(34.0, 24.0)
                .fixed()
                .radius(RADIUS_SM)
                .border(1.0, NEUTRAL_800),
        );
        let hex = ui.add_field(row, field_style().fill().mono(), "");
        ui.set_name(hex, "picker hex");
        self.popover_parts.insert(square, Part::PickerSquare);
        self.popover_parts.insert(hue, Part::PickerHue);
        self.popover_parts.insert(hex, Part::PickerHex);
        if !self.hue_drawn {
            self.hue_drawn = true;
            self.images.push((HUE_STRIP, PICTURE, hue_pixels()));
        }
        self.images.push((SV_SQUARE, PICTURE, sv_pixels(self.hsv[0])));
        self.popover = Some(Popover::Color {
            root: ground,
            target,
            hex,
            preview,
            square_mark,
            hue_mark,
        });
        self.show_picker(ui);
    }

    /// The marks, the preview and the hex, for the colour being picked.
    fn show_picker(&mut self, ui: &mut Ui) {
        let Some(Popover::Color {
            hex,
            preview,
            square_mark,
            hue_mark,
            ..
        }) = &self.popover
        else {
            return;
        };
        let [h, s, v] = self.hsv;
        let rgb = rgb_of(self.hsv);
        ui.restyle(*square_mark, |st| st.absolute(s * 220.0 - 5.0, (1.0 - v) * 150.0 - 5.0));
        ui.restyle(*hue_mark, |st| st.absolute(h * 220.0 - 3.0, -2.0));
        ui.restyle(*preview, |st| {
            st.background(runity_ui::Color::rgba(rgb[0], rgb[1], rgb[2], 255))
        });
        if ui.focused() != Some(*hex) {
            ui.set_text(*hex, &format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]));
        }
    }

    fn popover_event(
        &mut self,
        ui: &mut Ui,
        session: &mut Session,
        node: NodeId,
        part: Part,
        event: &Event,
        requests: &mut Requests,
    ) {
        let target = match &self.popover {
            Some(Popover::Color { target, .. }) => Some(target.clone()),
            _ => None,
        };
        match (part, event) {
            (Part::Dismiss, Event::Click { .. }) => self.close_popover(ui),
            (Part::AddSearch, Event::Changed(text)) => {
                let text = text.clone();
                self.fill_add(ui, session, &text);
            }
            (Part::AddSearch, Event::Submit(_)) => {
                let first = match &self.popover {
                    Some(Popover::Add { hits, .. }) => hits.first().cloned(),
                    _ => None,
                };
                if let Some(first) = first {
                    self.add(ui, session, first, requests);
                }
            }
            (Part::AddSearch, Event::Cancel) => self.close_popover(ui),
            (Part::AddEntry(addition), Event::Click { .. }) => {
                self.add(ui, session, addition, requests)
            }
            (
                part @ (Part::PickerSquare | Part::PickerHue),
                Event::Press { x, y, .. } | Event::Drag { x, y, .. },
            ) => {
                let Some(target) = target else { return };
                if matches!(event, Event::Press { .. }) {
                    session.begin_gesture();
                    self.scrubbing = Some((0.0, 0.0));
                }
                let r = ui.rect(node);
                let fx = ((x - r.x) / r.width.max(1.0)).clamp(0.0, 1.0);
                let fy = ((y - r.y) / r.height.max(1.0)).clamp(0.0, 1.0);
                if part == Part::PickerSquare {
                    self.hsv[1] = fx;
                    self.hsv[2] = 1.0 - fy;
                } else {
                    self.hsv[0] = fx.min(0.999);
                    self.images.push((SV_SQUARE, PICTURE, sv_pixels(self.hsv[0])));
                }
                self.show_picker(ui);
                self.apply_color(session, &target);
            }
            (Part::PickerSquare | Part::PickerHue, Event::Release { .. }) => {
                if self.scrubbing.take().is_some() {
                    session.end_gesture();
                }
                self.built = false;
                requests.refresh = true;
            }
            (Part::PickerHex, Event::Submit(text)) => {
                let Some(target) = target else { return };
                match parse_hex(text) {
                    Some(rgb) => {
                        self.hsv = hsv_of(rgb);
                        self.images.push((SV_SQUARE, PICTURE, sv_pixels(self.hsv[0])));
                        session.begin_gesture();
                        self.apply_color(session, &target);
                        session.end_gesture();
                        self.show_picker(ui);
                    }
                    None => session.say(Level::Error, format!("{text:?} is not a colour: #rrggbb")),
                }
                self.built = false;
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
