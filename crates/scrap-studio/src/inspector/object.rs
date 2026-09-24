//! Unity's object field: a field that names an asset or an entity shows
//! it — its picture or icon, its name — and is set by picking or by
//! dragging, never by typing its name or its ID.
//!
//! A click opens a picker of what fits with a search (arrows and Enter,
//! Esc closes); a Project tile — or, for an entity, a Hierarchy line —
//! dragged onto the field sets it when it fits, and the field lights while
//! one that fits is over it; the arrow beside an asset's shows it in the
//! Project; the eyedropper beside an entity's picks one in the Scene view.
//! What a field names — a model, a texture, an entity — is the engine's
//! word: the shape of the field's type ([`scrap::shape::Shape`]). Every
//! change is one undo step through the session, on everything shown.

use std::collections::HashMap;

use scrap::EntityId;
use scrap_editor::console::Level;
use scrap_editor::Session;
use scrap_ui::{Event, ImageId, NodeId, Style, Ui};

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
    /// `animator`, `bone`.
    Field(String),
    /// A place in a field's value: a particle's model, a joint's `to`.
    Form(Place),
    /// A field of a game component.
    Sub(String, String),
}

/// How the value names what it names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Holds {
    /// A field of the line that is a name as it is: `model: "rock"`.
    Name,
    /// An asset link: `"rock"`, or `("rock", "fc55…")`.
    Link,
    /// A component's typed link: `PrefabLink(("rock", "fc55…"))`.
    Typed,
    /// An asset by its ID alone: a material's `base_map: "fc55…"`.
    AssetId,
    /// A component's link to an entity: `EntityRef("4f1c…")`.
    EntityRef,
    /// An entity by its bare id: a joint's `to: "4f1c…"`.
    EntityId,
}

/// An object field: where it writes, what it takes, and how the value
/// names it; `optional` for one inside an `Option`, which None clears.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectRef {
    pub slot: Slot,
    pub kind: String,
    pub holds: Holds,
    pub optional: bool,
}

/// What an object field is set to from the outside: a Project entry or a
/// Hierarchy line.
#[derive(Debug, Clone, PartialEq)]
pub enum Dragged {
    Asset(Asset),
    Entity(EntityId),
}

/// One entry of a picker: what it shows, what it writes, and a dim note
/// beside it (where an entity is).
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub label: String,
    pub key: String,
    pub note: String,
}

impl ObjectRef {
    pub fn new(slot: Slot, kind: &str, holds: Holds) -> Self {
        Self {
            slot,
            kind: kind.to_string(),
            holds,
            optional: false,
        }
    }

    /// Whether it can be left naming nothing: a material or a prefab
    /// cannot.
    fn clearable(&self) -> bool {
        !matches!(&self.slot, Slot::Field(f) if f == "material" || f == "prefab")
    }

    fn is_entity(&self) -> bool {
        matches!(self.holds, Holds::EntityRef | Holds::EntityId)
    }

    /// Whether what is dragged goes here.
    pub fn takes(&self, dragged: &Dragged) -> bool {
        match dragged {
            Dragged::Asset(asset) => !self.is_entity() && asset_kind(asset) == self.kind,
            Dragged::Entity(_) => self.is_entity(),
        }
    }

    /// The value's text for `key` — a name, an ID — or for nothing; in
    /// an option, `Some(…)` around it or `None`.
    fn text(&self, session: &Session, key: Option<&str>) -> String {
        let key = key.filter(|k| !k.is_empty());
        match (key, self.optional) {
            (None, true) => "None".into(),
            (Some(key), true) => format!("Some({})", self.bare(session, key)),
            (key, false) => self.bare(session, key.unwrap_or("")),
        }
    }

