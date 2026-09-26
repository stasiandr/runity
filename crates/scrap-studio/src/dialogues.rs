//! Dialogues: `dialogues/*.ron` as a graph to read, edit and play — a
//! line a box, its `next`, `else` and answers arrows — on the same widget
//! as the Animator (`scrap_ui::graph_view`). Card: docs/dialogue.md.
//!
//! Choosing a line shows its fields on the right, each written into the
//! file as it is submitted, and only where it changed
//! ([`scrap::dialogue_text`]): comments stay, the diff is the change.
//! The chips there make another line the next one; answers are edited as
//! the text they are written as. What changed since the last commit — an
//! agent's edit to look over — is listed with a way to take each piece
//! back, and its boxes are marked. Play talks the dialogue through right
//! here, without the game: the line being said is lit, the answers on
//! offer are buttons, and the flags and numbers are shown as they change.
//!
//! The boxes place themselves from the start; nothing about where they
//! stand is kept.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use scrap::dialogue::{named_twice, Change, Choice, Conversation, Dialogue, Line, State};
use scrap::dialogue_text::{choice_entry, conditions, numbers, write};
use scrap_editor::console::Level;
use scrap_editor::Session;
use scrap_ui::graph_view::{self, BOX_H, BOX_W};
use scrap_ui::{Color, Event, NodeId, Style, Ui};

use crate::theme::*;

#[derive(Debug, Clone)]
enum Part {
    File(PathBuf),
    Line(String),
    Canvas,
    New,
    Wide,
    /// A field of the chosen line, by name.
    Field(&'static str),
    /// The chosen line's answer, by its place, as its text.
    Answer(usize),
    AddAnswer,
    AddLine,
    Start,
    Delete,
    /// Make this line the chosen one's next.
    To(String),
    /// Take back one change since the last commit, by its place in the list.
    Revert(usize),
    Play,
    Stop,
    Next,
    /// Answer with the chosen line's answer at this place.
    Say(usize),
}

pub struct Dialogues {
    pub root: NodeId,
    files_list: NodeId,
    canvas: NodeId,
    side: NodeId,
    wide_button: NodeId,
    /// Over the whole window, as the Animator can be.
    pub wide: bool,
    parts: HashMap<NodeId, Part>,
    open: Option<(PathBuf, Dialogue)>,
    chosen: Option<String>,
    layout: BTreeMap<String, (usize, usize)>,
    pan: (f32, f32),
    listed: bool,
    /// The open dialogue as the last commit has it: what the changes are
    /// against. `None` outside git, or for a file not yet committed.
    head: Option<Dialogue>,
    /// A playthrough, when Play is on: where it is and what it remembers.
    play: Option<(Conversation, State)>,
    /// The project's words in its language, to say `@key`s in when
    /// playing.
    words: Option<scrap::strings::Strings>,
}

impl Dialogues {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let root = ui.add(
            parent,
            Style::row()
                .fill()
                .full_width()
                .gap(SPACE_2)
                .padding(SPACE_2),
        );
        ui.set_name(root, "dialogues");
        let left = ui.add(
            root,
            Style::column()
                .width(170.0)
                .full_height()
                .fixed()
                .gap(SPACE_2),
        );
        let head = ui.add(left, Style::row().full_width().center_items().gap(SPACE_1));
        ui.add_text(head, caption(), "DIALOGUES");
        spacer(ui, head);
        let new = icon_button(ui, head, "dialogues new", "file-plus", false);
        let wide_button = icon_button(ui, head, "dialogues wide", "expand", false);
        let files_list = ui.add(left, Style::column().fill().full_width().gap(2.0).clip());
        let canvas = ui.add(
            root,
            Style::column()
                .fill()
                .full_height()
                .radius(RADIUS_SM)
                .border(1.0, DIVIDER)
                .background(BG)
                .clip()
                .draggable(),
        );
        ui.set_name(canvas, "dialogues canvas");
        let side = ui.add(
            root,
            Style::column()
                .width(290.0)
                .full_height()
                .fixed()
                .gap(3.0)
                .clip(),
        );
        let mut parts = HashMap::new();
        parts.insert(canvas, Part::Canvas);
        parts.insert(new, Part::New);
        parts.insert(wide_button, Part::Wide);
        Self {
            root,
            files_list,
            canvas,
            side,
            wide_button,
            wide: false,
            parts,
            open: None,
            chosen: None,
            layout: BTreeMap::new(),
            pan: (0.0, 0.0),
            listed: false,
            head: None,
            play: None,
            words: None,
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

    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        if !self.listed {
            self.listed = true;
            self.list(ui, session);
        }
    }

