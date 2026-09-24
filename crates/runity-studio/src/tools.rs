//! Project Settings and Profiler: two panels a dock can hold.
//!
//! **Project Settings** is Unity's at the grain this engine has them: the
//! project's own files, in git — `runity.ron` (its modules, the game),
//! `input.ron`, `layers.ron`, the tuning numbers in `tuning/`, the screens
//! in `ui/` — listed, one open in a field of several lines, saved only
//! when it still reads as RON. The engine picks the change up from disk
//! as it picks up any other (DNA, postulate 1); nothing here knows what
//! the fields mean, so nothing here goes stale when they grow. What is
//! the person's and not the project's — colours, keys, tools — is
//! Preferences (`crate::preferences`), a window of its own.
//!
//! **Profiler** is what a frame of the editor costs, as Unity's Profiler
//! shows a frame of the game: the last seconds as bars, each split into
//! the Scene view's input, the render, the panels and the UI, with the
//! slowest and the median beside them.

use std::path::PathBuf;

use runity_editor::console::Level;
use runity_editor::Session;
use runity_ui::{Event, NodeId, Style, Ui};

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
        let mut out = vec![root.join(runity::project::FILE)];
        for file in [runity::project::INPUT, runity::layers::FILE] {
            let path = root.join(file);
            if path.is_file() {
                out.push(path);
            }
        }
        for dir in [runity::project::TUNING, runity::project::UI] {
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
        // Open on the first, `runity.ron`: an empty page says nothing.
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
                            runity_ui::Color::TRANSPARENT
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
        if let Err(e) = runity::ron::from_str::<runity::ron::Value>(&text) {
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
}

/// How many frames the Profiler keeps.
const FRAMES: usize = 120;

pub struct Profiler {
    pub root: NodeId,
    bars: NodeId,
    summary: NodeId,
    frames: Vec<FrameCost>,
}

/// The parts' colours: the three accents of the gizmo and a neutral.
fn part_colors() -> [runity_ui::Color; 4] {
    [AXIS_Y, AXIS_Z, ACCENT, NEUTRAL_500]
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
        for (label, color) in ["scene input", "render", "panels", "UI"]
            .into_iter()
            .zip(part_colors())
        {
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
        let summary = ui.add_text(
            legend,
            Style::default().text_size(11.0).text_color(MUTED).nowrap(),
            "",
        );
        ui.set_name(summary, "profiler summary");
        let bars = ui.add(
            root,
            Style::row()
                .fill()
                .full_width()
                .gap(1.0)
                .radius(RADIUS_SM)
                .background(BG)
                .clip(),
        );
        ui.set_name(bars, "profiler bars");
        let _ = &bars;
        Self {
            root,
            bars,
            summary,
            frames: Vec::new(),
        }
    }

    pub fn record(&mut self, cost: FrameCost) {
        self.frames.push(cost);
        if self.frames.len() > FRAMES {
            self.frames.remove(0);
        }
    }

    /// Draw the bars: taller is slower, 33 ms is the top.
    pub fn update(&mut self, ui: &mut Ui) {
        let keys: Vec<usize> = (0..self.frames.len()).collect();
        let colors = part_colors();
        ui.sync_children(
            self.bars,
            &keys,
            |ui, bars, _| {
                let bar = ui.add(bars, Style::column().width(4.0).full_height().fixed());
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
        let height = ui.rect(self.bars).height.max(1.0);
        let scale = height / 33.0;
        for (bar, cost) in ui.children(self.bars).into_iter().zip(self.frames.clone()) {
            let kids = ui.children(bar);
            // Top to bottom as the legend reads.
            for (node, ms) in kids[1..]
                .iter()
                .zip([cost.input, cost.render, cost.panels, cost.ui])
            {
                let h = (ms * scale).min(height);
                ui.restyle(*node, |s| s.height(h));
            }
        }
        if !self.frames.is_empty() {
            let mut totals: Vec<f32> = self.frames.iter().map(FrameCost::total).collect();
            totals.sort_by(|a, b| a.total_cmp(b));
            let median = totals[totals.len() / 2];
            let worst = *totals.last().unwrap();
            ui.set_text(
                self.summary,
                &format!(
                    "median {median:.1} ms · worst {worst:.1} ms · {} frames",
                    totals.len()
                ),
            );
        }
    }

    pub fn owns(&self, ui: &Ui, node: NodeId) -> bool {
        ancestor(ui, node, self.root)
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
    showing: Option<runity::EntityId>,
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
                    runity_ui::Color::TRANSPARENT
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
