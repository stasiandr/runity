//! Surfaces of fields: a scalar field on a grid — how much water there is
//! at each point, a distance to the nearest thing — turned into triangles
//! where it crosses a level. What fluids are drawn with, and what an SDF
//! is looked at with.
//!
//! Naive Surface Nets (Gibson 1998; a dual cousin of marching cubes): one
//! vertex in each cell the surface passes through, at the mean of where it
//! crosses the cell's edges, and a quad across each crossed edge between
//! the four cells round it. No case tables, smooth, and every vertex is
//! shared, so normals come out even.

use glam::Vec3;

use crate::mesh_asset::Vertex;

/// A scalar field on a grid of `size` points, `cell` metres apart, the
/// first at `origin`; x fastest, then y, then z.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub size: [usize; 3],
    pub origin: Vec3,
    pub cell: f32,
    pub values: Vec<f32>,
}

impl Field {
    pub fn new(size: [usize; 3], origin: Vec3, cell: f32) -> Self {
        Self { size, origin, cell, values: vec![0.0; size[0] * size[1] * size[2]] }
    }

    pub fn index(&self, x: usize, y: usize, z: usize) -> usize {
        (z * self.size[1] + y) * self.size[0] + x
    }

    pub fn at(&self, x: usize, y: usize, z: usize) -> f32 {
        self.values[self.index(x, y, z)]
    }

    /// The point of the grid at `(x, y, z)`, in metres.
    pub fn point(&self, x: usize, y: usize, z: usize) -> Vec3 {
        self.origin + Vec3::new(x as f32, y as f32, z as f32) * self.cell
    }

    /// Add a soft ball of `amount` at `p`, reaching `radius` metres: what
    /// a particle leaves in the field.
    pub fn splat(&mut self, p: Vec3, radius: f32, amount: f32) {
        let g = (p - self.origin) / self.cell;
        let r = (radius / self.cell).ceil() as i64;
        let (cx, cy, cz) = (g.x.round() as i64, g.y.round() as i64, g.z.round() as i64);
        let r2 = radius * radius;
        // The ball, not the box round it: a slice or a row wholly outside
        // is passed over, and a row is walked only across the ball's
        // chord (a point wider). Each point still in is summed exactly as
        // before — a distance's part is never more than the distance, so
        // nothing passed over could have added anything.
        for z in (cz - r).max(0)..=(cz + r).min(self.size[2] as i64 - 1) {
            let dz = (self.origin.z + z as f32 * self.cell) - p.z;
            let dz2 = dz * dz;
            if 1.0 - dz2 / r2 <= 0.0 {
                continue;
            }
            for y in (cy - r).max(0)..=(cy + r).min(self.size[1] as i64 - 1) {
                let dy = (self.origin.y + y as f32 * self.cell) - p.y;
                let dyz2 = dy * dy + dz2;
                if 1.0 - dyz2 / r2 <= 0.0 {
                    continue;
                }
                let half = (r2 - dyz2).max(0.0).sqrt() / self.cell;
                let from = ((g.x - half).floor() as i64 - 1).max(cx - r).max(0);
                let to = ((g.x + half).ceil() as i64 + 1).min(cx + r).min(self.size[0] as i64 - 1);
                for x in from..=to {
                    let q = self.point(x as usize, y as usize, z as usize);
                    let t = 1.0 - q.distance_squared(p) / r2;
                    if t > 0.0 {
                        let i = self.index(x as usize, y as usize, z as usize);
                        self.values[i] += amount * t * t * t;
                    }
                }
            }
        }
    }

