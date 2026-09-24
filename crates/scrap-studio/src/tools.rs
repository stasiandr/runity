//! Project Settings and Profiler: two panels a dock can hold.
//!
//! **Project Settings** is Unity's at the grain this engine has them: the
//! project's own files, in git — `scrap.ron` (its modules, the game),
//! `input.ron`, `layers.ron`, the tuning numbers in `tuning/`, the screens
//! in `ui/` — listed, one open in a field of several lines, saved only
//! when it still reads as RON. The engine picks the change up from disk
//! as it picks up any other (DNA, postulate 1); nothing here knows what
//! the fields mean, so nothing here goes stale when they grow. What is
//! the person's and not the project's — colours, keys, tools — is
//! Preferences (`crate::preferences`), a window of its own.
//!
//! **Profiler** is what a frame costs, as Unity's Profiler shows it (DNA,
//! postulate 6): the editor's last seconds as bars, each split into the
//! Scene view's input, the render, the panels and the UI, under lines at
//! the 60 and 30 fps budgets, hitches tinted, the frame under the pointer
//! spelled out; then tables — each part's median, p99, worst and hitches,
//! the running game's systems as it reports them, and the Scene view's
//! passes on the GPU when timing is switched on. Pause holds it still.

use std::path::PathBuf;

use scrap_editor::console::Level;
use scrap_editor::Session;
use scrap_ui::{Event, NodeId, Style, Ui};

use crate::theme::*;

pub struct Settings {
    pub root: NodeId,
    list: NodeId,
    editor: NodeId,
    title: NodeId,
    save: NodeId,
    files: Vec<(NodeId, PathBuf)>,
    open: Option<PathBuf>,
    listed: bool,
}

impl Settings {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let root = ui.add(parent, Style::row().fill().full_width());
        ui.set_name(root, "settings");
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
            "PROJECT FILES",
        );
        let list = ui.add(left, Style::column().fill().full_width().gap(2.0).clip());
        ui.set_name(list, "settings files");
        let right = ui.add(
            root,
            Style::column()
                .fill()
                .full_height()
                .padding(SPACE_2)
                .gap(SPACE_2),
        );
        let bar = ui.add(right, Style::row().full_width().gap(SPACE_2).center_items());
        let title = ui.add_text(
            bar,
            text().fill().text_color(LABEL),
            "Choose a file on the left.",
        );
        let save = button(ui, bar, "settings save", "Save", true);
        let scroll = ui.add(right, Style::column().fill().full_width().clip());
        let editor = ui.add_textarea(
            scroll,
            field_style()
                .full_width()
                .auto_height()
                .min_height(80.0)
                .padding_y(6.0)
                .mono()
                .text_size(12.0),
            "",
        );
        ui.set_name(editor, "settings text");
        Self {
            root,
            list,
            editor,
            title,
            save,
            files: Vec::new(),
            open: None,
            listed: false,
        }
    }

    /// The project's settings files, in the order they matter.
    fn files(session: &Session) -> Vec<PathBuf> {
        let Some(project) = session.project() else {
            return Vec::new();
        };
        let root = project.root();
        let mut out = vec![root.join(scrap::project::FILE)];
        for file in [scrap::project::INPUT, scrap::layers::FILE] {
            let path = root.join(file);
            if path.is_file() {
                out.push(path);
            }
        }
        for dir in [scrap::project::TUNING, scrap::project::UI] {
            if let Ok(read) = std::fs::read_dir(root.join(dir)) {
                let mut more: Vec<PathBuf> = read
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "ron"))
                    .collect();
                more.sort();
                out.extend(more);
            }
        }
        out
    }

    pub fn update(&mut self, ui: &mut Ui, session: &mut Session) {
        if self.listed {
            return;
        }
        self.listed = true;
        ui.clear(self.list);
        self.files.clear();
        let root = session
            .project()
            .map(|p| p.root().to_path_buf())
            .unwrap_or_default();
        for path in Self::files(session) {
            let name = path
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let row = settings_row(ui, self.list, "file", &name);
            ui.set_name(row, format!("settings {name}"));
            self.files.push((row, path));
        }
        // Open on the first, `scrap.ron`: an empty page says nothing.
        if self.open.is_none() {
            if let Some((_, path)) = self.files.first().cloned() {
                self.open(ui, session, path);
            }
        }
    }

    fn open(&mut self, ui: &mut Ui, session: &mut Session, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                ui.set_text(self.editor, text.trim_end());
                let root = session
                    .project()
                    .map(|p| p.root().to_path_buf())
                    .unwrap_or_default();
                let name = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                ui.set_text(self.title, &name);
                ui.restyle(self.title, |s| s.text_color(TEXT));
                for (row, p) in &self.files {
                    let on = *p == path;
                    ui.restyle(*row, |s| {
                        s.background(if on {
                            ACCENT_900
                        } else {
                            scrap_ui::Color::TRANSPARENT
                        })
                    });
                }
                self.open = Some(path);
            }
            Err(e) => session.say(Level::Error, format!("{}: {e}", path.display())),
        }
    }

    /// Write the open file back, if it still reads as RON.
    fn save(&mut self, ui: &mut Ui, session: &mut Session) {
        let Some(path) = self.open.clone() else {
            return;
        };
        let text = ui.text(self.editor).unwrap_or_default().to_string();
        if let Err(e) = scrap::ron::from_str::<scrap::ron::Value>(&text) {
            session.say(Level::Error, format!("{}: not saved, {e}", path.display()));
            return;
        }
        match std::fs::write(&path, format!("{text}\n")) {
            Ok(()) => session.say(Level::Info, format!("saved {}", path.display())),
            Err(e) => session.say(Level::Error, format!("{}: {e}", path.display())),
        }
    }

    pub fn owns(&self, ui: &Ui, node: NodeId) -> bool {
        ancestor(ui, node, self.root)
    }

    pub fn event(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, event: &Event) {
        match event {
            Event::Click { .. } if node == self.save => self.save(ui, session),
            Event::Submit(_) if node == self.editor => self.save(ui, session),
            Event::Click { .. } => {
                if let Some(path) = self
                    .files
                    .iter()
                    .find(|(n, _)| *n == node)
                    .map(|(_, p)| p.clone())
                {
                    self.open(ui, session, path);
                }
            }
            _ => {}
        }
    }
}

