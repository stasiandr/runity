//! `.rpoly`: a floor plan pulled up into a solid — ProBuilder's Poly Shape.
//!
//! ```text
//! (
//!     points: [(0.0, 0.0), (8.0, 0.0), (8.0, 6.0), (3.0, 6.0), (3.0, 10.0), (0.0, 10.0)],
//!     height: 3.0,
//! )
//! ```
//!
//! `points` are the outline on the ground, x and z in metres in the
//! entity's own space, in either direction around; `height` is how far up
//! it goes. The outline may be any shape that does not cross itself — an
//! L-shaped room, a platform, a plinth — which a box cannot be.
//!
//! Imported into an ordinary mesh named after the file, where the points
//! say rather than centred: the outline is what someone drew, and moving
//! it would move what they drew. Placed like any model — `model: "hall",
//! collider: Model, body: Static` — and reloaded like any: change a point,
//! save, the running game has the new floor. Texture coordinates are
//! metres, on the caps and around the walls, so a material tiles the same
//! on every side.

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use runity::asset::{Bounds, MeshAsset, Submesh, Vertex};
use serde::{Deserialize, Serialize};

use crate::ImportSettings;

/// What a `.rpoly` says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolySource {
    pub points: Vec<(f32, f32)>,
    pub height: f32,
}

impl PolySource {
    /// The file's text: one line of points, as a person would write it.
    pub fn to_text(&self) -> String {
        let points: Vec<String> = self
            .points
            .iter()
            .map(|(x, z)| format!("({x:?}, {z:?})"))
            .collect();
        format!(
            "(\n    points: [{}],\n    height: {:?},\n)\n",
            points.join(", "),
            self.height
        )
    }
}

pub fn mesh_from_poly(path: &Path, settings: &ImportSettings) -> Result<MeshAsset> {
    let text = std::fs::read_to_string(path).with_context(|| format!("{}", path.display()))?;
    let source: PolySource = ron::from_str(&text).with_context(|| format!("{}", path.display()))?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "poly".into());
    let (vertices, indices) = build(&source).with_context(|| format!("{}", path.display()))?;
    Ok(MeshAsset {
        id: settings.asset_id(),
        name,
        bounds: Bounds::of(&vertices),
        submeshes: vec![Submesh {
            first_index: 0,
            index_count: indices.len() as u32,
            material: None,
        }],
        vertices,
        indices,
        skin: None,
    })
}

/// Twice the signed area of an outline, positive when it runs
/// counter-clockwise seen from above (+y): x to the right, z toward you.
fn doubled_area(points: &[(f32, f32)]) -> f32 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            // Seen from above, z points down the page: x × z is -y.
            b.0 * a.1 - a.0 * b.1
        })
        .sum()
}

/// The vertices and triangles of the solid.
pub fn build(source: &PolySource) -> Result<(Vec<Vertex>, Vec<u32>)> {
    ensure!(
        source.height.is_finite() && source.height > 0.0,
        "height is how far up it goes, more than zero metres, not {}",
        source.height
    );
    let mut points: Vec<(f32, f32)> = Vec::new();
    for &(x, z) in &source.points {
        ensure!(
            x.is_finite() && z.is_finite(),
            "a point is two numbers: ({x}, {z})"
        );
        // A point clicked twice is one point.
        if points
            .last()
            .is_none_or(|p| (p.0 - x).abs() > 1e-5 || (p.1 - z).abs() > 1e-5)
        {
            points.push((x, z));
        }
    }
    if points.len() > 1 {
        let (first, last) = (points[0], points[points.len() - 1]);
        if (first.0 - last.0).abs() <= 1e-5 && (first.1 - last.1).abs() <= 1e-5 {
            points.pop();
        }
    }
    ensure!(
        points.len() >= 3,
        "an outline is at least three points, not {}",
        points.len()
    );
    ensure!(
        !crosses_itself(&points),
        "the outline crosses itself: walk it once around without crossing a line"
    );
    let area = doubled_area(&points);
    ensure!(
        area.abs() > 1e-6,
        "the points are all on one line: there is no floor"
    );
    // Counter-clockwise from above, whichever way it was drawn.
    if area < 0.0 {
        points.reverse();
    }
    let floor = triangulate(&points)?;
    let h = source.height;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // Top, facing up; bottom, facing down.
    for (y, up) in [(h, true), (0.0, false)] {
        let base = vertices.len() as u32;
        vertices.extend(points.iter().map(|&(x, z)| Vertex {
            position: [x, y, z],
            normal: [0.0, if up { 1.0 } else { -1.0 }, 0.0],
            uv: [x, z],
        }));
        for t in &floor {
            let [a, b, c] = t.map(|i| base + i as u32);
            if up {
                indices.extend_from_slice(&[a, b, c]);
            } else {
                indices.extend_from_slice(&[a, c, b]);
            }
        }
    }

    // Walls: a quad per edge, with its own normal so the corners are sharp.
    let n = points.len();
    let mut around = 0.0;
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        let along = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        // Outward for a counter-clockwise outline seen from above.
        let normal = [-(b.1 - a.1) / along, 0.0, (b.0 - a.0) / along];
        let base = vertices.len() as u32;
        for (p, u) in [(a, around), (b, around + along)] {
            for y in [0.0, h] {
                vertices.push(Vertex {
                    position: [p.0, y, p.1],
                    normal,
                    uv: [u, y],
                });
            }
        }
        // a-low, a-high, b-low, b-high.
        let (al, ah, bl, bh) = (base, base + 1, base + 2, base + 3);
        indices.extend_from_slice(&[al, bl, bh, al, bh, ah]);
        around += along;
    }
    Ok((vertices, indices))
}

