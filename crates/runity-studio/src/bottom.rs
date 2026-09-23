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
use runity_ui::{Event, ImageId, NodeId, Style, Ui};

use crate::menu::Action;
use crate::studio::Requests;
use crate::theme::*;

/// What a Project entry is.
#[derive(Debug, Clone, PartialEq)]
pub enum Asset {
    Scene(PathBuf),
    Prefab(String),
    /// A model by name, and its source file when it has one in the project.
    Model(String, Option<String>),
    Material(String),
    /// A sound by name, and its source file.
    Sound(String, String),
}

impl Asset {
    pub fn icon(&self) -> &'static str {
        match self {
            Asset::Scene(_) => "mountain",
            Asset::Prefab(_) => "package",
            Asset::Model(..) => "box",
            Asset::Material(_) => "sparkles",
            Asset::Sound(..) => "music",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Asset::Scene(p) => p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Asset::Prefab(n) | Asset::Model(n, _) | Asset::Material(n) | Asset::Sound(n, _) => {
                n.strip_prefix("builtin:").unwrap_or(n).to_string()
            }
        }
    }
}

pub struct Bottom {
    /// Project, Console, History and Git: each panel's content.
    pub roots: [NodeId; 4],
    /// Which of them is on top in its dock, and so worth updating.
    visible: [bool; 4],
    // Project
    search: NodeId,
    /// Big tiles with pictures, or lines.
    big: bool,
    big_toggle: NodeId,
    /// Each model's or prefab's picture: its image, and whether it is drawn.
    thumbs: HashMap<String, (ImageId, bool)>,
    next_image: u32,
    /// The type filter's chips, and the kind chosen.
    kind_chips: Vec<(NodeId, &'static str)>,
    kind: &'static str,
    grid: NodeId,
    entries: HashMap<NodeId, Asset>,
    // Console
    counts: [NodeId; 3],
    filters: [NodeId; 3],
    clear: NodeId,
    lines: NodeId,
    at_least: Level,
    seen_lines: usize,
    /// Console lines opened to show all of their text, by index.
    expanded: std::collections::HashSet<usize>,
    /// Each Console row's line, by index.
    console_rows: HashMap<NodeId, usize>,
    // History
    history_list: NodeId,
    /// Each History row: how many steps from the start it stands for.
    history_rows: HashMap<NodeId, usize>,
    // Git
    git_list: NodeId,
    git_refresh: NodeId,
    revisions: HashMap<NodeId, String>,
    take_theirs: HashMap<NodeId, usize>,
    /// Git is asked when the tab opens or on Refresh, not every frame.
    git_stale: bool,
}

impl Bottom {
    /// The four panels' content, in `parent` — for docks to take.
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let root = |ui: &mut Ui, name: &str| {
            let r = ui.add(parent, Style::column().fill().full_width());
            ui.set_name(r, name.to_string());
            r
        };
        let project = root(ui, "project");
        let console = root(ui, "console");
        let history = root(ui, "history");
        let git = root(ui, "git");

        // Console's tools: a bar at its top.
        let tools = ui.add(
            console,
            Style::row()
                .full_width()
                .height(28.0)
                .fixed()
                .padding_x(SPACE_2)
                .gap(SPACE_1)
                .center_items(),
        );
        spacer(ui, tools);
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
        ui.set_placeholder(search, "Search assets");
        spacer(ui, bar);
        let big_toggle = crate::theme::icon_button(ui, bar, "project pictures", "image", false);
        // What kind to show: Unity's type filter.
        let kinds = ui.add(
            bar,
            Style::row().gap(2.0).padding_left(SPACE_2).center_items(),
        );
        let mut kind_chips = Vec::new();
        for kind in ["All", "Scenes", "Prefabs", "Models", "Sounds", "Materials"] {
            let chip = ui.add(
                kinds,
                Style::row()
                    .height(22.0)
                    .padding_x(SPACE_2)
                    .center()
                    .radius(6.0)
                    .hover(HOVER),
            );
            ui.set_name(chip, format!("kind {kind}"));
            ui.add_text(
                chip,
                Style::default().text_size(11.5).text_color(LABEL).nowrap(),
                kind,
            );
            kind_chips.push((chip, kind));
        }
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
        let lines = ui.add(
            console,
            Style::column()
                .fill()
                .full_width()
                .padding_y(SPACE_1)
                .clip(),
        );
        ui.set_name(lines, "console lines");

