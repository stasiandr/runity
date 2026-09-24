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
    /// The folders, as a tree, and the way to the one chosen.
    tree: NodeId,
    crumbs: NodeId,
    /// The folder shown, project-relative with `/`: `""` the project,
    /// [`BUILTIN`] the engine's own.
    folder: String,
    /// Folders opened in the tree.
    open_folders: std::collections::HashSet<String>,
    /// Each tree row's folder, and whether the click was on its arrow.
    folder_rows: HashMap<NodeId, (String, bool)>,
    /// Each folder tile in the grid.
    folder_tiles: HashMap<NodeId, String>,
    crumb_nodes: HashMap<NodeId, String>,
    entries: HashMap<NodeId, Asset>,
    /// Each tile's files, absolute: the asset's own and, for a model, its
    /// import settings beside it.
    tile_files: HashMap<NodeId, Vec<PathBuf>>,
    /// Files not as committed (see `git_marks`): their tiles get a dot.
    marks: std::collections::HashSet<PathBuf>,
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
    /// Unity's Clear on Play: the Console is emptied when Play starts
    /// (the studio clears it; the Console's ⋮ turns it on).
    pub clear_on_play: bool,
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
        // Pictures by default, as Unity's Project shows its assets.
        let big_toggle = crate::theme::icon_button(ui, bar, "project pictures", "image", true);
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
        // Unity's two columns: the folders as a tree, and what the chosen
        // one holds, under the way to it.
        let body = ui.add(project, Style::row().fill().full_width());
        let tree = ui.add(
            body,
            Style::column()
                .width(180.0)
                .fixed()
                .full_height()
                .padding(SPACE_1)
                .clip(),
        );
        ui.set_name(tree, "project folders");
        ui.add(
            body,
            Style::row().width(1.0).fixed().full_height().background(DIVIDER),
        );
        let right = ui.add(body, Style::column().fill().full_height());
        let crumbs = ui.add(
            right,
            Style::row()
                .full_width()
                .height(26.0)
                .fixed()
                .padding_x(SPACE_2)
                .gap(2.0)
                .center_items(),
        );
        ui.set_name(crumbs, "project path");
        let grid = ui.add(
            right,
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
            big: true,
            big_toggle,
            thumbs: HashMap::new(),
            next_image: 1000,
            grid,
            tree,
            crumbs,
            folder: String::new(),
            open_folders: Default::default(),
            folder_rows: HashMap::new(),
            folder_tiles: HashMap::new(),
            crumb_nodes: HashMap::new(),
            entries: HashMap::new(),
            tile_files: HashMap::new(),
            marks: Default::default(),
            counts,
            filters,
            clear,
            lines,
            at_least: Level::Info,
            seen_lines: 0,
            expanded: Default::default(),
            console_rows: HashMap::new(),
            clear_on_play: false,
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

    /// Show `folder`, its way open in the tree, and leave searching.
    fn go_to(&mut self, folder: String) {
        let mut up = parent_folder(&folder);
        while !up.is_empty() {
            self.open_folders.insert(up.clone());
            up = parent_folder(&up);
        }
        self.folder = folder;
    }


    /// The tree: every folder with something in it, the open ones' folders
    /// under them, the chosen one lit.
    fn update_tree(&mut self, ui: &mut Ui, folders: &[String]) {
        let mut rows: Vec<(String, usize)> = Vec::new();
        fn walk(
            folders: &[String],
            open: &std::collections::HashSet<String>,
            at: &str,
            depth: usize,
            rows: &mut Vec<(String, usize)>,
        ) {
            for f in folders.iter().filter(|f| !f.is_empty() && parent_folder(f) == at) {
                rows.push((f.clone(), depth));
                if open.contains(f) {
                    walk(folders, open, f, depth + 1, rows);
                }
            }
        }
        rows.push((String::new(), 0));
        walk(folders, &self.open_folders, "", 1, &mut rows);
        let keys: Vec<String> = rows.iter().map(|(f, d)| format!("{d} {f}")).collect();
        ui.sync_children(
            self.tree,
            &keys,
            |ui, tree, key| {
                let (folder, depth) = &rows[keys.iter().position(|k| k == key).unwrap()];
                let line = ui.add(
                    tree,
                    Style::row()
                        .height(22.0)
                        .fixed()
                        .full_width()
                        .padding_left(4.0 + *depth as f32 * 12.0)
                        .gap(4.0)
                        .center_items()
                        .radius(RADIUS_SM)
                        .hover(HOVER)
                        .clickable(),
                );
                ui.set_name(line, format!("folder {}", folder_label(folder)));
                let arrow = ui.add(
                    line,
                    Style::row()
                        .size(14.0, 14.0)
                        .fixed()
                        .center()
                        .radius(RADIUS_SM)
                        .clickable(),
                );
                ui.set_name(arrow, format!("folder arrow {}", folder_label(folder)));
                icon(ui, arrow, "chevron-right", MUTED);
                icon(ui, line, "folder", MUTED);
                ui.add_text(line, text().fill().nowrap(), &folder_label(folder));
                line
            },
            |_, _, _| {},
        );
        self.folder_rows.clear();
        for (line, (folder, _)) in ui.children(self.tree).into_iter().zip(&rows) {
            let kids = ui.children(line);
            let has_children = folders
                .iter()
                .any(|f| !f.is_empty() && parent_folder(f) == *folder);
            // The project's own row has no arrow: it is always open.
            let arrow_shown = has_children && !folder.is_empty();
            let open = self.open_folders.contains(folder);
            ui.restyle(kids[0], |s| s.opacity(if arrow_shown { 1.0 } else { 0.0 }));
            if let Some(glyph) = ui.children(kids[0]).first().copied() {
                ui.set_icon(glyph, if open { "chevron-down" } else { "chevron-right" });
            }
            let chosen = *folder == self.folder;
            ui.set_icon(kids[1], if chosen { "folder-open" } else { "folder" });
            ui.restyle(kids[1], |s| s.text_color(if chosen { ACCENT } else { MUTED }));
            ui.restyle(line, |s| {
                s.background(if chosen {
                    ACCENT_900
                } else {
                    runity_ui::Color::TRANSPARENT
                })
            });
            self.folder_rows.insert(line, (folder.clone(), false));
            if arrow_shown {
                self.folder_rows.insert(kids[0], (folder.clone(), true));
            }
        }
    }

    /// The way to the chosen folder, each step a click back up to it.
    fn update_crumbs(&mut self, ui: &mut Ui) {
        ui.clear(self.crumbs);
        self.crumb_nodes.clear();
        let mut steps = vec![String::new()];
        if !self.folder.is_empty() {
            let mut at = String::new();
            for part in self.folder.split('/') {
                at = if at.is_empty() {
                    part.to_string()
                } else {
                    format!("{at}/{part}")
                };
                steps.push(at.clone());
            }
        }
        let last = steps.len() - 1;
        for (i, step) in steps.into_iter().enumerate() {
            if i > 0 {
                icon(ui, self.crumbs, "chevron-right", MUTED);
            }
            let crumb = ui.add(
                self.crumbs,
                Style::row()
                    .height(22.0)
                    .padding_x(SPACE_1)
                    .center_items()
                    .radius(RADIUS_SM)
                    .hover(HOVER)
                    .clickable(),
            );
            let label = folder_label(&step);
            ui.set_name(crumb, format!("crumb {label}"));
            let ink = if i == last { TEXT } else { LABEL };
            ui.add_text(crumb, text().text_color(ink).nowrap(), &label);
            self.crumb_nodes.insert(crumb, step);
        }
    }

    /// The files not as committed: their tiles show a dot.
    pub fn set_marks(&mut self, ui: &mut Ui, marks: std::collections::HashSet<PathBuf>) {
        if marks != self.marks {
            self.marks = marks;
            self.show_marks(ui);
        }
    }

    fn show_marks(&self, ui: &mut Ui) {
        for (tile, files) in &self.tile_files {
            // A folder: marked when anything in it is.
            let on = if self.folder_tiles.contains_key(tile) {
                files
                    .iter()
                    .any(|dir| self.marks.iter().any(|m| m.starts_with(dir)))
            } else {
                files.iter().any(|f| self.marks.contains(f))
            };
            if let Some(dot) = ui.children(*tile).last().copied() {
                ui.restyle(dot, |s| s.opacity(if on { 1.0 } else { 0.0 }));
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

    /// Whether the grid shows search results from every folder: a word
    /// typed or a kind chosen, as Unity's search and type filter do.
    fn searching(&self, ui: &Ui) -> bool {
        self.kind != "All" || !ui.text(self.search).unwrap_or_default().trim().is_empty()
    }

    /// What the grid shows: while searching, what matches anywhere; else
    /// the chosen folder's own assets.
    fn assets(&self, ui: &Ui, session: &Session, all: &[Asset], sources: &Sources) -> Vec<Asset> {
        let mut out = all.to_vec();
        let query = ui
            .text(self.search)
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if !query.is_empty() {
            out.retain(|a| a.label().to_lowercase().contains(&query));
        }
        if !self.searching(ui) {
            out.retain(|a| folder_of(a, session, sources) == self.folder);
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
        let all = all_assets(session);
        let sources = material_sources(session);
        let folders = folders(&all, session, &sources);
        // A folder gone (moved, deleted): back to the project.
        if !self.folder.is_empty() && !folders.contains(&self.folder) {
            self.folder = String::new();
        }
        self.update_tree(ui, &folders);
        self.update_crumbs(ui);
        let subfolders: Vec<String> = if self.searching(ui) {
            Vec::new()
        } else {
            folders
                .iter()
                .filter(|f| parent_folder(f) == self.folder && !f.is_empty())
                .cloned()
                .collect()
        };
        let assets = self.assets(ui, session, &all, &sources);
        // The mode is part of the key: switching it makes the tiles again.
        let big = self.big;
        let keys: Vec<String> = subfolders
            .iter()
            .map(|f| format!("{big}folder {f}"))
            .chain(assets.iter().map(|a| format!("{big}{a:?}")))
            .collect();
        let n_folders = subfolders.len();
        let open = session.scene_path().map(|p| p.to_path_buf());
        let thumbs = &mut self.thumbs;
        let next_image = &mut self.next_image;
        ui.sync_children(
            self.grid,
            &keys,
            |ui, grid, key| {
                let at = keys
                    .iter()
                    .position(|k| k == key)
                    .expect("a key of this list");
                if at < n_folders {
                    return folder_tile(ui, grid, &subfolders[at], big);
                }
                let asset = &assets[at - n_folders];
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
                    mark_dot(ui, tile, &asset.label(), 158.0, 10.0);
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
                // Everything but a sound gets its picture (drawn a few a
                // frame, see Bottom::wanted_pictures); until then, its icon.
                let mut drawn = false;
                if let Some(name) = picture_key(asset) {
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
                mark_dot(ui, tile, &asset.label(), 80.0, 6.0);
                tile
            },
            |_, _, _| {},
        );
        self.entries.clear();
        self.tile_files.clear();
        self.folder_tiles.clear();
        let root = session.project().map(|p| p.root().to_path_buf());
        let tiles = ui.children(self.grid);
        for (tile, folder) in tiles.iter().zip(&subfolders) {
            self.folder_tiles.insert(*tile, folder.clone());
            if let Some(root) = &root {
                let dir = root.join(folder);
                let dir = dir.canonicalize().unwrap_or(dir);
                self.tile_files.insert(*tile, vec![dir]);
            }
        }
        for (tile, asset) in tiles.into_iter().skip(n_folders).zip(assets) {
            if let (Some(root), Some(file)) = (&root, asset_file(&asset, session)) {
                let file = root.join(file);
                let mut files = vec![file.with_extension(format!(
                    "{}.rimport",
                    file.extension().map(|e| e.to_string_lossy()).unwrap_or_default()
                ))];
                files.insert(0, file);
                let files = files
                    .into_iter()
                    .map(|f| f.canonicalize().unwrap_or(f))
                    .collect();
                self.tile_files.insert(tile, files);
            }
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
        self.show_marks(ui);

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
            Event::Click { .. } if self.folder_rows.contains_key(&node) => {
                let (folder, arrow) = self.folder_rows[&node].clone();
                if arrow {
                    if !self.open_folders.remove(&folder) {
                        self.open_folders.insert(folder);
                    }
                } else {
                    self.go_to(folder);
                }
                requests.refresh = true;
            }
            Event::Click { .. } if self.crumb_nodes.contains_key(&node) => {
                self.go_to(self.crumb_nodes[&node].clone());
                requests.refresh = true;
            }
            Event::Click { count, .. } if self.folder_tiles.contains_key(&node) => {
                if *count >= 2 {
                    self.go_to(self.folder_tiles[&node].clone());
                    requests.refresh = true;
                }
            }
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

/// The dot on a tile whose file is not as committed, in its corner: out of
/// the tile's flow, shown by [`Bottom::show_marks`].
fn mark_dot(ui: &mut Ui, tile: NodeId, label: &str, x: f32, y: f32) {
    let dot = ui.add(
        tile,
        Style::row()
            .absolute(x, y)
            .size(6.0, 6.0)
            .radius(3.0)
            .background(ACCENT_400)
            .opacity(0.0),
    );
    ui.set_name(dot, format!("asset mark {label}"));
}

/// What a Project entry's picture is asked for by: the model's or prefab's
/// name, a material's with `Session::thumbnail`'s prefix, a scene's path
/// after `scene:`. A sound has none.
fn picture_key(asset: &Asset) -> Option<String> {
    match asset {
        Asset::Model(name, _) | Asset::Prefab(name) => Some(name.clone()),
        Asset::Material(name) => Some(format!("{}{name}", runity_editor::MATERIAL_PICTURE)),
        Asset::Scene(path) => Some(format!("{SCENE_PICTURE}{}", path.display())),
        Asset::Sound(..) => None,
    }
}

/// Before a scene's path in a picture's name.
pub const SCENE_PICTURE: &str = "scene:";

/// The folder of the engine's own models and materials, which have no file.
pub const BUILTIN: &str = ":builtin";

/// Where an entry lives, project-relative with `/`: its file's folder, or
/// [`BUILTIN`] for what the engine brings.
fn folder_of(asset: &Asset, session: &Session, sources: &Sources) -> String {
    if let Asset::Material(name) = asset {
        // A material an import made lives beside what it came from.
        if let Some(file) = sources.get(name) {
            return parent_folder(file);
        }
    }
    match asset_file(asset, session) {
        Some(file) => parent_folder(&file),
        None => BUILTIN.to_string(),
    }
}

/// Materials without a file of their own, by the file they were imported
/// from: `session.assets()`, read once an update.
type Sources = HashMap<String, String>;

fn material_sources(session: &Session) -> Sources {
    session
        .assets()
        .map(|entries| {
            entries
                .into_iter()
                .filter(|e| e.kind == "material")
                .map(|e| (e.name, e.file))
                .collect()
        })
        .unwrap_or_default()
}

/// The folder above `path` (`assets/kenney` for `assets/kenney/food`, `""`
/// for a top one).
fn parent_folder(path: &str) -> String {
    if path == BUILTIN {
        return String::new();
    }
    path.rsplit_once('/')
        .map(|(up, _)| up.to_string())
        .unwrap_or_default()
}

/// How a folder is named on screen.
fn folder_label(folder: &str) -> String {
    match folder {
        "" => "Project".to_string(),
        BUILTIN => "Built-in".to_string(),
        f => f.rsplit('/').next().unwrap_or(f).to_string(),
    }
}

/// Every folder with an entry in it at any depth, sorted, the project's own
/// (`""`) first and the engine's last.
fn folders(assets: &[Asset], session: &Session, sources: &Sources) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    let mut builtin = false;
    for asset in assets {
        let mut folder = folder_of(asset, session, sources);
        if folder == BUILTIN {
            builtin = true;
            continue;
        }
        while !folder.is_empty() {
            let up = parent_folder(&folder);
            set.insert(folder);
            folder = up;
        }
    }
    let mut out = vec![String::new()];
    out.extend(set);
    if builtin {
        out.push(BUILTIN.to_string());
    }
    out
}

/// A folder in the grid: double-click to go in.
fn folder_tile(ui: &mut Ui, grid: NodeId, folder: &str, big: bool) -> NodeId {
    let label = folder_label(folder);
    let tile = if big {
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
                .clickable(),
        );
        let frame = ui.add(
            tile,
            Style::row().size(76.0, 76.0).fixed().center().radius(RADIUS_SM),
        );
        ui.add_icon(
            frame,
            Style::default().size(44.0, 44.0).text_color(ACCENT_300),
            "folder",
        );
        ui.add_text(
            tile,
            Style::default()
                .text_size(11.0)
                .text_color(TEXT)
                .nowrap()
                .max_width(84.0),
            &label,
        );
        mark_dot(ui, tile, &label, 80.0, 6.0);
        tile
    } else {
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
                .hover(HOVER)
                .clickable(),
        );
        icon(ui, tile, "folder", ACCENT_300);
        ui.add_text(tile, text().fill(), &label);
        mark_dot(ui, tile, &label, 158.0, 10.0);
        tile
    };
    ui.set_name(tile, format!("folder tile {label}"));
    tile
}
