//! `--eval`: the character through a set of courses with no screen, and
//! how well it did — what went into walls and floors, when it floated or
//! sank, how hard its jumps between frames were, how its feet slid. The
//! numbers to improve against, and the moments to look at.

use std::sync::Arc;

use scrap::glam::{Mat4, Vec3};
use scrap::matching::Database;

use crate::Walker;

/// A course: its boxes, where the character starts, for how long, and
/// what the stick asks at a time and place.
pub struct Course {
    pub name: &'static str,
    pub boxes: Vec<(Vec3, Vec3)>,
    pub start: Vec3,
    pub seconds: f32,
    pub stick: Box<dyn Fn(f32, Vec3) -> Vec3>,
}

/// A course by name.
pub fn course(name: &str) -> Option<Course> {
    courses().into_iter().find(|c| c.name == name)
}

/// Legs of a path: where to, how fast, for no longer than so long.
fn follow(legs: Vec<([f32; 2], f32, f32)>) -> Box<dyn Fn(f32, Vec3) -> Vec3> {
    Box::new(move |t, at| {
        let mut start = 0.0;
        for &([x, z], speed, seconds) in &legs {
            if t < start + seconds {
                let to = Vec3::new(x, 0.0, z) - Vec3::new(at.x, 0.0, at.z);
                return if to.length() < 0.3 { Vec3::ZERO } else { to.normalize() * speed };
            }
            start += seconds;
        }
        Vec3::ZERO
    })
}

/// A stick that changes its mind every second or three: a direction and a
/// speed from standing to running, the same every run.
fn wander() -> Box<dyn Fn(f32, Vec3) -> Vec3> {
    let mut seed: u64 = 0x5eed;
    let mut next = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as f32 / (1u64 << 31) as f32
    };
    let mut changes = Vec::new();
    let mut t = 0.0;
    while t < 200.0 {
        let angle = next() * std::f32::consts::TAU;
        let speed = [0.0, 1.2, 1.5, 1.5, 2.5, 3.5, 4.5][(next() * 7.0) as usize % 7];
        changes.push((t, Vec3::new(angle.sin(), 0.0, angle.cos()) * speed));
        t += 1.0 + next() * 2.0;
    }
    Box::new(move |t, _| changes.iter().rev().find(|c| c.0 <= t).map_or(Vec3::ZERO, |c| c.1))
}

/// A flight of `steps` steps of `rise` up along +z from `z`, `deep` each, a
/// landing, and the same down.
fn stairs(x: f32, z: f32, steps: usize, rise: f32, deep: f32, landing: f32) -> Vec<(Vec3, Vec3)> {
    let mut boxes = Vec::new();
    let top = rise * steps as f32;
    let down = z + deep * steps as f32 + landing;
    for i in 0..steps {
        let high = rise * (i + 1) as f32;
        boxes.push((Vec3::new(x, high / 2.0, z + deep * (i as f32 + 0.5)), Vec3::new(1.0, high / 2.0, deep / 2.0)));
        boxes.push((
            Vec3::new(x, high / 2.0, down + deep * ((steps - 1 - i) as f32 + 0.5)),
            Vec3::new(1.0, high / 2.0, deep / 2.0),
        ));
    }
    boxes.push((Vec3::new(x, top / 2.0, z + deep * steps as f32 + landing / 2.0), Vec3::new(1.0, top / 2.0, landing / 2.0)));
    boxes
}

fn courses() -> Vec<Course> {
    let mut row = Vec::new();
    for (i, h) in [0.3f32, 0.5, 0.8, 1.0, 1.2].into_iter().enumerate() {
        row.push((Vec3::new(0.0, h / 2.0, 5.0 + 7.0 * i as f32), Vec3::new(1.5, h / 2.0, 0.6)));
    }
    vec![
        Course { name: "flat", boxes: Vec::new(), start: Vec3::ZERO, seconds: 90.0, stick: wander() },
        Course {
            name: "walls",
            boxes: vec![(Vec3::new(0.0, 1.5, 4.25), Vec3::new(10.0, 1.5, 0.25))],
            start: Vec3::ZERO,
            seconds: 30.0,
            stick: follow(vec![
                ([0.0, 8.0], 1.4, 4.0),
                ([0.0, 0.0], 1.4, 3.0),
                ([6.0, 8.0], 1.4, 5.0),
                ([-6.0, 3.6], 1.4, 7.0),
                ([-3.0, 0.0], 3.5, 2.0),
                ([3.0, 8.0], 3.5, 4.0),
                ([3.0, 8.0], 0.0, 5.0),
            ]),
        },
        Course {
            name: "stairs",
            boxes: stairs(0.0, 2.0, 6, 0.17, 0.3, 3.0),
            start: Vec3::ZERO,
            seconds: 40.0,
            stick: follow(vec![
                ([0.0, 10.0], 1.3, 10.0),
                ([0.0, 0.0], 1.3, 10.0),
                ([0.0, 10.0], 2.5, 7.0),
                ([0.0, 0.0], 2.5, 7.0),
                ([0.0, 0.0], 0.0, 6.0),
            ]),
        },
        Course { name: "boxes", boxes: row, start: Vec3::ZERO, seconds: 40.0, stick: follow(vec![([0.0, 40.0], 1.3, 40.0)]) },
        Course { name: "yard", boxes: crate::yard(), start: Vec3::ZERO, seconds: crate::LEGS.iter().map(|l| l.2).sum(), stick: follow(crate::LEGS.to_vec()) },
    ]
}

