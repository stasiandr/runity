//! Preferences: the person's settings, the same in every project.
//!
//! Unity has two windows where this editor used to have one: Project
//! Settings — the project's files, in git, what every teammate gets
//! (`crate::tools::Settings`) — and Preferences, which are one person's
//! and follow them from project to project. This is the second: opened
//! with ⌘, (Ctrl+, elsewhere), from «runity › Settings…» on a Mac or
//! Edit › Preferences… elsewhere, in a window of its own that comes back
//! where it was left.
//!
//! Its pages:
//!
//! * **Appearance** — the palette (`crate::appearance`).
//! * **Keys** — every command's key (`crate::keymap`), searchable; a click
//!   on a key and a press of another rebinds it.
//! * **External Tools** — the code editor a Console line opens in, the
//!   Blender a `.blend` is read with, and which git is used.
//! * **General** — whether the scene open last opens on start, and how
//!   often an idle editor draws.
//! * **Layouts** — the layouts saved with Window › Save Layout As…,
//!   renamed or deleted.
//!
//! What is kept, and where: the colours in `theme.ron`, the keys in
//! `keys.ron`, the rest in `preferences.ron`, the layouts in `layouts/` —
//! all in the person's settings folder (`appearance::config_dir`). Things
//! that are one person's but about one project — where the view was in
//! each scene, the snap steps, Inspector Debug — stay in the project's
//! `.runity/editor.ron` (`runity_editor::prefs`): they mean nothing in
//! another project.
//!
//! Postulate 2 (git) is why the split matters: nothing here is in a
//! project, so choosing a colour or a key never makes a diff a teammate
//! has to read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use runity_ui::{Color, Event, NodeId, Style, Ui};
use serde::{Deserialize, Serialize};

use crate::appearance::Appearance;
use crate::keymap::Keymap;
use crate::theme::*;

/// The file, in the settings folder.
pub const FILE: &str = "preferences.ron";

/// What opens a file at a line when nothing else is set: VS Code's `code`.
pub const DEFAULT_EDITOR: &str = "code -g {file}:{line}:{column}";

/// How many frames a second an editor with nothing happening draws.
pub const DEFAULT_IDLE_FPS: u32 = 4;

/// A page of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Appearance,
    Keys,
    Tools,
    General,
    Layouts,
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::Appearance,
        Page::Keys,
        Page::Tools,
        Page::General,
        Page::Layouts,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Page::Appearance => "appearance",
            Page::Keys => "keys",
            Page::Tools => "tools",
            Page::General => "general",
            Page::Layouts => "layouts",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::Keys => "Keys",
            Page::Tools => "External Tools",
            Page::General => "General",
            Page::Layouts => "Layouts",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Page::Appearance => "palette",
            Page::Keys => "keyboard",
            Page::Tools => "wrench",
            Page::General => "settings",
            Page::Layouts => "layout-dashboard",
        }
    }
}

/// `preferences.ron`: what is the person's and is not a colour, a key or a
/// layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Personal {
    /// How a file is opened at a line: a command with `{file}`, `{line}`
    /// and `{column}` in it. Empty: [`DEFAULT_EDITOR`].
    pub code_editor: String,
    /// Blender's executable. Empty: found as `runity_import::blend::blender`
    /// finds it (`RUNITY_BLENDER`, where it installs, the `PATH`).
    pub blender: String,
    /// Open the scene open last when the editor starts with none named.
    pub open_last_scene: bool,
    /// Frames a second while nothing happens.
    pub idle_fps: u32,
    /// Where each window of its own was left, by its panel's name: x, y,
    /// width, height in logical pixels.
    pub windows: BTreeMap<String, [f32; 4]>,
}

impl Default for Personal {
    fn default() -> Self {
        Self {
            code_editor: String::new(),
            blender: String::new(),
            open_last_scene: true,
            idle_fps: DEFAULT_IDLE_FPS,
            windows: BTreeMap::new(),
        }
    }
}

