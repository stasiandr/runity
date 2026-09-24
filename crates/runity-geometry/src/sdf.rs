//! A signed distance field of a scene (docs/simulation.md, item 12): at
//! each point of a grid, how far the nearest solid surface is — negative
//! inside. One field for everyone: soft things collide with it (a cloth on
//! a statue, water round a rock, whatever mesh it is), and the render
//! darkens creases by it and softens shadows by marching it (SDF ambient
//! occlusion and soft shadows, as Unreal's mesh distance fields).
//!
//! Baked from what is solid: analytic shapes by their exact distance, and
//! meshes in a narrow band round their triangles, the sign from the
//! nearest triangle's side (far from any triangle the field only says
//! "outside, at least this far").

use glam::Vec3;

/// A grid of distances.
#[derive(Debug, Clone, PartialEq)]
pub struct Sdf {
    /// The grid's first point, in the world.
    pub low: Vec3,
    /// Metres between points.
    pub cell: f32,
    pub size: [usize; 3],
    pub values: Vec<f32>,
}

impl Sdf {
    /// A field from `low` to `high`, `cell` metres apart, everywhere at
    /// least `far` from anything.
    pub fn new(low: Vec3, high: Vec3, cell: f32, far: f32) -> Self {
        let cell = cell.max(1e-3);
        let n = ((high - low) / cell).ceil().max(Vec3::ONE) + Vec3::ONE;
        let size = [n.x as usize, n.y as usize, n.z as usize];
        Self { low, cell, size, values: vec![far; size[0] * size[1] * size[2]] }
    }

    pub fn high(&self) -> Vec3 {
        self.low + Vec3::new((self.size[0] - 1) as f32, (self.size[1] - 1) as f32, (self.size[2] - 1) as f32) * self.cell
    }

    fn index(&self, x: usize, y: usize, z: usize) -> usize {
        (z * self.size[1] + y) * self.size[0] + x
    }

    pub fn point(&self, x: usize, y: usize, z: usize) -> Vec3 {
        self.low + Vec3::new(x as f32, y as f32, z as f32) * self.cell
    }

    /// Each point the nearer of what it has and what `distance` says: an
    /// analytic shape, or several.
    pub fn add(&mut self, distance: impl Fn(Vec3) -> f32) {
        for z in 0..self.size[2] {
            for y in 0..self.size[1] {
                for x in 0..self.size[0] {
                    let i = self.index(x, y, z);
                    let d = distance(self.point(x, y, z));
                    if d < self.values[i] {
                        self.values[i] = d;
                    }
                }
            }
        }
    }

    /// A mesh's triangles in, within `band` metres of them: each point near
    /// a triangle the distance to the nearest, negative on the side its
    /// normal does not face.
    pub fn add_triangles(&mut self, vertices: &[Vec3], triangles: &[[u32; 3]], band: f32) {
        let mut best = vec![f32::MAX; self.values.len()];
        // The way from the nearest point, and the normals of every triangle
        // that point is nearest on: at an edge or a corner several are, and
        // one face alone may say "inside" of a point plainly outside (above
        // a cone's tip). Their sum is the corner's pseudo-normal (Bærentzen
        // and Aanæs 2005, unweighted).
        let mut away = vec![Vec3::ZERO; self.values.len()];
        let mut normals = vec![Vec3::ZERO; self.values.len()];
        let tie = self.cell * 1e-3;
        for t in triangles {
            let [a, b, c] = t.map(|i| vertices[i as usize]);
            let low = a.min(b).min(c) - Vec3::splat(band);
            let high = a.max(b).max(c) + Vec3::splat(band);
            let from = ((low - self.low) / self.cell).floor().max(Vec3::ZERO);
            let to = ((high - self.low) / self.cell).ceil();
            let normal = (b - a).cross(c - a).normalize_or_zero();
            for z in from.z as usize..=(to.z as usize).min(self.size[2] - 1) {
                for y in from.y as usize..=(to.y as usize).min(self.size[1] - 1) {
                    for x in from.x as usize..=(to.x as usize).min(self.size[0] - 1) {
                        let p = self.point(x, y, z);
                        let q = closest_on_triangle(p, a, b, c);
                        let d = p.distance(q);
                        let i = self.index(x, y, z);
                        if d < best[i] - tie {
                            best[i] = d;
                            away[i] = p - q;
                            normals[i] = normal;
                        } else if d <= best[i] + tie {
                            normals[i] += normal;
                        }
                    }
                }
            }
        }
        let sign: Vec<f32> = (0..self.values.len())
            .map(|i| if away[i].dot(normals[i]) < 0.0 { -1.0 } else { 1.0 })
            .collect();
        // Far from every triangle the sign is not known from one: what the
        // outside cannot reach without crossing the mesh's band from its
        // outer side is inside it. Flooded from the grid's faces through
        // what is far or on the outer side.
        let open = |i: usize| best[i] >= band || sign[i] > 0.0;
        let mut outside = vec![false; self.values.len()];
        let mut todo = Vec::new();
        let [nx, ny, nz] = self.size;
        for z in 0..nz {
            for y in 0..ny {
                for x in 0..nx {
                    let face = x == 0 || y == 0 || z == 0 || x == nx - 1 || y == ny - 1 || z == nz - 1;
                    let i = self.index(x, y, z);
                    if face && open(i) {
                        outside[i] = true;
                        todo.push((x, y, z));
                    }
                }
            }
        }
        while let Some((x, y, z)) = todo.pop() {
            let mut visit = |x: usize, y: usize, z: usize| {
                let i = (z * ny + y) * nx + x;
                if !outside[i] && open(i) {
                    outside[i] = true;
                    todo.push((x, y, z));
                }
            };
            if x > 0 { visit(x - 1, y, z); }
            if x + 1 < nx { visit(x + 1, y, z); }
            if y > 0 { visit(x, y - 1, z); }
            if y + 1 < ny { visit(x, y + 1, z); }
            if z > 0 { visit(x, y, z - 1); }
            if z + 1 < nz { visit(x, y, z + 1); }
        }
        for i in 0..self.values.len() {
            let d = if best[i] < band {
                best[i] * sign[i]
            } else if !outside[i] {
                -band
            } else {
                continue;
            };
            if d < self.values[i] {
                self.values[i] = d;
            }
        }
    }