    /// Where it crosses `level`, as triangles facing toward less: the
    /// outside of what is above the level.
    pub fn surface(&self, level: f32) -> (Vec<Vertex>, Vec<u32>) {
        let [nx, ny, nz] = self.size;
        if nx < 2 || ny < 2 || nz < 2 {
            return (Vec::new(), Vec::new());
        }
        let (cx, cy, cz) = (nx - 1, ny - 1, nz - 1);
        let cell_index = |x: usize, y: usize, z: usize| (z * cy + y) * cx + x;
        let mut vertex_of = vec![u32::MAX; cx * cy * cz];
        let mut vertices = Vec::new();
        const EDGES: [(usize, usize); 12] = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
        for z in 0..cz {
            for y in 0..cy {
                for x in 0..cx {
                    let corner = |c: usize| (x + (c & 1), y + ((c >> 1) & 1), z + ((c >> 2) & 1));
                    let values: [f32; 8] = std::array::from_fn(|c| {
                        let (a, b, d) = corner(c);
                        self.at(a, b, d) - level
                    });
                    let above = values.iter().filter(|v| **v > 0.0).count();
                    if above == 0 || above == 8 {
                        continue;
                    }
                    let mut sum = Vec3::ZERO;
                    let mut count = 0.0;
                    for (a, b) in EDGES {
                        let (va, vb) = (values[a], values[b]);
                        if (va > 0.0) != (vb > 0.0) {
                            let t = va / (va - vb);
                            let (pa, pb) = (corner(a), corner(b));
                            let pa = self.point(pa.0, pa.1, pa.2);
                            let pb = self.point(pb.0, pb.1, pb.2);
                            sum += pa + (pb - pa) * t;
                            count += 1.0;
                        }
                    }
                    vertex_of[cell_index(x, y, z)] = vertices.len() as u32;
                    let p = sum / count;
                    vertices.push(Vertex { position: p.to_array(), normal: [0.0, 1.0, 0.0], uv: [p.x, p.z] });
                }
            }
        }
        // A quad across each grid edge the level crosses, joining the four
        // cells that share it, turned so it faces toward less.
        let mut indices = Vec::new();
        for z in 0..nz {
            for y in 0..ny {
                for x in 0..nx {
                    let here = self.at(x, y, z) - level;
                    for axis in 0..3 {
                        let (x2, y2, z2) = match axis {
                            0 => (x + 1, y, z),
                            1 => (x, y + 1, z),
                            _ => (x, y, z + 1),
                        };
                        if x2 >= nx || y2 >= ny || z2 >= nz {
                            continue;
                        }
                        let there = self.at(x2, y2, z2) - level;
                        if (here > 0.0) == (there > 0.0) {
                            continue;
                        }
                        // The four cells round this edge.
                        let (u, v) = match axis {
                            0 => ((0, 1, 0), (0, 0, 1)),
                            1 => ((0, 0, 1), (1, 0, 0)),
                            _ => ((1, 0, 0), (0, 1, 0)),
                        };
                        let cell = |du: usize, dv: usize| -> Option<u32> {
                            let (px, py, pz) = (
                                x as i64 - (u.0 * du + v.0 * dv) as i64,
                                y as i64 - (u.1 * du + v.1 * dv) as i64,
                                z as i64 - (u.2 * du + v.2 * dv) as i64,
                            );
                            if px < 0 || py < 0 || pz < 0 || px as usize >= cx || py as usize >= cy || pz as usize >= cz {
                                return None;
                            }
                            let i = vertex_of[cell_index(px as usize, py as usize, pz as usize)];
                            (i != u32::MAX).then_some(i)
                        };
                        let (Some(a), Some(b), Some(c), Some(d)) = (cell(0, 0), cell(1, 0), cell(1, 1), cell(0, 1)) else {
                            continue;
                        };
                        if here > 0.0 {
                            indices.extend_from_slice(&[a, b, c, a, c, d]);
                        } else {
                            indices.extend_from_slice(&[a, c, b, a, d, c]);
                        }
                    }
                }
            }
        }
        // Normals from the triangles round each vertex.
        let mut normals = vec![Vec3::ZERO; vertices.len()];
        for t in indices.chunks_exact(3) {
            let p = |i: u32| Vec3::from_array(vertices[i as usize].position);
            let n = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
            for i in t {
                normals[*i as usize] += n;
            }
        }
        for (v, n) in vertices.iter_mut().zip(normals) {
            v.normal = n.normalize_or(Vec3::Y).to_array();
        }
        (vertices, indices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_splat_walking_only_the_ball_adds_what_walking_the_box_did() {
        // The whole box round each ball, every point tried: what `splat`
        // must match to the bit.
        fn by_box(field: &mut Field, p: Vec3, radius: f32, amount: f32) {
            let g = (p - field.origin) / field.cell;
            let r = (radius / field.cell).ceil() as i64;
            let (cx, cy, cz) = (g.x.round() as i64, g.y.round() as i64, g.z.round() as i64);
            for z in (cz - r).max(0)..=(cz + r).min(field.size[2] as i64 - 1) {
                for y in (cy - r).max(0)..=(cy + r).min(field.size[1] as i64 - 1) {
                    for x in (cx - r).max(0)..=(cx + r).min(field.size[0] as i64 - 1) {
                        let q = field.point(x as usize, y as usize, z as usize);
                        let t = 1.0 - q.distance_squared(p) / (radius * radius);
                        if t > 0.0 {
                            let i = field.index(x as usize, y as usize, z as usize);
                            field.values[i] += amount * t * t * t;
                        }
                    }
                }
            }
        }
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32
        };
        for (cell, radius) in [(0.06, 0.18), (0.05, 0.13), (0.1, 0.1), (0.07, 0.31)] {
            let origin = Vec3::new(-0.37, 0.11, 2.03);
            let mut ours = Field::new([24, 20, 22], origin, cell);
            let mut theirs = ours.clone();
            for _ in 0..400 {
                // Some off the edges of the grid, to try the clamping.
                let p = origin + Vec3::new(next(), next(), next()) * Vec3::new(26.0, 22.0, 24.0) * cell
                    - Vec3::splat(cell);
                ours.splat(p, radius, 1.0);
                by_box(&mut theirs, p, radius, 1.0);
            }
            assert!(ours.values.iter().zip(&theirs.values).all(|(a, b)| a.to_bits() == b.to_bits()));
            assert!(ours.values.iter().any(|v| *v > 0.0));
        }
    }

    #[test]
    fn a_ball_in_the_field_comes_out_a_closed_ball_facing_out() {
        let mut field = Field::new([24, 24, 24], Vec3::splat(-1.2), 0.1);
        for z in 0..24 {
            for y in 0..24 {
                for x in 0..24 {
                    let i = field.index(x, y, z);
                    field.values[i] = 0.7 - field.point(x, y, z).length();
                }
            }
        }
        let (vertices, indices) = field.surface(0.0);
        assert!(!indices.is_empty());
        for v in &vertices {
            let p = Vec3::from_array(v.position);
            assert!((p.length() - 0.7).abs() < 0.03, "on the ball: {p}");
            // Facing out: toward less.
            assert!(Vec3::from_array(v.normal).dot(p.normalize()) > 0.8);
        }
        // Wound as it faces.
        for t in indices.chunks_exact(3) {
            let p = |i: u32| Vec3::from_array(vertices[i as usize].position);
            let n = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
            assert!(n.dot(p(t[0])) > 0.0);
        }
        // Closed: every edge shared by two triangles.
        let mut edges = std::collections::HashMap::new();
        for t in indices.chunks_exact(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|n| *n == 2));
    }
}
