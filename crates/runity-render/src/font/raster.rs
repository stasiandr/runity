//! Contours to coverage: the glyph rasterizer.
//!
//! The outline arrives in font units with y up; a pixel bitmap wants y down and
//! a size, so every point goes through `size / unitsPerEm` and a sign flip
//! first. Curves are then flattened into lines — adaptively, by how far the
//! curve bends away from its chord, so a letter at 8 px costs a fraction of
//! what the same letter costs at 200 px.
//!
//! Filling is signed-area accumulation rather than scanline crossings. Each
//! line segment drops two numbers into every cell of the grid it touches: how
//! much of the cell it covers, and how much coverage everything to its right
//! inherits. One prefix sum along each row then turns that into finished
//! coverage, with no sorting of edges and no per-pixel inside/outside test. The
//! fill rule is the non-zero one, which falls out of clamping the magnitude of
//! the running sum to one: a contour drawn twice in the same direction is still
//! ink, a counter-drawn one is a hole.
//!
//! There is no hinting here and there will not be — it is a bytecode
//! interpreter and a whole second project. What replaces it is
//! [`contrast`]: a gamma curve on coverage that pulls thin stems back up to
//! something readable without moving an edge or softening one.

use runity_math::Vec2;

use super::outline::{Outline, Segment};

/// How far, in pixels, a flattened chord may stray from its curve.
///
/// A quarter of this is invisible and costs segments; twice it shows up on a
/// large `o` as a faceted edge.
const FLATNESS: f32 = 0.08;

/// The most line segments one quadratic is ever cut into.
const MAX_STEPS: usize = 64;

/// The gamma coverage is bent through; 1.0 would be no correction at all.
///
/// Larger numbers thicken thin stems and, past about 1.6, start to look like a
/// bolder font rather than a better-lit one.
const CONTRAST_GAMMA: f32 = 1.35;

/// The largest bitmap one glyph may rasterize into, per axis.
///
/// A sane font at a sane size never comes near it; a broken one asking for a
/// sixteen-thousand-pixel glyph gets cropped instead of an allocation failure.
const MAX_BITMAP: usize = 2048;

/// One glyph, rasterized at one size: 8-bit coverage and where to put it.
///
/// `coverage` is `width * height` bytes, row-major, the top row first, 0 for a
/// pixel the glyph misses and 255 for one it fills. It is not a color: the
/// caller decides what to blend with it.
///
/// The placement is the usual one. With the pen at `(x, y)` on the baseline,
/// the top-left pixel of the bitmap belongs at `(x + left, y - top)`, and the
/// pen then moves on by `advance`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GlyphBitmap {
    /// Pixels from the pen to the left edge of the bitmap, positive rightwards.
    pub left: i32,
    /// Pixels from the baseline to the top edge of the bitmap, positive upwards.
    pub top: i32,
    /// Width of the coverage bitmap in pixels.
    pub width: usize,
    /// Height of the coverage bitmap in pixels.
    pub height: usize,
    /// `width * height` coverage bytes, row-major, top row first.
    pub coverage: Vec<u8>,
    /// How far the pen moves after this glyph, in pixels.
    pub advance: f32,
}

impl GlyphBitmap {
    /// True for a glyph with no pixels at all — a space, or a glyph whose
    /// outline is thinner than the grid can catch.
    pub fn is_empty(&self) -> bool {
        self.coverage.is_empty()
    }

    /// Coverage at a pixel of the bitmap; 0 outside it.
    pub fn coverage_at(&self, x: usize, y: usize) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.coverage[y * self.width + x]
    }

    /// Roughly how much memory this bitmap holds — what a cache counts.
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<GlyphBitmap>() + self.coverage.capacity()
    }
}

