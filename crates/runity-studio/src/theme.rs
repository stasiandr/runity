//! Nocturne: the editor's look, as tokens, and its components as
//! `runity-ui` subtrees.
//!
//! The design system lives in Claude Design (project «Nocturne»); its
//! `styles.css` is the source of truth and the constants here are copied
//! from it, name for name — `--color-accent-800` is [`ACCENT_800`]. A quiet,
//! compact dark interface: a near-neutral blue-grey ground, Inter, 8px
//! radii, and an accent used as a line and a glow rather than a flood.
//!
//! Two colours Nocturne does not have and an editor needs are added and
//! marked: a warning and an error (Nocturne is a mono scheme, a Console
//! cannot be), kept at the ramps' low chroma.
//!
//! Icons are Lucide, shipped by `runity-ui`. Nocturne names Phosphor; the
//! two share a 24-unit grid and stroke, and swapping sets is swapping files.

use std::collections::HashMap;

use runity_ui::{Color, NodeId, Style, Ui};

// --- colour roles -----------------------------------------------------

/// `--color-bg`: the ground behind every panel.
pub const BG: Color = Color::hex(0x161826);
/// `--color-surface`: a panel.
pub const SURFACE: Color = Color::hex(0x232532);
/// `--color-text`.
pub const TEXT: Color = Color::hex(0xe9e9ed);
/// `--color-accent`: lines, marks, the focused thing. Never a flood.
pub const ACCENT: Color = Color::hex(0x9184d9);

pub const NEUTRAL_300: Color = Color::hex(0xcfd3e5);
pub const NEUTRAL_500: Color = Color::hex(0x9397ab);
pub const NEUTRAL_800: Color = Color::hex(0x3f424d);
pub const NEUTRAL_900: Color = Color::hex(0x292b31);

pub const ACCENT_100: Color = Color::hex(0xf5f4ff);
pub const ACCENT_200: Color = Color::hex(0xe7e5fe);
pub const ACCENT_300: Color = Color::hex(0xd2cefd);
pub const ACCENT_400: Color = Color::hex(0xb5abfc);
pub const ACCENT_800: Color = Color::hex(0x423a6a);
pub const ACCENT_900: Color = Color::hex(0x2b2741);

/// Not Nocturne's: a Console needs to say «careful».
pub const WARNING: Color = Color::hex(0xd9bc84);
/// Not Nocturne's: a Console needs to say «this failed».
pub const ERROR: Color = Color::hex(0xe58a96);

/// `--color-divider`: the text at 16%.
pub const DIVIDER: Color = TEXT.alpha(16);
/// `.text-muted`: the text at 55%.
pub const MUTED: Color = TEXT.alpha(55);
/// `.field > label`: the text at 70%.
pub const LABEL: Color = TEXT.alpha(70);
/// A secondary control's hover and pressed tints.
pub const HOVER: Color = TEXT.alpha(7);
pub const PRESSED: Color = TEXT.alpha(14);
/// The outlined primary button's hover.
pub const ACCENT_HOVER: Color = ACCENT.alpha(12);

/// The colour tokens a theme file can set, by name. The text at an alpha
/// (`MUTED`, `DIVIDER`, …) follows `TEXT`, and the accent's tints follow
/// `ACCENT`, because a palette swaps colours by RGB and keeps the alpha.
pub const TOKENS: [(&str, Color); 16] = [
    ("BG", BG),
    ("SURFACE", SURFACE),
    ("TEXT", TEXT),
    ("ACCENT", ACCENT),
    ("NEUTRAL_300", NEUTRAL_300),
    ("NEUTRAL_500", NEUTRAL_500),
    ("NEUTRAL_800", NEUTRAL_800),
    ("NEUTRAL_900", NEUTRAL_900),
    ("ACCENT_100", ACCENT_100),
    ("ACCENT_200", ACCENT_200),
    ("ACCENT_300", ACCENT_300),
    ("ACCENT_400", ACCENT_400),
    ("ACCENT_800", ACCENT_800),
    ("ACCENT_900", ACCENT_900),
    ("WARNING", WARNING),
    ("ERROR", ERROR),
];

/// A theme file's text — a RON map from token name to `#rrggbb`,
/// `{"ACCENT": "#e07a5f", "BG": "#101014"}` — as the palette that draws
/// Nocturne's colours as the file's. Tokens it leaves out stay Nocturne's.
pub fn palette(text: &str) -> Result<HashMap<[u8; 3], [u8; 3]>, String> {
    let map: std::collections::BTreeMap<String, String> =
        runity::ron::from_str(text).map_err(|e| e.to_string())?;
    let mut palette = HashMap::new();
    for (name, value) in map {
        let Some((_, token)) = TOKENS.iter().find(|(n, _)| *n == name) else {
            let names: Vec<&str> = TOKENS.iter().map(|(n, _)| *n).collect();
            return Err(format!("no token {name:?}; there are {}", names.join(", ")));
        };
        let hex = value.trim().trim_start_matches('#');
        let n = (hex.len() == 6)
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten()
            .ok_or_else(|| format!("{name}: {value:?} is not #rrggbb"))?;
        palette.insert(
            [token.r, token.g, token.b],
            [(n >> 16) as u8, (n >> 8) as u8, n as u8],
        );
    }
    Ok(palette)
}

// --- space and radius -------------------------------------------------

/// `--space-*`: the compact scale, density 0.7× baked in.
pub const SPACE_1: f32 = 2.8;
pub const SPACE_2: f32 = 5.6;
pub const SPACE_3: f32 = 8.4;
pub const SPACE_4: f32 = 11.2;
pub const SPACE_6: f32 = 16.8;

