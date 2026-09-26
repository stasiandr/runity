//! Configs: the project's `configs/`, as tables, edited a cell at a time.
//!
//! The files on the left, the open one on the right: its plain fields as
//! lines, and every list or map of like things in it as a grid — one row
//! a record, one column a field (`scrap_editor::configs` says how a file
//! becomes that, and how an edit goes back into it). A cell is a field to
//! type RON into; where the game has said what its table holds
//! (`library/tables.ron`), an enum, a flag or a link to a record is picked
//! from a list instead. What a record takes from its `base` is shown dim,
//! and typing over it gives the record its own. An edit that would not
//! read or not fit is not written, and says why; one that is written is a
//! step of Undo while nothing else has been edited since. A save on disk
//! from anywhere shows here within a second, as it reaches the running
//! game (DNA, postulate 1).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use scrap::table::{Index, TableShape};
use scrap_editor::configs::{self, Config, Edits, Place};
use scrap_editor::console::Level;
use scrap_editor::Session;
use scrap_ui::{Event, NodeId, Style, Ui};

use crate::menu::{Action, MenuItem};
use crate::studio::Requests;
use crate::theme::*;

/// How often the folder and the open file are looked at for a change.
const LOOK_EVERY: Duration = Duration::from_millis(500);

/// What a cell of the open file is.
enum Cell {
    /// Typed into: its place, and its text as shown.
    Typed(Place, String),
    /// Picked from a list: its place and the list, (reads, writes).
    Picked(Place, Vec<(String, String)>),
}

pub struct Configs {
    pub root: NodeId,
    list: NodeId,
    title: NodeId,
    body: NodeId,
    files: Vec<(NodeId, PathBuf)>,
    open: Option<PathBuf>,
    /// The open file's clock when it was last read.
    stamp: Option<SystemTime>,
    looked: Option<Instant>,
    cells: Vec<(NodeId, Cell)>,
    edits: Edits,
    /// The document's revision when this window last edited or undid:
    /// while it is still that, Undo is this window's.
    revision: Option<u64>,
}

