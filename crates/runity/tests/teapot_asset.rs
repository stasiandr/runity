//! The Newell teapot is the first binary asset checked into the repository
//! (see `examples/assets/SOURCE.txt`). It ships with only `v` and `f` lines —
//! no `vt`, no `vn` — which exercises the fallback path in `Mesh::from_obj`
//! that this parser otherwise only sees in hand-written test fixtures.

use runity::prelude::*;

fn load_teapot() -> Mesh {
    let source = include_str!("../examples/assets/teapot.obj");
    Mesh::from_obj(source).expect("teapot.obj should parse")
}

#[test]
fn teapot_parses_into_a_non_empty_mesh() {
    let mesh = load_teapot();
    assert!(!mesh.vertices.is_empty(), "teapot has no vertices");
    assert!(mesh.triangle_count() > 0, "teapot has no triangles");
}

#[test]
fn teapot_has_no_uvs_so_they_fall_back_to_zero() {
    let mesh = load_teapot();
    assert!(mesh.vertices.iter().all(|v| v.uv == Vec2::ZERO));
}

#[test]
fn teapot_has_no_authored_normals_so_recompute_normals_filled_them_in() {
    let mesh = load_teapot();
    // Every referenced vertex touches at least one triangle, so recomputed
    // normals should all be unit length, not the zero the parser starts with.
    assert!(
        mesh.vertices
            .iter()
            .all(|v| (v.normal.length() - 1.0).abs() < 1e-4),
        "teapot.obj has no `vn` lines; Mesh::from_obj should have derived normals via recompute_normals"
    );
}
