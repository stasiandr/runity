//! One sheet with everything `font::text` does at once.
//!
//! Pangrams in Cyrillic and Latin, the digits and signs `font_embedded.rs`
//! requires the embedded font to cover, six sizes from 11 to 48 pixels, three
//! alignments framed by [`Framebuffer::stroke_rect`], a wrapped paragraph, and
//! white text with a shadow and with a panel over a gradient. Acceptance is
//! by eye, especially at 11-13px, where a rasterizer with no hinting shows it.
//!
//! ```text
//! RUNITY_UPDATE_GOLDEN=1 cargo test -p runity-render --test golden_text
//! ```

use runity_render::font::{Align, Font, TextStyle};
use runity_render::golden::{self, Tolerance};
use runity_render::{Color, Framebuffer};

const BACKGROUND: Color = Color::rgb(0.07, 0.08, 0.10);
const INK: Color = Color::WHITE;
const CAPTION: Color = Color::rgb(0.45, 0.50, 0.58);
const FRAME: Color = Color::rgb(0.30, 0.34, 0.40);
const MARGIN: i32 = 16;
const WIDTH: usize = 900;
const CAPTION_SIZE: f32 = 13.0;
const GAP: i32 = 10;

/// A left-to-right gradient, so a panel or a shadow drawn over it proves it
/// still reads against changing content, not just a flat backdrop.
fn gradient_rect(
    frame: &mut Framebuffer,
    x: i32,
    y: i32,
    width: usize,
    height: usize,
    from: Color,
    to: Color,
) {
    for row in 0..height {
        for col in 0..width {
            let t = col as f32 / (width.max(2) - 1) as f32;
            frame.set_pixel(
                (x + col as i32) as usize,
                (y + row as i32) as usize,
                from.lerp(to, t),
            );
        }
    }
}

/// A small caption, and where the next section starts.
fn caption(frame: &mut Framebuffer, font: &Font, text: &str, y: i32) -> i32 {
    let style = TextStyle::new(font).size(CAPTION_SIZE).color(CAPTION);
    style.draw(frame, text, MARGIN, y);
    y + style.measure(text).height.ceil() as i32 + 6
}

/// Copy the top-left `(width, height)` of `frame` into a new, tightly sized
/// one — the sheet is built into a generous canvas so nothing has to be
/// measured twice, then cropped to what was actually used.
fn crop(frame: &Framebuffer, width: usize, height: usize) -> Framebuffer {
    let mut out = Framebuffer::new(width, height);
    for y in 0..height {
        for x in 0..width {
            out.set_pixel(x, y, frame.get_pixel(x, y));
        }
    }
    out
}