    fn list(&mut self, ui: &mut Ui, session: &Session) {
        ui.clear(self.files_list);
        self.parts.retain(|_, p| !matches!(p, Part::File(_)));
        let paths: Vec<PathBuf> = session
            .project()
            .map(|p| p.files(scrap::layout::Kind::Dialogue))
            .unwrap_or_default();
        if paths.is_empty() {
            ui.add_text(
                self.files_list,
                Style::default().text_size(11.5).text_color(MUTED),
                "No dialogues yet: the page button makes one.",
            );
        }
        for path in paths {
            let name = stem(&path);
            let on = self.open.as_ref().is_some_and(|(p, _)| *p == path);
            let row = ui.add(
                self.files_list,
                Style::row()
                    .full_width()
                    .height(22.0)
                    .fixed()
                    .padding_x(SPACE_2)
                    .gap(SPACE_2)
                    .center_items()
                    .radius(RADIUS_SM)
                    .background(if on { ACCENT_HOVER } else { Color::TRANSPARENT })
                    .hover(HOVER)
                    .clickable(),
            );
            ui.set_name(row, format!("dialogue {name}"));
            icon(ui, row, "type", MUTED);
            ui.add_text(row, text(), &name);
            self.parts.insert(row, Part::File(path));
        }
    }

    fn open(&mut self, ui: &mut Ui, session: &mut Session, path: PathBuf) {
        match Dialogue::load(&path) {
            Ok(dialogue) => {
                self.head = scrap_editor::history::show(&path, "HEAD")
                    .ok()
                    .and_then(|t| scrap::ron::from_str::<Dialogue>(&t).ok())
                    .map(|mut d| {
                        d.name = dialogue.name.clone();
                        d
                    });
                self.words = session.project().and_then(|p| {
                    scrap::strings::Strings::load(p.strings_dir(), &p.manifest().game.language).ok()
                });
                self.open = Some((path, dialogue));
                self.chosen = None;
                self.play = None;
                self.pan = (0.0, 0.0);
                self.list(ui, session);
                self.show(ui);
            }
            Err(e) => session.say(Level::Error, e),
        }
    }

    fn dialogue(&self) -> Option<&Dialogue> {
        self.open.as_ref().map(|(_, d)| d)
    }

    fn place(&self, name: &str) -> (f32, f32) {
        let (column, row) = self.layout.get(name).copied().unwrap_or((0, 0));
        (
            24.0 + column as f32 * (BOX_W + 70.0),
            24.0 + row as f32 * (BOX_H + 50.0),
        )
    }

    /// What differs from the last commit's dialogue, if there is one.
    fn changes(&self, dialogue: &Dialogue) -> Vec<Change> {
        self.head
            .as_ref()
            .map(|head| scrap::dialogue::diff(head, dialogue))
            .unwrap_or_default()
    }

