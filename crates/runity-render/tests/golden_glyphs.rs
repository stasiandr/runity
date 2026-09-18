//! What the glyph rasterizer draws, pixel by pixel.
//!
//! A unit test can say that the inside of a contour is solid and the outside is
//! not; it cannot say that the letters are shaped right, that the antialiasing
//! is even, or that the dots of `Ё` sit over the letter rather than beside it.
//! That is what this reference image is for. Regenerate it deliberately:
//!
//! ```text
//! RUNITY_UPDATE_GOLDEN=1 cargo test -p runity-render --test golden_glyphs
//! ```

use runity_render::font::{Font, GlyphBitmap};
use runity_render::golden::{self, Tolerance};
use runity_render::{Color, Framebuffer};

/// Blend a glyph's coverage onto the frame with the pen on the baseline.
///
/// The whole of the blitter this rasterizer needs: coverage is an alpha, and
/// the bitmap's own offsets say where it hangs off the pen.
fn blit(frame: &mut Framebuffer, bitmap: &GlyphBitmap, pen: (f32, f32), color: Color) {
    let left = pen.0.round() as i32 + bitmap.left;
    let top = pen.1.round() as i32 - bitmap.top;
    for y in 0..bitmap.height {
        for x in 0..bitmap.width {
            let coverage = bitmap.coverage_at(x, y);
            if coverage == 0 {
                continue;
            }
            let (px, py) = (left + x as i32, top + y as i32);
            if px < 0 || py < 0 || px as usize >= frame.width() || py as usize >= frame.height() {
                continue;
            }
            let (px, py) = (px as usize, py as usize);
            let blended = frame.get_pixel(px, py).lerp(color, coverage as f32 / 255.0);
            frame.set_pixel(px, py, blended);
        }
    }
}

/// Draw a string left to right, one cached glyph at a time, and return the pen.
fn draw(frame: &mut Framebuffer, font: &Font, text: &str, size: f32, at: (f32, f32)) -> f32 {
    let mut pen = at.0;
    for ch in text.chars() {
        let Some(glyph) = font.glyph_index(ch) else {
            continue;
        };
        let bitmap = font.rasterize(glyph, size).expect("Roboto rasterizes");
        blit(frame, &bitmap, (pen, at.1), Color::WHITE);
        pen += bitmap.advance;
    }
    pen
}

#[test]
fn a_sheet_of_glyphs_looks_the_way_it_did() {
    let font = Font::embedded();
    let mut frame = Framebuffer::new(300, 140);
    frame.clear(Color::rgb(0.07, 0.08, 0.10));

    // A Latin word, digits and the two composite Cyrillic letters, at the sizes
    // where antialiasing has to do the most work.
    let mut baseline = 2.0;
    for size in [12.0, 17.0, 24.0] {
        baseline += font.ascent(size) - font.descent(size);
        draw(
            &mut frame,
            &font,
            "Runity 015 Ёжик Йод",
            size,
            (8.0, baseline),
        );
        baseline += font.line_gap(size);
    }

    // And one size where the curves are what there is to look at.
    baseline += font.ascent(44.0) + 6.0;
    draw(&mut frame, &font, "Rg7ЁЙ", 44.0, (8.0, baseline));

    golden::assert_matches(
        "tests/golden/glyph-sheet.png",
        &frame,
        Tolerance::new(2, 0.002),
    );
}

#[test]
fn the_same_sheet_comes_out_of_the_cache_identically() {
    // Two passes over one font: the second is served entirely by the cache, and
    // a cache that handed back a stale or shifted bitmap would show up here as
    // a different image.
    let font = Font::embedded();
    let mut first = Framebuffer::new(300, 60);
    first.clear(Color::BLACK);
    draw(&mut first, &font, "Ёлка 42", 28.0, (8.0, 40.0));
    let cached = font.cache_bytes();

    let mut second = Framebuffer::new(300, 60);
    second.clear(Color::BLACK);
    draw(&mut second, &font, "Ёлка 42", 28.0, (8.0, 40.0));

    assert_eq!(first.pixels(), second.pixels());
    assert_eq!(font.cache_bytes(), cached, "the second pass cached nothing");
    assert!(cached > 0);
}
