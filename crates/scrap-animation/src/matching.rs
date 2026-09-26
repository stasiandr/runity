//! Motion matching on a skeleton: no graph, no transitions — every frame
//! of every clip given is a place the character can be, and every tenth of
//! a second it jumps to the frame whose features best match what it is
//! doing and where it is asked to go (Clavet, "Motion Matching and The
//! Road to Next-Gen Animation", GDC 2016; Holden, "Code vs Data Driven
//! Displacement", 2021, whose feature set and springs this follows).
//!
//! A frame's features, all in the character's own space (its root on the
//! ground under the hips, facing where the legs face):
//!
//! * where each foot is, and how fast it moves;
//! * how fast the hips move;
//! * where the root will be a third, two thirds and one second on, and
//!   which way it will face there.
//!
//! The first three say "a pose that follows on from this one", the last
//! "going where the stick points". The query takes the first from the
//! frame playing and the last from a spring that chases the asked-for
//! velocity — the same spring that moves the character, so the motion is
//! the animation's own and the character is kept within a few centimetres
//! of where the spring says it is.
//!
//! A jump between frames is smoothed by inertialization: the difference
//! between the pose left and the pose arrived at is kept as an offset and
//! decays to nothing, so the new frame plays at once, with no blend of two
//! motions and no foot sliding between them.

use std::ops::Range;

use glam::{Mat4, Quat, Vec3};

use crate::animation::{bare_joint_name, Clip, PoseTransform, Skeleton};

/// How the database is cut and weighed.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    /// Frames a second the clips are sampled at.
    pub rate: f32,
    /// The feet, left then right, by name less the rig's prefix. Their
    /// grandparents are the thighs, which say which way the character
    /// faces.
    pub feet: [String; 2],
    /// Frames ahead the trajectory is matched at.
    pub ahead: [usize; 3],
    /// Weights of the feature groups: foot positions, foot velocities, hip
    /// velocity, trajectory positions, trajectory directions.
    pub weights: [f32; 5],
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            rate: 30.0,
            feet: ["LeftFoot".into(), "RightFoot".into()],
            ahead: [10, 20, 30],
            weights: [0.75, 1.0, 1.0, 1.0, 1.5],
        }
    }
}

/// Floats in one frame's features.
pub const FEATURES: usize = 27;
/// Where each group starts in a frame's features, and its length.
const GROUPS: [Range<usize>; 5] = [0..6, 6..12, 12..15, 15..21, 21..27];

/// Every frame of the clips, ready to search.
#[derive(Debug, Clone)]
pub struct Database {
    pub skeleton: Skeleton,
    pub setup: Setup,
    /// Each frame's joints, local to their parents; the root joint's is in
    /// the character's space. `joints` a frame.
    poses: Vec<PoseTransform>,
    /// Each frame's root velocity (character space, metres a second) and
    /// turn (radians a second about up).
    motion: Vec<(Vec3, f32)>,
    /// Each frame's features, normalized and weighted.
    features: Vec<[f32; FEATURES]>,
    /// What was taken off each feature, and what it was divided by.
    offset: [f32; FEATURES],
    scale: [f32; FEATURES],
    /// Frames of each clip, and each clip's name.
    pub clips: Vec<(String, Range<usize>)>,
    /// Joints of the feet, and their knees and hips.
    legs: [(usize, usize, usize); 2],
    /// Whether each foot is down, each frame: low and still.
    contacts: Vec<[bool; 2]>,
}

/// Which way a character stands: its forward on the ground, from the
/// thighs (left minus right, crossed with up).
fn facing(world: &[Mat4], thighs: [usize; 2]) -> Vec3 {
    let across = world[thighs[0]].w_axis.truncate() - world[thighs[1]].w_axis.truncate();
    let forward = across.cross(Vec3::Y);
    Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::Z)
}

fn yaw(forward: Vec3) -> Quat {
    Quat::from_rotation_y(forward.x.atan2(forward.z))
}