pub const RADIUS_SM: f32 = 4.0;
pub const RADIUS_MD: f32 = 8.0;
pub const RADIUS_LG: f32 = 14.0;

/// Body text.
pub fn text() -> Style {
    Style::default().text_size(12.5).text_color(TEXT).nowrap()
}

/// `h6`: a panel's title — small, spaced, muted.
pub fn caption() -> Style {
    Style::default().text_size(10.5).text_color(MUTED).nowrap()
}

// --- components -------------------------------------------------------

/// A panel: Nocturne's `.card` — the surface on the ground, 8px round,
/// with its title as an `h6`. Returns the card, its header row (for
/// buttons at the right end) and its body.
pub fn panel(ui: &mut Ui, parent: NodeId, title: &str) -> (NodeId, NodeId, NodeId) {
    let card = ui.add(
        parent,
        Style::column()
            .full()
            .background(SURFACE)
            .radius(RADIUS_MD)
            .clip(),
    );
    let header = ui.add(
        card,
        Style::row()
            .height(30.0)
            .fixed()
            .full_width()
            .padding_left(SPACE_4)
            .padding_x(SPACE_2)
            .padding_left(SPACE_4)
            .gap(SPACE_2)
            .center_items(),
    );
    let t = ui.add_text(header, caption().fill(), &title.to_uppercase());
    ui.set_name(t, format!("{title} title"));
    let body = ui.add(card, Style::column().fill().full_width());
    (card, header, body)
}

/// An icon at the interface size.
pub fn icon(ui: &mut Ui, parent: NodeId, name: &str, color: Color) -> NodeId {
    ui.add_icon(
        parent,
        Style::default().size(14.0, 14.0).fixed().text_color(color),
        name,
    )
}

fn icon_button_style(on: bool) -> Style {
    let base = Style::row()
        .size(26.0, 26.0)
        .fixed()
        .center()
        .radius(RADIUS_MD);
    if on {
        base.border(1.0, ACCENT)
            .background(ACCENT_HOVER)
            .hover(ACCENT.alpha(18))
    } else {
        base.border(1.0, Color::TRANSPARENT)
            .hover(HOVER)
            .pressed(PRESSED)
    }
}

/// `.btn-icon` as a ghost: transparent until hovered. `on` draws it as the
/// thing that is chosen — the accent as a line and a tint, never a fill.
pub fn icon_button(ui: &mut Ui, parent: NodeId, name: &str, icon_name: &str, on: bool) -> NodeId {
    let b = ui.add(parent, icon_button_style(on));
    ui.set_name(b, name);
    icon(ui, b, icon_name, if on { ACCENT } else { LABEL });
    b
}

/// Change an icon button's state in place.
pub fn set_icon_button(ui: &mut Ui, button: NodeId, icon_name: &str, on: bool, enabled: bool) {
    ui.set_style(
        button,
        icon_button_style(on).opacity(if enabled { 1.0 } else { 0.45 }),
    );
    if let Some(glyph) = ui.children(button).first().copied() {
        ui.set_icon(glyph, icon_name);
        ui.restyle(glyph, |s| s.text_color(if on { ACCENT } else { LABEL }));
    }
}

fn button_style(primary: bool) -> Style {
    let base = Style::row()
        .height(26.0)
        .fixed()
        .padding_x(SPACE_3)
        .gap(6.0)
        .center()
        .radius(RADIUS_MD);
    if primary {
        base.border(1.0, ACCENT)
            .hover(ACCENT_HOVER)
            .pressed(ACCENT.alpha(22))
    } else {
        base.border(1.0, DIVIDER).hover(HOVER).pressed(PRESSED)
    }
}

/// `.btn`: `.btn-primary` (an accent outline) or `.btn-secondary` (a
/// divider outline) — never a fill.
pub fn button(ui: &mut Ui, parent: NodeId, name: &str, label: &str, primary: bool) -> NodeId {
    let b = ui.add(parent, button_style(primary));
    ui.set_name(b, name);
    ui.add_text(
        b,
        text().text_color(if primary { ACCENT } else { TEXT }),
        label,
    );
    b
}

pub fn set_button_primary(ui: &mut Ui, button: NodeId, primary: bool) {
    ui.set_style(button, button_style(primary));
    for child in ui.children(button) {
        ui.restyle(child, |s| s.text_color(if primary { ACCENT } else { TEXT }));
    }
}

/// `.tag`: a small label tinted from a ramp.
pub fn tag(ui: &mut Ui, parent: NodeId, label: &str, fill: Color, ink: Color) -> NodeId {
    let t = ui.add(
        parent,
        Style::row()
            .height(18.0)
            .fixed()
            .padding_x(7.0)
            .center()
            .radius(6.0)
            .background(fill),
    );
    ui.add_text(
        t,
        Style::default().text_size(10.5).text_color(ink).nowrap(),
        label,
    );
    t
}

/// `.input`: a text field on the ground inside a panel, divider-outlined,
/// accent-outlined when hovered.
pub fn field_style() -> Style {
    Style::row()
        .height(22.0)
        .fixed()
        .padding_x(6.0)
        .center_items()
        .radius(6.0)
        .background(BG)
        .border(1.0, DIVIDER)
        .hover_border(TEXT.alpha(35))
        .text_size(12.0)
        .text_color(TEXT)
}

/// A vertical hairline between groups of a toolbar.
pub fn separator(ui: &mut Ui, parent: NodeId) -> NodeId {
    ui.add(
        parent,
        Style::row()
            .size(1.0, 16.0)
            .fixed()
            .margin(SPACE_1)
            .background(DIVIDER),
    )
}

/// Flexible space in a row.
pub fn spacer(ui: &mut Ui, parent: NodeId) -> NodeId {
    ui.add(parent, Style::row().fill())
}
