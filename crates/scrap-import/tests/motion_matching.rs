//! Motion matching on real mocap: LAFAN1's locomotion, driven by a stick
//! that stands, walks, turns and runs. The character goes where it is
//! asked, as fast as it is asked, its feet stay put while they stand, and
//! a jump between frames does not pop.
//!
//! LAFAN1 (Ubisoft La Forge, CC BY-NC-ND 4.0) is not in the repository and
//! never ships: it is read from `$SCRAP_LAFAN1` or `~/.cache/scrap/lafan1`,
//! and the test says it is skipping when it is not there.

use std::path::PathBuf;

use scrap::animation::{Clip, Skeleton};
use scrap::glam::Vec3;
use scrap::matching::{Ask, Database, Matcher, Setup};

pub fn lafan1() -> Option<PathBuf> {
    let dir = std::env::var_os("SCRAP_LAFAN1")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/scrap/lafan1")))?;
    dir.join("walk1_subject1.bvh").exists().then_some(dir)
}

/// The locomotion takes: walking, running, sprinting.
pub fn locomotion(dir: &std::path::Path, names: &[&str]) -> (Skeleton, Vec<Clip>) {
    let mut skeleton = None;
    let mut clips = Vec::new();
    for name in names {
        let text = std::fs::read_to_string(dir.join(format!("{name}.bvh"))).unwrap();
        let (s, clip) = scrap_import::bvh::read(&text, name, 0.01).unwrap();
        skeleton.get_or_insert(s);
        clips.push(clip);
    }
    (skeleton.unwrap(), clips)
}

/// What the stick asks at a time: stand, walk ahead, walk right, run
/// back, stand.
fn stick(t: f32) -> Vec3 {
    match t {
        t if t < 2.0 => Vec3::ZERO,
        t if t < 6.0 => Vec3::Z * 1.4,
        t if t < 10.0 => Vec3::X * 1.4,
        t if t < 14.0 => -Vec3::Z * 3.5,
        _ => Vec3::ZERO,
    }
}

#[test]
fn a_character_on_lafan1_goes_where_the_stick_points() {
    let Some(dir) = lafan1() else {
        eprintln!("skipping: no LAFAN1 (set SCRAP_LAFAN1 or put it in ~/.cache/scrap/lafan1)");
        return;
    };
    let started = std::time::Instant::now();
    let (skeleton, clips) = locomotion(&dir, &["walk1_subject1", "walk1_subject2", "run1_subject2", "sprint1_subject2"]);
    let db = Database::build(&skeleton, &clips, Setup::default()).unwrap();
    eprintln!("{} frames in {:.1?}", db.len(), started.elapsed());

    let dt = 1.0 / 60.0;
    let mut matcher = Matcher::new(&db, Vec3::ZERO, Vec3::Z);
    let feet = db.feet();
    let mut last: Option<Vec<scrap::glam::Mat4>> = None;
    let mut last_root = matcher.root.0;
    let mut speed_off = Vec::new();
    let mut skate = Vec::new();
    let mut pop: f32 = 0.0;
    let mut leash: f32 = 0.0;
    let mut searching = std::time::Duration::ZERO;
    let steps = (16.0 / dt) as usize;
    for step in 0..steps {
        let t = step as f32 * dt;
        let ask = Ask { velocity: stick(t), facing: None };
        let clock = std::time::Instant::now();
        let pose = matcher.advance(&db, &ask, dt);
        searching += clock.elapsed();
        let world = matcher.world(&db, &pose);
        let root = matcher.root.0;
        leash = leash.max((root - matcher.spring.0).length());
        // A second after each change of the stick, the speed is the asked one.
        let since_change = [2.0, 6.0, 10.0, 14.0].iter().map(|c| t - c).filter(|d| *d >= 0.0).fold(t, f32::min);
        if since_change > 1.5 {
            let speed = (root - last_root).length() / dt;
            speed_off.push((speed - stick(t).length()).abs());
        }
        if let Some(last) = &last {
            for &foot in &feet {
                let (a, b) = (last[foot].w_axis.truncate(), world[foot].w_axis.truncate());
                // A foot down: low, and slow to begin with. How far it slides.
                if b.y < 0.12 && a.y < 0.12 {
                    let slide = Vec3::new(b.x - a.x, 0.0, b.z - a.z).length() / dt;
                    if slide < 0.5 {
                        skate.push(slide);
                    }
                }
            }
            // A pop: a joint moving further in a step than the body does
            // and than a limb can.
            for (a, b) in last.iter().zip(&world) {
                let moved = (b.w_axis.truncate() - a.w_axis.truncate()) - (root - last_root);
                pop = pop.max(moved.length() / dt);
            }
        }
        last = Some(world);
        last_root = root;
    }
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    eprintln!(
        "speed off {:.2} m/s, feet slide {:.3} m/s over {} contacts, fastest joint {:.1} m/s, \
         leash {:.3} m, {} jumps, {:.2} ms a step, at {:?}",
        mean(&speed_off),
        mean(&skate),
        skate.len(),
        pop,
        leash,
        matcher.jumps,
        searching.as_secs_f64() * 1000.0 / steps as f64,
        matcher.root.0,
    );
    let end = matcher.root.0;
    assert!(end.z < -3.0 && end.x > 3.0, "walked ahead and right, then ran back: {end:?}");
    assert!(mean(&speed_off) < 0.5, "keeps the asked speed");
    assert!(mean(&skate) < 0.05, "feet stay where they stand: the mocap itself rolls them 0.07 m/s");
    assert!(leash <= 0.151, "the character stays by its spring");
    assert!(pop < 10.0, "no joint pops: a sprinting foot is 8 m/s");
}