        // History
        let history_list = ui.add(
            history,
            Style::column()
                .fill()
                .full_width()
                .padding_y(SPACE_1)
                .clip(),
        );
        ui.set_name(history_list, "history lines");

        // Git
        let git_bar = ui.add(
            git,
            Style::row()
                .full_width()
                .height(30.0)
                .fixed()
                .padding_x(SPACE_3)
                .gap(SPACE_2)
                .center_items(),
        );
        ui.add_text(
            git_bar,
            Style::default()
                .text_size(11.5)
                .text_color(MUTED)
                .nowrap()
                .fill(),
            "The scene's history in git. Double-click a revision to bring it back — one undo step.",
        );
        let git_refresh = crate::theme::button(ui, git_bar, "git refresh", "Refresh", false);
        let git_list = ui.add(
            git,
            Style::column()
                .fill()
                .full_width()
                .padding_y(SPACE_1)
                .clip(),
        );
        ui.set_name(git_list, "git lines");

        Self {
            roots: [project, console, history, git],
            visible: [true, false, false, false],
            search,
            kind_chips,
            kind: "All",
            big: false,
            big_toggle,
            thumbs: HashMap::new(),
            next_image: 1000,
            grid,
            entries: HashMap::new(),
            counts,
            filters,
            clear,
            lines,
            at_least: Level::Info,
            seen_lines: 0,
            expanded: Default::default(),
            console_rows: HashMap::new(),
            history_list,
            history_rows: HashMap::new(),
            git_list,
            git_refresh,
            revisions: HashMap::new(),
            take_theirs: HashMap::new(),
            git_stale: true,
        }
    }

    /// Pictures the big tiles still want: models' and prefabs' names, and the
    /// image each goes to. At most `n`.
    pub fn wanted_pictures(&self, n: usize) -> Vec<(String, ImageId)> {
        if !self.big {
            return Vec::new();
        }
        self.thumbs
            .iter()
            .filter(|(_, (_, ready))| !ready)
            .take(n)
            .map(|(name, (id, _))| (name.clone(), *id))
            .collect()
    }

    /// A picture was drawn: the tile shows it instead of its icon.
    pub fn picture_ready(&mut self, ui: &mut Ui, name: &str) {
        if let Some(entry) = self.thumbs.get_mut(name) {
            entry.1 = true;
        }
        if let Some(img) = ui.find(&format!("thumb {name}")) {
            if let Some(frame) = ui.parent(img) {
                for child in ui.children(frame) {
                    if child != img {
                        ui.restyle(child, |s| s.hidden());
                    }
                }
            }
        }
    }

    /// Say which of the four panels are on top in their docks: hidden ones
    /// skip their updates. Git is asked again when it comes on top.
    pub fn set_visible(&mut self, visible: [bool; 4]) {
        if visible[3] && !self.visible[3] {
            self.git_stale = true;
        }
        self.visible = visible;
    }

    pub fn owns(&self, ui: &Ui, node: NodeId) -> bool {
        let mut at = Some(node);
        while let Some(n) = at {
            if self.roots.contains(&n) {
                return true;
            }
            at = ui.parent(n);
        }
        false
    }

    /// What the project has, in the order a person looks for it.
    fn assets(&self, ui: &Ui, session: &Session) -> Vec<Asset> {
        let mut out = all_assets(session);
        let query = ui
            .text(self.search)
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if !query.is_empty() {
            out.retain(|a| a.label().to_lowercase().contains(&query));
        }
        let kind = self.kind;
        out.retain(|a| {
            matches!(
                (kind, a),
                ("All", _)
                    | ("Scenes", Asset::Scene(_))
                    | ("Prefabs", Asset::Prefab(_))
                    | ("Models", Asset::Model(..))
                    | ("Sounds", Asset::Sound(..))
                    | ("Materials", Asset::Material(_))
            )
        });
        out
    }
}

