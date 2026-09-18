//! Debug overlays, as geometry.
//!
//! The rasterizer draws its wireframes, normals and axes with Bresenham
//! straight into the framebuffer, ignoring depth. A GPU cannot be handed a
//! loop like that, so the same segments are turned into quads here and drawn
//! through a line pipeline of their own.
//!
//! The projection below is a transcription of the one in
//! `runity_render::debug`, on purpose rather than by import: `runity-render`
//! is a data dependency of this crate and nothing more, and a debug overlay is
//! not worth widening that.
//!
//! These are the only pixels in the GPU path that are *not* expected to match
//! the rasterizer's, and the only deliberate difference in the whole backend:
//! a segment becomes a quad a couple of pixels across, which on a Retina
//! drawable is what makes a line look the way a one-pixel CPU line looks at
//! 1x. They are kept out of the differential tests for that reason.

use runity_math::{Mat4, Vec2, Vec3, Vec4};
use runity_render::{Color, Framebuffer, Mesh, Shader, Texture, Vertex, VertexOutput};

use crate::shader::{GpuShader, PulseUniforms, ENGINE_SOURCE};

/// One overlay segment, in pixel coordinates with the origin at the top left —
/// the same space `runity_render::debug::draw_line` works in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub color: Color,
}

/// Project a clip-space position to pixel coordinates, or `None` for anything
/// at or behind the near plane.
fn project(clip: Vec4, width: f32, height: f32) -> Option<(f32, f32)> {
    if clip.w <= 1e-6 || clip.z < 0.0 {
        return None;
    }
    let ndc = clip.perspective_divide();
    Some(((ndc.x * 0.5 + 0.5) * width, (0.5 - ndc.y * 0.5) * height))
}

/// Every triangle edge of a mesh, as segments.
pub fn wireframe_segments(
    mesh: &Mesh,
    mvp: Mat4,
    width: f32,
    height: f32,
    color: Color,
) -> Vec<Segment> {
    let mut out = Vec::new();
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
            if let (Some(a), Some(b)) = (points[edge], points[(edge + 1) % 3]) {
                out.push(Segment {
                    from: a,
                    to: b,
                    color,
                });
            }
        }
    }
    out
}

/// Each vertex normal as a short segment, in world space.
pub fn normal_segments(
    mesh: &Mesh,
    model: Mat4,
    view_projection: Mat4,
    length: f32,
    width: f32,
    height: f32,
    color: Color,
) -> Vec<Segment> {
    let normal_matrix = model.normal_matrix();
    let mut out = Vec::new();
    for vertex in &mesh.vertices {
        let base = model.transform_point(vertex.position).xyz();
        let tip = base + normal_matrix.transform_vector(vertex.normal).normalized() * length;
        let from = project(view_projection.transform_point(base), width, height);
        let to = project(view_projection.transform_point(tip), width, height);
        if let (Some(from), Some(to)) = (from, to) {
            out.push(Segment { from, to, color });
        }
    }
    out
}

/// The three world axes at the origin: X red, Y green, Z blue.
pub fn axis_segments(
    view_projection: Mat4,
    length: f32,
    width: f32,
    height: f32,
) -> Vec<Segment> {
    let origin = project(view_projection.transform_point(Vec3::ZERO), width, height);
    let mut out = Vec::new();
    for (direction, color) in [
        (Vec3::X, Color::RED),
        (Vec3::Y, Color::GREEN),
        (Vec3::Z, Color::BLUE),
    ] {
        let tip = project(
            view_projection.transform_point(direction * length),
            width,
            height,
        );
        if let (Some(origin), Some(tip)) = (origin, tip) {
            out.push(Segment {
                from: origin,
                to: tip,
                color,
            });
        }
    }
    out
}