/// Rasterize an outline given in font units at `scale` pixels per unit.
///
/// `advance` is passed through to [`GlyphBitmap::advance`]; this function does
/// not know the font it came from.
pub(crate) fn rasterize(outline: &Outline, scale: f32, advance: f32) -> GlyphBitmap {
    let empty = GlyphBitmap {
        advance,
        ..GlyphBitmap::default()
    };
    if !scale.is_finite() || scale <= 0.0 {
        return empty;
    }

    // Flatten first, in pixels: the exact bounds of the ink are the bounds of
    // the lines, and the control points that bulge past them are gone by then.
    let lines = flatten(outline, scale);
    let Some((min, max)) = bounds(&lines) else {
        return empty;
    };

    let left = min.x.floor();
    let top = min.y.floor();
    let width = ((max.x.ceil() - left) as usize).min(MAX_BITMAP);
    let height = ((max.y.ceil() - top) as usize).min(MAX_BITMAP);
    if width == 0 || height == 0 {
        return empty;
    }

    let origin = Vec2::new(left, top);
    let mut cells = Cells::new(width, height);
    for &(from, to) in &lines {
        cells.line(from - origin, to - origin);
    }

    GlyphBitmap {
        left: left as i32,
        // y grows downwards in the bitmap and upwards from the baseline.
        top: -(top as i32),
        width,
        height,
        coverage: cells.finish(),
        advance,
    }
}

/// Every contour as line segments in pixel space, y down from the baseline.
fn flatten(outline: &Outline, scale: f32) -> Vec<(Vec2, Vec2)> {
    let to_pixels = |p: Vec2| Vec2::new(p.x * scale, -p.y * scale);
    let mut lines = Vec::new();
    for contour in &outline.contours {
        let mut from = to_pixels(contour.start);
        for segment in &contour.segments {
            match *segment {
                Segment::Line(end) => {
                    let end = to_pixels(end);
                    lines.push((from, end));
                    from = end;
                }
                Segment::Quad(control, end) => {
                    let (control, end) = (to_pixels(control), to_pixels(end));
                    flatten_quad(from, control, end, &mut lines);
                    from = end;
                }
            }
        }
    }
    lines.retain(|(from, to)| from.y != to.y); // a horizontal line covers nothing
    lines
}

/// Cut one quadratic into as many chords as its own bend asks for.
fn flatten_quad(from: Vec2, control: Vec2, to: Vec2, lines: &mut Vec<(Vec2, Vec2)>) {
    // Cutting a quadratic into `n` pieces leaves each chord at most
    // `|p0 - 2c + p1| / (8 n^2)` away from the curve, so the count comes from
    // the curve rather than from a fixed guess.
    let bend = (from - control * 2.0 + to).length();
    let steps = ((bend / (8.0 * FLATNESS)).sqrt().ceil() as usize).clamp(1, MAX_STEPS);
    let mut previous = from;
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let point = if step == steps {
            to // the last point is the one the next segment starts from
        } else {
            let one = 1.0 - t;
            from * (one * one) + control * (2.0 * one * t) + to * (t * t)
        };
        lines.push((previous, point));
        previous = point;
    }
}

/// The box holding every endpoint, or `None` if there are none.
fn bounds(lines: &[(Vec2, Vec2)]) -> Option<(Vec2, Vec2)> {
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for &(from, to) in lines {
        for p in [from, to] {
            if !p.x.is_finite() || !p.y.is_finite() {
                return None;
            }
            min = Vec2::new(min.x.min(p.x), min.y.min(p.y));
            max = Vec2::new(max.x.max(p.x), max.y.max(p.y));
        }
    }
    (min.x <= max.x).then_some((min, max))
}

/// The accumulation grid: signed area per cell, waiting for its prefix sum.
struct Cells {
    width: usize,
    height: usize,
    /// `width + 2` per row: a cell can spill one column right, and an edge
    /// clamped to the right border lands one past the last visible column.
    stride: usize,
    area: Vec<f32>,
}

impl Cells {
    fn new(width: usize, height: usize) -> Cells {
        let stride = width + 2;
        Cells {
            width,
            height,
            stride,
            area: vec![0.0; stride * height],
        }
    }

    /// Accumulate one line segment, in bitmap pixels.
    fn line(&mut self, from: Vec2, to: Vec2) {
        if !(from.x.is_finite() && from.y.is_finite() && to.x.is_finite() && to.y.is_finite()) {
            return;
        }
        // Which way the edge runs is the sign of everything it contributes;
        // that sign is what makes the fill rule non-zero rather than even-odd.
        let (direction, top, bottom) = if from.y < to.y {
            (1.0, from, to)
        } else if to.y < from.y {
            (-1.0, to, from)
        } else {
            return;
        };

        let dxdy = (bottom.x - top.x) / (bottom.y - top.y);
        let first = top.y.floor().max(0.0) as usize;
        let last = bottom.y.ceil().clamp(0.0, self.height as f32) as usize;
        for y in first..last {
            let row_top = (y as f32).max(top.y);
            let row_bottom = ((y + 1) as f32).min(bottom.y);
            if row_bottom <= row_top {
                continue;
            }
            let x0 = top.x + (row_top - top.y) * dxdy;
            let x1 = top.x + (row_bottom - top.y) * dxdy;
            self.span(y, x0, x1, (row_bottom - row_top) * direction);
        }
    }

