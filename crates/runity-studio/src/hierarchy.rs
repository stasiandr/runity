//! The Hierarchy: the document's lines, as a tree to click.
//!
//! What a line *is* — depth, open, selected, hidden, an instance of a
//! prefab — is `Session::hierarchy`, tested without a window. This keeps a
//! node per line, matched to the lines by entity id so that a change to
//! one line changes one node, and turns what happens to the nodes into the
//! calls an agent makes: a click selects (Shift or Cmd adds), a double
//! click frames it, the arrow opens it, the eye hides it, the lock keeps
//! the Scene view's clicks off it, a line dropped on another becomes its
//! child, a right click opens the context menu, F2 renames.

use std::collections::HashMap;

use runity::EntityId;
use runity_editor::panels::Row;
use runity_editor::Session;
use runity_ui::{Event, NodeId, Style, Ui};

use crate::studio::Requests;
use crate::theme::*;

/// What a node of the panel stands for.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Part {
    Line(EntityId),
    Arrow(EntityId, bool),
    Eye(EntityId, bool),
    Lock(EntityId, bool),
    Rename(EntityId),
}

const INDENT: f32 = 14.0;
const LINE: f32 = 24.0;

pub struct Hierarchy {
    pub card: NodeId,
    count: NodeId,
    search: NodeId,
    list: NodeId,
    parts: HashMap<NodeId, Part>,
    /// A line being renamed: its field, in place of its label.
    renaming: Option<(EntityId, NodeId)>,
    /// The first line in view the last time a line was selected, so the
    /// view can follow a selection made in the Scene view.
    followed: Option<EntityId>,
}

