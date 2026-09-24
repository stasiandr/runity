//! Appearance: the editor's colours, as each person likes them.
//!
//! **Whose.** The choice is the person's, not the project's: it lives in
//! their own settings folder ([`config_dir`] — `~/Library/Application
//! Support/scrap/theme.ron` on macOS) and follows them into every
//! project, as Unity's skin does. It is a RON map, the same as a project's
//! `.scrap/theme.ron`, with the preset named:
//!
//! ```ron
//! {
//!     "preset": "graphite",
//!     "ACCENT": "#e07a5f",
//! }
//! ```
//!
//! **Which wins.** A project's `.scrap/theme.ron`, when there is one, lays
//! its tokens over the person's, token by token. `.scrap/` is not in git
//! (the project template ignores it), so that file is not a team imposing
//! a look on anyone: it is the same person's choice, narrower — one
//! checkout tinted red to tell it from another — and the narrower choice
//! wins, as a folder's settings win over a user's in a code editor. The
//! page says when a project file is in play, so it never looks as if the
//! choice did nothing.
//!
//! **Live.** Choosing recolours the open editor at once — `Ui::set_palette`
//! redraws the built tree, nothing is rebuilt — and writes the file. Both
//! files are read again twice a second, so an edit by hand, or another
//! editor window choosing, shows up here too (DNA, postulate 1).
//!
//! **Presets and one's own.** Five complete palettes (`theme::PRESETS`),
//! each readable as it comes; an accent from swatches made readable for
//! the preset, or typed; and under Advanced every token by hand. Choosing
//! a preset starts clean from it; «Reset to preset» drops one's own
//! colours and keeps the preset.
//!
//! Postulate 7 (one opinionated default) is bent here, and on purpose:
//! this is the editor's chrome, not the game or the engine's behaviour.
//! Nocturne stays the one default, nothing about a project depends on the
//! choice, and nothing outside the editor's window sees it.
//!
//! The previews on this page draw other palettes' colours, which the
//! palette must not recolour: it swaps by RGB, so a preview's colour that
//! happens to be one of Nocturne's is moved one step of blue ([`raw`]).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use scrap_ui::{Color, Event, NodeId, Style, Ui};

use crate::theme::*;

/// The file, in the settings folder.
pub const FILE: &str = "theme.ron";

/// Stands in for the person's settings folder when set: a test, or the
/// `shot` example, keeps its choices out of the real one.
pub const CONFIG_DIR_VAR: &str = "SCRAP_CONFIG_DIR";

/// Where the editor keeps what is the person's rather than a project's:
/// `~/Library/Application Support/scrap` on macOS, `%APPDATA%\scrap` on
/// Windows, `$XDG_CONFIG_HOME/scrap` or `~/.config/scrap` elsewhere — or
/// `SCRAP_CONFIG_DIR`. None with no home folder to find.
pub fn config_dir() -> Option<PathBuf> {
    let env = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = env(CONFIG_DIR_VAR) {
        return Some(dir);
    }
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    };
    base.map(|b| b.join("scrap"))
}

/// One theme file: the preset it names, if any, and the tokens it sets.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layer {
    pub preset: Option<String>,
    pub tokens: Tokens,
}

impl Layer {
    pub fn parse(text: &str) -> Result<Self, String> {
        let (preset, tokens) = crate::theme::parse(text)?;
        Ok(Self { preset, tokens })
    }

    /// As the file is written: the preset first, the tokens in the order
    /// the Advanced list shows them.
    pub fn text(&self) -> String {
        let mut out = String::from(
            "// The editor's colours, yours in every project: Preferences › Appearance.\n{\n",
        );
        if let Some(p) = &self.preset {
            out.push_str(&format!("    \"preset\": {p:?},\n"));
        }
        for (name, _) in TOKENS {
            if let Some(c) = self.tokens.get(name) {
                out.push_str(&format!("    {name:?}: {:?},\n", to_hex(*c)));
            }
        }
        out.push_str("}\n");
        out
    }