/// A line of the list on the left: an icon and a name.
fn settings_row(ui: &mut Ui, parent: NodeId, glyph: &str, name: &str) -> NodeId {
    let row = ui.add(
        parent,
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
    icon(ui, row, glyph, MUTED);
    ui.add_text(row, text(), name);
    row
}

/// Whether `node` is `root` or under it.
fn ancestor(ui: &Ui, node: NodeId, root: NodeId) -> bool {
    let mut at = Some(node);
    while let Some(n) = at {
        if n == root {
            return true;
        }
        at = ui.parent(n);
    }
    false
}

/// A frame's cost, by part, in milliseconds.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameCost {
    pub input: f32,
    pub render: f32,
    pub panels: f32,
    pub ui: f32,
}

impl FrameCost {
    fn total(&self) -> f32 {
        self.input + self.render + self.panels + self.ui
    }

    /// The parts as the legend lists them, then the whole frame.
    fn parts(&self) -> [f32; 5] {
        [self.input, self.render, self.panels, self.ui, self.total()]
    }
}

/// How many frames the Profiler keeps.
const FRAMES: usize = 120;
/// A bar's width and the gap after it: where the pointer is, as a frame.
const BAR: f32 = 4.0;
const BAR_GAP: f32 = 1.0;
/// A table's names: wide enough for "scene input".
const NAME: f32 = 84.0;
/// A table's number column.
const CELL: f32 = 56.0;
/// The budget lines: 60 and 30 frames a second.
const BUDGETS: [f32; 2] = [1000.0 / 60.0, 1000.0 / 30.0];
/// How often the running game's report is read from disk: it writes one
/// about twice a second.
const GAME_EVERY: std::time::Duration = std::time::Duration::from_millis(500);

/// The parts' names, as the legend and the table say them.
const PARTS: [&str; 4] = ["scene input", "render", "panels", "UI"];

pub struct Profiler {
    pub root: NodeId,
    chart: NodeId,
    bars: NodeId,
    /// Each budget's line and its label, over the bars.
    budgets: Vec<(NodeId, NodeId)>,
    summary: NodeId,
    detail: NodeId,
    pause: NodeId,
    gpu: NodeId,
    /// The editor table's number cells: a row per part, then the frame.
    part_cells: Vec<Vec<NodeId>>,
    systems_title: NodeId,
    systems_note: NodeId,
    systems: NodeId,
    passes_title: NodeId,
    passes_note: NodeId,
    passes: NodeId,
    frames: std::collections::VecDeque<FrameCost>,
    /// The same frames by part, for the median, p99 and hitches.
    times: Vec<scrap::FrameTimes>,
    paused: bool,
    /// The game's systems as it last reported them, and when that was read.
    game: Vec<(String, f32, f32)>,
    game_read: Option<std::time::Instant>,
}

