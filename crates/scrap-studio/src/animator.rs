//! Animator: which animation plays when, as a graph to look at and edit.
//!
//! Unity's Animator window for `scrap::animgraph`: the files in
//! `animators/` are graphs of states and transitions. Here a state is a
//! box, a transition an arrow, and "Any State" the box the graph's `any`
//! transitions leave from. Choosing a box or an arrow shows its fields on
//! the right. The chips there add a transition to another state, and the
//! buttons make a state the start or delete it.
//!
//! Where the boxes stand is worked out from the graph every time
//! ([`layout`]), never dragged and never kept: an agent that adds three
//! states gives them no places and needs none, a graph looks the same to
//! everyone who opens it, and there is no second file to fall out of step
//! with the first. The ground is dragged to pan.
//!
//! Every change is written at once, and only where it changed: a state's
//! entry with the transitions that leave it, Any State's list, the start
//! (see [`scrap::ron_edit`]). Comments stay, and the diff is the change.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use scrap::animgraph::{Condition, Graph, State, Transition, ANY};
use scrap_editor::console::Level;
use scrap_editor::Session;
use scrap_ui::{Color, Event, NodeId, Style, Ui};

use crate::theme::*;
use scrap::graph_text::{conditions, number, pairs, read_pairs, write};

use scrap_ui::graph_view::{BOX_H, BOX_W};

#[derive(Debug, Clone, PartialEq)]
enum Chosen {
    State(String),
    Transition(usize),
}

#[derive(Debug, Clone)]
enum Part {
    File(PathBuf),
    Box(String),
    Edge(usize),
    /// A field of the chosen state or transition, by name.
    Field(&'static str),
    Start,
    Looping,
    Delete,
    To(String),
    New,
    AddState,
    Wide,
    Canvas,
    /// Show a state's blend tree or events fields.
    More(&'static str),
    /// Take back one change since the last commit, by its place in the list.
    Revert(usize),
}

pub struct Animator {
    pub root: NodeId,
    files_list: NodeId,
    canvas: NodeId,
    side: NodeId,
    wide_button: NodeId,
    /// Over the whole window, as the UI Builder can be.
    pub wide: bool,
    parts: HashMap<NodeId, Part>,
    open: Option<(PathBuf, Graph)>,
    chosen: Option<Chosen>,
    /// Each state's column and row, from [`layout`], and how far the
    /// canvas is panned.
    layout: BTreeMap<String, (usize, usize)>,
    pan: (f32, f32),
    listed: bool,
    /// The canvas's two layers — arrows under boxes — moved whole to pan.
    edges_layer: Option<NodeId>,
    boxes_layer: Option<NodeId>,
    /// Arrow heads, over the boxes they point at, to be clicked.
    heads_layer: Option<NodeId>,
    boxes: HashMap<String, NodeId>,
    edge_nodes: Vec<NodeId>,
    /// Fields asked for on a state that has none of them yet.
    more: Vec<(String, &'static str)>,
    /// The state the running game says the selected entity is in, and
    /// when that was last asked.
    live: Option<String>,
    /// The open graph as the last commit has it: what the changes are
    /// against. `None` outside git, or for a file not yet committed.
    head: Option<Graph>,
    asked: std::time::Instant,
}

impl Animator {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let root = ui.add(
            parent,
            Style::row()
                .fill()
                .full_width()
                .gap(SPACE_2)
                .padding(SPACE_2),
        );
        ui.set_name(root, "animator");
        let left = ui.add(
            root,
            Style::column()
                .width(170.0)
                .full_height()
                .fixed()
                .gap(SPACE_2),
        );
        let head = ui.add(left, Style::row().full_width().center_items().gap(SPACE_1));
        ui.add_text(head, caption(), "ANIMATORS");
        spacer(ui, head);
        let new = icon_button(ui, head, "animator new", "file-plus", false);
        let wide_button = icon_button(ui, head, "animator wide", "expand", false);
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
        ui.set_name(canvas, "animator canvas");
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
        parts.insert(new, Part::New);
        parts.insert(wide_button, Part::Wide);
        parts.insert(canvas, Part::Canvas);
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
            edges_layer: None,
            boxes_layer: None,
            heads_layer: None,
            boxes: HashMap::new(),
            edge_nodes: Vec::new(),
            more: Vec::new(),
            live: None,
            head: None,
            asked: std::time::Instant::now(),
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

    fn dir(session: &Session) -> Option<PathBuf> {
        session
            .project()
            .map(|p| p.root().join(scrap::project::ANIMATORS))
    }

    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        self.follow_game(ui, session);
        if self.listed {
            return;
        }
        self.listed = true;
        self.list(ui, session);
    }

    fn list(&mut self, ui: &mut Ui, session: &Session) {
        ui.clear(self.files_list);
        self.parts.retain(|_, p| !matches!(p, Part::File(_)));
        let mut paths: Vec<PathBuf> = Self::dir(session)
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
                "No animators yet: the page button makes one.",
            );
        }
        let open = self.open.as_ref().map(|(p, _)| p.clone());
        for path in paths {
            let name = stem(&path);
            let on = open.as_ref() == Some(&path);
            let row = line(
                ui,
                self.files_list,
                &format!("animator {name}"),
                "route",
                &name,
                on,
            );
            self.parts.insert(row, Part::File(path));
        }
    }