    /// Build the canvas and the side again from the dialogue.
    fn show(&mut self, ui: &mut Ui) {
        self.parts
            .retain(|_, p| matches!(p, Part::File(_) | Part::Canvas | Part::New | Part::Wide));
        ui.clear(self.canvas);
        ui.clear(self.side);
        let Some(dialogue) = self.dialogue().cloned() else {
            ui.add_text(
                self.side,
                Style::default().text_size(12.0).text_color(MUTED),
                "Open a dialogue on the left.",
            );
            return;
        };
        let names: Vec<&str> = dialogue.lines.keys().map(String::as_str).collect();
        let edges = dialogue.edges();
        self.layout = graph_view::layout(&dialogue.start, &names, &edges);
        let changes = self.changes(&dialogue);
        let saying = self
            .play
            .as_ref()
            .and_then(|(c, _)| c.at().map(str::to_string));
        let layer = |ui: &mut Ui| {
            ui.add(
                self.canvas,
                Style::default()
                    .absolute(self.pan.0, self.pan.1)
                    .size(4000.0, 4000.0),
            )
        };
        let arrows = layer(ui);
        let boxes = layer(ui);
        let heads = layer(ui);
        for (i, (from, to)) in edges.iter().enumerate() {
            if from == to || !dialogue.lines.contains_key(*to) {
                continue;
            }
            let others: Vec<(f32, f32)> = names
                .iter()
                .filter(|n| *n != from && *n != to)
                .map(|n| self.place(n))
                .collect();
            let back = from > to && edges.iter().any(|(f, t)| f == to && t == from);
            graph_view::arrow(
                ui,
                arrows,
                heads,
                &format!("dialogue edge {i}"),
                self.place(from),
                self.place(to),
                back,
                &others,
                NEUTRAL_500,
                2.0,
            );
        }
        for name in &names {
            let (x, y) = self.place(name);
            let on = self.chosen.as_deref() == Some(*name);
            let start = *name == dialogue.start;
            let live = saying.as_deref() == Some(*name);
            let mark = changes.iter().find_map(|c| match c {
                Change::LineAdded(n) if n == name => Some(ACCENT_400),
                Change::LineChanged(n, _) if n == name => Some(WARNING),
                _ => None,
            });
            let b = ui.add(
                boxes,
                Style::row()
                    .absolute(x, y)
                    .size(BOX_W, BOX_H)
                    .padding_x(SPACE_3)
                    .gap(SPACE_2)
                    .center_items()
                    .radius(RADIUS_MD)
                    .background(if start { ACCENT_800 } else { SURFACE })
                    .border(
                        if on || live || mark.is_some() {
                            2.0
                        } else {
                            1.0
                        },
                        if live {
                            WARNING
                        } else if on {
                            ACCENT
                        } else if let Some(mark) = mark {
                            mark
                        } else {
                            NEUTRAL_800
                        },
                    )
                    .clickable(),
            );
            ui.set_name(b, format!("line {name}"));
            let line = &dialogue.lines[*name];
            let glyph = if !line.when.is_empty() {
                "route"
            } else if line.choices.is_empty() {
                "type"
            } else {
                "list-tree"
            };
            icon(ui, b, glyph, if start { ACCENT_200 } else { LABEL });
            ui.add_text(b, text().nowrap(), name);
            self.parts.insert(b, Part::Line(name.to_string()));
        }
        self.show_side(ui, &dialogue, &changes);
    }

    fn small(&self, ui: &mut Ui, words: &str, color: Color) {
        ui.add_text(
            self.side,
            Style::default().text_size(12.0).text_color(color),
            words,
        );
    }

    /// A `@key` in the project's language, or the text as it is.
    fn say(&self, text: &str) -> String {
        match &self.words {
            Some(w) => w.resolve(text).to_string(),
            None => text.to_string(),
        }
    }