/// Whether two edges that do not share a corner cross or touch.
fn crosses_itself(points: &[(f32, f32)]) -> bool {
    let n = points.len();
    let side = |o: (f32, f32), a: (f32, f32), b: (f32, f32)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    let meet = |a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)| {
        let (d1, d2) = (side(c, d, a), side(c, d, b));
        let (d3, d4) = (side(a, b, c), side(a, b, d));
        d1 * d2 <= 0.0 && d3 * d4 <= 0.0 && (d1 != 0.0 || d2 != 0.0 || d3 != 0.0 || d4 != 0.0)
    };
    (0..n).any(|i| {
        (i + 2..n).any(|j| {
            // The first edge and the last share the first corner.
            (i != 0 || j != n - 1)
                && meet(
                    points[i],
                    points[(i + 1) % n],
                    points[j],
                    points[(j + 1) % n],
                )
        })
    })
}

/// Ear clipping: cut off a corner that holds no other point, until one
/// triangle is left. Enough for a floor plan's few dozen points; an
/// outline that crosses itself has no ears to cut and says so.
fn triangulate(points: &[(f32, f32)]) -> Result<Vec<[usize; 3]>> {
    let cross = |o: (f32, f32), a: (f32, f32), b: (f32, f32)| {
        // Positive for a left turn (counter-clockwise from above).
        (a.1 - o.1) * (b.0 - o.0) - (a.0 - o.0) * (b.1 - o.1)
    };
    let inside = |p: (f32, f32), a, b, c| {
        cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
    };
    let mut left: Vec<usize> = (0..points.len()).collect();
    let mut out = Vec::with_capacity(points.len() - 2);
    while left.len() > 3 {
        let n = left.len();
        let ear = (0..n).find(|&i| {
            let (ia, ib, ic) = (left[(i + n - 1) % n], left[i], left[(i + 1) % n]);
            let (a, b, c) = (points[ia], points[ib], points[ic]);
            cross(a, b, c) > 1e-9
                && left
                    .iter()
                    .filter(|&&j| j != ia && j != ib && j != ic)
                    .all(|&j| !inside(points[j], a, b, c))
        });
        let Some(i) = ear else {
            bail!("the outline crosses itself: walk it once around without crossing a line");
        };
        out.push([left[(i + n - 1) % n], left[i], left[(i + 1) % n]]);
        left.remove(i);
    }
    out.push([left[0], left[1], left[2]]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l_shape() -> PolySource {
        PolySource {
            points: vec![
                (0.0, 0.0),
                (8.0, 0.0),
                (8.0, 6.0),
                (3.0, 6.0),
                (3.0, 10.0),
                (0.0, 10.0),
            ],
            height: 3.0,
        }
    }

    /// Volume by the divergence theorem: closed and facing out, or wrong.
    fn volume(vertices: &[Vertex], indices: &[u32]) -> f32 {
        indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]]
                    .map(|i| glam::Vec3::from_array(vertices[i as usize].position));
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    #[test]
    fn an_l_shaped_room_is_a_closed_solid_of_its_floor_times_its_height() {
        let (vertices, indices) = build(&l_shape()).unwrap();
        // 8×6 plus 3×4, three metres up.
        let expected = (48.0 + 12.0) * 3.0;
        assert!((volume(&vertices, &indices) - expected).abs() < 1e-3);
        // Drawn the other way round, the same solid.
        let mut back = l_shape();
        back.points.reverse();
        let (v, i) = build(&back).unwrap();
        assert!((volume(&v, &i) - expected).abs() < 1e-3);
        // Walls face out: the one along z = 0 faces -z.
        let wall = vertices
            .iter()
            .find(|v| v.normal[1] == 0.0 && v.position[2] == 0.0 && v.position[0] == 4.0);
        assert!(wall.is_none(), "corners only");
        assert!(vertices
            .iter()
            .any(|v| v.normal == [0.0, 0.0, -1.0] && v.position[2] == 0.0));
    }

    #[test]
    fn an_outline_that_cannot_be_a_floor_says_why() {
        let e = |points: Vec<(f32, f32)>, height: f32| {
            build(&PolySource { points, height })
                .unwrap_err()
                .to_string()
        };
        assert!(e(vec![(0.0, 0.0), (1.0, 0.0)], 1.0).contains("three points"));
        assert!(e(vec![(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)], 1.0).contains("one line"));
        assert!(e(l_shape().points, 0.0).contains("more than zero"));
        let bow = vec![(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)];
        assert!(e(bow, 1.0).contains("crosses itself"));
        // The first point again at the end is the same outline, not a fifth.
        let mut closed = l_shape();
        closed.points.push((0.0, 0.0));
        assert!(build(&closed).is_ok());
    }

    #[test]
    fn the_text_reads_back_as_what_was_written() {
        let text = l_shape().to_text();
        assert_eq!(ron::from_str::<PolySource>(&text).unwrap(), l_shape());
        assert!(text.contains("points: [(0.0, 0.0), (8.0, 0.0)"), "{text}");
    }
}