/// The parts' colours: the three accents of the gizmo and a neutral.
fn part_colors() -> [scrap_ui::Color; 4] {
    [AXIS_Y, AXIS_Z, ACCENT, NEUTRAL_500]
}

/// Small numbers in the tables and over the chart.
fn small() -> Style {
    Style::default().text_size(11.0).text_color(MUTED).nowrap()
}

/// A table's word when it has no rows: wrapped, it may be long.
fn note() -> Style {
    Style::default()
        .text_size(11.0)
        .text_color(MUTED)
        .full_width()
        .padding_x(SPACE_1)
}

/// A table of the Profiler: its title and column heads, and a body for
/// its rows. Returns the section, the title's text and the body.
fn table(
    ui: &mut Ui,
    parent: NodeId,
    name: &str,
    title: &str,
    heads: &[&str],
) -> (NodeId, NodeId, NodeId) {
    // As wide as its columns: a narrow panel wraps the tables instead.
    let wide = NAME + 20.0 + heads.len() as f32 * (CELL + SPACE_2);
    let section = ui.add(parent, Style::column().fill().min_width(wide).gap(2.0));
    ui.set_name(section, format!("profiler {name}"));
    let head = ui.add(
        section,
        Style::row()
            .full_width()
            .height(20.0)
            .fixed()
            .center_items()
            .gap(SPACE_2),
    );
    let title = ui.add_text(head, caption().fill().min_width(NAME), title);
    ui.set_name(title, format!("profiler {name} title"));
    for h in heads {
        let c = cell(ui, head, h);
        ui.restyle(c, |_| caption());
    }
    let body = ui.add(section, Style::column().full_width().gap(1.0));
    ui.set_name(body, format!("profiler {name} rows"));
    (section, title, body)
}

/// A number, right-aligned in a column of its own.
fn cell(ui: &mut Ui, row: NodeId, value: &str) -> NodeId {
    let c = ui.add(row, Style::row().width(CELL).fixed().center_items());
    spacer(ui, c);
    ui.add_text(c, small().mono(), value)
}

/// A table's line: a swatch (or a gap where one would be), the name, and
/// `cells` numbers.
fn table_row(
    ui: &mut Ui,
    parent: NodeId,
    swatch: Option<scrap_ui::Color>,
    name: &str,
    cells: usize,
) -> (NodeId, Vec<NodeId>) {
    let row = ui.add(
        parent,
        Style::row()
            .full_width()
            .height(20.0)
            .fixed()
            .padding_x(SPACE_1)
            .gap(SPACE_2)
            .center_items()
            .radius(RADIUS_SM)
            .hover(HOVER),
    );
    ui.add(
        row,
        Style::row()
            .size(8.0, 8.0)
            .fixed()
            .radius(2.0)
            .background(swatch.unwrap_or(scrap_ui::Color::TRANSPARENT)),
    );
    ui.add_text(row, text().fill().min_width(NAME).text_size(11.5), name);
    let cells = (0..cells).map(|_| cell(ui, row, "")).collect();
    (row, cells)
}

fn ms(v: f32) -> String {
    format!("{v:.2}")
}

fn duration_ms(d: std::time::Duration) -> f32 {
    d.as_secs_f32() * 1e3
}

/// The chart's top: 33 ms, doubled until all but the slowest few frames
/// fit — one frame that built a pipeline should not flatten the rest; it
/// is cut at the top, and pointing at it reads its number.
fn chart_top(frames: &std::collections::VecDeque<FrameCost>) -> f32 {
    let mut totals: Vec<f32> = frames.iter().map(FrameCost::total).collect();
    totals.sort_by(f32::total_cmp);
    let tall = totals
        .get((totals.len() as f32 * 0.95) as usize)
        .or(totals.last())
        .copied()
        .unwrap_or(0.0);
    let mut top = BUDGETS[1];
    while top < tall * 1.05 && top < 1000.0 {
        top *= 2.0;
    }
    top
}

