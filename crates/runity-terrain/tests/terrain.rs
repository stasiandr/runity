//! The ground: sampling it, meshing it, and hitting it with a ray.
//!
//! The theme running through these is that there must be exactly one surface.
//! The renderer, the height query and the ray cast each compute it a different
//! way, and every test that compares two of them is guarding against the bug
//! where a player stands visibly inside a hill.

use runity_math::{vec3, Fbm, Noise, Vec3};
use runity_serialize::{from_bytes, to_bytes};
use runity_terrain::{Chunk, Heightmap, Terrain};

/// A small map with a distinct shape: a ridge along one diagonal.
fn ridge() -> Terrain {
    let heightmap = Heightmap::from_fn(9, 9, |x, z| {
        let distance = (x as f32 - z as f32).abs();
        (4.0 - distance).max(0.0)
    });
    Terrain::new(heightmap, Vec3::ZERO, 1.0)
}

fn rolling() -> Terrain {
    let noise = Noise::named(7, "terrain");
    let heightmap = Heightmap::from_noise(33, 33, &noise, Fbm::default(), 0.08, 6.0);
    Terrain::new(heightmap, vec3(-16.0, 0.0, -16.0), 1.0)
}

#[test]
fn samples_are_exact_at_the_grid_points() {
    let terrain = ridge();
    for z in 0..9 {
        for x in 0..9 {
            let world = vec3(x as f32, 0.0, z as f32);
            assert_eq!(
                terrain.height_at(world.x, world.z),
                terrain.heightmap().height(x, z),
                "at ({x}, {z})"
            );
        }
    }
}

#[test]
fn a_flat_map_is_flat_everywhere() {
    let terrain = Terrain::flat(8, 8, 2.0);
    assert_eq!(terrain.height_at(0.3, -5.1), 0.0);
    assert_eq!(terrain.normal_at(0.3, -5.1), Vec3::Y);
    assert_eq!(terrain.slope_at(0.3, -5.1), 0.0);
    // Centred: the footprint straddles the origin.
    let (min, max) = terrain.bounds();
    assert_eq!(min.x, -8.0);
    assert_eq!(max.x, 8.0);
}

#[test]
fn the_origin_offsets_the_whole_surface() {
    let mut heightmap = Heightmap::new(3, 3);
    heightmap.set(1, 1, 2.0);
    let terrain = Terrain::new(heightmap, vec3(10.0, 5.0, -10.0), 1.0);
    assert_eq!(
        terrain.height_at(11.0, -9.0),
        7.0,
        "5 from y, 2 from the map"
    );
    assert_eq!(terrain.height_at(10.0, -10.0), 5.0);
    assert!(terrain.contains(10.5, -9.5));
    assert!(!terrain.contains(20.0, -9.5));
}

/// The one that matters: every point of every triangle the renderer emits has
/// to be a point the height query agrees with.
#[test]
fn the_mesh_and_the_height_query_are_the_same_surface() {
    let terrain = rolling();
    let mesh = terrain.mesh();
    for vertex in &mesh.vertices {
        let queried = terrain.height_at(vertex.position.x, vertex.position.z);
        assert!(
            (queried - vertex.position.y).abs() < 1e-4,
            "vertex at {:?} but the query says {queried}",
            vertex.position
        );
    }

    // And between the vertices, where the interpolation rule has to match the
    // triangulation rather than the bilinear patch.
    let mut steps = 0;
    for triangle in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [
            mesh.vertices[triangle[0] as usize].position,
            mesh.vertices[triangle[1] as usize].position,
            mesh.vertices[triangle[2] as usize].position,
        ];
        // The centroid is unambiguously inside one triangle.
        let centre = (a + b + c) / 3.0;
        let queried = terrain.height_at(centre.x, centre.z);
        assert!(
            (queried - centre.y).abs() < 1e-3,
            "inside a triangle the query says {queried}, the mesh says {}",
            centre.y
        );
        steps += 1;
    }
    assert!(steps > 100, "only checked {steps} triangles");
}

#[test]
fn triangles_face_upwards() {
    let terrain = rolling();
    let mesh = terrain.mesh();
    for triangle in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [
            mesh.vertices[triangle[0] as usize].position,
            mesh.vertices[triangle[1] as usize].position,
            mesh.vertices[triangle[2] as usize].position,
        ];
        let face = (b - a).cross(c - a);
        assert!(
            face.y > 0.0,
            "a downward-facing triangle would be invisible: {face:?}"
        );
    }
}

#[test]
fn normals_point_away_from_the_slope() {
    let heightmap = Heightmap::from_fn(5, 5, |x, _| x as f32);
    let terrain = Terrain::new(heightmap, Vec3::ZERO, 1.0);
    // Ground rising towards +X: the normal leans towards -X.
    let normal = terrain.normal_at(2.0, 2.0);
    assert!(normal.x < -0.5, "{normal:?}");
    assert!(normal.y > 0.0);
    assert!((normal.length() - 1.0).abs() < 1e-5);
    // A 45-degree slope.
    assert!((terrain.slope_at(2.0, 2.0) - std::f32::consts::FRAC_PI_4).abs() < 1e-4);
}

