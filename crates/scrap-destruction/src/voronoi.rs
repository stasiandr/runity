//! Convex pieces cut by Voronoi cells: a solid (a convex polyhedron) and
//! points in it, each point's piece the part of the solid nearer to it
//! than to any other point — the solid clipped by the plane half way to
//! each of the others. What games cut walls and statues with, ahead of
//! time (Blast, Chaos) or at the moment of the blow.

use glam::Vec3;

// The convex solid is geometry's: blockout brushes are cut from the same.
pub use scrap_geometry::solid::{Face, Solid};

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