impl Database {
    /// The database of `clips` on `skeleton`. Clips too short to see a
    /// second ahead are left out.
    pub fn build(skeleton: &Skeleton, clips: &[Clip], setup: Setup) -> Result<Self, String> {
        let find = |name: &str| {
            skeleton
                .joints
                .iter()
                .position(|j| bare_joint_name(&j.name) == name)
                .ok_or_else(|| format!("no joint {name} in the skeleton"))
        };
        let feet = [find(&setup.feet[0])?, find(&setup.feet[1])?];
        let thigh = |foot: usize| {
            let knee = skeleton.joints[foot].parent.ok_or("a foot with no knee")? as usize;
            Ok::<usize, String>(skeleton.joints[knee].parent.ok_or("a knee with no thigh")? as usize)
        };
        let thighs = [thigh(feet[0])?, thigh(feet[1])?];
        let knee = |foot: usize| skeleton.joints[foot].parent.unwrap_or_default() as usize;
        let legs = [(feet[0], knee(feet[0]), thighs[0]), (feet[1], knee(feet[1]), thighs[1])];
        let Some(root) = skeleton.joints.iter().position(|j| j.parent.is_none()) else {
            return Err("a skeleton with no root".into());
        };
        let joints = skeleton.len();
        let horizon = *setup.ahead.iter().max().unwrap_or(&30);
        let rate = setup.rate;

        let mut poses = Vec::new();
        let mut motion = Vec::new();
        let mut raw = Vec::new();
        let mut ranges = Vec::new();
        let mut heights = Vec::new();
        let mut speeds = Vec::new();
        for clip in clips {
            let count = (clip.duration * rate).floor() as usize + 1;
            if count <= horizon + 2 {
                continue;
            }
            let local: Vec<Vec<PoseTransform>> =
                (0..count).map(|i| clip.sample(skeleton, i as f32 / rate, false)).collect();
            let world: Vec<Vec<Mat4>> = local.iter().map(|p| skeleton.world_matrices(p)).collect();
            // The root: on the ground under the hips, facing where the legs
            // face, smoothed over a quarter second so the sway of a stride
            // does not turn it.
            let raw_facing: Vec<Vec3> = world.iter().map(|w| facing(w, thighs)).collect();
            let roots: Vec<(Vec3, Quat)> = (0..count)
                .map(|i| {
                    let mut sum = Vec3::ZERO;
                    for k in -4i32..=4 {
                        let j = (i as i32 + k).clamp(0, count as i32 - 1) as usize;
                        sum += raw_facing[j] * (-(k * k) as f32 / 8.0).exp();
                    }
                    let hips = world[i][root].w_axis.truncate();
                    (Vec3::new(hips.x, 0.0, hips.z), yaw(sum.normalize_or(Vec3::Z)))
                })
                .collect();
            let start = poses.len() / joints;
            for i in 0..count {
                let (at, turn) = roots[i];
                let into = Mat4::from_rotation_translation(turn, at).inverse();
                let mut pose = local[i].clone();
                let hips = into * local[i][root].matrix();
                let (_, rotation, translation) = hips.to_scale_rotation_translation();
                pose[root].translation = translation.to_array();
                pose[root].rotation = rotation.to_array();
                poses.extend(pose);

                let next = (i + 1).min(count - 1);
                let prev = if next == i { i - 1 } else { i };
                let (a, ta) = roots[prev];
                let (b, tb) = roots[prev + 1];
                let velocity = ta.inverse() * (b - a) * rate;
                let mut step = ta.inverse() * tb;
                if step.w < 0.0 {
                    step = -step;
                }
                let spin = step.to_scaled_axis().y * rate;
                motion.push((velocity, spin));

                let velocity_of = |joint: usize| {
                    let p = world[prev][joint].w_axis.truncate();
                    let n = world[prev + 1][joint].w_axis.truncate();
                    turn.inverse() * (n - p) * rate
                };
                let mut f = [0.0f32; FEATURES];
                for (side, &foot) in feet.iter().enumerate() {
                    let p = into.transform_point3(world[i][foot].w_axis.truncate());
                    f[side * 3..side * 3 + 3].copy_from_slice(&p.to_array());
                    f[6 + side * 3..9 + side * 3].copy_from_slice(&velocity_of(foot).to_array());
                }
                f[12..15].copy_from_slice(&velocity_of(root).to_array());
                let mut height = [0.0; 2];
                let mut speed = [0.0; 2];
                for side in 0..2 {
                    let v = velocity_of(feet[side]);
                    height[side] = world[i][feet[side]].w_axis.y;
                    speed[side] = Vec3::new(v.x, 0.0, v.z).length();
                }
                heights.push(height);
                speeds.push(speed);
                for (k, &ahead) in setup.ahead.iter().enumerate() {
                    let (there, facing_there) = roots[(i + ahead).min(count - 1)];
                    let p = into.transform_point3(there);
                    let d = turn.inverse() * (facing_there * Vec3::Z);
                    f[15 + k * 2..17 + k * 2].copy_from_slice(&[p.x, p.z]);
                    f[21 + k * 2..23 + k * 2].copy_from_slice(&[d.x, d.z]);
                }
                raw.push(f);
            }
            ranges.push((clip.name.clone(), start..start + count));
        }
        if raw.is_empty() {
            return Err("no clip long enough to match".into());
        }
        // A foot is down when it is near the lowest it goes and barely
        // moving: its floor is found from the data, whatever the rig.
        let mut lows: Vec<f32> = heights.iter().flatten().copied().collect();
        lows.sort_by(f32::total_cmp);
        let floor = lows[lows.len() / 20];
        let contacts = heights
            .iter()
            .zip(&speeds)
            .map(|(h, v)| std::array::from_fn(|side| h[side] < floor + 0.05 && v[side] < 0.4))
            .collect();

        // Each group divided by its own spread, then weighed: a centimetre
        // of foot and a centimetre of trajectory are not worth the same.
        let n = raw.len() as f32;
        let mut offset = [0.0f32; FEATURES];
        for f in &raw {
            for d in 0..FEATURES {
                offset[d] += f[d] / n;
            }
        }
        let mut scale = [1.0f32; FEATURES];
        for (group, weight) in GROUPS.iter().zip(setup.weights) {
            let mut spread = 0.0;
            for f in &raw {
                for d in group.clone() {
                    spread += (f[d] - offset[d]).powi(2);
                }
            }
            let deviation = (spread / (n * group.len() as f32)).sqrt().max(1e-4);
            for d in group.clone() {
                scale[d] = deviation / weight.max(1e-4);
            }
        }
        let features = raw
            .iter()
            .map(|f| std::array::from_fn(|d| (f[d] - offset[d]) / scale[d]))
            .collect();
        Ok(Self {
            skeleton: skeleton.clone(),
            setup,
            poses,
            motion,
            features,
            offset,
            scale,
            clips: ranges,
            legs,
            contacts,
        })
    }

