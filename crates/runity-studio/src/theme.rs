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
//! cannot be), kept at the ramps' low chroma — and a success, an info and
//! the three axes, so that every colour the chrome draws is a token.
//!
//! The code draws with Nocturne's constants; any other look is a palette
//! over them ([`PRESETS`], [`resolve`], [`palette`]): the tree built once,
//! drawn in the colours each person chose (`crate::appearance`).
//!
//! Icons are Lucide, shipped by `runity-ui`. Nocturne names Phosphor; the
//! two share a 24-unit grid and stroke, and swapping sets is swapping files.

use std::collections::{BTreeMap, HashMap};

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
/// Not Nocturne's: «this worked» and «for your information». The
/// Inspector's X, Y and Z are [`ERROR`] and these two, by value, so a
/// theme that sets the status colours sets those letters too.
pub const SUCCESS: Color = Color::hex(0x9fd49a);
pub const INFO: Color = Color::hex(0x8fb4e8);
/// The axes as the view's corner and the Profiler draw them.
pub const AXIS_X: Color = Color::hex(0xe5736f);
pub const AXIS_Y: Color = Color::hex(0x8cc26b);
pub const AXIS_Z: Color = Color::hex(0x6f9be5);

/// `--color-divider`: the text at 16%.
pub const DIVIDER: Color = TEXT.alpha(16);
/// `.text-muted`: the text at 55%. A key of its own — one step of blue off
/// [`TEXT`], which no eye tells apart — so that a theme can set it apart
/// from the text: on white, the text at 55% is never darker than mid-grey,
/// and a light theme needs muted text darker than that to be read.
pub const MUTED: Color = Color::hex(0xe9e9ee).alpha(55);
/// `.field > label`: the text at 70%, a key of its own as [`MUTED`] is.
pub const LABEL: Color = Color::hex(0xe9e9ec).alpha(70);
/// A secondary control's hover and pressed tints.
pub const HOVER: Color = TEXT.alpha(7);
pub const PRESSED: Color = TEXT.alpha(14);
/// The outlined primary button's hover.
pub const ACCENT_HOVER: Color = ACCENT.alpha(12);

/// The colour tokens a theme can set, by name. A palette swaps colours by
/// RGB and keeps the alpha, so the text at an alpha (`DIVIDER`, `HOVER`)
/// follows `TEXT` and the accent's tints follow `ACCENT`. Every token has
/// an RGB of its own, or the palette could not tell two apart (tested).
pub const TOKENS: [(&str, Color); 23] = [
    ("BG", BG),
    ("SURFACE", SURFACE),
    ("TEXT", TEXT),
    ("LABEL", LABEL),
    ("MUTED", MUTED),
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
    ("SUCCESS", SUCCESS),
    ("INFO", INFO),
    ("AXIS_X", AXIS_X),
    ("AXIS_Y", AXIS_Y),
    ("AXIS_Z", AXIS_Z),
];

/// Colours by token name, as a theme resolves them.
pub type Tokens = BTreeMap<String, [u8; 3]>;

/// A complete palette: every token, chosen together.
#[derive(Debug)]
pub struct Preset {
    /// What files and menus call it.
    pub name: &'static str,
    pub label: &'static str,
    /// A line under its card.
    pub blurb: &'static str,
    pub colors: &'static [(&'static str, u32)],
}

impl Preset {
    pub fn get(&self, token: &str) -> [u8; 3] {
        let (_, n) = self
            .colors
            .iter()
            .find(|(t, _)| *t == token)
            .unwrap_or_else(|| panic!("{} has no {token}", self.name));
        rgb(*n)
    }
}

