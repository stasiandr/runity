//! Levels of detail, made by the engine: a mesh of more than a few hundred
//! triangles gets two coarser versions of itself when it is uploaded, and
//! each frame whatever is small on the screen is drawn with one of them —
//! and what is smaller than a couple of pixels not at all.
//!
//! The coarser meshes are made by clustering: the mesh's box is cut into a
//! grid of cells, every vertex in a cell (and facing the same way — a thin
//! wall keeps both its faces) becomes one at their average, and the
//! triangles that fold to a line or a point go. Blunt, but it never makes
//! holes where there were none, it costs nothing to author, and at the
//! distance it is used the difference is a pixel.

use glam::Vec3;

use crate::asset::Vertex;

/// A coarser mesh from `vertices` and `indices`, merging what lies within
/// `cell` metres: its vertices and triangles, or `None` when it would be
/// hardly smaller.
pub fn simplify(vertices: &[Vertex], indices: &[u32], cell: f32) -> Option<(Vec<Vertex>, Vec<u32>)> {
    if cell <= 0.0 || indices.len() < 3 {
        return None;
    }
    let mut cells: std::collections::HashMap<(i32, i32, i32, u8), u32> = std::collections::HashMap::new();
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
        let key = (q.x as i32, q.y as i32, q.z as i32, facing);
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
    Some((out_vertices, out_indices))
}

/// The triangles a mesh needs before it is given coarser levels.
pub const FROM_TRIANGLES: usize = 600;

/// The levels a mesh is given: each as a share of its box's diagonal to
/// cluster within, and the least share of the screen's height it is drawn
/// for — below the next one's, the next one is drawn.
pub const LEVELS: [(f32, f32); 2] = [(1.0 / 28.0, 0.25), (1.0 / 11.0, 0.08)];

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
        let (v, i) = simplify(&sphere.vertices, &sphere.indices, diag / 11.0).expect("coarser");
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
        if let Some((tv, ti)) = simplify(&thin, &wall.indices, 0.5) {
            let facing = |z: f32| {
                ti.chunks_exact(3)
                    .filter(|t| t.iter().all(|&k| tv[k as usize].normal[2].signum() == z))
                    .count()
            };
            assert!(facing(1.0) > 0 && facing(-1.0) > 0, "both faces kept");
        }
        // A small mesh is not worth it.
        assert!(simplify(&sphere.vertices, &sphere.indices, 0.0001).is_none());
    }

    #[test]
    fn coverage_halves_as_the_distance_doubles() {
        let near = coverage(1.0, 10.0, 60.0);
        let far = coverage(1.0, 20.0, 60.0);
        assert!((near / far - 2.0).abs() < 1e-4);
        assert!(coverage(1.0, 1000.0, 60.0) < TOO_SMALL);
    }
}
