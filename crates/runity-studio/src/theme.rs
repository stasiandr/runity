//! Nocturne: the editor's look, as tokens.
//!
//! The design system lives in Claude Design (project «Nocturne»); its
//! `styles.css` is the source of truth and these constants are copied from
//! it, name for name — `--color-accent-800` is [`ACCENT_800`]. A quiet,
//! compact dark interface: a near-neutral blue-grey ground, Inter at medium
//! weight, 8px radii, and an accent used as a line and a glow rather than a
//! flood. Contrast comes from the tonal ramps, not from saturation.
//!
//! Two things Nocturne does not have and an editor needs are added here and
//! marked as such: a warning and an error colour (Nocturne is a mono scheme,
//! and a Console cannot be), kept at the ramps' low chroma so they read as
//! part of the same page.

use std::borrow::Cow;

use gpui::{px, rgb, rgba, App, Hsla, Pixels, Rgba};
use gpui_kit::component::{Theme, ThemeMode};

// --- colour roles -----------------------------------------------------

/// `--color-bg`: the ground behind every panel.
pub const BG: u32 = 0x161826;
/// `--color-surface`: a panel.
pub const SURFACE: u32 = 0x232532;
/// `--color-text`.
pub const TEXT: u32 = 0xe9e9ed;
/// `--color-accent`: lines, marks, the focused thing. Never a flood.
pub const ACCENT: u32 = 0x9184d9;

pub const NEUTRAL_300: u32 = 0xcfd3e5;
pub const NEUTRAL_500: u32 = 0x9397ab;
pub const NEUTRAL_600: u32 = 0x75798c;
pub const NEUTRAL_700: u32 = 0x595d6c;
pub const NEUTRAL_800: u32 = 0x3f424d;
pub const NEUTRAL_900: u32 = 0x292b31;

pub const ACCENT_100: u32 = 0xf5f4ff;
pub const ACCENT_200: u32 = 0xe7e5fe;
pub const ACCENT_300: u32 = 0xd2cefd;
pub const ACCENT_400: u32 = 0xb5abfc;
pub const ACCENT_700: u32 = 0x5d5294;
pub const ACCENT_800: u32 = 0x423a6a;
pub const ACCENT_900: u32 = 0x2b2741;

/// Not Nocturne's: a Console needs to say «careful». Amber at the ramps'
/// lightness and low chroma.
pub const WARNING: u32 = 0xd9bc84;
/// Not Nocturne's: a Console needs to say «this failed». A muted rose.
pub const ERROR: u32 = 0xe58a96;

/// A colour role at an opacity — Nocturne's `color-mix(in srgb, X n%,
/// transparent)`.
pub fn mix(color: u32, percent: u32) -> Rgba {
    rgba((color << 8) | (percent * 255 / 100))
}

/// `--color-divider`: the text at 16%.
pub fn divider() -> Rgba {
    mix(TEXT, 16)
}

/// Muted text: `.text-muted`, the text at 55%.
pub fn muted() -> Rgba {
    mix(TEXT, 55)
}

/// Labels: `.field > label`, the text at 70%.
pub fn label() -> Rgba {
    mix(TEXT, 70)
}

/// A secondary control's hover: the text at 7%.
pub fn hover() -> Rgba {
    mix(TEXT, 7)
}

/// A secondary control's pressed state: the text at 14%.
pub fn pressed() -> Rgba {
    mix(TEXT, 14)
}

/// The primary (outlined) button's hover: the accent at 12%.
pub fn accent_hover() -> Rgba {
    mix(ACCENT, 12)
}

// --- space, radius, type ---------------------------------------------

/// `--space-*`: the compact scale, density 0.7× already baked in.
pub const SPACE_1: Pixels = px(2.8);
pub const SPACE_2: Pixels = px(5.6);
pub const SPACE_3: Pixels = px(8.4);
pub const SPACE_4: Pixels = px(11.2);
pub const SPACE_6: Pixels = px(16.8);

pub const RADIUS_SM: Pixels = px(4.0);
pub const RADIUS_MD: Pixels = px(8.0);
pub const RADIUS_LG: Pixels = px(14.0);

/// `--font-body` and `--font-heading`: Inter, shipped with the editor so
/// that it looks the same on a machine that has never installed it.
pub const FONT: &str = "Inter Variable";
/// The monospace face for values that are RON and lines that are logs.
pub const MONO: &str = "Menlo";

/// Inter (SIL Open Font License, `assets/fonts/Inter-LICENSE.txt`).
static INTER: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");

/// Load the font and point GPUI Kit's components at Nocturne's colours, so
/// that an input or a scrollbar from the kit sits in the same page as the
/// panels drawn here.
pub fn install(cx: &mut App) {
    if let Err(error) = cx.text_system().add_fonts(vec![Cow::Borrowed(INTER)]) {
        eprintln!("Inter did not load ({error}); the system font stands in");
    }
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    theme.font_family = FONT.into();
    theme.font_size = px(13.0);
    theme.mono_font_family = MONO.into();
    theme.mono_font_size = px(12.0);
    theme.radius = RADIUS_MD;
    theme.radius_lg = RADIUS_LG;
    theme.shadow = false;

    let c = &mut theme.colors;
    let h = |color: u32| -> Hsla { rgb(color).into() };
    let a = |color: u32, percent: u32| -> Hsla { mix(color, percent).into() };
    c.background = h(BG);
    c.foreground = h(TEXT);
    c.border = a(TEXT, 16);
    c.input = a(TEXT, 16);
    c.ring = h(ACCENT);
    c.caret = h(ACCENT);
    c.selection = a(ACCENT, 30);
    c.accent = a(TEXT, 7);
    c.accent_foreground = h(TEXT);
    c.muted = h(NEUTRAL_900);
    c.muted_foreground = a(TEXT, 55);
    c.primary = h(ACCENT);
    c.primary_hover = h(ACCENT_400);
    c.primary_active = h(ACCENT_300);
    c.primary_foreground = h(BG);
    c.secondary = h(SURFACE);
    c.secondary_hover = a(TEXT, 7);
    c.secondary_active = a(TEXT, 14);
    c.secondary_foreground = h(TEXT);
    c.popover = h(SURFACE);
    c.popover_foreground = h(TEXT);
    c.list = h(SURFACE);
    c.list_hover = a(TEXT, 4);
    c.list_active = h(ACCENT_900);
    c.list_active_border = h(ACCENT);
    c.scrollbar = gpui::transparent_black();
    c.scrollbar_thumb = a(TEXT, 16);
    c.scrollbar_thumb_hover = a(TEXT, 28);
    c.drag_border = h(ACCENT);
    c.drop_target = a(ACCENT, 12);
    c.danger = h(ERROR);
    c.warning = h(WARNING);
    Theme::sync_base(cx);
}
