//! Shader Graph: `shaders/*.graph.ron`, `*.vfx.ron` and `*.subgraph.ron`
//! as boxes and arrows — each node a box, each read of one node by another
//! an arrow into the reader, what the graph sets a box of its own at the
//! right — on the same widget as the Animator and the Dialogues
//! (`scrap_ui::graph_view`). Card: docs/shadergraph.md.
//!
//! The boxes place themselves from what the graph sets; nothing about where
//! they stand is kept. Choosing a node shows its inputs on the right, each
//! written into the file as it is submitted and only there
//! ([`scrap::shader_graph::text`]): comments stay, the diff is the change.
//! A node is added by writing it, and taken out with its line. Whether the
//! graph builds is said on the right in the compiler's words, and the node
//! they name is marked; a material's graph is shown on a ball, lit, and the
//! chosen node's value on another — Shader Graph's main preview and a
//! node's.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use scrap::shader_graph::{effect::EffectGraph, subgraph::SubGraph, Input, Node, ShaderGraph};
use scrap_editor::console::Level;
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
}

/// What a file is.
#[derive(Debug, Clone)]
enum Doc {
    Surface(ShaderGraph),
    Effect(EffectGraph),
    Sub(SubGraph),
}

impl Doc {
    fn nodes(&self) -> &BTreeMap<String, Node> {
        match self {
            Doc::Surface(g) => &g.nodes,
            Doc::Effect(g) => &g.nodes,
            Doc::Sub(g) => &g.nodes,
        }
    }

    /// The boxes of what the graph sets, each with what it reads.
    fn outputs(&self) -> Vec<(&'static str, Vec<&Input>)> {
        fn some<'a>(v: Vec<&'a Option<Input>>) -> Vec<&'a Input> {
            v.into_iter().flatten().collect()
        }
        match self {
            Doc::Surface(g) => {
                let s = &g.surface;
                let mut out = vec![(
                    "surface",
                    some(vec![&s.albedo, &s.alpha, &s.metallic, &s.smoothness, &s.normal, &s.emission, &s.clip]),
                )];
                let v = some(vec![&g.vertex.position, &g.vertex.normal]);
                if !v.is_empty() {
                    out.push(("vertex", v));
                }
                out
            }
            Doc::Effect(g) => vec![
                ("spawn", some(vec![&g.spawn.position, &g.spawn.velocity, &g.spawn.life])),
                ("update", some(vec![&g.update.velocity, &g.update.position])),
                ("output", some(vec![&g.output.color, &g.output.alpha, &g.output.size])),
            ],
            Doc::Sub(g) => vec![("outputs", g.outputs.values().collect())],
        }
    }
}

/// A file's kind, by its name.
fn kind_of(path: &std::path::Path) -> Option<&'static str> {
    let f = path.file_name()?.to_str()?;
    if f.ends_with(".subgraph.ron") {
        Some("subgraph")
    } else if f.ends_with(".vfx.ron") {
        Some("effect")
    } else if f.ends_with(".graph.ron") {
        Some("material")
    } else {
        None
    }
}

fn load(path: &std::path::Path) -> Result<Doc, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    match kind_of(path) {
        Some("subgraph") => scrap::shader_graph::subgraph::parse(&text).map(Doc::Sub),
        Some("effect") => scrap::shader_graph::effect::parse(&text).map(Doc::Effect),
        _ => scrap::shader_graph::surface::parse(&text).map(Doc::Surface),
    }
}

/// An input as it is written.
fn written(input: &Input) -> String {
    match input {
        Input::Name(s) => format!("\"{s}\""),
        Input::Number(n) => format!("{n:?}"),
        Input::Vector(v) => format!("({})", v.iter().map(|n| format!("{n:?}")).collect::<Vec<_>>().join(", ")),
    }
}

