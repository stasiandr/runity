use crate::color::Color;
use crate::framebuffer::Framebuffer;
use crate::mesh::Mesh;
use crate::shader::{Shader, Varying, Vertex};
use runity_math::Vec4;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CullMode {
    None,
    /// Drop triangles whose vertices wind clockwise on screen (the default).
    #[default]
    Back,
    Front,
}

/// Fill triangles, or draw only their edges.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PolygonMode {
    #[default]
    Fill,
    /// Solid wireframe: the same pipeline, but only fragments within
    /// [`Rasterizer::line_width`] pixels of an edge survive. It works with any
    /// shader and still depth-tests, which line drawing over the top does not.
    Line,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    /// Overwrite the destination pixel.
    #[default]
    Replace,
    /// `src * a + dst * (1 - a)`.
    Alpha,
}

/// Counters for one draw call — cheap instrumentation, and what the tests assert on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DrawStats {
    pub triangles_in: usize,
    /// Triangles that survived clipping and culling.
    pub triangles_rasterized: usize,
    pub fragments_shaded: usize,
    pub fragments_written: usize,
}

impl DrawStats {
    fn merge(&mut self, o: DrawStats) {
        self.triangles_in += o.triangles_in;
        self.triangles_rasterized += o.triangles_rasterized;
        self.fragments_shaded += o.fragments_shaded;
        self.fragments_written += o.fragments_written;
    }
}

/// Fixed-function state around the programmable [`Shader`] stages.
#[derive(Debug, Clone, Copy)]
pub struct Rasterizer {
    pub cull: CullMode,
    pub depth_test: bool,
    pub depth_write: bool,
    pub blend: Blend,
    pub polygon_mode: PolygonMode,
    /// Half-width, in pixels, of the edges drawn by [`PolygonMode::Line`].
    pub line_width: f32,
}

impl Default for Rasterizer {
    fn default() -> Self {
        Self {
            cull: CullMode::Back,
            depth_test: true,
            depth_write: true,
            blend: Blend::Replace,
            polygon_mode: PolygonMode::Fill,
            line_width: 0.75,
        }
    }
}

#[derive(Clone, Copy)]
struct ClipVertex<V> {
    clip: Vec4,
    varying: V,
}

#[derive(Clone, Copy)]
struct ScreenVertex<V> {
    x: f32,
    y: f32,
    z: f32,
    inv_w: f32,
    varying: V,
}

/// Anything closer to the eye than this in clip space is thrown away; it keeps
/// `1 / w` finite.
const W_EPSILON: f32 = 1e-6;

