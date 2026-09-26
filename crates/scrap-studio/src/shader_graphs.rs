//! Shader Graph: `shaders/*.graph.ron`, `*.vfx.ron`, `*.post.ron` and
//! `*.subgraph.ron`
//! as boxes and arrows — each node a box, each read of one node by another
//! an arrow into the reader, what the graph sets a box of its own at the
//! right — on the same widget as the Animator and the Dialogues
//! (`scrap_ui::graph_view`). Card: docs/shadergraph.md.
//!
//! The boxes place themselves from what the graph sets; nothing about where
//! they stand is kept. Choosing a node shows its inputs on the right, each
//! written into the file as it is submitted and only there
//! ([`scrap::shader_graph::text`]): comments stay, the diff is the change.
//! A node is added by writing it, and taken out with its line; one node is
//! read by another by dragging its box onto the other, which then asks as
//! which of its inputs — an edit of the reader's input, never a place. The
//! loading, checking and editing are the engine's
//! ([`scrap_editor::shader_graphs`]), the MCP server's `shader_graph` tools
//! as well. Whether the
//! graph builds is said on the right in the compiler's words, and the node
//! they name is marked; a material's graph is shown on a ball, lit, and the
//! chosen node's value on another — Shader Graph's main preview and a
//! node's.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use scrap::shader_graph::Node;
use scrap_editor::console::Level;
use scrap_editor::shader_graphs::{self as graphs, written, Checked, Doc, Edit, Kind};
use scrap_editor::Session;
use scrap_ui::graph_view::{self, BOX_H, BOX_W};
use scrap_ui::{Color, Event, ImageId, NodeId, Style, Ui};

use crate::theme::*;

/// The picture of the whole graph on a ball, and of the chosen node.
pub const GRAPH_PREVIEW: ImageId = ImageId(20);
pub const NODE_PREVIEW: ImageId = ImageId(21);
const PREVIEW_SIZE: u32 = 160;