    fn open(&mut self, ui: &mut Ui, session: &mut Session, path: PathBuf) {
        let graph = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| scrap::ron::from_str::<Graph>(&t).map_err(|e| e.to_string()));
        match graph {
            Ok(graph) => {
                self.head = scrap_editor::history::show(&path, "HEAD")
                    .ok()
                    .and_then(|t| scrap::ron::from_str(&t).ok());
                self.open = Some((path, graph));
                self.chosen = None;
                self.pan = (0.0, 0.0);
                self.list(ui, session);
                self.show(ui, session);
            }
            Err(e) => session.say(Level::Error, format!("{}: {e}", path.display())),
        }
    }

    /// Where a box stands, from its column and row in the layout.
    fn place(&self, name: &str) -> (f32, f32) {
        if name == ANY {
            return (24.0, 20.0);
        }
        let (column, row) = self.layout.get(name).copied().unwrap_or((0, 0));
        (
            24.0 + column as f32 * (BOX_W + 70.0),
            90.0 + row as f32 * (BOX_H + 50.0),
        )
    }

    /// Build the canvas and the side again from the graph.
    fn show(&mut self, ui: &mut Ui, session: &Session) {
        self.parts
            .retain(|_, p| matches!(p, Part::File(_) | Part::New | Part::Wide | Part::Canvas));
        ui.clear(self.canvas);
        ui.clear(self.side);
        let Some((_, graph)) = self.open.clone() else {
            ui.add_text(
                self.side,
                Style::default().text_size(12.0).text_color(MUTED),
                "Open an animator on the left.",
            );
            return;
        };
        self.layout = layout(&graph);
        let layer = |ui: &mut Ui, canvas: NodeId, pan: (f32, f32)| {
            ui.add(
                canvas,
                Style::default().absolute(pan.0, pan.1).size(4000.0, 4000.0),
            )
        };
        let edges = layer(ui, self.canvas, self.pan);
        let boxes = layer(ui, self.canvas, self.pan);
        let heads = layer(ui, self.canvas, self.pan);
        self.edges_layer = Some(edges);
        self.boxes_layer = Some(boxes);
        self.heads_layer = Some(heads);
        self.boxes.clear();
        self.draw_edges(ui);
        let mut names: Vec<String> = vec![ANY.to_string()];
        names.extend(graph.states.keys().cloned());
        for name in names {
            let (x, y) = self.place(&name);
            let on = self.chosen == Some(Chosen::State(name.clone()));
            let start = name == graph.start;
            let any = name == ANY;
            let live = self.live.as_deref() == Some(name.as_str());
            let mark = self.mark(&graph, &name);
            let b = ui.add(boxes, box_style(&graph, &name, on, live, mark, (x, y)));
            ui.set_name(b, format!("state {name}"));
            let label = if any { "Any State" } else { name.as_str() };
            let glyph = if any {
                "sparkles"
            } else if graph.states[&name].blend.is_empty() {
                "play"
            } else {
                "sliders-horizontal"
            };
            icon(ui, b, glyph, if start { ACCENT_200 } else { LABEL });
            ui.add_text(b, text().nowrap(), label);
            self.parts.insert(b, Part::Box(name.clone()));
            self.boxes.insert(name, b);
        }
        self.show_side(ui, session, &graph);
    }

    /// Light up the state the running game's selected entity is in, as
    /// Unity's Animator does in play mode. The game says it in its report
    /// a few times a second; asking is reading that file, so not more
    /// often than four times a second.
    fn follow_game(&mut self, ui: &mut Ui, session: &Session) {
        if self.asked.elapsed().as_secs_f32() < 0.25 {
            return;
        }
        self.asked = std::time::Instant::now();
        let live = session.selected().and_then(|id| session.game_animator(id));
        if live != self.live {
            self.live = live;
            self.restyle_boxes(ui);
        }
    }

    fn restyle_boxes(&mut self, ui: &mut Ui) {
        let Some(graph) = self.graph().cloned() else {
            return;
        };
        for (name, node) in &self.boxes {
            let on = self.chosen == Some(Chosen::State(name.clone()));
            let live = self.live.as_deref() == Some(name.as_str());
            let mark = self.mark(&graph, name);
            ui.set_style(
                *node,
                box_style(&graph, name, on, live, mark, self.place(name)),
            );
        }
    }

    /// Choose a box or an arrow without building the canvas again.
    fn choose(&mut self, ui: &mut Ui, session: &Session, chosen: Option<Chosen>) {
        if self.chosen == chosen {
            return;
        }
        self.chosen = chosen;
        let Some(graph) = self.graph().cloned() else {
            return;
        };
        self.restyle_boxes(ui);
        self.draw_edges(ui);
        self.parts.retain(|n, p| {
            !matches!(
                p,
                Part::Field(_)
                    | Part::Start
                    | Part::Looping
                    | Part::Delete
                    | Part::To(_)
                    | Part::AddState
                    | Part::More(_)
                    | Part::Revert(_)
            ) && !(matches!(p, Part::Edge(_))
                && ui.parent(*n).is_some_and(|parent| parent == self.side))
        });
        ui.clear(self.side);
        self.show_side(ui, session, &graph);
    }

    /// The arrows again, from where the boxes stand now.
    fn draw_edges(&mut self, ui: &mut Ui) {
        let (Some(layer), Some(heads)) = (self.edges_layer, self.heads_layer) else {
            return;
        };
        let Some(graph) = self.graph().cloned() else {
            return;
        };
        ui.clear(layer);
        ui.clear(heads);
        for n in self.edge_nodes.drain(..) {
            self.parts.remove(&n);
        }
        for (i, t) in graph.transitions.iter().enumerate() {
            let known = |n: &str| n == ANY || graph.states.contains_key(n);
            if t.from == t.to || t.to == ANY || !known(&t.from) || !known(&t.to) {
                continue;
            }
            let on = self.chosen == Some(Chosen::Transition(i));
            let back = t.from > t.to
                && graph
                    .transitions
                    .iter()
                    .any(|o| o.from == t.to && o.to == t.from);
            let others: Vec<(f32, f32)> = std::iter::once(ANY)
                .chain(graph.states.keys().map(String::as_str))
                .filter(|n| *n != t.from && *n != t.to)
                .map(|n| self.place(n))
                .collect();
            self.arrow(
                ui,
                layer,
                i,
                self.place(&t.from),
                self.place(&t.to),
                on,
                back,
                &others,
            );
        }
    }

    /// An arrow for transition `i`, clickable as it: see
    /// [`scrap_ui::graph_view::arrow`].
    #[allow(clippy::too_many_arguments)]
    fn arrow(
        &mut self,
        ui: &mut Ui,
        layer: NodeId,
        i: usize,
        from: (f32, f32),
        to: (f32, f32),
        on: bool,
        back: bool,
        others: &[(f32, f32)],
    ) {
        let nodes = scrap_ui::graph_view::arrow(
            ui,
            layer,
            self.heads_layer.unwrap_or(layer),
            &format!("transition {i}"),
            from,
            to,
            back,
            others,
            if on { ACCENT } else { NEUTRAL_500 },
            if on { 3.0 } else { 2.0 },
        );
        for node in nodes {
            self.parts.insert(node, Part::Edge(i));
            self.edge_nodes.push(node);
        }
    }

    fn show_side(&mut self, ui: &mut Ui, session: &Session, graph: &Graph) {
        let side = self.side;
        // What will not work, first.
        let clips: Vec<String> = session
            .selected()
            .map(|id| session.clips(id).into_iter().map(|(n, _)| n).collect())
            .unwrap_or_default();
        let clip_refs: Vec<&str> = clips.iter().map(String::as_str).collect();
        let problems: Vec<String> = graph
            .problems(&clip_refs)
            .into_iter()
            .filter(|p| !clips.is_empty() || !p.contains("the model does not have"))
            .collect();
        for p in problems.iter().take(3) {
            ui.add_text(side, Style::default().text_size(11.5).text_color(ERROR), p);
        }
        // What changed since the last commit — an agent's edit to look over
        // — each with a way to take it back.
        let changes = self.changes(graph);
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
                let undo = button(ui, row, &format!("animator undo change {i}"), "Undo", false);
                self.parts.insert(undo, Part::Revert(i));
            }
        }
        let head = ui.add(side, Style::row().full_width().gap(SPACE_1).center_items());
        let add = button(ui, head, "animator add state", "+ State", false);
        self.parts.insert(add, Part::AddState);
        match self.chosen.clone() {
            None => {
                ui.add_text(
                    side,
                    Style::default().text_size(12.0).text_color(MUTED),
                    "Pick a state or an arrow. The boxes place themselves; drag the ground to pan.",
                );
            }
            Some(Chosen::State(name)) if name == ANY => {
                ui.add_text(side, text(), "Any State");
                ui.add_text(
                    side,
                    Style::default().text_size(11.5).text_color(MUTED),
                    "A transition from here is taken from whatever state is playing.",
                );
                self.targets(ui, graph, ANY);
                self.leaving(ui, graph, ANY);
            }
            Some(Chosen::State(name)) => {
                let Some(state) = graph.states.get(&name).cloned() else {
                    return;
                };
                self.field(ui, "name", "name", &name);
                if state.blend.is_empty() {
                    self.field(ui, "clip", "clip", &state.clip);
                }
                self.field(ui, "speed", "speed", &number(state.speed));
                self.field(
                    ui,
                    "speed from",
                    "speed_from",
                    state.speed_from.as_deref().unwrap_or(""),
                );
                self.field(
                    ui,
                    "time from",
                    "time_from",
                    state.time_from.as_deref().unwrap_or(""),
                );
                // A blend tree and events only where there are some, or
                // once asked for: most states are one clip.
                let blends =
                    !state.blend.is_empty() || self.more.contains(&(name.clone(), "blend"));
                if blends {
                    self.field(ui, "blend by", "blend_by", &state.blend_by);
                    self.field(ui, "blend", "blend", &pairs(&state.blend));
                }
                let events =
                    !state.events.is_empty() || self.more.contains(&(name.clone(), "events"));
                if events {
                    self.field(ui, "events", "events", &pairs(&state.events));
                }
                if !blends || !events {
                    let row = ui.add(side, Style::row().full_width().gap(3.0));
                    for (on, key, label) in [
                        (blends, "blend", "+ Blend tree"),
                        (events, "events", "+ Events"),
                    ] {
                        if !on {
                            let chip = ui.add(
                                row,
                                Style::row()
                                    .height(20.0)
                                    .padding_x(7.0)
                                    .center()
                                    .radius(6.0)
                                    .hover(HOVER)
                                    .clickable(),
                            );
                            ui.set_name(chip, format!("animator more {key}"));
                            ui.add_text(
                                chip,
                                Style::default().text_size(11.5).text_color(ACCENT),
                                label,
                            );
                            self.parts.insert(chip, Part::More(key));
                        }
                    }
                }
                let row = ui.add(side, Style::row().full_width().gap(SPACE_1));
                let looping = button(
                    ui,
                    row,
                    "animator looping",
                    if state.looping { "Loops" } else { "Plays once" },
                    state.looping,
                );
                self.parts.insert(looping, Part::Looping);
                let is_start = graph.start == name;
                let start = button(
                    ui,
                    row,
                    "animator start",
                    if is_start {
                        "Start state"
                    } else {
                        "Make start"
                    },
                    is_start,
                );
                self.parts.insert(start, Part::Start);
                let delete = button(ui, row, "animator delete", "Delete", false);
                self.parts.insert(delete, Part::Delete);
                self.targets(ui, graph, &name);
                self.leaving(ui, graph, &name);
            }
            Some(Chosen::Transition(i)) => {
                let Some(t) = graph.transitions.get(i).cloned() else {
                    return;
                };
                let from = if t.from == ANY { "Any State" } else { &t.from };
                ui.add_text(side, text(), &format!("{from} → {}", t.to));
                self.field(ui, "when", "when", &conditions(&t.when));
                self.field(ui, "fade", "fade", &number(t.fade));
                ui.add_text(
                    side,
                    Style::default().text_size(11.0).text_color(MUTED),
                    "when: Above(\"p\", 1), Below(\"p\", 1), Is(\"p\"), Not(\"p\"), Trigger(\"t\"), Finished",
                );
                let delete = button(ui, side, "animator delete", "Delete", false);
                self.parts.insert(delete, Part::Delete);
            }
        }
    }

    /// Chips that add a transition from `from` to each other state.
    fn targets(&mut self, ui: &mut Ui, graph: &Graph, from: &str) {
        ui.add_text(self.side, caption(), "ADD A TRANSITION TO");
        let chips = ui.add(self.side, Style::row().full_width().gap(3.0).wrap());
        for name in graph.states.keys().filter(|n| *n != from) {
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
            ui.set_name(chip, format!("to {name}"));
            ui.add_text(
                chip,
                Style::default().text_size(11.5).text_color(LABEL),
                name,
            );
            self.parts.insert(chip, Part::To(name.clone()));
        }
    }

    /// The transitions leaving `from`, to choose one.
    fn leaving(&mut self, ui: &mut Ui, graph: &Graph, from: &str) {
        let out: Vec<(usize, &Transition)> = graph
            .transitions
            .iter()
            .enumerate()
            .filter(|(_, t)| t.from == from)
            .collect();
        if out.is_empty() {
            return;
        }
        ui.add_text(self.side, caption(), "TRANSITIONS");
        for (i, t) in out {
            let label = format!("→ {}  {}", t.to, conditions(&t.when));
            let row = line(
                ui,
                self.side,
                &format!("leaving {i}"),
                "route",
                &label,
                false,
            );
            self.parts.insert(row, Part::Edge(i));
        }
    }

    fn field(&mut self, ui: &mut Ui, label: &str, key: &'static str, value: &str) {
        let r = ui.add(
            self.side,
            Style::row().full_width().gap(SPACE_2).center_items(),
        );
        ui.add_text(
            r,
            Style::default()
                .width(70.0)
                .fixed()
                .text_size(12.0)
                .text_color(LABEL)
                .nowrap(),
            label,
        );
        let b = ui.add_field(r, field_style().fill().mono().text_size(11.5), value);
        ui.set_name(b, format!("animator field {key}"));
        self.parts.insert(b, Part::Field(key));
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
            Part::Canvas => match event {
                Event::Drag { dx, dy, .. } => {
                    self.pan.0 += dx;
                    self.pan.1 += dy;
                    let (x, y) = self.pan;
                    for layer in [self.edges_layer, self.boxes_layer, self.heads_layer]
                        .into_iter()
                        .flatten()
                    {
                        ui.restyle(layer, |s| s.absolute(x, y));
                    }
                }
                Event::Click { .. } => self.choose(ui, session, None),
                _ => {}
            },
            Part::Box(name) if matches!(event, Event::Press { .. }) => {
                self.choose(ui, session, Some(Chosen::State(name)))
            }
            Part::Edge(i) if click => self.choose(ui, session, Some(Chosen::Transition(i))),
            Part::Field(key) => {
                if let Event::Submit(value) = event {
                    self.set(session, key, value.trim());
                    self.show(ui, session);
                }
            }
            Part::To(to) if click => {
                if let Some(Chosen::State(from)) = self.chosen.clone() {
                    let (f, t) = (from.clone(), to.clone());
                    self.edit(session, |g| {
                        g.transitions.push(Transition {
                            from,
                            to,
                            when: Vec::new(),
                            fade: 0.2,
                        });
                    });
                    // The new one is the last leaving `from` for `to`, wherever
                    // putting the list in order has put it.
                    let at = self
                        .graph()
                        .and_then(|g| g.transitions.iter().rposition(|x| x.from == f && x.to == t));
                    self.chosen = at.map(Chosen::Transition);
                    self.show(ui, session);
                }
            }
            Part::More(key) if click => {
                if let Some(Chosen::State(name)) = self.chosen.clone() {
                    self.more.push((name, key));
                    self.show(ui, session);
                }
            }
            Part::Start if click => {
                if let Some(Chosen::State(name)) = self.chosen.clone() {
                    self.edit(session, |g| g.start = name);
                    self.show(ui, session);
                }
            }
            Part::Looping if click => {
                if let Some(Chosen::State(name)) = self.chosen.clone() {
                    self.edit(session, |g| {
                        if let Some(s) = g.states.get_mut(&name) {
                            s.looping = !s.looping;
                        }
                    });
                    self.show(ui, session);
                }
            }
            Part::Delete if click => {
                match self.chosen.take() {
                    Some(Chosen::State(name)) if name != ANY => self.edit(session, |g| {
                        g.states.remove(&name);
                        g.transitions.retain(|t| t.from != name && t.to != name);
                    }),
                    Some(Chosen::Transition(i)) => self.edit(session, |g| {
                        if i < g.transitions.len() {
                            g.transitions.remove(i);
                        }
                    }),
                    _ => {}
                }
                self.show(ui, session);
            }
            Part::Revert(i) if click => {
                let Some(graph) = self.graph().cloned() else {
                    return;
                };
                if let Some(change) = self.changes(&graph).into_iter().nth(i) {
                    self.edit(session, |g| change.revert(g));
                }
                self.show(ui, session);
            }
            Part::AddState if click => {
                let Some(graph) = self.graph() else { return };
                let mut name = "state".to_string();
                let mut n = 2;
                while graph.states.contains_key(&name) {
                    name = format!("state_{n}");
                    n += 1;
                }
                let first = graph.states.is_empty();
                let new = name.clone();
                self.edit(session, |g| {
                    g.states.insert(new.clone(), state(&new));
                    if first {
                        g.start = new;
                    }
                });
                self.chosen = Some(Chosen::State(name));
                self.show(ui, session);
            }
            _ => {}
        }
    }

    fn graph(&self) -> Option<&Graph> {
        self.open.as_ref().map(|(_, g)| g)
    }

    /// A field of the chosen state or transition, typed.
    fn set(&mut self, session: &mut Session, key: &str, value: &str) {
        let parse_number = |v: &str| v.parse::<f32>().map_err(|_| format!("{key}: a number"));
        let result = match self.chosen.clone() {
            Some(Chosen::State(name)) if key == "name" => {
                if value.is_empty() || value == ANY {
                    Err("a state needs a name".to_string())
                } else if value != name
                    && self.graph().is_some_and(|g| g.states.contains_key(value))
                {
                    Err(format!("there is a state `{value}` already"))
                } else {
                    let new = value.to_string();
                    let old = name.clone();
                    self.edit(session, |g| {
                        if let Some(s) = g.states.remove(&old) {
                            g.states.insert(new.clone(), s);
                        }
                        for t in &mut g.transitions {
                            for end in [&mut t.from, &mut t.to] {
                                if *end == old {
                                    *end = new.clone();
                                }
                            }
                        }
                        if g.start == old {
                            g.start = new.clone();
                        }
                    });
                    self.chosen = Some(Chosen::State(value.to_string()));
                    Ok(())
                }
            }
            Some(Chosen::State(name)) => {
                let mut state = self
                    .graph()
                    .and_then(|g| g.states.get(&name))
                    .cloned()
                    .unwrap_or_else(|| state(&name));
                let set = match key {
                    "clip" => {
                        state.clip = value.to_string();
                        Ok(())
                    }
                    "speed" => parse_number(value).map(|n| state.speed = n),
                    "speed_from" => {
                        state.speed_from = (!value.is_empty()).then(|| value.to_string());
                        Ok(())
                    }
                    "time_from" => {
                        state.time_from = (!value.is_empty()).then(|| value.to_string());
                        Ok(())
                    }
                    "blend_by" => {
                        state.blend_by = value.to_string();
                        Ok(())
                    }
                    "blend" => read_pairs(value).map(|p| state.blend = p),
                    "events" => read_pairs(value).map(|p| state.events = p),
                    _ => Ok(()),
                };
                set.map(|()| {
                    self.edit(session, |g| {
                        g.states.insert(name, state);
                    })
                })
            }
            Some(Chosen::Transition(i)) => {
                let Some(mut t) = self.graph().and_then(|g| g.transitions.get(i)).cloned() else {
                    return;
                };
                let set = match key {
                    "when" => {
                        let text = if value.starts_with('[') {
                            value.to_string()
                        } else {
                            format!("[{value}]")
                        };
                        scrap::ron::from_str::<Vec<Condition>>(&text)
                            .map(|w| t.when = w)
                            .map_err(|e| format!("when: {e}"))
                    }
                    "fade" => parse_number(value).map(|n| t.fade = n.max(0.0)),
                    _ => Ok(()),
                };
                set.map(|()| self.edit(session, |g| g.transitions[i] = t))
            }
            None => Ok(()),
        };
        if let Err(e) = result {
            session.say(Level::Error, e);
        }
    }

    /// How a state is marked for what changed since the last commit: new
    /// in the accent's light, changed in the warning colour.
    fn mark(&self, graph: &Graph, name: &str) -> Option<Color> {
        self.changes(graph).iter().find_map(|c| match c {
            scrap::animgraph::Change::StateAdded(n) if n == name => Some(ACCENT_400),
            scrap::animgraph::Change::StateChanged(n, _) if n == name => Some(WARNING),
            _ => None,
        })
    }

    /// What differs from the last commit's graph, if there is one.
    fn changes(&self, graph: &Graph) -> Vec<scrap::animgraph::Change> {
        self.head
            .as_ref()
            .map(|head| scrap::animgraph::diff(head, graph))
            .unwrap_or_default()
    }

    /// Change the graph and write it.
    fn edit(&mut self, session: &mut Session, change: impl FnOnce(&mut Graph)) {
        let Some((path, graph)) = self.open.as_mut() else {
            return;
        };
        change(graph);
        graph.normalize();
        let old = std::fs::read_to_string(&*path).unwrap_or_default();
        let text = write(&old, graph);
        if text != old {
            if let Err(e) = std::fs::write(&*path, text) {
                session.say(Level::Error, format!("{}: {e}", path.display()));
            }
        }
    }

    fn new_file(&mut self, ui: &mut Ui, session: &mut Session) {
        let Some(dir) = Self::dir(session) else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        let mut path = dir.join("animator.ron");
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("animator_{n}.ron"));
            n += 1;
        }
        let text = "(\n    start: \"idle\",\n    states: {\n        \"idle\": (clip: \"idle\"),\n    },\n)\n";
        match std::fs::write(&path, text) {
            Ok(()) => self.open(ui, session, path),
            Err(e) => session.say(Level::Error, format!("{}: {e}", path.display())),
        }
    }
}