impl Rasterizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run the whole pipeline over an indexed mesh.
    pub fn draw_mesh<S: Shader>(
        &self,
        target: &mut Framebuffer,
        mesh: &Mesh,
        shader: &S,
    ) -> DrawStats {
        let mut stats = DrawStats::default();
        for tri in mesh.indices.chunks_exact(3) {
            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            debug_assert!(
                a < mesh.vertices.len() && b < mesh.vertices.len() && c < mesh.vertices.len(),
                "index out of range"
            );
            let (Some(a), Some(b), Some(c)) = (
                mesh.vertices.get(a),
                mesh.vertices.get(b),
                mesh.vertices.get(c),
            ) else {
                continue;
            };
            stats.merge(self.draw_triangle(target, shader, a, b, c));
        }
        stats
    }

    /// Run the pipeline for a single triangle.
    pub fn draw_triangle<S: Shader>(
        &self,
        target: &mut Framebuffer,
        shader: &S,
        a: &Vertex,
        b: &Vertex,
        c: &Vertex,
    ) -> DrawStats {
        let mut stats = DrawStats {
            triangles_in: 1,
            ..DrawStats::default()
        };

        let out = [shader.vertex(a), shader.vertex(b), shader.vertex(c)];
        let input = [
            ClipVertex {
                clip: out[0].clip_position,
                varying: out[0].varying,
            },
            ClipVertex {
                clip: out[1].clip_position,
                varying: out[1].varying,
            },
            ClipVertex {
                clip: out[2].clip_position,
                varying: out[2].varying,
            },
        ];

        let (poly, count) = clip_near(&input);
        if count < 3 {
            return stats;
        }

        // Fan-triangulate the clipped polygon (at most 4 vertices for one plane).
        for i in 1..count - 1 {
            let tri = [poly[0], poly[i], poly[i + 1]];
            if tri.iter().any(|v| v.clip.w <= W_EPSILON) {
                continue;
            }
            let screen = [
                to_screen(&tri[0], target),
                to_screen(&tri[1], target),
                to_screen(&tri[2], target),
            ];
            if self.rasterize(target, shader, screen, &mut stats) {
                stats.triangles_rasterized += 1;
            }
        }
        stats
    }

    /// Screen-space scan conversion. Returns whether the triangle covered the
    /// viewport at all (i.e. survived culling and the bounding-box clip).
    fn rasterize<S: Shader>(
        &self,
        target: &mut Framebuffer,
        shader: &S,
        v: [ScreenVertex<S::Varying>; 3],
        stats: &mut DrawStats,
    ) -> bool {
        let [a, mut b, mut c] = v;

        let mut area = edge(a.x, a.y, b.x, b.y, c.x, c.y);
        // A counter-clockwise winding in NDC becomes a negative screen-space
        // area, because the Y axis is flipped on the way to pixel coordinates.
        let front_facing = area < 0.0;
        match self.cull {
            CullMode::Back if !front_facing => return false,
            CullMode::Front if front_facing => return false,
            _ => {}
        }
        if area == 0.0 || !area.is_finite() {
            return false;
        }
        // Normalize to a positive area so the inside test is a single sign.
        if area < 0.0 {
            core::mem::swap(&mut b, &mut c);
            area = -area;
        }

        let width = target.width();
        let height = target.height();
        let min_x = a.x.min(b.x).min(c.x).floor().max(0.0) as usize;
        let max_x = (a.x.max(b.x).max(c.x).ceil() as i64).clamp(0, width as i64) as usize;
        let min_y = a.y.min(b.y).min(c.y).floor().max(0.0) as usize;
        let max_y = (a.y.max(b.y).max(c.y).ceil() as i64).clamp(0, height as i64) as usize;
        if min_x >= max_x || min_y >= max_y {
            return false;
        }

        let inv_area = 1.0 / area;
        // Edge i is opposite vertex i, so its edge function is vertex i's weight.
        let edges = [(b, c), (c, a), (a, b)];
        // For wireframe: an edge function divided by the edge's length is the
        // pixel distance to that edge.
        let edge_lengths = edges.map(|(p, q)| ((q.x - p.x).powi(2) + (q.y - p.y).powi(2)).sqrt());
        let bias = [
            top_left_bias(edges[0].0, edges[0].1),
            top_left_bias(edges[1].0, edges[1].1),
            top_left_bias(edges[2].0, edges[2].1),
        ];

        for py in min_y..max_y {
            let y = py as f32 + 0.5;
            for px in min_x..max_x {
                let x = px as f32 + 0.5;

                let mut w = [0.0f32; 3];
                let mut inside = true;
                for i in 0..3 {
                    let (p, q) = edges[i];
                    let e = edge(p.x, p.y, q.x, q.y, x, y);
                    if e < 0.0 || (e == 0.0 && !bias[i]) {
                        inside = false;
                        break;
                    }
                    w[i] = e * inv_area;
                }
                if !inside {
                    continue;
                }

                if self.polygon_mode == PolygonMode::Line {
                    let distance = (0..3)
                        .map(|i| {
                            let length = edge_lengths[i];
                            if length > 0.0 {
                                (w[i] * area) / length
                            } else {
                                f32::INFINITY
                            }
                        })
                        .fold(f32::INFINITY, f32::min);
                    if distance > self.line_width {
                        continue;
                    }
                }

                // z/w is linear in screen space, so plain barycentrics are correct.
                let z = w[0] * a.z + w[1] * b.z + w[2] * c.z;
                if !(0.0..=1.0).contains(&z) {
                    continue;
                }

                let index = py * width + px;
                if self.depth_test && z >= target.depth()[index] {
                    continue;
                }

                // Everything else needs the perspective-correct weights.
                let inv_w = w[0] * a.inv_w + w[1] * b.inv_w + w[2] * c.inv_w;
                if inv_w <= 0.0 || !inv_w.is_finite() {
                    continue;
                }
                let scale = 1.0 / inv_w;
                let varying = a
                    .varying
                    .scale(w[0] * a.inv_w * scale)
                    .add(b.varying.scale(w[1] * b.inv_w * scale))
                    .add(c.varying.scale(w[2] * c.inv_w * scale));

                stats.fragments_shaded += 1;
                let Some(src) = shader.fragment(&varying) else {
                    continue;
                };

                // Deferred: the geometry pass records what the surface is, so
                // the lighting and screen-space passes can read it back.
                if target.gbuffer().is_some() {
                    if let Some(surface) = shader.surface(&varying) {
                        if let Some(gbuffer) = target.gbuffer_mut() {
                            gbuffer.set(index, surface);
                        }
                    }
                }

                // The depth write happens after the fragment stage, so a
                // discarded fragment leaves the depth buffer untouched.
                if self.depth_write {
                    target.set_depth(index, z);
                }

                let color = match self.blend {
                    Blend::Replace => src,
                    Blend::Alpha => {
                        // Blending happens in linear light, the only space
                        // where "half of this plus half of that" is true.
                        let dst = target.colors()[index];
                        let alpha = src.a.clamp(0.0, 1.0);
                        dst.lerp(Color::rgba(src.r, src.g, src.b, 1.0), alpha)
                    }
                };
                target.write_color(index, color);
                stats.fragments_written += 1;
            }
        }
        true
    }
}

