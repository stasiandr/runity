//! Configs: the project's `configs/`, to look at as tables.
//!
//! The files on the left, the open one on the right: its plain fields as
//! lines, and every list or map of like things in it as a grid — one row
//! a record, one column a field (`scrap_editor::configs` says how a file
//! becomes that). Only to look at: the text is the truth and is edited as
//! text (Project Settings, or any editor). A save on disk shows here within
//! a second, as it reaches the running game (DNA, postulate 1); a save that
//! is not RON shows what is wrong and where.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use scrap_editor::configs::{self, Config};
use scrap_editor::Session;
use scrap_ui::{Event, NodeId, Style, Ui};

use crate::theme::*;

/// How often the folder and the open file are looked at for a change.
const LOOK_EVERY: Duration = Duration::from_millis(500);

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
            Some(path) if modified(path) != self.stamp => {
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
                        &format!(
                            "No configs yet: a RON file in {}/ shows here.",
                            scrap::project::CONFIGS
                        ),
                    );
                    ui.clear(self.body);
                }
            }
        }
    }

    fn list_files(&mut self, ui: &mut Ui, root: &std::path::Path, found: Vec<PathBuf>) {
        ui.clear(self.list);
        self.files.clear();
        let dir = root.join(scrap::project::CONFIGS);
        for path in found {
            let name = path
                .strip_prefix(&dir)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
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

    fn show(&mut self, ui: &mut Ui, root: &std::path::Path, path: PathBuf) {
        self.stamp = modified(&path);
        let name = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        ui.clear(self.body);
        match scrap::files::read_to_string(&path).map_err(|e| e.to_string()) {
            Ok(text) => match configs::read(&text) {
                Ok(config) => {
                    ui.set_text(self.title, &name);
                    ui.restyle(self.title, |s| s.text_color(TEXT));
                    draw(ui, self.body, &config);
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

    pub fn event(&mut self, ui: &mut Ui, session: &Session, node: NodeId, event: &Event) {
        let Event::Click { .. } = event else { return };
        let Some(path) = self
            .files
            .iter()
            .find(|(n, _)| *n == node)
            .map(|(_, p)| p.clone())
        else {
            return;
        };
        if let Some(root) = session.project().map(|p| p.root().to_path_buf()) {
            self.show(ui, &root, path);
        }
    }
}

fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// A cell's width for this many characters of monospace text.
fn width_for(chars: usize) -> f32 {
    (chars as f32 * 7.2 + 2.0 * SPACE_2).clamp(40.0, 280.0)
}

fn cell_text() -> Style {
    Style::default()
        .text_size(12.0)
        .mono()
        .text_color(TEXT)
        .nowrap()
}

/// The config's fields as lines, then its tables as grids.
fn draw(ui: &mut Ui, body: NodeId, config: &Config) {
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
        for (key, value) in &config.fields {
            let line = ui.add(lines, Style::row().full_width().gap(SPACE_2).center_items());
            ui.add_text(line, text().text_color(LABEL).width(key_width).fixed(), key);
            let v = ui.add_text(line, cell_text(), value);
            ui.set_name(v, format!("configs field {key}"));
        }
    }
    for table in &config.tables {
        let block = ui.add(body, Style::column().full_width().gap(SPACE_1));
        let title = table.name.clone().unwrap_or_default();
        ui.set_name(block, format!("configs table {title}"));
        ui.add_text(
            block,
            caption(),
            format!("{} · {} ROWS", title.to_uppercase(), table.rows.len())
                .trim_start_matches(" · "),
        );
        let widths: Vec<f32> = (0..table.columns.len())
            .map(|c| {
                let longest = table
                    .rows
                    .iter()
                    .map(|r| r[c].chars().count())
                    .chain([table.columns[c].chars().count()])
                    .max()
                    .unwrap_or(0);
                width_for(longest)
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
        let row = |ui: &mut Ui, cells: &[String], head: bool, odd: bool| {
            let line = ui.add(
                grid,
                Style::row()
                    .full_width()
                    .height(22.0)
                    .fixed()
                    .center_items()
                    .background(if odd {
                        HOVER
                    } else {
                        scrap_ui::Color::TRANSPARENT
                    }),
            );
            for (c, (cell, width)) in cells.iter().zip(&widths).enumerate() {
                let style = if head {
                    caption()
                } else if c == 0 {
                    cell_text().text_color(LABEL)
                } else {
                    cell_text()
                };
                ui.add_text(
                    line,
                    style.width(*width).fixed().padding_x(SPACE_2).clip(),
                    cell,
                );
            }
            line
        };
        let head: Vec<String> = table.columns.iter().map(|c| c.to_uppercase()).collect();
        row(ui, &head, true, false);
        for (i, cells) in table.rows.iter().enumerate() {
            let line = row(ui, cells, false, i % 2 == 1);
            ui.set_name(line, format!("configs row {title} {}", cells[0]));
        }
    }
}