    /// One row's worth of an edge: `height` of signed coverage, spread over the
    /// cells the edge crosses between `xa` and `xb`.
    fn span(&mut self, y: usize, xa: f32, xb: f32, height: f32) {
        let border = self.width as f32;
        // An edge left of the bitmap covers every cell of the row, one right of
        // it covers none; clamping to the borders says exactly that.
        let low = xa.min(xb).clamp(0.0, border);
        let high = xa.max(xb).clamp(0.0, border);
        let first = low as usize;
        let last = high as usize;

        if first == last {
            self.cell(y, first, height, 0.5 * (low + high) - first as f32);
            return;
        }
        // The edge is a straight line, so the share of its height spent in a
        // cell is the share of its width that falls inside that cell.
        let per_x = (high - low).recip();
        for cell in first..=last {
            let from = low.max(cell as f32);
            let to = high.min((cell + 1) as f32);
            if to <= from {
                continue;
            }
            let share = height * (to - from) * per_x;
            self.cell(y, cell, share, 0.5 * (from + to) - cell as f32);
        }
    }

    /// Put one trapezoid into a cell: the part of the cell right of the edge
    /// belongs to the cell itself, the rest passes on to the cell after it.
    fn cell(&mut self, y: usize, x: usize, height: f32, middle: f32) {
        let at = y * self.stride + x;
        self.area[at] += height * (1.0 - middle);
        self.area[at + 1] += height * middle;
    }

    /// Prefix sum each row, clamp, and bend the result through [`contrast`].
    fn finish(&self) -> Vec<u8> {
        let mut coverage = vec![0u8; self.width * self.height];
        for y in 0..self.height {
            let mut winding = 0.0f32;
            for x in 0..self.width {
                winding += self.area[y * self.stride + x];
                let inside = winding.abs().min(1.0);
                coverage[y * self.width + x] = (contrast(inside) * 255.0 + 0.5) as u8;
            }
        }
        coverage
    }
}

/// The stand-in for hinting: a gamma curve on coverage.
///
/// Empty and full stay exactly where they are, so the inside of a glyph is 255
/// and the outside 0; everything between is lifted, which is what makes a stem
/// half a pixel wide look like a stem instead of a grey smear.
fn contrast(coverage: f32) -> f32 {
    coverage.clamp(0.0, 1.0).powf(1.0 / CONTRAST_GAMMA)
}

#[cfg(test)]
mod tests {
    use super::super::outline::Contour;
    use super::*;

    /// A closed contour through the points given, in font units.
    fn contour(points: &[(f32, f32)]) -> Contour {
        let point = |&(x, y): &(f32, f32)| Vec2::new(x, y);
        Contour {
            start: point(&points[0]),
            segments: points[1..]
                .iter()
                .map(|p| Segment::Line(point(p)))
                .chain(std::iter::once(Segment::Line(point(&points[0]))))
                .collect(),
        }
    }

