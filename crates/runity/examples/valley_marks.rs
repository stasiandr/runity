//! One-off generator for Valley's mark atlas: thirty owner's marks, the kind a
//! settler might carve into a fence post or brand into a barrel, drawn by rule
//! and baked once into `assets/valley/textures/marks.png`.
//!
//! ```text
//! cargo run --example valley_marks
//! ```
//!
//! Run it once, commit the PNG, and forget it: the file that ships is an
//! ordinary asset from then on, free to be touched up by hand. Re-running
//! this generator reproduces today's file byte for byte (see the tests
//! below) — it is not meant to run on every build.

use runity::prelude::*;

/// Pixel size of one mark's cell.
const CELL: usize = 32;
const COLS: usize = 8;
const ROWS: usize = 4;
/// Cells the atlas actually draws into; the remaining `COLS * ROWS - MARK_COUNT`
/// cells stay fully transparent.
const MARK_COUNT: usize = 30;
/// How many stroke families [`draw_mark`] cycles through.
const FAMILIES: usize = 5;

/// Distance from a cell's edge to its first grid line, and the grid's pitch —
/// five lines spaced six pixels apart center a 5x5 grid in a 32-pixel cell.
const MARGIN: f32 = 4.0;
const STEP: f32 = 6.0;

const INK: Color = Color::rgb(0.14, 0.10, 0.07);

/// A point on the mark's 5x5 grid (columns and rows both `0..5`), in a cell
/// whose top-left pixel is `origin`.
fn grid_point(origin: (f32, f32), col: usize, row: usize) -> (f32, f32) {
    (
        origin.0 + MARGIN + col as f32 * STEP,
        origin.1 + MARGIN + row as f32 * STEP,
    )
}

/// A hard-edged line with a little bit of body to it — three 1-pixel passes
/// offset by a pixel each, rather than a single hairline.
fn thick_line(frame: &mut Framebuffer, from: (f32, f32), to: (f32, f32), color: Color) {
    debug::draw_line(frame, from, to, color);
    debug::draw_line(frame, (from.0 + 1.0, from.1), (to.0 + 1.0, to.1), color);
    debug::draw_line(frame, (from.0, from.1 + 1.0), (to.0, to.1 + 1.0), color);
}

/// Filled disc, for the dot family — a small square selection would read as a
/// pixel, not a dot.
fn fill_circle(frame: &mut Framebuffer, center: (f32, f32), radius: f32, color: Color) {
    let (cx, cy) = center;
    let r = radius.ceil() as i32;
    let x0 = (cx.round() as i32 - r).max(0);
    let x1 = cx.round() as i32 + r;
    let y0 = (cy.round() as i32 - r).max(0);
    let y1 = cy.round() as i32 + r;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            if dx * dx + dy * dy <= radius * radius {
                frame.set_pixel(x as usize, y as usize, color);
            }
        }
    }
}

/// Filled triangle (flat scanline fill; good enough for a 24x24 icon).
fn fill_triangle(
    frame: &mut Framebuffer,
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    color: Color,
) {
    let mut points = [a, b, c];
    points.sort_by(|p, q| p.1.partial_cmp(&q.1).unwrap());
    let [top, mid, bottom] = points;

    let edge_x = |p0: (f32, f32), p1: (f32, f32), y: f32| -> f32 {
        if (p1.1 - p0.1).abs() < 1e-6 {
            p0.0
        } else {
            p0.0 + (p1.0 - p0.0) * (y - p0.1) / (p1.1 - p0.1)
        }
    };

    let y0 = top.1.round() as i32;
    let y1 = bottom.1.round() as i32;
    for y in y0..=y1 {
        if y < 0 {
            continue;
        }
        let yf = y as f32;
        let long_edge = edge_x(top, bottom, yf);
        let short_edge = if yf < mid.1 {
            edge_x(top, mid, yf)
        } else {
            edge_x(mid, bottom, yf)
        };
        let (left, right) = if long_edge <= short_edge {
            (long_edge, short_edge)
        } else {
            (short_edge, long_edge)
        };
        let x0 = left.round() as i32;
        let x1 = right.round() as i32;
        for x in x0.max(0)..=x1 {
            frame.set_pixel(x as usize, y as usize, color);
        }
    }
}

// ---------------------------------------------------------------------------
// The five stroke families
// ---------------------------------------------------------------------------

/// 2-3 strokes joining points of the 5x5 grid — a tamga-style personal mark,
/// six hand-drawn variants.
fn draw_strokes(frame: &mut Framebuffer, origin: (f32, f32), variant: usize) {
    let g = |col: usize, row: usize| grid_point(origin, col, row);
    let segments: [((usize, usize), (usize, usize)); 3] = match variant {
        0 => [((2, 0), (2, 4)), ((2, 1), (4, 0)), ((2, 3), (4, 4))],
        1 => [((2, 0), (2, 4)), ((2, 1), (0, 0)), ((2, 3), (0, 4))],
        2 => [((0, 2), (4, 2)), ((1, 2), (1, 0)), ((3, 2), (3, 4))],
        3 => [((0, 0), (2, 4)), ((4, 0), (2, 4)), ((2, 4), (2, 2))],
        4 => [((0, 0), (2, 2)), ((2, 2), (0, 4)), ((2, 2), (4, 2))],
        _ => [((0, 0), (4, 0)), ((2, 0), (2, 4)), ((0, 4), (4, 4))],
    };
    for (from, to) in segments {
        thick_line(frame, g(from.0, from.1), g(to.0, to.1), INK);
    }
}

