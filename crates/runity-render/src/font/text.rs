//! Laying out and drawing text: what a [`GlyphBitmap`] does not know.
//!
//! A [`Font`] rasterizes one glyph at a time and has no idea what a line, a
//! paragraph or a color is. This module is the layer above it: [`TextStyle`]
//! turns a string into positioned glyphs — `\n`, word wrap, tab stops,
//! kerning, alignment — and blits them onto a [`Framebuffer`].
//!
//! The pen moves in `f32` so fractional advances do not accumulate rounding
//! error over a long line, but every glyph is dropped onto a whole pixel: an
//! antialiased rasterizer gains nothing from a bitmap placed at a fractional
//! offset, and it costs a second interpolation to get there. Coverage becomes
//! alpha the same way [`Blend::Alpha`](crate::raster::Blend::Alpha) does —
//! [`Color::blend_over`] is the shared formula — so a letter and a
//! semi-transparent triangle drawn over the same pixel end up the same color.
//!
//! A missing codepoint draws `.notdef` (glyph 0), visibly, rather than being
//! skipped: a box of nothing is a bug you can see, a silent gap is not.

use super::{Font, GlyphBitmap};
use crate::color::Color;
use crate::framebuffer::Framebuffer;
use std::rc::Rc;

/// Horizontal alignment of each line, relative to the anchor point passed to
/// [`TextStyle::draw`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    /// Every line starts at the anchor.
    #[default]
    Left,
    /// Every line is centered within the block.
    Center,
    /// Every line ends at the right edge of the block.
    Right,
}

/// The size of a laid-out block of text, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextSize {
    pub width: f32,
    pub height: f32,
}

/// How to set, wrap and paint a run of text — a builder around a borrowed
/// [`Font`].
///
/// Cheap to copy: it holds a reference and a handful of scalars, so a caller
/// can build one per frame, or once and reuse it.
#[derive(Clone, Copy)]
pub struct TextStyle<'a> {
    font: &'a Font,
    size: f32,
    color: Color,
    shadow: Option<Color>,
    panel: Option<Color>,
    outline: Option<Color>,
    contrast: f32,
    tabular_digits: bool,
    align: Align,
    line_gap: f32,
    wrap_width: Option<f32>,
}

/// One glyph, already placed on the pen line of its line.
struct PositionedGlyph {
    bitmap: Rc<GlyphBitmap>,
    /// Pen position along the line, from its own left edge.
    x: f32,
}

/// One line's glyphs plus the advance-width layout used it.
struct Line {
    glyphs: Vec<PositionedGlyph>,
    width: f32,
}

impl<'a> TextStyle<'a> {
    /// A left-aligned, unwrapped, opaque white style at 16 pixels.
    pub fn new(font: &'a Font) -> Self {
        Self {
            font,
            size: 16.0,
            color: Color::WHITE,
            shadow: None,
            panel: None,
            outline: None,
            contrast: 1.0,
            tabular_digits: false,
            align: Align::Left,
            line_gap: 0.0,
            wrap_width: None,
        }
    }

    /// Size in pixels, passed straight through to [`Font::rasterize`].
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// The color the glyphs themselves are blended in.
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Draw a second copy one pixel right and down, in `color`, before the
    /// main glyph.
    pub fn shadow(mut self, color: Color) -> Self {
        self.shadow = Some(color);
        self
    }

    /// A rectangle the size of [`TextStyle::measure`], filled with `color`
    /// behind the text — usually semi-transparent.
    pub fn panel(mut self, color: Color) -> Self {
        self.panel = Some(color);
        self
    }

    /// Draw eight extra copies, one pixel out in every direction, in `color`,
    /// before the main glyph — a stand-in for hinting at large sizes, where a
    /// one-pixel shadow reads as an edge rather than a light source.
    pub fn outline(mut self, color: Color) -> Self {
        self.outline = Some(color);
        self
    }

    /// A gamma applied to coverage on top of the rasterizer's own contrast
    /// curve (`font::raster`'s stand-in for hinting). `1.0` (the default)
    /// leaves coverage as the rasterizer produced it;
    /// larger values thicken thin strokes further, which reads better at the
    /// smallest sizes this font has no hinting for.
    pub fn contrast(mut self, contrast: f32) -> Self {
        self.contrast = contrast;
        self
    }

