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
    /// The hands, left then right, by name less the rig's prefix: where
    /// they are and how they move are matched too, or a jump between
    /// frames swings the arms. A rig without them matches without.
    pub hands: [String; 2],
    /// Frames ahead the trajectory is matched at.
    pub ahead: [usize; 3],
    /// Weights of the feature groups: foot positions, foot velocities, hip
    /// velocity, trajectory positions, trajectory directions, and the
    /// height of the ground along the trajectory, hand positions and hand
    /// velocities.
    pub weights: [f32; 8],
    /// Frames with the hips lower than this over the ground — crawling,
    /// ducking under — are played on to, never jumped to: nothing here
    /// asks for them yet. Metres; `0.0` keeps them all.
    pub lowest_hips: f32,
    /// Frames bent further than this from upright (hips to neck, degrees),
    /// or with a hand lower than `lowest_hands` over the ground, are not
    /// jumped to either, unless they are part of a climb: catching breath
    /// hands on knees, a hand on a step, going up stairs on all fours are
    /// not walking. `180.0` and `0.0` keep them.
    pub steepest_lean: f32,
    pub lowest_hands: f32,
    /// Each clip also mirrored, left for right: twice the frames, and a
    /// turn one way as good as the other.
    pub mirror: bool,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            rate: 30.0,
            feet: ["LeftFoot".into(), "RightFoot".into()],
            hands: ["LeftHand".into(), "RightHand".into()],
            ahead: [10, 20, 30],
            weights: [0.75, 1.0, 1.0, 1.0, 1.5, 1.5, 0.5, 0.4],
            lowest_hips: 0.7,
            steepest_lean: 30.0,
            lowest_hands: 0.6,
            mirror: true,
        }
    }
}

/// Floats in one frame's features.
pub const FEATURES: usize = 42;
/// Where each group starts in a frame's features, and its length.
const GROUPS: [Range<usize>; 8] = [0..6, 6..12, 12..15, 15..21, 21..27, 27..30, 30..36, 36..42];

/// Every frame of the clips, ready to search.
#[derive(Debug, Clone)]
pub struct Database {
    pub skeleton: Skeleton,
    pub setup: Setup,
    /// Each frame's joints, local to their parents; the root joint's is in
    /// the character's space. `joints` a frame.
    poses: Vec<PoseTransform>,
    /// Each frame's root velocity (character space, metres a second; up is
    /// the ground it stands on rising) and turn (radians a second about up).
    motion: Vec<(Vec3, f32)>,
    /// How high each frame's ground is over its take's floor: what the
    /// planted feet stand on — a step, a box, a ledge.
    support: Vec<f32>,
    /// Frames a search may jump to.
    open: Vec<bool>,
    /// Each frame's features, normalized and weighted.
    features: Vec<[f32; FEATURES]>,
    /// What was taken off each feature, and what it was divided by.
    offset: [f32; FEATURES],
    scale: [f32; FEATURES],
    /// Frames of each clip, and each clip's name.
    pub clips: Vec<(String, Range<usize>)>,
    /// Joints of the feet, and their knees and hips.
    legs: [(usize, usize, usize); 2],
    /// Each foot's toe, when it has one: the foot stands on the higher of
    /// the ground under the ankle and under the toe.
    toes: [Option<usize>; 2],
    /// Whether each foot is down, each frame: low and still.
    contacts: Vec<[bool; 2]>,
    /// Whether each toe is down, each frame: the heel may be lifting.
    toe_contacts: Vec<[bool; 2]>,
    /// How high an ankle is over the floor it stands on, and a toe.
    ankle: f32,
    toe: f32,
}

/// The joint on the other side: `LeftArm` for `RightArm`, itself for the
/// spine.
fn other_side(skeleton: &Skeleton, joint: usize) -> usize {
    let name = bare_joint_name(&skeleton.joints[joint].name);
    let swapped = if let Some(rest) = name.strip_prefix("Left") {
        format!("Right{rest}")
    } else if let Some(rest) = name.strip_prefix("Right") {
        format!("Left{rest}")
    } else {
        return joint;
    };
    skeleton.joints.iter().position(|j| bare_joint_name(&j.name) == swapped).unwrap_or(joint)
}

