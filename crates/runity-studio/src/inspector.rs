//! The Inspector: the selection's fields, to read and to type into.
//!
//! What the fields are and what typing into one does is
//! `Session::inspect_all` and `Session::set_field_all` — the same calls an
//! agent makes, one undo step each, with `—` where several selected things
//! disagree. This panel lays them out: the name on top, the transform as
//! three numbers a line, the rest grouped as Unity groups them, each as the
//! RON the scene file holds (a form built from the game's own types waits
//! on how modules reach the editor — DNA, open question 2).
//!
//! A field is committed on Enter or when the cursor leaves it, and only if
//! the text changed; what does not parse is said in the Console and the
//! field goes back to what the scene holds.

use std::collections::BTreeSet;

use gpui::{
    div, prelude::*, px, rgb, AnyElement, ClickEvent, Context, Entity, Focusable, SharedString,
    Window,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use runity::EntityId;
use runity_editor::console::Level;
use runity_editor::panels::{Field, MIXED};
use runity_editor::Session;

use crate::theme::*;
use crate::ui::{icon, panel, tag};

/// Where each field goes, in Unity's order. A field in none of them — a
/// game's component — goes under Components.
const OBJECT: [&str; 4] = ["model", "material", "prefab", "layer"];
const TRANSFORM: [&str; 3] = ["position", "rotation", "scale"];
const PHYSICS: [&str; 4] = ["body", "collider", "physics", "joint"];
const PARTS: [&str; 4] = ["camera", "light", "particles", "route"];

/// A field that says nothing: not shown, but offered under «Add».
fn is_empty(value: &str) -> bool {
    matches!(value, "" | "None" | "r#None" | "()" | "\"\"")
}

/// One text box: a field, or one axis of a vector field.
struct Slot {
    field: String,
    axis: Option<usize>,
    state: Entity<InputState>,
    /// What it was last given from the scene: typing that leaves it the
    /// same is not an edit.
    shown: String,
}

pub struct Inspector {
    session: Entity<Session>,
    /// Whose fields the slots hold.
    showing: Vec<EntityId>,
    slots: Vec<Slot>,
    /// Empty fields asked for with «Add», shown until something is typed.
    revealed: BTreeSet<String>,
    add_component: Entity<InputState>,
    /// The «Add component» box is emptied on the next draw, which is where
    /// a window to do it with is at hand.
    pending_clear: bool,
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

impl Inspector {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let add_component = cx.new(|cx| InputState::new(window, cx).placeholder("Add component…"));
        cx.subscribe(&add_component, |this, state, event: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let name = state.read(cx).value().trim().to_string();
                if name.is_empty() {
                    return;
                }
                let ids = this.showing.clone();
                this.session.update(cx, |session, cx| {
                    for id in ids {
                        if let Err(e) = session.add_component(id, &name) {
                            session.say(Level::Error, e.to_string());
                            break;
                        }
                    }
                    cx.notify();
                });
                this.pending_clear = true;
            }
        })
        .detach();
        Self {
            session,
            showing: Vec::new(),
            slots: Vec::new(),
            revealed: BTreeSet::new(),
            add_component,
            pending_clear: false,
        }
    }

    /// Make the slots match `fields`: new boxes for a new selection, new
    /// text for boxes nobody is typing into.
    fn sync(
        &mut self,
        ids: &[EntityId],
        fields: &[Field],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ids != self.showing.as_slice() {
            self.showing = ids.to_vec();
            self.slots.clear();
            self.revealed.clear();
        }
        let mut wanted: Vec<(String, Option<usize>, String)> = Vec::new();
        for field in fields {
            if field.name.starts_with("game") {
                continue;
            }
            match TRANSFORM
                .contains(&field.name.as_str())
                .then(|| axes(&field.value))
                .flatten()
            {
                Some(three) => {
                    for (i, text) in three.into_iter().enumerate() {
                        wanted.push((field.name.clone(), Some(i), text));
                    }
                }
                None => wanted.push((field.name.clone(), None, field.value.clone())),
            }
        }
        // Rebuilt when the set of boxes changes shape — a vector that
        // became mixed, a component added — else updated in place, so the
        // box being typed into keeps its cursor.
        let same_shape = wanted.len() == self.slots.len()
            && wanted
                .iter()
                .zip(&self.slots)
                .all(|((f, a, _), s)| *f == s.field && *a == s.axis);
        if !same_shape {
            self.slots = wanted
                .into_iter()
                .map(|(field, axis, text)| self.slot(field, axis, text, window, cx))
                .collect();
            return;
        }
        for ((_, _, text), slot) in wanted.into_iter().zip(self.slots.iter_mut()) {
            if slot.shown == text {
                continue;
            }
            let typing = slot.state.read(cx).focus_handle(cx).is_focused(window);
            if !typing {
                slot.state
                    .update(cx, |state, cx| state.set_value(text.clone(), window, cx));
                slot.shown = text;
            }
        }
    }

    fn slot(
        &self,
        field: String,
        axis: Option<usize>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Slot {
        let state = cx.new(|cx| InputState::new(window, cx).default_value(text.clone()));
        let (name, at) = (field.clone(), axis);
        cx.subscribe(&state, move |this, _, event: &InputEvent, cx| match event {
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit(&name, at, cx),
            _ => {}
        })
        .detach();
        Slot {
            field,
            axis,
            state,
            shown: text,
        }
    }

    /// Hand what was typed into `field` to the session, if it changed.
    fn commit(&mut self, field: &str, axis: Option<usize>, cx: &mut Context<Self>) {
        let text = match axis {
            None => {
                let Some(slot) = self.slots.iter().find(|s| s.field == field) else {
                    return;
                };
                let typed = slot.state.read(cx).value().to_string();
                if typed == slot.shown {
                    return;
                }
                typed
            }
            Some(_) => {
                let three: Vec<&Slot> = self.slots.iter().filter(|s| s.field == field).collect();
                let typed: Vec<String> = three
                    .iter()
                    .map(|s| s.state.read(cx).value().trim().to_string())
                    .collect();
                if three.iter().zip(&typed).all(|(s, t)| s.shown == *t) {
                    return;
                }
                // A number is what a box of a vector takes; `1+1` is not.
                let numbers: Option<Vec<f32>> = typed.iter().map(|t| t.parse().ok()).collect();
                match numbers {
                    Some(n) => format!("({:?}, {:?}, {:?})", n[0], n[1], n[2]),
                    None => {
                        self.session.update(cx, |session, cx| {
                            session.say(Level::Error, format!("{field}: every box takes a number"));
                            cx.notify();
                        });
                        self.showing.clear();
                        cx.notify();
                        return;
                    }
                }
            }
        };
        if text == MIXED {
            return;
        }
        let ids = self.showing.clone();
        let field = field.to_string();
        self.session.update(cx, |session, cx| {
            if let Err(e) = session.set_field_all(&ids, &field, &text) {
                session.say(Level::Error, e.to_string());
            }
            cx.notify();
        });
        // Whatever the scene holds now is what the boxes show — including
        // the old text, when it refused.
        for slot in self.slots.iter_mut().filter(|s| s.field == field) {
            slot.shown.clear();
        }
        cx.notify();
    }

    fn revert(&mut self, field: &str, cx: &mut Context<Self>) {
        let ids = self.showing.clone();
        let field = field.to_string();
        self.session.update(cx, |session, cx| {
            for id in ids {
                let _ = session.revert_field(id, &field);
            }
            cx.notify();
        });
    }

    fn input(&self, field: &str, axis: Option<usize>) -> Option<&Entity<InputState>> {
        self.slots
            .iter()
            .find(|s| s.field == field && s.axis == axis)
            .map(|s| &s.state)
    }

    /// One line: the label (with the override mark and its undo) and what
    /// to type into.
    fn line(&self, field: &Field, cx: &mut Context<Self>) -> AnyElement {
        let name = field.name.clone();
        let mark = field.overridden.then(|| {
            div()
                .id(SharedString::from(format!("revert-{name}")))
                .flex()
                .items_center()
                .gap(px(4.0))
                .cursor_pointer()
                .child(div().size(px(6.0)).rounded_full().bg(rgb(ACCENT)))
                .on_click(cx.listener({
                    let name = name.clone();
                    move |this, _: &ClickEvent, _, cx| this.revert(&name, cx)
                }))
        });
        let label_ink = if field.overridden {
            rgb(ACCENT_300)
        } else {
            label()
        };
        let label = div()
            .flex()
            .items_center()
            .gap(SPACE_2)
            .w(px(84.0))
            .flex_none()
            .text_size(px(12.0))
            .text_color(label_ink)
            .children(mark)
            .child(title(&field.name));

        let value: AnyElement = if self.input(&field.name, Some(0)).is_some() {
            div()
                .flex()
                .flex_1()
                .gap(SPACE_1)
                .children((0..3).filter_map(|i| {
                    let state = self.input(&field.name, Some(i))?;
                    let letter = ["X", "Y", "Z"][i];
                    Some(
                        div().flex_1().min_w_0().child(
                            Input::new(state).small().prefix(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(mix(TEXT, 40))
                                    .child(letter),
                            ),
                        ),
                    )
                }))
                .into_any_element()
        } else if let Some(state) = self.input(&field.name, None) {
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(state).small())
                .into_any_element()
        } else {
            div().flex_1().into_any_element()
        };

        let mut line = div()
            .flex()
            .items_center()
            .gap(SPACE_2)
            .px(SPACE_4)
            .py(px(3.0))
            .child(label)
            .child(value);
        if !field.shape.is_empty() {
            line = line.child(
                div()
                    .id(SharedString::from(format!("shape-{name}")))
                    .child(icon("info", muted()))
                    .tooltip({
                        let shape = field.shape.clone();
                        move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(shape.clone())
                                .build(window, cx)
                        }
                    }),
            );
        }
        line.into_any_element()
    }

    fn section(&self, name: &'static str, lines: Vec<AnyElement>) -> Option<AnyElement> {
        if lines.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_col()
                .pb(SPACE_3)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .px(SPACE_4)
                        .pt(SPACE_3)
                        .pb(SPACE_2)
                        .text_size(px(12.0))
                        .text_color(rgb(NEUTRAL_300))
                        // The rule after a heading fades out at its end —
                        // Nocturne's signature.
                        .child(name)
                        .child(div().flex_1().h(px(1.0)).bg(gpui::linear_gradient(
                            90.0,
                            gpui::linear_color_stop(divider(), 0.0),
                            gpui::linear_color_stop(gpui::transparent_black(), 1.0),
                        ))),
                )
                .children(lines)
                .into_any_element(),
        )
    }
}