/// Each state's column and row. Columns go by how many transitions from
/// the start a state is, so arrows mostly run to the next column over;
/// states reached from nowhere but Any State, or from nowhere, stand in the
/// first column over. Within a column, a state goes near the ones that lead
/// into it — by the mean row of those already placed — and then by name,
/// so arrows cross little. The same graph always comes out the same.
pub fn layout(graph: &Graph) -> BTreeMap<String, (usize, usize)> {
    let nodes: Vec<&str> = graph.states.keys().map(String::as_str).collect();
    let edges: Vec<(&str, &str)> = graph
        .transitions
        .iter()
        .map(|t| (t.from.as_str(), t.to.as_str()))
        .collect();
    scrap_ui::graph_view::layout(&graph.start, &nodes, &edges)
}

/// A state's box: the start filled with the accent, the chosen one ringed
/// in it, and the one the running game is in ringed in warm light.
fn box_style(
    graph: &Graph,
    name: &str,
    on: bool,
    live: bool,
    changed: Option<Color>,
    (x, y): (f32, f32),
) -> Style {
    let start = name == graph.start;
    Style::row()
        .absolute(x, y)
        .size(BOX_W, BOX_H)
        .padding_x(SPACE_3)
        .gap(SPACE_2)
        .center_items()
        .radius(RADIUS_MD)
        .background(if start {
            ACCENT_800
        } else if name == ANY {
            NEUTRAL_900
        } else {
            SURFACE
        })
        .border(
            if on || live || changed.is_some() {
                2.0
            } else {
                1.0
            },
            if live {
                WARNING
            } else if on {
                ACCENT
            } else if let Some(mark) = changed {
                mark
            } else {
                NEUTRAL_800
            },
        )
        .clickable()
}

