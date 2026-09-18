use crate::color::Color;
use crate::shader::Vertex;
use runity_math::{Vec2, Vec3, Vec4};

/// An indexed triangle list. Front faces wind counter-clockwise.
#[derive(Debug, Default, Clone)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        debug_assert!(
            indices.len() % 3 == 0,
            "index count must be a multiple of 3"
        );
        Self { vertices, indices }
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn set_color(&mut self, color: Color) {
        for v in &mut self.vertices {
            v.color = color;
        }
    }

    /// Replace normals with area-weighted face normals.
    ///
    /// Vertices no triangle references keep the normal they were authored with,
    /// so unused seam and pole vertices do not end up with a zero normal.
    pub fn recompute_normals(&mut self) {
        let mut accum = vec![Vec3::ZERO; self.vertices.len()];
        for tri in self.indices.chunks_exact(3) {
            let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            let (p0, p1, p2) = (
                self.vertices[i0].position,
                self.vertices[i1].position,
                self.vertices[i2].position,
            );
            // Not normalized on purpose: the magnitude is twice the triangle
            // area, which is exactly the weight we want.
            let n = (p1 - p0).cross(p2 - p0);
            accum[i0] += n;
            accum[i1] += n;
            accum[i2] += n;
        }
        for (v, n) in self.vertices.iter_mut().zip(accum) {
            if n.length_squared() > 0.0 {
                v.normal = n.normalized();
            }
        }
    }

    /// Recompute per-vertex tangents from the UV layout.
    ///
    /// A normal map stores directions in *tangent space* — relative to how the
    /// texture is laid out on the surface — so the shader needs to know which
    /// way "along U" points in world space. This is the standard
    /// area-weighted accumulation (Lengyel): solve the 2x2 UV system per
    /// triangle, average per vertex, then Gram-Schmidt against the normal.
    pub fn recompute_tangents(&mut self) {
        let mut tangents = vec![Vec3::ZERO; self.vertices.len()];
        let mut bitangents = vec![Vec3::ZERO; self.vertices.len()];

        for tri in self.indices.chunks_exact(3) {
            let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            let (v0, v1, v2) = (&self.vertices[i0], &self.vertices[i1], &self.vertices[i2]);
            let edge1 = v1.position - v0.position;
            let edge2 = v2.position - v0.position;
            let duv1 = v1.uv - v0.uv;
            let duv2 = v2.uv - v0.uv;

            let determinant = duv1.x * duv2.y - duv2.x * duv1.y;
            if determinant.abs() < 1e-12 {
                continue; // degenerate UVs contribute nothing
            }
            let r = 1.0 / determinant;
            let tangent = (edge1 * duv2.y - edge2 * duv1.y) * r;
            let bitangent = (edge2 * duv1.x - edge1 * duv2.x) * r;
            for index in [i0, i1, i2] {
                tangents[index] += tangent;
                bitangents[index] += bitangent;
            }
        }

        for (vertex, (tangent, bitangent)) in self
            .vertices
            .iter_mut()
            .zip(tangents.into_iter().zip(bitangents))
        {
            if tangent.length_squared() <= 0.0 {
                continue;
            }
            let n = vertex.normal;
            // Gram-Schmidt: the tangent must be perpendicular to the normal.
            let t = (tangent - n * n.dot(tangent)).normalized();
            // Handedness tells the shader which way the bitangent runs.
            let handedness = if n.cross(t).dot(bitangent) < 0.0 {
                -1.0
            } else {
                1.0
            };
            vertex.tangent = Vec4::new(t.x, t.y, t.z, handedness);
        }
    }

    /// Axis-aligned cube centered on the origin, with per-face normals and UVs.
    pub fn cube(size: f32) -> Self {
        let h = size * 0.5;
        let faces: [([Vec3; 4], Vec3); 6] = [
            // +Z
            (
                [
                    Vec3::new(-h, -h, h),
                    Vec3::new(h, -h, h),
                    Vec3::new(h, h, h),
                    Vec3::new(-h, h, h),
                ],
                Vec3::Z,
            ),
            // -Z
            (
                [
                    Vec3::new(h, -h, -h),
                    Vec3::new(-h, -h, -h),
                    Vec3::new(-h, h, -h),
                    Vec3::new(h, h, -h),
                ],
                -Vec3::Z,
            ),
            // +X
            (
                [
                    Vec3::new(h, -h, h),
                    Vec3::new(h, -h, -h),
                    Vec3::new(h, h, -h),
                    Vec3::new(h, h, h),
                ],
                Vec3::X,
            ),
            // -X
            (
                [
                    Vec3::new(-h, -h, -h),
                    Vec3::new(-h, -h, h),
                    Vec3::new(-h, h, h),
                    Vec3::new(-h, h, -h),
                ],
                -Vec3::X,
            ),
            // +Y
            (
                [
                    Vec3::new(-h, h, h),
                    Vec3::new(h, h, h),
                    Vec3::new(h, h, -h),
                    Vec3::new(-h, h, -h),
                ],
                Vec3::Y,
            ),
            // -Y
            (
                [
                    Vec3::new(-h, -h, -h),
                    Vec3::new(h, -h, -h),
                    Vec3::new(h, -h, h),
                    Vec3::new(-h, -h, h),
                ],
                -Vec3::Y,
            ),
        ];

        let uvs = [
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 0.0),
        ];
        let mut vertices = Vec::with_capacity(24);
        let mut indices = Vec::with_capacity(36);
        for (corners, normal) in faces {
            let base = vertices.len() as u32;
            for (corner, uv) in corners.into_iter().zip(uvs) {
                vertices.push(Vertex::new(corner, normal, uv));
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        let mut mesh = Self::new(vertices, indices);
        mesh.recompute_tangents();
        mesh
    }

    /// Grid on the XZ plane facing +Y, centered on the origin.
    pub fn plane(size: f32, subdivisions: usize) -> Self {
        let n = subdivisions.max(1);
        let step = size / n as f32;
        let h = size * 0.5;
        let mut vertices = Vec::with_capacity((n + 1) * (n + 1));
        for j in 0..=n {
            for i in 0..=n {
                let x = -h + i as f32 * step;
                let z = -h + j as f32 * step;
                vertices.push(Vertex::new(
                    Vec3::new(x, 0.0, z),
                    Vec3::Y,
                    Vec2::new(i as f32 / n as f32, j as f32 / n as f32),
                ));
            }
        }
        let mut indices = Vec::with_capacity(n * n * 6);
        let stride = (n + 1) as u32;
        for j in 0..n as u32 {
            for i in 0..n as u32 {
                let a = j * stride + i;
                let b = a + 1;
                let c = a + stride;
                let d = c + 1;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        let mut mesh = Self::new(vertices, indices);
        mesh.recompute_tangents();
        mesh
    }

    /// UV sphere with `segments` meridians and `rings` parallels.
    pub fn sphere(radius: f32, segments: usize, rings: usize) -> Self {
        let segments = segments.max(3);
        let rings = rings.max(2);
        let mut vertices = Vec::with_capacity((segments + 1) * (rings + 1));
        for i in 0..=rings {
            let v = i as f32 / rings as f32;
            let phi = v * core::f32::consts::PI;
            let (sin_phi, cos_phi) = phi.sin_cos();
            for j in 0..=segments {
                let u = j as f32 / segments as f32;
                let theta = u * core::f32::consts::TAU;
                let (sin_theta, cos_theta) = theta.sin_cos();
                let normal = Vec3::new(sin_phi * cos_theta, cos_phi, sin_phi * sin_theta);
                vertices.push(Vertex::new(normal * radius, normal, Vec2::new(u, v)));
            }
        }
        let mut indices = Vec::with_capacity(segments * rings * 6);
        let stride = (segments + 1) as u32;
        for i in 0..rings as u32 {
            for j in 0..segments as u32 {
                let a = i * stride + j;
                let b = a + 1;
                let c = a + stride;
                let d = c + 1;
                // At the poles the quad collapses to a triangle: both vertices
                // of one edge sit on the same point, so skip that half.
                if i != 0 {
                    indices.extend_from_slice(&[a, b, c]);
                }
                if i != rings as u32 - 1 {
                    indices.extend_from_slice(&[b, d, c]);
                }
            }
        }
        let mut mesh = Self::new(vertices, indices);
        mesh.recompute_tangents();
        mesh
    }

    /// Parse a Wavefront OBJ file: `v`, `vt`, `vn` and `f` (polygons are
    /// fan-triangulated). Everything else — materials, groups, smoothing — is
    /// skipped.
    pub fn from_obj(source: &str) -> Result<Self, ObjError> {
        let mut positions: Vec<Vec3> = Vec::new();
        let mut uvs: Vec<Vec2> = Vec::new();
        let mut normals: Vec<Vec3> = Vec::new();
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut had_normals = false;

        for (line_no, raw) in source.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let err = |msg: &str| ObjError {
                line: line_no + 1,
                message: msg.to_string(),
            };
            let mut parts = line.split_whitespace();
            let Some(tag) = parts.next() else { continue };
            match tag {
                "v" => positions.push(parse_vec3(&mut parts).ok_or_else(|| err("bad `v`"))?),
                "vn" => normals.push(parse_vec3(&mut parts).ok_or_else(|| err("bad `vn`"))?),
                "vt" => {
                    let u: f32 = parts
                        .next()
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| err("bad `vt`"))?;
                    let v: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    // OBJ's V axis points up; ours points down.
                    uvs.push(Vec2::new(u, 1.0 - v));
                }
                "f" => {
                    let mut face = Vec::new();
                    for token in parts {
                        let mut fields = token.split('/');
                        let pi = resolve_index(fields.next(), positions.len())
                            .ok_or_else(|| err("bad face position index"))?;
                        let ti = fields
                            .next()
                            .and_then(|f| resolve_index(Some(f), uvs.len()));
                        let ni = fields
                            .next()
                            .and_then(|f| resolve_index(Some(f), normals.len()));
                        let position = *positions
                            .get(pi)
                            .ok_or_else(|| err("position index out of range"))?;
                        let uv = ti.and_then(|i| uvs.get(i).copied()).unwrap_or(Vec2::ZERO);
                        let normal = match ni.and_then(|i| normals.get(i).copied()) {
                            Some(n) => {
                                had_normals = true;
                                n
                            }
                            None => Vec3::ZERO,
                        };
                        face.push(Vertex::new(position, normal, uv));
                    }
                    if face.len() < 3 {
                        return Err(err("face needs at least 3 vertices"));
                    }
                    let base = vertices.len() as u32;
                    vertices.extend_from_slice(&face);
                    for i in 1..face.len() as u32 - 1 {
                        indices.extend_from_slice(&[base, base + i, base + i + 1]);
                    }
                }
                _ => {}
            }
        }

        let mut mesh = Self::new(vertices, indices);
        if !had_normals {
            mesh.recompute_normals();
        }
        mesh.recompute_tangents();
        Ok(mesh)
    }
}