impl Hierarchy {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let (card, header, body) = panel(ui, parent, "Hierarchy");
        ui.set_name(card, "hierarchy");
        let count = ui.add_text(header, caption(), "");
        let bar = ui.add(
            body,
            Style::row()
                .full_width()
                .padding_x(SPACE_2)
                .height(28.0)
                .fixed()
                .center_items(),
        );
        let search = ui.add_field(bar, field_style().fill().height(24.0), "");
        ui.set_name(search, "hierarchy search");
        ui.set_placeholder(search, "Search  (c:door  m:bark)");
        let list = ui.add(
            body,
            Style::column()
                .fill()
                .full_width()
                .padding_x(SPACE_2)
                .padding_y(SPACE_1)
                .clip()
                .clickable(),
        );
        ui.set_name(list, "hierarchy list");
        Self {
            card,
            count,
            search,
            list,
            parts: HashMap::new(),
            renaming: None,
            followed: None,
        }
    }

    fn rows(&self, ui: &Ui, session: &Session) -> Vec<Row> {
        let query = ui.text(self.search).unwrap_or_default().trim().to_string();
        if query.is_empty() {
            session.hierarchy()
        } else {
            session.hierarchy_matching(&query).unwrap_or_default()
        }
    }

    /// Make the lines match the document.
    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        ui.set_text(self.count, &session.entity_count().to_string());
        let rows = self.rows(ui, session);
        let keys: Vec<u64> = rows.iter().map(|r| r.id.raw()).collect();
        let by_key: HashMap<u64, &Row> = rows.iter().map(|r| (r.id.raw(), r)).collect();
        let renaming = self.renaming;
        let mut parts = HashMap::new();
        ui.sync_children(
            self.list,
            &keys,
            |ui, list, key| make_line(ui, list, by_key[key]),
            |_, _, _| {},
        );
        // Every line brought up to date, and what its nodes stand for.
        for (line, row) in ui.children(self.list).into_iter().zip(&rows) {
            update_line(ui, line, row, renaming);
            let kids = ui.children(line);
            parts.insert(line, Part::Line(row.id));
            parts.insert(kids[0], Part::Arrow(row.id, row.open));
            let tools = kids[4];
            let t = ui.children(tools);
            parts.insert(t[0], Part::Eye(row.id, row.hidden));
            parts.insert(t[1], Part::Lock(row.id, row.locked));
            if let Some((id, field)) = renaming {
                if id == row.id {
                    parts.insert(field, Part::Rename(id));
                }
            }
        }
        self.parts = parts;
        // Follow the selection into view when it changed from elsewhere.
        let selected = session.selected();
        if selected != self.followed {
            self.followed = selected;
            if let Some(line) = ui
                .children(self.list)
                .into_iter()
                .zip(&rows)
                .find(|(_, r)| Some(r.id) == selected)
                .map(|(n, _)| n)
            {
                ui.scroll_to(self.list, line);
            }
        }
    }

    /// Start renaming a line: its label becomes a field.
    pub fn rename(&mut self, ui: &mut Ui, session: &Session, id: EntityId) {
        let Some(line) = self.line_of(id) else {
            return;
        };
        let label = ui.children(line)[2];
        ui.restyle(label, |s| s.hidden());
        let name = session.entity_name(id).unwrap_or_default();
        let field = ui.add_field(line, field_style().fill().height(20.0), &name);
        ui.set_name(field, "rename");
        ui.focus(Some(field));
        self.renaming = Some((id, field));
        self.parts.insert(field, Part::Rename(id));
    }

    fn stop_renaming(&mut self, ui: &mut Ui) {
        if let Some((_, field)) = self.renaming.take() {
            ui.remove(field);
        }
    }

    fn line_of(&self, id: EntityId) -> Option<NodeId> {
        self.parts
            .iter()
            .find(|(_, p)| **p == Part::Line(id))
            .map(|(n, _)| *n)
    }

    /// Whether `node` is this panel's.
    pub fn owns(&self, ui: &Ui, node: NodeId) -> bool {
        node == self.search || node == self.list || self.parts.contains_key(&node) || {
            let mut at = ui.parent(node);
            while let Some(p) = at {
                if p == self.card {
                    return true;
                }
                at = ui.parent(p);
            }
            false
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
        if node == self.search {
            if matches!(event, Event::Changed(_) | Event::Cancel) {
                requests.refresh = true;
            }
            return;
        }
        let part = self.parts.get(&node).copied();
        let (shift, ctrl, _, command) = ui.modifiers();
        match (part, event) {
            (Some(Part::Rename(id)), Event::Submit(name)) => {
                if let Err(e) = session.rename(id, name.trim()) {
                    session.say(runity_editor::console::Level::Error, e.to_string());
                }
                self.stop_renaming(ui);
                requests.refresh = true;
            }
            (Some(Part::Rename(_)), Event::Cancel | Event::Blur) => {
                self.stop_renaming(ui);
                requests.refresh = true;
            }
            (Some(Part::Arrow(id, open)), Event::Click { .. }) => {
                session.set_open(id, !open);
                requests.refresh = true;
            }
            (Some(Part::Eye(id, hidden)), Event::Click { .. }) => {
                let _ = session.set_hidden(&[id], !hidden);
                requests.refresh = true;
            }
            (Some(Part::Lock(id, locked)), Event::Click { .. }) => {
                let _ = session.set_pickable(&[id], locked);
                requests.refresh = true;
            }
            (Some(Part::Line(id)), Event::Click { button, count }) => {
                use runity::input::MouseButton;
                match button {
                    MouseButton::Right => {
                        if !session.selection().contains(&id) {
                            let _ = session.select(Some(id));
                        }
                        let (x, y) = ui.pointer();
                        requests.menu = Some((crate::menu::context_menu(), x, y));
                    }
                    _ if *count >= 2 => {
                        session.focus_selected();
                    }
                    _ if shift || ctrl || command => {
                        let _ = session.add_to_selection(id);
                    }
                    _ => {
                        let _ = session.select(Some(id));
                    }
                }
                self.followed = Some(id);
                requests.refresh = true;
                requests.keyboard_to_scene = true;
            }
            (Some(Part::Line(id)), Event::DragEnd { over }) => {
                let target = over.and_then(|o| match self.parts.get(&o) {
                    Some(Part::Line(t)) => Some(Some(*t)),
                    _ if o == self.list => Some(None),
                    _ => None,
                });
                if let Some(parent) = target {
                    if parent != Some(id) {
                        if let Err(e) = session.reparent(id, parent) {
                            session.say(runity_editor::console::Level::Warning, e.to_string());
                        }
                        if let Some(p) = parent {
                            session.set_open(p, true);
                        }
                    }
                }
                requests.refresh = true;
            }
            (None, Event::Click { button, .. }) if node == self.list => {
                if *button == runity::input::MouseButton::Right {
                    let (x, y) = ui.pointer();
                    requests.menu = Some((crate::menu::create_menu(), x, y));
                } else {
                    let _ = session.select(None);
                    requests.refresh = true;
                }
            }
            _ => {}
        }
    }

    /// F2 on the selection, from the Edit menu or the keyboard.
    pub fn rename_selected(&mut self, ui: &mut Ui, session: &Session) {
        if let Some(id) = session.selected() {
            self.rename(ui, session, id);
        }
    }
}

