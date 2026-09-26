//! The Projects screen: Unity Hub's list inside the editor (docs/hub.md).
//!
//! Every project this person works on — pinned first, then the most
//! recently opened — with its branch and the engine its game builds
//! against: that checkout's commit, and a mark when it is not the one this
//! editor was built from. A click opens the project's start scene; Add finds
//! projects in a folder, New makes one. The list and what each line says are
//! `scrap_cli::hub`'s, the same `scrap projects` prints.
//!
//! Reading a project asks git a few things, so the list is read on a thread
//! and drawn when it arrives.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use scrap_cli::hub::{self, Entry, Known};
use scrap_ui::{Color, Event, NodeId, Style, Ui};

use crate::theme::*;

/// What a click on the screen asks the studio to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Nothing more: the screen took it.
    Handled,
    Close,
    /// Open this scene; its project goes on the list as opened.
    Open(PathBuf),
    /// Ask for a folder and add the projects in it.
    Add,
    /// Ask where, and make a project there.
    New,
}

struct Row {
    node: NodeId,
    pin: NodeId,
    forget: NodeId,
    entry: usize,
}

pub struct Hub {
    pub overlay: NodeId,
    field: NodeId,
    list: NodeId,
    status: NodeId,
    add: NodeId,
    new: NodeId,
    close: NodeId,
    rows: Vec<Row>,
    entries: Vec<Entry>,
    loading: Option<Receiver<Result<Vec<Entry>, String>>>,
    config: Option<PathBuf>,
    /// The project open in the editor, marked on its line.
    current: Option<PathBuf>,
    /// The engine checkout this editor was built from.
    engine: Option<PathBuf>,
}

impl Hub {
    pub fn open(ui: &mut Ui, config: Option<PathBuf>, current: Option<PathBuf>) -> Self {
        let root = ui.root();
        let (w, h, _) = ui.viewport();
        let overlay = ui.add(
            root,
            Style::column()
                .absolute(0.0, 0.0)
                .size(w, h)
                .padding(56.0)
                .center_items()
                .background(NEUTRAL_900.alpha(70))
                .clickable(),
        );
        ui.set_layer(overlay, true);
        ui.set_name(overlay, "projects overlay");
        let panel = ui.add(
            overlay,
            Style::column()
                .width(760.0)
                // A height of its own: the list fills what the header and
                // the search leave, and scrolls.
                .height((h - 112.0).min(640.0))
                .padding(SPACE_4)
                .gap(SPACE_3)
                .radius(RADIUS_LG)
                .background(SURFACE)
                .border(1.0, NEUTRAL_500)
                .clickable(),
        );
        ui.set_name(panel, "projects");
        let header = ui.add(panel, Style::row().full_width().gap(SPACE_2).center_items());
        ui.add_text(
            header,
            Style::default()
                .text_size(16.0)
                .weight(500)
                .text_color(TEXT)
                .nowrap()
                .fill(),
            "Projects",
        );
        let add = button(ui, header, "projects add", "Add…", false);
        let new = button(ui, header, "projects new", "New…", true);
        let close = icon_button(ui, header, "projects close", "x", false);
        let field = ui.add_field(
            panel,
            field_style().full_width().height(30.0).text_size(13.0),
            "",
        );
        ui.set_name(field, "projects field");
        ui.set_placeholder(field, "Search by name, folder or branch");
        let status = ui.add_text(panel, caption(), "Reading the projects…");
        ui.set_name(status, "projects status");
        let list = ui.add(
            panel,
            Style::column().full_width().gap(2.0).fill().clip(),
        );
        ui.set_name(list, "projects list");
        ui.focus(Some(field));
        let mut hub = Self {
            overlay,
            field,
            list,
            status,
            add,
            new,
            close,
            rows: Vec::new(),
            entries: Vec::new(),
            loading: None,
            config,
            current: current.map(|p| p.canonicalize().unwrap_or(p)),
            engine: hub::this_engine(),
        };
        hub.reload();
        hub
    }

