//! A regular grid of heights over a rectangle in the XZ plane.

use runity_math::{Vec2, Vec3};
use runity_render::mesh::Mesh;
use runity_render::shader::Vertex;

/// A regular grid of `f32` heights over a rectangle in the XZ plane.
///
/// The same grid backs both a renderable [`Mesh`] ([`Heightfield::to_mesh`])
/// and physics queries ([`Heightfield::height_at`],
/// [`Heightfield::normal_at`]): every cell is split into two triangles along
/// the same diagonal in both places, so there is no point where the ground
/// a foot rests on differs from the ground that gets drawn.
#[derive(Debug, Clone)]
pub struct Heightfield {
    /// World-space XZ coordinate of grid node `(col: 0, row: 0)`.
    pub origin: Vec2,
    /// Distance between adjacent grid nodes, in world units.
    pub cell: f32,
    /// Number of grid nodes along X.
    pub cols: usize,
    /// Number of grid nodes along Z.
    pub rows: usize,
    /// Height of each node, row-major: `heights[row * cols + col]`.
    pub heights: Vec<f32>,
}

impl Heightfield {
    /// Build a heightfield from a row-major grid of node heights.
    ///
    /// # Panics
    ///
    /// Panics if `cols < 2 || rows < 2` (a heightfield needs at least one
    /// full cell) or if `heights.len() != cols * rows`.
    pub fn new(origin: Vec2, cell: f32, cols: usize, rows: usize, heights: Vec<f32>) -> Self {
        assert!(
            cols >= 2 && rows >= 2,
            "a heightfield needs at least one cell (cols and rows >= 2)"
        );
        assert_eq!(
            heights.len(),
            cols * rows,
            "heights must contain exactly cols * rows entries"
        );
        Self {
            origin,
            cell,
            cols,
            rows,
            heights,
        }
    }

    fn height(&self, col: usize, row: usize) -> f32 {
        self.heights[row * self.cols + col]
    }

    fn node_position(&self, col: usize, row: usize) -> Vec3 {
        Vec3::new(
            self.origin.x + col as f32 * self.cell,
            self.height(col, row),
            self.origin.y + row as f32 * self.cell,
        )
    }

    /// Find the cell that contains `(x, z)` and the fractional coordinates
    /// `(u, v)` within it, each in `[0, 1]`.
    ///
    /// Points outside the field are clamped to the nearest edge cell so that
    /// queries stay defined (flat extrapolation) instead of panicking or
    /// reading out of bounds.
    fn locate(&self, x: f32, z: f32) -> (usize, usize, f32, f32) {
        let max_gx = (self.cols - 1) as f32;
        let max_gz = (self.rows - 1) as f32;
        let gx = ((x - self.origin.x) / self.cell).clamp(0.0, max_gx);
        let gz = ((z - self.origin.y) / self.cell).clamp(0.0, max_gz);
        let col = (gx as usize).min(self.cols - 2);
        let row = (gz as usize).min(self.rows - 2);
        (col, row, gx - col as f32, gz - row as f32)
    }

