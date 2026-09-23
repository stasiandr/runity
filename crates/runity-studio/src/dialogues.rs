//! Dialogues: `dialogues/*.ron` as a graph to read — a line a box, its
//! `next` and its choices arrows — on the same widget as the Animator
//! (`runity_ui::graph_view`). Choosing a line shows who says it, what, and
//! the answers with their conditions and what they set; what does not join
//! up is said first. The boxes place themselves from the start; the text is
//! written by hand or by an agent.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use runity::dialogue::{Dialogue, Flag};
use runity_editor::console::Level;
use runity_editor::Session;
use runity_ui::graph_view::{self, BOX_H, BOX_W};
use runity_ui::{Color, Event, NodeId, Style, Ui};

use crate::theme::*;

#[derive(Debug, Clone)]
enum Part {
    File(PathBuf),
    Line(String),
    Canvas,
}

pub struct Dialogues {
    pub root: NodeId,
    files_list: NodeId,
    canvas: NodeId,
    side: NodeId,
    parts: HashMap<NodeId, Part>,
    open: Option<(PathBuf, Dialogue)>,
    chosen: Option<String>,
    layout: BTreeMap<String, (usize, usize)>,
    pan: (f32, f32),
    listed: bool,
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
        ui.add_text(left, caption(), "DIALOGUES");
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
                .width(270.0)
                .full_height()
                .fixed()
                .gap(3.0)
                .clip(),
        );
        let mut parts = HashMap::new();
        parts.insert(canvas, Part::Canvas);
        Self {
            root,
            files_list,
            canvas,
            side,
            parts,
            open: None,
            chosen: None,
            layout: BTreeMap::new(),
            pan: (0.0, 0.0),
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

    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        if !self.listed {
            self.listed = true;
            self.list(ui, session);
        }
    }

    fn list(&mut self, ui: &mut Ui, session: &Session) {
        ui.clear(self.files_list);
        self.parts.retain(|_, p| !matches!(p, Part::File(_)));
        let mut paths: Vec<PathBuf> = session
            .project()
            .map(|p| p.root().join(runity::dialogue::DIR))
            .and_then(|d| std::fs::read_dir(d).ok())
            .map(|r| {
                r.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "ron"))
                    .collect()
            })
            .unwrap_or_default();
        paths.sort();
        if paths.is_empty() {
            ui.add_text(
                self.files_list,
                Style::default().text_size(11.5).text_color(MUTED),
                "No dialogues yet: dialogues/<name>.ron.",
            );
        }
        for path in paths {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
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
                self.open = Some((path, dialogue));
                self.chosen = None;
                self.pan = (0.0, 0.0);
                self.list(ui, session);
                self.show(ui);
            }
            Err(e) => session.say(Level::Error, e),
        }
    }

    fn place(&self, name: &str) -> (f32, f32) {
        let (column, row) = self.layout.get(name).copied().unwrap_or((0, 0));
        (
            24.0 + column as f32 * (BOX_W + 70.0),
            24.0 + row as f32 * (BOX_H + 50.0),
        )
    }

    fn show(&mut self, ui: &mut Ui) {
        self.parts
            .retain(|_, p| matches!(p, Part::File(_) | Part::Canvas));
        ui.clear(self.canvas);
        ui.clear(self.side);
        let Some((_, dialogue)) = self.open.clone() else {
            return;
        };
        let names: Vec<&str> = dialogue.lines.keys().map(String::as_str).collect();
        let edges = dialogue.edges();
        self.layout = graph_view::layout(&dialogue.start, &names, &edges);
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
            graph_view::arrow(
                ui,
                arrows,
                heads,
                &format!("dialogue edge {i}"),
                self.place(from),
                self.place(to),
                false,
                &others,
                NEUTRAL_500,
                2.0,
            );
        }
        for name in &names {
            let (x, y) = self.place(name);
            let on = self.chosen.as_deref() == Some(*name);
            let start = *name == dialogue.start;
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
                        if on { 2.0 } else { 1.0 },
                        if on { ACCENT } else { NEUTRAL_800 },
                    )
                    .clickable(),
            );
            ui.set_name(b, format!("line {name}"));
            let line = &dialogue.lines[*name];
            let glyph = if line.choices.is_empty() {
                "type"
            } else {
                "list-tree"
            };
            icon(ui, b, glyph, if start { ACCENT_200 } else { LABEL });
            ui.add_text(b, text().nowrap(), name);
            self.parts.insert(b, Part::Line(name.to_string()));
        }
        self.show_side(ui, &dialogue);
    }

    fn show_side(&mut self, ui: &mut Ui, dialogue: &Dialogue) {
        let side = self.side;
        for problem in dialogue.problems().iter().take(4) {
            ui.add_text(
                side,
                Style::default().text_size(11.5).text_color(ERROR),
                problem,
            );
        }
        let Some(name) = self.chosen.clone() else {
            ui.add_text(
                side,
                Style::default().text_size(12.0).text_color(MUTED),
                "Pick a line. The boxes place themselves from the start; drag the ground to pan.",
            );
            return;
        };
        let Some(line) = dialogue.lines.get(&name) else {
            return;
        };
        let small = |ui: &mut Ui, words: String, color| {
            ui.add_text(
                side,
                Style::default().text_size(12.0).text_color(color),
                &words,
            );
        };
        ui.add_text(side, caption(), &name.to_uppercase());
        if !line.speaker.is_empty() {
            small(ui, format!("{}:", line.speaker), LABEL);
        }
        small(ui, line.text.clone(), TEXT);
        if !line.set.is_empty() {
            small(ui, format!("sets {}", line.set.join(", ")), MUTED);
        }
        if !line.event.is_empty() {
            small(ui, format!("tells the game `{}`", line.event), MUTED);
        }
        if !line.next.is_empty() {
            small(ui, format!("→ {}", line.next), LABEL);
        }
        if !line.choices.is_empty() {
            ui.add_text(side, caption(), "ANSWERS");
        }
        for choice in &line.choices {
            let mut words = format!("“{}” → {}", choice.text, choice.to);
            if !choice.when.is_empty() {
                let when: Vec<String> = choice
                    .when
                    .iter()
                    .map(|f| match f {
                        Flag::Is(n) => n.clone(),
                        Flag::Not(n) => format!("not {n}"),
                    })
                    .collect();
                words.push_str(&format!(" if {}", when.join(", ")));
            }
            if !choice.set.is_empty() {
                words.push_str(&format!(", sets {}", choice.set.join(", ")));
            }
            if !choice.event.is_empty() {
                words.push_str(&format!(", tells `{}`", choice.event));
            }
            small(ui, words, TEXT);
        }
        if line.next.is_empty() && line.choices.is_empty() {
            small(ui, "the conversation ends here".into(), MUTED);
        }
    }

    pub fn event(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, event: &Event) {
        let Some(part) = self.parts.get(&node).cloned() else {
            return;
        };
        let click = matches!(event, Event::Click { .. });
        match part {
            Part::File(path) if click => self.open(ui, session, path),
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
            _ => {}
        }
    }
}