    fn bare(&self, session: &Session, key: &str) -> String {
        match self.holds {
            Holds::Name => key.to_string(),
            Holds::Link => {
                let link = session.link_to(&self.kind, key);
                scrap::ron::to_string(&link).unwrap_or_else(|_| tree::quote(key))
            }
            Holds::Typed => {
                let type_name = scrap::refs::LINK_KINDS
                    .iter()
                    .find(|(_, k)| *k == self.kind)
                    .map_or("ModelLink", |(n, _)| n);
                let inner = if key.is_empty() {
                    "\"\"".to_string()
                } else {
                    scrap::ron::to_string(&session.link_to(&self.kind, key)).unwrap_or_default()
                };
                format!("{type_name}({inner})")
            }
            Holds::AssetId => tree::quote(if key.is_empty() { "0" } else { key }),
            Holds::EntityRef => format!("{}({})", scrap::EntityRef::NAME, tree::quote(key)),
            Holds::EntityId => tree::quote(if key.is_empty() { "0" } else { key }),
        }
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

/// The icon a kind of thing is drawn with where it has no picture.
fn glyph(kind: &str) -> &'static str {
    match kind {
        "prefab" => "package",
        "sound" => "music",
        "scene" => "mountain",
        "texture" => "image",
        "shader" => "sparkles",
        "material" => "sparkles",
        "animator" => "route",
        "bone" => "move-3d",
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

/// The id inside a value's text — `"4f1c…"`, `EntityRef("4f1c…")`,
/// `Some("fc55…")` — when there is one.
fn id_text(text: &str) -> Option<String> {
    let node = tree::parse(text)?;
    match node.kind {
        tree::Kind::Text(t) => Some(t),
        tree::Kind::Tuple { items, .. } if items.len() == 1 => {
            id_text(&text[items[0].span.clone()])
        }
        _ => None,
    }
}

/// What an object field shows for a value: the name, whether it names
/// nothing, whether what it names is gone.
pub struct Shown {
    pub name: Option<String>,
    pub missing: bool,
}

impl Inspector {
    /// What `text` names, for an object field of `target`: its name as a
    /// person knows it, found by its ID for one that holds IDs.
    pub(super) fn shown(&self, session: &Session, target: &ObjectRef, text: &str) -> Shown {
        // `Some(…)`: what is inside.
        let inner = tree::parse(text).and_then(|n| match n.kind {
            tree::Kind::Tuple {
                name: Some(name),
                items,
            } if name == "Some" && items.len() == 1 => Some(items[0].span.clone()),
            _ => None,
        });
        let text = inner.map_or(text, |span| &text[span]);
        let found = |name: Option<String>, missing: bool| Shown { name, missing };
        match target.holds {
            Holds::Name | Holds::Link => {
                let name = if target.holds == Holds::Name {
                    Some(text.to_string())
                } else {
                    linked_name(text)
                };
                let missing = name.as_deref().is_some_and(|n| {
                    !n.is_empty()
                        && match target.kind.as_str() {
                            "material" => !session.palette().iter().any(|(p, _)| p == n),
                            "bone" => {
                                let bones = self
                                    .showing
                                    .first()
                                    .map(|id| session.bone_names(*id))
                                    .unwrap_or_default();
                                !bones.is_empty() && !bones.iter().any(|b| b == n)
                            }
                            kind => !session.link_exists(kind, &scrap::AssetLink::named(n)),
                        }
                });
                found(name, missing)
            }
            Holds::Typed => {
                let link = scrap::refs::links_in(text)
                    .into_iter()
                    .next()
                    .map(|(_, l)| l);
                let missing = link
                    .as_ref()
                    .is_some_and(|l| !l.is_empty() && !session.link_exists(&target.kind, l));
                found(link.map(|l| l.to_string()), missing)
            }
            Holds::AssetId => {
                let id = id_text(text).and_then(|t| t.parse::<scrap::asset::AssetId>().ok());
                match id.filter(|id| id.0 != 0) {
                    None => found(Some(String::new()), false),
                    Some(id) => match session.asset_name_of_id(&target.kind, id) {
                        Some(name) => found(Some(name), false),
                        None => found(Some(format!("missing {}", id.as_hex())), true),
                    },
                }
            }
            Holds::EntityRef | Holds::EntityId => {
                let id = id_text(text).and_then(|t| t.parse::<EntityId>().ok());
                match id.filter(|id| !id.is_unassigned()) {
                    None => found(Some(String::new()), false),
                    Some(id) => match session.entity_name(id) {
                        Some(name) => found(Some(name), false),
                        None => found(Some(format!("missing {id}")), true),
                    },
                }
            }
        }
    }

    /// A picture for an entry of a picker or a field, when there is one:
    /// the Project's for models, prefabs and materials; a texture's own,
    /// drawn small the first time it is asked for.
    fn picture_of(&mut self, session: &Session, kind: &str, name: &str) -> Option<ImageId> {
        if kind == "texture" {
            if let Some(image) = self.texture_pictures.get(name) {
                return Some(*image);
            }
            let pixels = session.texture_picture(name, 32).ok()?;
            let image = ImageId(TEXTURE_PICTURES + self.texture_pictures.len() as u32);
            self.images.push((image, 32, pixels));
            self.texture_pictures.insert(name.to_string(), image);
            return Some(image);
        }
        let key = picture_key(&as_asset(kind, name)?)?;
        self.pictures.get(&key).copied()
    }

    /// An object field into `line`: the thing's picture or icon and name,
    /// `None (Model)` when it names nothing, `—` when those shown disagree,
    /// red when what it names is gone; `Inline` for a material written out
    /// in full (`shown.name` `None`). Its ◎ opens the picker; an asset's
    /// arrow shows it in the Project, an entity's eyedropper picks one in
    /// the Scene view.
    pub(super) fn object_field(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        line: NodeId,
        name: &str,
        target: ObjectRef,
        shown: Shown,
        mixed: bool,
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
                .border(1.0, if shown.missing { ERROR } else { DIVIDER })
                .hover_border(TEXT.alpha(35))
                .clickable(),
        );
        ui.set_name(field, name);
        let named = shown.name.as_deref().filter(|v| !v.is_empty());
        let picture = match named {
            Some(n) if !mixed && !shown.missing => self.picture_of(session, &target.kind, n),
            _ => None,
        };
        match picture {
            Some(image) => {
                ui.add_image(field, Style::default().size(16.0, 16.0).radius(3.0), image);
            }
            None => {
                icon(
                    ui,
                    field,
                    glyph(&target.kind),
                    if named.is_some() { ACCENT } else { MUTED },
                );
            }
        }
        let (label, ink) = match named {
            _ if mixed => (super::MIXED.to_string(), TEXT),
            Some(n) => (n.strip_prefix("builtin:").unwrap_or(n).to_string(), TEXT),
            None if shown.name.is_some() => (format!("None ({})", title(&target.kind)), MUTED),
            None => ("Inline".to_string(), MUTED),
        };
        ui.add_text(
            field,
            text()
                .fill()
                .nowrap()
                .text_size(12.0)
                .text_color(if shown.missing { ERROR } else { ink }),
            &label,
        );
        let pick = self.little_button(ui, field, "circle-dot", LABEL);
        ui.set_name(pick, format!("{name} pick"));
        self.parts.insert(pick, Part::Object(target.clone()));
        if target.is_entity() {
            if !self.playing {
                let lit = self.picking.as_ref() == Some(&target);
                let eyedropper =
                    self.little_button(ui, line, "crosshair", if lit { ACCENT } else { MUTED });
                ui.set_name(eyedropper, format!("{name} in scene"));
                self.parts
                    .insert(eyedropper, Part::PickInScene(target.clone()));
            }
        } else if let (Some(n), false) = (named, mixed || shown.missing) {
            let ping = self.little_button(ui, line, "arrow-up-from-line", MUTED);
            ui.set_name(ping, format!("{name} ping"));
            self.parts
                .insert(ping, Part::Ping(target.kind.clone(), n.to_string()));
        }
        self.parts.insert(field, Part::Object(target.clone()));
        self.objects.push((field, target));
    }

    fn little_button(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        glyph: &str,
        ink: scrap_ui::Color,
    ) -> NodeId {
        let b = ui.add(
            parent,
            Style::row()
                .size(18.0, 20.0)
                .fixed()
                .center()
                .radius(RADIUS_SM)
                .hover(HOVER)
                .clickable(),
        );
        icon(ui, b, glyph, ink);
        b
    }

    /// What a picker for `target` offers.
    fn object_choices(&self, session: &Session, target: &ObjectRef) -> Vec<Choice> {
        let named = |names: Vec<String>| {
            names
                .into_iter()
                .map(|n| Choice {
                    label: n.strip_prefix("builtin:").unwrap_or(&n).to_string(),
                    key: n,
                    note: String::new(),
                })
                .collect()
        };
        match (target.holds, target.kind.as_str()) {
            (Holds::EntityRef | Holds::EntityId, _) => session
                .entity_choices()
                .into_iter()
                .filter(|c| !self.showing.contains(&c.id))
                .map(|c| Choice {
                    label: c.name,
                    key: c.id.to_string(),
                    note: c.path,
                })
                .collect(),
            (Holds::AssetId, kind) => session
                .asset_ids_of_kind(kind)
                .into_iter()
                .map(|(name, id)| Choice {
                    label: name,
                    key: id.as_hex(),
                    note: String::new(),
                })
                .collect(),
            (_, "bone") => named(
                self.showing
                    .first()
                    .map(|id| session.bone_names(*id))
                    .unwrap_or_default(),
            ),
            (_, "material") if !matches!(target.slot, Slot::Sub(..)) => {
                named(session.palette().into_iter().map(|(n, _)| n).collect())
            }
            (_, kind) => named(session.assets_of_kind(kind)),
        }
    }

    /// The picker under an object field: a search, what fits under it.
    pub(super) fn open_object_picker(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        target: ObjectRef,
        at: scrap_ui::Rect,
    ) {
        // Under the field, or over it when there is no room under it.
        let (_, h, _) = ui.viewport();
        let below = at.y + at.height + 4.0;
        let y = if below + 360.0 > h {
            (at.y - 364.0).max(8.0)
        } else {
            below
        };
        let (ground, card) = self.popover_card(ui, at.x, y, at.width.max(260.0));
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
        let choices = self.object_choices(session, &target);
        self.popover = Some(Popover::Object {
            root: ground,
            list,
            target,
            choices,
            hits: Vec::new(),
            at: 0,
        });
        self.fill_object_picker(ui, session, "");
    }

    /// The picker's list for what is typed, the chosen entry lit.
    fn fill_object_picker(&mut self, ui: &mut Ui, session: &Session, typed: &str) {
        let Some(Popover::Object {
            list,
            target,
            choices,
            hits,
            at,
            ..
        }) = &mut self.popover
        else {
            return;
        };
        let query = typed.trim().to_lowercase();
        let mut offered: Vec<Option<Choice>> = Vec::new();
        if target.clearable() && query.is_empty() {
            offered.push(None);
        }
        offered.extend(
            choices
                .iter()
                .filter(|c| {
                    query.is_empty()
                        || c.label.to_lowercase().contains(&query)
                        || c.note.to_lowercase().contains(&query)
                })
                .cloned()
                .map(Some),
        );
        *hits = offered.clone();
        *at = (*at).min(offered.len().saturating_sub(1));
        let (list, chosen, kind, target_holds) = (*list, *at, target.kind.clone(), target.holds);
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
                        scrap_ui::Color::TRANSPARENT
                    })
                    .hover(HOVER)
                    .clickable(),
            );
            let label = hit
                .as_ref()
                .map_or("None", |c| c.label.as_str())
                .to_string();
            ui.set_name(row, format!("object {label}"));
            // Pictures for what is near the top: a texture's is drawn when
            // first shown.
            let picture = match hit {
                // A picture is asked for by name: an ID's entry by its label.
                Some(c) if i < 40 => {
                    let by = if target_holds == Holds::AssetId {
                        &c.label
                    } else {
                        &c.key
                    };
                    self.picture_of(session, &kind, by)
                }
                _ => None,
            };
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
            if let Some(note) = hit.as_ref().map(|c| &c.note).filter(|n| !n.is_empty()) {
                ui.add_text(
                    row,
                    text().nowrap().fill().text_size(11.0).text_color(MUTED),
                    note,
                );
            }
            rows.push((row, i));
        }
        for (row, i) in rows {
            if i == chosen {
                ui.scroll_to(list, row);
            }
            self.popover_parts.insert(row, Part::PickEntry(i));
        }
    }

    /// Set what `target` names — by its key, a name or an ID; `None` for
    /// nothing — on everything shown, as one undo step.
    pub(super) fn assign(&mut self, session: &mut Session, target: &ObjectRef, key: Option<&str>) {
        let value = target.text(session, key);
        let done = match &target.slot {
            Slot::Field(field) => {
                let value = if field == "material" {
                    tree::quote(key.unwrap_or(""))
                } else {
                    value
                };
                session.set_field_all(&self.showing, field, &value)
            }
            Slot::Form(place) => {
                self.set_leaf(session, place, &value);
                Ok(())
            }
            Slot::Sub(component, key) => {
                let (component, key) = (component.clone(), key.clone());
                self.set_sub(session, &component, &key, &value);
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
                me.assign(session, &target, hit.as_ref().map(|c| c.key.as_str()));
                requests.refresh = true;
            }
        };
        match (part, event) {
            (Part::PickSearch, Event::Changed(typed)) => {
                if let Some(Popover::Object { at, .. }) = &mut self.popover {
                    *at = 0;
                }
                let typed = typed.clone();
                self.fill_object_picker(ui, session, &typed);
            }
            // Enter, not the keyboard leaving the search for a row.
            (Part::PickSearch, Event::Submit(_)) if ui.focused() == Some(node) => {
                pick(self, ui, session, chosen)
            }
            (Part::PickSearch, Event::Cancel) => self.close_popover(ui),
            (
                Part::PickSearch,
                Event::KeyDown(key @ (scrap::input::Key::Up | scrap::input::Key::Down)),
            ) => {
                if let Some(Popover::Object { at, hits, .. }) = &mut self.popover {
                    *at = if *key == scrap::input::Key::Up {
                        at.saturating_sub(1)
                    } else {
                        (*at + 1).min(hits.len().saturating_sub(1))
                    };
                }
                let typed = ui.text(node).unwrap_or_default().to_string();
                self.fill_object_picker(ui, session, &typed);
            }
            (Part::PickEntry(i), Event::Click { .. }) => pick(self, ui, session, *i),
            _ => {}
        }
    }

    /// Light the object field under the pointer while something that fits
    /// it is dragged over it; put the others back.
    pub fn hover_drop(&mut self, ui: &mut Ui, dragged: Option<&Dragged>) {
        let (x, y) = ui.pointer();
        let over = dragged.and_then(|d| {
            self.objects
                .iter()
                .find(|(node, target)| target.takes(d) && ui.rect(*node).contains(x, y))
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

    /// Something let go over the Inspector: onto an object field it fits,
    /// it is set there. `true` when it landed on an object field at all —
    /// one it does not fit takes nothing, and nothing changes.
    pub fn drop_on(&mut self, ui: &mut Ui, session: &mut Session, dragged: &Dragged) -> bool {
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
        if !target.takes(dragged) {
            let what = match dragged {
                Dragged::Asset(a) => format!("{} is a {}", a.label(), asset_kind(a)),
                Dragged::Entity(_) => "an entity".to_string(),
            };
            session.say(
                Level::Info,
                format!("{what}; this field takes a {}", target.kind),
            );
            return true;
        }
        let key = match dragged {
            Dragged::Asset(asset) => asset_name(asset),
            Dragged::Entity(id) => id.to_string(),
        };
        self.assign(session, &target, Some(&key));
        true
    }

    /// Whether the next click in the Scene view picks an entity for a
    /// field (its eyedropper is on).
    pub fn picking_in_scene(&self) -> bool {
        self.picking.is_some()
    }

    /// The Scene view was clicked while picking: what is under the pointer
    /// goes into the field — nothing there changes nothing — and picking
    /// stops.
    pub fn pick_in_scene(&mut self, session: &mut Session, hit: Option<EntityId>) {
        let Some(target) = self.picking.take() else {
            return;
        };
        if let Some(id) = hit.filter(|id| !self.showing.contains(id)) {
            self.assign(session, &target, Some(&id.to_string()));
        }
        self.built = false;
    }

    /// Stop picking in the Scene view (Escape).
    pub fn cancel_pick(&mut self) {
        if self.picking.take().is_some() {
            self.built = false;
        }
    }
}

/// Where a texture's own small picture's image ids start: past the
/// Project's and the picker's.
const TEXTURE_PICTURES: u32 = 40_000;

/// The pictures of textures drawn so far, by name.
pub type TexturePictures = HashMap<String, ImageId>;
