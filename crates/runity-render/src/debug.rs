//! Debug views: wireframes, normals, the depth buffer and overdraw.
//!
//! This renderer's job is debugging and headless development, so seeing *why* a
//! frame looks wrong matters more than drawing it quickly. Everything here
//! draws as an overlay, ignoring the depth buffer, so it is visible on top of
//! whatever the scene produced.

use crate::color::Color;
use crate::font;
use crate::framebuffer::Framebuffer;
use crate::mesh::Mesh;
use runity_math::{Mat4, Vec3};

/// Project a clip-space position to pixel coordinates.
///
/// Returns `None` for anything at or behind the near plane, which is where the
/// perspective divide stops meaning anything.
fn project(clip: runity_math::Vec4, width: f32, height: f32) -> Option<(f32, f32)> {
    if clip.w <= 1e-6 || clip.z < 0.0 {
        return None;
    }
    let ndc = clip.perspective_divide();
    Some(((ndc.x * 0.5 + 0.5) * width, (0.5 - ndc.y * 0.5) * height))
}

/// Clip a line segment to the rectangle `[0, width] x [0, height]`
/// (Liang-Barsky), so a line that starts far off-screen costs nothing to draw.
///
/// Returns the clipped endpoints, or `None` if the segment misses entirely.
pub fn clip_segment(
    (x0, y0): (f32, f32),
    (x1, y1): (f32, f32),
    width: f32,
    height: f32,
) -> Option<((f32, f32), (f32, f32))> {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;

    for (p, q) in [(-dx, x0), (dx, width - x0), (-dy, y0), (dy, height - y0)] {
        if p == 0.0 {
            if q < 0.0 {
                return None; // parallel to this edge and outside it
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            if r > t1 {
                return None;
            }
            t0 = t0.max(r);
        } else {
            if r < t0 {
                return None;
            }
            t1 = t1.min(r);
        }
    }
    Some(((x0 + dx * t0, y0 + dy * t0), (x0 + dx * t1, y0 + dy * t1)))
}

/// Draw a line in pixel coordinates (Bresenham), ignoring depth.
pub fn draw_line(target: &mut Framebuffer, from: (f32, f32), to: (f32, f32), color: Color) {
    let (width, height) = (target.width() as f32, target.height() as f32);
    let Some((from, to)) = clip_segment(from, to, width - 1.0, height - 1.0) else {
        return;
    };

    let (mut x, mut y) = (from.0.round() as i64, from.1.round() as i64);
    let (x1, y1) = (to.0.round() as i64, to.1.round() as i64);
    let dx = (x1 - x).abs();
    let dy = -(y1 - y).abs();
    let sx = if x < x1 { 1 } else { -1 };
    let sy = if y < y1 { 1 } else { -1 };
    let mut error = dx + dy;

    loop {
        target.set_pixel(x as usize, y as usize, color);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * error;
        if e2 >= dy {
            error += dy;
            x += sx;
        }
        if e2 <= dx {
            error += dx;
            y += sy;
        }
    }
}

/// Draw a line of text at a pixel position, ignoring depth.
///
/// `scale` multiplies the 5x7 glyphs; 1 is small but legible at 960x540, 2 is
/// comfortable. Nothing is antialiased and nothing is clipped to a box —
/// anything off the edge of the frame is simply not drawn.
pub fn draw_text(
    target: &mut Framebuffer,
    x: i32,
    y: i32,
    text: &str,
    color: Color,
    scale: usize,
) -> i32 {
    let scale = scale.max(1) as i32;
    let mut cursor = x;
    for character in text.chars() {
        if let Some(columns) = font::glyph(character) {
            for (column_index, column) in columns.iter().enumerate() {
                for row in 0..font::GLYPH_HEIGHT {
                    if column & (1 << row) == 0 {
                        continue;
                    }
                    let px = cursor + column_index as i32 * scale;
                    let py = y + row as i32 * scale;
                    fill_block(target, px, py, scale, color);
                }
            }
        }
        cursor += font::CELL_WIDTH as i32 * scale;
    }
    cursor
}

/// Draw several lines of text, one below the other.
///
/// Returns the y coordinate below the last line, so blocks of text can be
/// stacked without the caller counting rows.
pub fn draw_text_lines(
    target: &mut Framebuffer,
    x: i32,
    y: i32,
    lines: &[&str],
    color: Color,
    scale: usize,
) -> i32 {
    let step = font::line_height(scale) as i32;
    let mut cursor = y;
    for line in lines {
        draw_text(target, x, cursor, line, color, scale);
        cursor += step;
    }
    cursor
}

/// Draw text over a darkened rectangle, so it stays readable on a bright sky.
pub fn draw_text_panel(
    target: &mut Framebuffer,
    x: i32,
    y: i32,
    lines: &[&str],
    color: Color,
    scale: usize,
) {
    let padding = 2 * scale.max(1) as i32;
    let width = lines
        .iter()
        .map(|line| font::text_width(line, scale))
        .max()
        .unwrap_or(0) as i32;
    let height = font::line_height(scale) as i32 * lines.len() as i32;
    shade_rect(
        target,
        x - padding,
        y - padding,
        width + padding * 2,
        height + padding,
        0.25,
    );
    draw_text_lines(target, x, y, lines, color, scale);
}

/// Multiply a rectangle of the frame toward black.
///
/// Darkening rather than filling keeps the scene visible underneath, which is
/// what you want from an overlay that is in the way of the thing you are
/// debugging.
pub fn shade_rect(target: &mut Framebuffer, x: i32, y: i32, width: i32, height: i32, factor: f32) {
    for row in y.max(0)..(y + height).min(target.height() as i32) {
        for column in x.max(0)..(x + width).min(target.width() as i32) {
            let (cx, cy) = (column as usize, row as usize);
            let existing = target.get_pixel(cx, cy);
            target.set_pixel(cx, cy, existing.scale_rgb(factor.clamp(0.0, 1.0)));
        }
    }
}

/// One scaled pixel of a glyph.
fn fill_block(target: &mut Framebuffer, x: i32, y: i32, scale: i32, color: Color) {
    for row in 0..scale {
        for column in 0..scale {
            let (px, py) = (x + column, y + row);
            if px < 0 || py < 0 || px >= target.width() as i32 || py >= target.height() as i32 {
                continue;
            }
            target.set_pixel(px as usize, py as usize, color);
        }
    }
}

/// Draw every triangle edge of a mesh. `mvp` is model-view-projection.
pub fn draw_wireframe(target: &mut Framebuffer, mesh: &Mesh, mvp: Mat4, color: Color) {
    let (width, height) = (target.width() as f32, target.height() as f32);
    for triangle in mesh.indices.chunks_exact(3) {
        let points: Vec<Option<(f32, f32)>> = triangle
            .iter()
            .map(|i| {
                mesh.vertices
                    .get(*i as usize)
                    .and_then(|v| project(mvp.transform_point(v.position), width, height))
            })
            .collect();
        for edge in 0..3 {
            // An edge with a vertex behind the camera is skipped rather than
            // guessed at; this is a debug overlay, not geometry.
            if let (Some(a), Some(b)) = (points[edge], points[(edge + 1) % 3]) {
                draw_line(target, a, b, color);
            }
        }
    }
}

/// Draw each vertex normal as a short segment, in world space.
pub fn draw_normals(
    target: &mut Framebuffer,
    mesh: &Mesh,
    model: Mat4,
    view_projection: Mat4,
    length: f32,
    color: Color,
) {
    let (width, height) = (target.width() as f32, target.height() as f32);
    let normal_matrix = model.normal_matrix();
    for vertex in &mesh.vertices {
        let base = model.transform_point(vertex.position).xyz();
        let tip = base + normal_matrix.transform_vector(vertex.normal).normalized() * length;
        let from = project(view_projection.transform_point(base), width, height);
        let to = project(view_projection.transform_point(tip), width, height);
        if let (Some(from), Some(to)) = (from, to) {
            draw_line(target, from, to, color);
        }
    }
}

/// Draw the three world axes at the origin: X red, Y green, Z blue.
pub fn draw_axes(target: &mut Framebuffer, view_projection: Mat4, length: f32) {
    let (width, height) = (target.width() as f32, target.height() as f32);
    let origin = project(view_projection.transform_point(Vec3::ZERO), width, height);
    let axes = [
        (Vec3::X, Color::RED),
        (Vec3::Y, Color::GREEN),
        (Vec3::Z, Color::BLUE),
    ];
    for (direction, color) in axes {
        let tip = project(
            view_projection.transform_point(direction * length),
            width,
            height,
        );
        if let (Some(origin), Some(tip)) = (origin, tip) {
            draw_line(target, origin, tip, color);
        }
    }
}

/// Render the G-buffer's albedo — the surface color before any light touched it.
pub fn albedo_view(source: &Framebuffer) -> Framebuffer {
    gbuffer_view(source, |surface| surface.albedo)
}

/// Render world-space normals as color, the usual `n * 0.5 + 0.5` mapping:
/// +X red, +Y green, +Z blue.
pub fn normal_view(source: &Framebuffer) -> Framebuffer {
    gbuffer_view(source, |surface| {
        let n = surface.normal;
        Color::rgb(n.x * 0.5 + 0.5, n.y * 0.5 + 0.5, n.z * 0.5 + 0.5)
    })
}

/// Roughness in green, metallic in blue — the two numbers that decide how a
/// surface responds to light.
pub fn material_view(source: &Framebuffer) -> Framebuffer {
    gbuffer_view(source, |surface| {
        Color::rgb(0.0, surface.roughness, surface.metallic)
    })
}

/// Shared plumbing: walk the G-buffer, leave pixels with no geometry black.
fn gbuffer_view(
    source: &Framebuffer,
    map: impl Fn(&crate::gbuffer::Surface) -> Color,
) -> Framebuffer {
    let mut out = Framebuffer::new_raw(source.width(), source.height());
    out.clear(Color::BLACK);
    let Some(gbuffer) = source.gbuffer() else {
        return out;
    };
    for y in 0..source.height() {
        for x in 0..source.width() {
            let surface = gbuffer.get(x, y);
            if surface.is_geometry() {
                out.set_pixel(x, y, map(surface));
            }
        }
    }
    out
}

/// Render the depth buffer as greyscale: near is white, far is black, pixels
/// nothing was drawn to stay black.
///
/// Depth is normalized across what the frame actually contains, because a
/// perspective depth buffer spends almost its whole range near the camera.
pub fn depth_view(source: &Framebuffer) -> Framebuffer {
    let finite: Vec<f32> = source
        .depth()
        .iter()
        .copied()
        .filter(|d| d.is_finite())
        .collect();
    let min = finite.iter().copied().fold(f32::INFINITY, f32::min);
    let max = finite.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let range = (max - min).max(1e-6);

    // Debug views hold display values, not light.
    let mut out = Framebuffer::new_raw(source.width(), source.height());
    out.clear(Color::BLACK);
    for (index, depth) in source.depth().iter().enumerate() {
        let color = if depth.is_finite() {
            let normalized = 1.0 - ((depth - min) / range).clamp(0.0, 1.0);
            Color::rgb(normalized, normalized, normalized)
        } else {
            Color::BLACK
        };
        out.set_pixel(index % source.width(), index / source.width(), color);
    }
    out
}

/// Render the overdraw counter as a heat map: black (untouched) through blue
/// and green to red at `saturation` writes and above.
///
/// Requires [`Framebuffer::track_overdraw`]; without it the result is black.
pub fn overdraw_view(source: &Framebuffer, saturation: u32) -> Framebuffer {
    // Debug views hold display values, not light.
    let mut out = Framebuffer::new_raw(source.width(), source.height());
    out.clear(Color::BLACK);
    let Some(counts) = source.overdraw() else {
        return out;
    };
    let saturation = saturation.max(1) as f32;
    for (index, count) in counts.iter().enumerate() {
        let t = (*count as f32 / saturation).clamp(0.0, 1.0);
        let color = if *count == 0 {
            Color::BLACK
        } else if t < 0.5 {
            Color::rgb(0.0, t * 2.0, 1.0 - t * 2.0)
        } else {
            Color::rgb((t - 0.5) * 2.0, 1.0 - (t - 0.5) * 2.0, 0.0)
        };
        out.set_pixel(index % source.width(), index / source.width(), color);
    }
    out
}

#[cfg(test)]
mod tests {

    /// Count the pixels that are not the background.
    fn lit(target: &Framebuffer) -> usize {
        let mut count = 0;
        for y in 0..target.height() {
            for x in 0..target.width() {
                if target.get_pixel(x, y).luminance() > 0.01 {
                    count += 1;
                }
            }
        }
        count
    }

    /// The rightmost and lowest lit pixel, for checking extents.
    fn extent(target: &Framebuffer) -> (usize, usize) {
        let (mut right, mut bottom) = (0, 0);
        for y in 0..target.height() {
            for x in 0..target.width() {
                if target.get_pixel(x, y).luminance() > 0.01 {
                    right = right.max(x);
                    bottom = bottom.max(y);
                }
            }
        }
        (right, bottom)
    }

    #[test]
    fn text_puts_pixels_on_the_frame() {
        let mut target = Framebuffer::new(200, 40);
        assert_eq!(lit(&target), 0);
        draw_text(&mut target, 4, 4, "RUNITY 123", Color::WHITE, 1);
        let drawn = lit(&target);
        assert!(
            drawn > 40,
            "ten characters should light more than {drawn} pixels"
        );
    }

    #[test]
    fn what_is_measured_is_what_is_drawn() {
        let mut target = Framebuffer::new(300, 40);
        let text = "MEASURE ME";
        let end = draw_text(&mut target, 10, 5, text, Color::WHITE, 1);
        let (right, bottom) = extent(&target);

        // The advance matches the measured width plus the trailing spacing.
        assert_eq!(
            end - 10,
            font::text_width(text, 1) as i32 + font::GLYPH_SPACING as i32
        );
        assert!(
            right < 10 + font::text_width(text, 1),
            "nothing spills past the measured width"
        );
        assert!(bottom < 5 + font::GLYPH_HEIGHT, "nor below the line");
    }

    #[test]
    fn scaling_makes_it_bigger_and_nothing_else() {
        let mut small = Framebuffer::new(200, 60);
        let mut large = Framebuffer::new(200, 60);
        draw_text(&mut small, 2, 2, "ABC", Color::WHITE, 1);
        draw_text(&mut large, 2, 2, "ABC", Color::WHITE, 3);
        // Nine times the area per lit pixel.
        assert_eq!(lit(&large), lit(&small) * 9);
    }

    #[test]
    fn text_off_the_edge_is_clipped_rather_than_wrapped_or_fatal() {
        let mut target = Framebuffer::new(64, 20);
        draw_text(&mut target, -30, -5, "OFF THE TOP LEFT", Color::WHITE, 2);
        draw_text(&mut target, 60, 12, "OFF THE RIGHT", Color::WHITE, 2);
        draw_text(&mut target, 0, 400, "FAR BELOW", Color::WHITE, 1);
        // The point is that none of that panicked; some of it may be visible.
        assert!(lit(&target) < 64 * 20);
    }

    #[test]
    fn an_unknown_character_leaves_a_gap_rather_than_rubbish() {
        let mut with = Framebuffer::new(120, 20);
        let mut without = Framebuffer::new(120, 20);
        draw_text(&mut with, 2, 2, "A\u{1F600}B", Color::WHITE, 1);
        draw_text(&mut without, 2, 2, "A B", Color::WHITE, 1);
        assert_eq!(
            lit(&with),
            lit(&without),
            "an emoji should take a space, not draw one"
        );
    }

    #[test]
    fn lines_stack_downward_without_overlapping() {
        let mut target = Framebuffer::new(200, 80);
        let bottom = draw_text_lines(
            &mut target,
            4,
            4,
            &["FIRST", "SECOND", "THIRD"],
            Color::WHITE,
            1,
        );
        assert_eq!(bottom, 4 + font::line_height(1) as i32 * 3);

        // There is a blank row between lines, which is what makes them legible.
        let gap_row = 4 + font::GLYPH_HEIGHT;
        let blank = (0..target.width()).all(|x| target.get_pixel(x, gap_row).luminance() < 0.01);
        assert!(blank, "row {gap_row} should be the gap between lines");
    }

    #[test]
    fn a_panel_darkens_what_is_behind_it_without_hiding_it() {
        let mut target = Framebuffer::new(160, 40);
        for y in 0..target.height() {
            for x in 0..target.width() {
                target.set_pixel(x, y, Color::rgb(0.8, 0.8, 0.8));
            }
        }
        draw_text_panel(&mut target, 10, 10, &["STATS"], Color::WHITE, 1);

        // Just inside the panel's padding, where no glyph reaches.
        let darkened = target.get_pixel(9, 9);
        assert!(
            darkened.luminance() < 0.8,
            "the panel should be darker than the sky behind it"
        );
        assert!(darkened.luminance() > 0.0, "but not opaque");
        let outside = target.get_pixel(150, 35);
        assert!((outside.luminance() - Color::rgb(0.8, 0.8, 0.8).luminance()).abs() < 1e-6);
    }

    #[test]
    fn shading_a_rectangle_off_the_edge_is_harmless() {
        let mut target = Framebuffer::new(32, 32);
        shade_rect(&mut target, -100, -100, 500, 500, 0.5);
        shade_rect(&mut target, 100, 100, 10, 10, 0.5);
        shade_rect(&mut target, 0, 0, -5, -5, 0.5);
    }

    use super::*;
    use crate::shader::Vertex;
    use runity_math::{Vec2, Vec4};

    #[test]
    fn a_line_lands_on_both_endpoints() {
        let mut fb = Framebuffer::new(16, 16);
        fb.clear(Color::BLACK);
        draw_line(&mut fb, (2.0, 3.0), (12.0, 9.0), Color::WHITE);
        assert_eq!(fb.get_pixel(2, 3), Color::WHITE);
        assert_eq!(fb.get_pixel(12, 9), Color::WHITE);
    }

    #[test]
    fn a_line_outside_the_viewport_draws_nothing() {
        let mut fb = Framebuffer::new(8, 8);
        fb.clear(Color::BLACK);
        draw_line(&mut fb, (-50.0, -50.0), (-10.0, -10.0), Color::WHITE);
        assert!(fb.colors().iter().all(|c| *c == Color::BLACK));
    }

    #[test]
    fn a_long_line_is_clipped_rather_than_walked() {
        // Without clipping this would step through a million pixels.
        let clipped = clip_segment((-1e6, 4.0), (1e6, 4.0), 15.0, 15.0).expect("crosses the view");
        assert_eq!(clipped.0 .0, 0.0);
        assert_eq!(clipped.1 .0, 15.0);

        let mut fb = Framebuffer::new(16, 16);
        fb.clear(Color::BLACK);
        draw_line(&mut fb, (-1e6, 4.0), (1e6, 4.0), Color::WHITE);
        let lit = fb.colors().iter().filter(|c| **c == Color::WHITE).count();
        assert_eq!(lit, 16, "the whole row, and nothing else");
    }

    #[test]
    fn wireframe_draws_edges_but_not_interiors() {
        let mut fb = Framebuffer::new(32, 32);
        fb.clear(Color::BLACK);
        let mesh = Mesh::new(
            vec![
                Vertex::new(Vec3::new(-0.8, -0.8, 0.5), Vec3::Z, Vec2::ZERO),
                Vertex::new(Vec3::new(0.8, -0.8, 0.5), Vec3::Z, Vec2::ZERO),
                Vertex::new(Vec3::new(0.0, 0.8, 0.5), Vec3::Z, Vec2::ZERO),
            ],
            vec![0, 1, 2],
        );
        draw_wireframe(&mut fb, &mesh, Mat4::IDENTITY, Color::WHITE);

        let lit = fb.colors().iter().filter(|c| **c == Color::WHITE).count();
        assert!(
            lit > 40,
            "the three edges should be drawn, got {lit} pixels"
        );
        assert_eq!(fb.get_pixel(16, 20), Color::BLACK, "the inside stays empty");
    }

    #[test]
    fn geometry_behind_the_camera_is_skipped_not_mirrored() {
        assert_eq!(project(Vec4::new(0.0, 0.0, -1.0, 1.0), 64.0, 64.0), None);
        assert_eq!(project(Vec4::new(0.0, 0.0, 1.0, 0.0), 64.0, 64.0), None);
        assert_eq!(
            project(Vec4::new(0.0, 0.0, 0.5, 1.0), 64.0, 64.0),
            Some((32.0, 32.0))
        );
    }

    #[test]
    fn the_depth_view_puts_near_geometry_in_white() {
        let mut fb = Framebuffer::new(4, 1);
        fb.clear(Color::BLACK);
        // Two depths written directly; the rest stays at infinity.
        fb.set_depth_at(0, 0, 0.25);
        fb.set_depth_at(1, 0, 0.75);

        let view = depth_view(&fb);
        assert_eq!(
            view.get_pixel(0, 0),
            Color::WHITE,
            "the nearest sample is white"
        );
        assert_eq!(view.get_pixel(1, 0), Color::BLACK, "the farthest is black");
        assert_eq!(
            view.get_pixel(2, 0),
            Color::BLACK,
            "untouched pixels stay black"
        );
    }

    #[test]
    fn gbuffer_views_show_what_the_geometry_pass_wrote() {
        use crate::gbuffer::Surface;
        let mut fb = Framebuffer::new(2, 1);
        fb.enable_gbuffer(true);
        fb.gbuffer_mut().unwrap().set(
            0,
            Surface {
                albedo: Color::rgb(0.8, 0.1, 0.1),
                normal: Vec3::Y,
                roughness: 0.25,
                metallic: 1.0,
                ..Surface::default()
            },
        );

        assert_eq!(albedo_view(&fb).get_pixel(0, 0), Color::rgb(0.8, 0.1, 0.1));
        assert_eq!(
            albedo_view(&fb).get_pixel(1, 0),
            Color::BLACK,
            "no geometry, no color"
        );

        // +Y maps to green at full brightness.
        let normal = normal_view(&fb).get_pixel(0, 0);
        assert_eq!((normal.r, normal.g, normal.b), (0.5, 1.0, 0.5));

        let material = material_view(&fb).get_pixel(0, 0);
        assert_eq!((material.g, material.b), (0.25, 1.0));
    }

    #[test]
    fn gbuffer_views_are_black_without_a_gbuffer() {
        let fb = Framebuffer::new(4, 4);
        assert!(albedo_view(&fb).colors().iter().all(|c| *c == Color::BLACK));
    }

    #[test]
    fn the_overdraw_view_is_black_without_tracking() {
        let fb = Framebuffer::new(4, 4);
        assert!(overdraw_view(&fb, 4)
            .colors()
            .iter()
            .all(|c| *c == Color::BLACK));
    }
}
