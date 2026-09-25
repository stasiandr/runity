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

/// A mesh of one submesh from its vertices and triangles, with its bounds
/// and an ID from `name`: how every built-in shape, and the terrain, ends.
pub fn finish(name: &str, vertices: Vec<Vertex>, indices: Vec<u32>) -> MeshAsset {
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
        skin: None,
        colors: Vec::new(),
        look: None,
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

// --- blockout ---------------------------------------------------------------
//
// Greybox shapes: what a level is sketched from before it has art. Centred on
// the origin like the cube, one unit across, so the centred colliders —
// `Box`, `Cylinder`, `Ramp`, `Stairs` — fit them with the same numbers, and
// scaling the entity scales both.

/// A flat face from its corners, wound so that its normal points away from
/// `inside` — a point within the shape. Correct by construction rather than
/// by the care of whoever lists the corners.
fn face(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    corners: &[[f32; 3]],
    inside: [f32; 3],
) {
    let v = |i: usize| glam::Vec3::from_array(corners[i]);
    let mut normal = (v(1) - v(0)).cross(v(2) - v(0)).normalize_or_zero();
    let centre = corners
        .iter()
        .map(|c| glam::Vec3::from_array(*c))
        .sum::<glam::Vec3>()
        / corners.len() as f32;
    let flip = normal.dot(centre - glam::Vec3::from_array(inside)) < 0.0;
    if flip {
        normal = -normal;
    }
    let base = vertices.len() as u32;
    for (i, corner) in corners.iter().enumerate() {
        vertices.push(Vertex {
            position: *corner,
            normal: normal.to_array(),
            uv: [(i == 1 || i == 2) as u32 as f32, (i >= 2) as u32 as f32],
        });
    }
    for i in 1..corners.len() as u32 - 1 {
        if flip {
            indices.extend_from_slice(&[base, base + i + 1, base + i]);
        } else {
            indices.extend_from_slice(&[base, base + i, base + i + 1]);
        }
    }
}

/// An axis-aligned box between two corners, faces outward.
fn add_box(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, min: [f32; 3], max: [f32; 3]) {
    let inside = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    for corners in [
        [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
        [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
        [[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0]],
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        [[x0, y0, z0], [x0, y1, z0], [x0, y1, z1], [x0, y0, z1]],
        [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
    ] {
        face(vertices, indices, &corners, inside);
    }
}

/// An upright cylinder, centred, with flat caps and smooth sides: a pillar,
/// a tower, a barrel.
pub fn cylinder(radius: f32, height: f32, segments: u32) -> MeshAsset {
    let segments = segments.max(3);
    let (mut vertices, mut indices) = (Vec::new(), Vec::new());
    let h = height * 0.5;
    let tau = std::f32::consts::TAU;
    let at = |i: u32| {
        let a = tau * i as f32 / segments as f32;
        (a.cos(), a.sin())
    };
    // Sides: two vertices a column, normals straight out.
    for i in 0..=segments {
        let (c, s) = at(i);
        for y in [-h, h] {
            vertices.push(Vertex {
                position: [c * radius, y, s * radius],
                normal: [c, 0.0, s],
                uv: [i as f32 / segments as f32, if y < 0.0 { 1.0 } else { 0.0 }],
            });
        }
    }
    for i in 0..segments {
        let a = i * 2;
        indices.extend_from_slice(&[a, a + 1, a + 2, a + 2, a + 1, a + 3]);
    }
    // Caps, each a fan with its own flat normal.
    for (y, up) in [(h, 1.0f32), (-h, -1.0)] {
        let centre = vertices.len() as u32;
        vertices.push(Vertex {
            position: [0.0, y, 0.0],
            normal: [0.0, up, 0.0],
            uv: [0.5, 0.5],
        });
        for i in 0..segments {
            let (c, s) = at(i);
            vertices.push(Vertex {
                position: [c * radius, y, s * radius],
                normal: [0.0, up, 0.0],
                uv: [0.5 + c * 0.5, 0.5 + s * 0.5],
            });
        }
        for i in 0..segments {
            let a = centre + 1 + i;
            let b = centre + 1 + (i + 1) % segments;
            if up > 0.0 {
                indices.extend_from_slice(&[centre, b, a]);
            } else {
                indices.extend_from_slice(&[centre, a, b]);
            }
        }
    }
    finish("cylinder", vertices, indices)
}

/// A wedge: flat along the front (+z) at the bottom, full height at the
/// back (−z). What a slope, a ramp or a roof is sketched with.
pub fn ramp(size: f32) -> MeshAsset {
    let h = size * 0.5;
    let (mut vertices, mut indices) = (Vec::new(), Vec::new());
    let inside = [0.0, -h / 3.0, -h / 3.0];
    let (b0, b1, b2, b3) = ([-h, -h, -h], [h, -h, -h], [h, -h, h], [-h, -h, h]);
    let (t0, t1) = ([-h, h, -h], [h, h, -h]);
    for corners in [
        &[b0, b1, b2, b3][..],
        &[b0, b1, t1, t0][..],
        &[b3, b2, t1, t0][..],
        &[b1, b2, t1][..],
        &[b0, b3, t0][..],
    ] {
        face(&mut vertices, &mut indices, corners, inside);
    }
    finish("ramp", vertices, indices)
}

/// A flight of `steps` steps, rising toward the back (−z), in a unit box.
pub fn stairs(size: f32, steps: u32) -> MeshAsset {
    let steps = steps.max(1);
    let h = size * 0.5;
    let (mut vertices, mut indices) = (Vec::new(), Vec::new());
    let depth = size / steps as f32;
    for i in 0..steps {
        // A column from the ground up to its step's height.
        let front = h - i as f32 * depth;
        let top = -h + (i + 1) as f32 * depth;
        add_box(
            &mut vertices,
            &mut indices,
            [-h, -h, front - depth],
            [h, top, front],
        );
    }
    finish("stairs", vertices, indices)
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
        "cylinder" => Some(cylinder(0.5, 1.0, 24)),
        "ramp" => Some(ramp(1.0)),
        "stairs" => Some(stairs(1.0, STAIRS)),
        "link" => Some(link(24, 8)),
        "capsule" => Some(capsule(0.5, 0.5, 20, 12)),
        // Unity's own primitives, where they are not ours: its Plane is
        // ten metres a side, its Quad a metre square standing up, facing
        // its back (Unity's -z, mirrored to +z as a Unity scene is).
        "unity_plane" => Some(plane(10.0, 10)),
        "unity_quad" => Some(unity_quad()),
        _ => None,
    }
}

/// Every builtin name, for an editor's list and for tests that want to check
/// all of them without repeating the list.
pub const NAMES: [&str; 11] = [
    "builtin:plane",
    "builtin:cube",
    "builtin:cone",
    "builtin:sphere",
    "builtin:cylinder",
    "builtin:ramp",
    "builtin:stairs",
    "builtin:link",
    "builtin:capsule",
    "builtin:unity_plane",
    "builtin:unity_quad",
];

/// Unity's Quad: a metre square in its x and y, facing +z here (Unity's
/// -z, brought over the scene's mirror).
fn unity_quad() -> MeshAsset {
    let mut mesh = plane(1.0, 1);
    for v in &mut mesh.vertices {
        let [x, _, z] = v.position;
        v.position = [x, -z, 0.0];
        v.normal = [0.0, 0.0, 1.0];
    }
    mesh.bounds = crate::asset::Bounds::of(&mesh.vertices);
    mesh
}

/// A capsule standing up: a cylinder `2 × half_height` tall capped with
/// half balls of `radius` — what fits `Capsule(half_height, radius)`.
pub fn capsule(half_height: f32, radius: f32, segments: u32, rings: u32) -> MeshAsset {
    let segments = segments.max(3);
    let rings = rings.max(2) & !1; // even: the middle ring splits the caps
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    // A ball's rings, the top half raised and the bottom lowered, with the
    // middle ring twice: once for each cap, the cylinder between them.
    let mut ring_list: Vec<(f32, f32)> = Vec::new();
    for ring in 0..=rings {
        let phi = std::f32::consts::PI * ring as f32 / rings as f32;
        let lift = if ring <= rings / 2 { half_height } else { -half_height };
        ring_list.push((phi, lift));
        if ring == rings / 2 {
            ring_list.push((phi, -half_height));
        }
    }
    for (row, (phi, lift)) in ring_list.iter().enumerate() {
        for segment in 0..=segments {
            let theta = std::f32::consts::TAU * segment as f32 / segments as f32;
            let normal = [phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin()];
            vertices.push(Vertex {
                position: [normal[0] * radius, normal[1] * radius + lift, normal[2] * radius],
                normal,
                uv: [segment as f32 / segments as f32, row as f32 / (ring_list.len() - 1) as f32],
            });
        }
    }
    let stride = segments + 1;
    for row in 0..ring_list.len() as u32 - 1 {
        for segment in 0..segments {
            let a = row * stride + segment;
            let (b, c, d) = (a + 1, a + stride, a + stride + 1);
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    finish("capsule", vertices, indices)
}

/// A link of a chain: a ring of wire drawn out into a stadium, lying in
/// its x–z plane, a metre long along z and 0.6 across x. A chain draws one
/// per link, every other one turned a quarter about z, so they hang
/// through each other.
pub fn link(around: u32, sides: u32) -> MeshAsset {
    use glam::Vec3;
    let (wire, bend) = (0.08f32, 0.22f32);
    let straight = 0.5 - bend - wire;
    let around = around.max(8) as usize;
    let sides = sides.max(3) as usize;
    // The wire's middle, once round: up the right side, over the far end,
    // down the left, under the near end — by distance, so the rings are
    // even.
    let arc = std::f32::consts::PI * bend;
    let length = 4.0 * straight + 2.0 * arc;
    let centre = |d: f32| -> (Vec3, Vec3) {
        let d = d.rem_euclid(length);
        let side = 2.0 * straight;
        if d < side {
            (Vec3::new(bend, 0.0, -straight + d), Vec3::X)
        } else if d < side + arc {
            let a = (d - side) / bend;
            let out = Vec3::new(a.cos(), 0.0, a.sin());
            (Vec3::new(0.0, 0.0, straight) + out * bend, out)
        } else if d < 2.0 * side + arc {
            (Vec3::new(-bend, 0.0, straight - (d - side - arc)), -Vec3::X)
        } else {
            let a = std::f32::consts::PI + (d - 2.0 * side - arc) / bend;
            let out = Vec3::new(a.cos(), 0.0, a.sin());
            (Vec3::new(0.0, 0.0, -straight) + out * bend, out)
        }
    };
    let mut vertices = Vec::with_capacity(around * sides);
    for i in 0..around {
        let (c, out) = centre(i as f32 / around as f32 * length);
        for j in 0..sides {
            let a = j as f32 / sides as f32 * std::f32::consts::TAU;
            let normal = out * a.cos() + Vec3::Y * a.sin();
            vertices.push(Vertex {
                position: (c + normal * wire).to_array(),
                normal: normal.to_array(),
                uv: [i as f32 / around as f32, j as f32 / sides as f32],
            });
        }
    }
    let mut indices = Vec::with_capacity(around * sides * 6);
    let at = |i: usize, j: usize| ((i % around) * sides + j % sides) as u32;
    let facing = |a: u32, b: u32, c: u32| {
        let p = |i: u32| Vec3::from_array(vertices[i as usize].position);
        let n = Vec3::from_array(vertices[a as usize].normal);
        (p(b) - p(a)).cross(p(c) - p(a)).dot(n) > 0.0
    };
    for i in 0..around {
        for j in 0..sides {
            let (a, b, c, d) = (at(i, j), at(i, j + 1), at(i + 1, j), at(i + 1, j + 1));
            for [x, y, z] in [[a, c, b], [b, c, d]] {
                if facing(x, y, z) {
                    indices.extend_from_slice(&[x, y, z]);
                } else {
                    indices.extend_from_slice(&[x, z, y]);
                }
            }
        }
    }
    finish("link", vertices, indices)
}

/// How many steps `builtin:stairs` has: what a `Stairs` collider for it
/// should say.
pub const STAIRS: u32 = 4;

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