    /// When set, every ASCII digit advances by the width of the font's widest
    /// digit and is centered in that cell, so two numbers of the same length
    /// measure — and line up — identically regardless of which digits they
    /// hold. Off by default, since it is wrong for anything that is not a
    /// column of numbers.
    pub fn tabular_digits(mut self, enabled: bool) -> Self {
        self.tabular_digits = enabled;
        self
    }

    /// Horizontal alignment of each line within the block.
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// Extra pixels between lines, on top of the font's own line height.
    pub fn line_gap(mut self, gap: f32) -> Self {
        self.line_gap = gap;
        self
    }

    /// Wrap at this many pixels: words that fit stay on one line, a run of
    /// words too long for it breaks at the last space that keeps the line
    /// inside `width`, and a single word wider than `width` breaks mid-word
    /// rather than overflowing.
    pub fn wrap_width(mut self, width: f32) -> Self {
        self.wrap_width = Some(width);
        self
    }

    /// The size a block of `text` occupies when drawn at this style — an
    /// advance-based box, the same one [`TextStyle::draw`] fills with
    /// [`TextStyle::panel`]. Nothing [`TextStyle::draw`] draws for `text`
    /// lands outside it, provided neither [`TextStyle::shadow`] nor
    /// [`TextStyle::outline`] is set — both paint up to a pixel past it, by
    /// design, the same way a shadow falls outside the object casting it.
    pub fn measure(&self, text: &str) -> TextSize {
        let lines = self.layout(text);
        let width = lines.iter().map(|l| l.width).fold(0.0f32, f32::max);
        let line_advance = self.font.line_height(self.size) + self.line_gap;
        let height = match lines.len() {
            0 => 0.0,
            n => (n - 1) as f32 * line_advance + self.font.line_height(self.size),
        };
        TextSize { width, height }
    }

    /// Draw `text` with the top-left corner of its first line at `(x, y)`.
    pub fn draw(&self, target: &mut Framebuffer, text: &str, x: i32, y: i32) {
        let lines = self.layout(text);
        if lines.is_empty() {
            return;
        }

        if let Some(panel) = self.panel {
            let size = self.measure(text);
            target.fill_rect(
                x,
                y,
                size.width.ceil() as usize,
                size.height.ceil() as usize,
                panel,
            );
        }

        let align_width = self
            .wrap_width
            .unwrap_or_else(|| lines.iter().map(|l| l.width).fold(0.0f32, f32::max));
        let line_advance = self.font.line_height(self.size) + self.line_gap;
        let ascent = self.font.ascent(self.size);

        for (i, line) in lines.iter().enumerate() {
            let line_offset = match self.align {
                Align::Left => 0.0,
                Align::Center => (align_width - line.width) * 0.5,
                Align::Right => align_width - line.width,
            };
            let baseline_y = y as f32 + ascent + i as f32 * line_advance;
            for glyph in &line.glyphs {
                let pen_x = x as f32 + line_offset + glyph.x;
                self.blit_glyph(target, &glyph.bitmap, pen_x, baseline_y);
            }
        }
    }

    /// Every drawable copy of one glyph: outline, shadow, then the glyph
    /// itself, so the plain color always ends up on top.
    fn blit_glyph(
        &self,
        target: &mut Framebuffer,
        bitmap: &GlyphBitmap,
        pen_x: f32,
        baseline_y: f32,
    ) {
        if let Some(outline) = self.outline {
            for dy in [-1.0, 0.0, 1.0] {
                for dx in [-1.0, 0.0, 1.0] {
                    if dx == 0.0 && dy == 0.0 {
                        continue;
                    }
                    self.blit_bitmap(target, bitmap, pen_x + dx, baseline_y + dy, outline);
                }
            }
        }
        if let Some(shadow) = self.shadow {
            self.blit_bitmap(target, bitmap, pen_x + 1.0, baseline_y + 1.0, shadow);
        }
        self.blit_bitmap(target, bitmap, pen_x, baseline_y, self.color);
    }