/// `clip` played by the character's mirror image: every pose reflected
/// across the body's middle, left joints for right. The middle is found
/// from the rest pose — the plane halfway between the thighs — and each
/// joint's rest turn is kept, so rigs whose joints are not turned alike at
/// rest mirror too. `None` for a skeleton with no left and right.
fn mirror(skeleton: &Skeleton, clip: &Clip, rate: f32) -> Option<Clip> {
    use crate::animation::{Channel, Path};
    let rest_world = skeleton.world_matrices(&skeleton.rest_pose());
    let left = skeleton.joints.iter().position(|j| bare_joint_name(&j.name) == "LeftUpLeg")?;
    let right = other_side(skeleton, left);
    if right == left {
        return None;
    }
    // Reflect across the plane whose normal points from right to left.
    let across = (rest_world[left].w_axis - rest_world[right].w_axis).truncate();
    let axis = across.abs().max_position();
    let mut flip = Vec3::ONE;
    flip[axis] = -1.0;
    let m = Mat4::from_scale(flip);
    let rest_turn: Vec<Quat> = rest_world.iter().map(|w| w.to_scale_rotation_translation().1).collect();
    let count = (clip.duration * rate).floor() as usize + 1;
    let joints = skeleton.len();
    let mut rotations = vec![Vec::with_capacity(count * 4); joints];
    let mut roots = Vec::with_capacity(count * 3);
    let root = skeleton.joints.iter().position(|j| j.parent.is_none())?;
    for i in 0..count {
        let world = skeleton.world_matrices(&clip.sample(skeleton, i as f32 / rate, false));
        // Each joint's world turn, reflected and handed to its other side,
        // then set right against that side's own rest turn.
        let turned: Vec<Quat> = (0..joints)
            .map(|j| {
                let from = other_side(skeleton, j);
                let reflected = (m * Mat4::from_quat(world[from].to_scale_rotation_translation().1) * m)
                    .to_scale_rotation_translation()
                    .1;
                let rest_reflected = (m * Mat4::from_quat(rest_turn[from]) * m).to_scale_rotation_translation().1;
                (reflected * rest_reflected.inverse() * rest_turn[j]).normalize()
            })
            .collect();
        for j in 0..joints {
            let parent = skeleton.joints[j].parent.map_or(Quat::IDENTITY, |p| turned[p as usize]);
            let local = (parent.inverse() * turned[j]).normalize();
            rotations[j].extend(local.to_array());
        }
        roots.extend((m.transform_point3(world[root].w_axis.truncate())).to_array());
    }
    let times: Vec<f32> = (0..count).map(|i| i as f32 / rate).collect();
    let mut channels = vec![Channel { joint: root as u16, path: Path::Translation, times: times.clone(), values: roots }];
    for (j, values) in rotations.into_iter().enumerate() {
        channels.push(Channel { joint: j as u16, path: Path::Rotation, times: times.clone(), values });
    }
    Some(Clip { name: format!("{} (mirrored)", clip.name), duration: times.last().copied().unwrap_or(0.0), channels })
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
        let mirrored: Vec<Clip> = if setup.mirror {
            clips.iter().filter_map(|c| mirror(skeleton, c, setup.rate)).collect()
        } else {
            Vec::new()
        };
        let clips: Vec<&Clip> = clips.iter().chain(&mirrored).collect();
        let find = |name: &str| {
            skeleton
                .joints
                .iter()
                .position(|j| bare_joint_name(&j.name) == name)
                .ok_or_else(|| format!("no joint {name} in the skeleton"))
        };
        let feet = [find(&setup.feet[0])?, find(&setup.feet[1])?];
        let hands = [find(&setup.hands[0]).ok(), find(&setup.hands[1]).ok()];
        let thigh = |foot: usize| {
            let knee = skeleton.joints[foot].parent.ok_or("a foot with no knee")? as usize;
            Ok::<usize, String>(skeleton.joints[knee].parent.ok_or("a knee with no thigh")? as usize)
        };
        let thighs = [thigh(feet[0])?, thigh(feet[1])?];
        let knee = |foot: usize| skeleton.joints[foot].parent.unwrap_or_default() as usize;
        let legs = [(feet[0], knee(feet[0]), thighs[0]), (feet[1], knee(feet[1]), thighs[1])];
        let toe = |foot: usize| skeleton.joints.iter().position(|j| j.parent == Some(foot as u16));
        let toes = [toe(feet[0]), toe(feet[1])];
        let Some(root) = skeleton.joints.iter().position(|j| j.parent.is_none()) else {
            return Err("a skeleton with no root".into());
        };
        let joints = skeleton.len();
        let horizon = *setup.ahead.iter().max().unwrap_or(&30);
        let rate = setup.rate;

        let mut poses = Vec::new();
        let mut motion = Vec::new();
        let mut support = Vec::new();
        let mut contacts = Vec::new();
        let mut toe_contacts = Vec::new();
        let mut raw = Vec::new();
        let mut ranges = Vec::new();
        let mut ankles = Vec::new();
        for &clip in &clips {
            let count = (clip.duration * rate).floor() as usize + 1;
            if count <= horizon + 2 {
                continue;
            }
            let local: Vec<Vec<PoseTransform>> =
                (0..count).map(|i| clip.sample(skeleton, i as f32 / rate, false)).collect();
            let world: Vec<Vec<Mat4>> = local.iter().map(|p| skeleton.world_matrices(p)).collect();
            let at = |i: usize, joint: usize| world[i][joint].w_axis.truncate();
            let pace = |i: usize, joint: usize| {
                let (a, b) = if i + 1 < count { (i, i + 1) } else { (i - 1, i) };
                (at(b, joint) - at(a, joint)).length() * rate
            };
            // The feet's floor in this take: how high an ankle and a toe are
            // when they stand on it — the lowest they go, near enough.
            let low = |joint: usize| {
                let mut ys: Vec<f32> = (0..count).map(|i| at(i, joint).y).collect();
                ys.sort_by(f32::total_cmp);
                ys[ys.len() / 20]
            };
            let ankle = (low(feet[0]) + low(feet[1])) / 2.0;
            let toe_floor = toes.map(|t| t.map(low));
            ankles.push((ankle, toe_floor.iter().flatten().sum::<f32>() / toe_floor.iter().flatten().count().max(1) as f32));
            // A foot is planted while its heel or its toe is still; it
            // stands as high as the lower of the two over their floor.
            let planted: Vec<[Option<f32>; 2]> = (0..count)
                .map(|i| {
                    std::array::from_fn(|side| {
                        let foot = feet[side];
                        let mut height = at(i, foot).y - ankle;
                        let mut still = pace(i, foot) < 0.35;
                        if let (Some(toe), Some(floor)) = (toes[side], toe_floor[side]) {
                            height = height.min(at(i, toe).y - floor);
                            still |= pace(i, toe) < 0.35;
                        }
                        still.then_some(height)
                    })
                })
                .collect();
            // The ground each frame: what the planted feet stand on, carried
            // across the moments no foot is down, then smoothed.
            let stands: Vec<Option<f32>> = planted
                .iter()
                .map(|p| {
                    let down: Vec<f32> = p.iter().flatten().copied().collect();
                    (!down.is_empty()).then(|| down.iter().sum::<f32>() / down.len() as f32)
                })
                .collect();
            let mut ground = vec![0.0f32; count];
            let known: Vec<usize> = (0..count).filter(|&i| stands[i].is_some()).collect();
            for i in 0..count {
                let before = known.iter().rev().find(|&&k| k <= i);
                let after = known.iter().find(|&&k| k >= i);
                ground[i] = match (before, after) {
                    (Some(&a), Some(&b)) if a != b => {
                        let t = (i - a) as f32 / (b - a) as f32;
                        stands[a].unwrap() * (1.0 - t) + stands[b].unwrap() * t
                    }
                    (Some(&a), _) => stands[a].unwrap(),
                    (_, Some(&b)) => stands[b].unwrap(),
                    _ => 0.0,
                };
            }
            let ground: Vec<f32> = (0..count)
                .map(|i| {
                    let (mut sum, mut weight) = (0.0, 0.0);
                    for k in -4i32..=4 {
                        let j = (i as i32 + k).clamp(0, count as i32 - 1) as usize;
                        let w = (-(k * k) as f32 / 8.0).exp();
                        sum += ground[j] * w;
                        weight += w;
                    }
                    sum / weight
                })
                .collect();
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
                    let hips = at(i, root);
                    (Vec3::new(hips.x, ground[i], hips.z), yaw(sum.normalize_or(Vec3::Z)))
                })
                .collect();
            let start = poses.len() / joints;
            for i in 0..count {
                let (here, turn) = roots[i];
                let into = Mat4::from_rotation_translation(turn, here).inverse();
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
                support.push(ground[i]);
                // Locked: the heel down on what the feet stand on, and still.
                contacts.push(std::array::from_fn(|side| {
                    let foot = feet[side];
                    let heel = at(i, foot);
                    let (a, b) = if i + 1 < count { (i, i + 1) } else { (i - 1, i) };
                    let slide = flat(at(b, foot) - at(a, foot)).length() * rate;
                    let over = heel.y - ankle - ground[i];
                    slide < 0.4 && (at(b, foot).y - at(a, foot).y).abs() * rate < 0.4 && over < 0.06
                }));
                toe_contacts.push(std::array::from_fn(|side| {
                    let (Some(toe), Some(floor)) = (toes[side], toe_floor[side]) else { return false };
                    let (a, b) = if i + 1 < count { (i, i + 1) } else { (i - 1, i) };
                    (at(b, toe) - at(a, toe)).length() * rate < 0.4 && at(i, toe).y - floor - ground[i] < 0.05
                }));

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
                for (side, hand) in hands.iter().enumerate() {
                    if let Some(hand) = *hand {
                        let p = into.transform_point3(world[i][hand].w_axis.truncate());
                        f[30 + side * 3..33 + side * 3].copy_from_slice(&p.to_array());
                        f[36 + side * 3..39 + side * 3].copy_from_slice(&velocity_of(hand).to_array());
                    }
                }
                for (k, &ahead) in setup.ahead.iter().enumerate() {
                    let later = (i + ahead).min(count - 1);
                    let (there, facing_there) = roots[later];
                    let p = into.transform_point3(there);
                    let d = turn.inverse() * (facing_there * Vec3::Z);
                    f[15 + k * 2..17 + k * 2].copy_from_slice(&[p.x, p.z]);
                    f[21 + k * 2..23 + k * 2].copy_from_slice(&[d.x, d.z]);
                    f[27 + k] = ground[later] - ground[i];
                }
                raw.push(f);
            }
            ranges.push((clip.name.clone(), start..start + count));
        }
        if raw.is_empty() {
            return Err("no clip long enough to match".into());
        }
        let floor = ankles.iter().map(|a| a.0).sum::<f32>() / ankles.len() as f32;
        let neck = skeleton.joints.iter().position(|j| bare_joint_name(&j.name) == "Neck");
        let open = (0..raw.len())
            .map(|f| {
                let pose = &poses[f * joints..(f + 1) * joints];
                if pose[root].translation[1] < setup.lowest_hips {
                    return false;
                }
                // Part of a climb onto a ledge: the ground changes by nearly
                // half a metre within two thirds of a second either way —
                // bending is what that is. Stairs rise slower, and are
                // walked upright.
                let clip = ranges.iter().find(|(_, r)| r.contains(&f)).map(|(_, r)| r.clone()).unwrap_or(f..f + 1);
                let near = clip.start.max(f.saturating_sub(20))..clip.end.min(f + 20);
                if near.clone().any(|g| (support[g] - support[f]).abs() > 0.45) {
                    return true;
                }
                let world = skeleton.world_matrices(pose);
                let at = |j: usize| world[j].w_axis.truncate();
                let lean = neck.map_or(0.0, |n| (at(n) - at(root)).normalize_or(Vec3::Y).dot(Vec3::Y).clamp(-1.0, 1.0).acos().to_degrees());
                let hands_low = hands.iter().flatten().any(|&h| at(h).y < setup.lowest_hands);
                // Arms out level both sides: the T-pose a take starts with
                // for its calibration, not a way anyone walks.
                let t_pose = match (hands[0], hands[1], neck) {
                    (Some(l), Some(r), Some(n)) => {
                        let level = |h: usize| (at(h).y - at(n).y).abs() < 0.25;
                        level(l) && level(r) && flat(at(l) - at(r)).length() > 1.2
                    }
                    _ => false,
                };
                lean <= setup.steepest_lean && !hands_low && !t_pose
            })
            .collect::<Vec<bool>>();
        // A frame to jump to plays on for a third of a second as good: one
        // that turns into a crouch at once would be left at once, and
        // jumped back to — a pose frozen.
        let open = (0..raw.len())
            .map(|f| {
                let clip = ranges.iter().find(|(_, r)| r.contains(&f)).map(|(_, r)| r.end).unwrap_or(f + 1);
                (f..clip.min(f + 10)).all(|g| open[g])
            })
            .collect();
        let toe_height = ankles.iter().map(|a| a.1).sum::<f32>() / ankles.len() as f32;

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
            // Not below a quarter metre for heights: on flat takes the
            // ground varies by a centimetre, and a centimetre is not a step.
            let least = if group.start == 27 { 0.25 } else { 1e-4 };
            let deviation = (spread / (n * group.len() as f32)).sqrt().max(least);
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
            support,
            open,
            features,
            offset,
            scale,
            clips: ranges,
            legs,
            toes,
            contacts,
            toe_contacts,
            ankle: floor,
            toe: toe_height,
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
        self.clips
            .iter()
            .flat_map(move |(_, r)| r.start..r.end.saturating_sub(tail).max(r.start + 1))
            .filter(|&f| self.open[f])
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

    /// The `k` frames nearest `query`, nearest first.
    pub fn search_best(&self, query: &[f32; FEATURES], k: usize) -> Vec<(usize, f32)> {
        let mut best: Vec<(usize, f32)> = Vec::with_capacity(k + 1);
        let mut worst = f32::INFINITY;
        for frame in self.searchable() {
            let f = &self.features[frame];
            let mut cost = 0.0;
            for d in 0..FEATURES {
                cost += (f[d] - query[d]).powi(2);
                if cost >= worst {
                    break;
                }
            }
            if cost < worst {
                let at = best.partition_point(|b| b.1 <= cost);
                best.insert(at, (frame, cost));
                best.truncate(k);
                if best.len() == k {
                    worst = best[k - 1].1;
                }
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

    /// Each foot's toe, when it has one.
    pub fn toes(&self) -> [Option<usize>; 2] {
        self.toes
    }

    /// How high a frame's ground is over its take's floor.
    pub fn support(&self, frame: usize) -> f32 {
        self.support[frame]
    }

    /// Whether each foot is down at a frame.
    pub fn contacts(&self, frame: usize) -> [bool; 2] {
        self.contacts[frame]
    }
}

/// How far apart two poses are where it shows: each joint's turn between
/// them past `free` radians, squared, summed. Small differences everywhere
/// blend away unseen; one joint half a turn off is a limb swung through.
fn pose_distance(a: &[PoseTransform], b: &[PoseTransform], free: f32) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| {
            let (qa, qb) = (Quat::from_array(a.rotation), Quat::from_array(b.rotation));
            (qa.angle_between(qb) - free).max(0.0).powi(2)
        })
        .sum()
}