    pub fn len(&self) -> usize {
        self.features.len()
    }

    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }

    /// A frame's joints.
    pub fn pose(&self, frame: usize) -> &[PoseTransform] {
        let n = self.skeleton.len();
        &self.poses[frame * n..(frame + 1) * n]
    }

    /// The clip a frame is in, by index.
    pub fn clip_of(&self, frame: usize) -> usize {
        self.clips.iter().position(|(_, r)| r.contains(&frame)).unwrap_or(0)
    }

    /// Whether a frame is the last of its clip.
    fn at_end(&self, frame: usize) -> bool {
        self.clips.iter().any(|(_, r)| r.end == frame + 1)
    }

    /// Frames that can be jumped to: not so near a clip's end that there
    /// is nothing to play.
    fn searchable(&self) -> impl Iterator<Item = usize> + '_ {
        let tail = self.setup.ahead[0].max(10);
        self.clips.iter().flat_map(move |(_, r)| r.start..r.end.saturating_sub(tail).max(r.start + 1))
    }

    /// Features as they are stored, from raw values.
    fn normalize(&self, raw: &[f32; FEATURES]) -> [f32; FEATURES] {
        std::array::from_fn(|d| (raw[d] - self.offset[d]) / self.scale[d])
    }

    /// The frame nearest `query`, and how far it is. Brute force with an
    /// early out: a few tens of thousands of frames take well under a
    /// millisecond.
    pub fn search(&self, query: &[f32; FEATURES]) -> (usize, f32) {
        let mut best = (0, f32::INFINITY);
        for frame in self.searchable() {
            let f = &self.features[frame];
            let mut cost = 0.0;
            for d in 0..FEATURES {
                cost += (f[d] - query[d]).powi(2);
                if cost >= best.1 {
                    break;
                }
            }
            if cost < best.1 {
                best = (frame, cost);
            }
        }
        best
    }

    fn cost(&self, frame: usize, query: &[f32; FEATURES]) -> f32 {
        self.features[frame].iter().zip(query).map(|(a, b)| (a - b).powi(2)).sum()
    }

    /// The feet's joints, left then right.
    pub fn feet(&self) -> [usize; 2] {
        [self.legs[0].0, self.legs[1].0]
    }

    /// Whether each foot is down at a frame.
    pub fn contacts(&self, frame: usize) -> [bool; 2] {
        self.contacts[frame]
    }
}