impl Personal {
    /// The person's, or the defaults. A file that does not read gives the
    /// defaults and says why.
    pub fn load(dir: Option<&Path>) -> (Self, Option<String>) {
        let Some(file) = dir.map(|d| d.join(FILE)) else {
            return (Self::default(), None);
        };
        match std::fs::read_to_string(&file) {
            Ok(text) => match runity::ron::from_str(&text) {
                Ok(this) => (this, None),
                Err(e) => (Self::default(), Some(format!("{}: {e}", file.display()))),
            },
            Err(_) => (Self::default(), None),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let text = runity::ron::ser::to_string_pretty(self, Default::default())
            .map_err(|e| e.to_string())?;
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let file = dir.join(FILE);
        std::fs::write(
            &file,
            format!("// Yours, in every project: Preferences.\n{text}\n"),
        )
        .map_err(|e| format!("{}: {e}", file.display()))
    }

    /// The code editor command in force.
    pub fn editor(&self) -> &str {
        match self.code_editor.trim() {
            "" => DEFAULT_EDITOR,
            set => set,
        }
    }

    /// How long an idle editor waits between frames.
    pub fn idle(&self) -> std::time::Duration {
        let fps = self.idle_fps.clamp(1, 60);
        std::time::Duration::from_millis(1000 / fps as u64)
    }
}

/// The program and arguments that open `file` at `line`:`column` with
/// `template` — its words, with `{file}`, `{line}` and `{column}` put in,
/// and the file at the end when the template never says where. A word in
/// double quotes may have spaces. `None` for an empty template.
pub fn editor_command(
    template: &str,
    file: &Path,
    line: u32,
    column: u32,
) -> Option<(String, Vec<String>)> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in template.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            c => {
                word.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(word);
    }
    if words.is_empty() {
        return None;
    }
    let file = file.display().to_string();
    let says_where = words.iter().any(|w| w.contains("{file}"));
    let mut words: Vec<String> = words
        .into_iter()
        .map(|w| {
            w.replace("{file}", &file)
                .replace("{line}", &line.to_string())
                .replace("{column}", &column.to_string())
        })
        .collect();
    if !says_where {
        words.push(file);
    }
    let program = words.remove(0);
    Some((program, words))
}

/// A program on the `PATH`, as a shell would find it.
pub fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    std::env::split_paths(&path)
        .map(|dir| dir.join(&exe))
        .find(|p| p.is_file())
}