    /// Blit one bitmap with the pen rounded to a whole pixel first.
    ///
    /// Clipped to the frame before the pixel loop runs — a glyph, let alone a
    /// whole line, that starts far off one edge costs nothing but the bounds
    /// check, the same motif as the Liang-Barsky clip in `debug::clip_segment`.
    fn blit_bitmap(
        &self,
        target: &mut Framebuffer,
        bitmap: &GlyphBitmap,
        pen_x: f32,
        baseline_y: f32,
        color: Color,
    ) {
        if bitmap.is_empty() {
            return;
        }
        let left = pen_x.round() as i32 + bitmap.left;
        let top = baseline_y.round() as i32 - bitmap.top;
        let (width, height) = (bitmap.width as i32, bitmap.height as i32);
        if left + width <= 0
            || top + height <= 0
            || left >= target.width() as i32
            || top >= target.height() as i32
        {
            return;
        }

        let base_alpha = color.a.clamp(0.0, 1.0);
        let gamma = 1.0 / self.contrast.max(1e-3);
        let x0 = left.max(0);
        let y0 = top.max(0);
        let x1 = (left + width).min(target.width() as i32);
        let y1 = (top + height).min(target.height() as i32);
        for py in y0..y1 {
            let row = (py - top) as usize;
            for px in x0..x1 {
                let coverage = bitmap.coverage_at((px - left) as usize, row);
                if coverage == 0 {
                    continue;
                }
                // Coverage becomes alpha exactly as `Blend::Alpha` does:
                // `dst.lerp(src, cov/255 * color.a)`, `contrast` bent in.
                let alpha = (coverage as f32 / 255.0).powf(gamma) * base_alpha;
                let dst = target.get_pixel(px as usize, py as usize);
                target.set_pixel(px as usize, py as usize, dst.blend_over(color, alpha));
            }
        }
    }

    /// Split `text` into laid-out lines: `\n` first, then word wrap within
    /// each paragraph if [`TextStyle::wrap_width`] is set.
    fn layout(&self, text: &str) -> Vec<Line> {
        if text.is_empty() {
            return Vec::new();
        }
        match self.wrap_width {
            None => text
                .split('\n')
                .map(|line| self.layout_line(line))
                .collect(),
            Some(width) => text
                .split('\n')
                .flat_map(|paragraph| self.wrap_paragraph(paragraph, width))
                .map(|line| self.layout_line(&line))
                .collect(),
        }
    }

    /// Greedy word wrap: accumulate words while they fit, start a new line
    /// when the next one would not, and tear a word that alone is wider than
    /// `width` into chunks that do.
    fn wrap_paragraph(&self, paragraph: &str, width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut current = String::new();
        for word in paragraph.split(' ') {
            if current.is_empty() {
                current = self.place_word(word, width, &mut lines);
                continue;
            }
            let mut candidate = current.clone();
            candidate.push(' ');
            candidate.push_str(word);
            if self.line_width(&candidate) <= width {
                current = candidate;
            } else {
                lines.push(std::mem::take(&mut current));
                current = self.place_word(word, width, &mut lines);
            }
        }
        lines.push(current);
        lines
    }

    /// A word to start a fresh line with: itself if it fits, or the tail end
    /// of it once the chunks ahead of that tail have been pushed to `lines`.
    fn place_word(&self, word: &str, width: f32, lines: &mut Vec<String>) -> String {
        if self.line_width(word) <= width {
            return word.to_string();
        }
        let mut chunks = self.break_word(word, width);
        let last = chunks.pop().unwrap_or_default();
        lines.extend(chunks);
        last
    }

    /// Tear one overlong word into pieces that each fit `width`. The last
    /// piece may still be over, if even one of its characters is.
    fn break_word(&self, word: &str, width: f32) -> Vec<String> {
        let mut chunks = Vec::new();
        let mut current = String::new();
        for ch in word.chars() {
            let mut candidate = current.clone();
            candidate.push(ch);
            if current.is_empty() || self.line_width(&candidate) <= width {
                current = candidate;
            } else {
                chunks.push(std::mem::take(&mut current));
                current.push(ch);
            }
        }
        chunks.push(current);
        chunks
    }

    fn line_width(&self, text: &str) -> f32 {
        self.layout_line(text).width
    }

    /// The widest ASCII digit's advance at this size — the cell
    /// [`TextStyle::tabular_digits`] centers every digit inside.
    fn tabular_cell_width(&self) -> f32 {
        ('0'..='9')
            .filter_map(|d| self.font.glyph_index(d))
            .filter_map(|glyph| self.font.rasterize(glyph, self.size).ok())
            .map(|bitmap| bitmap.advance)
            .fold(0.0f32, f32::max)
    }