#[test]
fn a_full_sheet_of_text_features_looks_the_way_it_did() {
    let font = Font::embedded();
    let mut frame = Framebuffer::new(WIDTH, 2200);
    frame.clear(BACKGROUND);
    let mut y = MARGIN;

    // --- pangrams: Cyrillic and Latin -----------------------------------
    y = caption(&mut frame, &font, "Панграммы: кириллица и латиница", y);
    let pangram_style = TextStyle::new(&font).size(20.0).color(INK);
    let cyrillic = "Съешь же ещё этих мягких французских булок, да выпей чаю.";
    pangram_style.draw(&mut frame, cyrillic, MARGIN, y);
    y += pangram_style.measure(cyrillic).height.ceil() as i32 + 4;
    let latin = "The quick brown fox jumps over the lazy dog.";
    pangram_style.draw(&mut frame, latin, MARGIN, y);
    y += pangram_style.measure(latin).height.ceil() as i32 + GAP * 2;

    // --- digits and the required signs (font_embedded.rs's repertoire) --
    y = caption(
        &mut frame,
        &font,
        "Цифры и знаки из обязательного списка",
        y,
    );
    let signs =
        "0123456789   % ‰ ° № + − × ÷ ± = ≈ ≠ ≤ ≥ ·   $ € £ ¥ ₽   — – « » „ “ ” ‘ ’ … • § © ®";
    let signs_style = TextStyle::new(&font)
        .size(18.0)
        .color(INK)
        .tabular_digits(true);
    signs_style.draw(&mut frame, signs, MARGIN, y);
    y += signs_style.measure(signs).height.ceil() as i32 + GAP * 2;

    // --- six sizes, 11 to 48px --------------------------------------------
    y = caption(
        &mut frame,
        &font,
        "Шесть кеглей 11–48px — на 11–13px виден недостаток хинтинга",
        y,
    );
    let sample = "Rg7 Кёльн, Йод — fox 0123";
    for size in [11.0, 13.0, 16.0, 20.0, 28.0, 48.0] {
        let style = TextStyle::new(&font).size(size).color(INK);
        style.draw(&mut frame, sample, MARGIN, y);
        y += style.measure(sample).height.ceil() as i32 + 4;
    }
    y += GAP;

    // --- three alignments, each framed with stroke_rect --------------------
    y = caption(
        &mut frame,
        &font,
        "Выравнивание: слева / по центру / справа",
        y,
    );
    let column_width = 260usize;
    let pad = 8i32;
    let gap_x = 20i32;
    let aligned_text = "Каждая колонка переносится и выравнивается по-своему.";
    let inner_width = column_width as f32 - (pad * 2) as f32;
    let align_probe = TextStyle::new(&font).size(14.0).wrap_width(inner_width);
    let box_height = align_probe.measure(aligned_text).height.ceil() as i32 + pad * 2;

    for (i, align) in [Align::Left, Align::Center, Align::Right]
        .into_iter()
        .enumerate()
    {
        let x = MARGIN + i as i32 * (column_width as i32 + gap_x);
        frame.stroke_rect(x, y, column_width, box_height as usize, 1, FRAME);
        let style = TextStyle::new(&font)
            .size(14.0)
            .color(INK)
            .wrap_width(inner_width)
            .align(align);
        style.draw(&mut frame, aligned_text, x + pad, y + pad);
    }
    y += box_height + GAP * 2;

    // --- a paragraph with wrap ----------------------------------------------
    y = caption(&mut frame, &font, "Абзац с переносом", y);
    let paragraph = "Растеризатор рисует буквы сам, без хинтинга и без стороннего \
        шрифтового движка: контуры превращаются в отрезки, а отрезки — в покрытие \
        пикселя, и перенос строк ломает слово только тогда, когда оно само по себе \
        шире отведённой ширины.";
    let paragraph_style = TextStyle::new(&font)
        .size(15.0)
        .color(INK)
        .wrap_width(420.0);
    paragraph_style.draw(&mut frame, paragraph, MARGIN, y);
    y += paragraph_style.measure(paragraph).height.ceil() as i32 + GAP * 2;

    // --- shadow and panel, both over a gradient -----------------------------
    y = caption(
        &mut frame,
        &font,
        "Белый текст с тенью и с панелью поверх градиента",
        y,
    );
    let gradient_height = 96usize;
    gradient_rect(
        &mut frame,
        MARGIN,
        y,
        WIDTH - MARGIN as usize * 2,
        gradient_height,
        Color::rgb(0.85, 0.35, 0.20),
        Color::rgb(0.15, 0.35, 0.80),
    );
    let shadow_style = TextStyle::new(&font)
        .size(24.0)
        .color(INK)
        .shadow(Color::rgba(0.0, 0.0, 0.0, 0.85));
    shadow_style.draw(&mut frame, "Тень держит контраст", MARGIN + 16, y + 10);
    let panel_style = TextStyle::new(&font)
        .size(24.0)
        .color(INK)
        .panel(Color::rgba(0.0, 0.0, 0.0, 0.55));
    panel_style.draw(&mut frame, "Панель держит контраст", MARGIN + 16, y + 50);
    y += gradient_height as i32 + MARGIN;

    let sheet = crop(&frame, WIDTH, y as usize);
    golden::assert_matches(
        "tests/golden/font-showcase.png",
        &sheet,
        Tolerance::default(),
    );
}
