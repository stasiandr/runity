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
use scrap::matching::{Among, Ask, Database, Matcher, Setup};

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

/// Static boxes, `(centre, half size)`, as a physics world.
pub fn course(boxes: &[(Vec3, Vec3)]) -> scrap::PhysicsWorld {
    let mut world = scrap::hecs::World::new();
    for &(centre, half) in boxes {
        world.spawn((
            scrap::Transform { position: centre, ..Default::default() },
            scrap::world::WorldTransform(scrap::glam::Mat4::from_translation(centre)),
            scrap::world::Physics(scrap::Body::Static),
            scrap::Shape(scrap::scene::Collider::Box { half, center: Vec3::ZERO }),
        ));
    }
    let mut physics = scrap::PhysicsWorld::new(1.0 / 60.0);
    physics.sync_from_world(&mut world);
    physics.refresh_queries();
    physics
}

/// A floor, a wall across the way at z = 4, and to the side a flight of
/// six 17 cm steps up to a landing.
fn yard() -> scrap::PhysicsWorld {
    let mut boxes = vec![
        (Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 0.5, 40.0)),
        (Vec3::new(0.0, 1.0, 4.25), Vec3::new(2.0, 1.0, 0.25)),
    ];
    for i in 0..6 {
        let top = 0.17 * (i + 1) as f32;
        boxes.push((Vec3::new(6.0, top / 2.0, 2.0 + 0.3 * i as f32 + 0.15), Vec3::new(1.0, top / 2.0, 0.15)));
    }
    boxes.push((Vec3::new(6.0, 0.51, 4.3 + 2.0), Vec3::new(1.0, 0.51, 2.5)));
    course(&boxes)
}

#[test]
fn a_character_stops_at_a_wall_and_climbs_stairs_on_its_feet() {
    let Some(dir) = lafan1() else {
        eprintln!("skipping: no LAFAN1 (set SCRAP_LAFAN1 or put it in ~/.cache/scrap/lafan1)");
        return;
    };
    let (skeleton, clips) = with_obstacles(&dir);
    let db = Database::build(&skeleton, &clips, Setup::default()).unwrap();
    let physics = yard();
    let among = Among(&physics);
    let dt = 1.0 / 60.0;

    // At the wall: it walks up, stops short, and stands still.
    let mut matcher = Matcher::new(&db, Vec3::ZERO, Vec3::Z);
    for _ in 0..(6.0 / dt) as usize {
        matcher.advance_in(&db, &Ask { velocity: Vec3::Z * 1.4, facing: None }, dt, &among);
    }
    let stopped = matcher.root.0;
    for _ in 0..30 {
        matcher.advance_in(&db, &Ask { velocity: Vec3::Z * 1.4, facing: None }, dt, &among);
    }
    let pace = matcher.root.0.distance(stopped) / 0.5;
    eprintln!("at the wall: {stopped:?}, then {pace:.2} m/s");
    assert!(stopped.z > 3.0 && stopped.z < 4.0 - 0.25, "stopped before the wall: {stopped:?}");
    assert!(pace < 0.3, "and stands");

    // Up the stairs: every foot that is down stands on a step, not in it.
    let mut matcher = Matcher::new(&db, Vec3::new(6.0, 0.0, 0.0), Vec3::Z);
    let feet = db.feet();
    let mut worst_in: f32 = 0.0;
    let mut worst_over: f32 = 0.0;
    for _ in 0..(7.0 / dt) as usize {
        let pose = matcher.advance_in(&db, &Ask { velocity: Vec3::Z * 1.2, facing: None }, dt, &among);
        let world = matcher.world(&db, &pose);
        let down = db.contacts(matcher.frame);
        for side in 0..2 {
            // Each joint of the foot over the ground under that joint.
            let foot = world[feet[side]].w_axis.truncate();
            let toe = world[db.toes()[side].unwrap()].w_axis.truncate();
            // How far over the ground under it — or under a point three
            // centimetres back, so a toe touching a riser's edge is not
            // counted as the step's whole height into it.
            let back = matcher.root.1 * Vec3::Z * 0.03;
            let over = |p: Vec3| {
                let ground = |q: Vec3| physics.cast_ray_with_normal(Vec3::new(q.x, p.y + 0.5, q.z), -Vec3::Y, 2.0, true).map(|h| h.0.y);
                let lower = match (ground(p), ground(p - back)) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                lower.map(|g| p.y - g)
            };
            let (Some(a), Some(t)) = (over(foot), over(toe)) else { continue };
            worst_in = worst_in.max(-a.min(t));
            if down[side] {
                worst_over = worst_over.max(a.min(t));
            }
        }
    }
    let top = matcher.root.0;
    eprintln!("up the stairs to {top:?}: a foot {worst_in:.3} m into a step at worst, a planted foot {worst_over:.3} m over it");
    assert!(top.z > 5.0 && (top.y - 1.02).abs() < 0.05, "on the landing: {top:?}");
    // A foot put down astride a step's edge — heel on one tread, toe past
    // the next riser — shows its toe that far under the tread above. It
    // is a touch at the edge, not a leg through the stairs; held to it.
    assert!(worst_in < 0.2, "no foot sinks into a step");
    assert!(worst_over < 0.12, "a planted foot stands on its step");
}