/// A vector laid on the ground.
fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
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

/// The character's body against the world: a standing capsule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Capsule {
    pub radius: f32,
    pub height: f32,
    /// The highest edge it steps up or down without a thought, metres.
    pub step: f32,
    /// The steepest slope it walks up, radians.
    pub slope: f32,
}

impl Default for Capsule {
    fn default() -> Self {
        // A little wider than the body: standing at a wall, a head leaning
        // forward stays out of it.
        Self { radius: 0.35, height: 1.75, step: 0.35, slope: 50f32.to_radians() }
    }
}

/// What the character walks among: the solid world, as physics sees it —
/// or a test's flat floor. The animation module knows no physics; the
/// engine hands this in.
pub trait Surroundings {
    /// A standing capsule, feet at `feet`, swept by `by`: sliding along
    /// what it hits, up and down edges no higher than its `step`. Where the
    /// feet end up, and whether they stand on something.
    fn walk(&self, feet: Vec3, by: Vec3, body: &Capsule, dt: f32) -> (Vec3, bool);
    /// The ground under a point: a ray down from `from`, as far as `reach`
    /// — where it hits, and which way the surface faces.
    fn ground(&self, from: Vec3, reach: f32) -> Option<(Vec3, Vec3)>;
    /// How far a ray from `from` along `direction` goes before it hits
    /// something, as far as `reach`: how far a ledge's face is.
    fn ray(&self, _from: Vec3, _direction: Vec3, _reach: f32) -> Option<f32> {
        None
    }
}

