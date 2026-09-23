//! Animator: which animation plays when, as a graph to look at and edit.
//!
//! Unity's Animator window for `runity::animgraph`: the files in
//! `animators/` are graphs of states and transitions. Here a state is a
//! box, a transition an arrow, and "Any State" the box `"*"` transitions
//! leave from. A box is dragged to where it reads best, and where the boxes
//! stand is the editor's, in `.runity/animators.ron`, not the graph's: the
//! graph file stays what the game reads. Choosing a box or an arrow shows
//! its fields on the right. The chips there add a transition to another
//! state, and the buttons make a state the start or delete it.
//!
//! Every change is written at once, and only where it changed: a state's
//! entry, a transition, the start (see [`crate::patch`]). Comments stay, and
//! the diff is the change.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use runity::animgraph::{Condition, Graph, State, Transition};
use runity_editor::console::Level;
use runity_editor::Session;
use runity_ui::{Color, Event, NodeId, Style, Ui};

use crate::patch::{self, Change};
use crate::theme::*;

const BOX_W: f32 = 150.0;
const BOX_H: f32 = 36.0;
/// The name `"*"` transitions leave from, shown as its own box.
const ANY: &str = "*";

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
    /// Where each box stands, by file and state, and how far the canvas is
    /// panned.
    places: BTreeMap<String, BTreeMap<String, (f32, f32)>>,
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
            places: BTreeMap::new(),
            pan: (0.0, 0.0),
            listed: false,
            edges_layer: None,
            boxes_layer: None,
            heads_layer: None,
            boxes: HashMap::new(),
            edge_nodes: Vec::new(),
            more: Vec::new(),
            live: None,
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
            .map(|p| p.root().join(runity::project::ANIMATORS))
    }

    fn places_file(session: &Session) -> Option<PathBuf> {
        session
            .project()
            .map(|p| p.root().join(".runity").join("animators.ron"))
    }

    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        self.follow_game(ui, session);
        if self.listed {
            return;
        }
        self.listed = true;
        if let Some(text) = Self::places_file(session).and_then(|p| std::fs::read_to_string(p).ok())
        {
            self.places = runity::ron::from_str(&text).unwrap_or_default();
        }
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
            .and_then(|t| runity::ron::from_str::<Graph>(&t).map_err(|e| e.to_string()));
        match graph {
            Ok(graph) => {
                self.open = Some((path, graph));
                self.chosen = None;
                self.pan = (0.0, 0.0);
                self.list(ui, session);
                self.show(ui, session);
            }
            Err(e) => session.say(Level::Error, format!("{}: {e}", path.display())),
        }
    }

    /// Where a box stands: as dragged, or on a grid in name order.
    fn place(&self, name: &str) -> (f32, f32) {
        let Some((path, graph)) = &self.open else {
            return (0.0, 0.0);
        };
        if let Some(p) = self.places.get(&stem(path)).and_then(|m| m.get(name)) {
            return *p;
        }
        if name == ANY {
            return (24.0, 20.0);
        }
        // Columns by how many transitions from the start a state is,
        // rows in name order within a column: arrows mostly go to the
        // next column over, not under other boxes.
        let (column, row) = layered(graph, name);
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
            let b = ui.add(boxes, box_style(&graph, &name, on, live, (x, y)));
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
            ui.set_style(*node, box_style(&graph, name, on, live, self.place(name)));
        }
    }

    /// Choose a box or an arrow without building the canvas again: the
    /// node being pressed stays, so a drag that follows still has it.
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

    /// An arrow from one box to another, in right angles round the
    /// `others` boxes where it can, with a head where it meets the box. `back` moves it aside, so that a pair
    /// of transitions both ways are two arrows.
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
        let shift = if back { 8.0 } else { 0.0 };
        let color = if on { ACCENT } else { NEUTRAL_500 };
        let t = if on { 3.0 } else { 2.0 };
        let (fx, fy) = (from.0 + BOX_W / 2.0 + shift, from.1 + BOX_H / 2.0 + shift);
        let (tx, ty) = (to.0 + BOX_W / 2.0 + shift, to.1 + BOX_H / 2.0 + shift);
        // Routes to try, in order; the first that crosses no other box
        // is drawn, or the first when every one does.
        type Route = (Vec<(f32, f32, f32, f32)>, (f32, f32));
        let mut routes: Vec<Route> = Vec::new();
        let h_seg = |y: f32, a: f32, b: f32| (a.min(b), y - t / 2.0, (b - a).abs() + t, t);
        let v_seg = |x: f32, a: f32, b: f32| (x - t / 2.0, a.min(b), t, (b - a).abs());
        if (fy - ty).abs() < BOX_H {
            // Side by side: straight across to the box's edge…
            let end = if tx > fx { to.0 } else { to.0 + BOX_W };
            routes.push((vec![h_seg(fy, fx, end)], (end, fy)));
            // …or under the row, round whatever is between.
            let below = from.1.max(to.1) + BOX_H + 22.0 + shift;
            routes.push((
                vec![
                    v_seg(fx, from.1 + BOX_H, below),
                    h_seg(below, fx, tx),
                    v_seg(tx, below, to.1 + BOX_H),
                ],
                (tx, to.1 + BOX_H),
            ));
        } else {
            // Along, then down or up into the box…
            let end = if ty > fy { to.1 } else { to.1 + BOX_H };
            routes.push((vec![h_seg(fy, fx, tx), v_seg(tx, fy, end)], (tx, end)));
            // …or down or up first, then along into its side.
            if (fx - tx).abs() > BOX_W {
                let end = if tx > fx { to.0 } else { to.0 + BOX_W };
                routes.push((vec![v_seg(fx, fy, ty), h_seg(ty, fx, end)], (end, ty)));
            }
        }
        let crosses = |(x, y, w, h): (f32, f32, f32, f32)| {
            others
                .iter()
                .any(|&(bx, by)| x < bx + BOX_W && x + w > bx && y < by + BOX_H && y + h > by)
        };
        let pick = routes
            .iter()
            .position(|(segs, _)| !segs.iter().any(|s| crosses(*s)))
            .unwrap_or(0);
        let (segments, head) = routes.swap_remove(pick);
        for (x, y, w, h) in segments {
            // A wider strip to click than to see.
            let hit = ui.add(
                layer,
                Style::row()
                    .absolute(x - 3.0, y - 3.0)
                    .size(w + 6.0, h + 6.0)
                    .padding(3.0)
                    .clickable(),
            );
            ui.set_name(hit, format!("transition {i}"));
            ui.add(hit, Style::default().size(w, h).background(color));
            self.parts.insert(hit, Part::Edge(i));
            self.edge_nodes.push(hit);
        }
        let head_node = ui.add(
            self.heads_layer.unwrap_or(layer),
            Style::default()
                .absolute(head.0 - 5.0, head.1 - 5.0)
                .size(10.0, 10.0)
                .radius(5.0)
                .background(color)
                .clickable(),
        );
        ui.set_name(head_node, format!("transition {i} head"));
        self.parts.insert(head_node, Part::Edge(i));
        self.edge_nodes.push(head_node);
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
        let head = ui.add(side, Style::row().full_width().gap(SPACE_1).center_items());
        let add = button(ui, head, "animator add state", "+ State", false);
        self.parts.insert(add, Part::AddState);
        match self.chosen.clone() {
            None => {
                ui.add_text(
                    side,
                    Style::default().text_size(12.0).text_color(MUTED),
                    "Pick a state or an arrow. Drag a box to move it; drag the ground to pan.",
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
            Part::Box(name) => match event {
                Event::Press { .. } => self.choose(ui, session, Some(Chosen::State(name))),
                Event::Drag { dx, dy, .. } => {
                    let (x, y) = self.place(&name);
                    let (x, y) = (x + dx, y + dy);
                    if let Some((path, _)) = &self.open {
                        self.places
                            .entry(stem(path))
                            .or_default()
                            .insert(name.clone(), (x, y));
                    }
                    if let Some(b) = self.boxes.get(&name) {
                        ui.restyle(*b, |s| s.absolute(x, y));
                    }
                    self.draw_edges(ui);
                }
                Event::DragEnd { .. } => self.save_places(session),
                _ => {}
            },
            Part::Edge(i) if click => self.choose(ui, session, Some(Chosen::Transition(i))),
            Part::Field(key) => {
                if let Event::Submit(value) = event {
                    self.set(session, key, value.trim());
                    self.show(ui, session);
                }
            }
            Part::To(to) if click => {
                if let Some(Chosen::State(from)) = self.chosen.clone() {
                    self.edit(session, |g| {
                        g.transitions.push(Transition {
                            from,
                            to,
                            when: Vec::new(),
                            fade: 0.2,
                        });
                    });
                    let last = self.graph().map_or(0, |g| g.transitions.len() - 1);
                    self.chosen = Some(Chosen::Transition(last));
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
                    self.rename_place(session, &name, value);
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
                        runity::ron::from_str::<Vec<Condition>>(&text)
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

    /// Change the graph and write it.
    fn edit(&mut self, session: &mut Session, change: impl FnOnce(&mut Graph)) {
        let Some((path, graph)) = self.open.as_mut() else {
            return;
        };
        change(graph);
        let old = std::fs::read_to_string(&*path).unwrap_or_default();
        let text = write(&old, graph);
        if text != old {
            if let Err(e) = std::fs::write(&*path, text) {
                session.say(Level::Error, format!("{}: {e}", path.display()));
            }
        }
    }

    fn rename_place(&mut self, session: &Session, old: &str, new: &str) {
        let Some((path, _)) = &self.open else { return };
        if let Some(m) = self.places.get_mut(&stem(path)) {
            if let Some(p) = m.remove(old) {
                m.insert(new.to_string(), p);
            }
        }
        self.save_places(session);
    }

    fn save_places(&self, session: &Session) {
        let Some(path) = Self::places_file(session) else {
            return;
        };
        let pretty = runity::ron::ser::PrettyConfig::new();
        if let Ok(text) = runity::ron::ser::to_string_pretty(&self.places, pretty) {
            let _ = std::fs::create_dir_all(path.parent().expect("a file in .runity"));
            let _ = std::fs::write(path, text + "\n");
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
        let text = "(\n    start: \"idle\",\n    states: {\n        \"idle\": (clip: \"idle\"),\n    },\n    transitions: [],\n)\n";
        match std::fs::write(&path, text) {
            Ok(()) => self.open(ui, session, path),
            Err(e) => session.say(Level::Error, format!("{}: {e}", path.display())),
        }
    }
}

/// A state's column and row in the default layout.
fn layered(graph: &Graph, name: &str) -> (usize, usize) {
    let mut depth: BTreeMap<&str, usize> = BTreeMap::new();
    if graph.states.contains_key(&graph.start) {
        depth.insert(&graph.start, 0);
        let mut frontier = vec![graph.start.as_str()];
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for from in frontier {
                let d = depth[from];
                for t in graph.transitions.iter().filter(|t| t.from == from) {
                    if graph.states.contains_key(&t.to) && !depth.contains_key(t.to.as_str()) {
                        depth.insert(&t.to, d + 1);
                        next.push(t.to.as_str());
                    }
                }
            }
            frontier = next;
        }
    }
    // Reached from nowhere but Any State: the first column over.
    let column_of = |n: &str| depth.get(n).copied().unwrap_or(1);
    let column = column_of(name);
    let row = graph
        .states
        .keys()
        .filter(|k| column_of(k) == column)
        .position(|k| k == name)
        .unwrap_or(0);
    (column, row)
}

/// A state's box: the start filled with the accent, the chosen one ringed
/// in it, and the one the running game is in ringed in warm light.
fn box_style(graph: &Graph, name: &str, on: bool, live: bool, (x, y): (f32, f32)) -> Style {
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
            if on || live { 2.0 } else { 1.0 },
            if live {
                WARNING
            } else if on {
                ACCENT
            } else {
                NEUTRAL_800
            },
        )
        .clickable()
        .draggable()
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
        events: Vec::new(),
        looping: true,
        speed: 1.0,
        speed_from: None,
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

fn number(n: f32) -> String {
    let s = format!("{n:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".into()
    } else {
        s.into()
    }
}

fn quote(s: &str) -> String {
    format!("{s:?}")
}

/// `[(0, "idle"), (2, "walk")]`, as a person writes it.
fn pairs(p: &[(f32, String)]) -> String {
    let inner: Vec<String> = p
        .iter()
        .map(|(at, name)| format!("({}, {})", number(*at), quote(name)))
        .collect();
    format!("[{}]", inner.join(", "))
}

fn read_pairs(text: &str) -> Result<Vec<(f32, String)>, String> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    runity::ron::from_str(text).map_err(|e| e.to_string())
}

fn condition(c: &Condition) -> String {
    match c {
        Condition::Above(p, v) => format!("Above({}, {})", quote(p), number(*v)),
        Condition::Below(p, v) => format!("Below({}, {})", quote(p), number(*v)),
        Condition::Is(p) => format!("Is({})", quote(p)),
        Condition::Not(p) => format!("Not({})", quote(p)),
        Condition::Trigger(p) => format!("Trigger({})", quote(p)),
        Condition::Finished => "Finished".into(),
    }
}

fn conditions(when: &[Condition]) -> String {
    let inner: Vec<String> = when.iter().map(condition).collect();
    format!("[{}]", inner.join(", "))
}

/// A state's entry as the files are written by hand: what differs from
/// the defaults, on one line.
fn state_entry(name: &str, s: &State) -> String {
    let mut fields = Vec::new();
    if s.blend.is_empty() || !s.clip.is_empty() {
        fields.push(format!("clip: {}", quote(&s.clip)));
    }
    if !s.blend_by.is_empty() {
        fields.push(format!("blend_by: {}", quote(&s.blend_by)));
    }
    if !s.blend.is_empty() {
        fields.push(format!("blend: {}", pairs(&s.blend)));
    }
    if !s.events.is_empty() {
        fields.push(format!("events: {}", pairs(&s.events)));
    }
    if !s.looping {
        fields.push("looping: false".into());
    }
    if s.speed != 1.0 {
        fields.push(format!("speed: {}", number(s.speed)));
    }
    if let Some(p) = &s.speed_from {
        fields.push(format!("speed_from: {}", quote(p)));
    }
    format!("{}: ({})", quote(name), fields.join(", "))
}

fn transition_entry(t: &Transition) -> String {
    let mut out = format!("(from: {}, to: {}", quote(&t.from), quote(&t.to));
    if !t.when.is_empty() {
        out += &format!(", when: {}", conditions(&t.when));
    }
    if t.fade != 0.2 {
        out += &format!(", fade: {}", number(t.fade));
    }
    out + ")"
}

/// The file `old` with `graph` written into it where it differs, or the
/// graph written anew when the text is past patching.
pub fn write(old: &str, graph: &Graph) -> String {
    patched(old, graph).unwrap_or_else(|| {
        let pretty = runity::ron::ser::PrettyConfig::new();
        runity::ron::ser::to_string_pretty(graph, pretty).unwrap_or_default() + "\n"
    })
}

fn patched(old: &str, graph: &Graph) -> Option<String> {
    let was: Graph = runity::ron::from_str(old).ok()?;
    let mut text = old.to_string();
    // Transitions, by place: one changed, one added at the end, one gone.
    if was.transitions != graph.transitions {
        let (a, b) = (&was.transitions, &graph.transitions);
        let changes: Vec<Change> = if a.len() == b.len() {
            (0..a.len())
                .filter(|&i| a[i] != b[i])
                .map(|i| Change::Replace(i, transition_entry(&b[i])))
                .collect()
        } else if b.len() > a.len() && b[..a.len()] == a[..] {
            b[a.len()..]
                .iter()
                .map(|t| Change::Append(transition_entry(t)))
                .collect()
        } else if b.len() < a.len() {
            // Some gone, the rest in order: remove the ones not kept.
            let mut kept = b.iter().peekable();
            let mut gone = Vec::new();
            for (i, t) in a.iter().enumerate() {
                if kept.peek() == Some(&t) {
                    kept.next();
                } else {
                    gone.push(Change::Remove(i));
                }
            }
            if kept.next().is_some() {
                return None;
            }
            gone
        } else {
            return None;
        };
        let open = patch::value_start(&text, "transitions")?;
        text = patch::apply(&text, open, &changes)?;
    }
    // States, by name, in the order the file has them.
    if was.states != graph.states {
        let open = patch::value_start(&text, "states")?;
        let found = patch::items(&text, open)?;
        let keys: Vec<String> = found
            .items
            .iter()
            .map(|r| {
                let item = &text[r.clone()];
                runity::ron::from_str::<String>(item.split(':').next().unwrap_or("").trim())
                    .unwrap_or_default()
            })
            .collect();
        let mut changes = Vec::new();
        for (i, key) in keys.iter().enumerate() {
            match graph.states.get(key) {
                None => changes.push(Change::Remove(i)),
                Some(s) if was.states.get(key) != Some(s) => {
                    changes.push(Change::Replace(i, state_entry(key, s)))
                }
                Some(_) => {}
            }
        }
        for (key, s) in &graph.states {
            if !keys.contains(key) {
                changes.push(Change::Append(state_entry(key, s)));
            }
        }
        text = patch::apply(&text, open, &changes)?;
    }
    if was.start != graph.start {
        let at = patch::value_start(&text, "start")?;
        let span = patch::value_span(&text, at)?;
        text.replace_range(span, &quote(&graph.start));
    }
    let check: Graph = runity::ron::from_str(&text).ok()?;
    (check == *graph).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"// The hero.
(
    start: "idle",
    states: {
        "idle": (clip: "idle"), // standing
        "walk": (clip: "walk", speed_from: "speed"),
    },
    transitions: [
        (from: "idle", to: "walk", when: [Above("speed", 0.1)]),
        // Back when slow.
        (from: "walk", to: "idle", when: [Below("speed", 0.1)]),
    ],
)
"#;

    #[test]
    fn edits_keep_the_file() {
        let mut g: Graph = runity::ron::from_str(FILE).unwrap();
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
        assert!(out.contains(r#"(from: "*", to: "jump", when: [Trigger("jump")], fade: 0.1),"#));
        assert!(out.contains(r#""walk": (clip: "walk", speed: 1.5, speed_from: "speed")"#));
        assert!(out.contains(r#"start: "walk","#));
        assert_eq!(runity::ron::from_str::<Graph>(&out).unwrap(), g);

        // A state and its transitions gone.
        g.states.remove("idle");
        g.transitions.retain(|t| t.from != "idle" && t.to != "idle");
        let out = write(&out, &g);
        assert_eq!(runity::ron::from_str::<Graph>(&out).unwrap(), g);
        assert!(
            out.starts_with("// The hero."),
            "patched, not rewritten: {out}"
        );
    }
}