/// A stem with `1 + variant` notches — a tally cut into a fence post.
fn draw_notches(frame: &mut Framebuffer, origin: (f32, f32), variant: usize) {
    let top = grid_point(origin, 2, 0);
    let bottom = grid_point(origin, 2, 4);
    thick_line(frame, top, bottom, INK);

    let count = 1 + variant;
    for i in 0..count {
        let t = (i as f32 + 1.0) / (count as f32 + 1.0);
        let y = top.1 + (bottom.1 - top.1) * t;
        let side = if i % 2 == 0 { 1.0 } else { -1.0 };
        thick_line(frame, (top.0, y), (top.0 + side * 7.0, y), INK);
    }
}

/// Six cross variants: plain, X, and crossbar high, low, offset, or flagged.
fn draw_cross(frame: &mut Framebuffer, origin: (f32, f32), variant: usize) {
    let g = |col: usize, row: usize| grid_point(origin, col, row);
    match variant {
        0 => {
            thick_line(frame, g(2, 0), g(2, 4), INK);
            thick_line(frame, g(0, 2), g(4, 2), INK);
        }
        1 => {
            thick_line(frame, g(0, 0), g(4, 4), INK);
            thick_line(frame, g(0, 4), g(4, 0), INK);
        }
        2 => {
            thick_line(frame, g(2, 0), g(2, 4), INK);
            thick_line(frame, g(0, 1), g(4, 1), INK);
        }
        3 => {
            thick_line(frame, g(2, 0), g(2, 4), INK);
            thick_line(frame, g(0, 3), g(4, 3), INK);
        }
        4 => {
            thick_line(frame, g(0, 0), g(3, 3), INK);
            thick_line(frame, g(1, 4), g(4, 1), INK);
        }
        _ => {
            // A cross with a serif at each arm's end ("cross potent"), not
            // just the plain plus with one small addition.
            thick_line(frame, g(2, 0), g(2, 4), INK);
            thick_line(frame, g(0, 2), g(4, 2), INK);
            thick_line(frame, g(1, 0), g(3, 0), INK);
            thick_line(frame, g(1, 4), g(3, 4), INK);
            thick_line(frame, g(0, 1), g(0, 3), INK);
            thick_line(frame, g(4, 1), g(4, 3), INK);
        }
    }
}

/// A pine-like silhouette: `1 + variant % 3` canopy tiers, narrow or wide
/// depending on `variant / 3` — a scale, not a one-pixel trunk difference, so
/// the two halves of the family are easy to tell apart at a glance.
fn draw_tree(frame: &mut Framebuffer, origin: (f32, f32), variant: usize) {
    let trunk_x = origin.0 + 16.0;
    let tiers = 1 + variant % 3;
    let width_scale = if variant / 3 >= 1 { 1.7 } else { 1.0 };
    let canopy_top = origin.1 + 4.0;
    let canopy_bottom = origin.1 + 22.0;
    let trunk_bottom = origin.1 + 28.0;

    thick_line(
        frame,
        (trunk_x, canopy_bottom - 2.0),
        (trunk_x, trunk_bottom),
        INK,
    );

    let tier_span = (canopy_bottom - canopy_top) / tiers as f32;
    for tier in 0..tiers {
        let apex_y = canopy_top + tier as f32 * tier_span * 0.65;
        let base_y = (apex_y + tier_span * 1.15).min(canopy_bottom);
        let half_width = (5.0 + tier as f32 * 3.0) * width_scale;
        fill_triangle(
            frame,
            (trunk_x, apex_y),
            (trunk_x - half_width, base_y),
            (trunk_x + half_width, base_y),
            INK,
        );
    }
}

/// Dots stamped on the 5x5 grid: corners, a plus, both diagonals, the border,
/// or a checkerboard — six patterns with very different dot counts.
fn draw_dots(frame: &mut Framebuffer, origin: (f32, f32), variant: usize) {
    let points: Vec<(usize, usize)> = match variant {
        0 => vec![(0, 0), (4, 0), (0, 4), (4, 4)],
        1 => vec![
            (2, 0),
            (2, 1),
            (2, 2),
            (2, 3),
            (2, 4),
            (0, 2),
            (1, 2),
            (3, 2),
            (4, 2),
        ],
        2 => vec![(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)],
        3 => vec![(0, 4), (1, 3), (2, 2), (3, 1), (4, 0)],
        4 => (0..5)
            .flat_map(|i| [(i, 0), (i, 4)])
            .chain((1..4).flat_map(|i| [(0, i), (4, i)]))
            .collect(),
        _ => (0..5)
            .flat_map(|row| {
                (0..5)
                    .filter(move |col| (col + row) % 2 == 0)
                    .map(move |col| (col, row))
            })
            .collect(),
    };
    for (col, row) in points {
        fill_circle(frame, grid_point(origin, col, row), 1.6, INK);
    }
}