/// How far `p` is inside the box, if it is.
fn inside(p: Vec3, (centre, half): (Vec3, Vec3)) -> Option<f32> {
    let d = half - (p - centre).abs();
    (d.min_element() > 0.0).then(|| d.min_element())
}

#[derive(Default)]
struct Score {
    seconds: f32,
    /// Deepest a body point went into a box or the floor, and for how long
    /// something was more than 3 cm in.
    deepest: f32,
    into: f32,
    /// Seconds standing more than 12 cm over the ground, and with the hips
    /// under 65 cm over it.
    floating: f32,
    sunk: f32,
    /// Seconds bent more than 35° from upright, walking about.
    bent: f32,
    bent_in: std::collections::BTreeMap<String, usize>,
    /// Fastest a joint moved, less the body.
    pop: f32,
    jumps: usize,
    /// Jumps whose pose was more than half a radian off.
    hard_jumps: usize,
    slide: Vec<f32>,
    /// How far IK moved the feet off the animation, each step.
    feet_off: Vec<f32>,
    /// Each step's sharpest joint acceleration against the body, m/s².
    jerk: Vec<f32>,
    sharp_joints: Vec<(f32, usize)>,
    into_joints: std::collections::BTreeMap<String, usize>,
    speed_off: Vec<f32>,
    moments: Vec<(f32, f32, String)>,
}

fn mean(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len().max(1) as f32
}