    /// Read the list again, on a thread.
    pub fn reload(&mut self) {
        let (send, receive) = channel();
        let config = self.config.clone();
        std::thread::spawn(move || {
            let read = match config {
                Some(dir) => Known::load(&dir).map(|k| hub::entries(&k)),
                None => Ok(Vec::new()),
            };
            let _ = send.send(read);
        });
        self.loading = Some(receive);
    }

    /// Draw the list once it has been read. True when it changed.
    pub fn poll(&mut self, ui: &mut Ui) -> bool {
        let Some(receive) = &self.loading else {
            return false;
        };
        let Ok(read) = receive.try_recv() else {
            return false;
        };
        self.loading = None;
        match read {
            Ok(entries) => {
                self.entries = entries;
                let typed = ui.text(self.field).unwrap_or_default().to_string();
                self.fill(ui, &typed);
            }
            Err(problem) => ui.set_text(self.status, &problem),
        }
        true
    }

    /// Wait for the list: a test's screen, drawn as the person would see it.
    pub fn wait(&mut self, ui: &mut Ui) {
        while self.loading.is_some() {
            if !self.poll(ui) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    pub fn close(self, ui: &mut Ui) {
        ui.remove(self.overlay);
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// What an event on the screen asks for; None when it is not the
    /// screen's.
    pub fn event(&mut self, ui: &mut Ui, node: NodeId, event: &Event) -> Option<Outcome> {
        let row = self.rows.iter().position(|r| {
            r.node == node || r.pin == node || r.forget == node
        });
        match event {
            Event::Changed(text) if node == self.field => {
                let text = text.clone();
                self.fill(ui, &text);
                Some(Outcome::Handled)
            }
            Event::Submit(_) if node == self.field => Some(
                self.rows
                    .first()
                    .and_then(|r| self.entries[r.entry].scene.clone())
                    .map_or(Outcome::Handled, Outcome::Open),
            ),
            Event::Cancel if node == self.field => Some(Outcome::Close),
            Event::Click { .. } if node == self.close || node == self.overlay => {
                Some(Outcome::Close)
            }
            Event::Click { .. } if node == self.add => Some(Outcome::Add),
            Event::Click { .. } if node == self.new => Some(Outcome::New),
            Event::Click { .. } if row.is_some() => {
                let row = &self.rows[row.expect("checked")];
                let entry = &self.entries[row.entry];
                let path = entry.path.clone();
                if node == row.pin {
                    let pinned = !entry.pinned;
                    self.change(ui, |k| {
                        k.pin(&path, pinned);
                    });
                    Some(Outcome::Handled)
                } else if node == row.forget {
                    self.change(ui, |k| {
                        k.forget(&path);
                    });
                    Some(Outcome::Handled)
                } else {
                    Some(entry.scene.clone().map_or(Outcome::Handled, Outcome::Open))
                }
            }
            _ if node == self.field => Some(Outcome::Handled),
            _ => None,
        }
    }

    /// Change the list, save it, and read it again.
    pub fn change(&mut self, ui: &mut Ui, change: impl FnOnce(&mut Known)) {
        let Some(dir) = &self.config else { return };
        if let Err(problem) = hub::update(dir, change) {
            ui.set_text(self.status, &problem);
            return;
        }
        self.reload();
    }

    /// The lines for what is typed: a project matches by name, folder, or
    /// either branch.
    fn fill(&mut self, ui: &mut Ui, typed: &str) {
        let query = typed.trim().to_lowercase();
        ui.clear(self.list);
        self.rows.clear();
        let shown: Vec<usize> = (0..self.entries.len())
            .filter(|&i| {
                let e = &self.entries[i];
                query.is_empty()
                    || [
                        Some(e.name.clone()),
                        Some(e.path.display().to_string()),
                        e.branch.clone(),
                        e.engine.branch.clone(),
                        e.engine.commit.clone(),
                    ]
                    .into_iter()
                    .flatten()
                    .any(|s| s.to_lowercase().contains(&query))
            })
            .collect();
        let status = match (self.entries.len(), shown.len()) {
            (0, _) => "No projects yet: open a scene, or Add a folder of them.".to_string(),
            (all, n) if n == all => format!("{all} projects"),
            (all, n) => format!("{n} of {all} projects"),
        };
        let others = self.entries.iter().filter(|e| self.other_engine(e)).count();
        let status = if others > 0 {
            format!("{status} · an engine in yellow is another checkout than this editor's")
        } else {
            status
        };
        ui.set_text(self.status, &status);
        for i in shown {
            let row = self.row(ui, i);
            self.rows.push(row);
        }
    }

    fn row(&self, ui: &mut Ui, i: usize) -> Row {
        let e = &self.entries[i];
        let broken = e.problem.is_some();
        let current = self.current.as_deref() == Some(e.path.as_path());
        let node = ui.add(
            self.list,
            Style::row()
                .full_width()
                .height(52.0)
                .fixed()
                .padding_x(SPACE_3)
                .gap(SPACE_3)
                .center_items()
                .radius(RADIUS_MD)
                .hover(ACCENT.alpha(16))
                .background(if current { ACCENT_900 } else { Color::TRANSPARENT })
                .clickable(),
        );
        ui.set_name(node, format!("project {}", e.path.display()));
        icon(ui, node, "folder", if broken { MUTED } else { ACCENT_300 });

        // Name and folder.
        let names = ui.add(node, Style::column().fill().gap(2.0));
        let title = ui.add(names, Style::row().gap(SPACE_2).center_items());
        ui.add_text(
            title,
            text().weight(500).text_color(if broken { MUTED } else { TEXT }),
            &e.name,
        );
        if current {
            ui.add_text(title, caption().text_color(ACCENT_300), "open now");
        }
        let under = match &e.problem {
            Some(problem) => problem.clone(),
            None => home_relative(&e.path),
        };
        ui.add_text(
            names,
            caption().text_color(if broken { WARNING } else { MUTED }),
            &under,
        );

        // Branch, engine, when.
        if !broken {
            let facts = ui.add(node, Style::column().gap(2.0).width(270.0).fixed().clip());
            let line = ui.add(facts, Style::row().gap(SPACE_1).center_items());
            icon(ui, line, "git-branch", MUTED);
            ui.add_text(line, caption(), e.branch.as_deref().unwrap_or("no branch"));
            ui.add_text(line, caption(), &format!("· {}", hub::ago(e.opened)));
            let line = ui.add(facts, Style::row().gap(SPACE_1).center_items());
            let other = self.other_engine(e);
            icon(
                ui,
                line,
                "git-commit-horizontal",
                if other { WARNING } else { MUTED },
            );
            let engine = ui.add_text(
                line,
                caption().text_color(if other { WARNING } else { MUTED }),
                &format!("engine {}", e.engine.label_within(24)),
            );
            ui.set_name(engine, format!("engine {}", e.path.display()));
        }

        let pin = icon_button(
            ui,
            node,
            &format!("pin {}", e.path.display()),
            "pin",
            e.pinned,
        );
        let forget = icon_button(
            ui,
            node,
            &format!("forget {}", e.path.display()),
            "x",
            false,
        );
        Row {
            node,
            pin,
            forget,
            entry: i,
        }
    }

    /// The project's game builds against another checkout of the engine
    /// than the one this editor came from.
    fn other_engine(&self, e: &Entry) -> bool {
        match (&e.engine.checkout, &self.engine) {
            (Some(theirs), Some(ours)) => {
                theirs.canonicalize().unwrap_or(theirs.clone()) != *ours
            }
            _ => false,
        }
    }
}

/// `~/personal/dacha` rather than the whole path.
fn home_relative(path: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if path.starts_with(&home) => {
            format!("~/{}", path.strip_prefix(&home).unwrap_or(path).display())
        }
        _ => path.display().to_string(),
    }
}

/// What to tell the person after opening `root`: when its game builds
/// against another engine checkout than this editor's.
pub fn engine_note(root: &Path) -> Option<String> {
    let theirs = hub::engine_of(root);
    let checkout = theirs.checkout.as_ref()?;
    let ours = hub::this_engine()?;
    let checkout = checkout.canonicalize().unwrap_or(checkout.clone());
    (checkout != ours).then(|| {
        format!(
            "this project's game builds against the engine at {} ({}); the editor is from {}",
            home_relative(&checkout),
            theirs.label(),
            home_relative(&ours),
        )
    })
}