/// Chunk borders are where seams come from, so the normals on either side must
/// be identical rather than merely close.
#[test]
fn chunk_borders_share_their_normals() {
    let terrain = rolling();
    let chunks = terrain.chunks(8);
    assert!(chunks.len() > 4, "the map should have split up");

    let meshed: Vec<_> = chunks.iter().map(|c| terrain.chunk_mesh(*c)).collect();
    let mut shared = 0;
    for (index, left) in meshed.iter().enumerate() {
        for right in &meshed[index + 1..] {
            for a in &left.mesh.vertices {
                for b in &right.mesh.vertices {
                    if (a.position - b.position).length() < 1e-5 {
                        assert_eq!(
                            a.normal, b.normal,
                            "two chunks disagree at {:?} — that is a visible seam",
                            a.position
                        );
                        shared += 1;
                    }
                }
            }
        }
    }
    assert!(shared > 0, "the chunks did not actually touch");
}

#[test]
fn chunks_cover_the_map_exactly_once() {
    let terrain = rolling();
    let cells = terrain.heightmap().cells_x() * terrain.heightmap().cells_z();
    let covered: usize = terrain
        .chunks(7)
        .iter()
        .map(|chunk| chunk.cells_x * chunk.cells_z)
        .sum();
    assert_eq!(covered, cells, "a 32-cell map in chunks of 7");

    let triangles: usize = terrain
        .chunks(7)
        .iter()
        .map(|chunk| terrain.chunk_mesh(*chunk).mesh.triangle_count())
        .sum();
    assert_eq!(triangles, cells * 2);
}

#[test]
fn a_chunk_that_runs_off_the_map_is_trimmed() {
    let terrain = Terrain::flat(10, 10, 1.0);
    let chunk = terrain.chunk_mesh(Chunk {
        x: 8,
        z: 8,
        cells_x: 32,
        cells_z: 32,
    });
    assert_eq!(chunk.chunk.cells_x, 2);
    assert_eq!(chunk.chunk.cells_z, 2);
    assert_eq!(chunk.mesh.triangle_count(), 8);
}

// ------------------------------------------------------------------- rays

#[test]
fn a_ray_from_above_lands_on_the_surface() {
    let terrain = rolling();
    for (x, z) in [(0.0, 0.0), (3.5, -2.25), (-9.1, 7.7)] {
        let expected = terrain.height_at(x, z);
        let hit = terrain
            .cast_ray(vec3(x, 40.0, z), -Vec3::Y, 100.0)
            .unwrap_or_else(|| panic!("straight down at ({x}, {z}) must hit"));
        assert!(
            (hit.point.y - expected).abs() < 1e-3,
            "ray says {}, query says {expected}",
            hit.point.y
        );
        assert!((hit.distance - (40.0 - expected)).abs() < 1e-2);
        assert!(hit.normal.y > 0.0);
    }
}

/// A ray skimming along at a constant height must stop where the ground first
/// rises to meet it — on the near face — and not pass through the hill to come
/// out on the far side, which is what happens if the walk tests a cell's
/// triangles without checking the hit is inside that cell.
#[test]
fn a_shallow_ray_stops_at_the_near_face() {
    let terrain = ridge();
    // The ridge runs along x == z and peaks at 4.0. At z == 0.5 the surface
    // climbs from 0 at the right-hand edge to about 3.5 over the peak, so a
    // ray held at y == 1.0 meets it partway up the near face.
    let from = vec3(8.5, 1.0, 0.5);
    let hit = terrain
        .cast_ray(from, vec3(-1.0, 0.0, 0.0), 20.0)
        .expect("a ray across the map must meet the ridge");

    assert!(
        hit.point.x > 0.5,
        "it came out past the peak at x={}",
        hit.point.x
    );
    assert!(
        (hit.point.y - 1.0).abs() < 1e-3,
        "the ray is level, so the hit is where the ground reaches it: {:?}",
        hit.point
    );
    // And the ground at that point is exactly the ray's height, which is the
    // real claim: the ray and the height query found the same surface.
    assert!((terrain.height_at(hit.point.x, hit.point.z) - 1.0).abs() < 1e-3);
    assert!(
        hit.normal.x > 0.0,
        "the near face leans towards the ray: {:?}",
        hit.normal
    );
}

#[test]
fn a_ray_that_misses_returns_nothing() {
    let terrain = ridge();
    // Above everything, pointing away.
    assert!(terrain
        .cast_ray(vec3(4.0, 50.0, 4.0), Vec3::Y, 100.0)
        .is_none());
    // Starting beyond the edge and heading further out.
    assert!(terrain
        .cast_ray(vec3(-20.0, 5.0, -20.0), vec3(-1.0, 0.0, 0.0), 100.0)
        .is_none());
    // Pointing at the map but stopping short.
    assert!(terrain
        .cast_ray(vec3(4.0, 50.0, 4.0), -Vec3::Y, 10.0)
        .is_none());
}