/// An endless floor at height zero with nothing on it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Flat;

impl Surroundings for Flat {
    fn walk(&self, feet: Vec3, by: Vec3, _: &Capsule, _: f32) -> (Vec3, bool) {
        let to = feet + by;
        (Vec3::new(to.x, to.y.max(0.0), to.z), to.y <= 0.0)
    }

    fn ground(&self, from: Vec3, reach: f32) -> Option<(Vec3, Vec3)> {
        (from.y >= 0.0 && from.y <= reach).then_some((Vec3::new(from.x, 0.0, from.z), Vec3::Y))
    }
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
    /// How much better another frame must match than playing on, as a
    /// share of playing on's cost: jumps that buy little cost a visible
    /// blend.
    pub switch_margin: f32,
    /// What a jump's pose change costs against the features: radians²
    /// summed over the joints, times this.
    pub pose_weight: f32,
    /// How far a joint may turn in a jump for nothing, radians.
    pub pose_free: f32,
    /// How fast a jump's difference fades.
    pub blend_halflife: f32,
    /// How fast the animation's root is pulled to the spring, and how far
    /// it may stray from it, metres.
    pub hold_halflife: f32,
    pub leash: f32,
    /// Whether a foot that is down stays where it was put, the leg bent to
    /// it, until it lifts.
    pub lock_feet: bool,
    /// How far a locked foot may be left behind before it lets go, metres,
    /// and how fast it creeps after the animation while held, m/s.
    pub lock_reach: f32,
    pub lock_creep: f32,
    /// The body that collides.
    pub body: Capsule,
    /// How fast the shown character follows its capsule up and down a
    /// step, seconds to close half the gap.
    pub climb_halflife: f32,
    /// The highest thing it will get onto or over, metres: the path asked
    /// for is looked along as high as this, and the capsule goes up as far
    /// as the frame playing climbs.
    pub climb: f32,
}

impl Default for Feel {
    fn default() -> Self {
        Self {
            velocity_halflife: 0.27,
            facing_halflife: 0.27,
            search_every: 0.1,
            switch_margin: 0.0,
            pose_weight: 10.0,
            pose_free: 0.5,
            blend_halflife: 0.1,
            hold_halflife: 0.2,
            leash: 0.15,
            lock_feet: true,
            lock_reach: 0.2,
            lock_creep: 0.1,
            body: Capsule::default(),
            climb_halflife: 0.08,
            // LAFAN1's highest ledge is a metre; a little over is still a
            // metre's climb, warped.
            climb: 1.15,
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
    /// How fast the capsule falls, and whether it stands on something.
    fall: f32,
    pub grounded: bool,
    /// Committed to a climb or a drop last step, and what its rise is
    /// multiplied by to fit, and the frame it tops out at.
    climbing_on: bool,
    warp: f32,
    /// How much the walk to a ledge's face is stretched, and up to which
    /// frame.
    stretch: (f32, usize),
    plan_end: usize,
    /// How high the checked climb gets, in the world, and from where.
    plan_top: f32,
    plan_base: f32,
    /// How far the feet were moved off the animation this step, metres —
    /// the most of the two. A foot held far from where the animation has
    /// it bends the leg into a pose nobody captured.
    pub feet_off: f32,
    /// Print what the feet do each step, for a moment being looked into.
    pub debug: bool,
    /// How far the hips are lowered for the feet, smoothed, and which way
    /// each knee bent last step.
    drop: f32,
    bends: [Vec3; 2],
    /// Jumps made, for tests and the debug view, and the last: how far
    /// off its pose was (the largest joint's turn, radians), from which
    /// frame to which.
    pub jumps: usize,
    pub last_jump: Option<(f32, usize, usize)>,
}

#[derive(Debug, Clone, Copy, Default)]
struct FootLock {
    at: Option<Vec3>,
    /// Where the toe is held while the heel lifts.
    toe_at: Option<Vec3>,
    /// How far the ground lifts the foot off the animation's height,
    /// smoothed while it swings: over a step's edge the ground under it
    /// jumps, and the foot should not.
    lift: Option<f32>,
    /// Where it was put last step.
    shown: Option<Vec3>,
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
            fall: 0.0,
            grounded: true,
            climbing_on: false,
            last_jump: None,
            feet_off: 0.0,
            debug: false,
            drop: 0.0,
            bends: [Vec3::ZERO; 2],
            warp: 1.0,
            stretch: (1.0, 0),
            plan_end: 0,
            plan_top: 0.0,
            plan_base: 0.0,
            jumps: 0,
        }
    }

    /// Where the spring will be `seconds` on, and which way it will face:
    /// stopped by what is in the way, so a character walking at a wall
    /// asks for the frames that stop.
    fn spring_ahead(&self, ask: &Ask, goal_turn: Quat, seconds: f32, world: &dyn Surroundings) -> (Vec3, Quat) {
        let (x, v, a) = self.spring;
        let (at, _, _) = chase(x, v, a, flat(ask.velocity), self.feel.velocity_halflife, seconds);
        // Looked along as a climber would: over what is low enough to get
        // onto, stopped by what is not.
        let climber = Capsule { step: self.feel.climb, ..self.feel.body };
        let (at, _) = if seconds > 0.0 { world.walk(x, flat(at - x), &climber, seconds) } else { (at, true) };
        let (q, w) = self.spring_turn;
        let (turn, _) = turn_to(q, w, goal_turn, self.feel.facing_halflife, seconds);
        (at, turn)
    }