    fn read(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// What a person did on the page, or from the menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// A preset, clean: one's own colours are dropped.
    Preset(String),
    /// An accent; its ramp is made from it.
    Accent([u8; 3]),
    /// One token by hand, or back to what the preset says.
    Token(String, Option<[u8; 3]>),
    /// One's own colours dropped, the preset kept.
    Reset,
}

fn stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The person's choice and the project's file, as last read.
pub struct Theme {
    dir: Option<PathBuf>,
    pub user: Layer,
    pub project: Layer,
    /// When each file was last read (None inside: there was none); None
    /// outside: not read yet.
    user_seen: Option<Option<SystemTime>>,
    project_file: Option<PathBuf>,
    project_seen: Option<Option<SystemTime>>,
}

impl Theme {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            user: Layer::default(),
            project: Layer::default(),
            user_seen: None,
            project_file: None,
            project_seen: None,
        }
    }

    /// The person's file.
    pub fn user_file(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(FILE))
    }

    /// Keep the choice in another folder, read from there next.
    pub fn set_dir(&mut self, dir: PathBuf) {
        self.dir = Some(dir);
        self.user = Layer::default();
        self.user_seen = None;
    }

    /// Read what changed since the last time. True when the colours may
    /// have; a file that does not read is named with why, and the last
    /// good colours stay.
    pub fn poll(&mut self, project_root: Option<&Path>) -> (bool, Vec<String>) {
        let mut changed = false;
        let mut errors = Vec::new();
        if let Some(file) = self.user_file() {
            let now = stamp(&file);
            if self.user_seen != Some(now) {
                self.user_seen = Some(now);
                match Layer::read(&file) {
                    Ok(layer) => {
                        changed |= layer != self.user;
                        self.user = layer;
                    }
                    Err(e) => errors.push(format!("{}: {e}", file.display())),
                }
            }
        }
        let file = project_root.map(|r| r.join(".scrap").join(FILE));
        if file != self.project_file {
            self.project_file = file.clone();
            self.project_seen = None;
            if file.is_none() && self.project != Layer::default() {
                self.project = Layer::default();
                changed = true;
            }
        }
        if let Some(file) = file {
            let now = stamp(&file);
            if self.project_seen != Some(now) {
                self.project_seen = Some(now);
                match Layer::read(&file) {
                    Ok(layer) => {
                        changed |= layer != self.project;
                        self.project = layer;
                    }
                    Err(e) => errors.push(format!("{}: {e}", file.display())),
                }
            }
        }
        (changed, errors)
    }

    /// The preset in force: the project's if it names one, else the
    /// person's, else Nocturne.
    pub fn preset(&self) -> &'static Preset {
        self.project
            .preset
            .as_deref()
            .or(self.user.preset.as_deref())
            .and_then(preset)
            .unwrap_or(&PRESETS[0])
    }

    /// The preset the person chose, whatever the project says.
    pub fn chosen(&self) -> &'static Preset {
        self.user
            .preset
            .as_deref()
            .and_then(preset)
            .unwrap_or(&PRESETS[0])
    }

    /// Every token's colour: the preset, the person's, the project's.
    pub fn colours(&self) -> Tokens {
        resolve(self.preset(), &[&self.user.tokens, &self.project.tokens])
    }

    pub fn palette(&self) -> std::collections::HashMap<[u8; 3], [u8; 3]> {
        palette(&self.colours())
    }

    /// Make the change to the person's choice and write it down.
    pub fn change(&mut self, change: Change) -> Result<(), String> {
        let own_accent = self.chosen().get("ACCENT");
        let tokens = &mut self.user.tokens;
        match change {
            Change::Preset(name) => {
                if preset(&name).is_none() {
                    return Err(format!("no preset {name:?}"));
                }
                self.user.preset = Some(name);
                tokens.clear();
            }
            Change::Accent(c) => {
                for step in RAMP {
                    tokens.remove(step);
                }
                if c == own_accent {
                    tokens.remove("ACCENT");
                } else {
                    tokens.insert("ACCENT".into(), c);
                }
            }
            Change::Token(name, Some(c)) => {
                tokens.insert(name, c);
            }
            Change::Token(name, None) => {
                tokens.remove(&name);
            }
            Change::Reset => tokens.clear(),
        }
        self.save()
    }

    fn save(&mut self) -> Result<(), String> {
        let file = self
            .user_file()
            .ok_or("no settings folder to keep the colours in")?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&file, self.user.text()).map_err(|e| format!("{}: {e}", file.display()))?;
        self.user_seen = Some(stamp(&file));
        Ok(())
    }
}