/// The palettes to choose from, Nocturne first: it is the default. Each
/// holds every token, and its text, labels and muted text at 4.5:1 or more
/// on its ground and its panels, as WCAG asks of text ([`INK_ON_GROUND`],
/// tested).
pub const PRESETS: [Preset; 5] = [
    Preset {
        name: "nocturne",
        label: "Nocturne",
        blurb: "Blue-grey, violet",
        colors: &[
            ("BG", 0x161826),
            ("SURFACE", 0x232532),
            ("TEXT", 0xe9e9ed),
            ("LABEL", 0xe9e9ec),
            ("MUTED", 0xe9e9ee),
            ("ACCENT", 0x9184d9),
            ("NEUTRAL_300", 0xcfd3e5),
            ("NEUTRAL_500", 0x9397ab),
            ("NEUTRAL_800", 0x3f424d),
            ("NEUTRAL_900", 0x292b31),
            ("ACCENT_100", 0xf5f4ff),
            ("ACCENT_200", 0xe7e5fe),
            ("ACCENT_300", 0xd2cefd),
            ("ACCENT_400", 0xb5abfc),
            ("ACCENT_800", 0x423a6a),
            ("ACCENT_900", 0x2b2741),
            ("WARNING", 0xd9bc84),
            ("ERROR", 0xe58a96),
            ("SUCCESS", 0x9fd49a),
            ("INFO", 0x8fb4e8),
            ("AXIS_X", 0xe5736f),
            ("AXIS_Y", 0x8cc26b),
            ("AXIS_Z", 0x6f9be5),
        ],
    },
    Preset {
        name: "graphite",
        label: "Graphite",
        blurb: "Unity-like greys, blue",
        colors: &[
            ("BG", 0x191919),
            ("SURFACE", 0x2e2e2e),
            ("TEXT", 0xdcdcdc),
            ("LABEL", 0xe8e8e8),
            ("MUTED", 0xf4f4f4),
            ("ACCENT", 0x5f9fea),
            ("NEUTRAL_300", 0xd0d0d0),
            ("NEUTRAL_500", 0x8f8f8f),
            ("NEUTRAL_800", 0x474747),
            ("NEUTRAL_900", 0x262626),
            ("ACCENT_100", 0xf2f7ff),
            ("ACCENT_200", 0xe3efff),
            ("ACCENT_300", 0xc6dcf7),
            ("ACCENT_400", 0x8dbcf2),
            ("ACCENT_800", 0x234a6e),
            ("ACCENT_900", 0x2c5d87),
            ("WARNING", 0xe8c05a),
            ("ERROR", 0xf07878),
            ("SUCCESS", 0x8ccf7e),
            ("INFO", 0x7fb2f0),
            ("AXIS_X", 0xef6f5c),
            ("AXIS_Y", 0x88c540),
            ("AXIS_Z", 0x649ff2),
        ],
    },
    Preset {
        name: "daylight",
        label: "Daylight",
        blurb: "Light panels, indigo",
        colors: &[
            ("BG", 0xe8e9ed),
            ("SURFACE", 0xf8f8fa),
            ("TEXT", 0x1c1d24),
            ("LABEL", 0x23252f),
            // At 55% over white nothing is darker than mid-grey: black is
            // what reads.
            ("MUTED", 0x000000),
            ("ACCENT", 0x5b4fc4),
            ("NEUTRAL_300", 0x3b3e4a),
            ("NEUTRAL_500", 0x7c8091),
            ("NEUTRAL_800", 0xd3d5de),
            ("NEUTRAL_900", 0xeef0f3),
            // The ramp keeps its roles, not its lightness: the low steps
            // are ink, the high ones the tints behind it — dark ink and
            // pale tints here.
            ("ACCENT_100", 0x1f1850),
            ("ACCENT_200", 0x2d2475),
            ("ACCENT_300", 0x4a3fb0),
            ("ACCENT_400", 0x7b70e0),
            ("ACCENT_800", 0xcdc7f4),
            ("ACCENT_900", 0xe4e1fa),
            ("WARNING", 0x8a5a00),
            ("ERROR", 0xb3261e),
            ("SUCCESS", 0x2e7d32),
            ("INFO", 0x1f5fbf),
            ("AXIS_X", 0xd0342c),
            ("AXIS_Y", 0x2f7d1f),
            ("AXIS_Z", 0x2f6fd6),
        ],
    },
    Preset {
        name: "contrast",
        label: "High Contrast",
        blurb: "Black, white, yellow",
        colors: &[
            ("BG", 0x000000),
            ("SURFACE", 0x101010),
            ("TEXT", 0xffffff),
            ("LABEL", 0xffffff),
            ("MUTED", 0xffffff),
            ("ACCENT", 0xffd23f),
            ("NEUTRAL_300", 0xf0f0f0),
            ("NEUTRAL_500", 0xb0b0b0),
            ("NEUTRAL_800", 0x6b6b6b),
            ("NEUTRAL_900", 0x1a1a1a),
            ("ACCENT_100", 0xfff9e0),
            ("ACCENT_200", 0xfff1b8),
            ("ACCENT_300", 0xffe27a),
            ("ACCENT_400", 0xffdc5c),
            ("ACCENT_800", 0x5c4a00),
            ("ACCENT_900", 0x3d3200),
            ("WARNING", 0xffb000),
            ("ERROR", 0xff6b6b),
            ("SUCCESS", 0x5ef38c),
            ("INFO", 0x6cc4ff),
            ("AXIS_X", 0xff5a4f),
            ("AXIS_Y", 0x5ef05e),
            ("AXIS_Z", 0x5aa5ff),
        ],
    },
    Preset {
        name: "ember",
        label: "Ember",
        blurb: "Warm brown, terracotta",
        colors: &[
            ("BG", 0x1c1714),
            ("SURFACE", 0x2a231f),
            ("TEXT", 0xefe6dc),
            ("LABEL", 0xefe6dc),
            ("MUTED", 0xf7efe6),
            ("ACCENT", 0xe8956b),
            ("NEUTRAL_300", 0xe3d6c8),
            ("NEUTRAL_500", 0xa8998a),
            ("NEUTRAL_800", 0x4a3f38),
            ("NEUTRAL_900", 0x302823),
            ("ACCENT_100", 0xfff3ea),
            ("ACCENT_200", 0xfde2d0),
            ("ACCENT_300", 0xf6c3a3),
            ("ACCENT_400", 0xf0a77e),
            ("ACCENT_800", 0x5e3826),
            ("ACCENT_900", 0x43291d),
            ("WARNING", 0xe6c170),
            ("ERROR", 0xec8a8a),
            ("SUCCESS", 0xa9cf8a),
            ("INFO", 0x9bb8d9),
            ("AXIS_X", 0xe8735f),
            ("AXIS_Y", 0x9cc46a),
            ("AXIS_Z", 0x6f9be5),
        ],
    },
];