    /// The distance at a point, trilinear; outside the grid, the grid's
    /// edge's.
    pub fn sample(&self, p: Vec3) -> f32 {
        let g = ((p - self.low) / self.cell).clamp(Vec3::ZERO, Vec3::new((self.size[0] - 1) as f32, (self.size[1] - 1) as f32, (self.size[2] - 1) as f32) - Vec3::splat(1e-3));
        let (x, y, z) = (g.x as usize, g.y as usize, g.z as usize);
        let f = g - Vec3::new(x as f32, y as f32, z as f32);
        let (x1, y1, z1) = ((x + 1).min(self.size[0] - 1), (y + 1).min(self.size[1] - 1), (z + 1).min(self.size[2] - 1));
        let v = |a: usize, b: usize, c: usize| self.values[self.index(a, b, c)];
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let x00 = lerp(v(x, y, z), v(x1, y, z), f.x);
        let x10 = lerp(v(x, y1, z), v(x1, y1, z), f.x);
        let x01 = lerp(v(x, y, z1), v(x1, y, z1), f.x);
        let x11 = lerp(v(x, y1, z1), v(x1, y1, z1), f.x);
        lerp(lerp(x00, x10, f.y), lerp(x01, x11, f.y), f.z)
    }

    /// Which way is away from the nearest surface.
    pub fn gradient(&self, p: Vec3) -> Vec3 {
        let h = self.cell * 0.5;
        Vec3::new(
            self.sample(p + Vec3::X * h) - self.sample(p - Vec3::X * h),
            self.sample(p + Vec3::Y * h) - self.sample(p - Vec3::Y * h),
            self.sample(p + Vec3::Z * h) - self.sample(p - Vec3::Z * h),
        )
        .normalize_or(Vec3::Y)
    }

    /// Whether `p` is in it.
    pub fn contains(&self, p: Vec3) -> bool {
        p.cmpge(self.low).all() && p.cmple(self.high()).all()
    }

    /// Its distances as bytes for a texture, `range` metres either side of
    /// the surface mapped to 0–255 with the surface at 128.
    pub fn bytes(&self, range: f32) -> Vec<u8> {
        self.values.iter().map(|d| ((d / range * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0) as u8).collect()
    }
}

/// The point of triangle `a b c` nearest `p` (Ericson, Real-Time Collision
/// Detection, 5.1.5).
pub fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ball_mesh_baked_in_gives_its_distance_near_it_and_its_inside_negative() {
        let ball = crate::builtin::sphere(0.5, 32, 16);
        let vertices: Vec<Vec3> = ball.vertices.iter().map(|v| Vec3::from_array(v.position)).collect();
        let triangles: Vec<[u32; 3]> = ball.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
        let mut sdf = Sdf::new(Vec3::splat(-1.0), Vec3::splat(1.0), 0.05, 1.0);
        sdf.add_triangles(&vertices, &triangles, 0.4);
        for p in [Vec3::new(0.7, 0.0, 0.0), Vec3::new(0.0, 0.55, 0.1), Vec3::new(0.3, 0.2, 0.0)] {
            let exact = p.length() - 0.5;
            assert!((sdf.sample(p) - exact).abs() < 0.03, "{p}: {} vs {exact}", sdf.sample(p));
        }
        assert!(sdf.gradient(Vec3::new(0.6, 0.0, 0.0)).distance(Vec3::X) < 0.05);
        // And an analytic floor, the nearer of the two.
        sdf.add(|p| p.y + 0.8);
        assert!((sdf.sample(Vec3::new(0.0, -0.75, 0.0)) - 0.05).abs() < 1e-3);
        // Deep inside, further than the band: still inside.
        let mut thin = Sdf::new(Vec3::splat(-1.0), Vec3::splat(1.0), 0.05, 1.0);
        thin.add_triangles(&vertices, &triangles, 0.15);
        assert!(thin.sample(Vec3::ZERO) < 0.0 && thin.sample(Vec3::splat(0.9)) > 0.0);
        let bytes = sdf.bytes(0.5);
        assert_eq!(bytes.len(), sdf.values.len());
    }

    #[test]
    fn nothing_over_a_cones_tip_reads_as_inside() {
        // Nearest the tip, every side's face meets there; one face alone
        // says "inside" of points plainly above it.
        let cone = crate::builtin::by_name("builtin:cone").unwrap();
        let vertices: Vec<Vec3> = cone.vertices.iter().map(|v| Vec3::from_array(v.position) * Vec3::new(1.2, 1.4, 1.2)).collect();
        let triangles: Vec<[u32; 3]> = cone.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
        let top = vertices.iter().map(|v| v.y).fold(f32::MIN, f32::max);
        let mut sdf = Sdf::new(Vec3::splat(-2.0), Vec3::splat(2.0), 0.07, 1.0);
        sdf.add_triangles(&vertices, &triangles, 0.42);
        for z in 0..sdf.size[2] {
            for y in 0..sdf.size[1] {
                for x in 0..sdf.size[0] {
                    let p = sdf.point(x, y, z);
                    if p.y > top + 0.02 {
                        assert!(sdf.values[sdf.index(x, y, z)] > 0.0, "over the tip, inside at {p}");
                    }
                }
            }
        }
    }
}