/// The tokens as the Advanced list groups them, with what each is for.
pub const GROUPS: [(&str, &[(&str, &str)]); 5] = [
    (
        "Background",
        &[
            ("BG", "the ground between panels, fields"),
            ("SURFACE", "a panel"),
            ("NEUTRAL_900", "popups, wells"),
            ("NEUTRAL_800", "outlines, chips"),
        ],
    ),
    (
        "Text",
        &[
            ("TEXT", "text"),
            ("LABEL", "a field's name"),
            ("MUTED", "hints, captions"),
            ("NEUTRAL_300", "quiet text"),
            ("NEUTRAL_500", "quiet icons"),
        ],
    ),
    (
        "Accent",
        &[
            ("ACCENT", "lines, the focused thing"),
            ("ACCENT_100", "ink on a strong tint"),
            ("ACCENT_200", "the selection's text"),
            ("ACCENT_300", "accented text"),
            ("ACCENT_400", "marks, the git dot"),
            ("ACCENT_800", "a strong tint"),
            ("ACCENT_900", "the selection"),
        ],
    ),
    (
        "Status",
        &[
            ("WARNING", "careful"),
            ("ERROR", "this failed; the Inspector's X"),
            ("SUCCESS", "this worked; Y"),
            ("INFO", "for your information; Z"),
        ],
    ),
    (
        "Axes",
        &[
            ("AXIS_X", "the view's corner, X"),
            ("AXIS_Y", "Y"),
            ("AXIS_Z", "Z"),
        ],
    ),
];

/// Accents to choose from besides the preset's own: each shown, and
/// chosen, as the preset makes it readable.
const ACCENTS: [u32; 9] = [
    0x5f9fea, // blue
    0x4fb8a8, // teal
    0x7dbb6a, // green
    0xe0a84f, // amber
    0xe07a5f, // terracotta
    0xe0719a, // rose
    0xe05a5a, // red
    0xc77ddb, // orchid
    0x9184d9, // violet
];

/// A colour the palette leaves alone: one that is not a key of it.
pub fn raw(c: [u8; 3]) -> Color {
    let mut c = c;
    while TOKENS.iter().any(|(_, t)| [t.r, t.g, t.b] == c) {
        c[2] = if c[2] == 0 { 1 } else { c[2] - 1 };
    }
    Color::rgba(c[0], c[1], c[2], 255)
}

struct TokenRow {
    name: &'static str,
    swatch: NodeId,
    label: NodeId,
    field: NodeId,
    reset: NodeId,
}