/// The default: Nocturne.
pub const DEFAULT_PRESET: &str = "nocturne";

/// A preset by name.
pub fn preset(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

/// The accent's ramp, in its order: the steps that are ink
/// (`ACCENT_100`…`300`, text on a tint and accented text), a mark
/// (`ACCENT_400`), and the tints behind ink (`ACCENT_800`, `ACCENT_900`,
/// the selection).
pub const RAMP: [&str; 6] = [
    "ACCENT_100",
    "ACCENT_200",
    "ACCENT_300",
    "ACCENT_400",
    "ACCENT_800",
    "ACCENT_900",
];

/// What text is drawn on what, and at which alpha: each pair must read,
/// 4.5:1 or more, in every preset and for every accent chosen from the
/// swatches.
pub const INK_ON_GROUND: [(&str, u8, &str); 21] = [
    ("TEXT", 100, "BG"),
    ("TEXT", 100, "SURFACE"),
    ("LABEL", 70, "BG"),
    ("LABEL", 70, "SURFACE"),
    ("MUTED", 55, "BG"),
    ("MUTED", 55, "SURFACE"),
    ("TEXT", 100, "ACCENT_900"),
    ("ACCENT_200", 100, "ACCENT_900"),
    ("ACCENT_300", 100, "ACCENT_900"),
    ("ACCENT_100", 100, "ACCENT_800"),
    ("ACCENT_300", 100, "SURFACE"),
    ("ACCENT", 100, "SURFACE"),
    ("WARNING", 100, "SURFACE"),
    ("ERROR", 100, "SURFACE"),
    ("SUCCESS", 100, "SURFACE"),
    ("INFO", 100, "SURFACE"),
    ("NEUTRAL_300", 100, "NEUTRAL_800"),
    ("NEUTRAL_300", 100, "SURFACE"),
    ("AXIS_X", 100, "SURFACE"),
    ("AXIS_Y", 100, "SURFACE"),
    ("AXIS_Z", 100, "SURFACE"),
];

/// A theme file's text — a RON map from token name to `#rrggbb`,
/// `{"ACCENT": "#e07a5f", "BG": "#101014"}`, and optionally a preset,
/// `"preset": "daylight"` — as the preset it names and the colours it
/// sets.
pub fn parse(text: &str) -> Result<(Option<String>, Tokens), String> {
    let map: BTreeMap<String, String> = runity::ron::from_str(text).map_err(|e| e.to_string())?;
    let mut named = None;
    let mut tokens = Tokens::new();
    for (name, value) in map {
        if name == "preset" {
            if preset(&value).is_none() {
                let names: Vec<&str> = PRESETS.iter().map(|p| p.name).collect();
                return Err(format!(
                    "no preset {value:?}; there are {}",
                    names.join(", ")
                ));
            }
            named = Some(value);
            continue;
        }
        if !TOKENS.iter().any(|(n, _)| *n == name) {
            let names: Vec<&str> = TOKENS.iter().map(|(n, _)| *n).collect();
            return Err(format!("no token {name:?}; there are {}", names.join(", ")));
        }
        let rgb = parse_hex(&value).ok_or_else(|| format!("{name}: {value:?} is not #rrggbb"))?;
        tokens.insert(name, rgb);
    }
    Ok((named, tokens))
}

/// `#rrggbb` (the `#` optional) as its three bytes.
pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let hex = text.trim().trim_start_matches('#');
    let n = (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()?;
    Some(rgb(n))
}

pub fn to_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn rgb(n: u32) -> [u8; 3] {
    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}

/// Every token's colour: the preset's, then each layer over it in turn.
///
/// A layer that sets `TEXT` and not `LABEL` or `MUTED` takes those along,
/// as they were the text at an alpha before they had keys of their own. A
/// layer that sets `ACCENT` and not its ramp gets a ramp made from the
/// accent for the ground it is on — choosing an accent recolours the
/// selection and the accented text with it.
pub fn resolve(preset: &Preset, layers: &[&Tokens]) -> Tokens {
    let mut out: Tokens = preset
        .colors
        .iter()
        .map(|(n, c)| (n.to_string(), rgb(*c)))
        .collect();
    for layer in layers {
        for (name, c) in layer.iter() {
            out.insert(name.clone(), *c);
        }
        if let Some(text) = layer.get("TEXT") {
            for follows in ["LABEL", "MUTED"] {
                if !layer.contains_key(follows) {
                    out.insert(follows.to_string(), *text);
                }
            }
        }
        if let Some(accent) = layer.get("ACCENT") {
            let ramp = accent_ramp(*accent, out["SURFACE"], out["TEXT"]);
            for (name, c) in RAMP.iter().zip(ramp) {
                if !layer.contains_key(*name) {
                    out.insert(name.to_string(), c);
                }
            }
        }
    }
    out
}

/// What `Ui::set_palette` takes: each token's Nocturne RGB, which is what
/// the code draws with, to the colour it is to be.
pub fn palette(colours: &Tokens) -> HashMap<[u8; 3], [u8; 3]> {
    TOKENS
        .iter()
        .filter_map(|(name, base)| {
            let from = [base.r, base.g, base.b];
            let to = *colours.get(*name)?;
            (from != to).then_some((from, to))
        })
        .collect()
}

/// A ground whose text is dark.
pub fn is_light(surface: [u8; 3]) -> bool {
    luminance(surface) > 0.4
}

/// The accent's ramp for a ground, in [`RAMP`]'s order: the ink steps
/// toward white on a dark ground and toward black on a light one, the
/// tints the accent a little over the surface — and each pushed further
/// until the text on it reads.
pub fn accent_ramp(accent: [u8; 3], surface: [u8; 3], text: [u8; 3]) -> [[u8; 3]; 6] {
    let light = is_light(surface);
    let (ink, far) = if light {
        ([0.72, 0.58, 0.3], [0, 0, 0])
    } else {
        ([0.9, 0.78, 0.6], [255, 255, 255])
    };
    let a400 = if light {
        mix(accent, [255, 255, 255], 0.15)
    } else {
        mix(accent, far, 0.35)
    };
    // The tints fade toward the surface until the text reads on them…
    let fade = |mut c: [u8; 3]| {
        for _ in 0..20 {
            if contrast(text, c) >= 4.5 {
                break;
            }
            c = mix(c, surface, 0.3);
        }
        c
    };
    let a800 = fade(mix(surface, accent, 0.3));
    let a900 = fade(mix(surface, accent, if light { 0.13 } else { 0.16 }));
    // …and the ink moves away from them until it reads.
    let push = |mut c: [u8; 3], grounds: &[[u8; 3]]| {
        for _ in 0..20 {
            if grounds.iter().all(|g| contrast(c, *g) >= 4.5) {
                break;
            }
            c = mix(c, far, 0.2);
        }
        c
    };
    let a100 = push(mix(accent, far, ink[0]), &[a800]);
    let a200 = push(mix(accent, far, ink[1]), &[a900]);
    let a300 = push(mix(accent, far, ink[2]), &[a900, surface]);
    [a100, a200, a300, a400, a800, a900]
}

/// An accent that reads as text on this surface — the primary button's
/// label is the accent — moved toward white or black as little as it
/// takes.
pub fn readable_accent(accent: [u8; 3], surface: [u8; 3]) -> [u8; 3] {
    let far = if is_light(surface) {
        [0, 0, 0]
    } else {
        [255, 255, 255]
    };
    let mut c = accent;
    for _ in 0..40 {
        if contrast(c, surface) >= 4.5 {
            break;
        }
        c = mix(c, far, 0.08);
    }
    c
}

/// `a` moved toward `b` by `t`, channel by channel.
pub fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

/// `ink` at `alpha` percent over `ground`, as it is seen.
pub fn over(ink: [u8; 3], alpha: u8, ground: [u8; 3]) -> [u8; 3] {
    mix(ground, ink, alpha as f32 / 100.0)
}

/// WCAG's relative luminance.
pub fn luminance(c: [u8; 3]) -> f32 {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG's contrast ratio, 1 to 21: text wants 4.5.
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn nocturne() -> &'static Preset {
        preset(DEFAULT_PRESET).unwrap()
    }

    /// A palette is keyed by RGB: two tokens of one colour could not be
    /// told apart.
    #[test]
    fn every_token_has_a_colour_of_its_own() {
        let mut seen = std::collections::HashSet::new();
        for (name, c) in TOKENS {
            assert!(seen.insert([c.r, c.g, c.b]), "{name} shares its RGB");
        }
    }

    #[test]
    fn every_preset_defines_every_token_once() {
        for p in &PRESETS {
            for (name, _) in TOKENS {
                let n = p.colors.iter().filter(|(t, _)| *t == name).count();
                assert_eq!(n, 1, "{} defines {name} {n} times", p.name);
            }
            assert_eq!(p.colors.len(), TOKENS.len(), "{} has extra tokens", p.name);
        }
    }

    /// Nocturne the preset is Nocturne the constants: choosing it draws
    /// the code as written.
    #[test]
    fn nocturne_is_the_constants() {
        assert!(palette(&resolve(nocturne(), &[])).is_empty());
    }

    fn check_contrast(what: &str, colours: &Tokens) {
        for (ink, alpha, ground) in INK_ON_GROUND {
            let g = colours[ground];
            let seen = over(colours[ink], alpha, g);
            let c = contrast(seen, g);
            assert!(
                c >= 4.5,
                "{what}: {ink} at {alpha}% on {ground} is {c:.2}:1 ({} on {})",
                to_hex(seen),
                to_hex(g)
            );
        }
    }

    #[test]
    fn every_preset_reads() {
        for p in &PRESETS {
            check_contrast(p.name, &resolve(p, &[]));
        }
    }

    /// Any accent, made readable for the preset, keeps every pair
    /// readable: the ramp made from it included.
    #[test]
    fn any_accent_reads_in_every_preset() {
        for p in &PRESETS {
            for i in 0..36 {
                for (s, v) in [(0.7, 0.9), (0.4, 0.6), (1.0, 1.0), (0.2, 0.3)] {
                    let raw = hsv(i as f32 / 36.0, s, v);
                    let accent = readable_accent(raw, p.get("SURFACE"));
                    let layer = Tokens::from([("ACCENT".to_string(), accent)]);
                    check_contrast(
                        &format!("{} with accent {}", p.name, to_hex(accent)),
                        &resolve(p, &[&layer]),
                    );
                }
            }
        }
    }

    fn hsv(h: f32, s: f32, v: f32) -> [u8; 3] {
        let i = (h * 6.0).floor();
        let f = h * 6.0 - i;
        let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
        let (r, g, b) = match i as i32 % 6 {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        };
        [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
    }

    #[test]
    fn layers_resolve_in_order_and_the_text_takes_its_shades() {
        let daylight = preset("daylight").unwrap();
        let user = Tokens::from([("ACCENT".to_string(), [0xe0, 0x7a, 0x5f])]);
        let project = Tokens::from([
            ("SURFACE".to_string(), [0xff, 0xff, 0xff]),
            ("TEXT".to_string(), [0x10, 0x10, 0x10]),
        ]);
        let c = resolve(daylight, &[&user, &project]);
        assert_eq!(c["ACCENT"], [0xe0, 0x7a, 0x5f]);
        assert_ne!(
            c["ACCENT_900"],
            daylight.get("ACCENT_900"),
            "the ramp follows"
        );
        assert_eq!(c["SURFACE"], [0xff, 0xff, 0xff]);
        assert_eq!(c["MUTED"], [0x10, 0x10, 0x10], "muted follows the text");
        assert_eq!(c["BG"], daylight.get("BG"));
    }

    #[test]
    fn a_file_reads_or_says_why() {
        let (p, t) = parse(r##"{"preset": "ember", "ACCENT": "#e07a5f"}"##).unwrap();
        assert_eq!(p.as_deref(), Some("ember"));
        assert_eq!(t["ACCENT"], [0xe0, 0x7a, 0x5f]);
        assert!(parse(r##"{"preset": "neon"}"##)
            .unwrap_err()
            .contains("daylight"));
        assert!(parse(r##"{"ACCENTS": "#000000"}"##)
            .unwrap_err()
            .contains("ACCENT"));
        assert!(parse(r##"{"BG": "black"}"##).is_err());
    }
}
