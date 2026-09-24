//! A step comes out the same on any number of cores (DNA, postulate 6):
//! a scene of what simulates on every core — cloth falling on water, water by
//! both methods, snow and sand, smoke — stepped here and in a copy of this
//! test run with one worker (`RUNITY_JOBS=1`), and the frames it draws
//! compared bit for bit.

use std::hash::{Hash, Hasher};

const CHILD: &str = "RUNITY_CORES_CHILD";
const SCENES: [&str; 1] = ["small"];

/// Small, so a debug build steps it in seconds, and each part past what
/// `jobs` splits (hundreds of particles, a grid of several slices).
const SMALL: &str = r#"(
    view: (position: (0.0, 2.0, 4.0), target: (0.0, 0.5, 0.0)),
    entities: [
        (id: "0000000000000001", name: "floor", model: "builtin:plane", transform: (scale: (10.0, 1.0, 10.0)), body: Static, collider: Box(half: (0.5, 0.05, 0.5), center: (0.0, -0.05, 0.0))),
        (id: "0000000000000002", name: "crate", model: "builtin:cube", transform: (position: (0.0, 0.3, 0.0), scale: (0.6, 0.6, 0.6)), body: Static, collider: Box(half: (0.5, 0.5, 0.5))),
        (id: "0000000000000003", name: "sheet", transform: (position: (-1.5, 0.75, 0.0)), cloth: (size: (1.2, 1.2), cells: (32, 32), pinned: None)),
        (id: "0000000000000004", name: "pbf", transform: (position: (-1.5, 0.0, 0.0)), fluid: (size: (0.6, 0.6, 0.6), spacing: 0.06, method: Pbf)),
        (id: "0000000000000005", name: "sph", transform: (position: (1.5, 0.3, 0.0)), fluid: (size: (0.6, 0.6, 0.6), spacing: 0.06, method: Sph)),
        (id: "0000000000000006", name: "sand", transform: (position: (0.0, 0.0, -1.5)), mpm: (material: Sand, size: (0.6, 0.4, 0.6), height: 0.2, domain: (0.8, 0.8, 0.8), resolution: 10.0, stiffness: 100.0)),
        (id: "0000000000000007", name: "snow", transform: (position: (0.0, 0.0, 1.5)), mpm: (material: Snow, size: (0.3, 0.3, 0.3), height: 0.3, domain: (0.8, 0.8, 0.8), resolution: 10.0, stiffness: 20.0)),
        (id: "0000000000000008", name: "smoke", transform: (position: (2.5, 0.0, -1.5)), smoke: (size: (1.0, 1.5, 1.0), resolution: 8.0, source: 0.2, rate: 5.0, heat: 5.0)),
    ],
)"#;

/// What a scene draws after `steps` steps, hashed: every draw's place and
/// every live mesh's vertices.
fn stepped(scene: &str, steps: u32) -> Option<u64> {
    // Smoke on the CPU: its grid is what runs on every core here (on the
    // GPU it is the renderer's).
    runity::fluid::set_gpu_smoke(false);
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cores-{scene}.ron"));
    std::fs::write(&path, SMALL).ok()?;
    let mut shot = runity::shot::Shot::open(&path, 64, 64, None).ok()?;
    assert!(shot.problems.is_empty(), "{:?}", shot.problems);
    for _ in 0..steps {
        shot.step(1.0 / 60.0);
    }
    shot.build();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for draw in &shot.frame.draws {
        for v in draw.transform.to_cols_array() {
            v.to_bits().hash(&mut hash);
        }
    }
    for live in &shot.frame.live_meshes {
        for v in live.vertices.iter() {
            for c in v.position {
                c.to_bits().hash(&mut hash);
            }
        }
    }
    Some(hash.finish())
}

#[test]
fn a_step_is_the_same_on_one_core_as_on_all_of_them() {
    if std::env::var_os(CHILD).is_some() {
        // The copy: say what one worker made.
        for scene in SCENES {
            println!("{CHILD} {scene} {:?}", stepped(scene, 8));
        }
        return;
    }
    if runity_core_workers() < 2 {
        eprintln!("skipping: one core");
        return;
    }
    let ours: Vec<String> = SCENES.iter().map(|s| format!("{CHILD} {s} {:?}", stepped(s, 8))).collect();
    eprintln!("{}", ours.join("\n"));
    if ours.iter().all(|l| l.ends_with("None")) {
        eprintln!("skipping: no adapter");
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("this test"))
        .args(["--exact", "a_step_is_the_same_on_one_core_as_on_all_of_them", "--nocapture"])
        .env(CHILD, "1")
        .env("RUNITY_JOBS", "1")
        .output()
        .expect("the copy runs");
    let text = String::from_utf8_lossy(&out.stdout);
    for line in &ours {
        assert!(
            text.lines().any(|l| l == line),
            "on all cores {line}; with one:\n{}",
            text.lines().filter(|l| l.starts_with(CHILD)).collect::<Vec<_>>().join("\n")
        );
    }
}

fn runity_core_workers() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}
