//! The Projects screen: Unity Hub's list inside the editor (docs/hub.md).
//!
//! Every project this person works on — pinned first, then the most
//! recently opened — with its branch and the engine its game builds
//! against: that checkout's commit, and a mark when it is not the one this
//! editor was built from. A click opens the project's start scene; Add finds
//! projects in a folder; New makes one — empty, with a set of modules, or a
//! copy of one of the engine's examples. The list and what each line says
//! are `scrap_cli::hub`'s, the same `scrap projects` prints.
//!
//! The app starts on it with nothing open behind it: until a project is
//! chosen there is nothing to close it to.
//!
//! Reading a project asks git a few things, so the list is read on a thread
//! and drawn when it arrives.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use scrap_cli::hub::{self, Choice, Entry, Known};
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
    /// Ask for the folder a new project goes in.
    ChooseLocation,
}

struct Row {
    node: NodeId,
    pin: NodeId,
    forget: NodeId,
    entry: usize,
}

/// The list: a search over the projects.
struct List {
    field: NodeId,
    list: NodeId,
    status: NodeId,
    add: NodeId,
    new: NodeId,
    close: Option<NodeId>,
    rows: Vec<Row>,
}

/// New: a name, a folder, and what it starts from.
struct Form {
    name: NodeId,
    location: NodeId,
    choose: NodeId,
    back: NodeId,
    create: NodeId,
    status: NodeId,
    choices: Vec<(NodeId, Choice)>,
    chosen: usize,
}

enum Mode {
    List(List),
    New(Form),
}

pub struct Hub {
    pub overlay: NodeId,
    panel: NodeId,
    mode: Mode,
    entries: Vec<Entry>,
    loading: Option<Receiver<Result<Vec<Entry>, String>>>,
    config: Option<PathBuf>,
    /// The project open in the editor, marked on its line; None at the
    /// start, when the screen cannot be closed.
    current: Option<PathBuf>,
    /// The engine checkout this editor was built from.
    engine: Option<PathBuf>,
    /// Where New puts a project: chosen, or beside the one opened last.
    location: Option<PathBuf>,
}