pub fn run(db: Arc<Database>, verbose: bool) -> anyhow::Result<()> {
    let dt = 1.0 / 60.0;
    let skeleton = db.skeleton.clone();
    let feet = db.feet();
    let toes = db.toes();
    let root = skeleton.joints.iter().position(|j| j.parent.is_none()).unwrap_or(0);
    let neck = skeleton.joints.iter().position(|j| j.name.ends_with("Neck")).unwrap_or(root);
    println!(
        "{:8} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6} {:>7} {:>7}",
        "course", "deep m", "into s", "float s", "sunk s", "pop m/s", "jumps", "hard", "slide", "spd off", "µs/step"
    );
    println!("{:8} jerk p50/p95/p99 m/s² in the last column group", "");
    mocap_jerk(&db, dt);
    for course in courses() {
        let mut walker = Walker::new(db.clone(), course.boxes.clone(), course.start);
        let mut score = Score::default();
        let mut last: Option<Vec<Mat4>> = None;
        let mut before: Option<Vec<Vec3>> = None;
        let mut last_root = walker.matcher.root.0;
        let mut floating_run = 0.0;
        let mut last_ask = Vec3::ZERO;
        let mut since_change = 0.0;
        let mut clock = std::time::Duration::ZERO;
        let steps = (course.seconds / dt) as usize;
        let ground = |walker: &Walker, p: Vec3| {
            walker
                .physics
                .cast_ray_with_normal(Vec3::new(p.x, p.y + 0.5, p.z), -Vec3::Y, 3.0, true)
                .map_or(0.0, |h| h.0.y.max(0.0))
                .max(0.0)
        };
        for step in 0..steps {
            let t = step as f32 * dt;
            let ask = (course.stick)(t, walker.matcher.root.0);
            if ask.distance(last_ask) > 0.3 {
                since_change = 0.0;
            }
            last_ask = ask;
            since_change += dt;
            let jumps = walker.matcher.jumps;
            if let Some((name, at)) = std::env::var("MM_AT").ok().and_then(|v| v.split_once(':').map(|(a, b)| (a.to_string(), b.parse::<f32>().unwrap_or(0.0)))) {
                walker.matcher.debug = name == course.name && (t - at).abs() < 0.06;
                if walker.matcher.debug {
                    eprintln!("{t:.3}s frame {}", walker.matcher.frame);
                }
            }
            let started = std::time::Instant::now();
            walker.step(ask, dt);
            clock += started.elapsed();
            if walker.matcher.jumps > jumps {
                score.jumps += 1;
                if let Some((off, from, to)) = walker.matcher.last_jump {
                    if off > 0.5 {
                        score.hard_jumps += 1;
                        let name = |f: usize| db.clips[db.clip_of(f)].0.clone();
                        score.moments.push((t, off, format!("hard jump {off:.2} rad {} {} -> {} {}", name(from), from, name(to), to)));
                    }
                }
            }
            let world = walker.world.clone();
            if !walker.matcher.committed() {
                score.feet_off.push(walker.matcher.feet_off);
            }
            let body: Vec<(usize, Vec3)> = skeleton
                .joints
                .iter()
                .enumerate()
                .flat_map(|(j, joint)| {
                    let here = world[j].w_axis.truncate();
                    let mid = joint.parent.map(|p| (j, (here + world[p as usize].w_axis.truncate()) / 2.0));
                    std::iter::once((j, here)).chain(mid)
                })
                .collect();
            let floor = (Vec3::new(0.0, -50.0, 0.0), Vec3::new(1000.0, 50.0, 1000.0));
            let (deep, deep_joint) = body
                .iter()
                .filter_map(|&(j, p)| course.boxes.iter().copied().chain([floor]).filter_map(|b| inside(p, b)).reduce(f32::max).map(|d| (d, j)))
                .fold((0.0f32, 0), |a, b| if b.0 > a.0 { b } else { a });
            if deep > 0.05 {
                *score.into_joints.entry(skeleton.joints[deep_joint].name.clone()).or_insert(0) += 1;
            }
            if deep > score.deepest + 0.02 && deep > 0.05 {
                let clip = &db.clips[db.clip_of(walker.matcher.frame)].0;
                score.moments.push((t, deep, format!("{deep:.2} m into something ({}, {clip}, committed {})", skeleton.joints[deep_joint].name, walker.matcher.committed())));
            }
            score.deepest = score.deepest.max(deep);
            if deep > 0.03 {
                score.into += dt;
            }
            let calm = walker.matcher.grounded && !walker.matcher.committed();
            let lowest = feet
                .iter()
                .chain(toes.iter().flatten())
                .map(|&j| {
                    let p = world[j].w_axis.truncate();
                    p.y - ground(&walker, p)
                })
                .fold(f32::INFINITY, f32::min);
            floating_run = if calm && lowest > 0.12 { floating_run + dt } else { 0.0 };
            if floating_run > 0.2 {
                score.floating += dt;
                if (floating_run - 0.2).abs() < dt {
                    score.moments.push((t, lowest, format!("floating {lowest:.2} m")));
                }
            }
            let hips = world[root].w_axis.truncate();
            let neck = world[neck].w_axis.truncate();
            let lean = (neck - hips).normalize_or_zero().dot(Vec3::Y).clamp(-1.0, 1.0).acos().to_degrees();
            if calm && lean > 35.0 {
                score.bent += dt;
                *score.bent_in.entry(db.clips[db.clip_of(walker.matcher.frame)].0.clone()).or_insert(0) += 1;
                let clip = &db.clips[db.clip_of(walker.matcher.frame)].0;
                if score.bent < dt * 1.5 || lean > 55.0 {
                    score.moments.push((t, lean / 100.0, format!("bent {lean:.0}° in {clip} {}", walker.matcher.frame)));
                }
            }
            let hips_over = hips.y - ground(&walker, hips);
            if calm && hips_over < 0.65 {
                score.sunk += dt;
                if score.sunk < dt * 1.5 || hips_over < 0.45 {
                    score.moments.push((t, 0.65 - hips_over, format!("hips {hips_over:.2} m up")));
                }
            }
            let at = walker.matcher.root.0;
            if let Some(last) = &last {
                let down = db.contacts(walker.matcher.frame);
                for (side, &foot) in feet.iter().enumerate() {
                    if down[side] {
                        let (a, b) = (last[foot].w_axis.truncate(), world[foot].w_axis.truncate());
                        score.slide.push(Vec3::new(b.x - a.x, 0.0, b.z - a.z).length() / dt);
                    }
                }
                for (j, (a, b)) in last.iter().zip(&world).enumerate() {
                    let moved = (b.w_axis.truncate() - a.w_axis.truncate()) - (at - last_root);
                    let speed = moved.length() / dt;
                    if speed > 12.0 && speed > score.pop {
                        let clip = &db.clips[db.clip_of(walker.matcher.frame)].0;
                        score.moments.push((t, speed, format!(
                            "pop {speed:.1} m/s {} ({clip} {}, committed {}, jumped {}, root dy {:.3})",
                            skeleton.joints[j].name, walker.matcher.frame, walker.matcher.committed(),
                            walker.matcher.jumps > jumps, at.y - last_root.y
                        )));
                    }
                    score.pop = score.pop.max(speed);
                }
                if since_change > 1.5 && calm && course.name == "flat" {
                    let speed = Vec3::new(at.x - last_root.x, 0.0, at.z - last_root.z).length() / dt;
                    score.speed_off.push((speed - ask.length()).abs());
                }
            }
            let local: Vec<Vec3> = world.iter().map(|m| m.w_axis.truncate() - at).collect();
            if let (Some(before), Some(last)) = (&before, &last) {
                let last_local: Vec<Vec3> = last.iter().map(|m| m.w_axis.truncate() - last_root).collect();
                let (joint, sharpest) = local
                    .iter()
                    .zip(&last_local)
                    .zip(before)
                    .map(|((a, b), c)| (*a - *b * 2.0 + *c).length() / (dt * dt))
                    .enumerate()
                    .fold((0, 0.0f32), |best, (j, v)| if v > best.1 { (j, v) } else { best });
                score.jerk.push(sharpest);
                score.sharp_joints.push((sharpest, joint));
            }
            before = last.as_ref().map(|l| l.iter().map(|m| m.w_axis.truncate() - last_root).collect());
            last = Some(world);
            last_root = at;
            score.seconds += dt;
        }
        println!(
            "{:8} {:>7.2} {:>7.1} {:>7.1} {:>7.1} {:>7.1} {:>7} {:>6} {:>6.3} {:>7.2} {:>7.0}",
            course.name,
            score.deepest,
            score.into,
            score.floating,
            score.sunk,
            score.pop,
            score.jumps,
            score.hard_jumps,
            mean(&score.slide),
            mean(&score.speed_off),
            clock.as_secs_f64() * 1e6 / steps as f64,
        );
        let (p50, p95, p99) = percentiles(&mut score.jerk);
        let (o50, o95, o99) = percentiles(&mut score.feet_off);
        println!("{:8} jerk {p50:.0} / {p95:.0} / {p99:.0}, bent {:.1} s, feet off {o50:.2} / {o95:.2} / {o99:.2} m", "", score.bent);
        if verbose {
            score.sharp_joints.sort_by(|a, b| b.0.total_cmp(&a.0));
            let top = &score.sharp_joints[..score.sharp_joints.len() / 50];
            let mut counts = std::collections::BTreeMap::new();
            for (_, j) in top {
                *counts.entry(skeleton.joints[*j].name.clone()).or_insert(0) += 1;
            }
            println!("{:8} sharpest 2%: {counts:?}", "");
            println!("{:8} into things: {:?}", "", score.into_joints);
            println!("{:8} bent in: {:?}", "", score.bent_in);
        }
        if verbose {
            score.moments.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (t, _, what) in score.moments.iter().take(8) {
                println!("    {t:6.2}s  {what}");
            }
        }
    }
    Ok(())
}