    /// An axis-aligned box, counter-clockwise in font units (y up).
    fn box_outline(x0: f32, y0: f32, x1: f32, y1: f32) -> Outline {
        Outline {
            contours: vec![contour(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])],
        }
    }

    fn coverage_sum(bitmap: &GlyphBitmap) -> u64 {
        bitmap.coverage.iter().map(|&c| c as u64).sum()
    }

    #[test]
    fn a_whole_pixel_box_is_solid_inside_and_empty_outside() {
        // Ten units at a tenth of a pixel each: a 10x10 box of whole pixels.
        let bitmap = rasterize(&box_outline(0.0, 0.0, 100.0, 100.0), 0.1, 12.0);
        assert_eq!((bitmap.width, bitmap.height), (10, 10));
        assert_eq!((bitmap.left, bitmap.top), (0, 10));
        assert_eq!(bitmap.advance, 12.0);
        assert!(
            bitmap.coverage.iter().all(|&c| c == 255),
            "all of it is ink"
        );
    }

    #[test]
    fn coverage_is_high_inside_a_contour_and_low_outside_it() {
        // A 24 x 24 pixel box with a counter-drawn 20 x 20 one inside it: a
        // two-pixel ring of ink with empty pixels on both sides of it.
        let outline = Outline {
            contours: vec![
                contour(&[(0.0, 0.0), (240.0, 0.0), (240.0, 240.0), (0.0, 240.0)]),
                contour(&[(20.0, 20.0), (20.0, 220.0), (220.0, 220.0), (220.0, 20.0)]),
            ],
        };
        let bitmap = rasterize(&outline, 0.1, 0.0);
        assert_eq!((bitmap.width, bitmap.height), (24, 24));

        // The ring between the two contours is solid, the hole inside the
        // counter-drawn one is empty, and so is everything past the outer edge.
        assert_eq!(bitmap.coverage_at(0, 0), 255, "inside the outer contour");
        assert_eq!(bitmap.coverage_at(12, 1), 255);
        assert_eq!(bitmap.coverage_at(12, 12), 0, "the hole the inner one cuts");
        assert_eq!(bitmap.coverage_at(3, 3), 0);
        assert_eq!(bitmap.coverage_at(30, 30), 0, "past the bitmap entirely");
    }

    #[test]
    fn two_contours_wound_the_same_way_stay_filled() {
        // The non-zero rule: winding 2 is as solid as winding 1, where the
        // even-odd rule would punch a hole here.
        let mut outline = box_outline(0.0, 0.0, 100.0, 100.0);
        outline.contours.push(contour(&[
            (20.0, 20.0),
            (80.0, 20.0),
            (80.0, 80.0),
            (20.0, 80.0),
        ]));
        let bitmap = rasterize(&outline, 0.1, 0.0);
        assert!(
            bitmap.coverage.iter().all(|&c| c == 255),
            "a contour drawn twice the same way is still ink"
        );
    }

    #[test]
    fn a_half_covered_pixel_comes_out_half_covered() {
        // Half a pixel of ink, with the gamma taken back off: still a half.
        let bitmap = rasterize(&box_outline(0.0, 0.0, 5.0, 10.0), 0.1, 0.0);
        assert_eq!((bitmap.width, bitmap.height), (1, 1));
        let linear = (bitmap.coverage[0] as f32 / 255.0).powf(CONTRAST_GAMMA);
        assert!(
            (linear - 0.5).abs() < 0.02,
            "half a pixel of ink read as {linear}"
        );
    }

    #[test]
    fn contrast_keeps_the_ends_and_lifts_the_middle() {
        assert_eq!(contrast(0.0), 0.0);
        assert_eq!(contrast(1.0), 1.0);
        assert!(contrast(0.25) > 0.25, "thin stems come up");
        assert!(contrast(0.75) > 0.75);
        assert!(contrast(0.5) < 1.0);
        // Monotonic, so an edge stays an edge rather than a ripple.
        let mut previous = 0.0;
        for step in 0..=32 {
            let value = contrast(step as f32 / 32.0);
            assert!(value >= previous, "contrast went backwards at {step}");
            previous = value;
        }
    }

    #[test]
    fn doubling_the_size_quadruples_the_coverage() {
        // A shape with a curve in it, so the flattening is in the measurement
        // too: a lens made of two quadratics.
        let outline = Outline {
            contours: vec![Contour {
                start: Vec2::new(0.0, 0.0),
                segments: vec![
                    Segment::Quad(Vec2::new(150.0, 400.0), Vec2::new(300.0, 0.0)),
                    Segment::Quad(Vec2::new(150.0, -400.0), Vec2::new(0.0, 0.0)),
                ],
            }],
        };
        let small = rasterize(&outline, 0.05, 0.0);
        let large = rasterize(&outline, 0.1, 0.0);
        assert_eq!(large.width, small.width * 2);

        let ratio = coverage_sum(&large) as f32 / coverage_sum(&small) as f32;
        assert!(
            (3.6..4.4).contains(&ratio),
            "twice the size is four times the ink, not {ratio}"
        );
    }

    #[test]
    fn a_curve_is_cut_into_more_pieces_the_bigger_it_gets() {
        let curve = Outline {
            contours: vec![Contour {
                start: Vec2::new(0.0, 0.0),
                segments: vec![
                    Segment::Quad(Vec2::new(500.0, 1000.0), Vec2::new(1000.0, 0.0)),
                    Segment::Line(Vec2::new(0.0, 0.0)),
                ],
            }],
        };
        let small = flatten(&curve, 0.02).len();
        let large = flatten(&curve, 0.5).len();
        assert!(small >= 2, "even a tiny curve is more than a chord");
        assert!(
            large > small * 3,
            "{large} pieces at 25 times the size against {small}"
        );
        assert!(large <= MAX_STEPS + 1, "and it stops somewhere");
    }

    #[test]
    fn a_round_shape_covers_about_what_its_area_says() {
        // Four quadratic arcs, each bulging out of a corner of the diamond
        // through (±r, 0) and (0, ±r). A quadratic cuts off two thirds of the
        // triangle over its chord, so the area is exactly 2r² + 4·(r²/3) —
        // which catches a fill rule that leaks and a flattening that corners.
        let r = 100.0;
        let circle = Outline {
            contours: vec![Contour {
                start: Vec2::new(r, 0.0),
                segments: vec![
                    Segment::Quad(Vec2::new(r, r), Vec2::new(0.0, r)),
                    Segment::Quad(Vec2::new(-r, r), Vec2::new(-r, 0.0)),
                    Segment::Quad(Vec2::new(-r, -r), Vec2::new(0.0, -r)),
                    Segment::Quad(Vec2::new(r, -r), Vec2::new(r, 0.0)),
                ],
            }],
        };
        let bitmap = rasterize(&circle, 0.5, 0.0);
        let pixels = r * 0.5;
        let expected = 10.0 / 3.0 * pixels * pixels;
        let measured = coverage_sum(&bitmap) as f32 / 255.0;
        assert!(
            (measured / expected - 1.0).abs() < 0.06,
            "{measured} pixels of ink against an area of {expected}"
        );
    }

    #[test]
    fn an_outline_off_the_left_of_the_bitmap_still_fills_the_rest() {
        // The bitmap starts at the outline's own bounds, so this only happens
        // when a caller asks for one; the cell clamp is what keeps it sane.
        let mut cells = Cells::new(4, 1);
        cells.line(Vec2::new(-10.0, 0.0), Vec2::new(-10.0, 1.0));
        cells.line(Vec2::new(2.5, 1.0), Vec2::new(2.5, 0.0));
        let coverage = cells.finish();
        assert_eq!(coverage[0], 255);
        assert_eq!(coverage[1], 255);
        assert!(
            coverage[2] > 100 && coverage[2] < 160,
            "the half-covered one"
        );
        assert_eq!(coverage[3], 0);
    }

    #[test]
    fn an_outline_off_the_right_of_the_bitmap_is_dropped_and_not_wrapped() {
        let mut cells = Cells::new(4, 1);
        cells.line(Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0));
        cells.line(Vec2::new(99.0, 1.0), Vec2::new(99.0, 0.0));
        let coverage = cells.finish();
        assert_eq!(coverage, vec![0, 255, 255, 255]);
    }

    #[test]
    fn an_empty_or_impossible_outline_gives_an_empty_bitmap() {
        let empty = rasterize(&Outline::default(), 0.1, 7.0);
        assert!(empty.is_empty());
        assert_eq!(empty.advance, 7.0, "a space still moves the pen");
        assert_eq!(empty.coverage_at(0, 0), 0);

        let square = box_outline(0.0, 0.0, 100.0, 100.0);
        assert!(rasterize(&square, 0.0, 0.0).is_empty());
        assert!(rasterize(&square, -1.0, 0.0).is_empty());
        assert!(rasterize(&square, f32::NAN, 0.0).is_empty());
        assert!(rasterize(&square, f32::INFINITY, 0.0).is_empty());

        // A contour with no area at all: a single point, and a flat line.
        let degenerate = Outline {
            contours: vec![
                contour(&[(10.0, 10.0), (10.0, 10.0)]),
                contour(&[(0.0, 0.0), (100.0, 0.0)]),
            ],
        };
        assert!(rasterize(&degenerate, 0.1, 0.0).is_empty());
    }

    #[test]
    fn a_glyph_bigger_than_the_cap_is_cropped_and_not_allocated() {
        // 10000 units at one pixel each would be a hundred megapixels.
        let huge = box_outline(0.0, 0.0, 10_000.0, 10_000.0);
        let bitmap = rasterize(&huge, 1.0, 0.0);
        assert_eq!((bitmap.width, bitmap.height), (MAX_BITMAP, MAX_BITMAP));
        assert_eq!(bitmap.coverage.len(), MAX_BITMAP * MAX_BITMAP);
    }
}
