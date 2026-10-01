//! Levels of detail, made by the engine: a mesh of more than a few hundred
//! triangles gets two coarser versions of itself when it is uploaded, and
//! each frame whatever is small on the screen is drawn with one of them —
//! and what is smaller than a couple of pixels not at all.
//!
//! The coarser meshes are made by clustering: the mesh's box is cut into a
//! grid of cells, every vertex in a cell (and facing the same way — a thin
//! wall keeps both its faces — and near the same place on its picture, so
//! the pieces of a texture atlas stay apart) becomes one at their average,
//! and the
//! triangles that fold to a line or a point go. Blunt, but it never makes
//! holes where there were none, it costs nothing to author, and at the
//! distance it is used the difference is a pixel.

use glam::Vec3;

use crate::asset::Vertex;

/// A coarser mesh from `vertices` and `indices`, merging what lies within
/// `cell` metres: its vertices, their painted colours (averaged, when
/// `colors` has one a vertex; none when it is empty) and triangles, or
/// `None` when it would be hardly smaller.
pub fn simplify(
    vertices: &[Vertex],
    colors: &[[u8; 4]],
    indices: &[u32],
    cell: f32,
) -> Option<(Vec<Vertex>, Vec<[u8; 4]>, Vec<u32>)> {
    if cell <= 0.0 || indices.len() < 3 {
        return None;
    }
    let mut cells: std::collections::HashMap<(i32, i32, i32, u8, i32, i32), u32> = std::collections::HashMap::new();
    let mut sums: Vec<(Vec3, Vec3, [f32; 2], f32)> = Vec::new();
    let mut remap = Vec::with_capacity(vertices.len());
    for v in vertices {
        let p = Vec3::from_array(v.position);
        let n = Vec3::from_array(v.normal);
        // Which way it faces, by its largest axis: what faces apart stays
        // apart, so both sides of a thin thing survive.
        let a = n.abs();
        let facing = if a.x >= a.y && a.x >= a.z {
            if n.x >= 0.0 { 0 } else { 1 }
        } else if a.y >= a.z {
            if n.y >= 0.0 { 2 } else { 3 }
        } else if n.z >= 0.0 {
            4
        } else {
            5
        };
        let q = (p / cell).floor();
        // Where on its picture: a vertex on the seam of two pieces of an
        // atlas is two vertices, one in each, and merged into one it would
        // stretch a triangle of the one piece across the picture to the
        // other — far away, a model baked into an atlas came out in stripes.
        let (u, w) = ((v.uv[0] * UV_CELLS).floor() as i32, (v.uv[1] * UV_CELLS).floor() as i32);
        let key = (q.x as i32, q.y as i32, q.z as i32, facing, u, w);
        let at = *cells.entry(key).or_insert_with(|| {
            sums.push((Vec3::ZERO, Vec3::ZERO, v.uv, 0.0));
            sums.len() as u32 - 1
        });
        let s = &mut sums[at as usize];
        s.0 += p;
        s.1 += n;
        s.3 += 1.0;
        remap.push(at);
    }
    let out_vertices: Vec<Vertex> = sums
        .iter()
        .map(|(p, n, uv, count)| Vertex {
            position: (*p / count.max(1.0)).to_array(),
            normal: n.normalize_or(Vec3::Y).to_array(),
            uv: *uv,
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    let mut out_indices = Vec::with_capacity(indices.len());
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]);
        if a == b || b == c || a == c {
            continue;
        }
        // The same triangle twice is one.
        let mut key = [a, b, c];
        key.sort_unstable();
        if !seen.insert(key) {
            continue;
        }
        out_indices.extend_from_slice(&[a, b, c]);
    }
    if out_indices.is_empty() || out_indices.len() * 10 > indices.len() * 7 {
        return None;
    }
    let mut out_colors = Vec::new();
    if colors.len() == vertices.len() {
        let mut totals = vec![[0u32; 5]; out_vertices.len()];
        for (c, &at) in colors.iter().zip(&remap) {
            let t = &mut totals[at as usize];
            for k in 0..4 {
                t[k] += c[k] as u32;
            }
            t[4] += 1;
        }
        out_colors = totals
            .iter()
            .map(|t| std::array::from_fn(|k| (t[k] / t[4].max(1)) as u8))
            .collect();
    }
    Some((out_vertices, out_colors, out_indices))
}

/// Vertices whose places on the picture are in different ones of this
/// many cells a side are never merged.
const UV_CELLS: f32 = 8.0;

/// The triangles a mesh needs before it is given coarser levels.
pub const FROM_TRIANGLES: usize = 600;

/// The levels a mesh is given: each as a share of its box's diagonal to
/// cluster within, and the least share of the screen's height it is drawn
/// for — below the next one's, the next one is drawn.
pub const LEVELS: [(f32, f32); 2] = [(1.0 / 28.0, 0.25), (1.0 / 11.0, 0.08)];