    /// Place one line's glyphs: kerning between neighbours, tab stops every
    /// four space-widths, and `.notdef` for anything the font has no glyph
    /// for.
    fn layout_line(&self, text: &str) -> Line {
        let scale = self.font.scale(self.size);
        let space_advance = self
            .font
            .glyph_index(' ')
            .and_then(|g| self.font.rasterize(g, self.size).ok())
            .map(|b| b.advance)
            .unwrap_or(self.size * 0.3);
        let tab_width = (space_advance * 4.0).max(1.0);
        let cell_width = self.tabular_digits.then(|| self.tabular_cell_width());

        let mut glyphs = Vec::new();
        let mut x = 0.0f32;
        let mut previous: Option<u16> = None;
        for ch in text.chars() {
            if ch == '\t' {
                x = ((x / tab_width).floor() + 1.0) * tab_width;
                previous = None;
                continue;
            }
            // `.notdef` (glyph 0) for a codepoint this font has no glyph for:
            // a visible empty box, never a silent skip.
            let glyph_id = self.font.glyph_index(ch).unwrap_or(0);
            let Ok(bitmap) = self.font.rasterize(glyph_id, self.size) else {
                continue;
            };
            if let Some(prev) = previous {
                x += self.font.kerning(prev, glyph_id) as f32 * scale;
            }
            let tabular = cell_width.filter(|_| ch.is_ascii_digit());
            let (glyph_x, advance) = match tabular {
                Some(cell) => (x + (cell - bitmap.advance) * 0.5, cell),
                None => (x, bitmap.advance),
            };
            glyphs.push(PositionedGlyph { bitmap, x: glyph_x });
            x += advance;
            previous = Some(glyph_id);
        }
        Line { glyphs, width: x }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::golden::{self, Tolerance};

    fn lit_pixels(fb: &Framebuffer) -> usize {
        fb.pixels()
            .iter()
            .filter(|p| **p != Color::BLACK.to_argb8())
            .count()
    }

    #[test]
    fn measure_bounds_exactly_what_draw_touches() {
        let font = Font::embedded();
        for text in ["Rg7", "Ёжик 123", "  leading and trailing  ", "W"] {
            for size in [11.0, 17.0, 32.0] {
                let style = TextStyle::new(&font).size(size).color(Color::WHITE);
                let measured = style.measure(text);
                let margin = 4;
                let width = measured.width.ceil() as usize + margin * 2;
                let height = measured.height.ceil() as usize + margin * 2;
                let mut fb = Framebuffer::new(width.max(1), height.max(1));
                fb.clear(Color::BLACK);
                style.draw(&mut fb, text, margin as i32, margin as i32);

                for y in 0..fb.height() {
                    for x in 0..fb.width() {
                        let inside = x >= margin
                            && y >= margin
                            && (x as f32) < margin as f32 + measured.width
                            && (y as f32) < margin as f32 + measured.height;
                        if !inside {
                            assert_eq!(
                                fb.get_pixel(x, y),
                                Color::BLACK,
                                "{text:?} at {size}px lit ({x}, {y}) outside its measured box {measured:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn empty_text_measures_and_draws_to_nothing() {
        let font = Font::embedded();
        let style = TextStyle::new(&font).size(20.0);
        assert_eq!(
            style.measure(""),
            TextSize {
                width: 0.0,
                height: 0.0
            }
        );
        let mut fb = Framebuffer::new(8, 8);
        fb.clear(Color::BLACK);
        style.draw(&mut fb, "", 2, 2);
        assert_eq!(lit_pixels(&fb), 0);
    }

    #[test]
    fn text_far_off_frame_draws_nothing_and_does_not_scan_the_frame() {
        let font = Font::embedded();
        let style = TextStyle::new(&font).size(24.0);
        let mut fb = Framebuffer::new(32, 32);
        fb.clear(Color::BLACK);
        style.draw(&mut fb, "far away", 1_000_000, 1_000_000);
        style.draw(&mut fb, "also far", -1_000_000, -1_000_000);
        assert_eq!(lit_pixels(&fb), 0);
    }

    #[test]
    fn text_straddling_the_edge_draws_only_its_half() {
        let font = Font::embedded();
        let style = TextStyle::new(&font).size(40.0).color(Color::WHITE);
        let size = style.measure("M");
        let mut whole = Framebuffer::new(
            size.width.ceil() as usize + 4,
            size.height.ceil() as usize + 4,
        );
        whole.clear(Color::BLACK);
        style.draw(&mut whole, "M", 2, 2);
        let full_ink = lit_pixels(&whole);
        assert!(full_ink > 0);

        // The same glyph, half off the left edge: strictly fewer pixels lit,
        // and none of them past the frame (which a panic would prove anyway).
        let mut half = Framebuffer::new(whole.width(), whole.height());
        half.clear(Color::BLACK);
        let shift = (size.width / 2.0) as i32;
        style.draw(&mut half, "M", 2 - shift, 2);
        let half_ink = lit_pixels(&half);
        assert!(half_ink > 0 && half_ink < full_ink);
    }

    #[test]
    fn newline_starts_a_new_line_below_the_first() {
        let font = Font::embedded();
        let style = TextStyle::new(&font).size(16.0);
        let one_line = style.measure("Hi");
        let two_lines = style.measure("Hi\nHi");
        assert!((two_lines.height - one_line.height * 2.0).abs() < 1.0);
        assert!((two_lines.width - one_line.width).abs() < 1e-3);
    }

    #[test]
    fn line_gap_adds_pixels_between_lines_only() {
        let font = Font::embedded();
        let base = TextStyle::new(&font).size(16.0);
        let gapped = base.line_gap(10.0);
        let base_height = base.measure("a\nb").height;
        let gapped_height = gapped.measure("a\nb").height;
        assert!((gapped_height - base_height - 10.0).abs() < 1e-3);
        // A single line has no gap to add.
        assert_eq!(base.measure("a").height, gapped.measure("a").height);
    }

    #[test]
    fn wrap_never_exceeds_the_requested_width_when_words_fit() {
        let font = Font::embedded();
        let text = "the quick brown fox jumps over the lazy dog and then keeps going";
        for width in [40.0, 80.0, 150.0] {
            let style = TextStyle::new(&font).size(14.0).wrap_width(width);
            for line in style.layout(text) {
                assert!(
                    line.width <= width + 0.01,
                    "line {:?} is {} wide, past {width}",
                    line.glyphs.len(),
                    line.width
                );
            }
        }
    }

    #[test]
    fn wrap_preserves_every_glyph() {
        // Line doesn't keep the source text, so the proxy for "no word was
        // lost or duplicated" is a glyph count: wrapping only ever turns a
        // separating space into a line break, so the number of glyphs placed
        // is exactly the source's characters minus the number of breaks.
        let font = Font::embedded();
        let text = "one two three four five six seven";
        let style = TextStyle::new(&font).size(14.0).wrap_width(60.0);
        let lines = style.layout(text);
        assert!(
            lines.len() > 1,
            "60px must wrap this text onto more than one line"
        );
        let placed_glyphs: usize = lines.iter().map(|l| l.glyphs.len()).sum();
        let breaks = lines.len() - 1;
        assert_eq!(placed_glyphs, text.chars().count() - breaks);
    }

    #[test]
    fn an_overlong_word_is_torn_rather_than_overflowing() {
        let font = Font::embedded();
        let style = TextStyle::new(&font).size(14.0).wrap_width(20.0);
        let lines = style.layout("supercalifragilisticexpialidocious");
        assert!(lines.len() > 1, "one giant word must still break");
        for line in &lines {
            assert!(line.width <= 20.0 + 0.01, "a torn piece must still fit");
        }
    }

    #[test]
    fn tab_advances_to_the_next_four_space_stop() {
        let font = Font::embedded();
        let style = TextStyle::new(&font).size(16.0);
        let space = style.line_width(" ");
        let tab_width = space * 4.0;

        let before_tab = style.line_width("a");
        let with_tab = style.layout_line("a\t");
        assert!(
            (with_tab.width - tab_width).abs() < 0.5,
            "rounds to the stop, not past it"
        );
        assert!(with_tab.width > before_tab);

        // A second tab from a non-stop position still lands on a stop.
        let two_tabs = style.layout_line("a\t\t");
        assert!((two_tabs.width - tab_width * 2.0).abs() < 0.5);
    }

    /// A synthetic font whose ten digits have visibly different advance
    /// widths — unlike the embedded Roboto, whose numerals are tabular
    /// already, which would make `tabular_digits` look like a no-op here.
    /// Glyph 0 is `.notdef`; glyphs 1..=10 are `'0'..='9'`, each an empty
    /// outline (so only its advance matters) with a distinct width.
    fn digits_of_varying_width() -> Font {
        use super::super::fixture::{self, Builder, Seg};

        let metrics: Vec<(u16, i16)> = (0..11u16).map(|i| (400 + i * 50, 0)).collect();
        let cmap = fixture::cmap_table(&[(
            3,
            1,
            fixture::cmap_format4(&[Seg::Glyphs(b'0' as u16, (1..=10).collect())]),
        )]);
        let bytes = Builder::empty()
            .with(b"head", fixture::head(1000, true))
            .with(b"maxp", fixture::maxp(11))
            .with(b"hhea", fixture::hhea(800, -200, 100, 11))
            .with(b"hmtx", fixture::hmtx(&metrics, &[]))
            .with(b"loca", fixture::loca_long(&[0u32; 12]))
            .with(b"glyf", Vec::new())
            .with(b"cmap", cmap)
            .build();
        Font::from_bytes(bytes).expect("a minimal digits-only font")
    }

    #[test]
    fn tabular_digits_make_equal_length_numbers_measure_equal() {
        let font = digits_of_varying_width();
        let proportional = TextStyle::new(&font).size(18.0);
        // The fixture gives every digit a different width, so without the
        // flag two same-length numbers are not guaranteed to measure equal...
        assert_ne!(
            proportional.measure("111").width,
            proportional.measure("888").width
        );

        // ...but with it, every digit takes the widest digit's cell.
        let tabular = proportional.tabular_digits(true);
        assert_eq!(
            tabular.measure("111").width,
            tabular.measure("888").width,
            "tabular digits give every digit the same cell"
        );
        assert_eq!(tabular.measure("102").width, tabular.measure("889").width);
    }

    #[test]
    fn a_codepoint_with_no_glyph_still_draws_notdef_and_advances() {
        let font = Font::embedded();
        let style = TextStyle::new(&font).size(24.0);
        let missing = '\u{E000}'; // a private-use codepoint no font maps
        assert!(font.glyph_index(missing).is_none(), "test needs a real gap");

        let alone = style.measure(&missing.to_string());
        assert!(alone.width > 0.0, ".notdef still advances the pen");

        let mut fb = Framebuffer::new(
            alone.width.ceil() as usize + 4,
            alone.height.ceil() as usize + 4,
        );
        fb.clear(Color::BLACK);
        style.draw(&mut fb, &missing.to_string(), 2, 2);
        assert!(
            lit_pixels(&fb) > 0,
            ".notdef is a visible box, not a silent gap"
        );
    }

    #[test]
    fn alignment_shifts_the_line_within_the_wrap_width() {
        let font = Font::embedded();
        let wrap = 100.0;
        let base = TextStyle::new(&font)
            .size(20.0)
            .color(Color::WHITE)
            .wrap_width(wrap);

        let leftmost_lit_x = |fb: &Framebuffer| -> Option<usize> {
            (0..fb.width()).find(|&x| (0..fb.height()).any(|y| fb.get_pixel(x, y) != Color::BLACK))
        };

        let mut left_x = None;
        let mut center_x = None;
        let mut right_x = None;
        for (align, slot) in [
            (Align::Left, &mut left_x),
            (Align::Center, &mut center_x),
            (Align::Right, &mut right_x),
        ] {
            let mut fb = Framebuffer::new(wrap as usize + 20, 40);
            fb.clear(Color::BLACK);
            base.align(align).draw(&mut fb, "a", 10, 5);
            *slot = leftmost_lit_x(&fb);
        }

        let (left_x, center_x, right_x) = (left_x.unwrap(), center_x.unwrap(), right_x.unwrap());
        assert!(left_x < center_x, "centering must move the glyph right");
        assert!(
            center_x < right_x,
            "right alignment must move it further still"
        );
    }

    #[test]
    fn shadow_and_outline_extend_at_most_a_pixel_past_the_plain_measure() {
        let font = Font::embedded();
        let plain = TextStyle::new(&font).size(28.0).color(Color::WHITE);
        let size = plain.measure("Q");
        let pad = 3;
        let dims = (
            size.width.ceil() as usize + pad * 2,
            size.height.ceil() as usize + pad * 2,
        );

        for style in [
            plain.shadow(Color::rgb(0.1, 0.1, 0.1)),
            plain.outline(Color::BLACK),
        ] {
            let mut fb = Framebuffer::new(dims.0, dims.1);
            fb.clear(Color::BLACK);
            style.draw(&mut fb, "Q", pad as i32, pad as i32);
            for y in 0..fb.height() {
                for x in 0..fb.width() {
                    let inside = x + 1 >= pad
                        && y + 1 >= pad
                        && (x as f32) < pad as f32 + size.width + 1.0
                        && (y as f32) < pad as f32 + size.height + 1.0;
                    if !inside {
                        assert_eq!(fb.get_pixel(x, y), Color::BLACK, "at ({x}, {y})");
                    }
                }
            }
        }
    }

    #[test]
    fn contrast_default_matches_the_alpha_blend_formula_exactly() {
        let font = Font::embedded();
        let size = 48.0;
        let style = TextStyle::new(&font)
            .size(size)
            .color(Color::rgba(1.0, 1.0, 1.0, 0.6));
        let glyph = font.glyph_index('O').unwrap();
        let bitmap = font.rasterize(glyph, size).unwrap();
        // Somewhere along the ring the coverage is neither 0 nor 255.
        let (mut col, mut row) = (0, 0);
        let mut found = false;
        'search: for r in 0..bitmap.height {
            for c in 0..bitmap.width {
                if (1..255).contains(&bitmap.coverage_at(c, r)) {
                    col = c;
                    row = r;
                    found = true;
                    break 'search;
                }
            }
        }
        assert!(found, "an antialiased edge exists to check");

        let margin = 8i32;
        let mut fb = Framebuffer::new(
            bitmap.width + margin as usize * 2,
            bitmap.height + margin as usize * 2,
        );
        fb.clear(Color::BLACK);
        style.draw(&mut fb, "O", margin, margin);

        // Reproduce `blit_bitmap`'s own placement math to find the pixel.
        let baseline_y = margin as f32 + font.ascent(size);
        let top = baseline_y.round() as i32 - bitmap.top;
        let left = margin + bitmap.left;
        let (px, py) = ((left + col as i32) as usize, (top + row as i32) as usize);

        let coverage = bitmap.coverage_at(col, row);
        let expected_alpha = coverage as f32 / 255.0 * 0.6;
        let expected = Color::from_argb8(
            Color::BLACK
                .blend_over(style.color, expected_alpha)
                .to_argb8(),
        );
        assert_eq!(fb.get_pixel(px, py), expected);
    }

    #[test]
    fn panel_covers_exactly_the_measured_box() {
        let font = Font::embedded();
        let style = TextStyle::new(&font)
            .size(16.0)
            .color(Color::WHITE)
            .panel(Color::rgba(1.0, 0.0, 0.0, 1.0));
        let size = style.measure("Hi");
        let pad = 3;
        let mut fb = Framebuffer::new(
            size.width.ceil() as usize + pad * 2,
            size.height.ceil() as usize + pad * 2,
        );
        fb.clear(Color::BLACK);
        style.draw(&mut fb, "Hi", pad as i32, pad as i32);
        // Corner of the panel, away from any glyph ink, is exactly the panel color.
        assert_eq!(fb.get_pixel(pad, pad), Color::RED);
        // Outside the panel stays untouched.
        assert_eq!(fb.get_pixel(0, 0), Color::BLACK);
    }

    #[test]
    fn a_golden_sanity_check_renders_without_panicking() {
        // The real showcase lives in tests/golden_text.rs; this just proves
        // the module survives a mixed style through `golden::check` without
        // touching a committed reference.
        let font = Font::embedded();
        let style = TextStyle::new(&font)
            .size(18.0)
            .color(Color::WHITE)
            .shadow(Color::BLACK)
            .wrap_width(120.0)
            .align(Align::Center);
        let mut fb = Framebuffer::new(140, 80);
        fb.clear(Color::rgb(0.1, 0.1, 0.12));
        style.draw(
            &mut fb,
            "A short paragraph that wraps onto more than one line.",
            4,
            4,
        );
        let dir = std::env::temp_dir().join("runity-text-sanity");
        let _ = std::fs::create_dir_all(&dir);
        let _ = golden::check(dir.join("sanity.png"), &fb, Tolerance::default());
    }
}