    /// Height of the rendered surface at world XZ `(x, z)`.
    ///
    /// Every cell is cut along the diagonal from its `(col+1, row)` corner to
    /// its `(col, row+1)` corner — the same diagonal [`Heightfield::to_mesh`]
    /// uses — and the result is the exact bilinear-on-a-triangle height of
    /// that half of the cell, so this is continuous across cell edges and
    /// across the diagonal itself.
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let (col, row, u, v) = self.locate(x, z);
        let h00 = self.height(col, row);
        let h10 = self.height(col + 1, row);
        let h01 = self.height(col, row + 1);
        let h11 = self.height(col + 1, row + 1);
        if u + v <= 1.0 {
            h00 + (h10 - h00) * u + (h01 - h00) * v
        } else {
            h11 + (h01 - h11) * (1.0 - u) + (h10 - h11) * (1.0 - v)
        }
    }

    /// Outward surface normal of the triangle at world XZ `(x, z)`.
    ///
    /// Uses exactly the triangle [`Heightfield::height_at`] interpolates
    /// over, via a cross product of its edges — no trigonometry involved.
    pub fn normal_at(&self, x: f32, z: f32) -> Vec3 {
        let (col, row, u, v) = self.locate(x, z);
        let p00 = self.node_position(col, row);
        let p10 = self.node_position(col + 1, row);
        let p01 = self.node_position(col, row + 1);
        let p11 = self.node_position(col + 1, row + 1);
        let n = if u + v <= 1.0 {
            (p01 - p00).cross(p10 - p00)
        } else {
            (p01 - p10).cross(p11 - p10)
        };
        n.normalized()
    }

    /// Triangle mesh for the whole grid, one quad (two triangles) per cell.
    pub fn to_mesh(&self) -> Mesh {
        let col_indices: Vec<usize> = (0..self.cols).collect();
        let row_indices: Vec<usize> = (0..self.rows).collect();
        self.build_mesh(&col_indices, &row_indices)
    }

    /// Coarser mesh for distant rendering: keeps every `stride`-th grid node
    /// in each direction (plus the far edge, if it doesn't already land on a
    /// multiple of `stride`) and triangulates those nodes directly. Heights
    /// are never resampled — only the original grid nodes are used.
    pub fn to_mesh_lod(&self, stride: usize) -> Mesh {
        let stride = stride.max(1);
        let col_indices = Self::lod_indices(self.cols, stride);
        let row_indices = Self::lod_indices(self.rows, stride);
        self.build_mesh(&col_indices, &row_indices)
    }

    fn lod_indices(count: usize, stride: usize) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..count).step_by(stride).collect();
        if *indices.last().expect("count >= 2") != count - 1 {
            indices.push(count - 1);
        }
        indices
    }

    /// Build a mesh from a (possibly sparse) selection of grid nodes,
    /// splitting each resulting quad along the same diagonal `height_at` and
    /// `normal_at` use for the full-resolution grid.
    fn build_mesh(&self, col_indices: &[usize], row_indices: &[usize]) -> Mesh {
        let cols = col_indices.len();
        let rows = row_indices.len();
        let mut vertices = Vec::with_capacity(cols * rows);
        for (rj, &row) in row_indices.iter().enumerate() {
            for (ci, &col) in col_indices.iter().enumerate() {
                let position = self.node_position(col, row);
                let uv = Vec2::new(ci as f32 / (cols - 1) as f32, rj as f32 / (rows - 1) as f32);
                vertices.push(Vertex::new(position, Vec3::Y, uv));
            }
        }
        let mut indices = Vec::with_capacity((cols - 1) * (rows - 1) * 6);
        let stride = cols as u32;
        for j in 0..(rows - 1) as u32 {
            for i in 0..(cols - 1) as u32 {
                let a = j * stride + i;
                let b = a + 1;
                let c = a + stride;
                let d = c + 1;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        let mut mesh = Mesh::new(vertices, indices);
        mesh.recompute_normals();
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic xorshift so tests don't need a `rand` dependency.
    fn xorshift(state: &mut u32) -> u32 {
        *state ^= *state << 13;
        *state ^= *state >> 17;
        *state ^= *state << 5;
        *state
    }

    fn rand_f32(state: &mut u32, lo: f32, hi: f32) -> f32 {
        let t = xorshift(state) as f32 / u32::MAX as f32;
        lo + (hi - lo) * t
    }

    /// A field with distinct, non-planar heights so a wrong diagonal or a
    /// wrong corner shows up as a real numeric mismatch, not a coincidence.
    fn bumpy_field(cols: usize, rows: usize) -> Heightfield {
        let mut heights = Vec::with_capacity(cols * rows);
        for row in 0..rows {
            for col in 0..cols {
                let x = col as f32;
                let z = row as f32;
                heights.push(x * 0.7 - z * 1.3 + x * z * 0.2);
            }
        }
        Heightfield::new(Vec2::new(-1.0, 2.0), 0.5, cols, rows, heights)
    }

    /// Interpolate height from the mesh's own triangles, independent of
    /// `height_at`'s formula, by finding the triangle that contains `(x, z)`
    /// and evaluating its plane there.
    fn height_from_mesh(mesh: &Mesh, x: f32, z: f32) -> Option<f32> {
        for tri in mesh.indices.chunks_exact(3) {
            let p: Vec<Vec3> = tri
                .iter()
                .map(|&i| mesh.vertices[i as usize].position)
                .collect();
            let (p0, p1, p2) = (p[0], p[1], p[2]);
            // Barycentric coordinates via 2D edge functions on XZ.
            let denom = (p1.z - p2.z) * (p0.x - p2.x) + (p2.x - p1.x) * (p0.z - p2.z);
            if denom.abs() < 1e-9 {
                continue;
            }
            let l0 = ((p1.z - p2.z) * (x - p2.x) + (p2.x - p1.x) * (z - p2.z)) / denom;
            let l1 = ((p2.z - p0.z) * (x - p2.x) + (p0.x - p2.x) * (z - p2.z)) / denom;
            let l2 = 1.0 - l0 - l1;
            let eps = -1e-4;
            if l0 >= eps && l1 >= eps && l2 >= eps {
                return Some(l0 * p0.y + l1 * p1.y + l2 * p2.y);
            }
        }
        None
    }

    #[test]
    fn height_at_matches_the_mesh_on_random_points() {
        let field = bumpy_field(5, 4);
        let mesh = field.to_mesh();
        let mut state = 0x1234_5678u32;
        for _ in 0..500 {
            let x = rand_f32(&mut state, -1.0, -1.0 + 4.0 * 0.5);
            let z = rand_f32(&mut state, 2.0, 2.0 + 3.0 * 0.5);
            let expected = height_from_mesh(&mesh, x, z).expect("point must land in a triangle");
            let actual = field.height_at(x, z);
            assert!(
                (actual - expected).abs() < 1e-3,
                "at ({x}, {z}): height_at={actual}, mesh={expected}"
            );
        }
    }

    #[test]
    fn height_at_matches_the_mesh_exactly_on_diagonals() {
        let field = bumpy_field(4, 4);
        let mesh = field.to_mesh();
        for row in 0..3 {
            for col in 0..3 {
                // Midpoint of the diagonal shared by both triangles in this cell.
                let x = field.origin.x + (col as f32 + 0.5) * field.cell;
                let z = field.origin.y + (row as f32 + 0.5) * field.cell;
                let expected = height_from_mesh(&mesh, x, z).unwrap();
                let actual = field.height_at(x, z);
                assert!(
                    (actual - expected).abs() < 1e-4,
                    "cell ({col},{row}) diagonal: height_at={actual}, mesh={expected}"
                );
            }
        }
    }

    #[test]
    fn height_at_is_continuous_across_the_diagonal() {
        let field = bumpy_field(3, 3);
        // Straddle the diagonal of cell (0, 0), u + v == 1, from both sides.
        let eps = 1e-4;
        let a = field.height_at(
            field.origin.x + 0.5 * field.cell - eps,
            field.origin.y + 0.5 * field.cell,
        );
        let b = field.height_at(
            field.origin.x + 0.5 * field.cell + eps,
            field.origin.y + 0.5 * field.cell,
        );
        assert!((a - b).abs() < 1e-3, "{a} vs {b}");
    }

    #[test]
    fn height_at_is_continuous_across_cell_boundaries() {
        let field = bumpy_field(5, 5);
        let mut state = 0x9e37_79b9u32;
        for _ in 0..200 {
            // A boundary between cell columns, at a random row-fraction.
            let col = 1 + (xorshift(&mut state) % 3) as usize;
            let x = field.origin.x + col as f32 * field.cell;
            let z = field.origin.y + rand_f32(&mut state, 0.0, 4.0 * field.cell);
            let eps = 1e-4;
            let left = field.height_at(x - eps, z);
            let right = field.height_at(x + eps, z);
            assert!(
                (left - right).abs() < 1e-2,
                "{left} vs {right} at x boundary"
            );
        }
    }

    #[test]
    fn normal_at_matches_the_triangle_cross_product() {
        // A single tilted cell: height rises linearly along X, flat along Z,
        // wholly within one triangle no matter which diagonal side.
        let field = Heightfield::new(Vec2::ZERO, 1.0, 2, 2, vec![0.0, 1.0, 0.0, 1.0]);
        let n = field.normal_at(0.25, 0.25);
        // Plane y = x: normal (unnormalized) is (-1, 1, 0), normalized (-1/sqrt(2), 1/sqrt(2), 0).
        let expected = Vec3::new(-1.0, 1.0, 0.0).normalized();
        assert!((n - expected).length() < 1e-4, "{n:?} vs {expected:?}");
        assert!((n.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn normal_at_is_up_on_a_flat_field() {
        let field = Heightfield::new(Vec2::ZERO, 1.0, 3, 3, vec![2.0; 9]);
        for &(x, z) in &[(0.2, 0.2), (0.8, 0.8), (1.5, 0.5), (0.5, 1.5)] {
            let n = field.normal_at(x, z);
            assert!((n - Vec3::Y).length() < 1e-5, "{n:?} at ({x},{z})");
        }
    }

    #[test]
    fn to_mesh_has_two_triangles_per_cell_and_matching_vertex_count() {
        let field = bumpy_field(4, 3);
        let mesh = field.to_mesh();
        assert_eq!(mesh.vertices.len(), 4 * 3);
        assert_eq!(mesh.triangle_count(), (4 - 1) * (3 - 1) * 2);
    }

    #[test]
    fn to_mesh_lod_reuses_original_node_heights() {
        let mut heights = Vec::new();
        for row in 0..5 {
            for col in 0..5 {
                heights.push((row * 5 + col) as f32);
            }
        }
        let field = Heightfield::new(Vec2::ZERO, 2.0, 5, 5, heights);
        let lod = field.to_mesh_lod(2);
        // Nodes 0, 2, 4 survive in each direction: a 3x3 grid.
        assert_eq!(lod.vertices.len(), 9);
        for v in &lod.vertices {
            let col = (v.position.x / 2.0).round() as i64;
            let row = (v.position.z / 2.0).round() as i64;
            let expected = (row * 5 + col) as f32;
            assert_eq!(v.position.y, expected, "node ({col},{row})");
        }
    }

    #[test]
    fn to_mesh_lod_includes_the_far_edge_even_off_stride() {
        // 5 nodes (indices 0..=4), stride 3 lands on 0, 3 — 4 must still be kept.
        let field = bumpy_field(5, 2);
        let lod = field.to_mesh_lod(3);
        assert_eq!(lod.vertices.len(), 3 * 2);
        let max_x = lod
            .vertices
            .iter()
            .map(|v| v.position.x)
            .fold(f32::MIN, f32::max);
        assert_eq!(max_x, field.origin.x + 4.0 * field.cell);
    }

    #[test]
    fn to_mesh_lod_stride_one_matches_to_mesh() {
        let field = bumpy_field(4, 4);
        let full = field.to_mesh();
        let lod = field.to_mesh_lod(1);
        assert_eq!(full.vertices.len(), lod.vertices.len());
        assert_eq!(full.indices, lod.indices);
        for (a, b) in full.vertices.iter().zip(&lod.vertices) {
            assert_eq!(a.position, b.position);
        }
    }
}