#[derive(Debug, Clone)]
enum Part {
    File(PathBuf),
    Node(String),
    Canvas,
    Wide,
    /// An input of the chosen node, by its field.
    Input(&'static str),
    /// A new node, as `"name": Kind(…)`.
    Add,
    Delete,
    /// What the graph sets, by its part: a box to drop a node on.
    Output(String),
    /// The dragged node read by the one it was dropped on, as this input.
    As(&'static str),
    /// Not connecting after all.
    NotAs,
}

pub struct ShaderGraphs {
    pub root: NodeId,
    files_list: NodeId,
    canvas: NodeId,
    side: NodeId,
    wide_button: NodeId,
    /// Over the whole window, as the Animator can be.
    pub wide: bool,
    parts: HashMap<NodeId, Part>,
    open: Option<(PathBuf, Doc)>,
    chosen: Option<String>,
    layout: BTreeMap<String, (usize, usize)>,
    columns: usize,
    pan: (f32, f32),
    listed: bool,
    /// Whether it builds, and its warnings.
    checked: Checked,
    /// A node dragged onto another, waiting to be told as which input:
    /// (the one read, the reader).
    connecting: Option<(String, String)>,
    /// Pictures for the renderer: the previews.
    images: Vec<(ImageId, u32, Vec<u8>)>,
}

impl ShaderGraphs {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let root = ui.add(parent, Style::row().fill().full_width().gap(SPACE_2).padding(SPACE_2));
        ui.set_name(root, "shader graphs");
        let left = ui.add(root, Style::column().width(170.0).full_height().fixed().gap(SPACE_2));
        let head = ui.add(left, Style::row().full_width().center_items().gap(SPACE_1));
        ui.add_text(head, caption(), "SHADER GRAPH");
        spacer(ui, head);
        let wide_button = icon_button(ui, head, "shader graphs wide", "expand", false);
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
        ui.set_name(canvas, "shader graphs canvas");
        let side = ui.add(root, Style::column().width(290.0).full_height().fixed().gap(3.0).clip());
        let mut parts = HashMap::new();
        parts.insert(canvas, Part::Canvas);
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
            columns: 1,
            pan: (0.0, 0.0),
            listed: false,
            checked: Checked { built: Ok(()), problems: Vec::new() },
            connecting: None,
            images: Vec::new(),
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

    /// The previews made since the last call, for the renderer.
    pub fn take_images(&mut self) -> Vec<(ImageId, u32, Vec<u8>)> {
        std::mem::take(&mut self.images)
    }

    fn list(&mut self, ui: &mut Ui, session: &Session) {
        ui.clear(self.files_list);
        self.parts.retain(|_, p| !matches!(p, Part::File(_)));
        let paths = session.project().map(|p| graphs::list(p.root())).unwrap_or_default();
        if paths.is_empty() {
            ui.add_text(
                self.files_list,
                Style::default().text_size(11.5).text_color(MUTED),
                "No graphs yet: shaders/<name>.graph.ron is one (docs/shadergraph.md).",
            );
        }
        for path in paths {
            let name = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
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
            ui.set_name(row, format!("graph {name}"));
            let glyph = match Kind::of(&path) {
                Some(Kind::Effect) => "sparkles",
                Some(Kind::Subgraph) => "package",
                Some(Kind::Fullscreen) => "image",
                _ => "palette",
            };
            icon(ui, row, glyph, MUTED);
            ui.add_text(row, text().nowrap(), &name);
            self.parts.insert(row, Part::File(path));
        }
    }

    /// Open (or read again) the file at `path`: what it says, whether it
    /// builds, and its previews.
    fn open(&mut self, ui: &mut Ui, session: &mut Session, path: PathBuf) {
        match Doc::load(&path) {
            Ok(doc) => {
                let same = self.open.as_ref().is_some_and(|(p, _)| *p == path);
                if !same {
                    self.chosen = None;
                    self.pan = (0.0, 0.0);
                }
                self.connecting = None;
                self.checked = graphs::check(&path, &doc);
                self.open = Some((path, doc));
                self.preview(session);
                self.list(ui, session);
                self.show(ui);
            }
            Err(e) => session.say(Level::Error, format!("{}: {e}", path.display())),
        }
    }

    /// The pictures: the whole material on a ball, and the chosen node.
    fn preview(&mut self, session: &mut Session) {
        let Some((path, Doc::Material(_))) = &self.open else {
            return;
        };
        if self.checked.built.is_err() {
            return;
        }
        let path = path.clone();
        if let Ok(pixels) = session.graph_preview(&path, None, PREVIEW_SIZE) {
            self.images.push((GRAPH_PREVIEW, PREVIEW_SIZE, pixels));
        }
        if let Some(node) = self.chosen.clone() {
            match session.graph_preview(&path, Some(&node), PREVIEW_SIZE) {
                Ok(pixels) => self.images.push((NODE_PREVIEW, PREVIEW_SIZE, pixels)),
                Err(e) => session.say(Level::Warning, e.to_string()),
            }
        }
    }

    fn place(&self, name: &str) -> (f32, f32) {
        let (column, row) = self.layout.get(name).copied().unwrap_or((0, 0));
        // What the graph sets on the right, what it reads from to the left.
        let column = self.columns.saturating_sub(column + 1);
        (24.0 + column as f32 * (BOX_W + 40.0), 24.0 + row as f32 * (BOX_H + 30.0))
    }

    /// Build the canvas and the side again from the graph.
    fn show(&mut self, ui: &mut Ui) {
        self.parts.retain(|_, p| matches!(p, Part::File(_) | Part::Canvas | Part::Wide));
        ui.clear(self.canvas);
        ui.clear(self.side);
        let Some((_, doc)) = self.open.clone() else {
            ui.add_text(
                self.side,
                Style::default().text_size(12.0).text_color(MUTED),
                "Open a graph on the left.",
            );
            return;
        };
        let nodes = doc.nodes();
        let outputs = doc.outputs();
        // Data runs from what is read to what reads it.
        let edges = doc.edges();
        let mut names: Vec<&str> = outputs.iter().map(|(o, _)| *o).collect();
        names.extend(nodes.keys().map(String::as_str));
        // Laid out from what the graph sets, back along the reads.
        let back: Vec<(&str, &str)> = edges.iter().map(|(f, t)| (t.as_str(), f.as_str())).collect();
        self.layout = graph_view::layout(outputs[0].0, &names, &back);
        // The other outputs beside the first.
        for (i, (o, _)) in outputs.iter().enumerate().skip(1) {
            self.layout.insert(o.to_string(), (0, i));
        }
        self.columns = self.layout.values().map(|(c, _)| c + 1).max().unwrap_or(1);
        let blamed = self.checked.blamed();
        let layer = |ui: &mut Ui| ui.add(self.canvas, Style::default().absolute(self.pan.0, self.pan.1).size(4000.0, 4000.0));
        let arrows = layer(ui);
        let boxes = layer(ui);
        let heads = layer(ui);
        for (i, (from, to)) in edges.iter().enumerate() {
            if from == to {
                continue;
            }
            let others: Vec<(f32, f32)> =
                names.iter().filter(|n| **n != from.as_str() && **n != to.as_str()).map(|n| self.place(n)).collect();
            let on = self.chosen.as_deref().is_some_and(|c| c == from || c == to);
            graph_view::arrow(
                ui,
                arrows,
                heads,
                &format!("graph edge {i}"),
                self.place(from),
                self.place(to),
                false,
                &others,
                if on { ACCENT } else { NEUTRAL_500 },
                2.0,
            );
        }
        for name in &names {
            let (x, y) = self.place(name);
            let output = outputs.iter().any(|(o, _)| o == name);
            let on = self.chosen.as_deref() == Some(*name)
                || self.connecting.as_ref().is_some_and(|(f, t)| f == name || t == name);
            let wrong = blamed.as_deref() == Some(*name);
            let b = ui.add(
                boxes,
                Style::column()
                    .absolute(x, y)
                    .size(BOX_W, BOX_H)
                    .padding_x(SPACE_3)
                    .radius(RADIUS_MD)
                    .background(if output { ACCENT_800 } else { SURFACE })
                    .border(
                        if on || wrong { 2.0 } else { 1.0 },
                        if wrong {
                            ERROR
                        } else if on {
                            ACCENT
                        } else {
                            NEUTRAL_800
                        },
                    )
                    .clickable()
                    .draggable(),
            );
            ui.set_name(b, format!("graph node {name}"));
            ui.add_text(b, text().nowrap().text_size(12.0), name);
            let kind = nodes.get(*name).map(|n| match n {
                Node::Subgraph { name: sub, .. } => format!("Subgraph {sub}"),
                n => n.kind().to_string(),
            });
            ui.add_text(
                b,
                Style::default().text_size(10.0).text_color(if output { ACCENT_200 } else { MUTED }).nowrap(),
                kind.as_deref().unwrap_or("what the graph sets"),
            );
            self.parts.insert(
                b,
                if output {
                    Part::Output(name.to_string())
                } else {
                    Part::Node(name.to_string())
                },
            );
        }
        self.show_side(ui, &doc);
    }

    fn small(&self, ui: &mut Ui, words: &str, color: Color) {
        ui.add_text(self.side, Style::default().text_size(12.0).text_color(color), words);
    }

    fn show_side(&mut self, ui: &mut Ui, doc: &Doc) {
        let side = self.side;
        match &self.checked.built {
            Ok(()) => self.small(ui, "Builds.", SUCCESS),
            Err(e) => self.small(ui, e, ERROR),
        }
        for p in self.checked.problems.clone() {
            self.small(ui, &p, WARNING);
        }
        // A node dropped on another: as which of its inputs.
        if let Some((from, to)) = self.connecting.clone() {
            self.small(ui, &format!("`{to}` reads `{from}` as:"), LABEL);
            let row = ui.add(side, Style::row().full_width().gap(SPACE_1).wrap());
            for field in graphs::fields_of(doc, &to) {
                let b = ui.add(
                    row,
                    Style::row()
                        .padding_x(SPACE_2)
                        .height(22.0)
                        .center_items()
                        .radius(RADIUS_SM)
                        .background(SURFACE)
                        .border(1.0, NEUTRAL_800)
                        .hover(HOVER)
                        .clickable(),
                );
                ui.set_name(b, format!("graph connect as {field}"));
                ui.add_text(b, text().text_size(12.0), field);
                self.parts.insert(b, Part::As(field));
            }
            let b = ui.add(row, Style::row().padding_x(SPACE_2).height(22.0).center_items().radius(RADIUS_SM).hover(HOVER).clickable());
            ui.set_name(b, "graph connect cancel");
            ui.add_text(b, Style::default().text_size(12.0).text_color(MUTED), "Cancel");
            self.parts.insert(b, Part::NotAs);
        }
        if matches!(doc, Doc::Material(_)) && self.checked.built.is_ok() {
            let row = ui.add(side, Style::row().full_width().gap(SPACE_2));
            let size = 130.0;
            let whole = ui.add_image(row, Style::default().size(size, size).radius(RADIUS_MD), GRAPH_PREVIEW);
            ui.set_name(whole, "graph preview");
            if self.chosen.is_some() {
                let one = ui.add_image(row, Style::default().size(size, size).radius(RADIUS_MD), NODE_PREVIEW);
                ui.set_name(one, "node preview");
            }
        }
        if let Some(name) = self.chosen.clone() {
            if let Some(node) = doc.nodes().get(&name).cloned() {
                self.small(ui, &format!("{name}: {}", node.kind()), LABEL);
                if let Node::Subgraph { name: sub, inputs } = &node {
                    self.small(ui, &format!("calls shaders/{sub}.subgraph.ron"), MUTED);
                    let given = inputs.iter().map(|(k, v)| format!("\"{k}\": {}", written(v))).collect::<Vec<_>>().join(", ");
                    let r = ui.add(side, Style::row().full_width().gap(SPACE_2).center_items());
                    ui.add_text(r, Style::default().width(80.0).fixed().text_size(12.0).text_color(LABEL).nowrap(), "inputs");
                    let f = ui.add_field(r, field_style().fill().mono().text_size(11.5), &format!("{{ {given} }}"));
                    ui.set_name(f, "graph input inputs");
                    self.parts.insert(f, Part::Input("inputs"));
                }
                for (field, input) in node.inputs() {
                    if field == "in" {
                        continue;
                    }
                    let r = ui.add(side, Style::row().full_width().gap(SPACE_2).center_items());
                    ui.add_text(
                        r,
                        Style::default().width(80.0).fixed().text_size(12.0).text_color(LABEL).nowrap(),
                        field,
                    );
                    let f = ui.add_field(r, field_style().fill().mono().text_size(11.5), &written(input));
                    ui.set_name(f, format!("graph input {field}"));
                    self.parts.insert(f, Part::Input(field));
                }
                let d = ui.add(
                    side,
                    Style::row().padding_x(SPACE_2).height(22.0).fixed().center_items().radius(RADIUS_SM).hover(HOVER).clickable(),
                );
                ui.set_name(d, "graph delete node");
                ui.add_text(d, Style::default().text_size(12.0).text_color(ERROR), "Take the node out");
                self.parts.insert(d, Part::Delete);
            }
        }
        self.small(ui, "A new node, as it is written:", MUTED);
        let f = ui.add_field(side, field_style().full_width().mono().text_size(11.5), "\"name\": Multiply(a: \"uv.x\", b: 2.0)");
        ui.set_name(f, "graph add node");
        self.parts.insert(f, Part::Add);
    }

    /// Make `edit` to the open file, where it goes; then read it again.
    fn edit(&mut self, ui: &mut Ui, session: &mut Session, edit: Edit) {
        let Some((path, _)) = self.open.clone() else {
            return;
        };
        if let Err(e) = graphs::apply(&path, &edit) {
            session.say(Level::Error, e);
        }
        self.open(ui, session, path);
    }

    pub fn event(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, event: &Event) {
        let Some(part) = self.parts.get(&node).cloned() else {
            return;
        };
        let click = matches!(event, Event::Click { .. });
        match part {
            Part::File(path) if click => self.open(ui, session, path),
            Part::Wide if click => {
                self.wide = !self.wide;
                set_icon_button(ui, self.wide_button, "expand", self.wide, true);
            }
            Part::Node(name) if click => {
                self.chosen = Some(name);
                self.connecting = None;
                self.preview(session);
                self.show(ui);
            }
            // Dragged onto another box: that one reads it, as an input the
            // side asks for.
            Part::Node(name) => {
                if let Event::DragEnd { over: Some(over) } = event {
                    let to = match self.parts.get(over) {
                        Some(Part::Node(to) | Part::Output(to)) if *to != name => to.clone(),
                        _ => return,
                    };
                    let Some((_, doc)) = &self.open else { return };
                    if graphs::fields_of(doc, &to).is_empty() {
                        session.say(Level::Warning, format!("`{to}` reads nothing another node gives: its file says what it is"));
                        return;
                    }
                    self.connecting = Some((name, to));
                    self.show(ui);
                }
            }
            Part::As(field) if click => {
                let Some((from, to)) = self.connecting.take() else { return };
                let Some((_, doc)) = &self.open else { return };
                let edit = Edit::connect(doc, &from, &to, field);
                self.edit(ui, session, edit);
            }
            Part::NotAs if click => {
                self.connecting = None;
                self.show(ui);
            }
            Part::Canvas => {
                if let Event::Drag { dx, dy, .. } = event {
                    self.pan.0 += dx;
                    self.pan.1 += dy;
                    self.show(ui);
                }
            }
            Part::Input(field) => {
                if let (Event::Submit(value), Some(node)) = (event, self.chosen.clone()) {
                    let edit = Edit::Set { node, field: field.into(), value: value.trim().into() };
                    self.edit(ui, session, edit);
                }
            }
            Part::Add => {
                if let Event::Submit(value) = event {
                    let Some((name, kind)) = value.split_once(':') else {
                        session.say(Level::Error, "a new node is `\"name\": Kind(…)`");
                        return;
                    };
                    let node = name.trim().trim_matches('"').to_string();
                    self.edit(ui, session, Edit::Add { node: node.clone(), kind: kind.trim().into() });
                    if self.open.as_ref().is_some_and(|(_, d)| d.nodes().contains_key(&node)) {
                        self.chosen = Some(node);
                        self.show(ui);
                    }
                }
            }
            Part::Delete if click => {
                if let Some(node) = self.chosen.take() {
                    self.edit(ui, session, Edit::Remove { node });
                }
            }
            _ => {}
        }
    }
}