/// Walking, running, and every LAFAN1 take of getting over things.
fn with_obstacles(dir: &std::path::Path) -> (Skeleton, Vec<Clip>) {
    let mut names: Vec<String> = vec!["walk1_subject1".into(), "walk1_subject2".into(), "run1_subject2".into()];
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("obstacles"))
        .map(|n| n.trim_end_matches(".bvh").to_string())
        .collect();
    found.sort();
    names.extend(found);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    locomotion(dir, &names)
}

#[test]
fn a_character_gets_onto_a_box_over_a_ledge_and_stops_at_a_wall() {
    let Some(dir) = lafan1() else {
        eprintln!("skipping: no LAFAN1 (set SCRAP_LAFAN1 or put it in ~/.cache/scrap/lafan1)");
        return;
    };
    let (skeleton, clips) = with_obstacles(&dir);
    let db = Database::build(&skeleton, &clips, Setup::default()).unwrap();
    // A half-metre box, a metre-high ledge four metres deep, and a wall
    // too high to get over.
    let physics = course(&[
        (Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 0.5, 40.0)),
        (Vec3::new(0.0, 0.25, 4.0), Vec3::new(1.0, 0.25, 0.5)),
        (Vec3::new(0.0, 0.5, 9.0), Vec3::new(2.0, 0.5, 2.0)),
        (Vec3::new(0.0, 1.1, 14.25), Vec3::new(4.0, 1.1, 0.25)),
    ]);
    let among = Among(&physics);
    let mut matcher = Matcher::new(&db, Vec3::ZERO, Vec3::Z);
    let dt = 1.0 / 60.0;
    let (mut on_box, mut on_ledge, mut highest) = (0.0f32, 0.0f32, 0.0f32);
    let mut floating: f32 = 0.0;
    let mut settled = 0.0;
    for _ in 0..(24.0 / dt) as usize {
        matcher.advance_in(&db, &Ask { velocity: Vec3::Z * 1.3, facing: None }, dt, &among);
        let (root, feet) = (matcher.root.0, matcher.spring.0);
        if (3.6..4.4).contains(&root.z) && (root.y - 0.5).abs() < 0.1 {
            on_box += dt;
        }
        if (7.5..10.5).contains(&root.z) && (root.y - 1.0).abs() < 0.1 {
            on_ledge += dt;
        }
        highest = highest.max(root.y);
        // Over the ground its capsule stands on, once a climb is done
        // and the root has had half a second to settle.
        settled = if matcher.committed() { 0.0 } else { settled + dt };
        if settled > 0.5 {
            floating = floating.max(root.y - feet.y);
        }
    }
    let end = matcher.root.0;
    eprintln!(
        "{:.1} s on the box, {:.1} s on the ledge, highest {highest:.2} m, {floating:.2} m over its ground at worst, ends at {end:?}",
        on_box, on_ledge
    );
    assert!(on_box > 0.2, "got onto the box");
    assert!(on_ledge > 1.0, "got up the ledge and walked along it");
    assert!(highest < 1.4, "never higher than the ledge it climbed");
    assert!(floating < 0.3, "stands on what is there");
    assert!(end.z > 11.0 && end.z < 14.0 - 0.25 && end.y.abs() < 0.05, "down off the ledge, stopped at the wall: {end:?}");
}