#[test]
fn a_ray_starting_outside_still_reaches_the_map() {
    let terrain = ridge();
    let hit = terrain.cast_ray(vec3(-50.0, 2.0, 4.0), vec3(1.0, 0.0, 0.0), 200.0);
    let hit = hit.expect("entering from far away must work");
    assert!(hit.point.x >= 0.0 && hit.point.x <= 8.0, "{:?}", hit.point);
}

#[test]
fn a_degenerate_ray_is_not_a_panic() {
    let terrain = ridge();
    assert!(terrain
        .cast_ray(vec3(4.0, 5.0, 4.0), Vec3::ZERO, 10.0)
        .is_none());
    assert!(terrain
        .cast_ray(vec3(4.0, 5.0, 4.0), -Vec3::Y, 0.0)
        .is_none());
    let empty = Terrain::new(Heightmap::new(0, 0), Vec3::ZERO, 1.0);
    assert!(empty
        .cast_ray(vec3(0.0, 5.0, 0.0), -Vec3::Y, 10.0)
        .is_none());
    assert_eq!(empty.height_at(3.0, 3.0), 0.0);
    assert_eq!(empty.mesh().triangle_count(), 0);
}

// -------------------------------------------------------------- generation

/// Two machines have to grow the same island from the same seed without
/// sending it to each other.
#[test]
fn generation_depends_only_on_the_seed() {
    let build = || {
        let noise = Noise::named(2024, "island");
        Heightmap::from_noise(17, 17, &noise, Fbm::default(), 0.1, 5.0)
    };
    assert_eq!(build().heights(), build().heights());

    let other = Noise::named(2025, "island");
    let different = Heightmap::from_noise(17, 17, &other, Fbm::default(), 0.1, 5.0);
    assert_ne!(build().heights(), different.heights());
}

#[test]
fn smoothing_takes_the_edge_off_without_moving_the_hills() {
    let mut heightmap = Heightmap::from_fn(17, 17, |x, z| {
        // A smooth hill plus one-cell noise on top of it.
        let hill = 5.0 - ((x as f32 - 8.0).powi(2) + (z as f32 - 8.0).powi(2)).sqrt() * 0.4;
        hill + if (x + z) % 2 == 0 { 0.6 } else { -0.6 }
    });
    let before = roughness(&heightmap);
    let peak_before = heightmap.range().1;
    heightmap.smooth(2);
    let after = roughness(&heightmap);
    assert!(after < before * 0.4, "{before} -> {after}");
    assert!(
        (heightmap.range().1 - peak_before).abs() < 1.5,
        "the hill should still be a hill"
    );
}

/// Mean absolute difference between neighbouring samples.
fn roughness(heightmap: &Heightmap) -> f32 {
    let mut total = 0.0;
    let mut count = 0;
    for z in 0..heightmap.depth() {
        for x in 1..heightmap.width() {
            total += (heightmap.height(x, z) - heightmap.height(x - 1, z)).abs();
            count += 1;
        }
    }
    total / count as f32
}

#[test]
fn clamping_makes_a_sea_floor() {
    let mut heightmap = Heightmap::from_fn(5, 5, |x, _| x as f32 - 2.0);
    heightmap.clamp(0.0, 1.0);
    assert_eq!(heightmap.range(), (0.0, 1.0));
    heightmap.offset(3.0);
    assert_eq!(heightmap.range(), (3.0, 4.0));
}

#[test]
fn a_terrain_survives_a_round_trip() {
    let terrain = rolling();
    let bytes = to_bytes(&terrain);
    let back: Terrain = from_bytes(&bytes).expect("round trip");
    assert_eq!(back, terrain);
    assert_eq!(back.height_at(1.5, -2.5), terrain.height_at(1.5, -2.5));
}

#[test]
fn a_corrupt_terrain_is_an_error_and_not_a_panic() {
    let bytes = to_bytes(&rolling());
    for cut in 0..bytes.len().min(400) {
        let _ = from_bytes::<Terrain>(&bytes[..cut]);
    }
    // A length no message could back must not be trusted with an allocation.
    assert!(from_bytes::<Terrain>(&[0xff, 0xff, 0xff, 0x7f, 0xff, 0xff, 0xff, 0x7f]).is_err());
}

#[test]
fn out_of_range_queries_clamp_to_the_edge() {
    let terrain = ridge();
    let edge = terrain.height_at(0.0, 0.0);
    assert_eq!(terrain.height_at(-100.0, -100.0), edge);
    assert!(!terrain.contains(-100.0, -100.0));
    // Sampling past the far edge, too.
    assert_eq!(
        terrain.height_at(1000.0, 1000.0),
        terrain.height_at(8.0, 8.0)
    );
}
