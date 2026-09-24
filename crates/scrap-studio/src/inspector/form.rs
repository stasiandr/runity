//! A field's value as a form, the way Unity's and Godot's inspectors draw
//! one: a struct as a group of labelled lines, a number as a box whose
//! label drags it, a flag as a check box, three numbers as X, Y and Z, a
//! colour as a swatch, an enum as a list to pick from, an absent part of
//! the scene's look as a switched-off section.
//!
//! What a value looks like comes from the engine — the type of the module
//! that reads the field, traced ([`Session::field_shape`]) — and, where it
//! has nothing to say, from the value itself. Every box writes back through
//! the one path typing RON did: the whole value again, with that box's text
//! changed and every other byte as it was ([`super::tree`]), handed to
//! `Session::set_field` (or `set_environment`) as one undo step.

use scrap::shape::Shape;
use scrap_editor::console::Level;
use scrap_editor::panels::{Field, MIXED};
use scrap_editor::Session;
use scrap_ui::{NodeId, Style, Ui};

use super::tree::{self, Kind, Node, Step};
use super::{title, trim_number, Inspector, Part, Scrub, AXES};
use crate::theme::*;

/// How wide a line's label is at the top level; nested lines give up
/// their indent from it, so the boxes stay in one column.
pub(super) const LABEL_WIDTH: f32 = 112.0;
/// How far each level of a form is indented.
const INDENT: f32 = 12.0;

/// Whose value a form edits.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Owner {
    /// A field of what the Inspector shows (all of it, when several).
    Entity(String),
    /// A field of the scene's look: `sun`, `fog`, `sky`…
    Scene(String),
}

/// A place in a field's value: whose, and the steps into it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Place {
    pub owner: Owner,
    pub path: Vec<Step>,
}

impl Place {
    fn at(&self, step: Step) -> Place {
        let mut path = self.path.clone();
        path.push(step);
        Place {
            owner: self.owner.clone(),
            path,
        }
    }

    fn key(&self, key: &str) -> Place {
        self.at(Step::Key(key.to_string()))
    }
}

/// What a form's box does when used.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Control {
    /// A number; `whole` for a count.
    Number { whole: bool },
    /// A string, typed without its quotes.
    Text,
    /// A name the form knows no choices for, typed as it is.
    Name,
    /// A flag, and what it is now.
    Toggle(bool),
    /// A list to pick from: each entry's label and the value it writes.
    Choose(Vec<(String, String)>),
    /// An optional part's switch: the value it is set to when switched
    /// on, `None` to switch it off.
    Optional(Option<String>),
    /// A group's arrow, by the key its folding is kept under.
    Fold(String, bool),
    /// A list's plus: the item it adds.
    Push(String),
    /// A list item's trash.
    Pop,
}

/// What a form is being laid out for.
struct Cx {
    /// The field, as named on the nodes: `light`, `scene fog`, `door`.
    prefix: String,
    /// The value, as the first of what is shown holds it.
    text: String,
    /// The same field of the others shown, where they differ: a box they
    /// disagree on shows `—`.
    others: Vec<(String, Node)>,
    /// The field's engine name, to ask the session about it.
    field: String,
}

impl Cx {
    fn mixed(&self, path: &[Step], node: &Node) -> bool {
        let mine = &self.text[node.span.clone()];
        self.others.iter().any(|(text, root)| {
            root.get(path)
                .is_none_or(|n| text[n.span.clone()].trim() != mine.trim())
        })
    }

    fn name(&self, path: &[Step]) -> String {
        let mut out = self.prefix.clone();
        for step in path {
            out.push(' ');
            out.push_str(&step.to_string());
        }
        out
    }
}

/// A key that names a colour: `color`, `end_color`, `tint`, a sky's
/// `zenith`.
fn is_colour(key: &str) -> bool {
    key.contains("color")
        || key.contains("colour")
        || key.contains("tint")
        || matches!(
            key,
            "zenith" | "horizon" | "ground" | "equator" | "sky" | "albedo"
        )
}

/// The fields that are names typed as text, or laid out by the Inspector
/// itself: never a form.
fn is_plain(field: &str) -> bool {
    matches!(
        field,
        "name"
            | "model"
            | "prefab"
            | "layer"
            | "animator"
            | "bone"
            | "inactive"
            | "position"
            | "rotation"
            | "scale"
    )
}

/// What an Option or a wrapper holds, and whether it is an Option.
fn unwrap_option(shape: Option<&Shape>) -> (Option<&Shape>, bool) {
    match shape {
        Some(Shape::Option(inner)) => (Some(inner), true),
        other => (other, false),
    }
}

/// The shape of a struct's field.
fn field_of<'a>(shape: Option<&'a Shape>, key: &str) -> Option<&'a Shape> {
    match shape {
        Some(Shape::Struct(fields)) => fields.iter().find(|(k, _)| k == key).map(|(_, s)| s),
        _ => None,
    }
}

