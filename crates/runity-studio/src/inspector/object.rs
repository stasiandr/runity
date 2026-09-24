//! Unity's object field: a field that names an asset shows the asset — its
//! picture, its name — and is set by picking or by dragging, never by
//! typing its name.
//!
//! A click opens a picker of that kind's assets with a search (arrows and
//! Enter, Esc closes); a Project tile dragged onto the field sets it when
//! the kind fits, and the field lights while one that fits is over it; the
//! arrow beside it shows the asset in the Project. What a field links to —
//! a model, a material, a sound — is the engine's word: the shape of the
//! field's type ([`runity::shape::Shape::Asset`]). Every change is one
//! undo step through the session, on everything shown.

use runity_editor::console::Level;
use runity_editor::Session;
use runity_ui::{Event, NodeId, Style, Ui};

use super::form::Place;
use super::tree;
use super::{title, Inspector, Part, Popover};
use crate::bottom::{picture_key, Asset};
use crate::studio::Requests;
use crate::theme::*;

/// Where an object field's value is written.
#[derive(Debug, Clone, PartialEq)]
pub enum Slot {
    /// A field of what is shown, by name: `model`, `material`, `prefab`,
    /// `animator`.
    Field(String),
    /// A place in a field's value: a particle's model, a sound's clip.
    Form(Place),
    /// A field of a game component: its typed link.
    Sub(String, String),
}

/// An object field: where it writes, and the kind of asset it takes.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectRef {
    pub slot: Slot,
    pub kind: String,
}

impl ObjectRef {
    /// Whether it can be left naming nothing: a material or a prefab
    /// cannot.
    fn clearable(&self) -> bool {
        !matches!(&self.slot, Slot::Field(f) if f == "material" || f == "prefab")
    }

    /// Whether what the Project calls `asset` goes here.
    pub fn takes(&self, asset: &Asset) -> bool {
        asset_kind(asset) == self.kind
    }
}

/// The kind of asset a Project entry is, as a link names it.
fn asset_kind(asset: &Asset) -> &'static str {
    match asset {
        Asset::Model(..) => "model",
        Asset::Material(_) => "material",
        Asset::Prefab(_) => "prefab",
        Asset::Sound(..) => "sound",
        Asset::Scene(_) => "scene",
    }
}

/// A Project entry's name, as a link writes it.
fn asset_name(asset: &Asset) -> String {
    match asset {
        Asset::Model(n, _) | Asset::Material(n) | Asset::Prefab(n) | Asset::Sound(n, _) => {
            n.clone()
        }
        Asset::Scene(_) => asset.label(),
    }
}

/// What a Project entry of `kind` called `name` is, as far as its picture
/// needs to know.
fn as_asset(kind: &str, name: &str) -> Option<Asset> {
    Some(match kind {
        "model" => Asset::Model(name.to_string(), None),
        "material" => Asset::Material(name.to_string()),
        "prefab" => Asset::Prefab(name.to_string()),
        _ => return None,
    })
}

/// The icon a kind of asset is drawn with where it has no picture.
fn glyph(kind: &str) -> &'static str {
    match kind {
        "prefab" => "package",
        "sound" => "music",
        "scene" => "mountain",
        "texture" => "image",
        "material" => "sparkles",
        "animator" => "route",
        _ => "box",
    }
}

/// The name a link's text names: `"rock"`, `("rock", "fc55…")`, or empty.
pub fn linked_name(text: &str) -> Option<String> {
    let node = tree::parse(text)?;
    let name = match node.kind {
        tree::Kind::Text(name) => name,
        tree::Kind::Tuple { name: None, items } => match items.first().map(|i| &i.kind) {
            Some(tree::Kind::Text(name)) => name.clone(),
            _ => return None,
        },
        tree::Kind::Struct { fields, .. } => match fields.iter().find(|(k, _)| k == "name") {
            Some((_, n)) => match &n.kind {
                tree::Kind::Text(name) => name.clone(),
                _ => return None,
            },
            None => return None,
        },
        _ => return None,
    };
    Some(name)
}

