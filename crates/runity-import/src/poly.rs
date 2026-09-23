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
//! `standing: true` stands it up: the points are x and y, a wall drawn as
//! seen from the front, and `height` is its thickness along z — a U of
//! points is a wall with a doorway in it.
//!
//! `holes: [[(1.5, 1.0), (2.5, 1.0), (2.5, 2.0), (1.5, 2.0)]]` cuts
//! outlines all the way through: a window in that wall, a well in a
//! floor.
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
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PolySource {
    pub points: Vec<(f32, f32)>,
    pub height: f32,
    /// The outline stands up instead of lying down: points are x and y —
    /// a wall seen from its front — and `height` is how thick it is, along
    /// z. A U-shaped outline is a wall with a doorway.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub standing: bool,
    /// Outlines cut all the way through, inside `points`: a window in a
    /// standing wall, a well in a floor. Each goes either way round.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<(f32, f32)>>,
}

impl PolySource {
    /// The file's text: one line of points, as a person would write it.
    pub fn to_text(&self) -> String {
        let points: Vec<String> = self
            .points
            .iter()
            .map(|(x, z)| format!("({x:?}, {z:?})"))
            .collect();
        let line = |points: &[(f32, f32)]| {
            points
                .iter()
                .map(|(x, z)| format!("({x:?}, {z:?})"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let holes = if self.holes.is_empty() {
            String::new()
        } else {
            let each: Vec<String> = self
                .holes
                .iter()
                .map(|h| format!("        [{}],\n", line(h)))
                .collect();
            format!("    holes: [\n{}    ],\n", each.concat())
        };
        format!(
            "(\n    points: [{}],\n    height: {:?},\n{}{})\n",
            points.join(", "),
            self.height,
            if self.standing {
                "    standing: true,\n"
            } else {
                ""
            },
            holes
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

/// An outline as given, tidied: a point clicked twice is one point, the
/// first point again at the end closes it rather than adding a corner.
fn clean(given: &[(f32, f32)], what: &str) -> Result<Vec<(f32, f32)>> {
    let mut points: Vec<(f32, f32)> = Vec::new();
    for &(x, z) in given {
        ensure!(
            x.is_finite() && z.is_finite(),
            "a point is two numbers: ({x}, {z})"
        );
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
        "{what} is at least three points, not {}",
        points.len()
    );
    ensure!(
        !crosses_itself(&points),
        "{what} crosses itself: walk it once around without crossing a line"
    );
    ensure!(
        doubled_area(&points).abs() > 1e-6,
        "the points of {what} are all on one line: there is no floor"
    );
    Ok(points)
}

/// The vertices and triangles of the solid.
pub fn build(source: &PolySource) -> Result<(Vec<Vertex>, Vec<u32>)> {
    ensure!(
        source.height.is_finite() && source.height > 0.0,
        "height is how far up it goes, more than zero metres, not {}",
        source.height
    );
    let mut points = clean(&source.points, "an outline")?;
    // Counter-clockwise from above, whichever way it was drawn; holes the
    // other way, so the same rule turns every wall to face out of the solid.
    if doubled_area(&points) < 0.0 {
        points.reverse();
    }
    let mut holes: Vec<Vec<(f32, f32)>> = Vec::with_capacity(source.holes.len());
    for (k, given) in source.holes.iter().enumerate() {
        let mut hole = clean(given, &format!("hole {k}"))?;
        if doubled_area(&hole) > 0.0 {
            hole.reverse();
        }
        ensure!(
            hole.iter().all(|&p| strictly_inside(p, &points)),
            "hole {k} is not inside the outline: a hole goes through the shape, not past its edge"
        );
        for (j, other) in holes.iter().enumerate() {
            ensure!(
                !loops_meet(&hole, other)
                    && !strictly_inside(hole[0], other)
                    && !strictly_inside(other[0], &hole),
                "holes {j} and {k} overlap: make them one hole"
            );
        }
        ensure!(
            !loops_meet(&hole, &points),
            "hole {k} touches the outline: a hole goes through the shape, not past its edge"
        );
        holes.push(hole);
    }
    let merged = bridge(&points, &holes);
    let floor = triangulate(&merged)?;
    let h = source.height;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // Top, facing up; bottom, facing down.
    for (y, up) in [(h, true), (0.0, false)] {
        let base = vertices.len() as u32;
        vertices.extend(merged.iter().map(|&(x, z)| Vertex {
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

    // Walls: a quad per edge, with its own normal so the corners are sharp;
    // round the outline and round each hole.
    for ring in std::iter::once(&points).chain(holes.iter()) {
        let n = ring.len();
        let mut around = 0.0;
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let along = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            // Out of the solid: to the right of a counter-clockwise outline,
            // into the opening of a clockwise hole.
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
    }
    if source.standing {
        // Stood up: what was up is now along z, what was z is up. A swap
        // of two axes is a mirror, so every triangle turns the other way
        // to keep facing out.
        for v in &mut vertices {
            v.position.swap(1, 2);
            v.normal.swap(1, 2);
        }
        for t in indices.chunks_mut(3) {
            t.swap(1, 2);
        }
    }
    Ok((vertices, indices))
}

/// Whether a point is inside an outline, not on it.
fn strictly_inside(p: (f32, f32), ring: &[(f32, f32)]) -> bool {
    let n = ring.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        // On the edge is not inside.
        let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        let within = (p.0 - a.0) * (p.0 - b.0) <= 0.0 && (p.1 - a.1) * (p.1 - b.1) <= 0.0;
        if cross.abs() < 1e-6 && within {
            return false;
        }
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
            inside = !inside;
        }
    }
    inside
}

/// Whether segments ab and cd cross, their ends excluded.
fn crosses(a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)) -> bool {
    let side = |o: (f32, f32), p: (f32, f32), q: (f32, f32)| {
        (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0)
    };
    side(c, d, a) * side(c, d, b) < 0.0 && side(a, b, c) * side(a, b, d) < 0.0
}

/// Whether any edge of one ring crosses or touches an edge of another.
fn loops_meet(one: &[(f32, f32)], other: &[(f32, f32)]) -> bool {
    let edges = |r: &[(f32, f32)]| {
        (0..r.len())
            .map(|i| (r[i], r[(i + 1) % r.len()]))
            .collect::<Vec<_>>()
    };
    let (a, b) = (edges(one), edges(other));
    a.iter().any(|&(p, q)| {
        b.iter()
            .any(|&(r, t)| crosses(p, q, r, t) || [r, t].iter().any(|&x| on_segment(x, p, q)))
    })
}

fn on_segment(x: (f32, f32), p: (f32, f32), q: (f32, f32)) -> bool {
    let cross = (q.0 - p.0) * (x.1 - p.1) - (q.1 - p.1) * (x.0 - p.0);
    cross.abs() < 1e-6 && (x.0 - p.0) * (x.0 - q.0) <= 1e-9 && (x.1 - p.1) * (x.1 - q.1) <= 1e-9
}

/// One outline with the holes joined in: each hole's farthest point along
/// x is joined by a seam, walked there and back, to the nearest corner it
/// can see — so ear clipping, which knows one outline, cuts around them.
fn bridge(outline: &[(f32, f32)], holes: &[Vec<(f32, f32)>]) -> Vec<(f32, f32)> {
    let mut merged = outline.to_vec();
    let mut order: Vec<&Vec<(f32, f32)>> = holes.iter().collect();
    let far = |h: &Vec<(f32, f32)>| h.iter().map(|p| p.0).fold(f32::MIN, f32::max);
    order.sort_by(|a, b| far(b).total_cmp(&far(a)));
    for hole in order {
        let m = (0..hole.len())
            .max_by(|&i, &j| hole[i].0.total_cmp(&hole[j].0))
            .unwrap_or(0);
        let from = hole[m];
        let blocked = |to: (f32, f32)| {
            let edges = |r: &[(f32, f32)]| {
                (0..r.len())
                    .map(|i| (r[i], r[(i + 1) % r.len()]))
                    .collect::<Vec<_>>()
            };
            edges(&merged)
                .into_iter()
                .chain(holes.iter().flat_map(|h| edges(h)))
                .any(|(a, b)| crosses(from, to, a, b))
        };
        let distance = |p: (f32, f32)| (p.0 - from.0).powi(2) + (p.1 - from.1).powi(2);
        let Some(j) = (0..merged.len())
            .filter(|&j| !blocked(merged[j]))
            .min_by(|&a, &b| distance(merged[a]).total_cmp(&distance(merged[b])))
        else {
            continue;
        };
        let mut next = merged[..=j].to_vec();
        next.extend((0..=hole.len()).map(|k| hole[(m + k) % hole.len()]));
        next.extend_from_slice(&merged[j..]);
        merged = next;
    }
    merged
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
                    // A seam's corners are there twice: the copy of a
                    // corner of this very triangle is not inside it.
                    .filter(|&&j| ![a, b, c].contains(&points[j]))
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
            ..Default::default()
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
            build(&PolySource {
                points,
                height,
                ..Default::default()
            })
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
    fn a_standing_outline_is_a_wall_with_a_doorway() {
        // Four metres wide, three high, a door 1 m wide and 2.1 m high.
        let wall = PolySource {
            points: vec![
                (0.0, 0.0),
                (1.5, 0.0),
                (1.5, 2.1),
                (2.5, 2.1),
                (2.5, 0.0),
                (4.0, 0.0),
                (4.0, 3.0),
                (0.0, 3.0),
            ],
            height: 0.2,
            standing: true,
            ..Default::default()
        };
        let (vertices, indices) = build(&wall).unwrap();
        let expected = (4.0 * 3.0 - 1.0 * 2.1) * 0.2;
        assert!((volume(&vertices, &indices) - expected).abs() < 1e-3);
        let bounds = Bounds::of(&vertices);
        assert_eq!(bounds.max, [4.0, 3.0, 0.2]);
        // The front faces -z, toward someone standing in front of it.
        assert!(vertices
            .iter()
            .any(|v| v.normal == [0.0, 0.0, -1.0] && v.position[2] == 0.0));
        let text = wall.to_text();
        assert!(text.contains("standing: true"), "{text}");
        assert_eq!(ron::from_str::<PolySource>(&text).unwrap(), wall);
        assert!(!l_shape().to_text().contains("standing"));
    }

    #[test]
    fn a_hole_goes_through_and_its_walls_face_into_it() {
        // A wall four by three with a window one metre square.
        let window = vec![(1.5, 1.0), (2.5, 1.0), (2.5, 2.0), (1.5, 2.0)];
        let wall = PolySource {
            points: vec![(0.0, 0.0), (4.0, 0.0), (4.0, 3.0), (0.0, 3.0)],
            height: 0.2,
            standing: true,
            holes: vec![window.clone()],
        };
        let (vertices, indices) = build(&wall).unwrap();
        assert!((volume(&vertices, &indices) - (12.0 - 1.0) * 0.2).abs() < 1e-3);
        // The sill faces up into the window.
        assert!(vertices
            .iter()
            .any(|v| v.normal == [0.0, 1.0, 0.0] && (v.position[1] - 1.0).abs() < 1e-6));
        // Two holes in a floor, drawn either way round.
        let mut floor = l_shape();
        floor.holes = vec![
            vec![(1.0, 1.0), (2.0, 1.0), (2.0, 2.0), (1.0, 2.0)],
            vec![(6.0, 4.0), (6.0, 5.0), (5.0, 5.0), (5.0, 4.0)],
        ];
        let (v, i) = build(&floor).unwrap();
        assert!((volume(&v, &i) - (60.0 - 2.0) * 3.0).abs() < 1e-3);
        let text = floor.to_text();
        assert_eq!(ron::from_str::<PolySource>(&text).unwrap(), floor, "{text}");

        let e = |holes: Vec<Vec<(f32, f32)>>| {
            let mut s = l_shape();
            s.holes = holes;
            build(&s).unwrap_err().to_string()
        };
        // In the L's missing corner: outside the outline.
        assert!(e(vec![vec![(5.0, 7.0), (6.0, 7.0), (6.0, 8.0)]]).contains("not inside"));
        assert!(e(vec![
            vec![(1.0, 1.0), (2.0, 1.0), (2.0, 2.0), (1.0, 2.0)],
            vec![(1.5, 1.5), (2.5, 1.5), (2.5, 2.5), (1.5, 2.5)],
        ])
        .contains("overlap"));
    }

    #[test]
    fn the_text_reads_back_as_what_was_written() {
        let text = l_shape().to_text();
        assert_eq!(ron::from_str::<PolySource>(&text).unwrap(), l_shape());
        assert!(text.contains("points: [(0.0, 0.0), (8.0, 0.0)"), "{text}");
    }
}