/// A value for a variant of an enum: its name and, for one with fields,
/// each field — kept from `current` where it has one of that name, the
/// shape's plainest value otherwise.
fn variant_text(name: &str, content: &Shape, current: Option<(&str, &Node)>) -> String {
    let kept = |key: &str| -> Option<String> {
        let (text, node) = current?;
        node.get(&[Step::Key(key.to_string())])
            .map(|n| text[n.span.clone()].to_string())
    };
    match content {
        Shape::Struct(fields) => format!(
            "{name}({})",
            fields
                .iter()
                .map(|(k, s)| format!("{k}: {}", kept(k).unwrap_or_else(|| s.example())))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Shape::Tuple(items) => format!(
            "{name}({})",
            items
                .iter()
                .map(Shape::example)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Shape::Unit => name.to_string(),
        other => format!("{name}({})", other.example()),
    }
}

impl Inspector {
    /// Whether this field is laid out as a form (outside Debug mode).
    pub(super) fn is_form(&self, f: &Field) -> bool {
        if is_plain(&f.name) || f.name.starts_with("game") {
            return false;
        }
        // A material is an object field, its parts under it when it is
        // written out in full (`entity_form_parts`).
        f.name != "material"
    }

    /// One field of what is shown as a form: its line, and the lines of
    /// its parts under it. `line` is the field's line, its label in it
    /// already.
    pub(super) fn entity_form(&mut self, ui: &mut Ui, session: &Session, line: NodeId, f: &Field) {
        let Some(cx) = self.entity_cx(session, f) else {
            return;
        };
        let place = Place {
            owner: Owner::Entity(f.name.clone()),
            path: Vec::new(),
        };
        self.form_root(ui, session, &cx, line, place);
    }

    /// The lines of a field's parts only, a level in, under a line laid out
    /// otherwise: a material written out in full, under its object field.
    pub(super) fn entity_form_parts(&mut self, ui: &mut Ui, session: &Session, f: &Field) {
        let Some(cx) = self.entity_cx(session, f) else {
            return;
        };
        let Some(root) = tree::parse(&cx.text) else {
            return;
        };
        let shape = session.field_shape(&f.name);
        let shape = one_of(shape.as_ref(), &root);
        let place = Place {
            owner: Owner::Entity(f.name.clone()),
            path: Vec::new(),
        };
        self.form_children(ui, session, &cx, &place, &root, shape, 1, false);
    }

    /// What a field of what is shown is laid out from.
    fn entity_cx(&self, session: &Session, f: &Field) -> Option<Cx> {
        let mut text = f.value.clone();
        let mut others = Vec::new();
        if f.value == MIXED {
            // Several shown and they disagree: the first's value laid out,
            // each box that the others disagree on showing `—`.
            let values: Vec<String> = self
                .showing
                .iter()
                .filter_map(|id| {
                    session
                        .inspect(*id)?
                        .into_iter()
                        .find(|g| g.name == f.name)
                        .map(|g| g.value)
                })
                .collect();
            let first = values.first()?;
            text = first.clone();
            others = values[1..]
                .iter()
                .filter_map(|v| Some((v.clone(), tree::parse(v)?)))
                .collect();
        }
        let prefix = f
            .name
            .strip_prefix("components.")
            .unwrap_or(&f.name)
            .to_string();
        Some(Cx {
            prefix,
            text,
            others,
            field: f.name.clone(),
        })
    }

    /// One field of a game component laid out by its shape, where the
    /// shape is more than a box: a struct, a list, an option. Into `line`,
    /// its label in it already; a field the value leaves out is shown as
    /// the shape's plainest value, as the rest of the component's form does.
    pub(super) fn component_part(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        line: NodeId,
        f: &Field,
        key: &str,
        shape: &Shape,
    ) {
        let place = Place {
            owner: Owner::Entity(f.name.clone()),
            path: vec![Step::Key(key.to_string())],
        };
        let mut text = f.value.clone();
        let root = tree::parse(&text);
        if root.as_ref().is_some_and(|r| r.get(&place.path).is_none()) {
            let stand_in = shape.example();
            self.stand_ins.insert(place.clone(), stand_in.clone());
            text = tree::set(&text, &place.path, &stand_in).unwrap_or(text);
        }
        let Some(node) = tree::parse(&text).and_then(|r| r.get(&place.path).cloned()) else {
            return;
        };
        let cx = Cx {
            prefix: f
                .name
                .strip_prefix("components.")
                .unwrap_or(&f.name)
                .to_string(),
            text,
            others: Vec::new(),
            field: f.name.clone(),
        };
        self.form_value(
            ui,
            session,
            &cx,
            line,
            place,
            &node,
            Some(shape),
            &[],
            1,
            false,
        );
    }

    /// The scene's look, each field a section: the sun and the fog always,
    /// the rest switched on or off.
    pub(super) fn scene_form(&mut self, ui: &mut Ui, session: &Session) {
        for (field, value) in session.environment() {
            let place = Place {
                owner: Owner::Scene(field.to_string()),
                path: Vec::new(),
            };
            let cx = Cx {
                prefix: format!("scene {field}"),
                text: value.clone(),
                others: Vec::new(),
                field: field.to_string(),
            };
            let optional = !matches!(field, "sun" | "fog");
            let on = value.trim() != "None";
            let fold = format!("scene {field}");
            let open = on && self.is_open(&fold, true);
            let head = ui.add(
                self.body,
                Style::row()
                    .full_width()
                    .padding_x(SPACE_4)
                    .padding_y(3.0)
                    .gap(SPACE_2)
                    .center_items(),
            );
            let arrow = self.fold_arrow(ui, head, &fold, open, on);
            ui.set_name(arrow, format!("fold {}", cx.prefix));
            if optional {
                let blank = session.field_blank(field).unwrap_or_else(|| {
                    session
                        .field_shape(field)
                        .map_or("()".into(), |s| s.example())
                });
                let check = self.check_box(ui, head, on, false);
                ui.set_name(check, cx.prefix.clone());
                self.parts.insert(
                    check,
                    Part::Form(place.clone(), Control::Optional((!on).then_some(blank))),
                );
            }
            ui.add_text(
                head,
                Style::default()
                    .fill()
                    .text_size(12.0)
                    .text_color(if on { TEXT } else { MUTED })
                    .nowrap(),
                &title(field),
            );
            if open {
                let Some(root) = tree::parse(&value) else {
                    self.raw_line(ui, &cx, place, &value);
                    continue;
                };
                let shape = session.field_shape(field);
                self.form_children(ui, session, &cx, &place, &root, shape.as_ref(), 1, false);
            }
        }
    }

    /// A field's value at the top: its controls in its line, its parts
    /// under it.
    fn form_root(&mut self, ui: &mut Ui, session: &Session, cx: &Cx, line: NodeId, place: Place) {
        let Some(root) = tree::parse(&cx.text) else {
            // Not RON this reads (a value from a newer build, a typo in the
            // file): the text, so nothing is hidden.
            self.raw_box(ui, line, cx, place, &cx.text.clone());
            return;
        };
        let shape = session.field_shape(&cx.field);
        let variants = session.field_variants(&cx.field);
        self.form_value(
            ui,
            session,
            cx,
            line,
            place,
            &root,
            shape.as_ref(),
            &variants,
            0,
            false,
        );
    }

    /// The controls for `node` into `line`; for a value with parts, the
    /// parts' lines after it, a level in.
    #[allow(clippy::too_many_arguments)]
    fn form_value(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        cx: &Cx,
        line: NodeId,
        place: Place,
        node: &Node,
        shape: Option<&Shape>,
        variants: &[(String, Shape)],
        depth: usize,
        unknown: bool,
    ) {
        let (shape, optional) = unwrap_option(shape);
        let shape = one_of(shape, node);
        // An enum in a value: its variants, with what each holds when the
        // shape says (`Crate(Food)`), bare names when it only names them.
        let named: Vec<(String, Shape)>;
        let variants = match shape {
            _ if !variants.is_empty() => variants,
            Some(Shape::Tagged(v)) => v.as_slice(),
            Some(Shape::Enum(names)) => {
                named = names.iter().map(|n| (n.clone(), Shape::Unit)).collect();
                named.as_slice()
            }
            _ => variants,
        };
        let name = cx.name(&place.path);
        let mixed = !unknown && cx.mixed(&place.path, node);
        // A link to an asset or an entity: Unity's object field, never its
        // name or its id typed. `None` in an option is the field naming
        // nothing.
        use super::object::{Holds, ObjectRef, Slot};
        let holds = match shape {
            Some(Shape::Asset(kind)) => Some((kind.as_str(), Holds::Link)),
            Some(Shape::AssetId(kind)) => Some((kind.as_str(), Holds::AssetId)),
            Some(Shape::Entity) => Some(("entity", Holds::EntityRef)),
            Some(Shape::EntityId) => Some(("entity", Holds::EntityId)),
            Some(Shape::Record(record)) => Some((record.as_str(), Holds::Record)),
            _ => None,
        };
        if let Some((kind, holds)) = holds {
            let mut target = ObjectRef::new(Slot::Form(place.clone()), kind, holds);
            target.optional = optional;
            let text = &cx.text[node.span.clone()];
            let shown = if optional && text.trim() == "None" {
                super::object::Shown {
                    name: Some(String::new()),
                    missing: false,
                }
            } else {
                self.shown(session, &target, text)
            };
            self.object_field(ui, session, line, &name, target, shown, mixed);
            return;
        }
        let key = place
            .path
            .iter()
            .rev()
            .find_map(|s| match s {
                Step::Key(k) => Some(k.as_str()),
                Step::At(_) => None,
            })
            .unwrap_or(&cx.field);
        match &node.kind {
            Kind::Name(n) if n == "None" && (optional || depth == 0) => {
                let on_value = shape.map_or("()".to_string(), Shape::example);
                let check = self.check_box(ui, line, false, mixed);
                ui.set_name(check, name);
                self.parts.insert(
                    check,
                    Part::Form(place, Control::Optional(Some(format!("Some({on_value})")))),
                );
            }
            Kind::Tuple {
                name: Some(n),
                items,
            } if n == "Some" && items.len() == 1 => {
                let check = self.check_box(ui, line, true, mixed);
                ui.set_name(check, format!("{name} some"));
                self.parts
                    .insert(check, Part::Form(place.clone(), Control::Optional(None)));
                let inner = place.at(Step::At(0));
                self.form_value(
                    ui,
                    session,
                    cx,
                    line,
                    inner,
                    &items[0],
                    shape,
                    &[],
                    depth,
                    unknown,
                );
            }
            Kind::Number => {
                let whole = match shape {
                    Some(Shape::Int) => true,
                    Some(Shape::Float) => false,
                    _ => !cx.text[node.span.clone()].contains(['.', 'e', 'E', 'i', 'N']),
                };
                let shown = if unknown {
                    String::new()
                } else if mixed {
                    MIXED.to_string()
                } else {
                    trim_number(&cx.text[node.span.clone()])
                };
                let b = ui.add_field(line, field_style().fill().mono().text_size(11.5), &shown);
                if unknown {
                    ui.set_placeholder(b, "default");
                }
                ui.set_name(b, name.clone());
                self.form_nodes.insert(place.clone(), b);
                self.parts
                    .insert(b, Part::Form(place.clone(), Control::Number { whole }));
                // The label is the handle, as Unity's is.
                if let Some(label) = self.last_label {
                    if !self.playing {
                        ui.restyle(label, |s| s.hover(HOVER).draggable().clickable());
                        ui.set_name(label, format!("scrub {name}"));
                        self.parts
                            .insert(label, Part::Scrub(Scrub::Form(place, whole)));
                    }
                }
            }
            Kind::Bool(on) => {
                let check = self.check_box(ui, line, *on && !unknown, mixed);
                ui.set_name(check, name);
                self.parts
                    .insert(check, Part::Form(place, Control::Toggle(*on)));
                ui.add(line, Style::row().fill());
            }
            Kind::Text(t) => {
                let shown = if mixed { MIXED } else { t.as_str() };
                let b = ui.add_field(line, field_style().fill(), shown);
                ui.set_name(b, name);
                self.parts.insert(b, Part::Form(place, Control::Text));
            }
            Kind::Char => self.raw_box(ui, line, cx, place, &cx.text[node.span.clone()]),
            Kind::Name(n) => {
                // A variant with nothing in it: the engine's list to pick
                // from, or the name typed when there is none.
                let choices: Vec<(String, String)> = if !variants.is_empty() {
                    variants
                        .iter()
                        .map(|(v, content)| (v.clone(), variant_text(v, content, None)))
                        .collect()
                } else if let Some(Shape::Enum(names)) = shape {
                    names.iter().map(|v| (v.clone(), v.clone())).collect()
                } else {
                    Vec::new()
                };
                if choices.is_empty() {
                    let b = ui.add_field(line, field_style().fill(), n);
                    ui.set_name(b, name);
                    self.parts.insert(b, Part::Form(place, Control::Name));
                } else {
                    let shown = if mixed { MIXED } else { n.as_str() };
                    let pick = self.dropdown(ui, line, shown);
                    ui.set_name(pick, name);
                    self.parts
                        .insert(pick, Part::Form(place, Control::Choose(choices)));
                }
            }
            Kind::Tuple { name: None, items }
                if node.numbers().is_some() && is_colour(key) && (3..=4).contains(&items.len()) =>
            {
                let rgb = [0, 1, 2].map(|i| {
                    let v: f32 = cx.text[items[i].span.clone()].parse().unwrap_or(0.0);
                    (v.clamp(0.0, 1.0) * 255.0).round() as u8
                });
                let rgb = (!unknown).then_some(rgb);
                self.swatch(ui, line, &name, rgb, super::ColorTarget::Form(place), mixed);
            }
            Kind::Tuple { name: None, items } if node.numbers().is_some() && items.len() <= 4 => {
                let whole = |i: usize| match shape {
                    Some(Shape::Tuple(s)) => matches!(s.get(i), Some(Shape::Int)),
                    _ => false,
                };
                let boxes = ui.add(line, Style::row().fill().gap(SPACE_1));
                for (i, item) in items.iter().enumerate() {
                    let b = ui.add(boxes, Style::row().fill().gap(2.0).center_items());
                    let letter = ["X", "Y", "Z", "W"][i];
                    let axis = place.at(Step::At(i));
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
                    ui.set_name(handle, format!("scrub {name} {}", letter.to_lowercase()));
                    ui.add_text(
                        handle,
                        Style::default()
                            .text_size(10.5)
                            .text_color(AXES.get(i).copied().unwrap_or(LABEL))
                            .nowrap(),
                        letter,
                    );
                    if !self.playing {
                        self.parts
                            .insert(handle, Part::Scrub(Scrub::Form(axis.clone(), whole(i))));
                    }
                    let shown = if unknown {
                        String::new()
                    } else if !cx.others.is_empty() && cx.mixed(&axis.path, item) {
                        MIXED.to_string()
                    } else {
                        trim_number(&cx.text[item.span.clone()])
                    };
                    let slot = ui.add_field(b, field_style().fill().mono().text_size(11.5), &shown);
                    if unknown {
                        ui.set_placeholder(slot, "–");
                    }
                    ui.set_name(slot, format!("{name} {}", letter.to_lowercase()));
                    self.form_nodes.insert(axis.clone(), slot);
                    self.parts
                        .insert(slot, Part::Form(axis, Control::Number { whole: whole(i) }));
                }
            }
            Kind::Struct { name: Some(n), .. } | Kind::Tuple { name: Some(n), .. } => {
                // A variant with something in it — `Box(half: …)` — or a
                // wrapper: the variant to pick, then what it holds.
                let content = variants
                    .iter()
                    .find(|(v, _)| v == n)
                    .map(|(_, s)| s.clone());
                if variants.is_empty() {
                    tag(ui, line, n, ACCENT_900, ACCENT_300);
                    ui.add(line, Style::row().fill());
                } else {
                    let here = (cx.text.as_str(), node);
                    let choices = variants
                        .iter()
                        .map(|(v, s)| (v.clone(), variant_text(v, s, Some(here))))
                        .collect();
                    let pick = self.dropdown(
                        ui,
                        line,
                        if mixed
                            && cx.others.iter().any(|(_, r)| {
                                r.get(&place.path).is_none_or(|o| !same_variant(o, n))
                            })
                        {
                            MIXED
                        } else {
                            n
                        },
                    );
                    ui.set_name(pick, name);
                    self.parts
                        .insert(pick, Part::Form(place.clone(), Control::Choose(choices)));
                }
                // What the variant holds, as the shape says; nothing to go
                // by for a variant it only names.
                let inner = if variants.is_empty() {
                    shape
                } else {
                    content.as_ref().filter(|c| **c != Shape::Unit)
                };
                match &node.kind {
                    Kind::Tuple { items, .. }
                        if items.len() == 1
                            && content
                                .as_ref()
                                .is_none_or(|c| !matches!(c, Shape::Tuple(_))) =>
                    {
                        // A newtype: its one value — a group's lines under
                        // it, or one line by the variant's name
                        // (`Crate: Cabbage`).
                        let one = place.at(Step::At(0));
                        if is_group(&items[0]) {
                            self.form_children(
                                ui,
                                session,
                                cx,
                                &one,
                                &items[0],
                                inner,
                                depth + 1,
                                unknown,
                            );
                        } else {
                            self.form_line(
                                ui,
                                session,
                                cx,
                                one,
                                n,
                                &items[0],
                                inner,
                                depth + 1,
                                unknown,
                            );
                        }
                    }
                    _ => {
                        self.form_children(ui, session, cx, &place, node, inner, depth + 1, unknown)
                    }
                }
            }
            _ => {
                // A struct, a list, a map, a tuple of things: a group, its
                // parts a level in.
                let summary = match &node.kind {
                    Kind::List(items) => Some(format!(
                        "{} item{}",
                        items.len(),
                        if items.len() == 1 { "" } else { "s" }
                    )),
                    _ => None,
                };
                if let Some(summary) = summary {
                    ui.add_text(
                        line,
                        text().fill().text_size(11.5).text_color(MUTED),
                        &summary,
                    );
                } else {
                    ui.add(line, Style::row().fill());
                }
                if depth > 0 && !self.is_open(&name, group_len(node, shape) <= 4) {
                    return;
                }
                self.form_children(ui, session, cx, &place, node, shape, depth + 1, unknown);
            }
        }
    }

    /// The lines of a value's parts, `depth` levels in: a struct's fields
    /// in the order its type has them (those the text leaves out at what
    /// they stand at), a list's items with a trash each and a plus, a
    /// tuple's items by place.
    #[allow(clippy::too_many_arguments)]
    fn form_children(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        cx: &Cx,
        place: &Place,
        node: &Node,
        shape: Option<&Shape>,
        depth: usize,
        unknown: bool,
    ) {
        let (shape, _) = unwrap_option(shape);
        match &node.kind {
            Kind::Struct { fields, .. } => {
                let mut keys: Vec<String> = match shape {
                    Some(Shape::Struct(s)) => s.iter().map(|(k, _)| k.clone()).collect(),
                    _ => Vec::new(),
                };
                for (k, _) in fields {
                    if !keys.contains(k) {
                        keys.push(k.clone());
                    }
                }
                for key in keys {
                    let sub = place.key(&key);
                    let sub_shape = field_of(shape, &key);
                    match fields.iter().find(|(k, _)| *k == key) {
                        Some((_, value)) => {
                            self.form_line(
                                ui, session, cx, sub, &key, value, sub_shape, depth, unknown,
                            );
                        }
                        None => self.missing_line(ui, session, cx, sub, &key, sub_shape, depth),
                    }
                }
            }
            Kind::Unit => {
                if let Some(Shape::Struct(s)) = shape {
                    for (key, sub_shape) in s.clone() {
                        let sub = place.key(&key);
                        self.missing_line(ui, session, cx, sub, &key, Some(&sub_shape), depth);
                    }
                }
            }
            Kind::Tuple { items, .. } => {
                for (i, item) in items.iter().enumerate() {
                    let sub_shape = match shape {
                        Some(Shape::Tuple(s)) => s.get(i),
                        _ => None,
                    };
                    let label = format!("{i}");
                    self.form_line(
                        ui,
                        session,
                        cx,
                        place.at(Step::At(i)),
                        &label,
                        item,
                        sub_shape,
                        depth,
                        unknown,
                    );
                }
            }
            Kind::List(items) => {
                let item_shape = match shape {
                    Some(Shape::List(s)) => Some(s.as_ref()),
                    _ => None,
                };
                for (i, item) in items.iter().enumerate() {
                    let sub = place.at(Step::At(i));
                    let label = format!("Item {i}");
                    let line = self.form_line(
                        ui,
                        session,
                        cx,
                        sub.clone(),
                        &label,
                        item,
                        item_shape,
                        depth,
                        unknown,
                    );
                    if !self.playing {
                        let trash = self.small_button(ui, line, "trash");
                        ui.set_name(trash, format!("{} remove", cx.name(&sub.path)));
                        self.parts.insert(trash, Part::Form(sub, Control::Pop));
                    }
                }
                if !self.playing {
                    // A new item: the last one again, or the plainest there is.
                    let item = items
                        .last()
                        .map(|n| cx.text[n.span.clone()].to_string())
                        .or_else(|| item_shape.map(Shape::example))
                        .unwrap_or_else(|| "0.0".into());
                    let line = self.indented(ui, depth);
                    ui.add(line, Style::row().width(self.label_width(depth)).fixed());
                    let add = ui.add(
                        line,
                        Style::row()
                            .height(20.0)
                            .padding_x(6.0)
                            .gap(SPACE_1)
                            .center_items()
                            .radius(6.0)
                            .border(1.0, DIVIDER)
                            .hover(HOVER)
                            .clickable(),
                    );
                    icon(ui, add, "plus", LABEL);
                    ui.add_text(add, text().text_size(11.5).text_color(LABEL), "Add");
                    ui.set_name(add, format!("{} add", cx.name(&place.path)));
                    self.parts
                        .insert(add, Part::Form(place.clone(), Control::Push(item)));
                }
            }
            Kind::Map(entries) => {
                let value_shape = match shape {
                    Some(Shape::Map(_, v)) => Some(v.as_ref()),
                    _ => None,
                };
                for (i, (k, v)) in entries.iter().enumerate() {
                    let label = match &k.kind {
                        Kind::Text(t) | Kind::Name(t) => t.clone(),
                        _ => cx.text[k.span.clone()].to_string(),
                    };
                    self.form_line(
                        ui,
                        session,
                        cx,
                        place.at(Step::At(i)),
                        &label,
                        v,
                        value_shape,
                        depth,
                        unknown,
                    );
                }
            }
            _ => {}
        }
    }

    /// One line of a form: the label, `depth` levels in, and the value.
    #[allow(clippy::too_many_arguments)]
    fn form_line(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        cx: &Cx,
        place: Place,
        key: &str,
        node: &Node,
        shape: Option<&Shape>,
        depth: usize,
        unknown: bool,
    ) -> NodeId {
        let line = self.indented(ui, depth);
        let group = is_group(node);
        let name = cx.name(&place.path);
        let open = self.is_open(&name, group_len(node, shape) <= 4);
        let label_row = ui.add(
            line,
            Style::row()
                .width(self.label_width(depth))
                .fixed()
                .min_height(22.0)
                .gap(2.0)
                .center_items()
                .radius(RADIUS_SM),
        );
        if group {
            let arrow = self.fold_arrow(ui, label_row, &name, open, true);
            ui.set_name(arrow, format!("fold {name}"));
        }
        self.label_text(
            ui,
            label_row,
            &title(key),
            if unknown { MUTED } else { LABEL },
        );
        self.last_label = Some(label_row);
        self.form_value(
            ui,
            session,
            cx,
            line,
            place,
            node,
            shape,
            &[],
            depth,
            unknown,
        );
        self.last_label = None;
        line
    }

    /// A field the value leaves out: shown at what it stands at. The
    /// engine is asked: the shape's plainest value, a flag's other side,
    /// each variant, written in and read back — the one the file would
    /// leave out again is the default. A number whose default is none of
    /// those shows an empty box that says so.
    #[allow(clippy::too_many_arguments)]
    fn missing_line(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        cx: &Cx,
        place: Place,
        key: &str,
        shape: Option<&Shape>,
        depth: usize,
    ) {
        let Some(shape) = shape else { return };
        // A link left out links nothing: `""`, not a typed link's `ModelLink("")`.
        let example = match shape {
            Shape::Asset(_) => "\"\"".to_string(),
            other => other.example(),
        };
        let mut candidates = vec![example.clone()];
        match shape {
            Shape::Bool => candidates.push("true".into()),
            Shape::Enum(names) => candidates.extend(names.iter().skip(1).cloned()),
            _ => {}
        }
        let known = candidates.into_iter().find(|candidate| {
            let Some(with) = tree::set(&cx.text, &place.path, candidate) else {
                return false;
            };
            session
                .field_normal(&cx.field, &with)
                .ok()
                .and_then(|normal| tree::parse(&normal))
                .is_some_and(|root| root.get(&place.path).is_none())
        });
        let stand_in = known.clone().unwrap_or(example);
        self.stand_ins.insert(place.clone(), stand_in.clone());
        // Laid out as the value will be once written: the stand-in written
        // in, so every box's place is where it will be.
        let Some(with) = tree::set(&cx.text, &place.path, &stand_in) else {
            return;
        };
        let Some(node) = tree::parse(&with).and_then(|root| root.get(&place.path).cloned()) else {
            return;
        };
        let there = Cx {
            prefix: cx.prefix.clone(),
            text: with,
            others: Vec::new(),
            field: cx.field.clone(),
        };
        let line = self.indented(ui, depth);
        let label_row = ui.add(
            line,
            Style::row()
                .width(self.label_width(depth))
                .fixed()
                .min_height(22.0)
                .center_items()
                .radius(RADIUS_SM),
        );
        self.label_text(ui, label_row, &title(key), LABEL);
        self.last_label = Some(label_row);
        self.form_value(
            ui,
            session,
            &there,
            line,
            place,
            &node,
            Some(shape),
            &[],
            depth,
            known.is_none(),
        );
        self.last_label = None;
    }

    // --- pieces -----------------------------------------------------------

    fn label_width(&self, depth: usize) -> f32 {
        LABEL_WIDTH - depth as f32 * INDENT
    }

    /// A line of a form, `depth` levels in.
    fn indented(&mut self, ui: &mut Ui, depth: usize) -> NodeId {
        ui.add(
            self.body,
            Style::row()
                .full_width()
                .padding_x(SPACE_4)
                .padding_left(SPACE_4 + depth as f32 * INDENT)
                .padding_y(2.0)
                .gap(SPACE_2)
                .center_items(),
        )
    }

    /// A label that wraps rather than cuts a word in half.
    pub(super) fn label_text(
        &mut self,
        ui: &mut Ui,
        row: NodeId,
        label: &str,
        color: scrap_ui::Color,
    ) {
        ui.add_text(
            row,
            Style::default().fill().text_size(12.0).text_color(color),
            label,
        );
    }

    pub(super) fn is_open(&self, key: &str, default: bool) -> bool {
        self.folds.get(key).copied().unwrap_or(default)
    }

    /// The arrow before a group's label: a click folds it or opens it.
    pub(super) fn fold_arrow(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        key: &str,
        open: bool,
        enabled: bool,
    ) -> NodeId {
        let arrow = ui.add(
            parent,
            Style::row()
                .size(14.0, 18.0)
                .fixed()
                .center()
                .radius(RADIUS_SM)
                .hover(HOVER)
                .clickable(),
        );
        icon(
            ui,
            arrow,
            if open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            if enabled { LABEL } else { MUTED.alpha(30) },
        );
        if enabled {
            self.parts.insert(
                arrow,
                Part::Form(
                    Place {
                        owner: Owner::Scene(String::new()),
                        path: Vec::new(),
                    },
                    Control::Fold(key.to_string(), open),
                ),
            );
        }
        arrow
    }

    /// A check box, ticked or not; a dash when those shown disagree.
    pub(super) fn check_box(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        on: bool,
        mixed: bool,
    ) -> NodeId {
        let lit = on || mixed;
        let check = ui.add(
            parent,
            Style::row()
                .size(16.0, 16.0)
                .fixed()
                .center()
                .radius(4.0)
                .border(1.0, if lit { ACCENT } else { NEUTRAL_500 })
                .background(if lit {
                    ACCENT
                } else {
                    scrap_ui::Color::TRANSPARENT
                })
                .hover_border(ACCENT)
                .clickable(),
        );
        if mixed {
            icon(ui, check, "minus", NEUTRAL_900);
        } else if on {
            icon(ui, check, "check", NEUTRAL_900);
        }
        check
    }

    /// A list to pick from, closed: what is picked and an arrow.
    fn dropdown(&mut self, ui: &mut Ui, parent: NodeId, shown: &str) -> NodeId {
        let pick = ui.add(
            parent,
            Style::row()
                .fill()
                .height(22.0)
                .padding_x(6.0)
                .gap(SPACE_2)
                .center_items()
                .radius(6.0)
                .background(BG)
                .border(1.0, DIVIDER)
                .hover_border(TEXT.alpha(35))
                .clickable(),
        );
        ui.add_text(pick, text().fill().text_size(12.0), shown);
        icon(ui, pick, "chevron-down", MUTED);
        pick
    }

    fn small_button(&mut self, ui: &mut Ui, parent: NodeId, glyph: &str) -> NodeId {
        let b = ui.add(
            parent,
            Style::row()
                .size(20.0, 22.0)
                .fixed()
                .center()
                .radius(6.0)
                .hover(HOVER)
                .clickable(),
        );
        icon(ui, b, glyph, MUTED);
        b
    }

    /// A colour's swatch and its `#rrggbb` in `line`: a click opens the
    /// picker.
    pub(super) fn swatch(
        &mut self,
        ui: &mut Ui,
        line: NodeId,
        name: &str,
        rgb: Option<[u8; 3]>,
        target: super::ColorTarget,
        mixed: bool,
    ) {
        // `None`: a colour the value leaves out, at a default the form
        // cannot tell; the picker starts from black.
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
        let fill = rgb.map_or(scrap_ui::Color::TRANSPARENT, |[r, g, b]| {
            scrap_ui::Color::rgba(r, g, b, 255)
        });
        ui.add(
            swatch,
            Style::row()
                .size(28.0, 14.0)
                .fixed()
                .radius(3.0)
                .border(1.0, NEUTRAL_800)
                .background(fill),
        );
        let (hex, ink) = match rgb {
            _ if mixed => (MIXED.to_string(), TEXT),
            Some([r, g, b]) => (format!("#{r:02x}{g:02x}{b:02x}"), TEXT),
            None => ("default".to_string(), MUTED),
        };
        ui.add_text(
            swatch,
            text().mono().text_size(11.5).nowrap().text_color(ink),
            &hex,
        );
        self.parts.insert(swatch, Part::Swatch(target));
    }

    /// A value the form cannot lay out, as its text: typed into, it is
    /// written back whole at its place.
    fn raw_box(&mut self, ui: &mut Ui, line: NodeId, cx: &Cx, place: Place, value: &str) {
        let b = ui.add_field(line, field_style().fill().mono().text_size(11.5), value);
        ui.set_name(b, cx.name(&place.path));
        self.parts.insert(b, Part::Form(place, Control::Name));
    }

    fn raw_line(&mut self, ui: &mut Ui, cx: &Cx, place: Place, value: &str) {
        let line = self.indented(ui, 1);
        self.raw_box(ui, line, cx, place, value);
    }

    // --- writing back -----------------------------------------------------

    /// The value `owner` has now, for each thing it is on.
    fn values(&self, session: &Session, owner: &Owner) -> Vec<(Option<scrap::EntityId>, String)> {
        match owner {
            Owner::Scene(field) => session
                .environment()
                .into_iter()
                .filter(|(f, _)| f == field)
                .map(|(_, v)| (None, v))
                .collect(),
            Owner::Entity(field) => self
                .showing
                .iter()
                .filter_map(|id| {
                    let value = session
                        .inspect(*id)?
                        .into_iter()
                        .find(|f| f.name == *field)?
                        .value;
                    Some((Some(*id), value))
                })
                .collect(),
        }
    }

    /// Change the value of `owner` on everything it is on by `change`, as
    /// one undo step. What does not read, or that the engine refuses, is
    /// said in the Console, and nothing changes.
    pub(super) fn edit(
        &mut self,
        session: &mut Session,
        owner: &Owner,
        change: impl Fn(&str) -> Option<String>,
    ) {
        let values = self.values(session, owner);
        let many = values.len() > 1 && self.scrubbing.is_none();
        if many {
            session.begin_gesture();
        }
        for (id, value) in values {
            let Some(next) = change(&value) else {
                session.say(Level::Error, format!("{owner:?}: could not change {value}"));
                break;
            };
            if next == value {
                continue;
            }
            let done = match (id, owner) {
                (Some(id), Owner::Entity(field)) => session.set_field(id, field, &next),
                (_, Owner::Scene(field)) => session.set_environment(field, &next),
                _ => Ok(()),
            };
            if let Err(e) = done {
                session.say(Level::Error, e.to_string());
                break;
            }
        }
        if many {
            session.end_gesture();
        }
        self.built = false;
    }

    /// Write `value` at `place`: the value with that one part changed, or
    /// added where the text left it out.
    pub fn set_leaf(&mut self, session: &mut Session, place: &Place, value: &str) {
        let stand_ins = self.stand_ins.clone();
        let path = place.path.clone();
        self.edit(session, &place.owner, |text| {
            if let Some(next) = tree::set(text, &path, value) {
                return Some(next);
            }
            // Inside a part the text leaves out: that part as it was shown,
            // with this changed, written in.
            (1..path.len()).rev().find_map(|n| {
                let stand_in = stand_ins.get(&Place {
                    owner: place.owner.clone(),
                    path: path[..n].to_vec(),
                })?;
                let inner = tree::set(stand_in, &path[n..], value)?;
                tree::set(text, &path[..n], &inner)
            })
        });
    }

    /// The number at `place`, as the value holds it now.
    pub(super) fn number_at(&self, session: &Session, place: &Place) -> Option<f32> {
        let (_, text) = self.values(session, &place.owner).into_iter().next()?;
        let root = tree::parse(&text)?;
        match root.get(&place.path) {
            Some(node) => text[node.span.clone()].parse().ok(),
            None => {
                let stand_in = self.stand_ins.iter().find_map(|(p, t)| {
                    (p.owner == place.owner && place.path.starts_with(&p.path))
                        .then_some((p.path.len(), t))
                })?;
                let node = tree::parse(stand_in.1)?;
                let at = node.get(&place.path[stand_in.0..])?;
                stand_in.1[at.span.clone()].parse().ok()
            }
        }
    }

    /// The colour at `place`, as bytes (the value's numbers are 0 to 1).
    pub(super) fn colour_at(&self, session: &Session, place: &Place) -> Option<[u8; 3]> {
        let (_, text) = self.values(session, &place.owner).into_iter().next()?;
        let root = tree::parse(&text)?;
        let Some(node) = root.get(&place.path) else {
            // Left out of the value: the picker starts from black.
            return Some([0, 0, 0]);
        };
        let items = node.numbers()?;
        let v = |i: usize| -> Option<u8> {
            let n: f32 = text[items.get(i)?.span.clone()].parse().ok()?;
            Some((n.clamp(0.0, 1.0) * 255.0).round() as u8)
        };
        Some([v(0)?, v(1)?, v(2)?])
    }

    /// Set the colour at `place`, keeping a fourth number (alpha) as it is.
    pub(super) fn set_colour(&mut self, session: &mut Session, place: &Place, rgb: [f32; 3]) {
        let path = place.path.clone();
        self.edit(session, &place.owner, |text| {
            let root = tree::parse(text)?;
            let alpha = root
                .get(&path)
                .and_then(Node::numbers)
                .and_then(|items| items.get(3))
                .map(|a| format!(", {}", &text[a.span.clone()]))
                .unwrap_or_default();
            let value = format!("({:.3}, {:.3}, {:.3}{alpha})", rgb[0], rgb[1], rgb[2]);
            tree::set(text, &path, &value)
        });
    }

    /// Everything a form's control does when used.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn form_event(
        &mut self,
        ui: &mut Ui,
        session: &mut Session,
        node: NodeId,
        place: Place,
        control: Control,
        event: &scrap_ui::Event,
        requests: &mut crate::studio::Requests,
    ) {
        use scrap_ui::Event;
        match (control, event) {
            (Control::Number { whole }, Event::Submit(typed)) => {
                if typed.trim() == MIXED {
                    return;
                }
                match super::eval(typed.trim()) {
                    Some(n) => {
                        let value = if whole {
                            format!("{}", n.round() as i64)
                        } else {
                            format!("{n:?}")
                        };
                        self.set_leaf(session, &place, &value);
                    }
                    None if typed.trim().is_empty() => self.built = false,
                    None => {
                        session.say(Level::Error, format!("{typed:?} is not a number"));
                        self.built = false;
                    }
                }
                requests.refresh = true;
            }
            (Control::Text, Event::Submit(typed)) => {
                if typed != MIXED {
                    self.set_leaf(session, &place, &tree::quote(typed));
                }
                requests.refresh = true;
            }
            (Control::Name, Event::Submit(typed)) => {
                if typed.trim() != MIXED {
                    self.set_leaf(session, &place, typed.trim());
                }
                requests.refresh = true;
            }
            (Control::Text | Control::Name | Control::Number { .. }, Event::Cancel) => {
                self.built = false;
                requests.refresh = true;
            }
            (Control::Toggle(on), Event::Click { .. }) => {
                self.set_leaf(session, &place, if on { "false" } else { "true" });
                requests.refresh = true;
            }
            (Control::Optional(value), Event::Click { .. }) => {
                let value = value.unwrap_or_else(|| "None".to_string());
                if place.path.is_empty() {
                    self.folds
                        .remove(&format!("scene {}", owner_field(&place.owner)));
                }
                self.set_leaf(session, &place, &value);
                requests.refresh = true;
            }
            (Control::Choose(choices), Event::Click { .. }) => {
                let items = choices
                    .into_iter()
                    .map(|(label, value)| {
                        crate::menu::MenuItem::new(
                            &label,
                            crate::menu::Action::SetLeaf(place.clone(), value),
                        )
                    })
                    .collect();
                let r = ui.rect(node);
                requests.menu = Some((items, r.x, r.y + r.height));
            }
            (Control::Fold(key, open), Event::Click { .. }) => {
                self.folds.insert(key, !open);
                self.built = false;
                requests.refresh = true;
            }
            (Control::Push(item), Event::Click { .. }) => {
                let path = place.path.clone();
                self.edit(session, &place.owner, |text| tree::push(text, &path, &item));
                requests.refresh = true;
            }
            (Control::Pop, Event::Click { .. }) => {
                let path = place.path.clone();
                self.edit(session, &place.owner, |text| tree::remove(text, &path));
                requests.refresh = true;
            }
            _ => {}
        }
    }
}

fn owner_field(owner: &Owner) -> &str {
    match owner {
        Owner::Entity(f) | Owner::Scene(f) => f,
    }
}

fn same_variant(node: &Node, name: &str) -> bool {
    matches!(&node.kind, Kind::Struct { name: Some(n), .. } | Kind::Tuple { name: Some(n), .. } if n == name)
}

/// Which of an untagged enum's shapes a value is written as: a name or a
/// link for a link, `( … )` for a struct. Anything else as it is.
fn one_of<'a>(shape: Option<&'a Shape>, node: &Node) -> Option<&'a Shape> {
    let Some(Shape::OneOf(shapes)) = shape else {
        return shape;
    };
    let linked = matches!(node.kind, Kind::Text(_))
        || matches!(&node.kind, Kind::Tuple { name: None, items } if items.first().is_some_and(|i| matches!(i.kind, Kind::Text(_))));
    shapes.iter().find(|s| match s {
        Shape::Asset(_) => linked,
        Shape::Struct(_) => matches!(node.kind, Kind::Struct { .. } | Kind::Unit),
        _ => false,
    })
}

/// Whether a value is shown as a group of lines under its own.
fn is_group(node: &Node) -> bool {
    match &node.kind {
        Kind::Struct { name: None, .. } | Kind::List(_) | Kind::Map(_) => true,
        Kind::Tuple { name: None, items } => node.numbers().is_none() || items.len() > 4,
        _ => false,
    }
}

/// How many lines a group has: a short one starts open.
fn group_len(node: &Node, shape: Option<&Shape>) -> usize {
    let (shape, _) = unwrap_option(shape);
    match (&node.kind, shape) {
        (_, Some(Shape::Struct(fields))) => fields.len(),
        (Kind::Struct { fields, .. }, _) => fields.len(),
        (Kind::List(items) | Kind::Tuple { items, .. }, _) => items.len(),
        (Kind::Map(entries), _) => entries.len(),
        _ => 0,
    }
}
