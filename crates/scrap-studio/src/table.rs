//! Table: many things of one kind in rows — the records of a tuning file,
//! or the entities a search finds — their fields in columns, a cell a text
//! field (`scrap_editor::table`). What a designer balances a game in, as
//! Odin's table and Unreal's DataTable; the RON files stay the truth, and a
//! cell typed here changes that field in the file and nothing else.
//!
//! On the left, the tables there are (each file of `tuning/`, `c:<name>`
//! for each component of the scene) and a search box for any other; on
//! the right, the grid: a click on a column's name sorts by it, Enter in a
//! cell sets it, an emptied cell goes back to its default. A record that
//! does not fit the game's type has its name in red and says why above.

use std::collections::HashMap;

use scrap_editor::console::Level;
use scrap_editor::table::Table as Rows;
use scrap_editor::Session;
use scrap_ui::{Color, Event, NodeId, Style, Ui};

use crate::theme::*;

const LABEL_W: f32 = 140.0;
const CELL_W: f32 = 110.0;

#[derive(Debug, Clone)]
enum Part {
    Source(String),
    Column(usize),
    Cell { row: String, column: String },
}

pub struct Table {
    pub root: NodeId,
    sources: NodeId,
    query: NodeId,
    title: NodeId,
    undo: NodeId,
    notes: NodeId,
    grid: NodeId,
    parts: HashMap<NodeId, Part>,
    open: Option<String>,
    /// Column and whether it runs down (largest first).
    sort: Option<(usize, bool)>,
    /// What the grid was built from: the document's revision, and the
    /// file's clock for a tuning file — a change from outside shows.
    seen: Option<(u64, Option<std::time::SystemTime>)>,
    listed: bool,
}

impl Table {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let root = ui.add(parent, Style::row().fill().full_width());
        ui.set_name(root, "table");
        let left = ui.add(
            root,
            Style::column()
                .width(190.0)
                .full_height()
                .fixed()
                .padding(SPACE_2)
                .gap(2.0),
        );
        ui.add_text(
            left,
            caption().padding_x(SPACE_2).padding_y(SPACE_2),
            "TABLES",
        );
        let query = ui.add_field(left, field_style().full_width(), "");
        ui.set_name(query, "table query");
        let sources = ui.add(left, Style::column().fill().full_width().gap(2.0).clip());
        ui.set_name(sources, "table sources");
        let right = ui.add(
            root,
            Style::column()
                .fill()
                .full_height()
                .padding(SPACE_2)
                .gap(SPACE_2),
        );
        let bar = ui.add(right, Style::row().full_width().gap(SPACE_2).center_items());
        let title = ui.add_text(
            bar,
            text().fill().text_color(LABEL),
            "Choose a table on the left, or search the scene: c:enemy.",
        );
        let undo = button(ui, bar, "table undo", "Undo Cell", false);
        let notes = ui.add(right, Style::column().full_width().gap(2.0));
        let grid = ui.add(right, Style::column().fill().full_width().clip());
        ui.set_name(grid, "table grid");
        Self {
            root,
            sources,
            query,
            title,
            undo,
            notes,
            grid,
            parts: HashMap::new(),
            open: None,
            sort: None,
            seen: None,
            listed: false,
        }
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