fn make_line(ui: &mut Ui, list: NodeId, row: &Row) -> NodeId {
    let line = ui.add(
        list,
        Style::row()
            .height(LINE)
            .fixed()
            .full_width()
            .gap(SPACE_1)
            .center_items()
            .radius(RADIUS_SM)
            .border(1.0, runity_ui::Color::TRANSPARENT)
            .draggable()
            .clickable(),
    );
    ui.set_name(line, format!("line {}", row.name));
    // 0: the arrow
    let arrow = ui.add(
        line,
        Style::row()
            .size(16.0, 16.0)
            .fixed()
            .center()
            .radius(RADIUS_SM)
            .hover(HOVER),
    );
    icon(ui, arrow, "chevron-right", MUTED);
    // 1: what it is
    icon(ui, line, "box", MUTED);
    // 2: its name
    ui.add_text(line, text().fill(), &row.name);
    // 3: the prefab tag's slot
    ui.add(line, Style::row().fixed());
    // 4: eye and lock, shown on hover or when set
    let tools = ui.add(line, Style::row().fixed().center_items());
    for glyph in ["eye", "lock-open"] {
        let b = ui.add(
            tools,
            Style::row()
                .size(20.0, 20.0)
                .center()
                .radius(RADIUS_SM)
                .hover(HOVER),
        );
        icon(ui, b, glyph, TEXT.alpha(30));
    }
    line
}

fn update_line(ui: &mut Ui, line: NodeId, row: &Row, renaming: Option<(EntityId, NodeId)>) {
    if ui.name(line) != Some(&format!("line {}", row.name)) {
        ui.set_name(line, format!("line {}", row.name));
    }
    let mut style = ui
        .style(line)
        .clone()
        .padding_left(4.0 + row.depth as f32 * INDENT)
        .padding_x(0.0)
        .padding_left(4.0 + row.depth as f32 * INDENT);
    style = if row.selected {
        style.background(ACCENT_900).border(1.0, ACCENT.alpha(40))
    } else {
        style
            .background(runity_ui::Color::TRANSPARENT)
            .border(1.0, runity_ui::Color::TRANSPARENT)
    };
    style.look.hover_background = if row.selected {
        None
    } else {
        Some(TEXT.alpha(5))
    };
    ui.set_style(line, style);
    let kids = ui.children(line);
    // The arrow: only for lines with something under them.
    ui.restyle(kids[0], |s| {
        s.opacity(if row.has_children { 1.0 } else { 0.0 })
    });
    if let Some(glyph) = ui.children(kids[0]).first().copied() {
        ui.set_icon(
            glyph,
            if row.open {
                "chevron-down"
            } else {
                "chevron-right"
            },
        );
    }
    // What it is.
    let (kind, tint) = if row.prefab.is_some() {
        ("package", ACCENT)
    } else if row.has_children {
        ("boxes", MUTED)
    } else {
        ("box", MUTED)
    };
    ui.set_icon(kids[1], kind);
    ui.restyle(kids[1], |s| s.text_color(tint));
    // The name.
    let ink = if row.selected {
        ACCENT_200
    } else if row.hidden {
        MUTED
    } else if row.part {
        LABEL
    } else {
        TEXT
    };
    let being_renamed = renaming.is_some_and(|(id, _)| id == row.id);
    ui.set_text(
        kids[2],
        if row.name.is_empty() {
            "(unnamed)"
        } else {
            &row.name
        },
    );
    ui.restyle(kids[2], |s| {
        let s = s.text_color(ink);
        if being_renamed {
            s.hidden()
        } else {
            s.shown()
        }
    });
    // The prefab's name as a tag.
    let slot = kids[3];
    let has_tag = !ui.children(slot).is_empty();
    match (&row.prefab, has_tag) {
        (Some(p), false) => {
            tag(ui, slot, p, ACCENT_900, ACCENT_300);
        }
        (None, true) => ui.clear(slot),
        _ => {}
    }
    // Eye and lock: faint unless set.
    let tools = ui.children(kids[4]);
    let eye = ui.children(tools[0])[0];
    let lock = ui.children(tools[1])[0];
    ui.set_icon(eye, if row.hidden { "eye-off" } else { "eye" });
    ui.restyle(eye, |s| {
        s.text_color(if row.hidden { LABEL } else { TEXT.alpha(22) })
    });
    ui.set_icon(lock, if row.locked { "lock" } else { "lock-open" });
    ui.restyle(lock, |s| {
        s.text_color(if row.locked { LABEL } else { TEXT.alpha(22) })
    });
}