/// A mesh's coarser levels, finest first: for each, the share of the
/// screen's height it is drawn below, its vertices, colours and
/// triangles. None for a mesh of fewer than [`FROM_TRIANGLES`].
#[allow(clippy::type_complexity)]
pub fn levels(vertices: &[Vertex], colors: &[[u8; 4]], indices: &[u32]) -> Vec<(f32, Vec<Vertex>, Vec<[u8; 4]>, Vec<u32>)> {
    let mut levels = Vec::new();
    if indices.len() / 3 < FROM_TRIANGLES {
        return levels;
    }
    let bounds = crate::asset::Bounds::of(vertices);
    let diagonal = (Vec3::from_array(bounds.max) - Vec3::from_array(bounds.min)).length();
    for (share, below) in LEVELS {
        let Some((v, c, i)) = simplify(vertices, colors, indices, diagonal * share) else {
            break;
        };
        levels.push((below, v, c, i));
    }
    levels
}

/// Below this share of the screen's height a thing is not drawn at all.
pub const TOO_SMALL: f32 = 0.004;

/// How much of the screen's height a thing of `radius` at `distance`
/// covers, for a camera `fov_deg` high.
pub fn coverage(radius: f32, distance: f32, fov_deg: f32) -> f32 {
    let half = (fov_deg.to_radians() * 0.5).tan().max(1e-4);
    radius / (distance.max(1e-3) * half)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dense_sphere_simplifies_to_fewer_triangles_keeping_its_size_and_both_faces_of_a_wall() {
        let sphere = crate::builtin::sphere(1.0, 64, 32);
        let before = sphere.indices.len() / 3;
        let diag = 2.0 * 3f32.sqrt();
        let (v, _, i) = simplify(&sphere.vertices, &[], &sphere.indices, diag / 11.0).expect("coarser");
        let after = i.len() / 3;
        assert!(after * 4 < before, "{after} of {before} triangles");
        // Still about the size it was: its points about a unit out.
        let radii: Vec<f32> = v.iter().map(|p| Vec3::from_array(p.position).length()).collect();
        let mean = radii.iter().sum::<f32>() / radii.len() as f32;
        assert!((mean - 1.0).abs() < 0.12, "mean radius {mean}");
        assert!(i.iter().all(|&k| (k as usize) < v.len()));
        // A thin box keeps both of its big faces.
        let wall = crate::builtin::cube(1.0);
        let thin: Vec<Vertex> = wall
            .vertices
            .iter()
            .map(|v| Vertex {
                position: [v.position[0] * 4.0, v.position[1] * 4.0, v.position[2] * 0.02],
                ..*v
            })
            .collect();
        if let Some((tv, _, ti)) = simplify(&thin, &[], &wall.indices, 0.5) {
            let facing = |z: f32| {
                ti.chunks_exact(3)
                    .filter(|t| t.iter().all(|&k| tv[k as usize].normal[2].signum() == z))
                    .count()
            };
            assert!(facing(1.0) > 0 && facing(-1.0) > 0, "both faces kept");
        }
        // A small mesh is not worth it.
        assert!(simplify(&sphere.vertices, &[], &sphere.indices, 0.0001).is_none());
    }

    /// A plane in two pieces of an atlas — its left half painted from the
    /// left of the picture, its right half from the right, the seam's
    /// vertices doubled — keeps every triangle within one piece.
    #[test]
    fn the_pieces_of_an_atlas_stay_apart() {
        let n = 20;
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for half in 0..2 {
            let base = vertices.len() as u32;
            for j in 0..=n {
                for i in 0..=n {
                    let (x, z) = (half as f32 + i as f32 / n as f32 - 1.0, j as f32 / n as f32);
                    let u = if half == 0 { 0.05 + 0.35 * i as f32 / n as f32 } else { 0.6 + 0.35 * i as f32 / n as f32 };
                    vertices.push(Vertex { position: [x, 0.0, z], normal: [0.0, 1.0, 0.0], uv: [u, z] });
                }
            }
            for j in 0..n {
                for i in 0..n {
                    let a = base + j * (n + 1) + i;
                    let (b, c, d) = (a + 1, a + n + 1, a + n + 2);
                    indices.extend_from_slice(&[a, c, b, b, c, d]);
                }
            }
        }
        let (v, _, i) = simplify(&vertices, &[], &indices, 0.3).expect("coarser");
        let crossing = i
            .chunks_exact(3)
            .filter(|t| {
                let us: Vec<f32> = t.iter().map(|&k| v[k as usize].uv[0]).collect();
                us.iter().any(|u| *u < 0.5) && us.iter().any(|u| *u >= 0.5)
            })
            .count();
        assert_eq!(crossing, 0, "triangles stretched across the picture");
    }

    #[test]
    fn coverage_halves_as_the_distance_doubles() {
        let near = coverage(1.0, 10.0, 60.0);
        let far = coverage(1.0, 20.0, 60.0);
        assert!((near / far - 2.0).abs() < 1e-4);
        assert!(coverage(1.0, 1000.0, 60.0) < TOO_SMALL);
    }
}