fn percentiles(v: &mut [f32]) -> (f32, f32, f32) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    v.sort_by(f32::total_cmp);
    let at = |q: f32| v[((v.len() - 1) as f32 * q) as usize];
    (at(0.5), at(0.95), at(0.99))
}

/// The same measure on the takes themselves, played straight at 60 Hz:
/// how sharp real motion is.
fn mocap_jerk(db: &Database, dt: f32) {
    let mut all = Vec::new();
    for (_, range) in db.clips.iter().take(6) {
        let mut history: Vec<Vec<Vec3>> = Vec::new();
        let frames = range.len() as f32 / db.setup.rate;
        let mut t = 0.0;
        while t < frames - 0.1 {
            let f = t * db.setup.rate;
            let i = range.start + f as usize;
            let a = db.skeleton.world_matrices(db.pose(i));
            let b = db.skeleton.world_matrices(db.pose(i + 1));
            let k = f.fract();
            let local: Vec<Vec3> = a.iter().zip(&b).map(|(a, b)| a.w_axis.truncate().lerp(b.w_axis.truncate(), k)).collect();
            history.push(local);
            if history.len() >= 3 {
                let n = history.len();
                let sharpest = (0..history[n - 1].len())
                    .map(|j| (history[n - 1][j] - history[n - 2][j] * 2.0 + history[n - 3][j]).length() / (dt * dt))
                    .fold(0.0f32, f32::max);
                all.push(sharpest);
            }
            t += dt;
        }
    }
    let (p50, p95, p99) = percentiles(&mut all);
    println!("{:8} jerk {p50:.0} / {p95:.0} / {p99:.0}  (the takes played straight)", "mocap");
}
