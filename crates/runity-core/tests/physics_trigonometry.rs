//! The trigonometry `runity_core::physics` refuses to contain.
//!
//! Two things live here, and both are about the same rule. `physics::step` and
//! everything it calls is arithmetic and `sqrt` only, because those round
//! identically everywhere and `sin` carries no such promise. This file holds
//! the test that checks the arithmetic against the trigonometry it stands in
//! for — which naturally has to call `sin` — and the test that checks nothing
//! under `physics/` does.

use runity_core::physics::{FallingTree, Tuning};
use runity_math::Vec2;

#[test]
fn the_lean_tracks_the_exact_arc_closely_enough_to_look_right() {
    // A trunk advances its lean along the circle's tangent and renormalizes,
    // which is a first-order step, not an exact rotation. Over the thirty
    // ticks a fall lasts, the error against a real `sin` has to stay far below
    // anything an eye could catch on a six-metre tree.
    let dt = 1.0 / 20.0;
    let torque = Tuning::default().tree_torque;
    let mut tree = FallingTree::new(6.0, 0.35);
    tree.topple(Vec2::new(1.0, 0.0));

    let mut omega: f32 = 0.0;
    let mut angle: f32 = 0.0;
    for _ in 0..25 {
        tree.advance(dt, torque);
        omega += torque * dt;
        angle += omega * dt;
        assert!(
            (tree.lean.x - angle.sin()).abs() < 0.01,
            "lean {} vs sin {} at angle {angle}",
            tree.lean.x,
            angle.sin()
        );
        assert!(
            (tree.lean.y - angle.cos()).abs() < 0.01,
            "lean {} vs cos {}",
            tree.lean.y,
            angle.cos()
        );
    }
}

#[test]
fn nothing_under_physics_calls_a_transcendental_function() {
    // The rule is easy to state and easy to break by reflex — `to_radians` on
    // a tuning constant looks harmless. Reading the source is the only check
    // that actually holds, so the source is what this reads.
    let physics = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("physics");
    let mut files: Vec<_> = std::fs::read_dir(&physics)
        .expect("the physics module is where it has always been")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "rs"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 7,
        "expected the whole module, found {files:?}"
    );

    const FORBIDDEN: [&str; 9] = [
        ".sin(",
        ".cos(",
        ".tan(",
        ".atan2(",
        ".asin(",
        ".acos(",
        ".powf(",
        ".to_radians(",
        ".exp(",
    ];
    for path in &files {
        let source = std::fs::read_to_string(path).expect("readable source file");
        for (number, line) in source.lines().enumerate() {
            // A doc comment may name the thing it is refusing to call.
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for call in FORBIDDEN {
                assert!(
                    !code.contains(call),
                    "{}:{} calls {call}: {}",
                    path.display(),
                    number + 1,
                    line.trim()
                );
            }
        }
    }
}