/// The Appearance page of Preferences.
pub struct Appearance {
    pub root: NodeId,
    where_: NodeId,
    cards: Vec<(NodeId, &'static str)>,
    swatch_row: NodeId,
    swatches: Vec<(NodeId, [u8; 3])>,
    /// The preset the swatches were made readable for.
    swatches_for: Option<&'static str>,
    accent_field: NodeId,
    reset: NodeId,
    note: NodeId,
    fold: NodeId,
    fold_icon: NodeId,
    advanced: NodeId,
    open: bool,
    rows: Vec<TokenRow>,
}

impl Appearance {
    pub fn new(ui: &mut Ui, parent: NodeId) -> Self {
        let root = ui.add(
            parent,
            Style::column()
                .fill()
                .full_width()
                .padding(SPACE_4)
                .gap(SPACE_3)
                .clip(),
        );
        ui.set_name(root, "appearance");
        let head = ui.add(root, Style::column().full_width().gap(2.0));
        ui.add_text(head, text().text_size(14.0).weight(600), "Appearance");
        let where_ = ui.add_text(head, caption().text_size(11.0), "");
        ui.set_name(where_, "appearance where");

        section(ui, root, "Theme");
        let cards_row = ui.add(root, Style::row().full_width().gap(SPACE_3).wrap());
        let cards = PRESETS
            .iter()
            .map(|p| (preset_card(ui, cards_row, p), p.name))
            .collect();

        section(ui, root, "Accent");
        let accent_row = ui.add(root, Style::row().full_width().gap(SPACE_3).center_items());
        let swatch_row = ui.add(accent_row, Style::row().gap(SPACE_2).center_items());
        let accent_field = ui.add_field(accent_row, field_style().width(84.0).fixed().mono(), "");
        ui.set_name(accent_field, "accent hex");
        spacer(ui, accent_row);
        let reset = button(ui, accent_row, "appearance reset", "Reset to preset", false);

        let note = ui.add_text(
            root,
            text().text_size(11.5).text_color(WARNING).hidden(),
            "",
        );
        ui.set_name(note, "appearance note");

        let fold = ui.add(
            root,
            Style::row()
                .full_width()
                .height(24.0)
                .fixed()
                .gap(SPACE_1)
                .center_items()
                .radius(RADIUS_SM)
                .hover(HOVER),
        );
        ui.set_name(fold, "appearance advanced");
        let fold_icon = icon(ui, fold, "chevron-right", MUTED);
        ui.add_text(fold, caption(), "ADVANCED — EVERY COLOUR");
        let advanced = ui.add(
            root,
            Style::column()
                .full_width()
                .max_width(640.0)
                .gap(SPACE_2)
                .padding_left(SPACE_4)
                .hidden(),
        );
        let mut rows = Vec::new();
        for (group, tokens) in GROUPS {
            ui.add_text(advanced, caption().text_color(LABEL), &group.to_uppercase());
            for (name, role) in tokens {
                rows.push(token_row(ui, advanced, name, role));
            }
        }
        Self {
            root,
            where_,
            cards,
            swatch_row,
            swatches: Vec::new(),
            swatches_for: None,
            accent_field,
            reset,
            note,
            fold,
            fold_icon,
            advanced,
            open: false,
            rows,
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

    /// Bring the page up to date with the theme: which preset and accent
    /// are chosen, every token's colour, what the project's file sets.
    pub fn show(&mut self, ui: &mut Ui, theme: &Theme) {
        let chosen = theme.chosen();
        let colours = theme.colours();
        let surface = colours["SURFACE"];
        match theme.user_file() {
            Some(file) => ui.set_text(
                self.where_,
                &format!("Yours, in every project — kept in {}", tilde(&file)),
            ),
            None => ui.set_text(
                self.where_,
                "No settings folder: the choice lasts until you quit.",
            ),
        }
        for (card, name) in &self.cards {
            let on = *name == chosen.name;
            ui.restyle(*card, |s| {
                if on {
                    s.border(1.5, ACCENT).background(ACCENT_HOVER)
                } else {
                    s.border(1.0, DIVIDER).background(Color::TRANSPARENT)
                }
            });
        }
        if self.swatches_for != Some(chosen.name) {
            self.swatches_for = Some(chosen.name);
            ui.clear(self.swatch_row);
            self.swatches.clear();
            let own = chosen.get("ACCENT");
            let others = ACCENTS
                .iter()
                .map(|n| readable_accent([(n >> 16) as u8, (n >> 8) as u8, *n as u8], surface))
                .filter(|c| *c != own);
            for (i, c) in std::iter::once(own).chain(others).enumerate() {
                let s = ui.add(
                    self.swatch_row,
                    Style::row()
                        .size(22.0, 22.0)
                        .fixed()
                        .center()
                        .radius(11.0)
                        .border(2.0, Color::TRANSPARENT)
                        .hover_border(TEXT.alpha(40))
                        .clickable(),
                );
                ui.set_name(s, format!("accent {i}"));
                ui.add(
                    s,
                    Style::row()
                        .size(14.0, 14.0)
                        .fixed()
                        .radius(7.0)
                        .background(raw(c)),
                );
                self.swatches.push((s, c));
            }
        }
        let accent = colours["ACCENT"];
        for (s, c) in &self.swatches {
            let on = *c == accent;
            ui.restyle(*s, |st| {
                st.border(2.0, if on { TEXT } else { Color::TRANSPARENT })
            });
        }
        if ui.focused() != Some(self.accent_field) {
            ui.set_text(self.accent_field, &to_hex(accent));
        }
        let own = !theme.user.tokens.is_empty();
        ui.restyle(self.reset, |s| s.opacity(if own { 1.0 } else { 0.45 }));
        let mut set: Vec<&str> = theme.project.tokens.keys().map(|k| k.as_str()).collect();
        if let Some(p) = &theme.project.preset {
            set.insert(0, p);
        }
        if set.is_empty() {
            ui.restyle(self.note, |s| s.hidden());
        } else {
            ui.set_text(
                self.note,
                &format!(
                    "This project's .scrap/theme.ron sets {} over your choice.",
                    set.join(", ")
                ),
            );
            ui.restyle(self.note, |s| s.shown());
        }
        for row in &self.rows {
            let c = colours[row.name];
            let mine = theme.user.tokens.contains_key(row.name);
            ui.restyle(row.swatch, |s| s.background(raw(c)));
            ui.restyle(row.label, |s| {
                s.text_color(if mine { ACCENT_300 } else { LABEL })
            });
            ui.restyle(row.reset, |s| if mine { s.shown() } else { s.hidden() });
            if ui.focused() != Some(row.field) {
                ui.set_text(row.field, &to_hex(c));
            }
        }
    }

    /// What a click or a typed colour on the page asks for.
    pub fn event(&mut self, ui: &mut Ui, node: NodeId, event: &Event) -> Option<Change> {
        match event {
            Event::Click { .. } => {
                if node == self.fold {
                    self.open = !self.open;
                    let open = self.open;
                    ui.restyle(self.advanced, |s| if open { s.shown() } else { s.hidden() });
                    ui.set_icon(
                        self.fold_icon,
                        if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                    );
                    return None;
                }
                if node == self.reset {
                    return Some(Change::Reset);
                }
                if let Some((_, name)) = self.cards.iter().find(|(n, _)| *n == node) {
                    return Some(Change::Preset(name.to_string()));
                }
                if let Some((_, c)) = self.swatches.iter().find(|(n, _)| *n == node) {
                    return Some(Change::Accent(*c));
                }
                if let Some(row) = self.rows.iter().find(|r| r.reset == node) {
                    return Some(Change::Token(row.name.into(), None));
                }
                None
            }
            Event::Submit(text) => {
                let c = parse_hex(text);
                if node == self.accent_field {
                    return c.map(Change::Accent);
                }
                let row = self.rows.iter().find(|r| r.field == node)?;
                c.map(|c| Change::Token(row.name.into(), Some(c)))
            }
            _ => None,
        }
    }
}

/// A section's caption.
fn section(ui: &mut Ui, parent: NodeId, title: &str) {
    ui.add_text(
        parent,
        caption().text_color(LABEL).margin(0.0),
        &title.to_uppercase(),
    );
}

/// A preset as a card: a small editor in its colours — ground, a panel, a
/// line of text, a muted one, a selected row, a button, the status dots —
/// and its name under it.
fn preset_card(ui: &mut Ui, parent: NodeId, p: &Preset) -> NodeId {
    let c = |t: &str| raw(p.get(t));
    let card = ui.add(
        parent,
        Style::column()
            .width(142.0)
            .fixed()
            .padding(5.0)
            .gap(5.0)
            .radius(RADIUS_MD)
            .border(1.0, DIVIDER)
            .hover(HOVER),
    );
    ui.set_name(card, format!("theme {}", p.name));
    let ground = ui.add(
        card,
        Style::row()
            .full_width()
            .height(82.0)
            .fixed()
            .padding(5.0)
            .gap(4.0)
            .radius(6.0)
            .background(c("BG")),
    );
    // A narrow panel as the Hierarchy, a wide one as the Inspector.
    let left = ui.add(
        ground,
        Style::column()
            .width(40.0)
            .full_height()
            .fixed()
            .padding(4.0)
            .gap(3.0)
            .radius(4.0)
            .background(c("SURFACE")),
    );
    let line = |ui: &mut Ui, parent: NodeId, width: f32, color: Color| {
        ui.add(
            parent,
            Style::row()
                .width(width)
                .height(3.0)
                .fixed()
                .radius(1.5)
                .background(color),
        )
    };
    line(ui, left, 22.0, c("MUTED").alpha(55));
    line(ui, left, 30.0, c("TEXT"));
    let selected = ui.add(
        left,
        Style::row()
            .full_width()
            .height(9.0)
            .fixed()
            .padding_x(3.0)
            .center_items()
            .radius(2.0)
            .background(c("ACCENT_900")),
    );
    line(ui, selected, 24.0, c("ACCENT_200"));
    line(ui, left, 26.0, c("TEXT"));
    line(ui, left, 18.0, c("LABEL").alpha(70));
    let right = ui.add(
        ground,
        Style::column()
            .fill()
            .full_height()
            .padding(5.0)
            .gap(4.0)
            .radius(4.0)
            .background(c("SURFACE")),
    );
    ui.add_text(
        right,
        Style::default()
            .text_size(12.0)
            .weight(600)
            .text_color(c("TEXT"))
            .nowrap(),
        "Aa",
    );
    ui.add_text(
        right,
        Style::default()
            .text_size(9.0)
            .text_color(c("MUTED").alpha(55))
            .nowrap(),
        "muted",
    );
    let chip = ui.add(
        right,
        Style::row()
            .height(12.0)
            .fixed()
            .padding_x(5.0)
            .center()
            .radius(4.0)
            .border(1.0, c("ACCENT")),
    );
    ui.add_text(
        chip,
        Style::default()
            .text_size(8.0)
            .text_color(c("ACCENT"))
            .nowrap(),
        "Play",
    );
    let dots = ui.add(right, Style::row().gap(3.0));
    for t in ["ACCENT", "SUCCESS", "WARNING", "ERROR", "INFO"] {
        ui.add(
            dots,
            Style::row()
                .size(7.0, 7.0)
                .fixed()
                .radius(3.5)
                .background(c(t)),
        );
    }
    let names = ui.add(card, Style::column().full_width().padding_x(2.0).gap(1.0));
    ui.add_text(names, text().text_size(12.0), p.label);
    ui.add_text(names, caption(), p.blurb);
    card
}

fn token_row(ui: &mut Ui, parent: NodeId, name: &'static str, role: &str) -> TokenRow {
    let row = ui.add(
        parent,
        Style::row()
            .full_width()
            .height(24.0)
            .fixed()
            .gap(SPACE_2)
            .center_items(),
    );
    let swatch = ui.add(
        row,
        Style::row()
            .size(16.0, 16.0)
            .fixed()
            .radius(RADIUS_SM)
            .border(1.0, DIVIDER),
    );
    let label = ui.add_text(
        row,
        text().mono().text_size(11.0).width(104.0).fixed(),
        name,
    );
    ui.add_text(row, caption().text_size(11.0).fill(), role);
    let field = ui.add_field(row, field_style().width(84.0).fixed().mono(), "");
    ui.set_name(field, format!("token {name}"));
    let reset = ui.add(
        row,
        Style::row()
            .size(20.0, 20.0)
            .fixed()
            .center()
            .radius(RADIUS_SM)
            .hover(HOVER),
    );
    ui.set_name(reset, format!("token reset {name}"));
    icon(ui, reset, "x", MUTED);
    TokenRow {
        name,
        swatch,
        label,
        field,
        reset,
    }
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

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scrap-appearance-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_groups_hold_every_token_once() {
        for (name, _) in TOKENS {
            let n = GROUPS
                .iter()
                .flat_map(|(_, t)| t.iter())
                .filter(|(t, _)| *t == name)
                .count();
            assert_eq!(n, 1, "{name} is listed {n} times");
        }
    }

    #[test]
    fn the_choice_round_trips_through_its_file() {
        let dir = temp("round-trip");
        let mut theme = Theme::new(Some(dir.clone()));
        theme.poll(None);
        theme.change(Change::Preset("graphite".into())).unwrap();
        theme.change(Change::Accent([0xe0, 0x7a, 0x5f])).unwrap();
        theme
            .change(Change::Token("WARNING".into(), Some([1, 2, 3])))
            .unwrap();

        let mut again = Theme::new(Some(dir.clone()));
        let (changed, errors) = again.poll(None);
        assert!(changed && errors.is_empty(), "{errors:?}");
        assert_eq!(again.user, theme.user);
        assert_eq!(again.preset().name, "graphite");
        assert_eq!(again.colours()["WARNING"], [1, 2, 3]);
        assert_eq!(Layer::parse(&theme.user.text()).unwrap(), theme.user);

        // Choosing the preset's own accent is having none of one's own.
        again.change(Change::Token("WARNING".into(), None)).unwrap();
        again
            .change(Change::Accent(preset("graphite").unwrap().get("ACCENT")))
            .unwrap();
        assert!(again.user.tokens.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The project's file is laid over the person's, token by token, and
    /// names the preset if it names one.
    #[test]
    fn the_project_file_wins_token_by_token() {
        let dir = temp("precedence");
        let project = dir.join("project");
        std::fs::create_dir_all(project.join(".scrap")).unwrap();
        let mut theme = Theme::new(Some(dir.join("me")));
        theme.poll(Some(&project));
        theme.change(Change::Preset("daylight".into())).unwrap();
        theme
            .change(Change::Token("ACCENT".into(), Some([0x11, 0x22, 0x33])))
            .unwrap();
        theme
            .change(Change::Token("ERROR".into(), Some([0x44, 0, 0])))
            .unwrap();
        std::fs::write(
            project.join(".scrap").join(FILE),
            r##"{"ACCENT": "#e07a5f"}"##,
        )
        .unwrap();
        let (changed, _) = theme.poll(Some(&project));
        assert!(changed);
        let c = theme.colours();
        assert_eq!(c["ACCENT"], [0xe0, 0x7a, 0x5f], "the project's accent");
        assert_eq!(c["ERROR"], [0x44, 0, 0], "the person's error");
        assert_eq!(c["BG"], preset("daylight").unwrap().get("BG"));

        std::fs::write(
            project.join(".scrap").join(FILE),
            r##"{"preset": "ember"}"##,
        )
        .unwrap();
        // A second write within the file system's clock step would look
        // unchanged: read it as new.
        theme.project_seen = None;
        theme.poll(Some(&project));
        assert_eq!(theme.preset().name, "ember");
        assert_eq!(theme.chosen().name, "daylight");

        // Another project, without a file: the person's alone again.
        theme.poll(Some(&dir));
        assert_eq!(theme.preset().name, "daylight");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn raw_colours_are_not_the_palettes_keys() {
        for (_, t) in TOKENS {
            let r = raw([t.r, t.g, t.b]);
            assert_ne!([r.r, r.g, r.b], [t.r, t.g, t.b]);
        }
        let free = raw([1, 2, 3]);
        assert_eq!([free.r, free.g, free.b], [1, 2, 3]);
    }
}
