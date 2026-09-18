//! A bitmap font, written out by hand.
//!
//! Anything drawn on top of the frame — frame times, tick counts, what the AI
//! thinks it is doing — needs glyphs, and glyphs normally mean a font file, a
//! parser for it, and a rasterizer. For debug text that is all cost and no
//! benefit: five by seven pixels is legible, needs no hinting, no kerning and
//! no antialiasing, and lives in the binary as sixty-odd integers.
//!
//! Each glyph is five columns of seven rows, one bit per pixel, stored as a
//! `u8` per column with the low bit at the top. Written as binary literals
//! they are readable as pictures, which is the only reason this is
//! maintainable.

/// Width of a glyph in pixels.
pub const GLYPH_WIDTH: usize = 5;

/// Height of a glyph in pixels.
pub const GLYPH_HEIGHT: usize = 7;

/// Pixels between glyphs at scale 1.
pub const GLYPH_SPACING: usize = 1;

/// Width of one character cell, glyph plus spacing.
pub const CELL_WIDTH: usize = GLYPH_WIDTH + GLYPH_SPACING;

/// The columns of one glyph, or `None` for something unprintable.
///
/// Lower case is mapped to upper case: a debug overlay does not need
/// descenders, and doubling the table to get them would double the work of
/// maintaining it.
pub fn glyph(character: char) -> Option<[u8; GLYPH_WIDTH]> {
    let upper = character.to_ascii_uppercase();
    Some(match upper {
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00],
        '!' => [0x00, 0x00, 0x5F, 0x00, 0x00],
        '"' => [0x00, 0x07, 0x00, 0x07, 0x00],
        '#' => [0x14, 0x7F, 0x14, 0x7F, 0x14],
        '%' => [0x23, 0x13, 0x08, 0x64, 0x62],
        '\'' => [0x00, 0x00, 0x07, 0x00, 0x00],
        '(' => [0x00, 0x1C, 0x22, 0x41, 0x00],
        ')' => [0x00, 0x41, 0x22, 0x1C, 0x00],
        '*' => [0x14, 0x08, 0x3E, 0x08, 0x14],
        '+' => [0x08, 0x08, 0x3E, 0x08, 0x08],
        ',' => [0x00, 0x50, 0x30, 0x00, 0x00],
        '-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        '.' => [0x00, 0x60, 0x60, 0x00, 0x00],
        '/' => [0x20, 0x10, 0x08, 0x04, 0x02],
        '0' => [0x3E, 0x51, 0x49, 0x45, 0x3E],
        '1' => [0x00, 0x42, 0x7F, 0x40, 0x00],
        '2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        '3' => [0x21, 0x41, 0x45, 0x4B, 0x31],
        '4' => [0x18, 0x14, 0x12, 0x7F, 0x10],
        '5' => [0x27, 0x45, 0x45, 0x45, 0x39],
        '6' => [0x3C, 0x4A, 0x49, 0x49, 0x30],
        '7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        '8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        '9' => [0x06, 0x49, 0x49, 0x29, 0x1E],
        ':' => [0x00, 0x36, 0x36, 0x00, 0x00],
        ';' => [0x00, 0x56, 0x36, 0x00, 0x00],
        '<' => [0x08, 0x14, 0x22, 0x41, 0x00],
        '=' => [0x14, 0x14, 0x14, 0x14, 0x14],
        '>' => [0x00, 0x41, 0x22, 0x14, 0x08],
        '?' => [0x02, 0x01, 0x51, 0x09, 0x06],
        'A' => [0x7E, 0x11, 0x11, 0x11, 0x7E],
        'B' => [0x7F, 0x49, 0x49, 0x49, 0x36],
        'C' => [0x3E, 0x41, 0x41, 0x41, 0x22],
        'D' => [0x7F, 0x41, 0x41, 0x22, 0x1C],
        'E' => [0x7F, 0x49, 0x49, 0x49, 0x41],
        'F' => [0x7F, 0x09, 0x09, 0x09, 0x01],
        'G' => [0x3E, 0x41, 0x49, 0x49, 0x7A],
        'H' => [0x7F, 0x08, 0x08, 0x08, 0x7F],
        'I' => [0x00, 0x41, 0x7F, 0x41, 0x00],
        'J' => [0x20, 0x40, 0x41, 0x3F, 0x01],
        'K' => [0x7F, 0x08, 0x14, 0x22, 0x41],
        'L' => [0x7F, 0x40, 0x40, 0x40, 0x40],
        'M' => [0x7F, 0x02, 0x0C, 0x02, 0x7F],
        'N' => [0x7F, 0x04, 0x08, 0x10, 0x7F],
        'O' => [0x3E, 0x41, 0x41, 0x41, 0x3E],
        'P' => [0x7F, 0x09, 0x09, 0x09, 0x06],
        'Q' => [0x3E, 0x41, 0x51, 0x21, 0x5E],
        'R' => [0x7F, 0x09, 0x19, 0x29, 0x46],
        'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
        'T' => [0x01, 0x01, 0x7F, 0x01, 0x01],
        'U' => [0x3F, 0x40, 0x40, 0x40, 0x3F],
        'V' => [0x1F, 0x20, 0x40, 0x20, 0x1F],
        'W' => [0x7F, 0x20, 0x18, 0x20, 0x7F],
        'X' => [0x63, 0x14, 0x08, 0x14, 0x63],
        'Y' => [0x03, 0x04, 0x78, 0x04, 0x03],
        'Z' => [0x61, 0x51, 0x49, 0x45, 0x43],
        '[' => [0x00, 0x7F, 0x41, 0x41, 0x00],
        ']' => [0x00, 0x41, 0x41, 0x7F, 0x00],
        '_' => [0x40, 0x40, 0x40, 0x40, 0x40],
        '|' => [0x00, 0x00, 0x7F, 0x00, 0x00],
        _ => return None,
    })
}