impl Render for Inspector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.pending_clear) {
            self.add_component
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        let (ids, fields, playing) = {
            let session = self.session.read(cx);
            let ids = session.selection();
            let fields = session.inspect_all(&ids).unwrap_or_default();
            (ids, fields, session.is_playing())
        };
        if ids.is_empty() {
            self.showing.clear();
            self.slots.clear();
            return panel("Inspector").child(
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_2)
                    .px(SPACE_4)
                    .pt(SPACE_6)
                    .text_size(px(13.0))
                    .text_color(muted())
                    .child("Nothing selected")
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(mix(TEXT, 40))
                            .child("Click something in the Scene view or the Hierarchy."),
                    ),
            );
        }
        self.sync(&ids, &fields, window, cx);

        let shown = |f: &Field| !is_empty(&f.value) || self.revealed.contains(&f.name);
        let by = |names: &[&str]| -> Vec<&Field> {
            names
                .iter()
                .filter_map(|n| fields.iter().find(|f| f.name == *n))
                .filter(|f| shown(f))
                .collect()
        };
        let object: Vec<AnyElement> = by(&OBJECT).into_iter().map(|f| self.line(f, cx)).collect();
        let transform: Vec<AnyElement> = TRANSFORM
            .iter()
            .filter_map(|n| fields.iter().find(|f| f.name == *n))
            .map(|f| self.line(f, cx))
            .collect();
        let physics: Vec<AnyElement> = by(&PHYSICS).into_iter().map(|f| self.line(f, cx)).collect();
        let mut parts: Vec<AnyElement> = by(&PARTS).into_iter().map(|f| self.line(f, cx)).collect();
        parts.extend(
            fields
                .iter()
                .filter(|f| f.name.starts_with("components."))
                .map(|f| self.line(f, cx)),
        );
        let game: Vec<AnyElement> = fields
            .iter()
            .filter(|f| f.name.starts_with("game"))
            .map(|f| {
                div()
                    .flex()
                    .gap(SPACE_2)
                    .px(SPACE_4)
                    .py(px(2.0))
                    .text_size(px(11.5))
                    .child(
                        div()
                            .w(px(84.0))
                            .flex_none()
                            .text_color(label())
                            .child(title(f.name.strip_prefix("game.").unwrap_or(&f.name))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .font_family(MONO)
                            .text_color(muted())
                            .child(f.value.clone()),
                    )
                    .into_any_element()
            })
            .collect();

        // What could be added: the empty fields, as outlined chips.
        let addable: Vec<AnyElement> = OBJECT
            .iter()
            .chain(PHYSICS.iter())
            .chain(PARTS.iter())
            .filter(|n| **n != "prefab")
            .filter_map(|n| fields.iter().find(|f| f.name == *n))
            .filter(|f| !shown(f))
            .map(|f| {
                let name = f.name.clone();
                div()
                    .id(SharedString::from(format!("add-{name}")))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .h(px(22.0))
                    .px(SPACE_2)
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(divider())
                    .text_size(px(11.5))
                    .text_color(label())
                    .cursor_pointer()
                    .hover(|s| {
                        s.bg(hover())
                            .text_color(rgb(ACCENT))
                            .border_color(rgb(ACCENT))
                    })
                    .child(icon("plus", muted()))
                    .child(title(&name))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.revealed.insert(name.clone());
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();

        let header_name = self.input("name", None).cloned();
        let count = ids.len();
        let prefab = fields
            .iter()
            .find(|f| f.name == "prefab")
            .map(|f| f.value.clone())
            .filter(|p| !p.is_empty() && p != MIXED);

        panel("Inspector").child(
            div()
                .id("inspector-body")
                .flex()
                .flex_col()
                .flex_1()
                .overflow_y_scroll()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .px(SPACE_4)
                        .pb(SPACE_2)
                        .child(icon(if prefab.is_some() { "package" } else { "box" }, rgb(ACCENT)))
                        .children(header_name.map(|state| div().flex_1().child(Input::new(&state))))
                        .when(count > 1, |d| d.child(tag(format!("{count} selected"), ACCENT_800, ACCENT_100)))
                        .children(prefab.map(|p| tag(p, ACCENT_900, ACCENT_300))),
                )
                .when(playing, |d| {
                    d.child(
                        div()
                            .mx(SPACE_4)
                            .mb(SPACE_2)
                            .px(SPACE_3)
                            .py(SPACE_2)
                            .rounded(RADIUS_MD)
                            .border_1()
                            .border_color(mix(ACCENT, 40))
                            .text_size(px(11.5))
                            .text_color(rgb(ACCENT_300))
                            .child("Playing: this is where things are now. Stop brings the scene back."),
                    )
                })
                .children(self.section("Transform", transform))
                .children(self.section("Object", object))
                .children(self.section("Physics", physics))
                .children(self.section("Components", parts))
                .children(self.section("Running game", game))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_2)
                        .px(SPACE_4)
                        .pt(SPACE_2)
                        .pb(SPACE_6)
                        .child(div().flex().flex_wrap().gap(SPACE_1).children(addable))
                        .child(
                            Input::new(&self.add_component)
                                .small()
                                .prefix(icon("component", muted())),
                        ),
                ),
        )
    }
}
