//! Glyphs out of the font rasterizer, written as PNGs.
//!
//! No window and no scene: the font module turns outlines into coverage, and
//! coverage is an alpha you can blend with. What there is to look at is the
//! quality of the edges at small sizes and the seam that is not there where a
//! composite glyph was assembled from its components.
//!
//! ```text
//! cargo run --release --example glyph_sheet
//! cargo run --release --example glyph_sheet -- /tmp/frames
//! ```

use runity::prelude::*;
use runity::render::font::{Font, GlyphBitmap};

/// Blend a glyph's coverage onto the frame with the pen on the baseline.
///
/// This is the whole blitter: the bitmap says how far left of the pen and how
/// far above the baseline it starts, and each byte of coverage is an alpha.
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

/// Draw a string left to right and return where the pen ended up.
fn draw(
    frame: &mut Framebuffer,
    font: &Font,
    text: &str,
    size: f32,
    at: (f32, f32),
    color: Color,
) -> f32 {
    let mut pen = at.0;
    for ch in text.chars() {
        let Some(glyph) = font.glyph_index(ch) else {
            continue;
        };
        let bitmap = font
            .rasterize(glyph, size)
            .expect("the embedded font reads");
        blit(frame, &bitmap, (pen, at.1), color);
        pen += bitmap.advance;
    }
    pen
}

const BACKGROUND: Color = Color::rgb(0.07, 0.08, 0.10);
const CAPTION: Color = Color::rgb(0.45, 0.50, 0.58);
const CAPTION_SIZE: f32 = 13.0;

fn sheet(font: &Font) -> Framebuffer {
    let mut frame = Framebuffer::new(640, 230);
    frame.clear(BACKGROUND);
    draw(
        &mut frame,
        font,
        "Один и тот же текст на шести кеглях — Roboto, растеризованный здесь",
        CAPTION_SIZE,
        (16.0, 24.0),
        CAPTION,
    );

    let mut baseline = 40.0;
    for size in [9.0, 11.0, 14.0, 18.0, 24.0, 32.0] {
        baseline += font.ascent(size) - font.descent(size) + font.line_gap(size);
        draw(
            &mut frame,
            font,
            "Runity 0159 — Ёжик, Йод, Щука",
            size,
            (16.0, baseline),
            Color::WHITE,
        );
    }
    frame
}

fn large(font: &Font) -> Framebuffer {
    let mut frame = Framebuffer::new(640, 300);
    frame.clear(BACKGROUND);
    draw(
        &mut frame,
        font,
        "Латинская буква, цифра и составные Ё и Й крупно: края кривых",
        CAPTION_SIZE,
        (16.0, 24.0),
        CAPTION,
    );
    draw(
        &mut frame,
        font,
        "Rg7ЁЙ",
        120.0,
        (16.0, 160.0),
        Color::WHITE,
    );
    draw(&mut frame, font, "Rg7ЁЙ", 48.0, (16.0, 230.0), Color::WHITE);
    draw(&mut frame, font, "Rg7ЁЙ", 24.0, (16.0, 270.0), Color::WHITE);
    frame
}

fn composite(font: &Font) -> Framebuffer {
    let mut frame = Framebuffer::new(640, 300);
    frame.clear(BACKGROUND);
    draw(
        &mut frame,
        font,
        "Ё поверх Е: компонент красным, диерезис над ним — белым",
        CAPTION_SIZE,
        (16.0, 24.0),
        CAPTION,
    );

    // The same pen for both: Ё in white, then the plain Е over it in red. The
    // red covers the component exactly, so anything white below the dots would
    // be a component placed by the wrong transform.
    for (pen, size) in [
        ((30.0, 200.0), 190.0),
        ((260.0, 200.0), 96.0),
        ((400.0, 200.0), 48.0),
    ] {
        draw(&mut frame, font, "Ё", size, pen, Color::WHITE);
        draw(
            &mut frame,
            font,
            "Е",
            size,
            pen,
            Color::rgb(0.90, 0.25, 0.30),
        );
    }

    // And Й beside И, at a size where the breve is a handful of pixels.
    draw(&mut frame, font, "ИЙ", 48.0, (480.0, 200.0), Color::WHITE);
    draw(
        &mut frame,
        font,
        "И / Й",
        CAPTION_SIZE,
        (480.0, 230.0),
        CAPTION,
    );
    frame
}

fn main() -> std::io::Result<()> {
    let directory = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let font = Font::embedded();

    for (name, frame) in [
        ("010-sizes.png", sheet(&font)),
        ("020-large.png", large(&font)),
        ("030-composite.png", composite(&font)),
    ] {
        let path = std::path::Path::new(&directory).join(name);
        save_png(&path, &frame)?;
        println!("wrote {}", path.display());
    }
    println!(
        "{} glyphs cached, {} bytes",
        font.cached_glyphs(),
        font.cache_bytes()
    );
    Ok(())
}