/// Spring damping for a half-life: how fast a spring closes half the gap.
fn damping(halflife: f32) -> f32 {
    4.0 * std::f32::consts::LN_2 / (halflife + 1e-5)
}

/// A value and its velocity decaying to zero, critically damped.
fn decay<T>(x: T, v: T, halflife: f32, dt: f32) -> (T, T)
where
    T: Copy + std::ops::Add<Output = T> + std::ops::Sub<Output = T> + std::ops::Mul<f32, Output = T>,
{
    let y = damping(halflife) / 2.0;
    let j1 = v + x * y;
    let e = (-y * dt).exp();
    ((x + j1 * dt) * e, (v - j1 * (y * dt)) * e)
}

/// A position, velocity and acceleration chasing a goal velocity,
/// `dt` on (Holden's spring character).
fn chase(x: Vec3, v: Vec3, a: Vec3, goal: Vec3, halflife: f32, dt: f32) -> (Vec3, Vec3, Vec3) {
    let y = damping(halflife) / 2.0;
    let j0 = v - goal;
    let j1 = a + j0 * y;
    let e = (-y * dt).exp();
    let x = e * ((-j1) / (y * y) + (-j0 - j1 * dt) / y) + j1 / (y * y) + j0 / y + goal * dt + x;
    let v = e * (j0 + j1 * dt) + goal;
    let a = e * (a - j1 * y * dt);
    (x, v, a)
}

/// A rotation and its angular velocity chasing a goal rotation.
fn turn_to(q: Quat, w: Vec3, goal: Quat, halflife: f32, dt: f32) -> (Quat, Vec3) {
    let y = damping(halflife) / 2.0;
    let mut gap = q * goal.inverse();
    if gap.w < 0.0 {
        gap = -gap;
    }
    let j0 = gap.to_scaled_axis();
    let j1 = w + j0 * y;
    let e = (-y * dt).exp();
    (Quat::from_scaled_axis(e * (j0 + j1 * dt)) * goal, e * (w - j1 * y * dt))
}

/// What the character is asked to do this step.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Ask {
    /// Metres a second on the ground, in the world.
    pub velocity: Vec3,
    /// Which way to face; along the velocity when `None`.
    pub facing: Option<Vec3>,
}

/// How the matcher reacts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Feel {
    /// How fast the spring reaches the asked-for velocity, and facing.
    pub velocity_halflife: f32,
    pub facing_halflife: f32,
    /// How often to search, seconds.
    pub search_every: f32,
    /// How fast a jump's difference fades.
    pub blend_halflife: f32,
    /// How fast the animation's root is pulled to the spring, and how far
    /// it may stray from it, metres.
    pub hold_halflife: f32,
    pub leash: f32,
    /// Whether a foot that is down stays where it was put, the leg bent to
    /// it, until it lifts.
    pub lock_feet: bool,
    /// How far a locked foot may be left behind before it lets go, metres.
    pub lock_reach: f32,
}

impl Default for Feel {
    fn default() -> Self {
        Self {
            velocity_halflife: 0.27,
            facing_halflife: 0.27,
            search_every: 0.1,
            blend_halflife: 0.1,
            hold_halflife: 0.2,
            leash: 0.15,
            lock_feet: true,
            lock_reach: 0.2,
        }
    }
}

