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
    Arrow(EntityId),
    Eye(EntityId),
    Lock(EntityId),
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
    /// The lines as last shown, in order.
    shown: Vec<Row>,
    /// The arrow keys move the selection here.
    active: bool,
    /// Each line's node as last updated.
    lines: HashMap<EntityId, NodeId>,
    /// Where a Shift click selects from.
    anchor: Option<EntityId>,
    /// The accent line that shows where a dragged line will land.
    indicator: NodeId,
}

/// Where a dragged line lands, relative to the line under the pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drop {
    Before(EntityId),
    Into(EntityId),
    After(EntityId),
    /// Below every line: the end of the top level.
    End,
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
        let indicator = ui.add(
            card,
            Style::row()
                .absolute(0.0, 0.0)
                .size(0.0, 2.0)
                .radius(1.0)
                .background(ACCENT)
                .hidden(),
        );
        Self {
            shown: Vec::new(),
            active: false,
            lines: HashMap::new(),
            anchor: None,
            indicator,
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
        // Lines made for new rows say what their nodes stand for once, when
        // they are made; a line kept keeps its parts.
        let mut made: Vec<(EntityId, NodeId)> = Vec::new();
        ui.sync_children(
            self.list,
            &keys,
            |ui, list, key| {
                let row = by_key[key];
                let line = make_line(ui, list, row);
                made.push((row.id, line));
                line
            },
            |_, _, _| {},
        );
        for (id, line) in made {
            let kids = ui.children(line);
            let tools = ui.children(kids[4]);
            self.parts.insert(line, Part::Line(id));
            self.parts.insert(kids[0], Part::Arrow(id));
            self.parts.insert(tools[0], Part::Eye(id));
            self.parts.insert(tools[1], Part::Lock(id));
            self.lines.insert(id, line);
        }
        // Lines whose rows went: forget their parts.
        let alive: std::collections::HashSet<EntityId> = rows.iter().map(|r| r.id).collect();
        if self.lines.len() > alive.len() {
            let gone: Vec<EntityId> = self
                .lines
                .keys()
                .filter(|id| !alive.contains(id))
                .copied()
                .collect();
            for id in gone {
                self.lines.remove(&id);
            }
            self.parts.retain(|node, _| ui.exists(*node));
        }
        // Every line brought up to date — only those whose row changed: a
        // selection change touches two lines, not two thousand.
        let before: HashMap<EntityId, &Row> = self.shown.iter().map(|r| (r.id, r)).collect();
        for row in &rows {
            let Some(&line) = self.lines.get(&row.id) else {
                continue;
            };
            let same = before.get(&row.id).is_some_and(|b| *b == row) && renaming.is_none();
            if !same {
                update_line(ui, line, row, renaming);
            }
        }
        if let Some((id, field)) = renaming {
            self.parts.insert(field, Part::Rename(id));
        }
        self.shown = rows.clone();

        // Follow the selection into view when it changed from elsewhere.
        let selected = session.selected();
        if selected != self.followed {
            self.followed = selected;
            if let Some(line) = selected.and_then(|id| self.lines.get(&id).copied()) {
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
        self.lines.get(&id).copied()
    }

    fn row(&self, id: EntityId) -> Option<&Row> {
        self.shown.iter().find(|r| r.id == id)
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
            (Some(Part::Arrow(id)), Event::Click { .. }) => {
                let open = self.row(id).is_some_and(|r| r.open);
                session.set_open(id, !open);
                requests.refresh = true;
            }
            (Some(Part::Eye(id)), Event::Click { .. }) => {
                let hidden = self.row(id).is_some_and(|r| r.hidden);
                let _ = session.set_hidden(&[id], !hidden);
                requests.refresh = true;
            }
            (Some(Part::Lock(id)), Event::Click { .. }) => {
                let locked = self.row(id).is_some_and(|r| r.locked);
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
                    _ if shift => self.select_range(session, id),
                    _ if ctrl || command => {
                        // Cmd toggles one line in or out.
                        let mut ids = session.selection();
                        if let Some(i) = ids.iter().position(|x| *x == id) {
                            ids.remove(i);
                        } else {
                            ids.push(id);
                        }
                        let _ = session.select(None);
                        for x in ids {
                            let _ = session.add_to_selection(x);
                        }
                        self.anchor = Some(id);
                    }
                    _ => {
                        let _ = session.select(Some(id));
                        self.anchor = Some(id);
                    }
                }
                self.active = true;
                self.followed = Some(id);
                requests.refresh = true;
                requests.keyboard_to_scene = true;
            }
            (Some(Part::Line(_)), Event::Drag { .. }) => {
                self.show_drop(ui);
            }
            (Some(Part::Line(id)), Event::DragEnd { .. }) => {
                ui.restyle(self.indicator, |s| s.hidden());
                if let Some(drop) = self.drop_at(ui) {
                    self.land(session, id, drop);
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

    /// Where something from elsewhere — a Project entry — let go at the
    /// pointer lands: `Some(Some(line))` on a line, `Some(None)` on the list
    /// past its lines, `None` off the Hierarchy.
    pub fn drop_target(&self, ui: &Ui) -> Option<Option<EntityId>> {
        match self.drop_at(ui)? {
            Drop::Into(id) | Drop::Before(id) | Drop::After(id) => Some(Some(id)),
            Drop::End => Some(None),
        }
    }

    /// Where the pointer is over the list, as a drop.
    fn drop_at(&self, ui: &Ui) -> Option<Drop> {
        let (x, y) = ui.pointer();
        if !ui.rect(self.list).contains(x, y) {
            return None;
        }
        for (line, row) in ui.children(self.list).into_iter().zip(&self.shown) {
            let r = ui.rect(line);
            if y >= r.y && y < r.y + r.height {
                let t = (y - r.y) / r.height;
                return Some(if t < 0.25 {
                    Drop::Before(row.id)
                } else if t > 0.75 && !(row.has_children && row.open) {
                    Drop::After(row.id)
                } else {
                    Drop::Into(row.id)
                });
            }
        }
        Some(Drop::End)
    }

    /// The accent line (or the outlined line) where a drop would land.
    fn show_drop(&mut self, ui: &mut Ui) {
        let Some(drop) = self.drop_at(ui) else {
            ui.restyle(self.indicator, |s| s.hidden());
            return;
        };
        let card = ui.rect(self.card);
        let line_of = |id: EntityId| {
            ui.children(self.list)
                .into_iter()
                .zip(&self.shown)
                .find(|(_, r)| r.id == id)
                .map(|(n, r)| (ui.rect(n), r.depth))
        };
        let (y, x, w, h) = match drop {
            Drop::Before(id) | Drop::After(id) | Drop::Into(id) => {
                let Some((r, depth)) = line_of(id) else {
                    return;
                };
                let indent = 22.0 + depth as f32 * INDENT;
                match drop {
                    Drop::Before(_) => (r.y - 1.0, r.x + indent, r.width - indent, 2.0),
                    Drop::After(_) => (r.y + r.height - 1.0, r.x + indent, r.width - indent, 2.0),
                    _ => (r.y, r.x, r.width, r.height),
                }
            }
            Drop::End => {
                let last = ui.children(self.list).last().map(|n| ui.rect(*n));
                let Some(r) = last else { return };
                (r.y + r.height, r.x, r.width, 2.0)
            }
        };
        let into = matches!(drop, Drop::Into(_));
        ui.restyle(self.indicator, |s| {
            let s = s.shown().absolute(x - card.x, y - card.y).size(w, h);
            if into {
                s.background(ACCENT.alpha(12)).border(1.0, ACCENT)
            } else {
                s.background(ACCENT)
                    .border(0.0, runity_ui::Color::TRANSPARENT)
            }
        });
    }

    /// Put `id` where it was dropped: before or after a line (a sibling
    /// of it), into a line (its last child), or at the end of the top.
    fn land(&mut self, session: &mut Session, id: EntityId, drop: Drop) {
        let parent_of = |target: EntityId| -> Option<EntityId> {
            let i = self.shown.iter().position(|r| r.id == target)?;
            let depth = self.shown[i].depth;
            self.shown[..i]
                .iter()
                .rev()
                .find(|r| r.depth + 1 == depth)
                .map(|r| r.id)
        };
        let index_of = |target: EntityId| -> usize {
            let Some(i) = self.shown.iter().position(|r| r.id == target) else {
                return 0;
            };
            let depth = self.shown[i].depth;
            let mut n = 0;
            for r in self.shown[..i].iter().rev() {
                if r.depth < depth {
                    break;
                }
                if r.depth == depth && r.id != id {
                    n += 1;
                }
            }
            n
        };
        let (parent, index) = match drop {
            Drop::Into(t) if t == id => return,
            Drop::Into(t) => (Some(t), None),
            Drop::Before(t) if t == id => return,
            Drop::After(t) if t == id => return,
            Drop::Before(t) => (parent_of(t), Some(index_of(t))),
            Drop::After(t) => (parent_of(t), Some(index_of(t) + 1)),
            Drop::End => (None, None),
        };
        if let Err(e) = session.move_in_hierarchy(id, parent, index) {
            session.say(runity_editor::console::Level::Warning, e.to_string());
        }
        if let Some(p) = parent {
            session.set_open(p, true);
        }
    }

    /// Shift click: everything between the last line clicked and this one.
    fn select_range(&mut self, session: &mut Session, to: EntityId) {
        let from = self.anchor.unwrap_or(to);
        let a = self.shown.iter().position(|r| r.id == from);
        let b = self.shown.iter().position(|r| r.id == to);
        let (Some(a), Some(b)) = (a, b) else {
            let _ = session.add_to_selection(to);
            return;
        };
        let (lo, hi) = (a.min(b), a.max(b));
        let _ = session.select(None);
        for r in &self.shown[lo..=hi] {
            let _ = session.add_to_selection(r.id);
        }
    }

    /// The arrow keys, while the Hierarchy was the last thing clicked: up
    /// and down move the selection, left folds (or goes to the parent),
    /// right unfolds. `true` when it used the key.
    pub fn key(&mut self, session: &mut Session, key: runity::input::Key, shift: bool) -> bool {
        use runity::input::Key;
        if !self.active {
            return false;
        }
        let Some(current) = session.selected() else {
            return false;
        };
        let Some(i) = self.shown.iter().position(|r| r.id == current) else {
            return false;
        };
        let row = &self.shown[i];
        match key {
            Key::Up | Key::Down => {
                let j = if key == Key::Up {
                    i.checked_sub(1)
                } else {
                    Some(i + 1)
                };
                let Some(next) = j.and_then(|j| self.shown.get(j)) else {
                    return true;
                };
                if shift {
                    let _ = session.add_to_selection(next.id);
                } else {
                    let _ = session.select(Some(next.id));
                    self.anchor = Some(next.id);
                }
            }
            Key::Left => {
                if row.has_children && row.open {
                    session.set_open(row.id, false);
                } else if let Some(p) = self.shown[..i]
                    .iter()
                    .rev()
                    .find(|r| r.depth + 1 == row.depth)
                {
                    let _ = session.select(Some(p.id));
                }
            }
            Key::Right => {
                if row.has_children {
                    session.set_open(row.id, true);
                }
            }
            _ => return false,
        }
        true
    }

    /// Whether the Hierarchy has the arrows: after a click on it, until a
    /// click elsewhere.
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
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