    /// What the open table was built from now.
    fn stamp(&self, session: &Session) -> (u64, Option<std::time::SystemTime>) {
        let file = self
            .open
            .as_deref()
            .filter(|s| s.starts_with("tuning/"))
            .and_then(|s| session.project().map(|p| p.root().join(s)))
            .and_then(|path| std::fs::metadata(path).ok())
            .and_then(|m| m.modified().ok());
        (session.revision(), file)
    }

    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        if !self.listed {
            self.listed = true;
            self.list(ui, session);
        }
        // Rebuilt when the document or the file changed — not under a
        // cell being typed into.
        let typing = ui
            .focused()
            .is_some_and(|f| matches!(self.parts.get(&f), Some(Part::Cell { .. })));
        if self.open.is_some() && !typing && self.seen != Some(self.stamp(session)) {
            self.show(ui, session);
        }
    }

    fn list(&mut self, ui: &mut Ui, session: &Session) {
        ui.clear(self.sources);
        self.parts.retain(|_, p| !matches!(p, Part::Source(_)));
        let sources = session.table_sources();
        if sources.is_empty() {
            ui.add_text(
                self.sources,
                Style::default().text_size(11.5).text_color(MUTED),
                "No tables yet: tuning/<name>.ron, or a component in the scene.",
            );
        }
        for source in sources {
            let on = self.open.as_deref() == Some(source.as_str());
            let row = ui.add(
                self.sources,
                Style::row()
                    .full_width()
                    .height(22.0)
                    .fixed()
                    .padding_x(SPACE_2)
                    .gap(SPACE_2)
                    .center_items()
                    .radius(RADIUS_SM)
                    .background(if on { ACCENT_900 } else { Color::TRANSPARENT })
                    .hover(HOVER)
                    .clickable(),
            );
            ui.set_name(row, format!("table source {source}"));
            let glyph = if source.starts_with("tuning/") {
                "file"
            } else {
                "list-tree"
            };
            icon(ui, row, glyph, MUTED);
            ui.add_text(row, text(), &source);
            self.parts.insert(row, Part::Source(source));
        }
    }

    fn open(&mut self, ui: &mut Ui, session: &Session, source: String) {
        self.open = Some(source);
        self.sort = None;
        self.list(ui, session);
        self.show(ui, session);
    }

    /// Build the grid again from the session.
    fn show(&mut self, ui: &mut Ui, session: &Session) {
        self.parts.retain(|_, p| matches!(p, Part::Source(_)));
        ui.clear(self.grid);
        ui.clear(self.notes);
        self.seen = Some(self.stamp(session));
        let Some(source) = self.open.clone() else {
            return;
        };
        let table = match session.table(&source) {
            Ok(table) => table,
            Err(e) => {
                ui.set_text(self.title, &source);
                ui.add_text(
                    self.notes,
                    Style::default().text_size(11.5).text_color(ERROR),
                    &e.to_string(),
                );
                return;
            }
        };
        ui.set_text(self.title, &format!("{source} — {} rows", table.rows.len()));
        ui.restyle(self.title, |s| s.text_color(TEXT));
        let problems = table.problems.iter().cloned().chain(
            table
                .rows
                .iter()
                .flat_map(|r| r.problems.iter().map(move |p| format!("{}: {p}", r.label))),
        );
        for problem in problems.take(4) {
            ui.add_text(
                self.notes,
                Style::default().text_size(11.5).text_color(ERROR),
                &problem,
            );
        }
        self.grid(ui, &table);
    }

    fn grid(&mut self, ui: &mut Ui, table: &Rows) {
        let line = |ui: &mut Ui, parent| {
            ui.add(
                parent,
                Style::row()
                    .full_width()
                    .height(26.0)
                    .fixed()
                    .gap(SPACE_1)
                    .center_items(),
            )
        };
        let header = line(ui, self.grid);
        ui.add_text(header, caption().width(LABEL_W).fixed(), "");
        for (i, column) in table.columns.iter().enumerate() {
            let arrow = match self.sort {
                Some((c, true)) if c == i => " ↓",
                Some((c, false)) if c == i => " ↑",
                _ => "",
            };
            let cell = ui.add(
                header,
                Style::row()
                    .width(CELL_W)
                    .height(22.0)
                    .fixed()
                    .padding_x(6.0)
                    .center_items()
                    .radius(RADIUS_SM)
                    .hover(HOVER)
                    .clickable()
                    .clip(),
            );
            ui.set_name(cell, format!("table column {}", column.name));
            ui.add_text(
                cell,
                caption().text_color(LABEL),
                &format!("{}{arrow}", column.name),
            );
            self.parts.insert(cell, Part::Column(i));
        }
        let mut order: Vec<usize> = (0..table.rows.len()).collect();
        if let Some((c, down)) = self.sort {
            order.sort_by(|a, b| {
                let (x, y) = (&table.rows[*a].cells[c], &table.rows[*b].cells[c]);
                let by = match (x.parse::<f64>(), y.parse::<f64>()) {
                    (Ok(x), Ok(y)) => x.total_cmp(&y),
                    _ => x.cmp(y),
                };
                if down {
                    by.reverse()
                } else {
                    by
                }
            });
        }
        for i in order {
            let row = &table.rows[i];
            let r = line(ui, self.grid);
            ui.set_name(r, format!("table row {}", row.label));
            ui.add_text(
                r,
                text()
                    .width(LABEL_W)
                    .fixed()
                    .text_color(if row.problems.is_empty() { TEXT } else { ERROR })
                    .clip(),
                &row.label,
            );
            for (column, value) in table.columns.iter().zip(&row.cells) {
                let cell = ui.add_field(r, field_style().width(CELL_W).fixed().mono(), value);
                ui.set_name(cell, format!("table cell {} {}", row.label, column.name));
                self.parts.insert(
                    cell,
                    Part::Cell {
                        row: row.key.clone(),
                        column: column.name.clone(),
                    },
                );
            }
        }
    }

    pub fn event(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, event: &Event) {
        match event {
            Event::Submit(query) if node == self.query => {
                let query = query.trim();
                if !query.is_empty() {
                    self.open(ui, session, query.to_string());
                }
                return;
            }
            Event::Click { .. } if node == self.undo => {
                match session.undo_cell() {
                    Ok(Some(what)) => session.say(Level::Info, format!("undone: {what}")),
                    Ok(None) => session.say(Level::Info, "no cell to take back"),
                    Err(e) => session.say(Level::Error, e.to_string()),
                }
                self.show(ui, session);
                return;
            }
            _ => {}
        }
        let Some(part) = self.parts.get(&node).cloned() else {
            return;
        };
        match (part, event) {
            (Part::Source(source), Event::Click { .. }) => self.open(ui, session, source),
            (Part::Column(i), Event::Click { .. }) => {
                self.sort = match self.sort {
                    Some((c, false)) if c == i => Some((i, true)),
                    Some((c, true)) if c == i => None,
                    _ => Some((i, false)),
                };
                self.show(ui, session);
            }
            (Part::Cell { row, column }, Event::Submit(value)) => {
                let Some(source) = self.open.clone() else {
                    return;
                };
                if let Err(e) = session.set_cell(&source, &row, &column, value) {
                    session.say(Level::Error, e.to_string());
                }
                // The cell shows what the file says now, taken or not.
                ui.focus(None);
                self.show(ui, session);
            }
            _ => {}
        }
    }
}