/// One character's place in a database.
#[derive(Debug, Clone)]
pub struct Matcher {
    pub feel: Feel,
    /// The frame playing, and how far toward the next one.
    pub frame: usize,
    between: f32,
    since_search: f32,
    /// The spring the stick drives.
    pub spring: (Vec3, Vec3, Vec3),
    pub spring_turn: (Quat, Vec3),
    /// Where the character is: the animation's root, held near the spring.
    pub root: (Vec3, Quat),
    /// Jumps' differences, fading: each joint's rotation (scaled axis) and
    /// its velocity, and the root joint's translation and its velocity.
    turns: Vec<(Vec3, Vec3)>,
    shift: (Vec3, Vec3),
    /// Each foot: where it is locked, and what is left of the last lock
    /// fading after it let go (offset, velocity), and where the animation
    /// had it last step.
    feet: [FootLock; 2],
    /// Jumps made, for tests and the debug view.
    pub jumps: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct FootLock {
    at: Option<Vec3>,
    fading: (Vec3, Vec3),
    last: Option<Vec3>,
}

impl Matcher {
    /// A character at `at`, facing `facing`, standing on `db`'s first frame.
    pub fn new(db: &Database, at: Vec3, facing: Vec3) -> Self {
        let turn = yaw(Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::Z));
        Self {
            feel: Feel::default(),
            frame: db.searchable().next().unwrap_or(0),
            between: 0.0,
            since_search: f32::INFINITY,
            spring: (at, Vec3::ZERO, Vec3::ZERO),
            spring_turn: (turn, Vec3::ZERO),
            root: (at, turn),
            turns: vec![(Vec3::ZERO, Vec3::ZERO); db.skeleton.len()],
            shift: (Vec3::ZERO, Vec3::ZERO),
            feet: [FootLock::default(); 2],
            jumps: 0,
        }
    }

    /// Where the spring will be `seconds` on, and which way it will face.
    fn spring_ahead(&self, ask: &Ask, goal_turn: Quat, seconds: f32) -> (Vec3, Quat) {
        let (x, v, a) = self.spring;
        let (at, _, _) = chase(x, v, a, ask.velocity, self.feel.velocity_halflife, seconds);
        let (q, w) = self.spring_turn;
        let (turn, _) = turn_to(q, w, goal_turn, self.feel.facing_halflife, seconds);
        (at, turn)
    }

    fn goal_turn(&self, ask: &Ask) -> Quat {
        let along = ask.facing.unwrap_or(ask.velocity);
        let flat = Vec3::new(along.x, 0.0, along.z);
        if flat.length() > 0.05 {
            yaw(flat.normalize())
        } else {
            self.spring_turn.0
        }
    }

    /// The features to look for: the pose from the frame playing, the
    /// trajectory from the spring, both in the character's space.
    pub fn query(&self, db: &Database, ask: &Ask) -> [f32; FEATURES] {
        let raw_now = db.features[self.frame];
        let mut raw: [f32; FEATURES] = std::array::from_fn(|d| raw_now[d] * db.scale[d] + db.offset[d]);
        let goal = self.goal_turn(ask);
        let (root, turn) = self.root;
        for (k, &ahead) in db.setup.ahead.iter().enumerate() {
            let (at, facing) = self.spring_ahead(ask, goal, ahead as f32 / db.setup.rate);
            // The spring's path, carried to start where the character is.
            let p = turn.inverse() * (at - root);
            let d = turn.inverse() * (facing * Vec3::Z);
            raw[15 + k * 2..17 + k * 2].copy_from_slice(&[p.x, p.z]);
            raw[21 + k * 2..23 + k * 2].copy_from_slice(&[d.x, d.z]);
        }
        db.normalize(&raw)
    }

