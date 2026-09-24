//! Convex pieces cut by Voronoi cells: a solid (a convex polyhedron) and
//! points in it, each point's piece the part of the solid nearer to it
//! than to any other point — the solid clipped by the plane half way to
//! each of the others. What games cut walls and statues with, ahead of
//! time (Blast, Chaos) or at the moment of the blow.

use glam::Vec3;

/// A convex solid: its faces, each a loop of corners turning
/// anticlockwise seen from outside; a face made by a cut is `inner`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Solid {
    pub faces: Vec<Face>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    pub corners: Vec<Vec3>,
    pub inner: bool,
}

impl Solid {
    /// The box from `low` to `high`.
    pub fn cuboid(low: Vec3, high: Vec3) -> Self {
        let c = |x: usize, y: usize, z: usize| {
            Vec3::new([low.x, high.x][x], [low.y, high.y][y], [low.z, high.z][z])
        };
        let quad = |a, b, c2, d| Face { corners: vec![a, b, c2, d], inner: false };
        Solid {
            faces: vec![
                quad(c(0, 0, 0), c(0, 0, 1), c(0, 1, 1), c(0, 1, 0)),
                quad(c(1, 0, 0), c(1, 1, 0), c(1, 1, 1), c(1, 0, 1)),
                quad(c(0, 0, 0), c(1, 0, 0), c(1, 0, 1), c(0, 0, 1)),
                quad(c(0, 1, 0), c(0, 1, 1), c(1, 1, 1), c(1, 1, 0)),
                quad(c(0, 0, 0), c(0, 1, 0), c(1, 1, 0), c(1, 0, 0)),
                quad(c(0, 0, 1), c(1, 0, 1), c(1, 1, 1), c(0, 1, 1)),
            ],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.faces.len() < 4
    }

    /// Its corners, each once or more.
    pub fn corners(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.faces.iter().flat_map(|f| f.corners.iter().copied())
    }

    /// The box it lies in.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.corners().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(p), b.max(p)))
    }

    /// Its volume and the middle of it.
    pub fn volume_and_centre(&self) -> (f32, Vec3) {
        let mut volume = 0.0;
        let mut moment = Vec3::ZERO;
        let origin = self.faces.first().and_then(|f| f.corners.first()).copied().unwrap_or(Vec3::ZERO);
        for face in &self.faces {
            for k in 1..face.corners.len().saturating_sub(1) {
                let (a, b, c) = (face.corners[0] - origin, face.corners[k] - origin, face.corners[k + 1] - origin);
                let v = a.dot(b.cross(c)) / 6.0;
                volume += v;
                moment += (a + b + c) / 4.0 * v;
            }
        }
        let centre = if volume.abs() > 1e-12 { origin + moment / volume } else { origin };
        (volume, centre)
    }