/// Everything the project has, in the order a person looks for it: scenes,
/// prefabs, models, sounds, materials.
pub fn all_assets(session: &Session) -> Vec<Asset> {
    {
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
                .map(|n| Asset::Model(n.to_string(), None)),
        );
        if let Ok(assets) = session.assets() {
            out.extend(
                assets
                    .into_iter()
                    .filter(|a| a.kind == "model")
                    .map(|a| Asset::Model(a.name, Some(a.file))),
            );
        }
        if let Ok(assets) = session.assets() {
            out.extend(
                assets
                    .into_iter()
                    .filter(|a| a.kind == "sound")
                    .map(|a| Asset::Sound(a.name, a.file)),
            );
        }
        out.extend(
            session
                .palette()
                .into_iter()
                .map(|(n, _)| Asset::Material(n)),
        );
        out
    }
}

impl Bottom {
    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        // Project
        let assets = self.assets(ui, session);
        // The mode is part of the key: switching it makes the tiles again.
        let big = self.big;
        let keys: Vec<String> = assets.iter().map(|a| format!("{big}{a:?}")).collect();
        let open = session.scene_path().map(|p| p.to_path_buf());
        let thumbs = &mut self.thumbs;
        let next_image = &mut self.next_image;
        ui.sync_children(
            self.grid,
            &keys,
            |ui, grid, key| {
                let asset = &assets[keys
                    .iter()
                    .position(|k| k == key)
                    .expect("a key of this list")];
                let tint = if matches!(asset, Asset::Prefab(_)) {
                    ACCENT
                } else {
                    MUTED
                };
                if !big {
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
                    icon(ui, tile, asset.icon(), tint);
                    ui.add_text(tile, text().fill(), &asset.label());
                    return tile;
                }
                // A card: the picture, the name under it.
                let tile = ui.add(
                    grid,
                    Style::column()
                        .width(92.0)
                        .height(104.0)
                        .fixed()
                        .padding(4.0)
                        .gap(3.0)
                        .center_items()
                        .radius(RADIUS_MD)
                        .border(1.0, runity_ui::Color::TRANSPARENT)
                        .hover(HOVER)
                        .draggable(),
                );
                ui.set_name(tile, format!("asset {}", asset.label()));
                let frame = ui.add(
                    tile,
                    Style::row()
                        .size(76.0, 76.0)
                        .fixed()
                        .center()
                        .radius(RADIUS_SM)
                        .background(BG),
                );
                // Models and prefabs get their picture (drawn a few a frame,
                // see Bottom::wanted_pictures); until then, and for the rest,
                // their icon.
                let mut drawn = false;
                if let Asset::Model(name, _) | Asset::Prefab(name) = asset {
                    let (id, ready) = *thumbs.entry(name.clone()).or_insert_with(|| {
                        *next_image += 1;
                        (ImageId(*next_image), false)
                    });
                    let img = ui.add_image(
                        frame,
                        Style::default().size(76.0, 76.0).radius(RADIUS_SM),
                        id,
                    );
                    ui.set_name(img, format!("thumb {name}"));
                    drawn = ready;
                }
                let glyph = ui.add_icon(
                    frame,
                    Style::default()
                        .size(28.0, 28.0)
                        .text_color(tint)
                        .absolute(24.0, 24.0),
                    asset.icon(),
                );
                if drawn {
                    ui.restyle(glyph, |s| s.hidden());
                }
                ui.add_text(
                    tile,
                    Style::default()
                        .text_size(11.0)
                        .text_color(TEXT)
                        .nowrap()
                        .max_width(84.0),
                    &asset.label(),
                );
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

        for (chip, kind) in &self.kind_chips {
            let on = *kind == self.kind;
            ui.restyle(*chip, |s| {
                s.background(if on {
                    ACCENT_900
                } else {
                    runity_ui::Color::TRANSPARENT
                })
            });
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
        self.console_rows.clear();
        for (i, (row, line)) in ui.children(self.lines).into_iter().zip(&lines).enumerate() {
            self.console_rows.insert(row, i);
            if ui.name(row) != Some(&format!("console line {i}")) {
                ui.set_name(row, format!("console line {i}"));
            }
            let open = self.expanded.contains(&i) && line.text.contains('\n');
            ui.restyle(row, |s| {
                let s = s.clickable();
                if open {
                    s.auto_height()
                        .min_height(22.0)
                        .padding_y(3.0)
                        .background(TEXT.alpha(4))
                } else {
                    s.height(22.0)
                        .padding_y(0.0)
                        .background(runity_ui::Color::TRANSPARENT)
                }
            });
            let kids = ui.children(row);
            let (glyph, ink) = match line.level {
                Level::Info => ("info", MUTED),
                Level::Warning => ("triangle-alert", WARNING),
                Level::Error => ("circle-alert", ERROR),
            };
            ui.set_icon(kids[0], glyph);
            ui.restyle(kids[0], |s| s.text_color(ink));
            let more = line.text.lines().count() > 1;
            let shown = if open {
                line.text.clone()
            } else if more {
                format!("{}  …", line.text.lines().next().unwrap_or_default())
            } else {
                line.text.clone()
            };
            ui.set_text(kids[1], &shown);
            ui.restyle(kids[1], |s| {
                let mut s = s;
                s.text.nowrap = !open;
                s
            });
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

        if self.visible[2] {
            self.update_history(ui, session);
        }
    }

    /// Every step undo can take back, oldest first, the last one marked —
    /// Unity's Undo History window. A click goes back (or forward) to it.
    fn update_history(&mut self, ui: &mut Ui, session: &Session) {
        let mut steps = vec!["(as opened)".to_string()];
        steps.extend(session.undo_steps());
        let now = steps.len() - 1;
        if let Some(redo) = session.redo_label() {
            steps.push(format!("{redo}  (undone)"));
        }
        ui.clear(self.history_list);
        self.history_rows.clear();
        for (i, step) in steps.iter().enumerate() {
            let current = i == now;
            let row = ui.add(
                self.history_list,
                Style::row()
                    .height(22.0)
                    .fixed()
                    .full_width()
                    .padding_x(SPACE_4)
                    .gap(SPACE_2)
                    .center_items()
                    .hover(TEXT.alpha(5))
                    .background(if current {
                        ACCENT_900
                    } else {
                        runity_ui::Color::TRANSPARENT
                    }),
            );
            ui.set_name(row, format!("history {i}"));
            icon(
                ui,
                row,
                if current {
                    "chevron-right"
                } else {
                    "circle-dot"
                },
                if current { ACCENT } else { MUTED },
            );
            ui.add_text(
                row,
                Style::default()
                    .text_size(12.0)
                    .text_color(if i > now {
                        MUTED
                    } else if current {
                        ACCENT_200
                    } else {
                        TEXT
                    })
                    .nowrap(),
                step,
            );
            self.history_rows.insert(row, i);
        }
        if let Some(last) = ui.children(self.history_list).get(now).copied() {
            ui.scroll_to(self.history_list, last);
        }
    }

    /// The scene's revisions in git and, mid-merge, its conflicts.
    pub fn update_git(&mut self, ui: &mut Ui, session: &mut Session) {
        if !self.visible[3] || !self.git_stale {
            return;
        }
        self.git_stale = false;
        ui.clear(self.git_list);
        self.revisions.clear();
        self.take_theirs.clear();
        let small = |c| Style::default().text_size(11.5).text_color(c).nowrap();
        match session.merge_conflicts() {
            Ok(conflicts) if !conflicts.is_empty() => {
                let h = ui.add(
                    self.git_list,
                    Style::row()
                        .full_width()
                        .padding_x(SPACE_4)
                        .padding_y(SPACE_1),
                );
                ui.add_text(
                    h,
                    small(WARNING),
                    &format!("{} conflicts in this merge", conflicts.len()),
                );
                for (i, c) in conflicts.iter().enumerate() {
                    let row = ui.add(
                        self.git_list,
                        Style::row()
                            .height(26.0)
                            .fixed()
                            .full_width()
                            .padding_x(SPACE_4)
                            .gap(SPACE_2)
                            .center_items(),
                    );
                    icon(ui, row, "triangle-alert", WARNING);
                    ui.add_text(row, small(TEXT).fill(), &c.to_string());
                    let b = crate::theme::button(
                        ui,
                        row,
                        &format!("take theirs {i}"),
                        "Take theirs",
                        false,
                    );
                    self.take_theirs.insert(b, i);
                }
            }
            _ => {}
        }
        match session.scene_history() {
            Ok(revisions) if !revisions.is_empty() => {
                for r in revisions.iter().take(200) {
                    let row = ui.add(
                        self.git_list,
                        Style::row()
                            .height(22.0)
                            .fixed()
                            .full_width()
                            .padding_x(SPACE_4)
                            .gap(SPACE_3)
                            .center_items()
                            .hover(TEXT.alpha(5)),
                    );
                    ui.set_name(
                        row,
                        format!("revision {}", &r.commit[..r.commit.len().min(8)]),
                    );
                    ui.add_text(
                        row,
                        small(MUTED).mono().width(64.0).fixed(),
                        &r.commit[..r.commit.len().min(7)],
                    );
                    ui.add_text(row, small(MUTED).width(84.0).fixed(), &r.date);
                    ui.add_text(row, small(LABEL).width(120.0).fixed(), &r.author);
                    ui.add_text(row, small(TEXT).fill(), &r.summary);
                    self.revisions.insert(row, r.commit.clone());
                }
            }
            Ok(_) => {
                let h = ui.add(self.git_list, Style::row().padding_x(SPACE_4));
                ui.add_text(h, small(MUTED), "No commits of this scene yet.");
            }
            Err(e) => {
                let h = ui.add(self.git_list, Style::row().padding_x(SPACE_4));
                ui.add_text(h, small(MUTED), &format!("No git history: {e}"));
            }
        }
    }

    pub fn event(
        &mut self,
        _ui: &mut Ui,
        session: &mut Session,
        node: NodeId,
        event: &Event,
        requests: &mut Requests,
    ) {
        match event {
            Event::Click { .. } if node == self.git_refresh => {
                self.git_stale = true;
            }
            Event::Click { .. } if self.take_theirs.contains_key(&node) => {
                let i = self.take_theirs[&node];
                if let Err(e) = session.take_theirs(i) {
                    session.say(Level::Error, e.to_string());
                }
                self.git_stale = true;
                requests.refresh = true;
            }
            Event::Click { count, .. } if *count >= 2 && self.revisions.contains_key(&node) => {
                let commit = self.revisions[&node].clone();
                match session.restore_revision(&commit) {
                    Ok(()) => session.say(
                        Level::Info,
                        format!(
                            "brought back the scene as of {}",
                            &commit[..commit.len().min(7)]
                        ),
                    ),
                    Err(e) => session.say(Level::Error, e.to_string()),
                }
                requests.refresh = true;
            }
            Event::Click { .. } if self.history_rows.contains_key(&node) => {
                let target = self.history_rows[&node];
                let now = session.undo_steps().len();
                if target < now {
                    for _ in target..now {
                        let _ = session.undo();
                    }
                } else if target > now {
                    for _ in now..target {
                        let _ = session.redo();
                    }
                }
                requests.refresh = true;
            }
            Event::Click { count, .. } if self.console_rows.contains_key(&node) => {
                let i = self.console_rows[&node];
                if *count >= 2 {
                    let line = session
                        .console()
                        .iter()
                        .filter(|l| l.level >= self.at_least)
                        .nth(i)
                        .cloned();
                    if let Some(at) = line.and_then(|l| l.location()) {
                        open_in_editor(session, &at);
                    }
                } else if !self.expanded.remove(&i) {
                    self.expanded.insert(i);
                }
                requests.refresh = true;
            }
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
            Event::Click { .. } if node == self.big_toggle => {
                self.big = !self.big;
                crate::theme::set_icon_button(_ui, self.big_toggle, "image", self.big, true);
                requests.refresh = true;
            }
            Event::Click { .. } if self.kind_chips.iter().any(|(c, _)| *c == node) => {
                self.kind = self.kind_chips.iter().find(|(c, _)| *c == node).unwrap().1;
                requests.refresh = true;
            }
            Event::Click {
                button: runity::input::MouseButton::Right,
                ..
            } if self.entries.contains_key(&node) => {
                let asset = self.entries[&node].clone();
                let (x, y) = _ui.pointer();
                requests.menu = Some((asset_menu(&asset, session), x, y));
            }
            Event::Click { count, .. } if *count >= 2 => match self.entries.get(&node).cloned() {
                Some(Asset::Scene(path)) => requests.action = Some(Action::OpenScene(path)),
                Some(Asset::Prefab(name)) => {
                    let _ = session;
                    requests.action = Some(Action::OpenPrefab(name));
                }
                Some(Asset::Model(name, _)) => requests.action = Some(Action::Place(name)),
                Some(Asset::Material(name)) => {
                    requests.action = Some(Action::SetField("material".into(), name))
                }
                Some(Asset::Sound(name, _)) => requests.action = Some(Action::PlaySound(name)),
                None => {}
            },
            Event::Click {
                count: 1,
                button: runity::input::MouseButton::Left,
            } => {
                if let Some(asset) = self.entries.get(&node).cloned() {
                    requests.inspect = Some(asset);
                }
            }
            Event::DragEnd { .. } => {
                if let Some(asset) = self.entries.get(&node).cloned() {
                    requests.dropped = Some(asset);
                }
            }
            _ => {}
        }
    }
}

/// Open the file a Console line points at: in VS Code at the line when it
/// is there (`code -g`), else with whatever the system opens it with.
fn open_in_editor(session: &mut Session, at: &runity_editor::console::Location) {
    let root = session
        .project()
        .map(|p| p.root().to_path_buf())
        .unwrap_or_default();
    let file = root.join(&at.file);
    if !file.is_file() {
        session.say(Level::Warning, format!("{} is not in the project", at.file));
        return;
    }
    let spot = format!("{}:{}:{}", file.display(), at.line, at.column);
    let code = std::process::Command::new("code")
        .arg("-g")
        .arg(&spot)
        .spawn();
    if code.is_err() {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(target_os = "windows") {
            "explorer"
        } else {
            "xdg-open"
        };
        if let Err(e) = std::process::Command::new(opener).arg(&file).spawn() {
            session.say(Level::Error, format!("cannot open {}: {e}", file.display()));
        }
    }
}

/// A Project entry's file, project-relative, when it has one of its own —
/// builtins do not.
fn asset_file(asset: &Asset, session: &Session) -> Option<String> {
    let root = session.project()?.root().to_path_buf();
    let rel = |p: std::path::PathBuf| {
        p.strip_prefix(&root)
            .ok()
            .map(|r| r.to_string_lossy().replace('\\', "/"))
    };
    match asset {
        Asset::Scene(p) => rel(p.clone()),
        Asset::Model(_, file) => file.clone(),
        Asset::Prefab(n) => {
            let p = root.join("prefabs").join(format!("{n}.prefab"));
            p.is_file().then(|| rel(p)).flatten()
        }
        Asset::Material(n) => {
            let p = root.join("materials").join(format!("{n}.rmat"));
            p.is_file().then(|| rel(p)).flatten()
        }
        Asset::Sound(_, file) => Some(file.clone()),
    }
}

/// A right click on a Project entry: Unity's asset context menu.
fn asset_menu(asset: &Asset, session: &Session) -> Vec<crate::menu::MenuItem> {
    use crate::menu::MenuItem;
    let mut items = Vec::new();
    match asset {
        Asset::Scene(p) => items.push(MenuItem::new("Open", Action::OpenScene(p.clone()))),
        Asset::Prefab(n) => {
            items.push(MenuItem::new("Open Prefab", Action::OpenPrefab(n.clone())));
            items.push(MenuItem::new(
                "Place in the Scene",
                Action::Place(n.clone()),
            ));
        }
        Asset::Model(n, _) => items.push(MenuItem::new(
            "Place in the Scene",
            Action::Place(n.clone()),
        )),
        Asset::Material(n) => {
            items.push(MenuItem::new(
                "Apply to the Selection",
                Action::SetField("material".into(), n.clone()),
            ));
            items.push(MenuItem::new(
                "Create Material Instance",
                Action::MaterialInstance(n.clone()),
            ));
        }
        Asset::Sound(n, _) => {
            items.push(MenuItem::new("Play", Action::PlaySound(n.clone())));
            items.push(MenuItem::new("Stop", Action::StopSound));
        }
    }
    if let Some(file) = asset_file(asset, session) {
        items.push(MenuItem::separator());
        items.push(MenuItem::new("Rename…", Action::AssetRename(file.clone())));
        items.push(MenuItem::new(
            "Duplicate…",
            Action::AssetDuplicate(file.clone()),
        ));
        items.push(MenuItem::new("Delete", Action::AssetDelete(file.clone())));
        items.push(MenuItem::separator());
        let reveal = if cfg!(target_os = "macos") {
            "Reveal in Finder"
        } else {
            "Show in Explorer"
        };
        items.push(MenuItem::new(reveal, Action::AssetReveal(file)));
    }
    items
}