    /// The joints' velocities at a frame: rotation (scaled axis a second)
    /// and the root joint's translation.
    fn velocities(db: &Database, frame: usize) -> (Vec<Vec3>, Vec3) {
        let rate = db.setup.rate;
        let next = if db.at_end(frame) { frame } else { frame + 1 };
        let prev = if next == frame { frame.saturating_sub(1) } else { frame };
        let (a, b) = (db.pose(prev), db.pose(next));
        let spins = a
            .iter()
            .zip(b)
            .map(|(a, b)| {
                let (qa, qb) = (Quat::from_array(a.rotation), Quat::from_array(b.rotation));
                let mut d = qb * qa.inverse();
                if d.w < 0.0 {
                    d = -d;
                }
                d.to_scaled_axis() * rate
            })
            .collect();
        let root = db.skeleton.joints.iter().position(|j| j.parent.is_none()).unwrap_or(0);
        let shift = (Vec3::from_array(b[root].translation) - Vec3::from_array(a[root].translation)) * rate;
        (spins, shift)
    }

    fn jump(&mut self, db: &Database, to: usize) {
        let root = db.skeleton.joints.iter().position(|j| j.parent.is_none()).unwrap_or(0);
        let (from_spin, from_shift) = Self::velocities(db, self.frame);
        let (to_spin, to_shift) = Self::velocities(db, to);
        let from = self.blended(db);
        let dest = db.pose(to);
        for (j, offset) in self.turns.iter_mut().enumerate() {
            let a = Quat::from_array(from[j].rotation);
            let b = Quat::from_array(dest[j].rotation);
            let mut d = a * b.inverse();
            if d.w < 0.0 {
                d = -d;
            }
            *offset = (d.to_scaled_axis(), offset.1 + from_spin[j] - to_spin[j]);
        }
        self.shift = (
            Vec3::from_array(from[root].translation) - Vec3::from_array(dest[root].translation),
            self.shift.1 + from_shift - to_shift,
        );
        self.frame = to;
        self.between = 0.0;
        self.jumps += 1;
    }

    /// The database's pose between the frame playing and the next.
    fn raw_pose(&self, db: &Database) -> Vec<PoseTransform> {
        let next = if db.at_end(self.frame) { self.frame } else { self.frame + 1 };
        db.pose(self.frame).iter().zip(db.pose(next)).map(|(a, b)| a.lerp(b, self.between)).collect()
    }

    /// The pose shown: the database's, with what is left of the jumps.
    fn blended(&self, db: &Database) -> Vec<PoseTransform> {
        let root = db.skeleton.joints.iter().position(|j| j.parent.is_none()).unwrap_or(0);
        let mut pose = self.raw_pose(db);
        for (joint, (offset, _)) in pose.iter_mut().zip(&self.turns) {
            let q = Quat::from_scaled_axis(*offset) * Quat::from_array(joint.rotation);
            joint.rotation = q.normalize().to_array();
        }
        let t = Vec3::from_array(pose[root].translation) + self.shift.0;
        pose[root].translation = t.to_array();
        pose
    }

    /// A step of `dt` seconds toward `ask`: the spring moves, the frame
    /// plays on or jumps, the root follows the animation and is held near
    /// the spring. Returns the pose in the character's space; the
    /// character itself is at [`Matcher::root`].
    pub fn advance(&mut self, db: &Database, ask: &Ask, dt: f32) -> Vec<PoseTransform> {
        let goal = self.goal_turn(ask);
        let (x, v, a) = self.spring;
        self.spring = chase(x, v, a, ask.velocity, self.feel.velocity_halflife, dt);
        let (q, w) = self.spring_turn;
        self.spring_turn = turn_to(q, w, goal, self.feel.facing_halflife, dt);

        // Play on, a frame at a time.
        self.between += dt * db.setup.rate;
        let mut forced = false;
        while self.between >= 1.0 {
            self.between -= 1.0;
            if db.at_end(self.frame) {
                forced = true;
                self.between = 0.0;
            } else {
                self.frame += 1;
            }
        }
        self.since_search += dt;
        if forced || self.since_search >= self.feel.search_every {
            self.since_search = 0.0;
            let query = self.query(db, ask);
            let (best, cost) = db.search(&query);
            let current = if forced { f32::INFINITY } else { db.cost(self.frame, &query) };
            let near = db.clip_of(best) == db.clip_of(self.frame) && best.abs_diff(self.frame) <= 3;
            if cost < current && !near {
                self.jump(db, best);
            }
        }
        for (offset, speed) in &mut self.turns {
            (*offset, *speed) = decay(*offset, *speed, self.feel.blend_halflife, dt);
        }
        self.shift = decay(self.shift.0, self.shift.1, self.feel.blend_halflife, dt);

        // The animation moves the root; the spring holds it.
        let (velocity, spin) = db.motion[self.frame];
        let (mut at, mut turn) = self.root;
        at += turn * velocity * dt;
        turn = (turn * Quat::from_rotation_y(spin * dt)).normalize();
        let pull = 1.0 - (-std::f32::consts::LN_2 * dt / self.feel.hold_halflife).exp();
        at = at.lerp(self.spring.0, pull);
        turn = turn.slerp(self.spring_turn.0, pull);
        let gap = at - self.spring.0;
        if gap.length() > self.feel.leash {
            at = self.spring.0 + gap.normalize() * self.feel.leash;
        }
        self.root = (at, turn);
        let mut pose = self.blended(db);
        if self.feel.lock_feet {
            self.lock_feet(db, &mut pose, dt);
        }
        pose
    }