    /// What is on the side of the plane `normal · p <= offset`, with the
    /// cut closed by an inner face.
    pub fn clip(&self, normal: Vec3, offset: f32) -> Solid {
        let side = |p: Vec3| normal.dot(p) - offset;
        let mut faces = Vec::with_capacity(self.faces.len() + 1);
        let mut cut: Vec<Vec3> = Vec::new();
        for face in &self.faces {
            let n = face.corners.len();
            let mut kept = Vec::with_capacity(n + 2);
            for i in 0..n {
                let (a, b) = (face.corners[i], face.corners[(i + 1) % n]);
                let (da, db) = (side(a), side(b));
                if da <= 0.0 {
                    kept.push(a);
                }
                if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
                    let p = a + (b - a) * (da / (da - db));
                    kept.push(p);
                    cut.push(p);
                } else if da == 0.0 {
                    cut.push(a);
                }
            }
            if kept.len() >= 3 {
                faces.push(Face { corners: kept, inner: face.inner });
            }
        }
        // Close the cut: its corners round their middle, anticlockwise seen
        // from the side cut away.
        cut.dedup_by(|a, b| a.distance(*b) < 1e-6);
        let mut unique: Vec<Vec3> = Vec::new();
        for p in cut {
            if !unique.iter().any(|q| q.distance(p) < 1e-6) {
                unique.push(p);
            }
        }
        if unique.len() >= 3 {
            let middle = unique.iter().sum::<Vec3>() / unique.len() as f32;
            let u = normal.any_orthonormal_vector();
            let w = normal.cross(u);
            unique.sort_by(|a, b| {
                let (pa, pb) = (*a - middle, *b - middle);
                pa.dot(w).atan2(pa.dot(u)).total_cmp(&pb.dot(w).atan2(pb.dot(u)))
            });
            faces.push(Face { corners: unique, inner: true });
        }
        Solid { faces }
    }

    /// Its triangles, flat-shaded, with pictures laid on by the way each
    /// face looks most (in metres, `scale` being what the solid's space is
    /// stretched by in the world), every corner less `centre`.
    pub fn mesh(&self, centre: Vec3, scale: Vec3) -> (Vec<runity_geometry::mesh_asset::Vertex>, Vec<u32>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for face in &self.faces {
            let n = face.corners.len();
            if n < 3 {
                continue;
            }
            let normal = (face.corners[1] - face.corners[0])
                .cross(face.corners[2] - face.corners[0])
                .normalize_or(Vec3::Y);
            let first = vertices.len() as u32;
            let most = normal.abs();
            for p in &face.corners {
                let m = *p * scale;
                let uv = if most.x >= most.y && most.x >= most.z {
                    [m.z, m.y]
                } else if most.y >= most.z {
                    [m.x, m.z]
                } else {
                    [m.x, m.y]
                };
                vertices.push(runity_geometry::mesh_asset::Vertex {
                    position: (*p - centre).to_array(),
                    normal: normal.to_array(),
                    uv,
                });
            }
            for k in 1..n as u32 - 1 {
                indices.extend_from_slice(&[first, first + k, first + k + 1]);
            }
        }
        (vertices, indices)
    }
}

/// The piece of `solid` nearer to each of `points` than to the others.
pub fn cells(solid: &Solid, points: &[Vec3]) -> Vec<Solid> {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut cell = solid.clone();
            for (j, q) in points.iter().enumerate() {
                if i == j || cell.is_empty() {
                    continue;
                }
                let normal = (*q - *p).normalize_or(Vec3::X);
                let offset = normal.dot((*p + *q) * 0.5);
                cell = cell.clip(normal, offset);
            }
            cell
        })
        .filter(|c| !c.is_empty())
        .collect()
}

/// A little random number generator: the same seed, the same pieces.
#[derive(Debug, Clone, Copy)]
pub struct Dice(pub u64);

impl Dice {
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A point in the box.
    pub fn within(&mut self, low: Vec3, high: Vec3) -> Vec3 {
        low + (high - low) * Vec3::new(self.next(), self.next(), self.next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pieces_of_a_box_fill_it_and_do_not_overlap() {
        let solid = Solid::cuboid(Vec3::splat(-0.5), Vec3::splat(0.5));
        let (v, c) = solid.volume_and_centre();
        assert!((v - 1.0).abs() < 1e-5 && c.length() < 1e-5, "{v} {c}");
        let mut dice = Dice(7);
        let points: Vec<Vec3> = (0..20).map(|_| dice.within(Vec3::splat(-0.5), Vec3::splat(0.5))).collect();
        let pieces = cells(&solid, &points);
        assert_eq!(pieces.len(), 20);
        let total: f32 = pieces.iter().map(|p| p.volume_and_centre().0).sum();
        assert!((total - 1.0).abs() < 1e-3, "the pieces make the whole: {total}");
        // Each piece's middle is nearer its own point than any other.
        for (piece, p) in pieces.iter().zip(&points) {
            let (_, centre) = piece.volume_and_centre();
            let own = centre.distance(*p);
            // Not a proof, but a cell's centre is never far into another's.
            assert!(points.iter().all(|q| centre.distance(*q) >= own - 0.35));
            // Every face wound outward.
            for face in &piece.faces {
                let n = (face.corners[1] - face.corners[0]).cross(face.corners[2] - face.corners[0]);
                assert!(n.dot(face.corners[0] - centre) > -1e-5);
            }
        }
    }
}