fn stem(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn state(name: &str) -> State {
    State {
        clip: name.to_string(),
        blend: Vec::new(),
        blend_by: String::new(),
        directional: Vec::new(),
        blend_by_y: String::new(),
        events: Vec::new(),
        looping: true,
        speed: 1.0,
        speed_from: None,
        time_from: None,
    }
}

fn line(ui: &mut Ui, parent: NodeId, name: &str, glyph: &str, label: &str, on: bool) -> NodeId {
    let row = ui.add(
        parent,
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
    ui.set_name(row, name.to_string());
    icon(ui, row, glyph, MUTED);
    ui.add_text(row, text().nowrap(), label);
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"// The hero.
(
    start: "idle",
    states: {
        "idle": (clip: "idle", transitions: [
            (to: "walk", when: [Above("speed", 0.1)]),
        ]), // standing
        // Back when slow.
        "walk": (clip: "walk", speed_from: "speed", transitions: [
            (to: "idle", when: [Below("speed", 0.1)]),
        ]),
    },
)
"#;

    #[test]
    fn edits_keep_the_file() {
        let mut g: Graph = scrap::ron::from_str(FILE).unwrap();
        g.states.insert(
            "jump".into(),
            State {
                looping: false,
                ..state("jump")
            },
        );
        g.transitions.push(Transition {
            from: "*".into(),
            to: "jump".into(),
            when: vec![Condition::Trigger("jump".into())],
            fade: 0.1,
        });
        g.states.get_mut("walk").unwrap().speed = 1.5;
        g.start = "walk".into();
        let out = write(FILE, &g);
        assert!(out.starts_with("// The hero."), "{out}");
        assert!(out.contains("// standing"));
        assert!(out.contains("// Back when slow."));
        assert!(
            out.contains(r#"        "jump": (clip: "jump", looping: false),"#),
            "{out}"
        );
        assert!(
            out.contains(
                "    any: [\n        (to: \"jump\", when: [Trigger(\"jump\")], fade: 0.1),\n    ],"
            ),
            "{out}"
        );
        assert!(out
            .contains(r#""walk": (clip: "walk", speed: 1.5, speed_from: "speed", transitions: ["#));
        assert!(out.contains(r#"start: "walk","#));
        g.normalize();
        assert_eq!(scrap::ron::from_str::<Graph>(&out).unwrap(), g);

        // A transition added to one state touches that state's entry only.
        let before = out.clone();
        g.transitions.push(Transition {
            from: "idle".into(),
            to: "jump".into(),
            when: Vec::new(),
            fade: 0.2,
        });
        let out = write(&before, &g);
        let added: Vec<&str> = out
            .lines()
            .filter(|l| !before.lines().any(|b| b == *l))
            .collect();
        assert_eq!(added, [r#"            (to: "jump"),"#], "{out}");

        // A state and its transitions gone.
        g.states.remove("idle");
        g.transitions.retain(|t| t.from != "idle" && t.to != "idle");
        let out = write(&out, &g);
        g.normalize();
        assert_eq!(scrap::ron::from_str::<Graph>(&out).unwrap(), g);
        assert!(
            out.starts_with("// The hero."),
            "patched, not rewritten: {out}"
        );
    }

    #[test]
    fn a_file_of_the_older_shape_is_written_in_the_new_one_and_keeps_its_comments() {
        let old = r#"// The hero.
(
    start: "idle",
    states: {
        "idle": (clip: "idle"), // standing
        "walk": (clip: "walk"),
    },
    transitions: [
        (from: "idle", to: "walk", when: [Above("speed", 0.1)]),
        (from: "*", to: "idle", when: [Trigger("reset")]),
    ],
)
"#;
        let mut g: Graph = scrap::ron::from_str(old).unwrap();
        g.states.get_mut("walk").unwrap().speed = 2.0;
        let out = write(old, &g);
        assert!(out.starts_with("// The hero."), "{out}");
        assert!(out.contains("// standing"), "{out}");
        assert!(!out.contains("from:"), "{out}");
        assert!(out.contains("any: ["), "{out}");
        g.normalize();
        assert_eq!(scrap::ron::from_str::<Graph>(&out).unwrap(), g);
    }

    #[test]
    fn the_layout_is_the_graph_s_own() {
        let g: Graph = scrap::ron::from_str(
            r#"(start: "idle", states: {
                "idle": (clip: "idle", transitions: [(to: "walk"), (to: "crouch")]),
                "walk": (clip: "walk", transitions: [(to: "run")]),
                "crouch": (clip: "crouch", transitions: [(to: "crawl")]),
                "run": (clip: "run"),
                "crawl": (clip: "crawl"),
                "dead": (clip: "dead"),
            }, any: [(to: "dead")])"#,
        )
        .unwrap();
        let l = layout(&g);
        assert_eq!(l["idle"], (0, 0));
        assert_eq!(l["crouch"], (1, 0));
        assert_eq!(l["walk"], (1, 1));
        assert_eq!(l["dead"], (1, 2), "reached only from Any State");
        // Each goes beside what leads to it, not in name order.
        assert_eq!(l["crawl"], (2, 0));
        assert_eq!(l["run"], (2, 1));
        assert_eq!(layout(&g), l, "the same every time");
    }
}