/// What a click or a field on the window asks the studio for.
#[derive(Debug, Clone, PartialEq)]
pub enum Asked {
    Theme(crate::appearance::Change),
    /// Listen for the next key: it is this command's.
    Capture(&'static str),
    ResetKey(&'static str),
    ResetAllKeys,
    CodeEditor(String),
    Blender(String),
    /// Open the scene open last on start, or not: whichever it is not.
    ToggleOpenLastScene,
    IdleFps(u32),
    ApplyLayout(String),
    RenameLayout(String, String),
    DeleteLayout(String),
}

struct KeyRow {
    id: &'static str,
    group: &'static str,
    row: NodeId,
    chip: NodeId,
    chip_text: NodeId,
    reset: NodeId,
    /// What the search looks in: the label, the group, the name, the keys.
    words: String,
}

/// The window's content: pages on the left, the page on the right.
pub struct Preferences {
    pub root: NodeId,
    pub appearance: Appearance,
    page: Page,
    nav: Vec<(NodeId, Page)>,
    pages: Vec<(NodeId, Page)>,
    // Keys.
    keys_where: NodeId,
    search: NodeId,
    reset_all: NodeId,
    keys_note: NodeId,
    groups: Vec<(NodeId, &'static str)>,
    rows: Vec<KeyRow>,
    capturing: Option<&'static str>,
    // External tools.
    editor: NodeId,
    blender: NodeId,
    blender_found: NodeId,
    git_found: NodeId,
    // General.
    open_last: NodeId,
    idle: NodeId,
    general_where: NodeId,
    tools_where: NodeId,
    // Layouts.
    layouts_list: NodeId,
    layouts: Vec<(NodeId, NodeId, NodeId, String)>,
    shown_layouts: Option<Vec<String>>,
}

impl Preferences {
    pub fn new(ui: &mut Ui, parent: NodeId, keymap: &Keymap) -> Self {
        let root = ui.add(parent, Style::row().fill().full_width().full_height());
        ui.set_name(root, "preferences");
        let left = ui.add(
            root,
            Style::column()
                .width(176.0)
                .full_height()
                .fixed()
                .padding(SPACE_2)
                .gap(2.0),
        );
        let mut nav = Vec::new();
        for page in Page::ALL {
            let row = nav_row(ui, left, page.icon(), page.label());
            ui.set_name(row, format!("preferences {}", page.name()));
            nav.push((row, page));
        }
        ui.add(
            root,
            Style::column()
                .width(1.0)
                .full_height()
                .fixed()
                .background(DIVIDER),
        );
        let body = ui.add(root, Style::column().fill().full_height());
        let mut pages = Vec::new();
        let mut page_node = |ui: &mut Ui, page: Page| {
            let node = ui.add(body, Style::column().fill().full_width().hidden());
            ui.set_name(node, format!("preferences page {}", page.name()));
            pages.push((node, page));
            node
        };

        let appearance_page = page_node(ui, Page::Appearance);
        let appearance = Appearance::new(ui, appearance_page);

        // Keys.
        let keys = page_node(ui, Page::Keys);
        let head = page_head(ui, keys, "Keys");
        let keys_where = ui.add_text(head, caption().text_size(11.0), "");
        let bar = ui.add(
            keys,
            Style::row()
                .full_width()
                .padding_x(SPACE_4)
                .gap(SPACE_2)
                .center_items(),
        );
        let search = ui.add_field(bar, field_style().fill(), "");
        ui.set_placeholder(search, "Search commands or keys");
        ui.set_name(search, "keys search");
        let reset_all = button(ui, bar, "keys reset all", "Reset All", false);
        let keys_note = ui.add_text(
            keys,
            text()
                .text_size(11.5)
                .text_color(LABEL)
                .padding_x(SPACE_4)
                .padding_y(SPACE_2),
            "Click a key, then press the one you want. Esc cancels.",
        );
        ui.set_name(keys_note, "keys note");
        let keys_list = ui.add(
            keys,
            Style::column()
                .fill()
                .full_width()
                .padding_x(SPACE_4)
                .padding_y(SPACE_1)
                .gap(1.0)
                .clip(),
        );
        ui.set_name(keys_list, "keys list");
        let mut groups = Vec::new();
        let mut rows = Vec::new();
        let mut order: Vec<&'static str> = Vec::new();
        for c in keymap.commands() {
            if !order.contains(&c.group) {
                order.push(c.group);
            }
        }
        for group in order {
            let g = ui.add_text(
                keys_list,
                caption()
                    .text_color(LABEL)
                    .padding_left(SPACE_2)
                    .padding_top(SPACE_4)
                    .padding_bottom(SPACE_1),
                &group.to_uppercase(),
            );
            groups.push((g, group));
            for c in keymap.commands().iter().filter(|c| c.group == group) {
                let row = ui.add(
                    keys_list,
                    Style::row()
                        .full_width()
                        .height(28.0)
                        .fixed()
                        .padding_x(SPACE_2)
                        .gap(SPACE_2)
                        .center_items()
                        .radius(RADIUS_SM)
                        .hover(HOVER),
                );
                ui.set_name(row, format!("key row {}", c.id));
                ui.add_text(row, text().fill(), c.label);
                let reset = icon_button(ui, row, &format!("key reset {}", c.id), "undo-2", false);
                ui.restyle(reset, |s| s.size(22.0, 22.0).hidden());
                let chip = ui.add(
                    row,
                    Style::row()
                        .min_width(104.0)
                        .height(22.0)
                        .fixed()
                        .padding_x(SPACE_3)
                        .center()
                        .radius(6.0)
                        .background(BG)
                        .border(1.0, DIVIDER)
                        .hover_border(TEXT.alpha(35))
                        .clickable(),
                );
                ui.set_name(chip, format!("key {}", c.id));
                let chip_text = ui.add_text(chip, text().text_size(12.0), "");
                rows.push(KeyRow {
                    id: c.id,
                    group,
                    row,
                    chip,
                    chip_text,
                    reset,
                    words: format!("{} {} {}", c.label, c.group, c.id).to_lowercase(),
                });
            }
        }

        // External tools.
        let tools = page_node(ui, Page::Tools);
        let head = page_head(ui, tools, "External Tools");
        let tools_where = ui.add_text(head, caption().text_size(11.0), "");
        let form = add_form(ui, tools);
        let (editor, _) = form_field(
            ui,
            form,
            "Code editor",
            "tools editor",
            DEFAULT_EDITOR,
            "Opens a Console line's file. {file}, {line} and {column} are put in; empty is VS Code.",
        );
        ui.restyle(editor, |s| s.mono());
        let (blender, blender_found) =
            form_field(ui, form, "Blender", "tools blender", "found by itself", "");
        ui.set_name(blender_found, "tools blender found");
        let git_row = form_row(ui, form, "Git");
        let git_found = ui.add_text(git_row, text().text_color(LABEL).fill(), "");
        ui.set_name(git_found, "tools git found");

        // General.
        let general = page_node(ui, Page::General);
        let head = page_head(ui, general, "General");
        let general_where = ui.add_text(head, caption().text_size(11.0), "");
        let form = add_form(ui, general);
        let row = form_row(ui, form, "On start");
        let open_last = ui.add(
            row,
            Style::row()
                .gap(SPACE_2)
                .center_items()
                .radius(RADIUS_SM)
                .clickable(),
        );
        ui.set_name(open_last, "general open last");
        let check = ui.add(
            open_last,
            Style::row()
                .size(16.0, 16.0)
                .fixed()
                .center()
                .radius(4.0)
                .border(1.0, DIVIDER),
        );
        icon(ui, check, "check", ACCENT);
        ui.add_text(open_last, text(), "Open the scene I had open last");
        let (idle, _) = form_field(
            ui,
            form,
            "Idle frame rate",
            "general idle fps",
            "4",
            "Frames a second while nothing moves and nobody touches anything.",
        );
        ui.restyle(idle, |s| s.unfilled().width(64.0));

        // Layouts.
        let layouts = page_node(ui, Page::Layouts);
        let head = page_head(ui, layouts, "Layouts");
        ui.add_text(
            head,
            caption().text_size(11.0),
            "Saved with Window › Save Layout As… — yours, in every project. Enter renames.",
        );
        let layouts_list = ui.add(
            layouts,
            Style::column()
                .fill()
                .full_width()
                .padding_x(SPACE_4)
                .gap(SPACE_1)
                .clip(),
        );
        ui.set_name(layouts_list, "layouts list");

        let mut this = Self {
            root,
            appearance,
            page: Page::Appearance,
            nav,
            pages,
            keys_where,
            search,
            reset_all,
            keys_note,
            groups,
            rows,
            capturing: None,
            editor,
            blender,
            blender_found,
            git_found,
            open_last,
            idle,
            general_where,
            tools_where,
            layouts_list,
            layouts: Vec::new(),
            shown_layouts: None,
        };
        this.set_page(ui, Page::Appearance);
        this
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn set_page(&mut self, ui: &mut Ui, page: Page) {
        self.page = page;
        for (node, p) in &self.pages {
            let on = *p == page;
            ui.restyle(*node, |s| if on { s.shown() } else { s.hidden() });
        }
        for (row, p) in &self.nav {
            let on = *p == page;
            ui.restyle(*row, |s| {
                s.background(if on { ACCENT_900 } else { Color::TRANSPARENT })
            });
        }
        if page != Page::Keys {
            self.capturing = None;
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

    /// The command waiting for its new key, if one is.
    pub fn capturing(&self) -> Option<&'static str> {
        self.capturing
    }

    pub fn stop_capture(&mut self) {
        self.capturing = None;
    }

    /// Say something under the search: a conflict resolved, a key taken.
    pub fn note(&mut self, ui: &mut Ui, text: &str, warn: bool) {
        ui.set_text(self.keys_note, text);
        ui.restyle(self.keys_note, |s| {
            s.text_color(if warn { WARNING } else { LABEL })
        });
    }

    /// Where the pages' files are, under their titles.
    pub fn show_where(&mut self, ui: &mut Ui, dir: Option<&Path>) {
        for (node, file) in [
            (self.keys_where, crate::keymap::FILE),
            (self.general_where, FILE),
            (self.tools_where, FILE),
        ] {
            ui.set_text(
                node,
                &match dir {
                    Some(d) => {
                        format!("Yours, in every project — kept in {}", tilde(&d.join(file)))
                    }
                    None => "No settings folder: changes last until you quit.".into(),
                },
            );
        }
    }

    /// The Keys page as the keymap has it, filtered by the search.
    pub fn show_keys(&mut self, ui: &mut Ui, keymap: &Keymap) {
        let query = ui
            .text(self.search)
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let mut shown_groups: Vec<&'static str> = Vec::new();
        for row in &self.rows {
            let keys = keymap.keys(row.id);
            let label = if self.capturing == Some(row.id) {
                "Press a key…".to_string()
            } else if keys.is_empty() {
                "—".to_string()
            } else {
                keys.iter()
                    .map(|k| k.label())
                    .collect::<Vec<_>>()
                    .join("  ")
            };
            ui.set_text(row.chip_text, &label);
            let listening = self.capturing == Some(row.id);
            ui.restyle(row.chip, |s| {
                if listening {
                    s.border(1.5, ACCENT).background(ACCENT_HOVER)
                } else {
                    s.border(1.0, DIVIDER).background(BG)
                }
            });
            ui.restyle(row.chip_text, |s| {
                s.text_color(if listening {
                    ACCENT
                } else if keys.is_empty() {
                    MUTED
                } else {
                    TEXT
                })
            });
            let changed = !keymap.is_default(row.id);
            ui.restyle(row.reset, |s| if changed { s.shown() } else { s.hidden() });
            let matches = query.is_empty()
                || row.words.contains(&query)
                || label.to_lowercase().contains(&query);
            ui.restyle(row.row, |s| if matches { s.shown() } else { s.hidden() });
            if matches && !shown_groups.contains(&row.group) {
                shown_groups.push(row.group);
            }
        }
        for (node, group) in &self.groups {
            let on = shown_groups.contains(group);
            ui.restyle(*node, |s| if on { s.shown() } else { s.hidden() });
        }
        let any = self.rows.iter().any(|r| !keymap.is_default(r.id));
        ui.restyle(self.reset_all, |s| s.opacity(if any { 1.0 } else { 0.45 }));
    }

    /// External Tools and General as the person's preferences have them.
    pub fn show_personal(&mut self, ui: &mut Ui, personal: &Personal) {
        if ui.focused() != Some(self.editor) {
            ui.set_text(self.editor, &personal.code_editor);
        }
        if ui.focused() != Some(self.blender) {
            ui.set_text(self.blender, &personal.blender);
        }
        let found = if personal.blender.trim().is_empty() {
            match runity_import::blend::blender() {
                Some(p) => format!("Found: {}", tilde(&p)),
                None => "Not found — install Blender, or put its executable here.".into(),
            }
        } else if Path::new(personal.blender.trim()).is_file() {
            "Used for .blend files and the Blender link.".into()
        } else {
            "No file there.".into()
        };
        ui.set_text(self.blender_found, &found);
        ui.set_text(
            self.git_found,
            &match on_path("git") {
                Some(p) => format!("{} — from the PATH", tilde(&p)),
                None => "Not on the PATH: History and Git need it.".into(),
            },
        );
        let on = personal.open_last_scene;
        if let Some(check) = ui.children(self.open_last).first().copied() {
            ui.restyle(check, |s| {
                if on {
                    s.border(1.0, ACCENT).background(ACCENT_HOVER)
                } else {
                    s.border(1.0, DIVIDER).background(Color::TRANSPARENT)
                }
            });
            if let Some(mark) = ui.children(check).first().copied() {
                ui.restyle(mark, |s| if on { s.shown() } else { s.hidden() });
            }
        }
        if ui.focused() != Some(self.idle) {
            ui.set_text(self.idle, &personal.idle_fps.to_string());
        }
    }

    /// The Layouts page: one line per saved layout.
    pub fn show_layouts(&mut self, ui: &mut Ui, saved: Vec<String>) {
        if self.shown_layouts.as_ref() == Some(&saved) {
            return;
        }
        ui.clear(self.layouts_list);
        self.layouts.clear();
        if saved.is_empty() {
            ui.add_text(
                self.layouts_list,
                text().text_color(MUTED).padding_y(SPACE_2),
                "No saved layouts yet.",
            );
        }
        for name in &saved {
            let row = ui.add(
                self.layouts_list,
                Style::row()
                    .full_width()
                    .height(30.0)
                    .fixed()
                    .gap(SPACE_2)
                    .center_items(),
            );
            icon(ui, row, "layout-dashboard", MUTED);
            let field = ui.add_field(row, field_style().fill(), name);
            ui.set_name(field, format!("layout name {name}"));
            let apply = button(ui, row, &format!("layout apply {name}"), "Apply", false);
            let delete = icon_button(ui, row, &format!("layout delete {name}"), "trash", false);
            self.layouts.push((field, apply, delete, name.clone()));
        }
        self.shown_layouts = Some(saved);
    }

    /// What a click or a field asks for; page changes and the search are
    /// handled here.
    pub fn event(
        &mut self,
        ui: &mut Ui,
        node: NodeId,
        event: &Event,
        keymap: &Keymap,
    ) -> Option<Asked> {
        if let Some(change) = self.appearance.event(ui, node, event) {
            return Some(Asked::Theme(change));
        }
        match event {
            Event::Click { .. } => {
                if let Some((_, page)) = self.nav.iter().find(|(n, _)| *n == node) {
                    let page = *page;
                    self.set_page(ui, page);
                    return None;
                }
                if node == self.reset_all {
                    self.capturing = None;
                    return Some(Asked::ResetAllKeys);
                }
                if let Some(row) = self.rows.iter().find(|r| r.reset == node) {
                    self.capturing = None;
                    return Some(Asked::ResetKey(row.id));
                }
                if let Some(row) = self
                    .rows
                    .iter()
                    .find(|r| r.chip == node || under(ui, node, r.chip))
                {
                    let id = row.id;
                    // A second click on the listening key stops listening.
                    self.capturing = if self.capturing == Some(id) {
                        None
                    } else {
                        Some(id)
                    };
                    ui.focus(None);
                    self.show_keys(ui, keymap);
                    return self.capturing.map(Asked::Capture);
                }
                if node == self.open_last || under(ui, node, self.open_last) {
                    return Some(Asked::ToggleOpenLastScene);
                }
                for (_, apply, delete, name) in &self.layouts {
                    if node == *apply || under(ui, node, *apply) {
                        return Some(Asked::ApplyLayout(name.clone()));
                    }
                    if node == *delete || under(ui, node, *delete) {
                        return Some(Asked::DeleteLayout(name.clone()));
                    }
                }
                None
            }
            Event::Changed(text) if node == self.search => {
                let _ = text;
                self.show_keys(ui, keymap);
                None
            }
            Event::Changed(text) | Event::Submit(text) if node == self.editor => {
                Some(Asked::CodeEditor(text.trim().to_string()))
            }
            Event::Changed(text) | Event::Submit(text) if node == self.blender => {
                Some(Asked::Blender(text.trim().to_string()))
            }
            Event::Submit(text) if node == self.idle => {
                text.trim().parse().ok().map(Asked::IdleFps)
            }
            Event::Submit(text) => {
                let (_, _, _, old) = self.layouts.iter().find(|(f, ..)| *f == node)?;
                let new = text.trim().to_string();
                (new != *old && !new.is_empty()).then(|| Asked::RenameLayout(old.clone(), new))
            }
            _ => None,
        }
    }

    /// The layouts page must be built again (a layout saved, renamed or
    /// deleted).
    pub fn forget_layouts(&mut self) {
        self.shown_layouts = None;
    }
}

/// Whether `node` is under `root`.
fn under(ui: &Ui, node: NodeId, root: NodeId) -> bool {
    let mut at = ui.parent(node);
    while let Some(n) = at {
        if n == root {
            return true;
        }
        at = ui.parent(n);
    }
    false
}

/// A line of the pages list.
fn nav_row(ui: &mut Ui, parent: NodeId, glyph: &str, name: &str) -> NodeId {
    let row = ui.add(
        parent,
        Style::row()
            .full_width()
            .height(28.0)
            .fixed()
            .padding_x(SPACE_3)
            .gap(SPACE_2)
            .center_items()
            .radius(RADIUS_SM)
            .hover(HOVER),
    );
    icon(ui, row, glyph, MUTED);
    ui.add_text(row, text(), name);
    row
}

/// A page's title and, under it, room for a line saying where it is kept.
fn page_head(ui: &mut Ui, page: NodeId, title: &str) -> NodeId {
    let head = ui.add(
        page,
        Style::column()
            .full_width()
            .padding_x(SPACE_4)
            .padding_top(SPACE_4)
            .padding_bottom(SPACE_3)
            .gap(2.0),
    );
    ui.add_text(head, text().text_size(14.0).weight(600), title);
    head
}

/// The column of a page's labelled lines.
fn add_form(ui: &mut Ui, page: NodeId) -> NodeId {
    ui.add(
        page,
        Style::column()
            .full_width()
            .max_width(640.0)
            .padding_x(SPACE_4)
            .gap(SPACE_4),
    )
}

/// A line of a form: its name on the left, what is set on the right.
fn form_row(ui: &mut Ui, form: NodeId, label: &str) -> NodeId {
    let line = ui.add(form, Style::row().full_width().gap(SPACE_3).center_items());
    ui.add_text(line, text().text_color(LABEL).width(120.0).fixed(), label);
    ui.add(line, Style::row().fill().gap(SPACE_2).center_items())
}

/// A line of a form with a field and a note under it. Returns the field
/// and the note.
fn form_field(
    ui: &mut Ui,
    form: NodeId,
    label: &str,
    name: &str,
    hint: &str,
    note: &str,
) -> (NodeId, NodeId) {
    let block = ui.add(form, Style::column().full_width().gap(SPACE_1));
    let line = ui.add(block, Style::row().full_width().gap(SPACE_3).center_items());
    ui.add_text(line, text().text_color(LABEL).width(120.0).fixed(), label);
    let field = ui.add_field(line, field_style().fill(), "");
    ui.set_name(field, name);
    ui.set_placeholder(field, hint);
    let under = ui.add(block, Style::row().full_width().gap(SPACE_3));
    ui.add(under, Style::row().width(120.0).fixed());
    // Not `caption()`: a note is a sentence, and wraps.
    let note = ui.add_text(
        under,
        Style::default().text_size(11.0).text_color(MUTED).fill(),
        note,
    );
    (field, note)
}

/// A path under the home folder written from `~`.
fn tilde(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.and_then(|h| path.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_command_puts_the_place_in_and_the_file_last_otherwise() {
        let file = Path::new("/p/src/main.rs");
        assert_eq!(
            editor_command(DEFAULT_EDITOR, file, 12, 5),
            Some((
                "code".into(),
                vec!["-g".into(), "/p/src/main.rs:12:5".into()]
            ))
        );
        assert_eq!(
            editor_command("subl", file, 3, 1),
            Some(("subl".into(), vec!["/p/src/main.rs".into()]))
        );
        assert_eq!(
            editor_command(
                "\"/Applications/My Editor.app/bin/ed\" --line {line} {file}",
                file,
                7,
                1
            ),
            Some((
                "/Applications/My Editor.app/bin/ed".into(),
                vec!["--line".into(), "7".into(), "/p/src/main.rs".into()]
            ))
        );
        assert_eq!(editor_command("", file, 1, 1), None);
    }

    #[test]
    fn preferences_read_back_and_default_what_they_do_not_say() {
        let dir = std::env::temp_dir().join(format!("runity-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (none, error) = Personal::load(Some(&dir));
        assert_eq!(none, Personal::default());
        assert!(error.is_none());
        assert!(none.open_last_scene);
        let mut p = Personal {
            code_editor: "zed {file}:{line}".into(),
            ..Personal::default()
        };
        p.windows
            .insert("preferences".into(), [10.0, 20.0, 760.0, 540.0]);
        p.save(&dir).unwrap();
        let (back, error) = Personal::load(Some(&dir));
        assert!(error.is_none(), "{error:?}");
        assert_eq!(back, p);
        assert_eq!(back.editor(), "zed {file}:{line}");
        std::fs::write(dir.join(FILE), "(idle_fps: 10)").unwrap();
        let (partial, _) = Personal::load(Some(&dir));
        assert_eq!(partial.idle_fps, 10);
        assert!(partial.open_last_scene, "unsaid is the default");
        assert_eq!(partial.editor(), DEFAULT_EDITOR);
        let _ = std::fs::remove_dir_all(dir);
    }
}