impl Inspector {
    /// An object field into `line`: the asset's picture (or its kind's
    /// icon) and name, `None (Model)` when it names nothing, `—` when those
    /// shown disagree; red when what it names is not there. The picker
    /// button and, when it names something, the arrow to the Project.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn object_field(
        &mut self,
        ui: &mut Ui,
        line: NodeId,
        name: &str,
        target: ObjectRef,
        value: Option<&str>,
        mixed: bool,
        missing: bool,
    ) {
        let field = ui.add(
            line,
            Style::row()
                .fill()
                .height(22.0)
                .padding_x(4.0)
                .gap(SPACE_2)
                .center_items()
                .radius(6.0)
                .background(BG)
                .border(1.0, if missing { ERROR } else { DIVIDER })
                .hover_border(TEXT.alpha(35))
                .clickable(),
        );
        ui.set_name(field, name);
        let named = value.filter(|v| !v.is_empty());
        let picture = named
            .and_then(|n| as_asset(&target.kind, n))
            .and_then(|a| picture_key(&a))
            .and_then(|key| self.pictures.get(&key).copied());
        match picture {
            Some(image) if !mixed => {
                ui.add_image(field, Style::default().size(16.0, 16.0).radius(3.0), image);
            }
            _ => {
                icon(
                    ui,
                    field,
                    glyph(&target.kind),
                    if named.is_some() { ACCENT } else { MUTED },
                );
            }
        }
        let (shown, ink) = match named {
            _ if mixed => (super::MIXED.to_string(), TEXT),
            Some(n) => (n.strip_prefix("builtin:").unwrap_or(n).to_string(), TEXT),
            None if value.is_some() => (format!("None ({})", title(&target.kind)), MUTED),
            // A material written out in full rather than named.
            None => ("Inline".to_string(), MUTED),
        };
        ui.add_text(
            field,
            text()
                .fill()
                .nowrap()
                .text_size(12.0)
                .text_color(if missing { ERROR } else { ink }),
            &shown,
        );
        let pick = ui.add(
            field,
            Style::row()
                .size(18.0, 18.0)
                .fixed()
                .center()
                .radius(RADIUS_SM)
                .hover(HOVER)
                .clickable(),
        );
        ui.set_name(pick, format!("{name} pick"));
        icon(ui, pick, "circle-dot", LABEL);
        self.parts.insert(pick, Part::Object(target.clone()));
        if named.is_some() && !mixed {
            let ping = ui.add(
                line,
                Style::row()
                    .size(18.0, 22.0)
                    .fixed()
                    .center()
                    .radius(6.0)
                    .hover(HOVER)
                    .clickable(),
            );
            ui.set_name(ping, format!("{name} ping"));
            icon(ui, ping, "arrow-up-from-line", MUTED);
            self.parts.insert(
                ping,
                Part::Ping(target.kind.clone(), named.unwrap().to_string()),
            );
        }
        self.parts.insert(field, Part::Object(target.clone()));
        self.objects.push((field, target));
    }

    /// What a picker of `target`'s kind offers: every asset of it, by
    /// name. A material field takes the engine's builtins as well as the
    /// project's.
    fn object_choices(&self, session: &Session, target: &ObjectRef) -> Vec<String> {
        match (&target.kind[..], &target.slot) {
            ("material", Slot::Field(_) | Slot::Form(_)) => {
                session.palette().into_iter().map(|(n, _)| n).collect()
            }
            (kind, _) => session.assets_of_kind(kind),
        }
    }

    /// The picker under an object field: a search, the kind's assets
    /// under it.
    pub(super) fn open_object_picker(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        target: ObjectRef,
        at: runity_ui::Rect,
    ) {
        let (ground, card) =
            self.popover_card(ui, at.x, at.y + at.height + 4.0, at.width.max(240.0));
        ui.set_name(card, "object picker");
        let head = ui.add(card, Style::row().full_width().gap(SPACE_2).center_items());
        icon(ui, head, glyph(&target.kind), ACCENT);
        ui.add_text(
            head,
            text().fill().text_size(12.0).text_color(LABEL),
            &format!("Select {}", title(&target.kind)),
        );
        let search = ui.add_field(card, field_style().full_width().height(26.0), "");
        ui.set_placeholder(search, "Search");
        ui.set_name(search, "object search");
        self.popover_parts.insert(search, Part::PickSearch);
        let list = ui.add(
            card,
            Style::column()
                .full_width()
                .max_height(300.0)
                .gap(1.0)
                .clip(),
        );
        ui.focus(Some(search));
        let names = self.object_choices(session, &target);
        self.popover = Some(Popover::Object {
            root: ground,
            list,
            target,
            names,
            hits: Vec::new(),
            at: 0,
        });
        self.fill_object_picker(ui, "");
    }

    /// The picker's list for what is typed, the chosen entry lit.
    fn fill_object_picker(&mut self, ui: &mut Ui, typed: &str) {
        let Some(Popover::Object {
            list,
            target,
            names,
            hits,
            at,
            ..
        }) = &mut self.popover
        else {
            return;
        };
        let query = typed.trim().to_lowercase();
        let mut offered: Vec<Option<String>> = Vec::new();
        if target.clearable() && query.is_empty() {
            offered.push(None);
        }
        offered.extend(
            names
                .iter()
                .filter(|n| query.is_empty() || n.to_lowercase().contains(&query))
                .cloned()
                .map(Some),
        );
        *hits = offered.clone();
        *at = (*at).min(offered.len().saturating_sub(1));
        let (list, chosen, kind) = (*list, *at, target.kind.clone());
        ui.clear(list);
        let mut rows = Vec::new();
        for (i, hit) in offered.iter().enumerate() {
            let row = ui.add(
                list,
                Style::row()
                    .full_width()
                    .height(24.0)
                    .fixed()
                    .padding_x(SPACE_2)
                    .gap(SPACE_2)
                    .center_items()
                    .radius(RADIUS_SM)
                    .background(if i == chosen {
                        ACCENT_900
                    } else {
                        runity_ui::Color::TRANSPARENT
                    })
                    .hover(HOVER)
                    .clickable(),
            );
            let label = match hit {
                Some(n) => n.strip_prefix("builtin:").unwrap_or(n).to_string(),
                None => "None".to_string(),
            };
            ui.set_name(row, format!("object {label}"));
            let picture = hit
                .as_deref()
                .and_then(|n| as_asset(&kind, n))
                .and_then(|a| picture_key(&a))
                .and_then(|key| self.pictures.get(&key).copied());
            match picture {
                Some(image) => {
                    ui.add_image(row, Style::default().size(18.0, 18.0).radius(3.0), image);
                }
                None => {
                    icon(
                        ui,
                        row,
                        glyph(&kind),
                        if hit.is_some() { MUTED } else { NEUTRAL_500 },
                    );
                }
            }
            ui.add_text(row, text().nowrap().text_size(12.0), &label);
            rows.push((row, i));
        }
        for (row, i) in rows {
            if i == chosen {
                ui.scroll_to(list, row);
            }
            self.popover_parts.insert(row, Part::PickEntry(i));
        }
    }

    /// Set what `target` names — `None` for nothing — on everything shown,
    /// as one undo step.
    pub(super) fn assign(&mut self, session: &mut Session, target: &ObjectRef, name: Option<&str>) {
        let name = name.unwrap_or("");
        let done = match &target.slot {
            Slot::Field(field) => {
                let value = if field == "material" {
                    tree::quote(name)
                } else {
                    name.to_string()
                };
                session.set_field_all(&self.showing, field, &value)
            }
            Slot::Form(place) => {
                let link = session.link_to(&target.kind, name);
                let value = runity::ron::to_string(&link).unwrap_or_else(|_| tree::quote(name));
                self.set_leaf(session, place, &value);
                Ok(())
            }
            Slot::Sub(component, key) => {
                let type_name = runity::refs::LINK_KINDS
                    .iter()
                    .find(|(_, k)| *k == target.kind)
                    .map_or("ModelLink", |(n, _)| n);
                let inner = if name.is_empty() {
                    "\"\"".to_string()
                } else {
                    runity::ron::to_string(&session.link_to(&target.kind, name)).unwrap_or_default()
                };
                let (component, key) = (component.clone(), key.clone());
                self.set_sub(session, &component, &key, &format!("{type_name}({inner})"));
                Ok(())
            }
        };
        if let Err(e) = done {
            session.say(Level::Error, e.to_string());
        }
        self.built = false;
    }

    /// The picker's search and list, used.
    pub(super) fn object_picker_event(
        &mut self,
        ui: &mut Ui,
        session: &mut Session,
        node: NodeId,
        part: &Part,
        event: &Event,
        requests: &mut Requests,
    ) {
        let Some(Popover::Object {
            target, hits, at, ..
        }) = &self.popover
        else {
            return;
        };
        let (target, hits, chosen) = (target.clone(), hits.clone(), *at);
        let mut pick = |me: &mut Self, ui: &mut Ui, session: &mut Session, i: usize| {
            if let Some(hit) = hits.get(i) {
                me.close_popover(ui);
                me.assign(session, &target, hit.as_deref());
                requests.refresh = true;
            }
        };
        match (part, event) {
            (Part::PickSearch, Event::Changed(typed)) => {
                if let Some(Popover::Object { at, .. }) = &mut self.popover {
                    *at = 0;
                }
                let typed = typed.clone();
                self.fill_object_picker(ui, &typed);
            }
            // Enter, not the keyboard leaving the search for a row.
            (Part::PickSearch, Event::Submit(_)) if ui.focused() == Some(node) => {
                pick(self, ui, session, chosen)
            }
            (Part::PickSearch, Event::Cancel) => self.close_popover(ui),
            (
                Part::PickSearch,
                Event::KeyDown(key @ (runity::input::Key::Up | runity::input::Key::Down)),
            ) => {
                if let Some(Popover::Object { at, hits, .. }) = &mut self.popover {
                    *at = if *key == runity::input::Key::Up {
                        at.saturating_sub(1)
                    } else {
                        (*at + 1).min(hits.len().saturating_sub(1))
                    };
                }
                let typed = ui.text(node).unwrap_or_default().to_string();
                self.fill_object_picker(ui, &typed);
            }
            (Part::PickEntry(i), Event::Click { .. }) => pick(self, ui, session, *i),
            _ => {}
        }
    }

    /// Light the object field under the pointer while a Project entry that
    /// fits it is dragged over it; put the others back.
    pub fn hover_drop(&mut self, ui: &mut Ui, dragged: Option<&Asset>) {
        let (x, y) = ui.pointer();
        let over = dragged.and_then(|asset| {
            self.objects
                .iter()
                .find(|(node, target)| target.takes(asset) && ui.rect(*node).contains(x, y))
                .map(|(node, _)| *node)
        });
        if over == self.drop_lit {
            return;
        }
        if let Some(old) = self.drop_lit.take() {
            if ui.exists(old) {
                ui.restyle(old, |s| s.border(1.0, DIVIDER).background(BG));
            }
        }
        if let Some(node) = over {
            ui.restyle(node, |s| s.border(1.0, ACCENT).background(ACCENT_900));
            self.drop_lit = Some(node);
        }
    }

    /// A Project entry let go over the Inspector: onto an object field it
    /// fits, it is set there. `true` when it landed on an object field at
    /// all — one of another kind takes nothing, and nothing changes.
    pub fn drop_asset(&mut self, ui: &mut Ui, session: &mut Session, asset: &Asset) -> bool {
        let (x, y) = ui.pointer();
        let Some((_, target)) = self
            .objects
            .iter()
            .find(|(node, _)| ui.rect(*node).contains(x, y))
            .cloned()
        else {
            return false;
        };
        self.hover_drop(ui, None);
        if !target.takes(asset) {
            session.say(
                Level::Info,
                format!(
                    "{} is a {}; this field takes a {}",
                    asset.label(),
                    asset_kind(asset),
                    target.kind
                ),
            );
            return true;
        }
        self.assign(session, &target, Some(&asset_name(asset)));
        true
    }
}
