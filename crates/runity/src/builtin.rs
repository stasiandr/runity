//! Meshes the engine can produce without opening a file.
//!
//! Every renderer needs a ground to stand things on and a box to check a
//! shadow against, and needing an asset pipeline to get either one makes the
//! simplest test drag the whole world in behind it. These are built in code,
//! come out as ordinary [`MeshAsset`]s, and upload through the same path an
//! imported model does — so a test using them exercises the real code, not a
//! shortcut around it.
//!
//! All of them are wound counter-clockwise seen from outside, because back-face
//! culling is on. A mesh wound the other way does not vanish, which is what
//! makes the mistake expensive: it renders the inside of itself and looks
//! merely wrong, usually lit from the opposite side.

use crate::asset::{AssetId, Bounds, MeshAsset, Submesh, Vertex};

fn finish(name: &str, vertices: Vec<Vertex>, indices: Vec<u32>) -> MeshAsset {
    MeshAsset {
        id: AssetId::from_source(&format!("builtin:{name}"), 0),
        name: name.to_string(),
        bounds: Bounds::of(&vertices),
        submeshes: vec![Submesh {
            first_index: 0,
            index_count: indices.len() as u32,
            material: None,
        }],
        vertices,
        indices,
    }
}

/// A flat square on the ground, centred on the origin, facing up.
///
/// `segments` splits it into a grid. One quad is enough for flat colour, but
/// a shadow map or any per-vertex effect needs somewhere to land, so this
/// takes a subdivision rather than making callers write their own later.
pub fn plane(size: f32, segments: u32) -> MeshAsset {
    let segments = segments.max(1);
    let half = size * 0.5;
    let step = size / segments as f32;
    let mut vertices = Vec::new();
    for z in 0..=segments {
        for x in 0..=segments {
            vertices.push(Vertex {
                position: [-half + x as f32 * step, 0.0, -half + z as f32 * step],
                normal: [0.0, 1.0, 0.0],
                uv: [x as f32 / segments as f32, z as f32 / segments as f32],
            });
        }
    }
    let stride = segments + 1;
    let mut indices = Vec::new();
    for z in 0..segments {
        for x in 0..segments {
            let a = z * stride + x;
            let (b, c, d) = (a + 1, a + stride, a + stride + 1);
            // Counter-clockwise seen from above, which for a floor is what
            // puts the normal in the sky.
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    finish("plane", vertices, indices)
}

/// A box centred on the origin, with flat faces.
///
/// Every corner appears three times, once per face it belongs to, because a
/// shared vertex would have to average three perpendicular normals and turn
/// the corners round.
pub fn cube(size: f32) -> MeshAsset {
    let h = size * 0.5;
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ];
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for (normal, right, up) in faces {
        let base = vertices.len() as u32;
        for (u, v) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            vertices.push(Vertex {
                position: [
                    (normal[0] + right[0] * u + up[0] * v) * h,
                    (normal[1] + right[1] * u + up[1] * v) * h,
                    (normal[2] + right[2] * u + up[2] * v) * h,
                ],
                normal,
                uv: [(u + 1.0) * 0.5, (1.0 - v) * 0.5],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    finish("cube", vertices, indices)
}

/// A cone standing on the ground, tip up — a stand-in tree, and the shape
/// whose shading is easiest to read wrong.
pub fn cone(radius: f32, height: f32, segments: u32) -> MeshAsset {
    let segments = segments.max(3);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let tau = std::f32::consts::TAU;

    for i in 0..segments {
        // Each side gets its own three vertices: a cone's normal changes
        // along the rim, and sharing them would smooth it into a blob.
        let (a0, a1) = (
            tau * i as f32 / segments as f32,
            tau * (i + 1) as f32 / segments as f32,
        );
        let p0 = [radius * a0.cos(), 0.0, radius * a0.sin()];
        let p1 = [radius * a1.cos(), 0.0, radius * a1.sin()];
        let tip = [0.0, height, 0.0];
        let mid = (a0 + a1) * 0.5;
        // Slope: the normal leans out by the radius and up by the height.
        let slope = (radius * radius + height * height).sqrt();
        let normal = [
            height / slope * mid.cos(),
            radius / slope,
            height / slope * mid.sin(),
        ];
        let base = vertices.len() as u32;
        for position in [p0, p1, tip] {
            vertices.push(Vertex {
                position,
                normal,
                uv: [0.5, 0.5],
            });
        }
        indices.extend_from_slice(&[base, base + 2, base + 1]);
    }

    // The underside, so the cone is closed when seen from below.
    let centre = vertices.len() as u32;
    vertices.push(Vertex {
        position: [0.0, 0.0, 0.0],
        normal: [0.0, -1.0, 0.0],
        uv: [0.5, 0.5],
    });
    for i in 0..segments {
        let a = tau * i as f32 / segments as f32;
        vertices.push(Vertex {
            position: [radius * a.cos(), 0.0, radius * a.sin()],
            normal: [0.0, -1.0, 0.0],
            uv: [0.5, 0.5],
        });
    }
    for i in 0..segments {
        let a = centre + 1 + i;
        let b = centre + 1 + (i + 1) % segments;
        indices.extend_from_slice(&[centre, a, b]);
    }
    finish("cone", vertices, indices)
}

/// A UV sphere. The reference shape for smooth shading: anything wrong with
/// normals shows on a sphere before it shows anywhere else.
pub fn sphere(radius: f32, segments: u32, rings: u32) -> MeshAsset {
    let segments = segments.max(3);
    let rings = rings.max(2);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for ring in 0..=rings {
        let phi = std::f32::consts::PI * ring as f32 / rings as f32;
        for segment in 0..=segments {
            let theta = std::f32::consts::TAU * segment as f32 / segments as f32;
            let normal = [phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin()];
            vertices.push(Vertex {
                position: [normal[0] * radius, normal[1] * radius, normal[2] * radius],
                normal,
                uv: [segment as f32 / segments as f32, ring as f32 / rings as f32],
            });
        }
    }
    let stride = segments + 1;
    for ring in 0..rings {
        for segment in 0..segments {
            let a = ring * stride + segment;
            let (b, c, d) = (a + 1, a + stride, a + stride + 1);
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    finish("sphere", vertices, indices)
}

/// Look a builtin up by the name a scene file uses.
///
/// Scenes say `builtin:plane` rather than a path, so an example scene needs
/// no library and no import step to open.
pub fn by_name(name: &str) -> Option<MeshAsset> {
    match name.strip_prefix("builtin:")? {
        "plane" => Some(plane(1.0, 1)),
        "cube" => Some(cube(1.0)),
        "cone" => Some(cone(0.5, 1.0, 16)),
        "sphere" => Some(sphere(0.5, 24, 16)),
        _ => None,
    }
}

/// Every builtin name, for an editor's list and for tests that want to check
/// all of them without repeating the list.
pub const NAMES: [&str; 4] = [
    "builtin:plane",
    "builtin:cube",
    "builtin:cone",
    "builtin:sphere",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// A face wound the wrong way does not disappear — it renders inside out.
    /// So the check that matters is whether each face's winding agrees with
    /// the normal its vertices carry.
    fn winding_agrees_with_normals(mesh: &MeshAsset) -> bool {
        mesh.indices.chunks_exact(3).all(|tri| {
            let [a, b, c] = [
                mesh.vertices[tri[0] as usize],
                mesh.vertices[tri[1] as usize],
                mesh.vertices[tri[2] as usize],
            ];
            let u = sub(b.position, a.position);
            let v = sub(c.position, a.position);
            let face = cross(u, v);
            // Degenerate triangles have no opinion.
            if dot(face, face) < 1e-12 {
                return true;
            }
            dot(face, a.normal) > 0.0
        })
    }

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    fn cross(u: [f32; 3], v: [f32; 3]) -> [f32; 3] {
        [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ]
    }
    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn every_builtin_faces_outward() {
        for name in NAMES {
            let mesh = by_name(name).expect(name);
            assert!(
                winding_agrees_with_normals(&mesh),
                "{name} has faces wound against their own normals; it would \
                 render inside out rather than vanish"
            );
        }
    }

    #[test]
    fn every_builtin_has_geometry_and_a_box_around_it() {
        for name in NAMES {
            let mesh = by_name(name).expect(name);
            assert!(!mesh.vertices.is_empty(), "{name} has no vertices");
            assert_eq!(mesh.indices.len() % 3, 0, "{name} has a partial triangle");
            for index in &mesh.indices {
                assert!(
                    (*index as usize) < mesh.vertices.len(),
                    "{name} indexes past its own vertices"
                );
            }
            assert!(mesh.bounds.max[1] >= mesh.bounds.min[1], "{name} bounds");
        }
    }

    #[test]
    fn a_plane_faces_the_sky_and_a_cone_stands_on_the_ground() {
        let plane = plane(4.0, 2);
        assert!(plane.vertices.iter().all(|v| v.normal == [0.0, 1.0, 0.0]));
        assert_eq!(plane.bounds.min[1], 0.0);
        assert_eq!(plane.bounds.max[0], 2.0);

        let cone = cone(1.0, 3.0, 12);
        assert_eq!(cone.bounds.min[1], 0.0, "a tree grows up from the ground");
        assert_eq!(cone.bounds.max[1], 3.0);
    }

    #[test]
    fn a_spheres_normals_all_point_away_from_its_centre() {
        // The shape that catches normal mistakes first, so it is worth
        // stating what correct means for it.
        let sphere = sphere(2.0, 16, 12);
        for v in &sphere.vertices {
            let length = dot(v.normal, v.normal).sqrt();
            assert!((length - 1.0).abs() < 1e-4, "normal not unit: {length}");
            assert!(dot(v.normal, v.position) > 0.0, "normal points inward");
        }
    }

    #[test]
    fn an_unknown_builtin_is_none_rather_than_a_default_shape() {
        assert!(by_name("builtin:teapot").is_none());
        assert!(by_name("plane").is_none(), "the prefix is required");
    }
}
