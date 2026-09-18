//! Debug views: wireframes, normals, the depth buffer and overdraw.
//!
//! This renderer's job is debugging and headless development, so seeing *why* a
//! frame looks wrong matters more than drawing it quickly. Everything here
//! draws as an overlay, ignoring the depth buffer, so it is visible on top of
//! whatever the scene produced.

use crate::color::Color;
use crate::framebuffer::Framebuffer;
use crate::mesh::Mesh;
use runity_math::{Mat4, Vec3};

/// Project a clip-space position to pixel coordinates.
///
/// Returns `None` for anything at or behind the near plane, which is where the
/// perspective divide stops meaning anything. Public so callers outside this
/// crate — `runity-core`'s world-space text, among them — place things on
/// screen exactly where this crate's own debug overlays do.
pub fn project(clip: runity_math::Vec4, width: f32, height: f32) -> Option<(f32, f32)> {
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

    let mut out = Framebuffer::new(source.width(), source.height());
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
    let mut out = Framebuffer::new(source.width(), source.height());
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
        assert!(fb.pixels().iter().all(|p| *p == Color::BLACK.to_argb8()));
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
        let lit = fb
            .pixels()
            .iter()
            .filter(|p| **p == Color::WHITE.to_argb8())
            .count();
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

        let lit = fb
            .pixels()
            .iter()
            .filter(|p| **p == Color::WHITE.to_argb8())
            .count();
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
    fn the_overdraw_view_is_black_without_tracking() {
        let fb = Framebuffer::new(4, 4);
        assert!(overdraw_view(&fb, 4)
            .pixels()
            .iter()
            .all(|p| *p == 0xff00_0000));
    }
}
