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

use crate::menu::{Action, MenuItem};
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
}

pub struct Inspector {
    body: NodeId,
    /// Whose fields are shown, and the shape they were laid out in.
    showing: Vec<EntityId>,
    shape: Vec<(String, Option<usize>)>,
    parts: HashMap<NodeId, Part>,
    /// Slots by field and axis, and what each was last given.
    slots: Vec<(String, Option<usize>, NodeId, String)>,
    revealed: BTreeSet<String>,
    playing: bool,
}

impl Inspector {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let (card, _header, body) = panel(ui, parent, "Inspector");
        ui.set_name(card, "inspector");
        let body = ui.add(body, Style::column().fill().full_width().clip());
        let _ = card;
        Self {
            body,
            showing: Vec::new(),
            shape: Vec::new(),
            parts: HashMap::new(),
            slots: Vec::new(),
            revealed: BTreeSet::new(),
            playing: false,
        }
    }

    pub fn owns(&self, node: NodeId) -> bool {
        self.parts.contains_key(&node)
    }

    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        let ids = session.selection();
        let fields = session.inspect_all(&ids).unwrap_or_default();
        let playing = session.is_playing();
        if ids != self.showing {
            self.revealed.clear();
        }
        let shown = |f: &Field| !is_empty(&f.value) || self.revealed.contains(&f.name);
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
        if ids != self.showing || shape != self.shape || playing != self.playing {
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
            return;
        }
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

        let shown = |f: &&Field| !is_empty(&f.value) || self.revealed.contains(&f.name);
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
                self.line(ui, f);
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
        let add = ui.add_field(foot, field_style().full_width(), "");
        ui.set_name(add, "add component");
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
    fn line(&mut self, ui: &mut Ui, f: &Field) {
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
            Style::row().width(84.0).fixed().gap(SPACE_2).center_items(),
        );
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
                let slot =
                    ui.add_field(line, field_style().fill().mono().text_size(11.5), &f.value);
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

    pub fn event(
        &mut self,
        ui: &mut Ui,
        session: &mut Session,
        node: NodeId,
        event: &Event,
        requests: &mut Requests,
    ) {
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
    fn numbers_read_as_typed() {
        assert_eq!(
            axes("(2.6,0.5,2.0)"),
            Some(["2.6".into(), "0.5".into(), "2".into()])
        );
        assert_eq!(trim_number("-0.0"), "0");
    }
}