/// Expand segments into clip-space quads `thickness` pixels across.
///
/// Positions come out already in clip space — the line shader's matrix is the
/// identity — so nothing downstream has to know these are screen-space lines.
pub fn segments_to_mesh(
    segments: &[Segment],
    width: f32,
    height: f32,
    thickness: f32,
) -> Mesh {
    let mut vertices = Vec::with_capacity(segments.len() * 4);
    let mut indices = Vec::with_capacity(segments.len() * 6);
    let half = (thickness * 0.5).max(0.5);
    // Pixel coordinates to clip space: x right, y *down*, matching the
    // rasterizer's viewport map read backwards.
    let to_clip = |x: f32, y: f32| Vec3::new(x / width * 2.0 - 1.0, 1.0 - y / height * 2.0, 0.0);

    for segment in segments {
        let (dx, dy) = (
            segment.to.0 - segment.from.0,
            segment.to.1 - segment.from.1,
        );
        let length = (dx * dx + dy * dy).sqrt();
        if !length.is_finite() || length <= f32::EPSILON {
            continue;
        }
        let (nx, ny) = (-dy / length * half, dx / length * half);
        let base = vertices.len() as u32;
        for (x, y) in [
            (segment.from.0 + nx, segment.from.1 + ny),
            (segment.to.0 + nx, segment.to.1 + ny),
            (segment.to.0 - nx, segment.to.1 - ny),
            (segment.from.0 - nx, segment.from.1 - ny),
        ] {
            vertices.push(
                Vertex::new(to_clip(x, y), Vec3::Z, Vec2::ZERO).with_color(segment.color),
            );
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(vertices, indices)
}

/// A quad covering the whole target, in clip space, with the top-left corner
/// at `uv = (0, 0)` — which is where both a `Framebuffer`'s first row and a
/// Metal texture's first row live.
pub fn fullscreen_quad() -> Mesh {
    let corner = |x: f32, y: f32, u: f32, v: f32| {
        Vertex::new(Vec3::new(x, y, 0.0), Vec3::Z, Vec2::new(u, v))
    };
    Mesh::new(
        vec![
            corner(-1.0, 1.0, 0.0, 0.0),
            corner(1.0, 1.0, 1.0, 0.0),
            corner(1.0, -1.0, 1.0, 1.0),
            corner(-1.0, -1.0, 0.0, 1.0),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

/// Read a framebuffer back as a texture, so a picture the CPU produced can be
/// put on screen through the GPU path.
pub fn image_to_texture(image: &Framebuffer) -> Texture {
    let mut texture = Texture::from_fn(image.width(), image.height(), |x, y| {
        image.get_pixel(x, y)
    });
    // One texel per pixel, edge to edge: nothing to interpolate or wrap.
    texture.filter = runity_render::Filter::Nearest;
    texture.wrap = runity_render::Wrap::Clamp;
    texture
}

/// The overlay shader: vertex colour, straight through, no lighting and no
/// depth. Its Metal twin is `line_vertex` / `line_fragment`.
pub struct LineShader;

impl Shader for LineShader {
    type Varying = Color;

    fn vertex(&self, vertex: &Vertex) -> VertexOutput<Color> {
        VertexOutput {
            // The positions are already in clip space.
            clip_position: Vec4::new(
                vertex.position.x,
                vertex.position.y,
                vertex.position.z,
                1.0,
            ),
            varying: vertex.color,
        }
    }

    fn fragment(&self, color: &Color) -> Option<Color> {
        Some(*color)
    }
}

impl GpuShader for LineShader {
    const SOURCE: &'static str = ENGINE_SOURCE;
    const VERTEX: &'static str = "line_vertex";
    const FRAGMENT: &'static str = "line_fragment";
    type Uniforms = PulseUniforms;

    fn uniforms(&self) -> PulseUniforms {
        PulseUniforms::new(Mat4::IDENTITY, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_becomes_a_quad_of_the_asked_for_width() {
        let segments = [Segment {
            from: (10.0, 20.0),
            to: (10.0, 40.0),
            color: Color::RED,
        }];
        let mesh = segments_to_mesh(&segments, 100.0, 100.0, 4.0);
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.triangle_count(), 2);
        // A vertical segment is offset horizontally by half the width.
        let xs: Vec<f32> = mesh.vertices.iter().map(|v| v.position.x).collect();
        let span = xs.iter().cloned().fold(f32::MIN, f32::max)
            - xs.iter().cloned().fold(f32::MAX, f32::min);
        // 4 pixels of a 100-pixel-wide target is 0.08 of clip space's 2.0.
        assert!((span - 0.08).abs() < 1e-5, "{span}");
        assert!(mesh.vertices.iter().all(|v| v.color == Color::RED));
    }

    #[test]
    fn a_zero_length_segment_is_dropped_rather_than_producing_nothing() {
        let segments = [
            Segment {
                from: (5.0, 5.0),
                to: (5.0, 5.0),
                color: Color::WHITE,
            },
            Segment {
                from: (0.0, 0.0),
                to: (8.0, 8.0),
                color: Color::WHITE,
            },
        ];
        let mesh = segments_to_mesh(&segments, 16.0, 16.0, 2.0);
        assert_eq!(mesh.triangle_count(), 2, "only the segment with a length");
    }

    #[test]
    fn pixel_coordinates_map_to_clip_space_with_y_running_down() {
        let segments = [Segment {
            from: (0.0, 0.0),
            to: (64.0, 0.0),
            color: Color::WHITE,
        }];
        let mesh = segments_to_mesh(&segments, 64.0, 32.0, 1.0);
        // The top-left pixel is clip (-1, +1): y is flipped exactly once.
        let top_left = mesh
            .vertices
            .iter()
            .find(|v| v.position.x <= -0.99)
            .expect("a corner at the left edge");
        assert!(top_left.position.y > 0.9, "{:?}", top_left.position);
    }

    #[test]
    fn the_fullscreen_quad_puts_the_first_row_of_the_image_at_the_top() {
        let quad = fullscreen_quad();
        let top_left = quad
            .vertices
            .iter()
            .find(|v| v.uv == Vec2::new(0.0, 0.0))
            .expect("a uv origin");
        assert_eq!(top_left.position, Vec3::new(-1.0, 1.0, 0.0));
        assert_eq!(quad.triangle_count(), 2);
    }

    #[test]
    fn an_image_survives_the_trip_through_a_texture() {
        let mut frame = Framebuffer::new(3, 2);
        frame.set_pixel(0, 0, Color::RED);
        frame.set_pixel(2, 1, Color::BLUE);
        let texture = image_to_texture(&frame);
        assert_eq!(texture.width(), 3);
        let sample = |x: usize, y: usize| texture.sample((x as f32 + 0.5) / 3.0, (y as f32 + 0.5) / 2.0);
        assert_eq!(sample(0, 0).to_argb8(), Color::RED.to_argb8());
        assert_eq!(sample(2, 1).to_argb8(), Color::BLUE.to_argb8());
    }

    #[test]
    fn geometry_behind_the_camera_produces_no_segments() {
        let mesh = Mesh::cube(1.0);
        // Everything is behind the eye: the projection rejects it all.
        let behind = Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0));
        assert!(wireframe_segments(&mesh, behind, 64.0, 64.0, Color::WHITE).is_empty());
    }

    #[test]
    fn the_axes_come_out_in_the_order_and_the_colours_debug_draws_them() {
        let view_projection = Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0));
        let axes = axis_segments(view_projection, 1.0, 64.0, 64.0);
        assert_eq!(axes.len(), 3);
        assert_eq!(axes[0].color, Color::RED);
        assert_eq!(axes[1].color, Color::GREEN);
        assert_eq!(axes[2].color, Color::BLUE);
    }
}