impl Configs {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let root = ui.add(parent, Style::row().fill().full_width());
        ui.set_name(root, "configs");
        let left = ui.add(
            root,
            Style::column()
                .width(200.0)
                .full_height()
                .fixed()
                .padding(SPACE_2)
                .gap(2.0),
        );
        ui.add_text(
            left,
            caption().padding_x(SPACE_2).padding_y(SPACE_2),
            "CONFIGS",
        );
        let list = ui.add(left, Style::column().fill().full_width().gap(2.0).clip());
        ui.set_name(list, "configs files");
        let right = ui.add(
            root,
            Style::column()
                .fill()
                .full_height()
                .padding(SPACE_2)
                .gap(SPACE_2),
        );
        let title = ui.add_text(right, text().text_color(LABEL), "");
        ui.set_name(title, "configs title");
        let body = ui.add(
            right,
            Style::column().fill().full_width().gap(SPACE_4).clip(),
        );
        ui.set_name(body, "configs body");
        Self {
            root,
            list,
            title,
            body,
            files: Vec::new(),
            open: None,
            stamp: None,
            looked: None,
            cells: Vec::new(),
            edits: Edits::new(),
            revision: None,
        }
    }

    /// Look at the disk, at most every [`LOOK_EVERY`]: a file added or
    /// removed changes the list, a save of the open one redraws it.
    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        if self.looked.is_some_and(|at| at.elapsed() < LOOK_EVERY) {
            return;
        }
        self.looked = Some(Instant::now());
        let Some(root) = session.project().map(|p| p.root().to_path_buf()) else {
            ui.set_text(self.title, "No project open.");
            return;
        };
        let found = configs::files(&root);
        let listed: Vec<&PathBuf> = self.files.iter().map(|(_, p)| p).collect();
        if found.iter().collect::<Vec<_>>() != listed {
            self.list_files(ui, &root, found);
        }
        match &self.open {
            Some(path) if !self.files.iter().any(|(_, p)| p == path) => {
                self.open = None;
            }
            // Not while a cell is being typed in: the redraw would take the
            // typing away. The next look, after it, redraws.
            Some(path) if modified(path) != self.stamp && !self.typing(ui) => {
                self.show(ui, &root, path.clone());
            }
            _ => {}
        }
        if self.open.is_none() {
            match self.files.first().map(|(_, p)| p.clone()) {
                Some(path) => self.show(ui, &root, path),
                None => {
                    ui.set_text(
                        self.title,
                        "No configs yet: a table or tuned numbers — a plain .ron — shows here.",
                    );
                    ui.clear(self.body);
                    self.cells.clear();
                }
            }
        }
    }

    /// Whether the keyboard is in one of this window's cells.
    fn typing(&self, ui: &Ui) -> bool {
        ui.focused()
            .is_some_and(|f| self.cells.iter().any(|(n, _)| *n == f))
    }

    fn list_files(&mut self, ui: &mut Ui, root: &Path, found: Vec<PathBuf>) {
        ui.clear(self.list);
        self.files.clear();
        for path in found {
            let name = scrap::layout::relative(root, &path);
            let name = name.strip_prefix("configs/").unwrap_or(&name).to_string();
            let row = ui.add(
                self.list,
                Style::row()
                    .full_width()
                    .height(24.0)
                    .fixed()
                    .padding_x(SPACE_2)
                    .gap(SPACE_2)
                    .center_items()
                    .radius(RADIUS_SM)
                    .hover(HOVER),
            );
            icon(ui, row, "grid-3x3", MUTED);
            ui.add_text(row, text(), &name);
            ui.set_name(row, format!("configs {name}"));
            self.files.push((row, path));
        }
        self.mark(ui);
    }

    fn show(&mut self, ui: &mut Ui, root: &Path, path: PathBuf) {
        self.stamp = modified(&path);
        let name = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        ui.clear(self.body);
        self.cells.clear();
        let shape = configs::shape_of(root, &path);
        match scrap::files::read_to_string(&path).map_err(|e| e.to_string()) {
            Ok(text) => match configs::read_with(&text, shape.as_ref()) {
                Ok(config) => {
                    ui.set_text(self.title, &name);
                    ui.restyle(self.title, |s| s.text_color(TEXT));
                    let index = shape.as_ref().map(|_| configs::index(root));
                    self.cells = draw(ui, self.body, &config, shape.as_ref(), index.as_ref());
                }
                Err(e) => self.problem(ui, &name, &e),
            },
            Err(e) => self.problem(ui, &name, &e),
        }
        self.open = Some(path);
        self.mark(ui);
    }

    /// The file as it cannot be read, and why — the last good view would
    /// hide that the save did not take.
    fn problem(&mut self, ui: &mut Ui, name: &str, error: &str) {
        ui.set_text(self.title, &format!("{name}: not RON"));
        ui.restyle(self.title, |s| s.text_color(ERROR));
        let e = ui.add_text(
            self.body,
            Style::default()
                .text_size(12.0)
                .mono()
                .text_color(TEXT)
                .full_width(),
            error,
        );
        ui.set_name(e, "configs problem");
    }

    fn mark(&self, ui: &mut Ui) {
        for (row, p) in &self.files {
            let on = Some(p) == self.open.as_ref();
            ui.restyle(*row, |s| {
                s.background(if on {
                    ACCENT_900
                } else {
                    scrap_ui::Color::TRANSPARENT
                })
            });
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

    pub fn event(
        &mut self,
        ui: &mut Ui,
        session: &mut Session,
        node: NodeId,
        event: &Event,
        requests: &mut Requests,
    ) {
        if let Event::Click { .. } = event {
            if let Some(path) = self
                .files
                .iter()
                .find(|(n, _)| *n == node)
                .map(|(_, p)| p.clone())
            {
                if let Some(root) = session.project().map(|p| p.root().to_path_buf()) {
                    self.show(ui, &root, path);
                }
                return;
            }
        }
        let Some(cell) = self.cells.iter().find(|(n, _)| *n == node).map(|(_, c)| c) else {
            return;
        };
        match (cell, event) {
            (Cell::Picked(place, options), Event::Click { .. }) => {
                let r = ui.rect(node);
                let items = options
                    .iter()
                    .map(|(label, text)| {
                        MenuItem::new(label, Action::SetConfig(place.clone(), text.clone()))
                    })
                    .collect();
                requests.menu = Some((items, r.x, r.y + r.height));
            }
            (Cell::Typed(place, shown), Event::Submit(value)) => {
                if value.trim() != shown.trim() {
                    let place = place.clone();
                    self.set(ui, session, &place, value);
                }
            }
            (Cell::Typed(_, shown), Event::Cancel) => {
                let shown = shown.clone();
                ui.set_text(node, &shown);
            }
            _ => {}
        }
    }

    /// Write `value` at `place` of the open file: a cell typed in, or a
    /// line picked from a cell's list.
    pub fn set(&mut self, ui: &mut Ui, session: &mut Session, place: &Place, value: &str) {
        let (Some(root), Some(file)) = (
            session.project().map(|p| p.root().to_path_buf()),
            self.open.clone(),
        ) else {
            return;
        };
        match self.edits.set(&root, &file, place, value) {
            Ok(label) if !label.is_empty() => {
                self.revision = Some(session.revision());
                session.say(Level::Info, format!("edited {label}"));
            }
            Ok(_) => {}
            Err(e) => session.say(Level::Error, format!("{place}: {e}")),
        }
        // Drawn again either way: a refused edit shows the file as it is.
        self.show(ui, &root, file);
    }

    /// Whether Undo, now, is this window's: it has a step, and nothing in
    /// the document was edited since its last one.
    pub fn takes_undo(&self, session: &Session) -> bool {
        self.edits.can_undo() && self.revision == Some(session.revision())
    }

    pub fn takes_redo(&self, session: &Session) -> bool {
        self.edits.can_redo() && self.revision == Some(session.revision())
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.edits.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.edits.redo_label()
    }

    /// Undo (or redo, `forward`) this window's last edit, and show the file.
    pub fn step(&mut self, ui: &mut Ui, session: &mut Session, forward: bool) {
        let result = if forward {
            self.edits.redo()
        } else {
            self.edits.undo()
        };
        match result {
            Ok(Some(label)) => session.say(
                Level::Info,
                format!("{} {label}", if forward { "redone" } else { "undone" }),
            ),
            Ok(None) => {}
            Err(e) => session.say(Level::Error, e),
        }
        self.revision = Some(session.revision());
        if let (Some(root), Some(file)) = (
            session.project().map(|p| p.root().to_path_buf()),
            self.open.clone(),
        ) {
            self.show(ui, &root, file);
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// A cell's width for this many characters of monospace text.
fn width_for(chars: usize) -> f32 {
    (chars as f32 * 7.2 + 2.0 * SPACE_2 + 4.0).clamp(48.0, 560.0)
}

fn cell_text() -> Style {
    Style::default()
        .text_size(12.0)
        .mono()
        .text_color(TEXT)
        .nowrap()
}

/// A cell to type in: no box until the pointer is over it, as a
/// spreadsheet's.
fn cell_field(width: f32, dim: bool) -> Style {
    Style::row()
        .width(width)
        .height(22.0)
        .fixed()
        .padding_x(SPACE_2)
        .center_items()
        .radius(RADIUS_SM)
        .hover_border(TEXT.alpha(35))
        .text_size(12.0)
        .mono()
        .text_color(if dim { MUTED } else { TEXT })
}

/// Draw one editable value: a field, or a box that opens its list.
fn value_cell(
    ui: &mut Ui,
    parent: NodeId,
    width: f32,
    shown: &str,
    dim: bool,
    place: Place,
    options: Option<Vec<(String, String)>>,
) -> (NodeId, Cell) {
    match options {
        Some(options) => {
            let pick = ui.add(
                parent,
                Style::row()
                    .width(width)
                    .height(22.0)
                    .fixed()
                    .padding_x(SPACE_2)
                    .gap(SPACE_1)
                    .center_items()
                    .radius(RADIUS_SM)
                    .hover(HOVER),
            );
            ui.add_text(
                pick,
                cell_text()
                    .fill()
                    .clip()
                    .text_color(if dim { MUTED } else { TEXT }),
                shown,
            );
            icon(ui, pick, "chevron-down", MUTED);
            (pick, Cell::Picked(place, options))
        }
        None => {
            let field = ui.add_field(parent, cell_field(width, dim), shown);
            (field, Cell::Typed(place, shown.to_string()))
        }
    }
}

/// What a column of a table's records is picked from, when it is.
fn column_choices(
    column: &str,
    shape: Option<&TableShape>,
    index: Option<&Index>,
) -> Option<Vec<(String, String)>> {
    let (shape, index) = (shape?, index?);
    if column == "base" {
        let own = scrap::shape::Shape::Option(Box::new(scrap::shape::Shape::Record(
            shape.record.clone(),
        )));
        // A base is a name, or none: `None` takes the base away.
        return configs::choices(&own, index).map(|options| {
            options
                .into_iter()
                .map(|(label, text)| {
                    let text = text
                        .strip_prefix("Some(")
                        .and_then(|t| t.strip_suffix(')'))
                        .map_or(String::new(), str::to_string);
                    (label, text)
                })
                .collect()
        });
    }
    configs::choices(shape.field(column)?, index)
}

/// The config's fields as lines, then its tables as grids; what each
/// editable cell is.
fn draw(
    ui: &mut Ui,
    body: NodeId,
    config: &Config,
    shape: Option<&TableShape>,
    index: Option<&Index>,
) -> Vec<(NodeId, Cell)> {
    let mut cells = Vec::new();
    if !config.fields.is_empty() {
        let lines = ui.add(body, Style::column().full_width().gap(2.0));
        ui.set_name(lines, "configs fields");
        let key_width = width_for(
            config
                .fields
                .iter()
                .map(|(k, _)| k.chars().count())
                .max()
                .unwrap_or(0),
        );
        let value_width = width_for(
            config
                .fields
                .iter()
                .map(|(_, v)| v.chars().count())
                .max()
                .unwrap_or(0)
                + 8,
        );
        for (key, value) in &config.fields {
            let line = ui.add(lines, Style::row().full_width().gap(SPACE_2).center_items());
            ui.add_text(line, text().text_color(LABEL).width(key_width).fixed(), key);
            let (node, cell) = value_cell(
                ui,
                line,
                value_width,
                value,
                false,
                Place::Field(key.clone()),
                None,
            );
            ui.set_name(node, format!("configs field {key}"));
            cells.push((node, cell));
        }
    }
    for table in &config.tables {
        let block = ui.add(body, Style::column().full_width().gap(SPACE_1));
        let title = table.name.clone().unwrap_or_default();
        ui.set_name(block, format!("configs table {title}"));
        let heading = match (&table.name, shape) {
            (Some(name), _) => format!("{} · {} ROWS", name.to_uppercase(), table.rows.len()),
            (None, Some(shape)) => format!(
                "{} · {} RECORDS",
                shape.record.to_uppercase(),
                table.rows.len()
            ),
            (None, None) if table.records => format!("{} RECORDS", table.rows.len()),
            (None, None) => format!("{} ROWS", table.rows.len()),
        };
        ui.add_text(block, caption(), &heading);
        let widths: Vec<f32> = (0..table.columns.len())
            .map(|c| {
                let longest = table
                    .rows
                    .iter()
                    .map(|r| r[c].chars().count())
                    .chain([table.columns[c].chars().count()])
                    .max()
                    .unwrap_or(0);
                // Room for a list's chevron.
                width_for(longest + 2)
            })
            .collect();
        let grid = ui.add(
            block,
            Style::column()
                .full_width()
                .radius(RADIUS_SM)
                .border(1.0, HOVER)
                .clip(),
        );
        let line = |ui: &mut Ui, odd: bool| {
            ui.add(
                grid,
                Style::row()
                    .full_width()
                    .height(24.0)
                    .fixed()
                    .center_items()
                    .background(if odd {
                        HOVER
                    } else {
                        scrap_ui::Color::TRANSPARENT
                    }),
            )
        };
        let head = line(ui, false);
        for (column, width) in table.columns.iter().zip(&widths) {
            ui.add_text(
                head,
                caption().width(*width).fixed().padding_x(SPACE_2).clip(),
                &column.to_uppercase(),
            );
        }
        for (i, row) in table.rows.iter().enumerate() {
            let at = line(ui, i % 2 == 1);
            ui.set_name(at, format!("configs row {title} {}", row[0]));
            for (c, (value, width)) in row.iter().zip(&widths).enumerate() {
                let column = &table.columns[c];
                // A list's place is not a value.
                if column == "#" {
                    ui.add_text(
                        at,
                        cell_text()
                            .text_color(LABEL)
                            .width(*width)
                            .fixed()
                            .padding_x(SPACE_2),
                        value,
                    );
                    continue;
                }
                let place = Place::Cell {
                    grid: table.name.clone(),
                    row: i,
                    column: column.clone(),
                };
                let options = if table.records {
                    column_choices(column, shape, index)
                } else {
                    None
                };
                let dim = table.inherited[i][c];
                let (node, cell) = value_cell(ui, at, *width, value, dim, place, options);
                ui.set_name(node, format!("configs cell {title} {} {column}", row[0]));
                cells.push((node, cell));
            }
        }
    }
    cells
}