    fn show_side(&mut self, ui: &mut Ui, dialogue: &Dialogue, changes: &[Change]) {
        let side = self.side;
        // What will not work, first.
        let old = self
            .open
            .as_ref()
            .and_then(|(p, _)| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        for problem in named_twice(&old)
            .into_iter()
            .chain(dialogue.problems())
            .take(4)
        {
            self.small(ui, &problem, ERROR);
        }
        if !changes.is_empty() {
            ui.add_text(side, caption(), "CHANGES SINCE THE LAST COMMIT");
            for (i, change) in changes.iter().enumerate() {
                let row = ui.add(side, Style::row().full_width().gap(SPACE_2).center_items());
                ui.add_text(
                    row,
                    Style::default()
                        .text_size(11.5)
                        .text_color(TEXT)
                        .mono()
                        .fill(),
                    &change.to_string(),
                );
                let undo = button(
                    ui,
                    row,
                    &format!("dialogues undo change {i}"),
                    "Undo",
                    false,
                );
                self.parts.insert(undo, Part::Revert(i));
            }
        }
        let row = ui.add(side, Style::row().full_width().gap(SPACE_1));
        let add = button(ui, row, "dialogues add line", "+ Line", false);
        self.parts.insert(add, Part::AddLine);
        if self.play.is_some() {
            let stop = button(ui, row, "dialogues stop", "Stop", true);
            self.parts.insert(stop, Part::Stop);
            self.show_play(ui);
            return;
        }
        let play = button(ui, row, "dialogues play", "Play", false);
        self.parts.insert(play, Part::Play);
        let Some(name) = self.chosen.clone() else {
            self.small(
                ui,
                "Pick a line. Play talks it through from the start, or from the line picked.",
                MUTED,
            );
            return;
        };
        let Some(line) = dialogue.lines.get(&name).cloned() else {
            return;
        };
        self.field(ui, "name", "name", &name);
        self.field(ui, "speaker", "speaker", &line.speaker);
        self.field(ui, "text", "text", &line.text);
        if line.text.starts_with('@') {
            let said = self.say(&line.text);
            if said != line.text[1..] {
                self.small(ui, &format!("“{said}”"), MUTED);
            }
        }
        self.field(ui, "when", "when", &conditions(&line.when));
        if !line.when.is_empty() {
            self.field(ui, "else", "else", &line.otherwise);
        }
        self.field(ui, "next", "next", &line.next);
        self.field(ui, "sets", "set", &line.set.join(", "));
        self.field(ui, "adds", "add", &numbers(&line.add));
        self.field(ui, "puts", "put", &numbers(&line.put));
        self.field(ui, "event", "event", &line.event);
        self.small(
            ui,
            "when: Is(\"f\"), Not(\"f\"), Var(\"n\", Ge, 3) — Eq Ne Lt Le Gt Ge. adds: {\"n\": 1}",
            MUTED,
        );
        let row = ui.add(side, Style::row().full_width().gap(SPACE_1));
        let is_start = dialogue.start == name;
        let start = button(
            ui,
            row,
            "dialogues start",
            if is_start { "Start line" } else { "Make start" },
            is_start,
        );
        self.parts.insert(start, Part::Start);
        let delete = button(ui, row, "dialogues delete", "Delete", false);
        self.parts.insert(delete, Part::Delete);
        ui.add_text(side, caption(), "ANSWERS");
        for (i, choice) in line.choices.iter().enumerate() {
            let b = ui.add_field(
                side,
                field_style().full_width().mono().text_size(11.0),
                &choice_entry(choice),
            );
            ui.set_name(b, format!("dialogues answer {i}"));
            self.parts.insert(b, Part::Answer(i));
        }
        let more = button(ui, side, "dialogues add answer", "+ Answer", false);
        self.parts.insert(more, Part::AddAnswer);
        ui.add_text(side, caption(), "NEXT IS");
        let chips = ui.add(side, Style::row().full_width().gap(3.0).wrap());
        for other in dialogue.lines.keys().filter(|n| **n != name) {
            let chip = ui.add(
                chips,
                Style::row()
                    .height(20.0)
                    .padding_x(7.0)
                    .center()
                    .radius(6.0)
                    .border(1.0, NEUTRAL_800)
                    .hover(HOVER)
                    .clickable(),
            );
            ui.set_name(chip, format!("next {other}"));
            ui.add_text(
                chip,
                Style::default().text_size(11.5).text_color(LABEL),
                other,
            );
            self.parts.insert(chip, Part::To(other.clone()));
        }
    }

    /// The playthrough: the line being said, the answers on offer as
    /// buttons, and what is remembered.
    fn show_play(&mut self, ui: &mut Ui) {
        let Some((talk, state)) = self.play.clone() else {
            return;
        };
        ui.add_text(self.side, caption(), "PLAYING");
        match talk.line() {
            None => self.small(ui, "The conversation is over.", MUTED),
            Some(line) => {
                if !line.speaker.is_empty() {
                    self.small(ui, &format!("{}:", self.say(&line.speaker)), LABEL);
                }
                self.small(ui, &self.say(&line.text), TEXT);
                let offered: Vec<(usize, Choice)> =
                    talk.choices(&state).map(|(i, c)| (i, c.clone())).collect();
                if line.choices.is_empty() {
                    let next = button(ui, self.side, "dialogues next", "Next", true);
                    self.parts.insert(next, Part::Next);
                } else if offered.is_empty() {
                    self.small(ui, "No answer is on offer: stuck here.", ERROR);
                }
                for (i, choice) in offered {
                    let b = button(
                        ui,
                        self.side,
                        &format!("dialogues say {i}"),
                        &self.say(&choice.text),
                        false,
                    );
                    self.parts.insert(b, Part::Say(i));
                }
            }
        }
        let flags: Vec<&str> = state.flags.iter().map(String::as_str).collect();
        let vars: Vec<String> = state
            .vars
            .iter()
            .map(|(k, v)| format!("{k} = {v}"))
            .collect();
        self.small(
            ui,
            &format!(
                "flags: {}",
                if flags.is_empty() {
                    "—".into()
                } else {
                    flags.join(", ")
                }
            ),
            MUTED,
        );
        if !vars.is_empty() {
            self.small(ui, &format!("numbers: {}", vars.join(", ")), MUTED);
        }
        let trail = talk.trail().join(" → ");
        self.small(ui, &format!("said: {trail}"), MUTED);
    }

    fn field(&mut self, ui: &mut Ui, label: &str, key: &'static str, value: &str) {
        let r = ui.add(
            self.side,
            Style::row().full_width().gap(SPACE_2).center_items(),
        );
        ui.add_text(
            r,
            Style::default()
                .width(60.0)
                .fixed()
                .text_size(12.0)
                .text_color(LABEL)
                .nowrap(),
            label,
        );
        let b = ui.add_field(r, field_style().fill().mono().text_size(11.5), value);
        ui.set_name(b, format!("dialogues field {key}"));
        self.parts.insert(b, Part::Field(key));
    }

    /// Change the dialogue and write it where it changed.
    fn edit(&mut self, session: &mut Session, change: impl FnOnce(&mut Dialogue)) {
        let Some((path, dialogue)) = self.open.as_mut() else {
            return;
        };
        change(dialogue);
        let old = std::fs::read_to_string(&*path).unwrap_or_default();
        let text = write(&old, dialogue);
        if text != old {
            if let Err(e) = std::fs::write(&*path, text) {
                session.say(Level::Error, format!("{}: {e}", path.display()));
            }
        }
    }

    /// A field of the chosen line, typed.
    fn set(&mut self, session: &mut Session, key: &str, value: &str) {
        let Some(name) = self.chosen.clone() else {
            return;
        };
        if key == "name" {
            let mut renamed = self.dialogue().cloned().unwrap_or_default();
            match scrap::dialogue::rename(&mut renamed, &name, value) {
                Ok(()) => {
                    let to = value.to_string();
                    self.edit(session, |d| *d = renamed);
                    self.chosen = Some(to);
                }
                Err(e) => session.say(Level::Error, e),
            }
            return;
        }
        let Some(mut line) = self.dialogue().and_then(|d| d.lines.get(&name)).cloned() else {
            return;
        };
        let list = |v: &str| -> Vec<String> {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        };
        let map = |v: &str| -> Result<BTreeMap<String, i64>, String> {
            if v.trim().is_empty() {
                return Ok(BTreeMap::new());
            }
            let text = if v.trim_start().starts_with('{') {
                v.to_string()
            } else {
                format!("{{{v}}}")
            };
            scrap::ron::from_str(&text).map_err(|e| format!("{key}: {e}"))
        };
        let words = |field: &mut String| {
            *field = value.to_string();
            Ok(())
        };
        let result = match key {
            "speaker" => words(&mut line.speaker),
            "text" => words(&mut line.text),
            "next" => words(&mut line.next),
            "else" => words(&mut line.otherwise),
            "event" => words(&mut line.event),
            "set" => {
                line.set = list(value);
                Ok(())
            }
            "add" => map(value).map(|m| line.add = m),
            "put" => map(value).map(|m| line.put = m),
            "when" => {
                let text = if value.starts_with('[') {
                    value.to_string()
                } else {
                    format!("[{value}]")
                };
                scrap::ron::from_str(&text)
                    .map(|w| line.when = w)
                    .map_err(|e| format!("when: {e}"))
            }
            _ => Ok(()),
        };
        match result {
            Ok(()) => self.edit(session, |d| {
                d.lines.insert(name, line);
            }),
            Err(e) => session.say(Level::Error, e),
        }
    }

    fn step_play(&mut self, answer: Option<usize>) {
        if let Some((talk, state)) = self.play.as_mut() {
            match answer {
                Some(i) => {
                    talk.choose(i, state);
                }
                None => talk.next(state),
            }
            talk.events();
        }
    }

    pub fn event(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, event: &Event) {
        let Some(part) = self.parts.get(&node).cloned() else {
            return;
        };
        let click = matches!(event, Event::Click { .. });
        match part {
            Part::File(path) if click => self.open(ui, session, path),
            Part::New if click => self.new_file(ui, session),
            Part::Wide if click => {
                self.wide = !self.wide;
                set_icon_button(ui, self.wide_button, "expand", self.wide, true);
            }
            Part::Line(name) if click => {
                self.chosen = Some(name);
                self.show(ui);
            }
            Part::Canvas => {
                if let Event::Drag { dx, dy, .. } = event {
                    self.pan.0 += dx;
                    self.pan.1 += dy;
                    self.show(ui);
                }
            }
            Part::Field(key) => {
                if let Event::Submit(value) = event {
                    self.set(session, key, value.trim());
                    self.show(ui);
                }
            }
            Part::Answer(i) => {
                if let Event::Submit(value) = event {
                    let Some(name) = self.chosen.clone() else {
                        return;
                    };
                    let value = value.trim();
                    if value.is_empty() {
                        self.edit(session, |d| {
                            if let Some(l) = d.lines.get_mut(&name) {
                                if i < l.choices.len() {
                                    l.choices.remove(i);
                                }
                            }
                        });
                    } else {
                        match scrap::ron::from_str::<Choice>(value) {
                            Ok(choice) => self.edit(session, |d| {
                                if let Some(c) =
                                    d.lines.get_mut(&name).and_then(|l| l.choices.get_mut(i))
                                {
                                    *c = choice;
                                }
                            }),
                            Err(e) => session.say(Level::Error, format!("answer: {e}")),
                        }
                    }
                    self.show(ui);
                }
            }
            Part::AddAnswer if click => {
                let Some(name) = self.chosen.clone() else {
                    return;
                };
                // The first answer takes over where the line went on to:
                // the choices win over a next.
                self.edit(session, |d| {
                    let start = d.start.clone();
                    if let Some(l) = d.lines.get_mut(&name) {
                        let to = if l.next.is_empty() {
                            l.choices.last().map(|c| c.to.clone()).unwrap_or(start)
                        } else {
                            std::mem::take(&mut l.next)
                        };
                        l.choices.push(Choice {
                            text: String::new(),
                            to,
                            ..Choice::default()
                        });
                    }
                });
                self.show(ui);
            }
            Part::AddLine if click => {
                let Some(dialogue) = self.dialogue() else {
                    return;
                };
                let mut name = "line".to_string();
                let mut n = 2;
                while dialogue.lines.contains_key(&name) {
                    name = format!("line_{n}");
                    n += 1;
                }
                let first = dialogue.lines.is_empty();
                let from = self.chosen.clone();
                let new = name.clone();
                self.edit(session, |d| {
                    d.lines.insert(new.clone(), Line::default());
                    if first {
                        d.start = new.clone();
                    }
                    // Said after the line picked, when it goes nowhere yet.
                    if let Some(l) = from.and_then(|f| d.lines.get_mut(&f)) {
                        if l.next.is_empty() && l.choices.is_empty() {
                            l.next = new;
                        }
                    }
                });
                self.chosen = Some(name);
                self.show(ui);
            }
            Part::Start if click => {
                if let Some(name) = self.chosen.clone() {
                    self.edit(session, |d| d.start = name);
                    self.show(ui);
                }
            }
            Part::Delete if click => {
                if let Some(name) = self.chosen.take() {
                    self.edit(session, |d| {
                        d.lines.remove(&name);
                    });
                    self.show(ui);
                }
            }
            Part::To(to) if click => {
                if let Some(name) = self.chosen.clone() {
                    self.edit(session, |d| {
                        if let Some(l) = d.lines.get_mut(&name) {
                            l.next = to;
                        }
                    });
                    self.show(ui);
                }
            }
            Part::Revert(i) if click => {
                let Some(dialogue) = self.dialogue().cloned() else {
                    return;
                };
                if let Some(change) = self.changes(&dialogue).into_iter().nth(i) {
                    self.edit(session, |d| change.revert(d));
                }
                self.show(ui);
            }
            Part::Play if click => {
                if let Some(dialogue) = self.dialogue().cloned() {
                    let mut state = State::default();
                    let talk = match self.chosen.clone() {
                        Some(line) => Conversation::begin_at(dialogue, &line, &mut state),
                        None => Conversation::begin(dialogue, &mut state),
                    };
                    self.play = Some((talk, state));
                    self.show(ui);
                }
            }
            Part::Stop if click => {
                self.play = None;
                self.show(ui);
            }
            Part::Next if click => {
                self.step_play(None);
                self.show(ui);
            }
            Part::Say(i) if click => {
                self.step_play(Some(i));
                self.show(ui);
            }
            _ => {}
        }
    }

    fn new_file(&mut self, ui: &mut Ui, session: &mut Session) {
        let Some(project) = session.project() else {
            return;
        };
        let mut path = project.new_file(scrap::layout::Kind::Dialogue, "dialogue");
        let mut n = 2;
        while path.exists() {
            path = project.new_file(scrap::layout::Kind::Dialogue, &format!("dialogue_{n}"));
            n += 1;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text = "(\n    start: \"hello\",\n    lines: {\n        \"hello\": (speaker: \"\", text: \"\"),\n    },\n)\n";
        match std::fs::write(&path, text) {
            Ok(()) => {
                self.listed = false;
                self.open(ui, session, path);
            }
            Err(e) => session.say(Level::Error, format!("{}: {e}", path.display())),
        }
    }
}

fn stem(path: &std::path::Path) -> String {
    scrap::layout::name_of(path)
}