impl Profiler {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let root = ui.add(
            parent,
            Style::column()
                .fill()
                .full_width()
                .padding(SPACE_2)
                .gap(SPACE_2),
        );
        ui.set_name(root, "profiler");
        let legend = ui.add(root, Style::row().full_width().gap(SPACE_4).center_items());
        for (label, color) in PARTS.into_iter().zip(part_colors()) {
            let item = ui.add(legend, Style::row().gap(SPACE_1).center_items());
            ui.add(
                item,
                Style::row().size(10.0, 10.0).radius(2.0).background(color),
            );
            ui.add_text(
                item,
                Style::default().text_size(11.0).text_color(LABEL).nowrap(),
                label,
            );
        }
        spacer(ui, legend);
        let summary = ui.add_text(legend, small(), "");
        ui.set_name(summary, "profiler summary");
        let gpu = button(ui, legend, "profiler gpu", "GPU timing", false);
        let pause = button(ui, legend, "profiler pause", "Pause", false);
        let chart = ui.add(
            root,
            Style::column()
                .fill()
                .min_height(60.0)
                .full_width()
                .radius(RADIUS_SM)
                .background(BG)
                .clip(),
        );
        ui.set_name(chart, "profiler chart");
        let bars = ui.add(chart, Style::row().fill().full_width().gap(BAR_GAP));
        ui.set_name(bars, "profiler bars");
        let budgets = BUDGETS
            .iter()
            .map(|budget| {
                let line = ui.add(
                    chart,
                    Style::row()
                        .absolute(0.0, 0.0)
                        .full_width()
                        .height(1.0)
                        .fixed()
                        .background(DIVIDER),
                );
                let label = ui.add_text(
                    chart,
                    small()
                        .absolute(SPACE_2, 0.0)
                        .padding_x(SPACE_1)
                        .radius(RADIUS_SM)
                        .background(BG.alpha(80)),
                    &format!("{budget:.1} ms · {:.0} fps", 1000.0 / budget),
                );
                (line, label)
            })
            .collect();
        let detail = ui.add_text(root, small(), "");
        ui.set_name(detail, "profiler detail");
        let tables = ui.add(root, Style::row().full_width().wrap().gap(SPACE_6));
        let (_, _, editor) = table(
            ui,
            tables,
            "editor",
            "EDITOR FRAME, MS",
            &["median", "p99", "worst", "hitches"],
        );
        let colors = part_colors();
        let mut part_cells = Vec::new();
        for (i, name) in PARTS.into_iter().chain(["frame"]).enumerate() {
            let (row, cells) = table_row(ui, editor, colors.get(i).copied(), name, 4);
            ui.set_name(row, format!("profiler part {name}"));
            part_cells.push(cells);
        }
        let (systems_section, systems_title, systems) = table(
            ui,
            tables,
            "systems",
            "GAME SYSTEMS, MS",
            &["median", "worst"],
        );
        let systems_note = ui.add_text(systems_section, note(), "");
        ui.set_name(systems_note, "profiler systems note");
        let (passes_section, passes_title, passes) =
            table(ui, tables, "passes", "GPU PASSES, MS", &["ms", "share"]);
        let passes_note = ui.add_text(passes_section, note(), "");
        ui.set_name(passes_note, "profiler passes note");
        Self {
            root,
            chart,
            bars,
            budgets,
            summary,
            detail,
            pause,
            gpu,
            part_cells,
            systems_title,
            systems_note,
            systems,
            passes_title,
            passes_note,
            passes,
            frames: std::collections::VecDeque::with_capacity(FRAMES),
            times: (0..5).map(|_| scrap::FrameTimes::new(FRAMES)).collect(),
            paused: false,
            game: Vec::new(),
            game_read: None,
        }
    }

    /// Keep a frame's cost — unless paused, when the chart holds still to
    /// be read.
    pub fn record(&mut self, cost: FrameCost) {
        if self.paused {
            return;
        }
        if self.frames.len() == FRAMES {
            self.frames.pop_front();
        }
        self.frames.push_back(cost);
        for (times, part) in self.times.iter_mut().zip(cost.parts()) {
            times.record(std::time::Duration::from_secs_f32(part.max(0.0) / 1e3));
        }
    }

    /// Draw it all: the bars (taller is slower, the budgets as lines), the
    /// frame under the pointer, and the three tables.
    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        let summary = self.times[4].summary();
        self.chart(ui, summary.map(|s| duration_ms(s.median)));
        self.editor_table(ui);
        match summary {
            Some(s) => ui.set_text(
                self.summary,
                &format!(
                    "median {:.1} ms · p99 {:.1} · worst {:.1} · {} hitches / {} frames",
                    duration_ms(s.median),
                    duration_ms(s.p99),
                    duration_ms(s.worst),
                    s.hitches,
                    s.frames
                ),
            ),
            None => ui.set_text(self.summary, ""),
        }
        self.systems_table(ui, session);
        self.passes_table(ui, session);
        set_button_primary(ui, self.pause, self.paused);
        set_button_primary(ui, self.gpu, session.profiling_gpu());
        if let Some(label) = ui.children(self.pause).first().copied() {
            ui.set_text(label, if self.paused { "Resume" } else { "Pause" });
        }
    }

    fn chart(&mut self, ui: &mut Ui, median: Option<f32>) {
        let keys: Vec<usize> = (0..self.frames.len()).collect();
        let colors = part_colors();
        ui.sync_children(
            self.bars,
            &keys,
            |ui, bars, _| {
                let bar = ui.add(bars, Style::column().width(BAR).full_height().fixed());
                ui.add(bar, Style::row().fill());
                for c in colors {
                    ui.add(
                        bar,
                        Style::row().full_width().height(0.0).fixed().background(c),
                    );
                }
                bar
            },
            |_, _, _| {},
        );
        let area = ui.rect(self.bars);
        let height = area.height.max(1.0);
        let top = chart_top(&self.frames);
        let scale = height / top;
        // The frame under the pointer, if it is over the chart.
        let (px, py) = ui.pointer();
        let chart = ui.rect(self.chart);
        let over = (px >= chart.x
            && px < chart.x + chart.width
            && py >= chart.y
            && py < chart.y + chart.height)
            .then(|| ((px - area.x) / (BAR + BAR_GAP)).floor())
            .filter(|i| *i >= 0.0)
            .map(|i| i as usize)
            .filter(|i| *i < self.frames.len());
        let hitch = median.map_or(f32::INFINITY, |m| m * 2.0);
        for (i, (bar, cost)) in ui
            .children(self.bars)
            .into_iter()
            .zip(self.frames.iter().copied())
            .enumerate()
        {
            let kids = ui.children(bar);
            // Top to bottom as the legend reads.
            for (node, ms) in kids[1..]
                .iter()
                .zip([cost.input, cost.render, cost.panels, cost.ui])
            {
                let h = (ms * scale).min(height);
                ui.restyle(*node, |s| s.height(h));
            }
            // A hitch — longer than twice the median — is what a person
            // feels: tinted, so the eye finds it.
            let ground = if over == Some(i) {
                TEXT.alpha(20)
            } else if cost.total() > hitch {
                ERROR.alpha(28)
            } else {
                scrap_ui::Color::TRANSPARENT
            };
            ui.restyle(bar, |s| s.background(ground));
        }
        // A label a line too close to the next one's would overlap it: the
        // lower budget goes unlabelled then.
        let ys: Vec<f32> = BUDGETS
            .iter()
            .map(|b| (height - b * scale).max(0.0))
            .collect();
        for (i, (line, label)) in self.budgets.iter().enumerate() {
            let y = ys[i];
            let crowded = ys.get(i + 1).is_some_and(|next| y - next < 16.0);
            ui.restyle(*line, |s| s.absolute(0.0, y));
            ui.restyle(*label, |s| {
                let s = s.absolute(SPACE_2, (y - 14.0).max(0.0));
                if crowded {
                    s.hidden()
                } else {
                    s.shown()
                }
            });
        }
        let detail = match over.and_then(|i| Some((i, self.frames.get(i)?))) {
            Some((i, cost)) => format!(
                "{} frames ago: {:.2} ms — scene input {:.2} · render {:.2} · panels {:.2} · UI {:.2}",
                self.frames.len() - 1 - i,
                cost.total(),
                cost.input,
                cost.render,
                cost.panels,
                cost.ui
            ),
            None if self.paused => "Paused: point at a bar to read its frame.".to_string(),
            None => format!("Top of the chart: {top:.0} ms. Point at a bar to read its frame."),
        };
        ui.set_text(self.detail, &detail);
    }

    fn editor_table(&mut self, ui: &mut Ui) {
        for (cells, times) in self.part_cells.iter().zip(&self.times) {
            let values = match times.summary() {
                Some(s) => [
                    ms(duration_ms(s.median)),
                    ms(duration_ms(s.p99)),
                    ms(duration_ms(s.worst)),
                    s.hitches.to_string(),
                ],
                None => Default::default(),
            };
            for (cell, value) in cells.iter().zip(values) {
                ui.set_text(*cell, &value);
            }
        }
    }

    /// What the game started from here says its systems cost, read from
    /// its report every half a second.
    fn systems_table(&mut self, ui: &mut Ui, session: &Session) {
        if !session.is_game_running() {
            self.game.clear();
            self.game_read = None;
        } else if !self.paused && self.game_read.is_none_or(|t| t.elapsed() >= GAME_EVERY) {
            self.game_read = Some(std::time::Instant::now());
            if let Some(systems) = session
                .game_state()
                .and_then(|s| s.diagnostics)
                .map(|d| d.systems)
            {
                self.game = systems;
            }
        }
        let note = if !session.is_game_running() {
            "Run the game (Play ▸ Run Game) to see what its systems cost."
        } else if self.game.is_empty() {
            "Waiting for the game's first report…"
        } else {
            ""
        };
        ui.set_text(self.systems_note, note);
        let total: f32 = self.game.iter().map(|(_, median, _)| median).sum();
        ui.set_text(
            self.systems_title,
            &if self.game.is_empty() {
                "GAME SYSTEMS, MS".to_string()
            } else {
                format!("GAME SYSTEMS · {total:.2}")
            },
        );
        let names: Vec<String> = self.game.iter().map(|(n, _, _)| n.clone()).collect();
        let game = &self.game;
        let fill = |ui: &mut Ui, row: NodeId, name: &String| {
            let Some((_, median, worst)) = game.iter().find(|(n, _, _)| n == name) else {
                return;
            };
            let cells: Vec<NodeId> = ui
                .children(row)
                .into_iter()
                .skip(2)
                .filter_map(|c| ui.children(c).get(1).copied())
                .collect();
            for (cell, v) in cells.into_iter().zip([median, worst]) {
                ui.set_text(cell, &ms(*v));
            }
        };
        ui.sync_children(
            self.systems,
            &names,
            |ui, parent, name| {
                let (row, _) = table_row(ui, parent, None, name, 2);
                ui.set_name(row, format!("profiler system {name}"));
                fill(ui, row, name);
                row
            },
            |ui, row, name| fill(ui, row, name),
        );
    }

    /// Each pass of the Scene view on the GPU, while timing is on.
    fn passes_table(&mut self, ui: &mut Ui, session: &Session) {
        let on = session.profiling_gpu();
        let passes = if on { session.gpu_times() } else { Vec::new() };
        let note = if !on {
            "Off. GPU timing reads back a query set each frame; turn it on above."
        } else if passes.is_empty() {
            "Waiting for the GPU — or this device has no timestamps."
        } else {
            ""
        };
        ui.set_text(self.passes_note, note);
        let total: f32 = passes.iter().map(|(_, t)| t).sum();
        ui.set_text(
            self.passes_title,
            &if passes.is_empty() {
                "GPU PASSES, MS".to_string()
            } else {
                format!("GPU PASSES · {total:.2}")
            },
        );
        let names: Vec<String> = passes.iter().map(|(n, _)| n.clone()).collect();
        let fill = |ui: &mut Ui, row: NodeId, name: &String| {
            let Some((_, t)) = passes.iter().find(|(n, _)| n == name) else {
                return;
            };
            let cells: Vec<NodeId> = ui
                .children(row)
                .into_iter()
                .skip(2)
                .filter_map(|c| ui.children(c).get(1).copied())
                .collect();
            let share = if total > 0.0 { t / total * 100.0 } else { 0.0 };
            for (cell, v) in cells.into_iter().zip([ms(*t), format!("{share:.0}%")]) {
                ui.set_text(cell, &v);
            }
        };
        ui.sync_children(
            self.passes,
            &names,
            |ui, parent, name| {
                let (row, _) = table_row(ui, parent, None, name, 2);
                ui.set_name(row, format!("profiler pass {name}"));
                fill(ui, row, name);
                row
            },
            |ui, row, name| fill(ui, row, name),
        );
    }

    pub fn owns(&self, ui: &Ui, node: NodeId) -> bool {
        ancestor(ui, node, self.root)
    }

    pub fn event(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, event: &Event) {
        let Event::Click { .. } = event else { return };
        if node == self.pause {
            self.paused = !self.paused;
        } else if node == self.gpu {
            let on = !session.profiling_gpu();
            session.profile_gpu(on);
        } else {
            return;
        }
        self.update(ui, session);
    }
}