impl Hub {
    pub fn open(ui: &mut Ui, config: Option<PathBuf>, current: Option<PathBuf>) -> Self {
        let root = ui.root();
        let (w, h, _) = ui.viewport();
        let start = current.is_none();
        let overlay = ui.add(
            root,
            Style::column()
                .absolute(0.0, 0.0)
                .size(w, h)
                .padding(56.0)
                .center_items()
                // Nothing open behind it at the start: the ground, not a
                // dimmed empty editor.
                .background(if start { BG } else { NEUTRAL_900.alpha(70) })
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
        let mode = Mode::List(Self::build_list(ui, panel, start));
        let mut hub = Self {
            overlay,
            panel,
            mode,
            entries: Vec::new(),
            loading: None,
            config,
            current: current.map(|p| p.canonicalize().unwrap_or(p)),
            engine: hub::this_engine(),
            location: None,
        };
        hub.reload();
        hub
    }

    fn build_list(ui: &mut Ui, panel: NodeId, start: bool) -> List {
        ui.clear(panel);
        let header = ui.add(panel, Style::row().full_width().gap(SPACE_2).center_items());
        title(ui, header, "Projects");
        let add = button(ui, header, "projects add", "Add…", false);
        let new = button(ui, header, "projects new", "New…", true);
        let close = (!start).then(|| icon_button(ui, header, "projects close", "x", false));
        let field = ui.add_field(
            panel,
            field_style().full_width().height(30.0).text_size(13.0),
            "",
        );
        ui.set_name(field, "projects field");
        ui.set_placeholder(field, "Search by name, folder or branch");
        let status = ui.add_text(panel, caption(), "Reading the projects…");
        ui.set_name(status, "projects status");
        let list = ui.add(panel, Style::column().full_width().gap(2.0).fill().clip());
        ui.set_name(list, "projects list");
        ui.focus(Some(field));
        List {
            field,
            list,
            status,
            add,
            new,
            close,
            rows: Vec::new(),
        }
    }

    fn build_form(&self, ui: &mut Ui) -> Form {
        let panel = self.panel;
        ui.clear(panel);
        let header = ui.add(panel, Style::row().full_width().gap(SPACE_2).center_items());
        title(ui, header, "New project");
        let back = button(ui, header, "new back", "Back", false);

        let row = ui.add(panel, Style::row().full_width().gap(SPACE_2).center_items());
        ui.add_text(row, text().width(64.0).fixed(), "Name");
        let name = ui.add_field(row, field_style().fill().height(28.0).text_size(13.0), "my-game");
        ui.set_name(name, "new name");

        let row = ui.add(panel, Style::row().full_width().gap(SPACE_2).center_items());
        ui.add_text(row, text().width(64.0).fixed(), "In");
        let location = ui.add_text(row, text().text_color(MUTED).fill(), "");
        ui.set_name(location, "new location");
        let choose = button(ui, row, "new choose", "Choose…", false);

        ui.add_text(panel, caption(), "START FROM");
        let list = ui.add(panel, Style::column().full_width().gap(2.0).fill().clip());
        ui.set_name(list, "new choices");
        let mut choices = Vec::new();
        for choice in hub::choices() {
            let node = ui.add(
                list,
                Style::row()
                    .full_width()
                    .height(44.0)
                    .fixed()
                    .padding_x(SPACE_3)
                    .gap(SPACE_3)
                    .center_items()
                    .radius(RADIUS_MD)
                    .hover(ACCENT.alpha(16))
                    .clickable(),
            );
            let key = match &choice.start {
                hub::Start::Set(set) => format!("set {set}"),
                hub::Start::Template(name) => format!("template {name}"),
            };
            ui.set_name(node, format!("new {key}"));
            let glyph = match choice.start {
                hub::Start::Set(_) => "file-plus",
                hub::Start::Template(_) => "package",
            };
            icon(ui, node, glyph, ACCENT_300);
            let words = ui.add(node, Style::column().fill().gap(2.0));
            ui.add_text(words, text().weight(500), &choice.title);
            ui.add_text(words, caption(), &choice.about);
            choices.push((node, choice));
        }

        let footer = ui.add(panel, Style::row().full_width().gap(SPACE_2).center_items());
        let status = ui.add_text(footer, caption().text_color(WARNING).fill(), "");
        ui.set_name(status, "new status");
        let create = button(ui, footer, "new create", "Create", true);
        ui.focus(Some(name));
        let mut form = Form {
            name,
            location,
            choose,
            back,
            create,
            status,
            choices,
            chosen: 0,
        };
        Self::mark_chosen(ui, &mut form, 0);
        ui.set_text(form.location, &home_relative(&self.new_location()));
        form
    }

    fn mark_chosen(ui: &mut Ui, form: &mut Form, chosen: usize) {
        form.chosen = chosen;
        for (i, (node, _)) in form.choices.iter().enumerate() {
            let on = i == chosen;
            ui.restyle(*node, |s| {
                if on {
                    s.background(ACCENT_900).border(1.0, ACCENT)
                } else {
                    s.background(Color::TRANSPARENT).border(1.0, Color::TRANSPARENT)
                }
            });
        }
    }

    /// Where New puts a project unless told: beside the one opened last —
    /// not one of the engine's own examples — else beside the engine, else
    /// the home folder.
    fn new_location(&self) -> PathBuf {
        if let Some(chosen) = &self.location {
            return chosen.clone();
        }
        let inside_engine = |p: &Path| self.engine.as_ref().is_some_and(|e| p.starts_with(e));
        self.entries
            .iter()
            .filter(|e| e.problem.is_none() && !inside_engine(&e.path))
            .max_by_key(|e| e.opened)
            .and_then(|e| e.path.parent().map(Path::to_path_buf))
            .or_else(|| self.engine.as_ref().and_then(|e| e.parent().map(Path::to_path_buf)))
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// The folder New puts the project in, as chosen.
    pub fn set_location(&mut self, ui: &mut Ui, folder: PathBuf) {
        self.location = Some(folder);
        if let Mode::New(form) = &self.mode {
            ui.set_text(form.location, &home_relative(&self.new_location()));
        }
    }

    /// New, as the button opens it: for a test.
    pub fn show_new(&mut self, ui: &mut Ui) {
        self.mode = Mode::New(self.build_form(ui));
    }

    fn show_list(&mut self, ui: &mut Ui) {
        self.mode = Mode::List(Self::build_list(ui, self.panel, self.current.is_none()));
        self.fill(ui, "");
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
        match (read, &self.mode) {
            (Ok(entries), Mode::List(list)) => {
                self.entries = entries;
                let typed = ui.text(list.field).unwrap_or_default().to_string();
                self.fill(ui, &typed);
            }
            (Ok(entries), Mode::New(form)) => {
                self.entries = entries;
                ui.set_text(form.location, &home_relative(&self.new_location()));
            }
            (Err(problem), Mode::List(list)) => ui.set_text(list.status, &problem),
            (Err(_), Mode::New(_)) => {}
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
        match &self.mode {
            Mode::List(_) => self.list_event(ui, node, event),
            Mode::New(_) => self.form_event(ui, node, event),
        }
    }

    fn list_event(&mut self, ui: &mut Ui, node: NodeId, event: &Event) -> Option<Outcome> {
        let Mode::List(list) = &self.mode else { return None };
        let (field, add, new, close) = (list.field, list.add, list.new, list.close);
        let row = list
            .rows
            .iter()
            .position(|r| r.node == node || r.pin == node || r.forget == node);
        // At the start there is nothing behind to close it to.
        let closing = if self.current.is_some() {
            Outcome::Close
        } else {
            Outcome::Handled
        };
        match event {
            Event::Changed(text) if node == field => {
                let text = text.clone();
                self.fill(ui, &text);
                Some(Outcome::Handled)
            }
            Event::Submit(_) if node == field => Some(
                list.rows
                    .first()
                    .and_then(|r| self.entries[r.entry].scene.clone())
                    .map_or(Outcome::Handled, Outcome::Open),
            ),
            Event::Cancel if node == field => Some(closing),
            Event::Click { .. } if Some(node) == close || node == self.overlay => Some(closing),
            Event::Click { .. } if node == add => Some(Outcome::Add),
            Event::Click { .. } if node == new => {
                self.show_new(ui);
                Some(Outcome::Handled)
            }
            Event::Click { .. } if row.is_some() => {
                let row = &list.rows[row.expect("checked")];
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
            _ if node == field => Some(Outcome::Handled),
            _ => None,
        }
    }

    fn form_event(&mut self, ui: &mut Ui, node: NodeId, event: &Event) -> Option<Outcome> {
        let Mode::New(form) = &mut self.mode else { return None };
        let choice = form.choices.iter().position(|(n, _)| *n == node);
        match event {
            Event::Click { .. } if node == form.back => {
                self.show_list(ui);
                Some(Outcome::Handled)
            }
            Event::Cancel if node == form.name => {
                self.show_list(ui);
                Some(Outcome::Handled)
            }
            Event::Click { .. } if node == form.choose => Some(Outcome::ChooseLocation),
            Event::Click { .. } if choice.is_some() => {
                Self::mark_chosen(ui, form, choice.expect("checked"));
                Some(Outcome::Handled)
            }
            Event::Click { .. } if node == form.create => Some(self.create(ui)),
            Event::Submit(_) if node == form.name => Some(self.create(ui)),
            _ if node == form.name => Some(Outcome::Handled),
            Event::Click { .. } if node == self.overlay => Some(Outcome::Handled),
            _ => None,
        }
    }

    /// Make the project the form describes; its scene to open, or what is
    /// wrong said under the choices.
    fn create(&mut self, ui: &mut Ui) -> Outcome {
        let Mode::New(form) = &self.mode else {
            return Outcome::Handled;
        };
        let name = ui.text(form.name).unwrap_or_default().trim().to_string();
        let status = form.status;
        if name.is_empty() || name.contains('/') {
            ui.set_text(status, "A project needs a name, one folder's worth.");
            return Outcome::Handled;
        }
        let start = form.choices[form.chosen].1.start.clone();
        let folder = self.new_location().join(&name);
        ui.set_text(status, "Making it…");
        match hub::create(&folder, &start) {
            Ok(scene) => Outcome::Open(scene),
            Err(problem) => {
                ui.set_text(status, &problem);
                Outcome::Handled
            }
        }
    }

    /// Change the list, save it, and read it again.
    pub fn change(&mut self, ui: &mut Ui, change: impl FnOnce(&mut Known)) {
        let Some(dir) = &self.config else { return };
        if let Err(problem) = hub::update(dir, change) {
            if let Mode::List(list) = &self.mode {
                ui.set_text(list.status, &problem);
            }
            return;
        }
        self.reload();
    }

    /// The lines for what is typed: a project matches by name, folder, or
    /// either branch.
    fn fill(&mut self, ui: &mut Ui, typed: &str) {
        let Mode::List(list) = &self.mode else { return };
        let (status, list_node) = (list.status, list.list);
        let query = typed.trim().to_lowercase();
        ui.clear(list_node);
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
        let said = match (self.entries.len(), shown.len()) {
            (0, _) => "No projects yet: New makes one, Add finds the ones you have.".to_string(),
            (all, n) if n == all => format!("{all} projects"),
            (all, n) => format!("{n} of {all} projects"),
        };
        let others = self.entries.iter().filter(|e| self.other_engine(e)).count();
        let said = if others > 0 {
            format!("{said} · an engine in yellow is another checkout than this editor's")
        } else {
            said
        };
        ui.set_text(status, &said);
        let rows: Vec<Row> = shown.into_iter().map(|i| self.row(ui, list_node, i)).collect();
        if let Mode::List(list) = &mut self.mode {
            list.rows = rows;
        }
    }

    fn row(&self, ui: &mut Ui, list: NodeId, i: usize) -> Row {
        let e = &self.entries[i];
        let broken = e.problem.is_some();
        let current = self.current.as_deref() == Some(e.path.as_path());
        let node = ui.add(
            list,
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

fn title(ui: &mut Ui, parent: NodeId, words: &str) {
    ui.add_text(
        parent,
        Style::default()
            .text_size(16.0)
            .weight(500)
            .text_color(TEXT)
            .nowrap()
            .fill(),
        words,
    );
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