    /// Keep each foot that is down where it went down: the leg bent to it,
    /// and when it lifts, the difference fading as a jump's does.
    fn lock_feet(&mut self, db: &Database, pose: &mut [PoseTransform], dt: f32) {
        let placed = Mat4::from_rotation_translation(self.root.1, self.root.0);
        let forward = self.root.1 * Vec3::Z;
        let mut world = crate::ik::placed_joints(&db.skeleton, pose, placed);
        let down = db.contacts[self.frame];
        for side in 0..2 {
            let leg = db.legs[side];
            let foot = &mut self.feet[side];
            let animated = world[leg.0].w_axis.truncate();
            let velocity = foot.last.map_or(Vec3::ZERO, |last| (animated - last) / dt.max(1e-4));
            foot.last = Some(animated);
            foot.fading = decay(foot.fading.0, foot.fading.1, self.feel.blend_halflife, dt);
            let shown = animated + foot.fading.0;
            foot.at = match foot.at {
                Some(at) if down[side] && at.distance(animated) <= self.feel.lock_reach => Some(at),
                Some(at) => {
                    // Let go: from where it was held, fading to the animation.
                    foot.fading = (at - animated, -velocity);
                    None
                }
                None if down[side] => Some(shown),
                None => None,
            };
            let target = foot.at.unwrap_or(animated + foot.fading.0);
            if target.distance(animated) > 1e-4 {
                let kept = crate::ik::turn_of(world[leg.0]);
                crate::ik::reach_leg(&db.skeleton, pose, placed, &mut world, leg, target, forward);
                let now = crate::ik::turn_of(world[leg.0]);
                crate::ik::turn_joint(&db.skeleton, pose, &world, placed, leg.0, kept * now.inverse());
                world = crate::ik::placed_joints(&db.skeleton, pose, placed);
            }
        }
    }

    /// The pose's joints in the world, the character standing at its root.
    pub fn world(&self, db: &Database, pose: &[PoseTransform]) -> Vec<Mat4> {
        let place = Mat4::from_rotation_translation(self.root.1, self.root.0);
        db.skeleton.world_matrices(pose).into_iter().map(|m| place * m).collect()
    }

    /// Where the trajectory is asked to go, in the world: the spring now
    /// and at each matched horizon. For the debug view.
    pub fn wanted(&self, db: &Database, ask: &Ask) -> Vec<(Vec3, Vec3)> {
        let goal = self.goal_turn(ask);
        std::iter::once(0)
            .chain(db.setup.ahead)
            .map(|k| {
                let (at, turn) = self.spring_ahead(ask, goal, k as f32 / db.setup.rate);
                (at, turn * Vec3::Z)
            })
            .collect()
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Channel, Joint, Path};