/// Width in pixels of a string drawn at a given scale.
pub fn text_width(text: &str, scale: usize) -> usize {
    let scale = scale.max(1);
    let characters = text.chars().count();
    if characters == 0 {
        0
    } else {
        (characters * CELL_WIDTH - GLYPH_SPACING) * scale
    }
}

/// Height in pixels of one line at a given scale.
pub fn line_height(scale: usize) -> usize {
    (GLYPH_HEIGHT + 2) * scale.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_printable_character_has_a_glyph() {
        for character in "abcdefghijklmnopqrstuvwxyz0123456789 .,:;!?-+/=<>()[]|_'\"#%*".chars() {
            assert!(glyph(character).is_some(), "no glyph for {character:?}");
        }
    }

    #[test]
    fn lower_case_borrows_the_upper_case_glyph() {
        assert_eq!(glyph('a'), glyph('A'));
        assert_eq!(glyph('z'), glyph('Z'));
    }

    #[test]
    fn an_unknown_character_is_reported_rather_than_drawn_as_rubbish() {
        assert_eq!(glyph('\u{1F600}'), None);
        assert_eq!(glyph('\t'), None);
    }

    #[test]
    fn glyphs_are_distinct_enough_to_read() {
        // A copy-paste slip in a hand-written table shows up as two letters
        // with identical bitmaps, which is exactly the sort of thing nobody
        // notices by eye.
        let alphabet: Vec<(char, [u8; GLYPH_WIDTH])> = ('A'..='Z')
            .chain('0'..='9')
            .filter_map(|character| glyph(character).map(|bits| (character, bits)))
            .collect();
        for (index, (character, bits)) in alphabet.iter().enumerate() {
            for (other, other_bits) in &alphabet[index + 1..] {
                assert_ne!(bits, other_bits, "{character} and {other} look the same");
            }
        }
    }

    #[test]
    fn glyphs_fit_the_cell() {
        // Seven rows means the top bit of the column byte must be clear;
        // a stray eighth row would bleed into the line below.
        for character in ('A'..='Z').chain('0'..='9').chain(" .,:;!?-+/=".chars()) {
            let Some(bits) = glyph(character) else {
                continue;
            };
            for column in bits {
                assert_eq!(column & 0x80, 0, "{character} has an eighth row");
            }
        }
    }

    #[test]
    fn a_blank_is_blank_and_everything_else_is_not() {
        assert_eq!(glyph(' '), Some([0; GLYPH_WIDTH]));
        for character in ('A'..='Z').chain('0'..='9') {
            assert_ne!(
                glyph(character),
                Some([0; GLYPH_WIDTH]),
                "{character} is invisible"
            );
        }
    }

    #[test]
    fn measurements_match_what_is_drawn() {
        assert_eq!(text_width("", 1), 0);
        assert_eq!(text_width("A", 1), GLYPH_WIDTH);
        assert_eq!(text_width("AB", 1), GLYPH_WIDTH * 2 + GLYPH_SPACING);
        assert_eq!(text_width("AB", 2), (GLYPH_WIDTH * 2 + GLYPH_SPACING) * 2);
        assert_eq!(line_height(1), GLYPH_HEIGHT + 2);
        assert_eq!(line_height(3), (GLYPH_HEIGHT + 2) * 3);
    }
}