/// Which family owns `index`, and which of its six variants.
fn family_and_variant(index: usize) -> (usize, usize) {
    (index % FAMILIES, index / FAMILIES)
}

/// Draw mark `index` into the cell whose top-left pixel is `origin`.
fn draw_mark(frame: &mut Framebuffer, origin: (f32, f32), index: usize) {
    let (family, variant) = family_and_variant(index);
    match family {
        0 => draw_strokes(frame, origin, variant),
        1 => draw_notches(frame, origin, variant),
        2 => draw_cross(frame, origin, variant),
        3 => draw_tree(frame, origin, variant),
        _ => draw_dots(frame, origin, variant),
    }
}

/// The whole atlas: `COLS * ROWS` cells of `CELL` pixels, the first
/// [`MARK_COUNT`] of them drawn, the rest left transparent.
fn generate_atlas() -> Framebuffer {
    let mut frame = Framebuffer::new(CELL * COLS, CELL * ROWS);
    frame.clear(Color::TRANSPARENT);
    for index in 0..MARK_COUNT {
        let col = index % COLS;
        let row = index / COLS;
        let origin = ((col * CELL) as f32, (row * CELL) as f32);
        draw_mark(&mut frame, origin, index);
    }
    frame
}

/// `assets/valley/textures/marks.png`, found relative to this file rather
/// than the process's current directory.
fn asset_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/valley/textures/marks.png")
}

fn main() -> std::io::Result<()> {
    let frame = generate_atlas();
    let path = asset_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    save_png_rgba(&path, &frame)?;
    println!(
        "wrote {} ({}x{}, {} marks)",
        path.display(),
        frame.width(),
        frame.height(),
        MARK_COUNT
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed asset, embedded at compile time so these tests do not
    /// depend on the process's current directory.
    const COMMITTED: &[u8] = include_bytes!("../../../assets/valley/textures/marks.png");

    fn cell_pixels(image: &Image, index: usize) -> Vec<u32> {
        let col = index % COLS;
        let row = index / COLS;
        let mut out = Vec::with_capacity(CELL * CELL);
        for y in 0..CELL {
            for x in 0..CELL {
                let (px, py) = (col * CELL + x, row * CELL + y);
                out.push(image.pixels[py * image.width + px]);
            }
        }
        out
    }

    #[test]
    fn generating_the_atlas_twice_is_byte_identical() {
        let first = generate_atlas();
        let second = generate_atlas();
        let a = encode_png_rgba(first.width(), first.height(), first.pixels());
        let b = encode_png_rgba(second.width(), second.height(), second.pixels());
        assert_eq!(a, b, "the generator must be a pure function of nothing");
    }

    #[test]
    fn the_committed_atlas_decodes_at_the_right_size() {
        let image = decode_png(COMMITTED).expect("assets/valley/textures/marks.png decodes");
        assert_eq!((image.width, image.height), (CELL * COLS, CELL * ROWS));
        assert_eq!((image.width, image.height), (256, 128));
    }

    #[test]
    fn every_mark_actually_uses_its_alpha_channel() {
        let image = decode_png(COMMITTED).unwrap();
        for index in 0..MARK_COUNT {
            let pixels = cell_pixels(&image, index);
            let transparent = pixels.iter().any(|p| (p >> 24) == 0);
            let opaque = pixels.iter().any(|p| (p >> 24) == 0xff);
            assert!(
                transparent && opaque,
                "mark {index} should have both fully transparent background and opaque ink, \
                 not a flat fill"
            );
        }
    }

    #[test]
    fn all_thirty_marks_are_pairwise_distinguishable() {
        let image = decode_png(COMMITTED).unwrap();
        let cells: Vec<Vec<u32>> = (0..MARK_COUNT).map(|i| cell_pixels(&image, i)).collect();

        // Two marks whose cells agree everywhere but a handful of pixels
        // would be an accident, not a family/variant with a distinct shape.
        let min_differing = (CELL * CELL) / 40; // at least 2.5% of the cell

        for i in 0..MARK_COUNT {
            for j in (i + 1)..MARK_COUNT {
                let differing = cells[i]
                    .iter()
                    .zip(&cells[j])
                    .filter(|(a, b)| a != b)
                    .count();
                assert!(
                    differing >= min_differing,
                    "marks {i} and {j} differ in only {differing} of {} pixels",
                    CELL * CELL
                );
            }
        }
    }

    #[test]
    fn credits_mentions_the_generated_atlas() {
        let credits = include_str!("../../../assets/valley/CREDITS.txt");
        assert!(
            credits.contains("marks.png"),
            "CREDITS.txt should say where marks.png came from"
        );
    }
}