    /// Hips and two legs, standing a metre up.
    fn legs() -> Skeleton {
        let joint = |name: &str, parent: Option<u16>, at: [f32; 3]| Joint {
            name: name.into(),
            parent,
            inverse_bind: Mat4::IDENTITY.to_cols_array_2d(),
            rest: PoseTransform { translation: at, ..Default::default() },
        };
        Skeleton {
            joints: vec![
                joint("Hips", None, [0.0, 0.95, 0.0]),
                joint("LeftUpLeg", Some(0), [0.1, 0.0, 0.0]),
                joint("LeftLeg", Some(1), [0.0, -0.45, 0.0]),
                joint("LeftFoot", Some(2), [0.0, -0.45, 0.0]),
                joint("RightUpLeg", Some(0), [-0.1, 0.0, 0.0]),
                joint("RightLeg", Some(4), [0.0, -0.45, 0.0]),
                joint("RightFoot", Some(5), [0.0, -0.45, 0.0]),
            ],
        }
    }

    /// Four seconds along +Z at `speed(t)` metres a second, the legs
    /// swinging a stride a second, as wide as the speed.
    fn going(name: &str, speed: impl Fn(f32) -> f32) -> Clip {
        let times: Vec<f32> = (0..=120).map(|i| i as f32 / 30.0).collect();
        let mut z = 0.0;
        let mut hips = Vec::new();
        let mut phase = 0.0f32;
        let mut phases = Vec::new();
        for t in &times {
            hips.extend([0.0, 0.95, z]);
            phases.push((phase, speed(*t).min(1.0)));
            z += speed(*t) / 30.0;
            phase += speed(*t).min(1.0) * std::f32::consts::TAU / 30.0;
        }
        let swing = |offset: f32| {
            phases
                .iter()
                .flat_map(|(p, wide)| Quat::from_rotation_x((p + offset).sin() * 0.4 * wide).to_array())
                .collect::<Vec<f32>>()
        };
        let channel = |joint, path, values| Channel { joint, path, times: times.clone(), values };
        Clip {
            name: name.into(),
            duration: 4.0,
            channels: vec![
                channel(0, Path::Translation, hips),
                channel(1, Path::Rotation, swing(0.0)),
                channel(4, Path::Rotation, swing(std::f32::consts::PI)),
            ],
        }
    }

    /// Standing, walking, and walking then stopping.
    fn clips() -> Vec<Clip> {
        vec![
            going("stand", |_| 0.0),
            going("walk", |_| 1.2),
            going("stop", |t| 1.2 * (1.0 - (t - 1.0)).clamp(0.0, 1.0)),
        ]
    }

    #[test]
    fn asked_to_walk_it_walks_and_asked_to_stop_it_stands() {
        let skeleton = legs();
        let db = Database::build(&skeleton, &clips(), Setup::default()).unwrap();
        assert_eq!(db.clips.len(), 3);
        let mut matcher = Matcher::new(&db, Vec3::ZERO, Vec3::Z);
        let dt = 1.0 / 60.0;
        let ask = Ask { velocity: Vec3::Z * 1.2, facing: None };
        for _ in 0..120 {
            matcher.advance(&db, &ask, dt);
        }
        assert_eq!(db.clips[db.clip_of(matcher.frame)].0, "walk");
        let before = matcher.root.0;
        for _ in 0..60 {
            matcher.advance(&db, &ask, dt);
        }
        let speed = (matcher.root.0 - before).length();
        assert!((speed - 1.2).abs() < 0.15, "a second at {speed} m/s");

        let stop = Ask::default();
        for _ in 0..90 {
            matcher.advance(&db, &stop, dt);
        }
        let still = matcher.root.0;
        matcher.advance(&db, &stop, dt);
        assert!(matcher.root.0.distance(still) < 0.01, "stands in {}", db.clips[db.clip_of(matcher.frame)].0);
    }

    #[test]
    fn a_turned_ask_turns_the_character() {
        let db = Database::build(&legs(), &clips(), Setup::default()).unwrap();
        let mut matcher = Matcher::new(&db, Vec3::ZERO, Vec3::Z);
        let ask = Ask { velocity: Vec3::X * 1.2, facing: None };
        for _ in 0..180 {
            matcher.advance(&db, &ask, 1.0 / 60.0);
        }
        let facing = matcher.root.1 * Vec3::Z;
        assert!(facing.dot(Vec3::X) > 0.95, "faces where it goes: {facing}");
        assert!(matcher.root.0.x > 1.5 && matcher.root.0.z.abs() < 0.6, "went along x: {}", matcher.root.0);
    }
}