/// Signed area of the triangle `(ax,ay) (bx,by) (px,py)`, doubled.
#[inline]
fn edge(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> f32 {
    (bx - ax) * (py - ay) - (by - ay) * (px - ax)
}

/// Fill rule: a pixel center that lands exactly on a shared edge belongs to
/// exactly one of the two triangles, so adjacent triangles neither double-blend
/// nor leave a seam.
#[inline]
fn top_left_bias<V>(p: ScreenVertex<V>, q: ScreenVertex<V>) -> bool {
    let dx = q.x - p.x;
    let dy = q.y - p.y;
    dy < 0.0 || (dy == 0.0 && dx > 0.0)
}

fn to_screen<V: Varying>(v: &ClipVertex<V>, target: &Framebuffer) -> ScreenVertex<V> {
    let inv_w = 1.0 / v.clip.w;
    let ndc_x = v.clip.x * inv_w;
    let ndc_y = v.clip.y * inv_w;
    let ndc_z = v.clip.z * inv_w;
    ScreenVertex {
        x: (ndc_x * 0.5 + 0.5) * target.width() as f32,
        // Framebuffer rows run top to bottom; NDC +Y is up.
        y: (0.5 - ndc_y * 0.5) * target.height() as f32,
        z: ndc_z,
        inv_w,
        varying: v.varying,
    }
}

/// Sutherland-Hodgman clipping against the single near plane `z >= 0`.
///
/// Only the near plane has to be clipped: the other five are handled for free by
/// the screen-space bounding box and the depth range test. Without this one,
/// geometry behind the eye would divide by a negative `w` and fold onto screen.
fn clip_near<V: Varying>(input: &[ClipVertex<V>; 3]) -> ([ClipVertex<V>; 4], usize) {
    let mut out = [input[0]; 4];
    let mut n = 0;
    for i in 0..3 {
        let cur = input[i];
        let next = input[(i + 1) % 3];
        let d_cur = cur.clip.z;
        let d_next = next.clip.z;
        let cur_in = d_cur >= 0.0;
        let next_in = d_next >= 0.0;
        if cur_in {
            out[n] = cur;
            n += 1;
        }
        if cur_in != next_in {
            let denom = d_cur - d_next;
            if denom.abs() > f32::EPSILON {
                let t = d_cur / denom;
                out[n] = ClipVertex {
                    clip: cur.clip.lerp(next.clip, t),
                    varying: cur.varying.lerp(next.varying, t),
                };
                n += 1;
            }
        }
    }
    (out, n)
}