/// Animation: the selection's clips, to play in the view — Unity's
/// Animation window with its preview on. The document never changes;
/// see `Session::preview_clip`.
pub struct Animation {
    pub root: NodeId,
    title: NodeId,
    list: NodeId,
    stop: NodeId,
    speeds: Vec<(NodeId, f32)>,
    clips: Vec<(NodeId, usize)>,
    showing: Option<scrap::EntityId>,
    playing: Option<usize>,
    speed: f32,
}

impl Animation {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let root = ui.add(
            parent,
            Style::column()
                .fill()
                .full_width()
                .padding(SPACE_2)
                .gap(SPACE_2),
        );
        ui.set_name(root, "animation");
        let bar = ui.add(root, Style::row().full_width().gap(SPACE_2).center_items());
        let title = ui.add_text(bar, text().fill().text_color(LABEL), "");
        let mut speeds = Vec::new();
        for (label, speed) in [("½×", 0.5), ("1×", 1.0), ("2×", 2.0)] {
            let b = button(ui, bar, &format!("speed {label}"), label, speed == 1.0);
            speeds.push((b, speed));
        }
        let stop = button(ui, bar, "animation stop", "Stop", false);
        let list = ui.add(root, Style::column().fill().full_width().gap(2.0).clip());
        ui.set_name(list, "animation clips");
        Self {
            root,
            title,
            list,
            stop,
            speeds,
            clips: Vec::new(),
            showing: None,
            playing: None,
            speed: 1.0,
        }
    }

    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        let selected = session.selected();
        if selected == self.showing && !self.clips.is_empty() {
            return;
        }
        self.showing = selected;
        self.playing = None;
        ui.clear(self.list);
        self.clips.clear();
        let Some(id) = selected else {
            ui.set_text(self.title, "Select something with a skinned model.");
            return;
        };
        let clips = session.clips(id);
        let name = session.entity_name(id).unwrap_or_default();
        if clips.is_empty() {
            ui.set_text(
                self.title,
                &format!("{name}: no clips — its model has no skeleton"),
            );
            return;
        }
        ui.set_text(self.title, &format!("{name}: {} clips", clips.len()));
        for (i, (clip, seconds)) in clips.into_iter().enumerate() {
            let row = ui.add(
                self.list,
                Style::row()
                    .full_width()
                    .height(26.0)
                    .fixed()
                    .padding_x(SPACE_3)
                    .gap(SPACE_2)
                    .center_items()
                    .radius(RADIUS_SM)
                    .hover(HOVER),
            );
            ui.set_name(row, format!("clip {clip}"));
            icon(ui, row, "play", ACCENT);
            ui.add_text(row, text().fill(), &clip);
            ui.add_text(
                row,
                Style::default().text_size(11.0).text_color(MUTED).nowrap(),
                &format!("{seconds:.2} s"),
            );
            self.clips.push((row, i));
        }
    }

    fn mark(&self, ui: &mut Ui) {
        for (row, i) in &self.clips {
            let on = Some(*i) == self.playing;
            ui.restyle(*row, |s| {
                s.background(if on {
                    ACCENT_900
                } else {
                    scrap_ui::Color::TRANSPARENT
                })
            });
        }
        for (b, speed) in &self.speeds {
            set_button_primary(ui, *b, *speed == self.speed);
        }
    }

    pub fn owns(&self, ui: &Ui, node: NodeId) -> bool {
        ancestor(ui, node, self.root)
    }

    pub fn event(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, event: &Event) {
        let Event::Click { .. } = event else { return };
        let Some(id) = self.showing else { return };
        let result = if node == self.stop {
            self.playing = None;
            session.preview_clip(id, None, self.speed)
        } else if let Some((_, speed)) = self.speeds.iter().find(|(b, _)| *b == node) {
            self.speed = *speed;
            match self.playing {
                Some(clip) => session.preview_clip(id, Some(clip), self.speed),
                None => Ok(()),
            }
        } else if let Some((_, clip)) = self.clips.iter().find(|(r, _)| *r == node) {
            self.playing = Some(*clip);
            session.preview_clip(id, Some(*clip), self.speed)
        } else {
            return;
        };
        if let Err(e) = result {
            session.say(Level::Error, e.to_string());
        }
        self.mark(ui);
    }
}
