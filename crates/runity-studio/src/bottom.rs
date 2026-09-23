//! The panel under the Scene view: Project and Console, as tabs.
//!
//! **Project** lists what the project has — scenes, prefabs, models,
//! materials — from the session (`assets`, `prefab_names`, `palette`). A
//! scene opens on a double click; a prefab opens in prefab mode; a model or
//! prefab dragged into the Scene view is placed where it lands
//! (`Session::drop_asset`), a material dragged onto a thing paints it.
//!
//! **Console** is `Session::console`: what opening a scene skipped, an
//! import warned about, an edit refused, what the running game printed —
//! collapsed, a repeat counting up — with filters by level and Clear.

use std::collections::HashMap;
use std::path::PathBuf;

use runity_editor::console::{Level, Line};
use runity_editor::Session;
use runity_ui::{Event, NodeId, Style, Ui};

use crate::menu::Action;
use crate::studio::Requests;
use crate::theme::*;

/// What a Project entry is.
#[derive(Debug, Clone, PartialEq)]
pub enum Asset {
    Scene(PathBuf),
    Prefab(String),
    Model(String),
    Material(String),
}

impl Asset {
    fn icon(&self) -> &'static str {
        match self {
            Asset::Scene(_) => "mountain",
            Asset::Prefab(_) => "package",
            Asset::Model(_) => "box",
            Asset::Material(_) => "sparkles",
        }
    }

    fn label(&self) -> String {
        match self {
            Asset::Scene(p) => p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Asset::Prefab(n) | Asset::Model(n) | Asset::Material(n) => {
                n.strip_prefix("builtin:").unwrap_or(n).to_string()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tab {
    Project,
    Console,
}

pub struct Bottom {
    pub card: NodeId,
    tab: Tab,
    tab_project: NodeId,
    tab_console: NodeId,
    // Project
    project: NodeId,
    search: NodeId,
    grid: NodeId,
    entries: HashMap<NodeId, Asset>,
    // Console
    console: NodeId,
    counts: [NodeId; 3],
    filters: [NodeId; 3],
    clear: NodeId,
    lines: NodeId,
    at_least: Level,
    seen_lines: usize,
}

fn tab_style(on: bool) -> Style {
    let s = Style::row()
        .height(24.0)
        .padding_x(SPACE_3)
        .center()
        .radius(6.0)
        .clickable();
    if on {
        s.background(ACCENT_900).hover(ACCENT_900)
    } else {
        s.hover(HOVER)
    }
}

impl Bottom {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let card = ui.add(
            parent,
            Style::column()
                .full()
                .background(SURFACE)
                .radius(RADIUS_MD)
                .clip(),
        );
        ui.set_name(card, "bottom");
        let header = ui.add(
            card,
            Style::row()
                .height(32.0)
                .fixed()
                .full_width()
                .padding_x(SPACE_2)
                .gap(SPACE_1)
                .center_items(),
        );
        let tab_project = ui.add(header, tab_style(true));
        ui.set_name(tab_project, "tab project");
        icon(ui, tab_project, "folder", ACCENT);
        ui.add_text(tab_project, text().margin(0.0).padding_left(6.0), "Project");
        let tab_console = ui.add(header, tab_style(false));
        ui.set_name(tab_console, "tab console");
        icon(ui, tab_console, "terminal", LABEL);
        ui.add_text(tab_console, text().padding_left(6.0), "Console");
        spacer(ui, header);
        // Console's tools live in the header, shown on its tab.
        let tools = ui.add(header, Style::row().gap(SPACE_1).center_items());
        let mut counts = [tools; 3];
        let mut filters = [tools; 3];
        for (i, (glyph, ink, name)) in [
            ("info", NEUTRAL_500, "infos"),
            ("triangle-alert", WARNING, "warnings"),
            ("circle-alert", ERROR, "errors"),
        ]
        .into_iter()
        .enumerate()
        {
            let f = ui.add(
                tools,
                Style::row()
                    .height(22.0)
                    .padding_x(SPACE_2)
                    .gap(4.0)
                    .center_items()
                    .radius(6.0)
                    .border(1.0, runity_ui::Color::TRANSPARENT)
                    .hover(HOVER),
            );
            ui.set_name(f, format!("console {name}"));
            icon(ui, f, glyph, ink);
            counts[i] = ui.add_text(
                f,
                Style::default().text_size(11.0).text_color(LABEL).nowrap(),
                "0",
            );
            filters[i] = f;
        }
        let clear = crate::theme::icon_button(ui, tools, "console clear", "x", false);

        // Project
        let project = ui.add(card, Style::column().fill().full_width());
        let bar = ui.add(
            project,
            Style::row()
                .full_width()
                .padding_x(SPACE_2)
                .height(28.0)
                .fixed()
                .center_items(),
        );
        let search = ui.add_field(bar, field_style().width(240.0).height(24.0), "");
        ui.set_name(search, "project search");
        let grid = ui.add(
            project,
            Style::row()
                .wrap()
                .fill()
                .full_width()
                .padding(SPACE_2)
                .gap(SPACE_1)
                .clip(),
        );
        ui.set_name(grid, "project grid");

        // Console
        let console = ui.add(card, Style::column().fill().full_width().hidden());
        let lines = ui.add(
            console,
            Style::column()
                .fill()
                .full_width()
                .padding_y(SPACE_1)
                .clip(),
        );
        ui.set_name(lines, "console lines");
        ui.restyle(tools, |s| s.hidden());

        Self {
            card,
            tab: Tab::Project,
            tab_project,
            tab_console,
            project,
            search,
            grid,
            entries: HashMap::new(),
            console,
            counts,
            filters,
            clear,
            lines,
            at_least: Level::Info,
            seen_lines: 0,
        }
    }

    fn set_tab(&mut self, ui: &mut Ui, tab: Tab) {
        self.tab = tab;
        let on = |t| t == tab;
        ui.set_style(self.tab_project, tab_style(on(Tab::Project)));
        ui.set_style(self.tab_console, tab_style(on(Tab::Console)));
        for (button, t) in [
            (self.tab_project, Tab::Project),
            (self.tab_console, Tab::Console),
        ] {
            let glyph = ui.children(button)[0];
            ui.restyle(glyph, |s| s.text_color(if on(t) { ACCENT } else { LABEL }));
        }
        ui.restyle(self.project, |s| {
            if on(Tab::Project) {
                s.shown()
            } else {
                s.hidden()
            }
        });
        ui.restyle(self.console, |s| {
            if on(Tab::Console) {
                s.shown()
            } else {
                s.hidden()
            }
        });
        let tools = ui.parent(self.clear).expect("the tools row");
        ui.restyle(tools, |s| {
            if on(Tab::Console) {
                s.shown()
            } else {
                s.hidden()
            }
        });
    }

    /// Show the Console: an error just happened.
    pub fn show_console(&mut self, ui: &mut Ui) {
        self.set_tab(ui, Tab::Console);
    }

    pub fn owns(&self, ui: &Ui, node: NodeId) -> bool {
        let mut at = Some(node);
        while let Some(n) = at {
            if n == self.card {
                return true;
            }
            at = ui.parent(n);
        }
        false
    }

    /// What the project has, in the order a person looks for it.
    fn assets(&self, ui: &Ui, session: &Session) -> Vec<Asset> {
        let mut out = Vec::new();
        if let Some(project) = session.project() {
            let dir = project.root().join("scenes");
            if let Ok(read) = std::fs::read_dir(&dir) {
                let mut scenes: Vec<PathBuf> = read
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|e| e == "ron"))
                    .collect();
                scenes.sort();
                out.extend(scenes.into_iter().map(Asset::Scene));
            }
        }
        out.extend(session.prefab_names().into_iter().map(Asset::Prefab));
        out.extend(
            runity::builtin::NAMES
                .iter()
                .map(|n| Asset::Model(n.to_string())),
        );
        if let Ok(assets) = session.assets() {
            out.extend(
                assets
                    .into_iter()
                    .filter(|a| a.kind == "model")
                    .map(|a| Asset::Model(a.name)),
            );
        }
        out.extend(
            session
                .palette()
                .into_iter()
                .map(|(n, _)| Asset::Material(n)),
        );
        let query = ui
            .text(self.search)
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if !query.is_empty() {
            out.retain(|a| a.label().to_lowercase().contains(&query));
        }
        out
    }

    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        // Project
        let assets = self.assets(ui, session);
        let keys: Vec<String> = assets.iter().map(|a| format!("{a:?}")).collect();
        let open = session.scene_path().map(|p| p.to_path_buf());
        ui.sync_children(
            self.grid,
            &keys,
            |ui, grid, key| {
                let asset = &assets[keys
                    .iter()
                    .position(|k| k == key)
                    .expect("a key of this list")];
                let tile = ui.add(
                    grid,
                    Style::row()
                        .height(26.0)
                        .width(170.0)
                        .fixed()
                        .padding_x(SPACE_2)
                        .gap(SPACE_2)
                        .center_items()
                        .radius(6.0)
                        .border(1.0, runity_ui::Color::TRANSPARENT)
                        .hover(HOVER)
                        .draggable(),
                );
                ui.set_name(tile, format!("asset {}", asset.label()));
                let tint = if matches!(asset, Asset::Prefab(_)) {
                    ACCENT
                } else {
                    MUTED
                };
                icon(ui, tile, asset.icon(), tint);
                ui.add_text(tile, text().fill(), &asset.label());
                tile
            },
            |_, _, _| {},
        );
        self.entries.clear();
        for (tile, asset) in ui.children(self.grid).into_iter().zip(assets) {
            let current = matches!(&asset, Asset::Scene(p) if Some(p) == open.as_ref());
            ui.restyle(tile, |s| {
                if current {
                    s.border(1.0, ACCENT.alpha(40)).background(ACCENT_900)
                } else {
                    s.border(1.0, runity_ui::Color::TRANSPARENT)
                        .background(runity_ui::Color::TRANSPARENT)
                }
            });
            self.entries.insert(tile, asset);
        }

        // Console
        let (info, warnings, errors) = session.console_counts();
        for (node, n) in self.counts.iter().zip([info, warnings, errors]) {
            ui.set_text(*node, &n.to_string());
        }
        for (f, level) in self
            .filters
            .iter()
            .zip([Level::Info, Level::Warning, Level::Error])
        {
            let on = self.at_least == level && level != Level::Info;
            ui.restyle(*f, |s| {
                s.border(
                    1.0,
                    if on {
                        ACCENT.alpha(60)
                    } else {
                        runity_ui::Color::TRANSPARENT
                    },
                )
            });
        }
        let lines: Vec<&Line> = session
            .console()
            .iter()
            .filter(|l| l.level >= self.at_least)
            .collect();
        let keys: Vec<usize> = (0..lines.len()).collect();
        ui.sync_children(
            self.lines,
            &keys,
            |ui, list, _| {
                let row = ui.add(
                    list,
                    Style::row()
                        .height(22.0)
                        .fixed()
                        .full_width()
                        .padding_x(SPACE_4)
                        .gap(SPACE_2)
                        .center_items()
                        .hover(TEXT.alpha(4)),
                );
                icon(ui, row, "info", MUTED);
                ui.add_text(
                    row,
                    Style::default().fill().text_size(11.5).mono().nowrap(),
                    "",
                );
                ui.add(row, Style::row().fixed());
                row
            },
            |_, _, _| {},
        );
        for (row, line) in ui.children(self.lines).into_iter().zip(&lines) {
            let kids = ui.children(row);
            let (glyph, ink) = match line.level {
                Level::Info => ("info", MUTED),
                Level::Warning => ("triangle-alert", WARNING),
                Level::Error => ("circle-alert", ERROR),
            };
            ui.set_icon(kids[0], glyph);
            ui.restyle(kids[0], |s| s.text_color(ink));
            let first = line.text.lines().next().unwrap_or_default();
            ui.set_text(kids[1], first);
            ui.restyle(kids[1], |s| {
                s.text_color(if line.level == Level::Info {
                    LABEL
                } else {
                    ink
                })
            });
            let has = !ui.children(kids[2]).is_empty();
            if line.count > 1 {
                if has {
                    let t = ui.children(kids[2])[0];
                    let label = ui.children(t)[0];
                    ui.set_text(label, &line.count.to_string());
                } else {
                    tag(
                        ui,
                        kids[2],
                        &line.count.to_string(),
                        NEUTRAL_800,
                        NEUTRAL_300,
                    );
                }
            } else if has {
                ui.clear(kids[2]);
            }
        }
        if lines.len() > self.seen_lines {
            if let Some(last) = ui.children(self.lines).last().copied() {
                ui.scroll_to(self.lines, last);
            }
        }
        self.seen_lines = lines.len();
    }

    pub fn event(
        &mut self,
        ui: &mut Ui,
        session: &mut Session,
        node: NodeId,
        event: &Event,
        requests: &mut Requests,
    ) {
        match event {
            Event::Click { .. } if node == self.tab_project => self.set_tab(ui, Tab::Project),
            Event::Click { .. } if node == self.tab_console => self.set_tab(ui, Tab::Console),
            Event::Click { .. } if node == self.clear => {
                requests.action = Some(Action::ClearConsole);
            }
            Event::Click { .. } if self.filters.contains(&node) => {
                let i = self.filters.iter().position(|f| *f == node).unwrap_or(0);
                let level = [Level::Info, Level::Warning, Level::Error][i];
                self.at_least = if self.at_least == level {
                    Level::Info
                } else {
                    level
                };
                self.seen_lines = 0;
                requests.refresh = true;
            }
            Event::Changed(_) | Event::Cancel if node == self.search => requests.refresh = true,
            Event::Click { count, .. } if *count >= 2 => match self.entries.get(&node).cloned() {
                Some(Asset::Scene(path)) => requests.action = Some(Action::OpenScene(path)),
                Some(Asset::Prefab(name)) => {
                    match session.open_prefab(&name) {
                        Ok(missing) => {
                            for m in missing {
                                session.say(Level::Warning, m);
                            }
                        }
                        Err(e) => session.say(Level::Error, e.to_string()),
                    }
                    requests.refresh = true;
                }
                Some(Asset::Model(name)) => requests.action = Some(Action::Place(name)),
                Some(Asset::Material(name)) => {
                    requests.action = Some(Action::SetField("material".into(), name))
                }
                None => {}
            },
            Event::DragEnd { .. } => {
                if let Some(asset) = self.entries.get(&node).cloned() {
                    requests.dropped = Some(asset);
                }
            }
            _ => {}
        }
    }
}