fn parse_vec3<'a>(parts: &mut impl Iterator<Item = &'a str>) -> Option<Vec3> {
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    let z = parts.next()?.parse().ok()?;
    Some(Vec3::new(x, y, z))
}

/// OBJ indices are 1-based; negative values count back from the end.
fn resolve_index(field: Option<&str>, len: usize) -> Option<usize> {
    let field = field?.trim();
    if field.is_empty() {
        return None;
    }
    let i: i64 = field.parse().ok()?;
    match i {
        0 => None,
        i if i > 0 => Some(i as usize - 1),
        i => len.checked_sub(i.unsigned_abs() as usize),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjError {
    pub line: usize,
    pub message: String,
}

impl core::fmt::Display for ObjError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "OBJ parse error on line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ObjError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every face of a closed mesh must have its normal pointing outward, which
    /// for a shape centered on the origin means agreeing with the centroid.
    fn assert_outward(mesh: &Mesh) {
        for tri in mesh.indices.chunks_exact(3) {
            let p: Vec<Vec3> = tri
                .iter()
                .map(|i| mesh.vertices[*i as usize].position)
                .collect();
            let face_normal = (p[1] - p[0]).cross(p[2] - p[0]);
            let centroid = (p[0] + p[1] + p[2]) * (1.0 / 3.0);
            assert!(
                face_normal.dot(centroid) > 0.0,
                "face {p:?} winds inward (normal {face_normal:?})"
            );
        }
    }

    #[test]
    fn cube_has_six_quads_wound_outward() {
        let cube = Mesh::cube(2.0);
        assert_eq!(cube.vertices.len(), 24);
        assert_eq!(cube.triangle_count(), 12);
        assert_outward(&cube);
    }

    #[test]
    fn sphere_is_wound_outward_and_on_the_radius() {
        let s = Mesh::sphere(1.5, 8, 6);
        assert_outward(&s);
        for v in &s.vertices {
            assert!((v.position.length() - 1.5).abs() < 1e-4);
        }
    }

    #[test]
    fn plane_faces_up() {
        let p = Mesh::plane(2.0, 2);
        for tri in p.indices.chunks_exact(3) {
            let v: Vec<Vec3> = tri
                .iter()
                .map(|i| p.vertices[*i as usize].position)
                .collect();
            assert!((v[1] - v[0]).cross(v[2] - v[0]).y > 0.0);
        }
    }

    #[test]
    fn recomputed_normals_match_authored_ones() {
        let mut s = Mesh::sphere(1.0, 16, 12);
        let authored: Vec<Vec3> = s.vertices.iter().map(|v| v.normal).collect();
        s.recompute_normals();
        for (a, v) in authored.iter().zip(&s.vertices) {
            // Pole and seam vertices only touch triangles on one side, so their
            // averaged normal is tilted by up to half a quad; the honest
            // assertion is a direction check, not an equality.
            assert!(a.dot(v.normal) > 0.95, "{a:?} vs {:?}", v.normal);
        }
    }

    #[test]
    fn tangents_follow_the_uv_layout() {
        let cube = Mesh::cube(2.0);
        for vertex in &cube.vertices {
            let t = Vec3::new(vertex.tangent.x, vertex.tangent.y, vertex.tangent.z);
            assert!((t.length() - 1.0).abs() < 1e-4, "tangents are unit length");
            assert!(
                t.dot(vertex.normal).abs() < 1e-4,
                "and perpendicular to the normal"
            );
            assert!(vertex.tangent.w.abs() == 1.0, "handedness is +1 or -1");
        }
        // The +Z face has U running along +X, so its tangent must too.
        let front = cube
            .vertices
            .iter()
            .find(|v| v.normal == Vec3::Z)
            .expect("a +Z face");
        assert!(front.tangent.x > 0.9, "{:?}", front.tangent);
    }

    #[test]
    fn obj_triangle_round_trips() {
        let src = "\
# a quad
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
vt 0 0
vt 1 0
vt 1 1
vt 0 1
vn 0 0 1
f 1/1/1 2/2/1 3/3/1 4/4/1
";
        let mesh = Mesh::from_obj(src).expect("parses");
        assert_eq!(mesh.triangle_count(), 2, "the quad is fan-triangulated");
        assert_eq!(mesh.vertices[0].normal, Vec3::Z);
        // OBJ's V axis is flipped on import.
        assert_eq!(mesh.vertices[0].uv, Vec2::new(0.0, 1.0));
    }

    #[test]
    fn obj_accepts_negative_indices_and_missing_normals() {
        let src = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3 -2 -1\n";
        let mesh = Mesh::from_obj(src).expect("parses");
        assert_eq!(mesh.triangle_count(), 1);
        assert!(
            (mesh.vertices[0].normal - Vec3::Z).length() < 1e-5,
            "normals are generated"
        );
    }

    #[test]
    fn obj_reports_the_failing_line() {
        let err = Mesh::from_obj("v 0 0 0\nv oops\n").unwrap_err();
        assert_eq!(err.line, 2);
    }
}
