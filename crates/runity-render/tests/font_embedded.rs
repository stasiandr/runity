//! What the embedded font has to contain.
//!
//! `Font::embedded()` is the font every later text feature falls back to, so
//! the repertoire it must cover is written down here rather than discovered
//! when a glyph comes out as an empty box. Swap `assets/Roboto-Regular.ttf` for
//! a font with a thinner character set and this test names the first code point
//! that went missing.

use runity_render::font::{Font, FontError};

/// The repertoire, in the groups it was agreed in.
fn required() -> Vec<(&'static str, Vec<char>)> {
    vec![
        // Everything printable in ASCII, space included.
        ("ASCII", (0x20u32..=0x7E).map(char_of).collect()),
        // Russian, both cases, with the two letters that live outside the block.
        (
            "Cyrillic",
            (0x410u32..=0x44F).map(char_of).chain(['Ё', 'ё']).collect(),
        ),
        // Percent, degrees, the number sign and the arithmetic a UI shows.
        // The minus is the real U+2212, not the hyphen from ASCII.
        (
            "signs",
            "% ‰ ° № + − × ÷ ± = ≈ ≠ ≤ ≥ ·"
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect(),
        ),
        (
            "currencies",
            "$ € £ ¥ ₽".chars().filter(|c| !c.is_whitespace()).collect(),
        ),
        (
            "typography",
            "— – « » „ “ ” ‘ ’ … • § © ®"
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect(),
        ),
    ]
}

fn char_of(code: u32) -> char {
    char::from_u32(code).expect("the ranges here are all valid scalar values")
}

#[test]
fn embedded_covers_required_codepoints() {
    let font = Font::embedded();
    for (group, chars) in required() {
        for ch in chars {
            assert!(
                font.glyph_index(ch).is_some(),
                "the embedded font has no glyph for {ch:?} (U+{:04X}, {group})",
                ch as u32
            );
        }
    }
}

#[test]
fn every_required_codepoint_has_an_advance() {
    // A glyph id that maps to nothing in `hmtx` would draw as a zero-width
    // pile-up rather than as text.
    let font = Font::embedded();
    for (group, chars) in required() {
        for ch in chars {
            let glyph = font.glyph_index(ch).expect("covered, per the test above");
            let metrics = font
                .glyph_metrics(glyph)
                .unwrap_or_else(|| panic!("no metrics for {ch:?} ({group})"));
            assert!(
                metrics.advance_width > 0,
                "{ch:?} (U+{:04X}, {group}) has no advance",
                ch as u32
            );
        }
    }
}

#[test]
fn embedded_font_metrics_are_the_ones_roboto_ships() {
    let font = Font::embedded();
    assert_eq!(font.units_per_em(), 2048, "Roboto is a 2048-unit font");
    assert_eq!(font.num_glyphs(), 3387);
    assert!(font.ascender_units() > 0);
    assert!(font.descender_units() < 0);
    assert!(
        font.line_height_units() > font.units_per_em() as i32,
        "lines must not overlap by default"
    );
}

#[test]
fn the_embedded_font_is_also_readable_from_disk() {
    // `Font::load` and `Font::embedded` are two ways to the same font.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/Roboto-Regular.ttf");
    let loaded = Font::load(path).expect("the asset is where the crate says it is");
    let embedded = Font::embedded();
    assert_eq!(loaded.units_per_em(), embedded.units_per_em());
    assert_eq!(loaded.num_glyphs(), embedded.num_glyphs());

    let a = loaded.glyph_index('A').unwrap();
    assert_eq!(loaded.glyph_index('A'), embedded.glyph_index('A'));
    assert_eq!(loaded.raw_glyph(a), embedded.raw_glyph(a));

    // Index 1 of a file that holds one font does not exist.
    assert_eq!(
        Font::load_indexed(path, 1).unwrap_err(),
        FontError::NoSuchFont { index: 1, count: 1 }
    );
}

#[test]
fn the_font_asset_ships_with_its_licence() {
    let licence = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/Roboto-LICENSE.txt");
    let text = std::fs::read_to_string(licence).expect("the licence sits next to the font");
    assert!(
        text.contains("Apache License"),
        "Roboto is redistributed under Apache-2.0 and the text has to come with it"
    );
}