    /// What of `ask` the world allows: the stick's velocity swept a third
    /// of a second through the world by a climber's capsule. Along a wall
    /// it slides along it; straight at one it is nothing, the character
    /// standing facing it. What can be got onto is not in the way.
    pub fn possible(&self, ask: &Ask, world: &dyn Surroundings) -> Ask {
        let wanted = flat(ask.velocity);
        if wanted.length() < 0.05 {
            return *ask;
        }
        const AHEAD: f32 = 0.3;
        let climber = Capsule { step: self.feel.climb, ..self.feel.body };
        let from = self.spring.0;
        let (to, _) = world.walk(from, wanted * AHEAD, &climber, AHEAD);
        let can = flat(to - from) / AHEAD;
        if can.length() < wanted.length() * 0.25 {
            Ask { velocity: Vec3::ZERO, facing: Some(ask.facing.unwrap_or(wanted)) }
        } else {
            // As fast as asked, along where it can go.
            Ask { velocity: can.normalize() * wanted.length().min(can.length() * 1.5), facing: ask.facing }
        }
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
    pub fn query(&self, db: &Database, ask: &Ask, world: &dyn Surroundings) -> [f32; FEATURES] {
        let raw_now = db.features[self.frame];
        let mut raw: [f32; FEATURES] = std::array::from_fn(|d| raw_now[d] * db.scale[d] + db.offset[d]);
        let goal = self.goal_turn(ask);
        let (root, turn) = self.root;
        for (k, &ahead) in db.setup.ahead.iter().enumerate() {
            let (at, facing) = self.spring_ahead(ask, goal, ahead as f32 / db.setup.rate, world);
            // The spring's path, carried to start where the character is.
            let p = turn.inverse() * (at - root);
            let d = turn.inverse() * (facing * Vec3::Z);
            raw[15 + k * 2..17 + k * 2].copy_from_slice(&[p.x, p.z]);
            raw[21 + k * 2..23 + k * 2].copy_from_slice(&[d.x, d.z]);
            // How high the ground is there, over where the character stands.
            let climb = self.feel.climb;
            let probe = Vec3::new(at.x, root.y + climb, at.z);
            raw[27 + k] = match world.ground(probe, climb * 2.0) {
                Some((hit, _)) => (hit.y - root.y).clamp(-climb, climb),
                None => -climb,
            };
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
        let hardest = self.turns.iter().map(|t| t.0.length()).fold(0.0f32, f32::max);
        if std::env::var_os("MM_DEBUG").is_some() && hardest > 0.5 {
            let j = self.turns.iter().enumerate().max_by(|a, b| a.1 .0.length().total_cmp(&b.1 .0.length())).unwrap().0;
            let raw = |f: usize| -> [f32; FEATURES] { std::array::from_fn(|d| db.features[f][d] * db.scale[d] + db.offset[d]) };
            let (a, b) = (raw(self.frame), raw(to));
            let hand = |f: &[f32; FEATURES], side: usize| Vec3::new(f[30 + side * 3], f[31 + side * 3], f[32 + side * 3]);
            let wa = db.skeleton.world_matrices(db.pose(self.frame));
            let wb = db.skeleton.world_matrices(db.pose(to));
            let elbow = |w: &[Mat4]| w[j].w_axis.truncate();
            eprintln!("hard {hardest:.2} joint {} hands off {:.2} {:.2} joint pos off {:.2}", db.skeleton.joints[j].name, hand(&a, 0).distance(hand(&b, 0)), hand(&a, 1).distance(hand(&b, 1)), elbow(&wa).distance(elbow(&wb)));
        }
        self.last_jump = Some((hardest, self.frame, to));
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
        self.advance_in(db, ask, dt, &Flat)
    }

    /// [`Matcher::advance`] among `world`: the capsule slides along walls,
    /// steps up stairs and falls off edges, the path asked for stops where
    /// the world stops it, and feet stand on the ground that is there.
    pub fn advance_in(&mut self, db: &Database, ask: &Ask, dt: f32, world: &dyn Surroundings) -> Vec<PoseTransform> {
        let ask = &self.possible(ask, world);
        let goal = self.goal_turn(ask);
        // On the ground, snapping keeps it there (down steps too); pushing
        // down as well drags it along the floor.
        self.fall = if self.grounded { 0.0 } else { self.fall + 9.81 * dt };
        // Committed: the frame playing gets onto something or down off it,
        // higher than a step, soon. The animation leads until it has, the
        // capsule following it up as far as it climbs.
        let over = |(rise, drop): (f32, f32)| rise.max(drop) > self.feel.body.step + 0.05;
        // Held to the climb that was checked: up to the frame it tops out
        // at, and no further along the take without checking again.
        let rising = self.plan_top > self.plan_base;
        // Up: done once at the top. And while it stands on something, not
        // held against a stick that points elsewhere.
        let topped = rising && self.root.0.y >= self.plan_top - 0.05;
        let steady = self.grounded && (self.root.0.y - self.spring.0.y).abs() < 0.1;
        let heading = self.root.1 * Vec3::Z;
        let turned_away = flat(ask.velocity).length() > 0.2 && flat(ask.velocity).normalize().dot(heading) < 0.5;
        let holding = self.climbing_on
            && self.frame <= self.plan_end
            && db.clip_of(self.frame) == db.clip_of(self.plan_end)
            && !topped
            && !(steady && turned_away);
        let wants = holding || over(self.climbing(db, 20));
        let committed = wants
            && (holding || {
                // Only a climb the world has: where the take gets to, and
                // how high, set against the ground there.
                match self.plan(db, ask, world) {
                    Some((warp, end, top, stretch, edge)) => {
                        self.warp = warp;
                        self.stretch = (stretch, edge);
                        self.plan_end = end;
                        self.plan_top = top;
                        self.plan_base = self.spring.0.y;
                        true
                    }
                    None => false,
                }
            });
        self.climbing_on = committed;
        // As high as the checked climb goes over the ground the capsule
        // stands on now.
        let (rise, _) = self.climbing(db, 20);
        let reach = if committed { self.plan_top - self.spring.0.y } else { rise };
        let body = Capsule { step: self.feel.body.step.max((reach + 0.15).min(self.feel.climb)), ..self.feel.body };
        if !committed {
            let (x, v, a) = self.spring;
            let (to, v_to, a_to) = chase(x, flat(v), flat(a), flat(ask.velocity), self.feel.velocity_halflife, dt);
            let want = flat(to - x);
            let (feet, grounded) = world.walk(x, want - Vec3::Y * self.fall * dt, &body, dt);
            self.grounded = grounded;
            let moved = flat(feet - x);
            // Held back by a wall: the spring goes as fast as it went, or it
            // would wind up against it and shoot off when let go. A step up
            // holds it back a little, and that is not a wall.
            self.spring = if moved.length() + 1e-4 < want.length() * 0.5 {
                (feet, moved / dt.max(1e-4), Vec3::ZERO)
            } else {
                (feet, v_to, a_to)
            };
            let (q, w) = self.spring_turn;
            self.spring_turn = turn_to(q, w, goal, self.feel.facing_halflife, dt);
        }

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
        if forced || (!committed && self.since_search >= self.feel.search_every) {
            self.since_search = 0.0;
            let query = self.query(db, ask, world);
            // Played on into a frame that is not walking: away from it.
            let current = if forced || !db.open[self.frame] { f32::INFINITY } else { db.cost(self.frame, &query) };
            // The nearest few by features, then weighed by how far each
            // whole pose is from the one shown: a jump to a pose far off is
            // a blend through poses nobody made — a leg swung up, an arm
            // through the body.
            let shown = self.blended(db);
            let clip = db.clip_of(self.frame);
            let best = db
                .search_best(&query, 16)
                .into_iter()
                // Not to a frame of the same take within half a second either
                // way: back a few frames is a stutter, and round again a loop.
                .filter(|&(f, _)| !(db.clip_of(f) == clip && f.abs_diff(self.frame) <= 15))
                .map(|(f, cost)| (f, cost + self.feel.pose_weight * pose_distance(&shown, db.pose(f), self.feel.pose_free)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((best, score)) = best {
                if score < current * (1.0 - self.feel.switch_margin) {
                    self.jump(db, best);
                }
            }
        }
        for (offset, speed) in &mut self.turns {
            (*offset, *speed) = decay(*offset, *speed, self.feel.blend_halflife, dt);
        }
        self.shift = decay(self.shift.0, self.shift.1, self.feel.blend_halflife, dt);

        let (mut velocity, spin) = db.motion[self.frame];
        if committed {
            // Warped: the take's climb made the height this one is, and
            // the walk to it the length this one is.
            velocity.y *= self.warp;
            if self.frame < self.stretch.1 && db.clip_of(self.frame) == db.clip_of(self.stretch.1) {
                velocity.x *= self.stretch.0;
                velocity.z *= self.stretch.0;
            }
        }
        let (mut at, mut turn) = self.root;
        at += turn * velocity * dt;
        turn = (turn * Quat::from_rotation_y(spin * dt)).normalize();
        let pull = |halflife: f32| 1.0 - (-std::f32::consts::LN_2 * dt / halflife).exp();
        if committed {
            // The animation leads; the capsule follows it through the world,
            // and the character goes no further than its capsule could.
            let x = self.spring.0;
            let (feet, grounded) = world.walk(x, flat(at - x) - Vec3::Y * self.fall * dt, &body, dt);
            self.grounded = grounded;
            let gap = flat(at - feet);
            if gap.length() > self.feel.leash {
                let held = flat(feet) + gap.normalize() * self.feel.leash;
                at = Vec3::new(held.x, at.y, held.z);
            }
            self.spring = (feet, flat(turn * velocity), Vec3::ZERO);
            self.spring_turn = (turn, Vec3::ZERO);
            // Not down before the edge: a drop taken where the take's box
            // ended waits for this one's. Not higher than the checked climb
            // tops out: the take's actor may have climbed higher.
            let rising = self.plan_top > self.plan_base;
            if !rising {
                at.y = at.y.max(feet.y - 0.1);
            }
            at.y = at.y.min(self.plan_top.max(self.plan_base).max(feet.y) + 0.05);
            self.root = (at, turn);
        } else {
            // The animation moves the root; the spring holds it.
            let spring = self.spring.0;
            let mut ground = flat(at).lerp(flat(spring), pull(self.feel.hold_halflife));
            turn = turn.slerp(self.spring_turn.0, pull(self.feel.hold_halflife));
            let gap = ground - flat(spring);
            if gap.length() > self.feel.leash {
                ground = flat(spring) + gap.normalize() * self.feel.leash;
            }
            // Not into a wall: the root is carried out to where it strays
            // as the capsule would be, so a body leaning at a wall stops
            // at it.
            let (held, _) = world.walk(spring, flat(ground - spring), &body, dt);
            ground = flat(held);
            // Up and down a step smoothly: the feet find the steps.
            let height = at.y + (spring.y - at.y) * pull(self.feel.climb_halflife);
            self.root = (Vec3::new(ground.x, height, ground.z), turn);
        }
        let mut pose = self.blended(db);
        if self.feel.lock_feet {
            self.lock_feet(db, &mut pose, dt, world);
        }
        pose
    }

    /// How far the frame playing is about to climb, and to drop, over the
    /// next `frames` of its take. Two thirds of a second is soon enough
    /// that it is the edge in front of it, not one further along the take.
    fn climbing(&self, db: &Database, frames: usize) -> (f32, f32) {
        let clip = &db.clips[db.clip_of(self.frame)].1;
        let here = db.support[self.frame];
        (self.frame..clip.end.min(self.frame + frames))
            .map(|f| db.support[f] - here)
            .fold((0.0f32, 0.0f32), |(up, down), d| (up.max(d), down.max(-d)))
    }

    /// Which feet are held where they were put down (heel or toe), left
    /// then right.
    pub fn feet_held(&self) -> [bool; 2] {
        self.feet.map(|f| f.at.is_some() || f.toe_at.is_some())
    }

    /// Whether the character is getting onto or down off something, the
    /// animation leading: it starts two thirds of a second before the
    /// edge, and holds while the take has more climbing in the next two
    /// seconds — a climb is not given up half way.
    pub fn committed(&self) -> bool {
        self.climbing_on
    }

    /// The climb (or drop) the frame playing is about to make, set against
    /// the world: where the take gets to within two seconds and how much
    /// higher, and how high the ground is there. What to multiply the
    /// take's rise by to make the world's, and the frame it tops out at;
    /// `None` when it is not the same climb to within 30 cm.
    fn plan(&self, db: &Database, ask: &Ask, world: &dyn Surroundings) -> Option<(f32, usize, f32, f32, usize)> {
        let clip = &db.clips[db.clip_of(self.frame)].1;
        let here = db.support[self.frame];
        let end = clip.end.min(self.frame + 60);
        let peak = (self.frame..end).max_by(|&a, &b| {
            (db.support[a] - here).abs().total_cmp(&(db.support[b] - here).abs())
        })?;
        let take = db.support[peak] - here;
        // Where the root will be: the take's steps, turned as it turns.
        let (mut at, mut turn) = (self.root.0, self.root.1);
        for f in self.frame..=peak {
            let (v, spin) = db.motion[f];
            at += turn * flat(v) / db.setup.rate;
            turn *= Quat::from_rotation_y(spin / db.setup.rate);
        }
        // Where the stick points: a climb that goes elsewhere — the take's
        // actor turning off the side of a landing — is not what is asked.
        let wanted = flat(ask.velocity);
        let goes = flat(at - self.root.0);
        if wanted.length() < 0.2 || goes.length() < 0.1 || goes.normalize().dot(wanted.normalize()) < 0.7 {
            return None;
        }
        let climb = self.feel.climb;
        let base = self.spring.0.y;
        let (hit, _) = world.ground(Vec3::new(at.x, base + climb, at.z), climb * 2.0)?;
        let real = hit.y - base;
        if (real - take).abs() >= 0.3 {
            return None;
        }
        // Up a ledge: how far the take walks before it is half way up — to
        // the ledge's face, near enough — against how far this one's face
        // is. The walk there is stretched or squeezed to meet it, so the
        // body climbs at the face and not through it.
        let mut reach = (1.0, self.frame);
        if take > 0.0 {
            let edge = (self.frame..=peak).find(|&f| db.support[f] - here > take / 2.0).unwrap_or(peak);
            let mut walked = Vec3::ZERO;
            let mut turn = self.root.1;
            for f in self.frame..edge {
                let (v, spin) = db.motion[f];
                walked += turn * flat(v) / db.setup.rate;
                turn *= Quat::from_rotation_y(spin / db.setup.rate);
            }
            if walked.length() > 0.1 {
                let from = Vec3::new(self.root.0.x, base + real.min(take) * 0.5, self.root.0.z);
                if let Some(face) = world.ray(from, walked.normalize(), 3.0) {
                    reach = ((face / walked.length()).clamp(0.5, 2.0), edge);
                }
            }
        }
        Some(((real / take).clamp(0.5, 1.5), peak, hit.y, reach.0, reach.1))
    }

    /// Keep each foot that is down where it went down: the leg bent to it,
    /// and when it lifts, the difference fading as a jump's does. A foot
    /// stands on the ground under it, as high over it as the animation has
    /// it over the floor; the hips go down as far as the lower leg needs.
    fn lock_feet(&mut self, db: &Database, pose: &mut [PoseTransform], dt: f32, surroundings: &dyn Surroundings) {
        let placed = Mat4::from_rotation_translation(self.root.1, self.root.0);
        let forward = self.root.1 * Vec3::Z;
        let floor = self.root.0.y;
        let mut world = crate::ik::placed_joints(&db.skeleton, pose, placed);
        let down = db.contacts[self.frame];
        let mut targets = [Vec3::ZERO; 2];
        let mut animated = [Vec3::ZERO; 2];
        // Each leg's length: a foot held further from its hip than this is
        // let go — the body has climbed away from it.
        let leg_length: [f32; 2] = std::array::from_fn(|side| {
            let (foot, knee, hip) = db.legs[side];
            let at = |j: usize| world[j].w_axis.truncate();
            at(hip).distance(at(knee)) + at(knee).distance(at(foot))
        });
        for side in 0..2 {
            let leg = db.legs[side];
            let foot = &mut self.feet[side];
            let a = world[leg.0].w_axis.truncate();
            animated[side] = a;
            let velocity = foot.last.map_or(Vec3::ZERO, |last| (a - last) / dt.max(1e-4));
            foot.last = Some(a);
            foot.fading = decay(foot.fading.0, foot.fading.1, self.feel.blend_halflife, dt);
            // Where the ground under the foot puts it: the higher of what is
            // under the ankle and under the toe.
            let reach = self.feel.body.step * 2.0 + 0.3;
            // A ray that starts inside something found a wall, not ground.
            let under = |p: Vec3| {
                let from = Vec3::new(p.x, floor + reach * 0.5, p.z);
                surroundings.ground(from, reach).map(|(hit, _)| hit.y).filter(|&y| y < from.y - 1e-3)
            };
            let toe = db.toes[side].map(|t| world[t].w_axis.truncate());
            let ground = [Some(a), toe].into_iter().flatten().filter_map(under).reduce(f32::max);
            let free = match ground {
                Some(y) => Vec3::new(a.x, y + (a.y - floor), a.z),
                None => a,
            };
            let shown = free + foot.fading.0;
            // The heel lifts, the toe stays: the foot rolls over its toe,
            // held where the toe is, until the toe lifts too.
            let toe_down = db.toe_contacts[self.frame][side];
            if let (Some(at), Some(toe)) = (foot.at, toe) {
                if !down[side] && toe_down {
                    // The toe held on the ground under it.
                    let mut held = at + (toe - a);
                    if let Some(y) = under(held) {
                        held.y = y + db.toe;
                    }
                    foot.toe_at = Some(held);
                    foot.at = None;
                }
            }
            if let (Some(held), Some(toe)) = (foot.toe_at, toe) {
                let free_toe = free + (toe - a);
                if !toe_down || flat(held - free_toe).length() > self.feel.lock_reach {
                    foot.fading = (held - free_toe, -velocity);
                    foot.toe_at = None;
                }
            }
            foot.at = match foot.at {
                Some(at)
                    if down[side]
                        && flat(at - free).length() <= self.feel.lock_reach
                        && at.distance(world[leg.2].w_axis.truncate()) < leg_length[side] * 1.02 =>
                {
                    // Held, but creeping after the animation: a stance the
                    // animation has moved a foot of — another take's idle, a
                    // shift of weight — is taken up slowly rather than held
                    // off with bent knees.
                    // Only standing about: walking, a foot is down too
                    // briefly to drift, and creeping would be skating.
                    let gap = flat(free - at);
                    let still = flat(self.spring.1).length() < 0.3;
                    let creep = if still { self.feel.lock_creep * dt } else { 0.0 };
                    let mut held = at + if gap.length() > creep { gap.normalize() * creep } else { gap };
                    // Down onto the ground it stands on, quickly but not in a
                    // step.
                    // A planted foot stands on the ground, whatever height
                    // the take had it at over its own floor.
                    let toe_offset = toe.map_or(Vec3::ZERO, |t| t - a);
                    let floor_y = [held, held + toe_offset]
                        .into_iter()
                        .filter_map(under)
                        .reduce(f32::max)
                        .map_or(free.y, |y| y + db.ankle);
                    held.y += (floor_y - held.y) * (1.0 - (-std::f32::consts::LN_2 * dt / 0.025).exp());
                    Some(held)
                }
                Some(at) => {
                    // Let go: from where it was held, fading to the animation.
                    foot.fading = (at - free, -velocity);
                    None
                }
                None if foot.toe_at.is_some() => None,
                None if down[side] => {
                    // Put down where it is shown: the foot does not jump to
                    // be held. It settles onto the ground below.
                    Some(foot.shown.unwrap_or(shown))
                }
                None => None,
            };
            let rolled = foot.toe_at.zip(toe).map(|(held, toe)| held + (a - toe));
            let mut target = foot.at.or(rolled).unwrap_or(free + foot.fading.0);
            // Never lower than standing on what is under it: a foot fading
            // from one step to the next goes over the edge, not through it.
            if let Some(y) = under(target) {
                target.y = target.y.max(y + db.ankle - 0.01);
            }
            if let Some(toe) = toe {
                // The toe as the foot is turned now, clear of the ground
                // under it: a pointed toe goes over the next riser.
                let toe_at = target + (toe - a);
                if let Some(y) = under(toe_at) {
                    target.y = target.y.max(y + db.toe + (a.y - toe.y) - 0.01);
                }
            }
            if foot.at.is_some() || foot.toe_at.is_some() {
                foot.lift = Some(target.y - a.y);
            } else {
                let raw = target.y - a.y;
                let was = foot.lift.unwrap_or(raw);
                // Up quickly — the next step is in the way — down gently.
                let halflife = if raw > was { 0.03 } else { 0.08 };
                let lift = was + (raw - was) * (1.0 - (-std::f32::consts::LN_2 * dt / halflife).exp());
                foot.lift = Some(lift);
                target.y = a.y + lift;
            }
            targets[side] = target;
            foot.shown = Some(target);
        }
        self.feet_off = (0..2).map(|s| targets[s].distance(animated[s])).fold(0.0f32, f32::max);
        // The hips go down for the foot that has further down to go.
        let root = db.skeleton.joints.iter().position(|j| j.parent.is_none()).unwrap_or(0);
        let wanted = (0..2).map(|s| targets[s].y - animated[s].y).fold(0.0f32, f32::min).max(-self.feel.body.step * 1.5);
        // Smoothed: a foot let go must not drop the hips back in a step.
        let drop = self.drop + (wanted - self.drop) * (1.0 - (-std::f32::consts::LN_2 * dt / 0.05).exp());
        self.drop = drop;
        if drop < -1e-4 {
            pose[root].translation[1] += drop;
            world = crate::ik::placed_joints(&db.skeleton, pose, placed);
        }
        for side in 0..2 {
            let leg = db.legs[side];
            let now = world[leg.0].w_axis.truncate();
            // Soft near full stretch (Autodesk's soft IK): the last few
            // per cent of the leg's length are approached ever more slowly,
            // so a knee near straight does not snap between straight and
            // bent as the target moves by a millimetre.
            let hip = world[leg.2].w_axis.truncate();
            let knee = world[leg.1].w_axis.truncate();
            let length = hip.distance(knee) + knee.distance(now);
            let to = targets[side] - hip;
            let soft = length * 0.95;
            let far = to.length();
            if far > soft {
                let give = length - soft;
                let eased = soft + give * (1.0 - (-(far - soft) / give).exp());
                targets[side] = hip + to * (eased / far);
            }
            if targets[side].distance(now) > 1e-4 {
                if self.debug {
                    let f = &self.feet[side];
                    eprintln!(
                        "  side {side} lock {:?} toe {:?} target {:.3?} anim {:.3?} knee {:.3?} bend {:.2?} drop {:.3}",
                        f.at.map(|v| (v * 1000.0).round() / 1000.0), f.toe_at.is_some(), targets[side], animated[side], world[leg.1].w_axis.truncate(), self.bends[side], self.drop
                    );
                }
                let kept = crate::ik::turn_of(world[leg.0]);
                // The way it bent last step leads, then forward: a leg near
                // straight keeps its knee where it was.
                let lean = self.bends[side] * 0.3 + forward * 0.03;
                crate::ik::reach_leg(&db.skeleton, pose, placed, &mut world, leg, targets[side], lean);
                let (h, k, f) = (world[leg.2].w_axis.truncate(), world[leg.1].w_axis.truncate(), world[leg.0].w_axis.truncate());
                let along = (f - h).normalize_or(-Vec3::Y);
                let out = (k - h) - along * (k - h).dot(along);
                if out.length() > 0.01 {
                    self.bends[side] = out.normalize();
                }
                let turned = crate::ik::turn_of(world[leg.0]);
                crate::ik::turn_joint(&db.skeleton, pose, &world, placed, leg.0, kept * turned.inverse());
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
    pub fn wanted(&self, db: &Database, ask: &Ask, world: &dyn Surroundings) -> Vec<(Vec3, Vec3)> {
        let ask = &self.possible(ask, world);
        let goal = self.goal_turn(ask);
        std::iter::once(0)
            .chain(db.setup.ahead)
            .map(|k| {
                let (at, turn) = self.spring_ahead(ask, goal, k as f32 / db.setup.rate, world);
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
        assert_eq!(db.clips.len(), 6, "three and their mirror images");
        let mut matcher = Matcher::new(&db, Vec3::ZERO, Vec3::Z);
        let dt = 1.0 / 60.0;
        let ask = Ask { velocity: Vec3::Z * 1.2, facing: None };
        for _ in 0..120 {
            matcher.advance(&db, &ask, dt);
        }
        let playing = &db.clips[db.clip_of(matcher.frame)].0;
        assert!(playing.starts_with("walk") || playing.starts_with("stop"), "{playing}");
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

#[cfg(test)]
mod mirror_tests {
    use super::*;
    use crate::animation::{Channel, Joint, Path};

    #[test]
    fn a_mirrored_clip_puts_each_joint_where_its_other_side_was_reflected() {
        // Rest turns unlike on the two sides, as a rig from a DCC has them.
        let joint = |name: &str, parent: Option<u16>, at: [f32; 3], turn: Quat| Joint {
            name: name.into(),
            parent,
            inverse_bind: Mat4::IDENTITY.to_cols_array_2d(),
            rest: PoseTransform { translation: at, rotation: turn.to_array(), ..Default::default() },
        };
        // Mirror-image in the world at rest, whatever each joint's own turn:
        // the shin hangs straight down under each thigh.
        let (left_turn, right_turn) = (Quat::from_rotation_z(0.3), Quat::from_rotation_y(-0.7));
        let down = Vec3::new(0.0, -0.45, 0.0);
        let skeleton = Skeleton {
            joints: vec![
                joint("Hips", None, [0.0, 0.95, 0.0], Quat::IDENTITY),
                joint("LeftUpLeg", Some(0), [0.1, 0.0, 0.0], left_turn),
                joint("LeftLeg", Some(1), (left_turn.inverse() * down).to_array(), left_turn.inverse()),
                joint("RightUpLeg", Some(0), [-0.1, 0.0, 0.0], right_turn),
                joint("RightLeg", Some(3), (right_turn.inverse() * down).to_array(), right_turn.inverse()),
            ],
        };
        let times = vec![0.0, 1.0];
        let clip = Clip {
            name: "kick".into(),
            duration: 1.0,
            channels: vec![
                Channel { joint: 0, path: Path::Translation, times: times.clone(), values: vec![0.0, 0.95, 0.0, 0.3, 0.9, 0.5] },
                Channel {
                    joint: 1,
                    path: Path::Rotation,
                    times: times.clone(),
                    values: [Quat::from_rotation_z(0.3), Quat::from_rotation_x(0.9) * Quat::from_rotation_z(0.3)]
                        .iter()
                        .flat_map(|q| q.to_array())
                        .collect(),
                },
                Channel {
                    joint: 0,
                    path: Path::Rotation,
                    times,
                    values: [Quat::IDENTITY, Quat::from_rotation_y(0.5)].iter().flat_map(|q| q.to_array()).collect(),
                },
            ],
        };
        let mirrored = mirror(&skeleton, &clip, 1.0).unwrap();
        let flip = Vec3::new(-1.0, 1.0, 1.0);
        for t in [0.0, 1.0] {
            let a = skeleton.world_matrices(&clip.sample(&skeleton, t, false));
            let b = skeleton.world_matrices(&mirrored.sample(&skeleton, t, false));
            for j in 0..skeleton.len() {
                let other = other_side(&skeleton, j);
                let want = a[other].w_axis.truncate() * flip;
                let got = b[j].w_axis.truncate();
                assert!(got.distance(want) < 1e-4, "{} at {t}: {got} for {want}", skeleton.joints[j].name);
            }
        }
    }
}
