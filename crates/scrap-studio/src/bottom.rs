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

use scrap_editor::console::{Level, Line};
use scrap_editor::Session;
use scrap_import::assets::Entry;
use scrap_ui::{Event, ImageId, NodeId, Style, Ui};

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
    /// Unity's One Column Layout: the whole panel one tree, folders with
    /// their files under them, in place of the two columns.
    one_column: bool,
    one_column_toggle: NodeId,
    /// The two columns, hidden while the one tree shows.
    columns: NodeId,
    /// The one tree's lines.
    files: NodeId,
    /// Each line of the one tree, in order: what it stands for, and how
    /// deep it is.
    file_lines: Vec<(NodeId, Pick)>,
    file_depths: Vec<usize>,
    /// A line's arrow, by the folder it opens.
    line_arrows: HashMap<NodeId, String>,
    /// How each line was last drawn — selected, open, the open scene — so
    /// that an update touches only the lines that changed.
    line_looks: HashMap<NodeId, (bool, bool, bool)>,
    /// The grid's tiles in order, for the arrows to walk.
    tiles: Vec<(NodeId, Pick)>,
    /// The one thing chosen in the Project, lit where it shows.
    selected: Option<Pick>,
    /// Scroll the chosen thing into view at the next update.
    reveal: bool,
    /// Whether the arrows, Enter, F2 and Delete are the Project's: after a
    /// click on it, until a click elsewhere.
    active: bool,
    /// Under the panel: the chosen thing's path, the folder part a click
    /// back to its folder.
    path_icon: NodeId,
    path_folder: NodeId,
    path_file: NodeId,
    /// The folder the path's folder part goes to.
    path_target: Option<String>,
    entries: HashMap<NodeId, Asset>,
    /// The entry an Inspector field pointed at (Unity's ping): selected,
    /// and scrolled to once.
    pinged: Option<(Asset, bool)>,
    /// Each tile's or line's files, absolute: the asset's own and, for a
    /// model, its import settings beside it; a folder's directory. Worked
    /// out once, when the node is made.
    tile_files: HashMap<NodeId, (bool, Vec<PathBuf>)>,
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
    /// How a Console line's file is opened: Preferences › External Tools
    /// (`preferences::editor_command`).
    pub editor_command: String,
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
                    .border(1.0, scrap_ui::Color::TRANSPARENT)
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
        // Unity's One Column Layout: one tree of folders and files.
        let one_column_toggle =
            crate::theme::icon_button(ui, bar, "project one column", "list-tree", false);
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
        // The one tree, in the columns' place while it is chosen.
        let files = ui.add(
            project,
            Style::column()
                .fill()
                .full_width()
                .padding(SPACE_1)
                .clip()
                .hidden(),
        );
        ui.set_name(files, "project files");
        // The chosen thing's path, as Unity's Project shows it at its foot.
        ui.add(
            project,
            Style::row().height(1.0).fixed().full_width().background(DIVIDER),
        );
        let foot = ui.add(
            project,
            Style::row()
                .full_width()
                .height(22.0)
                .fixed()
                .padding_x(SPACE_3)
                .gap(4.0)
                .center_items(),
        );
        ui.set_name(foot, "asset path");
        let path_icon = ui.add_icon(
            foot,
            Style::default()
                .size(12.0, 12.0)
                .fixed()
                .text_color(MUTED)
                .hidden(),
            "file",
        );
        let path_folder = ui.add_text(
            foot,
            Style::default()
                .text_size(11.0)
                .text_color(MUTED)
                .nowrap()
                .radius(RADIUS_SM)
                .hover(HOVER),
            "",
        );
        ui.set_name(path_folder, "asset path folder");
        let path_file = ui.add_text(
            foot,
            Style::default().text_size(11.0).text_color(LABEL).nowrap(),
            "",
        );
        ui.set_name(path_file, "asset path file");

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
            one_column: false,
            one_column_toggle,
            columns: body,
            files,
            file_lines: Vec::new(),
            file_depths: Vec::new(),
            line_arrows: HashMap::new(),
            line_looks: HashMap::new(),
            tiles: Vec::new(),
            selected: None,
            reveal: false,
            active: false,
            path_icon,
            path_folder,
            path_file,
            path_target: None,
            entries: HashMap::new(),
            pinged: None,
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
            editor_command: crate::preferences::DEFAULT_EDITOR.to_string(),
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

    /// The entry being dragged from the grid, if one is: what the Inspector
    /// lights a field for.
    pub fn dragged(&self, ui: &Ui) -> Option<Asset> {
        ui.dragging().and_then(|n| self.entries.get(&n).cloned())
    }

    /// The pictures drawn so far, by what each is asked for by
    /// ([`picture_key`]): what the Inspector shows beside an asset's name.
    pub fn pictures(&self) -> HashMap<String, ImageId> {
        self.thumbs
            .iter()
            .filter(|(_, (_, ready))| *ready)
            .map(|(name, (id, _))| (name.clone(), *id))
            .collect()
    }

    /// Show an entry in the grid — its folder chosen, the search left, its
    /// tile outlined — as Unity pings what an object field names.
    pub fn ping(&mut self, ui: &mut Ui, session: &Session, asset: Asset) {
        // Pointing at it is selecting it where it lives, in either layout.
        let key = asset_file(&asset, session).unwrap_or_else(|| match &asset {
            Asset::Prefab(n) | Asset::Model(n, _) | Asset::Material(n) | Asset::Sound(n, _) => {
                n.clone()
            }
            Asset::Scene(p) => p.display().to_string(),
        });
        if !self.show_asset(ui, session, &key) {
            ui.set_text(self.search, "");
            self.go_to(folder_of(&asset, session, &material_sources(session)));
        }
        self.selected = Some(Pick::Asset(asset.clone()));
        self.pinged = Some((asset, false));
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
                    scrap_ui::Color::TRANSPARENT
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
        for (tile, (folder, files)) in &self.tile_files {
            // A folder: marked when anything in it is.
            let on = if *folder {
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
    /// skip their updates. History is brought up to date when it comes on
    /// top, and Git asked again; the rest are kept up to date hidden, so a
    /// tab clicked redoes nothing else.
    pub fn set_visible(&mut self, ui: &mut Ui, session: &Session, visible: [bool; 4]) {
        if visible[3] && !self.visible[3] {
            self.git_stale = true;
        }
        let history = visible[2] && !self.visible[2];
        self.visible = visible;
        if history {
            self.update_history(ui, session);
        }
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
    assets_of(session, &session.assets().unwrap_or_default())
}

/// [`all_assets`] from a listing already read: the listing walks the
/// project on disk and reads every scene, so an update reads it once.
fn assets_of(session: &Session, entries: &[Entry]) -> Vec<Asset> {
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
            scrap::builtin::NAMES
                .iter()
                .map(|n| Asset::Model(n.to_string(), None)),
        );
        out.extend(
            entries
                .iter()
                .filter(|a| a.kind == "model")
                .map(|a| Asset::Model(a.name.clone(), Some(a.file.clone()))),
        );
        out.extend(
            entries
                .iter()
                .filter(|a| a.kind == "sound")
                .map(|a| Asset::Sound(a.name.clone(), a.file.clone())),
        );
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
    /// The Project: the two columns or the one tree, the path of what is
    /// chosen under them, the kind chips.
    fn update_project(&mut self, ui: &mut Ui, session: &Session) {
        let entries = session.assets().unwrap_or_default();
        let all = assets_of(session, &entries);
        let sources = sources_of(&entries);
        let folders = folders(&all, session, &sources);
        // A folder gone (moved, deleted): back to the project.
        if !self.folder.is_empty() && !folders.contains(&self.folder) {
            self.folder = String::new();
        }
        // The chosen thing gone (renamed, deleted): nothing is chosen.
        let there = match &self.selected {
            Some(Pick::Folder(f)) => folders.contains(f),
            Some(Pick::Asset(a)) => all.contains(a),
            None => true,
        };
        if !there {
            self.selected = None;
        }
        self.tile_files.retain(|node, _| ui.exists(*node));
        let one = self.one_column;
        ui.restyle(self.columns, |s| if one { s.hidden() } else { s.shown() });
        ui.restyle(self.files, |s| if one { s.shown() } else { s.hidden() });
        // Only the layout shown has nodes: a hidden one's would still be
        // found by name, and cost an update each.
        if one {
            for list in [self.tree, self.grid] {
                empty(ui, list);
            }
            ui.clear(self.crumbs);
            self.tiles.clear();
            self.update_files(ui, session, &all, &folders, &sources);
        } else {
            empty(ui, self.files);
            self.file_lines.clear();
            self.file_depths.clear();
            self.line_looks.clear();
            self.line_arrows.clear();
            self.update_tree(ui, &folders);
            self.update_crumbs(ui);
            self.update_grid(ui, session, &all, &folders, &sources);
        }
        self.show_marks(ui);
        self.update_path(ui, session, &sources);
        if self.reveal {
            self.reveal = false;
            let (list, shown) = if one {
                (self.files, &self.file_lines)
            } else {
                (self.grid, &self.tiles)
            };
            let node = shown
                .iter()
                .find(|(_, p)| Some(p) == self.selected.as_ref())
                .map(|(n, _)| *n);
            if let Some(node) = node {
                ui.scroll_to(list, node);
            }
        }

        for (chip, kind) in &self.kind_chips {
            let on = *kind == self.kind;
            ui.restyle(*chip, |s| {
                s.background(if on {
                    ACCENT_900
                } else {
                    scrap_ui::Color::TRANSPARENT
                })
            });
        }
    }

    /// The right column: the chosen folder's folders and assets, or what a
    /// search found anywhere.
    fn update_grid(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        all: &[Asset],
        folders: &[String],
        sources: &Sources,
    ) {
        let subfolders: Vec<String> = if self.searching(ui) {
            Vec::new()
        } else {
            folders
                .iter()
                .filter(|f| parent_folder(f) == self.folder && !f.is_empty())
                .cloned()
                .collect()
        };
        let assets = self.assets(ui, session, all, sources);
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
                let tint = tint_of(asset);
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
                            .border(1.0, scrap_ui::Color::TRANSPARENT)
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
                        .border(1.0, scrap_ui::Color::TRANSPARENT)
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
                picture(ui, frame, asset, 76.0, 28.0, Some((thumbs, next_image)));
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
        self.folder_tiles.clear();
        self.tiles.clear();
        let root = session.project().map(|p| p.root().to_path_buf());
        let tiles = ui.children(self.grid);
        for (tile, folder) in tiles.iter().zip(&subfolders) {
            self.folder_tiles.insert(*tile, folder.clone());
            if let (Some(root), false) = (&root, self.tile_files.contains_key(tile)) {
                self.tile_files.insert(*tile, (true, folder_files(root, folder)));
            }
            let chosen = self.selected == Some(Pick::Folder(folder.clone()));
            ui.restyle(*tile, |s| lit(s, chosen, false));
            self.tiles.push((*tile, Pick::Folder(folder.clone())));
        }
        for (tile, asset) in tiles.into_iter().skip(n_folders).zip(assets) {
            if let (Some(root), false) = (&root, self.tile_files.contains_key(&tile)) {
                if let Some(file) = asset_file(&asset, session) {
                    self.tile_files.insert(tile, (false, asset_files(root, &file)));
                }
            }
            let current = matches!(&asset, Asset::Scene(p) if Some(p) == open.as_ref());
            if let Some((p, scrolled)) = &mut self.pinged {
                if *p == asset && !*scrolled {
                    *scrolled = true;
                    ui.scroll_to(self.grid, tile);
                }
            }
            let chosen = self.selected.as_ref() == Some(&Pick::Asset(asset.clone()));
            ui.restyle(tile, |s| lit(s, chosen, current));
            self.tiles.push((tile, Pick::Asset(asset.clone())));
            self.entries.insert(tile, asset);
        }
    }

    /// The one tree: the folders, each open one's folders and files under
    /// it — or, while searching, what matches anywhere, as one flat list.
    /// Lines are kept by what they stand for, so opening a folder makes
    /// its lines and touches no other, and a line off screen costs no
    /// drawing (the list clips).
    fn update_files(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        all: &[Asset],
        folders: &[String],
        sources: &Sources,
    ) {
        let flat = self.searching(ui);
        let rows: Vec<(Pick, usize)> = if flat {
            self.assets(ui, session, all, sources)
                .into_iter()
                .map(|a| (Pick::Asset(a), 0))
                .collect()
        } else {
            let mut placed: HashMap<String, Vec<&Asset>> = HashMap::new();
            for a in all {
                placed
                    .entry(folder_of(a, session, sources))
                    .or_default()
                    .push(a);
            }
            let mut rows = Vec::new();
            fn walk(
                folders: &[String],
                placed: &HashMap<String, Vec<&Asset>>,
                open: &std::collections::HashSet<String>,
                at: &str,
                depth: usize,
                rows: &mut Vec<(Pick, usize)>,
            ) {
                for f in folders
                    .iter()
                    .filter(|f| !f.is_empty() && parent_folder(f) == at)
                {
                    rows.push((Pick::Folder(f.clone()), depth));
                    if open.contains(f) {
                        walk(folders, placed, open, f, depth + 1, rows);
                    }
                }
                for a in placed.get(at).into_iter().flatten() {
                    rows.push((Pick::Asset((*a).clone()), depth));
                }
            }
            walk(folders, &placed, &self.open_folders, "", 0, &mut rows);
            rows
        };
        let big = self.big;
        let keys: Vec<String> = rows
            .iter()
            .map(|(pick, _)| format!("{flat} {big} {pick:?}"))
            .collect();
        let index: HashMap<&String, usize> = keys.iter().enumerate().map(|(i, k)| (k, i)).collect();
        let mut made: Vec<(NodeId, usize)> = Vec::new();
        let thumbs = &mut self.thumbs;
        let next_image = &mut self.next_image;
        ui.sync_children(
            self.files,
            &keys,
            |ui, list, key| {
                let at = index[key];
                let (pick, depth) = &rows[at];
                // While searching, where each is: the list is flat.
                let hint = match pick {
                    Pick::Asset(a) if flat => path_label(&folder_of(a, session, sources)),
                    _ => String::new(),
                };
                let pictures = big.then_some((&mut *thumbs, &mut *next_image));
                let line = file_line(ui, list, pick, *depth, &hint, pictures);
                made.push((line, at));
                line
            },
            |_, _, _| {},
        );
        let root = session.project().map(|p| p.root().to_path_buf());
        self.entries.retain(|node, _| ui.exists(*node));
        self.line_arrows.retain(|node, _| ui.exists(*node));
        self.line_looks.retain(|node, _| ui.exists(*node));
        for (line, at) in made {
            let kids = ui.children(line);
            match &rows[at].0 {
                Pick::Folder(f) => {
                    self.line_arrows.insert(kids[1], f.clone());
                    if let Some(root) = &root {
                        self.tile_files.insert(line, (true, folder_files(root, f)));
                    }
                }
                Pick::Asset(a) => {
                    self.entries.insert(line, a.clone());
                    if let (Some(root), Some(file)) = (&root, asset_file(a, session)) {
                        self.tile_files.insert(line, (false, asset_files(root, &file)));
                    }
                }
            }
        }
        // Each line brought up to date — only those whose look changed: a
        // click touches two lines, not two thousand.
        let open = session.scene_path().map(|p| p.to_path_buf());
        let lines = ui.children(self.files);
        self.file_lines.clear();
        self.file_depths.clear();
        for (line, (pick, depth)) in lines.into_iter().zip(rows) {
            self.file_depths.push(depth);
            let chosen = self.selected.as_ref() == Some(&pick);
            let (unfolded, current) = match &pick {
                Pick::Folder(f) => (self.open_folders.contains(f), false),
                Pick::Asset(a) => (
                    false,
                    matches!(a, Asset::Scene(p) if Some(p) == open.as_ref()),
                ),
            };
            let look = (chosen, unfolded, current);
            if self.line_looks.get(&line) != Some(&look) {
                self.line_looks.insert(line, look);
                ui.restyle(line, |s| lit(s, chosen, current));
                if let Pick::Folder(_) = pick {
                    let kids = ui.children(line);
                    if let Some(arrow) = ui.children(kids[1]).first().copied() {
                        ui.set_icon(arrow, if unfolded { "chevron-down" } else { "chevron-right" });
                    }
                    if let Some(glyph) = ui.children(kids[2]).last().copied() {
                        ui.set_icon(glyph, if unfolded { "folder-open" } else { "folder" });
                    }
                }
            }
            self.file_lines.push((line, pick));
        }
    }

    /// Under the panel: where the chosen thing is, as the project names it
    /// (`assets/kenney/food/bread.glb`, `Built-in/cube`).
    fn update_path(&mut self, ui: &mut Ui, session: &Session, sources: &Sources) {
        let (glyph, folder, file) = match &self.selected {
            None => (None, String::new(), String::new()),
            Some(Pick::Folder(f)) => (
                Some("folder"),
                parent_folder(f),
                folder_label(f),
            ),
            Some(Pick::Asset(a)) => {
                let (folder, file) = match asset_file(a, session) {
                    Some(file) => match file.rsplit_once('/') {
                        Some((dir, name)) => (dir.to_string(), name.to_string()),
                        None => (String::new(), file),
                    },
                    // A material an import made: in the file it came from.
                    None => match (a, sources.get(&a.label())) {
                        (Asset::Material(_), Some(source)) => {
                            let (dir, name) = source.rsplit_once('/').unwrap_or(("", source));
                            (dir.to_string(), format!("{name} › {}", a.label()))
                        }
                        _ => (BUILTIN.to_string(), a.label()),
                    },
                };
                (Some(a.icon()), folder, file)
            }
        };
        match glyph {
            Some(g) => {
                ui.set_icon(self.path_icon, g);
                ui.restyle(self.path_icon, |s| s.shown());
            }
            None => ui.restyle(self.path_icon, |s| s.hidden()),
        }
        let shown = if folder.is_empty() {
            String::new()
        } else {
            format!("{}/", path_label(&folder))
        };
        ui.set_text(self.path_folder, &shown);
        ui.set_text(self.path_file, &file);
        self.path_target = (!shown.is_empty()).then_some(folder);
    }

    /// The Project's view, as the studio's layout file keeps it:
    /// `one-column` or `two-column`, then `pictures` or `list`.
    pub fn modes(&self) -> String {
        format!(
            "{} {}",
            if self.one_column {
                "one-column"
            } else {
                "two-column"
            },
            if self.big { "pictures" } else { "list" }
        )
    }

    /// Back to a view [`Bottom::modes`] wrote down.
    pub fn set_modes(&mut self, ui: &mut Ui, modes: &str) {
        self.one_column = modes.contains("one-column");
        self.big = !modes.contains("list");
        self.show_toggles(ui);
    }

    fn show_toggles(&self, ui: &mut Ui) {
        crate::theme::set_icon_button(ui, self.big_toggle, "image", self.big, true);
        crate::theme::set_icon_button(
            ui,
            self.one_column_toggle,
            "list-tree",
            self.one_column,
            true,
        );
    }

    /// Whether `node` is in the Project panel.
    pub fn owns_project(&self, ui: &Ui, node: NodeId) -> bool {
        let mut at = Some(node);
        while let Some(n) = at {
            if n == self.roots[0] {
                return true;
            }
            at = ui.parent(n);
        }
        false
    }

    /// Whether the Project has the keyboard's arrows, Enter, F2 and
    /// Delete: after a click on it, until a click elsewhere.
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    /// Go to an asset in the Project, choose it and scroll it into view —
    /// what a ping from an object field or the Hierarchy's "Show in
    /// Project" does.
    ///
    /// `file_or_name` is a project-relative file (`assets/kenney/food/
    /// bread.glb`, `scenes/main.ron`), an asset's name as the scene names
    /// it (`campfire`, `builtin:cube`, a material's name) or its label as
    /// the Project shows it (`cube`), or a folder (`assets/kenney`). The
    /// file is tried first, then the name, then the label; the first match
    /// wins.
    ///
    /// In two columns the grid goes to the asset's folder, the folder
    /// tree opened down to it; in one column the tree opens down to it.
    /// A search or kind filter is cleared so that it shows. The Inspector
    /// is not touched and the keyboard stays where it is: this points, it
    /// does not open. Returns `false`, changing nothing, when nothing in
    /// the project matches. The panel's nodes are brought up to date
    /// before this returns, so the caller can paint straight away.
    pub fn show_asset(&mut self, ui: &mut Ui, session: &Session, file_or_name: &str) -> bool {
        let entries = session.assets().unwrap_or_default();
        let all = assets_of(session, &entries);
        let sources = sources_of(&entries);
        let wanted = file_or_name.trim_end_matches('/').replace('\\', "/");
        let root = session.project().map(|p| p.root().to_path_buf());
        let file_of = |a: &Asset| -> Option<String> {
            match a {
                // A scene's path may come absolute.
                Asset::Scene(p) if p.to_string_lossy().replace('\\', "/") == wanted => {
                    Some(wanted.clone())
                }
                _ => asset_file(a, session),
            }
        };
        let name_of = |a: &Asset| match a {
            Asset::Prefab(n) | Asset::Model(n, _) | Asset::Material(n) | Asset::Sound(n, _) => {
                n.clone()
            }
            Asset::Scene(_) => String::new(),
        };
        let found = all
            .iter()
            .find(|a| file_of(a).as_deref() == Some(wanted.as_str()))
            .or_else(|| all.iter().find(|a| name_of(a) == wanted))
            .or_else(|| all.iter().find(|a| a.label() == wanted))
            .cloned();
        let pick = match found {
            Some(a) => Pick::Asset(a),
            None => {
                let folders = folders(&all, session, &sources);
                let relative = root
                    .as_ref()
                    .and_then(|r| std::path::Path::new(&wanted).strip_prefix(r).ok())
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or(wanted.clone());
                match folders.into_iter().find(|f| !f.is_empty() && *f == relative) {
                    Some(f) => Pick::Folder(f),
                    None => return false,
                }
            }
        };
        let folder = match &pick {
            Pick::Asset(a) => folder_of(a, session, &sources),
            Pick::Folder(f) => parent_folder(f),
        };
        ui.set_text(self.search, "");
        self.kind = "All";
        self.go_to(folder.clone());
        if self.one_column && !folder.is_empty() {
            self.open_folders.insert(folder);
        }
        self.selected = Some(pick);
        self.reveal = true;
        self.update_project(ui, session);
        true
    }

    /// Choose `pick`, show it in the Inspector when it is an asset, and
    /// bring it into view at the next [`Bottom::update_project`].
    fn choose(&mut self, pick: Pick, requests: &mut Requests) {
        if let Pick::Asset(a) = &pick {
            requests.inspect = Some(a.clone());
        }
        self.selected = Some(pick);
        self.reveal = true;
    }

    /// What a double click or Enter does: a folder goes in (or, in the one
    /// tree, opens or closes); a scene opens, a prefab opens in prefab
    /// mode, a model is placed, a material painted on, a sound played.
    fn open(&mut self, pick: Pick, requests: &mut Requests) {
        match pick {
            Pick::Folder(f) if self.one_column => {
                if !self.open_folders.remove(&f) {
                    self.open_folders.insert(f);
                }
            }
            Pick::Folder(f) => {
                self.go_to(f);
                self.selected = None;
            }
            Pick::Asset(Asset::Scene(path)) => requests.action = Some(Action::OpenScene(path)),
            Pick::Asset(Asset::Prefab(name)) => requests.action = Some(Action::OpenPrefab(name)),
            Pick::Asset(Asset::Model(name, _)) => requests.action = Some(Action::Place(name)),
            Pick::Asset(Asset::Material(name)) => {
                requests.action = Some(Action::SetField("material".into(), name))
            }
            Pick::Asset(Asset::Sound(name, _)) => requests.action = Some(Action::PlaySound(name)),
        }
        requests.refresh = true;
    }

    /// A key while the Project has the keyboard (see
    /// [`Bottom::set_active`]): the arrows walk the grid, or the tree
    /// (right opens a folder, left closes it or goes up to its folder);
    /// Enter does what a double click does; F2 renames the chosen asset's
    /// file; Delete asks, in a menu, before deleting it. `true` when it
    /// used the key.
    pub fn key(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        key: scrap::input::Key,
        requests: &mut Requests,
    ) -> bool {
        use scrap::input::Key;
        if !self.active || !self.visible[0] {
            return false;
        }
        let shown: Vec<(NodeId, Pick)> = if self.one_column {
            self.file_lines.clone()
        } else {
            self.tiles.clone()
        };
        let at = self
            .selected
            .as_ref()
            .and_then(|p| shown.iter().position(|(_, q)| q == p));
        match key {
            Key::Up | Key::Down | Key::Left | Key::Right => {
                if shown.is_empty() {
                    return true;
                }
                let last = shown.len() - 1;
                let Some(i) = at else {
                    // Nothing chosen here yet: the arrows start at the top.
                    self.choose(shown[0].1.clone(), requests);
                    self.update_project(ui, session);
                    return true;
                };
                let next = if self.one_column {
                    let depth = |j: usize| self.file_depths[j];
                    let folder = match &shown[i].1 {
                        Pick::Folder(f) => Some(f.clone()),
                        Pick::Asset(_) => None,
                    };
                    let unfolded = folder
                        .as_ref()
                        .is_some_and(|f| self.open_folders.contains(f));
                    match key {
                        Key::Up => i.checked_sub(1),
                        Key::Down => (i < last).then_some(i + 1),
                        Key::Right => match folder {
                            Some(f) if !unfolded => {
                                self.open_folders.insert(f);
                                None
                            }
                            Some(_) => (i < last).then_some(i + 1),
                            None => None,
                        },
                        _ => match folder {
                            Some(f) if unfolded => {
                                self.open_folders.remove(&f);
                                None
                            }
                            // Up to the folder it is in: the nearest line
                            // above that is less deep.
                            _ => {
                                let own = depth(i);
                                (0..i).rev().find(|j| depth(*j) < own)
                            }
                        },
                    }
                } else {
                    // How many tiles a row of the grid holds.
                    ui.layout();
                    let top = |n: NodeId| ui.layout_rect(n).map(|r| r.y.round());
                    let across = shown
                        .iter()
                        .take_while(|(n, _)| top(*n) == top(shown[0].0))
                        .count()
                        .max(1);
                    match key {
                        Key::Left => i.checked_sub(1),
                        Key::Right => (i < last).then_some(i + 1),
                        Key::Up => i.checked_sub(across),
                        _ => (i + across <= last).then_some(i + across),
                    }
                };
                if let Some(j) = next {
                    self.choose(shown[j].1.clone(), requests);
                }
                self.update_project(ui, session);
            }
            Key::Enter => {
                if let Some(pick) = self.selected.clone() {
                    self.open(pick, requests);
                }
            }
            Key::F2 => {
                if let Some(Pick::Asset(a)) = &self.selected {
                    match asset_file(a, session) {
                        Some(file) => requests.action = Some(Action::AssetRename(file)),
                        None => return true,
                    }
                }
            }
            Key::Delete | Key::Backspace => {
                let command = {
                    let (_, ctrl, _, cmd) = ui.modifiers();
                    ctrl || cmd
                };
                if key == Key::Backspace && !command {
                    return false;
                }
                let (Some(i), Some(Pick::Asset(a))) = (at, &self.selected) else {
                    return true;
                };
                if let Some(file) = asset_file(a, session) {
                    // Asked first, as Unity asks: the menu is the question,
                    // Escape or a click elsewhere the no.
                    let r = ui.rect(shown[i].0);
                    requests.menu = Some((
                        vec![crate::menu::MenuItem::new(
                            &format!("Delete {file}"),
                            Action::AssetDelete(file),
                        )],
                        r.x,
                        r.y + r.height,
                    ));
                }
            }
            _ => return false,
        }
        true
    }

    pub fn update(&mut self, ui: &mut Ui, session: &Session) {
        self.update_project(ui, session);

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
                        scrap_ui::Color::TRANSPARENT
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
                        .background(scrap_ui::Color::TRANSPARENT)
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
                        scrap_ui::Color::TRANSPARENT
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
                        open_in_editor(session, &at, &self.editor_command);
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
                let folder = self.folder_tiles[&node].clone();
                if *count >= 2 {
                    self.go_to(folder);
                    self.selected = None;
                } else {
                    self.selected = Some(Pick::Folder(folder));
                }
                requests.refresh = true;
            }
            Event::Click { .. } if self.line_arrows.contains_key(&node) => {
                let folder = self.line_arrows[&node].clone();
                if !self.open_folders.remove(&folder) {
                    self.open_folders.insert(folder);
                }
                requests.refresh = true;
            }
            Event::Click { count, .. }
                if self
                    .file_lines
                    .iter()
                    .any(|(n, p)| *n == node && matches!(p, Pick::Folder(_))) =>
            {
                let pick = self
                    .file_lines
                    .iter()
                    .find(|(n, _)| *n == node)
                    .map(|(_, p)| p.clone())
                    .expect("checked");
                if *count >= 2 {
                    self.open(pick, requests);
                } else {
                    self.selected = Some(pick);
                    requests.refresh = true;
                }
            }
            Event::Click { .. } if node == self.path_folder => {
                if let Some(folder) = self.path_target.clone() {
                    if self.one_column {
                        // The one tree has no folder to go to: the folder's
                        // line is chosen instead, opened down to.
                        self.go_to(folder.clone());
                        self.selected = Some(Pick::Folder(folder));
                        self.reveal = true;
                    } else {
                        self.go_to(folder);
                    }
                    requests.refresh = true;
                }
            }
            Event::Click { .. } if node == self.one_column_toggle => {
                self.one_column = !self.one_column;
                self.reveal = true;
                self.show_toggles(_ui);
                requests.refresh = true;
            }
            Event::Click { .. } if node == self.big_toggle => {
                self.big = !self.big;
                self.show_toggles(_ui);
                requests.refresh = true;
            }
            Event::Click { .. } if self.kind_chips.iter().any(|(c, _)| *c == node) => {
                self.kind = self.kind_chips.iter().find(|(c, _)| *c == node).unwrap().1;
                requests.refresh = true;
            }
            Event::Click {
                button: scrap::input::MouseButton::Right,
                ..
            } if self.entries.contains_key(&node) => {
                let asset = self.entries[&node].clone();
                self.selected = Some(Pick::Asset(asset.clone()));
                self.update_project(_ui, session);
                let (x, y) = _ui.pointer();
                requests.menu = Some((asset_menu(&asset, session), x, y));
            }
            Event::Click { count, .. } if *count >= 2 => {
                if let Some(asset) = self.entries.get(&node).cloned() {
                    self.open(Pick::Asset(asset), requests);
                }
            }
            Event::Click {
                count: 1,
                button: scrap::input::MouseButton::Left,
            } => {
                if let Some(asset) = self.entries.get(&node).cloned() {
                    self.pinged = None;
                    self.selected = Some(Pick::Asset(asset.clone()));
                    requests.inspect = Some(asset);
                    // Lit here and now: a refresh of every panel would
                    // take the Inspector back to the scene's selection.
                    self.update_project(_ui, session);
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

/// Open the file a Console line points at: with the person's code editor
/// at the line (Preferences › External Tools; VS Code's `code -g` unless
/// they chose another), else with whatever the system opens it with.
pub(crate) fn open_in_editor(
    session: &mut Session,
    at: &scrap_editor::console::Location,
    command: &str,
) {
    let root = session
        .project()
        .map(|p| p.root().to_path_buf())
        .unwrap_or_default();
    let file = root.join(&at.file);
    if !file.is_file() {
        session.say(Level::Warning, format!("{} is not in the project", at.file));
        return;
    }
    let opened = crate::preferences::editor_command(command, &file, at.line, at.column)
        .map(|(program, args)| std::process::Command::new(program).args(args).spawn());
    if !matches!(opened, Some(Ok(_))) {
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
            let p = root.join("materials").join(format!("{n}.scrmat"));
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
pub fn picture_key(asset: &Asset) -> Option<String> {
    match asset {
        Asset::Model(name, _) | Asset::Prefab(name) => Some(name.clone()),
        Asset::Material(name) => Some(format!("{}{name}", scrap_editor::MATERIAL_PICTURE)),
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
    sources_of(&session.assets().unwrap_or_default())
}

/// [`material_sources`] from a listing already read.
fn sources_of(entries: &[Entry]) -> Sources {
    entries
        .iter()
        .filter(|e| e.kind == "material")
        .map(|e| (e.name.clone(), e.file.clone()))
        .collect()
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
                .border(1.0, scrap_ui::Color::TRANSPARENT)
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

/// What can be chosen in the Project: a folder, or an entry.
#[derive(Debug, Clone, PartialEq)]
enum Pick {
    Folder(String),
    Asset(Asset),
}

/// A folder as the path under the panel names it: project-relative, the
/// engine's own `Built-in`.
fn path_label(folder: &str) -> String {
    match folder.strip_prefix(BUILTIN) {
        Some(rest) => format!("Built-in{rest}"),
        None => folder.to_string(),
    }
}

/// A prefab's icon is the accent's, as the Hierarchy draws instances; the
/// rest are muted.
fn tint_of(asset: &Asset) -> scrap_ui::Color {
    if matches!(asset, Asset::Prefab(_)) {
        ACCENT
    } else {
        MUTED
    }
}

/// A tile or line as chosen (`chosen`), as the open scene (`current`), or
/// as neither.
fn lit(s: Style, chosen: bool, current: bool) -> Style {
    if chosen {
        s.border(1.0, ACCENT.alpha(60)).background(ACCENT_800)
    } else if current {
        s.border(1.0, ACCENT.alpha(40)).background(ACCENT_900)
    } else {
        s.border(1.0, scrap_ui::Color::TRANSPARENT)
            .background(scrap_ui::Color::TRANSPARENT)
    }
}

/// No children left in a keyed list.
fn empty(ui: &mut Ui, list: NodeId) {
    if !ui.children(list).is_empty() {
        ui.sync_children(list, &[] as &[u8], |ui, l, _| ui.add(l, Style::row()), |_, _, _| {});
    }
}

/// A folder's directory, absolute, for its git dot.
fn folder_files(root: &std::path::Path, folder: &str) -> Vec<PathBuf> {
    let dir = root.join(folder);
    vec![dir.canonicalize().unwrap_or(dir)]
}

/// An asset's files, absolute, for its git dot: its own and its import
/// settings beside it.
fn asset_files(root: &std::path::Path, file: &str) -> Vec<PathBuf> {
    let file = root.join(file);
    let settings = file.with_extension(format!(
        "{}.scrimport",
        file.extension()
            .map(|e| e.to_string_lossy())
            .unwrap_or_default()
    ));
    [file, settings]
        .into_iter()
        .map(|f| f.canonicalize().unwrap_or(f))
        .collect()
}

/// An entry's picture in `frame`, `size` across: the drawn picture when
/// pictures are on and it has one (see [`Bottom::wanted_pictures`]), its
/// kind's icon, `glyph` across, until then — and for a sound.
fn picture(
    ui: &mut Ui,
    frame: NodeId,
    asset: &Asset,
    size: f32,
    glyph: f32,
    pictures: Pictures,
) {
    let mut drawn = false;
    if let (Some(name), Some((thumbs, next_image))) = (picture_key(asset), pictures) {
        let (id, ready) = *thumbs.entry(name.clone()).or_insert_with(|| {
            *next_image += 1;
            (ImageId(*next_image), false)
        });
        let img = ui.add_image(
            frame,
            Style::default()
                .size(size, size)
                .radius(if size > 24.0 { RADIUS_SM } else { 2.0 }),
            id,
        );
        ui.set_name(img, format!("thumb {name}"));
        drawn = ready;
    }
    let at = (size - glyph) / 2.0;
    let g = ui.add_icon(
        frame,
        Style::default()
            .size(glyph, glyph)
            .text_color(tint_of(asset))
            .absolute(at, at),
        asset.icon(),
    );
    if drawn {
        ui.restyle(g, |s| s.hidden());
    }
}

/// The pictures' images by name (see [`Bottom::wanted_pictures`]) and the
/// number of the last image made — for a node that wants a picture, when
/// pictures are on.
type Pictures<'a> = Option<(&'a mut HashMap<String, (ImageId, bool)>, &'a mut u32)>;

/// How far in each level of the one tree goes.
const INDENT: f32 = 14.0;

/// A line of the one tree: the guides of the levels it is under, an arrow
/// (a folder's; an asset's is empty room, so names line up), its icon or
/// picture, its name, where it is while searching, its git dot.
fn file_line(
    ui: &mut Ui,
    list: NodeId,
    pick: &Pick,
    depth: usize,
    hint: &str,
    pictures: Pictures,
) -> NodeId {
    const H: f32 = 20.0;
    let base = Style::row()
        .height(H)
        .fixed()
        .full_width()
        .padding_left(4.0 + depth as f32 * INDENT)
        .gap(4.0)
        .center_items()
        .radius(RADIUS_SM)
        .border(1.0, scrap_ui::Color::TRANSPARENT)
        .hover(HOVER);
    let (line, label) = match pick {
        Pick::Folder(f) => {
            let label = folder_label(f);
            let line = ui.add(list, base.clickable());
            ui.set_name(line, format!("folder line {label}"));
            (line, label)
        }
        Pick::Asset(a) => {
            let label = a.label();
            let line = ui.add(list, base.draggable());
            ui.set_name(line, format!("asset {label}"));
            (line, label)
        }
    };
    // 0: the guides, one faint line down each level above it.
    let guides = ui.add(
        line,
        Style::row()
            .absolute(0.0, 0.0)
            .size(8.0 + depth as f32 * INDENT, H),
    );
    for d in 0..depth {
        ui.add(
            guides,
            Style::row()
                .absolute(4.0 + d as f32 * INDENT + 6.0, 0.0)
                .size(1.0, H)
                .background(DIVIDER),
        );
    }
    // 1: the arrow.
    let arrow = ui.add(
        line,
        Style::row()
            .size(12.0, 12.0)
            .fixed()
            .center()
            .radius(RADIUS_SM),
    );
    // 2: the icon or picture.
    let frame = ui.add(line, Style::row().size(16.0, 16.0).fixed().center());
    match pick {
        Pick::Folder(f) => {
            ui.restyle(arrow, |s| s.clickable());
            ui.set_name(arrow, format!("folder line arrow {label}"));
            ui.add_icon(
                arrow,
                Style::default().size(12.0, 12.0).text_color(MUTED),
                "chevron-right",
            );
            ui.add_icon(
                frame,
                Style::default().size(15.0, 15.0).text_color(NEUTRAL_500),
                "folder",
            );
            let _ = f;
        }
        Pick::Asset(a) => picture(ui, frame, a, 16.0, 13.0, pictures),
    }
    // 3: the name; 4: where it is, while searching.
    ui.add_text(line, text().text_size(12.0), &label);
    ui.add_text(
        line,
        Style::default()
            .text_size(11.0)
            .text_color(MUTED)
            .nowrap()
            .fill(),
        hint,
    );
    // 5: the git dot, last, where `show_marks` looks for it.
    let dot = ui.add(
        line,
        Style::row()
            .size(6.0, 6.0)
            .fixed()
            .radius(3.0)
            .background(ACCENT_400)
            .opacity(0.0),
    );
    ui.set_name(dot, format!("asset mark {label}"));
    line
}