/// The node a read names, without its swizzle.
fn base(input: &Input) -> Option<&str> {
    match input {
        Input::Name(s) => Some(scrap::shader_graph::expr::split(s).0),
        _ => None,
    }
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
    /// Whether it builds: `Err` in the compiler's words.
    built: Result<(), String>,
    problems: Vec<String>,
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
            built: Ok(()),
            problems: Vec::new(),
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

    fn dir(session: &Session) -> Option<PathBuf> {
        session.project().map(|p| p.root().join(scrap::project::SHADERS))
    }

    fn list(&mut self, ui: &mut Ui, session: &Session) {
        ui.clear(self.files_list);
        self.parts.retain(|_, p| !matches!(p, Part::File(_)));
        let mut paths: Vec<PathBuf> = Self::dir(session)
            .and_then(|d| std::fs::read_dir(d).ok())
            .map(|r| r.flatten().map(|e| e.path()).filter(|p| kind_of(p).is_some()).collect())
            .unwrap_or_default();
        paths.sort();
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
            let glyph = match kind_of(&path) {
                Some("effect") => "sparkles",
                Some("subgraph") => "package",
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
        match load(&path) {
            Ok(doc) => {
                let same = self.open.as_ref().is_some_and(|(p, _)| *p == path);
                if !same {
                    self.chosen = None;
                    self.pan = (0.0, 0.0);
                }
                self.open = Some((path, doc));
                self.check();
                self.preview(session);
                self.list(ui, session);
                self.show(ui);
            }
            Err(e) => session.say(Level::Error, format!("{}: {e}", path.display())),
        }
    }

    /// Whether the open graph builds, and what in it is likely a mistake.
    fn check(&mut self) {
        let Some((path, doc)) = &self.open else {
            return;
        };
        let library = scrap::render::subgraphs_beside(path);
        self.problems.clear();
        self.built = match doc {
            Doc::Surface(g) => {
                self.problems = scrap::shader_graph::surface::problems_with(g, &library);
                scrap::render::material_shader_source(path)
                    .and_then(|s| scrap::render::check_material_shader(&s))
            }
            Doc::Effect(g) => {
                self.problems = scrap::shader_graph::effect::problems_with(g, &library);
                scrap::render::effect_source(path).and_then(|s| scrap::particles_gpu::check_effect(&s))
            }
            // A subgraph builds as the graphs that call it do.
            Doc::Sub(_) => Ok(()),
        };
    }

    /// The pictures: the whole material on a ball, and the chosen node.
    fn preview(&mut self, session: &mut Session) {
        let Some((path, Doc::Surface(_))) = &self.open else {
            return;
        };
        if self.built.is_err() {
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

    /// The node the compiler's words name, if any: `node `x``.
    fn blamed(&self) -> Option<String> {
        let words = self.built.as_ref().err()?;
        let at = words.find("node `")? + 6;
        let end = words[at..].find('`')?;
        let name = &words[at..at + end];
        // A subgraph's node, put in under its call: the call.
        Some(name.split("__").next().unwrap_or(name).to_string())
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
        let mut edges: Vec<(String, String)> = Vec::new();
        for (name, node) in nodes {
            for (_, input) in node.inputs() {
                if let Some(b) = base(input).filter(|b| nodes.contains_key(*b)) {
                    edges.push((b.to_string(), name.clone()));
                }
            }
        }
        for (out, reads) in &outputs {
            for input in reads {
                if let Some(b) = base(input).filter(|b| nodes.contains_key(*b)) {
                    edges.push((b.to_string(), out.to_string()));
                }
            }
        }
        edges.sort();
        edges.dedup();
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
        let blamed = self.blamed();
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
            let on = self.chosen.as_deref() == Some(*name);
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
                    .clickable(),
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
            if !output {
                self.parts.insert(b, Part::Node(name.to_string()));
            }
        }
        self.show_side(ui, &doc);
    }

    fn small(&self, ui: &mut Ui, words: &str, color: Color) {
        ui.add_text(self.side, Style::default().text_size(12.0).text_color(color), words);
    }

    fn show_side(&mut self, ui: &mut Ui, doc: &Doc) {
        let side = self.side;
        match &self.built {
            Ok(()) => self.small(ui, "Builds.", SUCCESS),
            Err(e) => self.small(ui, e, ERROR),
        }
        for p in self.problems.clone() {
            self.small(ui, &p, WARNING);
        }
        if matches!(doc, Doc::Surface(_)) && self.built.is_ok() {
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

    /// Write `new` in place of the open file, when it still reads as a
    /// graph of its kind; then read it again.
    fn write(&mut self, ui: &mut Ui, session: &mut Session, new: Option<String>, what: &str) {
        let Some((path, _)) = self.open.clone() else {
            return;
        };
        let Some(new) = new else {
            session.say(Level::Error, format!("{what}: not found in {}", path.display()));
            return;
        };
        let reads = match kind_of(&path) {
            Some("subgraph") => scrap::shader_graph::subgraph::parse(&new).map(|_| ()),
            Some("effect") => scrap::shader_graph::effect::parse(&new).map(|_| ()),
            _ => scrap::shader_graph::surface::parse(&new).map(|_| ()),
        };
        if let Err(e) = reads {
            session.say(Level::Error, format!("{what}: {e}"));
            return;
        }
        if let Err(e) = std::fs::write(&path, new) {
            session.say(Level::Error, format!("{}: {e}", path.display()));
            return;
        }
        self.open(ui, session, path);
    }

    pub fn event(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, event: &Event) {
        let Some(part) = self.parts.get(&node).cloned() else {
            return;
        };
        let click = matches!(event, Event::Click { .. });
        let text_now = || self.open.as_ref().and_then(|(p, _)| std::fs::read_to_string(p).ok()).unwrap_or_default();
        match part {
            Part::File(path) if click => self.open(ui, session, path),
            Part::Wide if click => {
                self.wide = !self.wide;
                set_icon_button(ui, self.wide_button, "expand", self.wide, true);
            }
            Part::Node(name) if click => {
                self.chosen = Some(name);
                self.preview(session);
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
                if let (Event::Submit(value), Some(name)) = (event, self.chosen.clone()) {
                    let new = scrap::shader_graph::text::set_input(&text_now(), &name, field, value.trim());
                    self.write(ui, session, new, &format!("node `{name}`, input `{field}`"));
                }
            }
            Part::Add => {
                if let Event::Submit(value) = event {
                    let Some((name, node)) = value.split_once(':') else {
                        session.say(Level::Error, "a new node is `\"name\": Kind(…)`");
                        return;
                    };
                    let name = name.trim().trim_matches('"').to_string();
                    let new = scrap::shader_graph::text::add_node(&text_now(), &name, node.trim());
                    self.write(ui, session, new, &format!("node `{name}` (a name is one node's)"));
                    if self.open.as_ref().is_some_and(|(_, d)| d.nodes().contains_key(&name)) {
                        self.chosen = Some(name);
                        self.show(ui);
                    }
                }
            }
            Part::Delete if click => {
                if let Some(name) = self.chosen.take() {
                    let new = scrap::shader_graph::text::remove_node(&text_now(), &name);
                    self.write(ui, session, new, &format!("node `{name}`"));
                }
            }
            _ => {}
        }
    }
}
