//! A humanoid as eleven capsules on joints, and poses of it: what a
//! ragdoll is built from and what drives it.
//!
//! The body stands in its rest pose — arms down, facing +z — `height`
//! tall with its feet at the origin. A [`Pose`] is each part's turn from
//! its rest relative to its parent, so the same pose fits any body; the
//! pelvis's turn is the whole body's lean. [`gait`] makes the poses of a
//! walk or a run at a speed and a phase — procedural animation, a cycle
//! from sines, not a clip.

use glam::{Quat, Vec3};

/// One part of the body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Part {
    pub name: &'static str,
    /// Its parent part; `None` for the pelvis.
    pub parent: Option<usize>,
    /// Where it joins its parent, at rest, feet at the origin, 1.8 m tall.
    pub joint: Vec3,
    /// The capsule's two ends and its radius, at rest.
    pub from: Vec3,
    pub to: Vec3,
    pub radius: f32,
    /// It bends one way only, about x (a knee, an elbow).
    pub hinge: bool,
    /// Its share of the body's mass.
    pub mass: f32,
}

/// The parts, parents before children.
pub const PARTS: [Part; 11] = [
    Part { name: "pelvis", parent: None, joint: Vec3::new(0.0, 0.97, 0.0), from: Vec3::new(0.0, 0.9, 0.0), to: Vec3::new(0.0, 1.04, 0.0), radius: 0.13, hinge: false, mass: 0.15 },
    Part { name: "chest", parent: Some(0), joint: Vec3::new(0.0, 1.06, 0.0), from: Vec3::new(0.0, 1.12, 0.0), to: Vec3::new(0.0, 1.38, 0.0), radius: 0.15, hinge: false, mass: 0.28 },
    Part { name: "head", parent: Some(1), joint: Vec3::new(0.0, 1.52, 0.0), from: Vec3::new(0.0, 1.6, 0.0), to: Vec3::new(0.0, 1.7, 0.0), radius: 0.11, hinge: false, mass: 0.08 },
    Part { name: "upper arm left", parent: Some(1), joint: Vec3::new(-0.21, 1.43, 0.0), from: Vec3::new(-0.23, 1.38, 0.0), to: Vec3::new(-0.24, 1.17, 0.0), radius: 0.05, hinge: false, mass: 0.03 },
    Part { name: "forearm left", parent: Some(3), joint: Vec3::new(-0.24, 1.13, 0.0), from: Vec3::new(-0.24, 1.09, 0.0), to: Vec3::new(-0.24, 0.85, 0.0), radius: 0.045, hinge: true, mass: 0.02 },
    Part { name: "upper arm right", parent: Some(1), joint: Vec3::new(0.21, 1.43, 0.0), from: Vec3::new(0.23, 1.38, 0.0), to: Vec3::new(0.24, 1.17, 0.0), radius: 0.05, hinge: false, mass: 0.03 },
    Part { name: "forearm right", parent: Some(5), joint: Vec3::new(0.24, 1.13, 0.0), from: Vec3::new(0.24, 1.09, 0.0), to: Vec3::new(0.24, 0.85, 0.0), radius: 0.045, hinge: true, mass: 0.02 },
    Part { name: "thigh left", parent: Some(0), joint: Vec3::new(-0.1, 0.9, 0.0), from: Vec3::new(-0.1, 0.84, 0.0), to: Vec3::new(-0.1, 0.55, 0.0), radius: 0.07, hinge: false, mass: 0.1 },
    Part { name: "shin left", parent: Some(7), joint: Vec3::new(-0.1, 0.5, 0.0), from: Vec3::new(-0.1, 0.44, 0.0), to: Vec3::new(-0.1, 0.1, 0.0), radius: 0.055, hinge: true, mass: 0.07 },
    Part { name: "thigh right", parent: Some(0), joint: Vec3::new(0.1, 0.9, 0.0), from: Vec3::new(0.1, 0.84, 0.0), to: Vec3::new(0.1, 0.55, 0.0), radius: 0.07, hinge: false, mass: 0.1 },
    Part { name: "shin right", parent: Some(9), joint: Vec3::new(0.1, 0.5, 0.0), from: Vec3::new(0.1, 0.44, 0.0), to: Vec3::new(0.1, 0.1, 0.0), radius: 0.055, hinge: true, mass: 0.07 },
];

pub const PELVIS: usize = 0;
pub const CHEST: usize = 1;
pub const UPPER_ARM: [usize; 2] = [3, 5];
pub const FOREARM: [usize; 2] = [4, 6];
pub const THIGH: [usize; 2] = [7, 9];
pub const SHIN: [usize; 2] = [8, 10];

/// Each part's turn from its rest, relative to its parent's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose(pub [Quat; 11]);

impl Default for Pose {
    fn default() -> Self {
        Pose([Quat::IDENTITY; 11])
    }
}

impl Pose {
    /// Each part's turn in the world and where its joint is, the pelvis's
    /// joint at `root` turned by `facing`, the body `scale` times 1.8 m.
    pub fn world(&self, root: Vec3, facing: Quat, scale: f32) -> [(Quat, Vec3); 11] {
        let mut out = [(Quat::IDENTITY, Vec3::ZERO); 11];
        for (i, part) in PARTS.iter().enumerate() {
            match part.parent {
                None => out[i] = (facing * self.0[i], root),
                Some(p) => {
                    let (turn, at) = out[p];
                    let offset = (part.joint - PARTS[p].joint) * scale;
                    out[i] = (turn * self.0[i], at + turn * offset);
                }
            }
        }
        out
    }

    /// Where a point of part `i` at rest is, the body posed so.
    pub fn point(&self, world: &[(Quat, Vec3); 11], i: usize, rest: Vec3, scale: f32) -> Vec3 {
        let (turn, at) = world[i];
        at + turn * ((rest - PARTS[i].joint) * scale)
    }

    /// Each pair blended by `t`.
    pub fn blend(&self, other: &Pose, t: f32) -> Pose {
        let mut out = *self;
        for i in 0..11 {
            let mut b = other.0[i];
            if self.0[i].dot(b) < 0.0 {
                b = -b;
            }
            out.0[i] = self.0[i].slerp(b, t);
        }
        out
    }
}

/// A walk or a run at `speed` metres a second, `phase` 0 to 1 through its
/// stride — standing still at 0. The legs swing from the hip and the
/// knee folds on the way through; the arms swing against them; the body
/// leans into it and bobs. Procedural: sines, as a walk cycle is.
pub fn gait(speed: f32, phase: f32) -> Pose {
    let s = speed.clamp(0.0, 4.0);
    let t = phase * std::f32::consts::TAU;
    let swing = (0.35 + 0.12 * s).min(0.8) * (s / (s + 0.3));
    let mut pose = Pose::default();
    for (side, offset) in [(0usize, 0.0f32), (1, std::f32::consts::PI)] {
        let a = (t + offset).sin() * swing;
        // Knee folds most as the leg comes through (swinging forward).
        let fold = ((t + offset + 1.2).sin().max(0.0) * (0.5 + 0.35 * s)).min(1.6) * (s / (s + 0.3));
        // Forward is −x-turn: the foot toward +z.
        pose.0[THIGH[side]] = Quat::from_rotation_x(-a);
        pose.0[SHIN[side]] = Quat::from_rotation_x(fold);
        // Arms against the legs, elbows a little bent, more when running.
        pose.0[UPPER_ARM[side]] = Quat::from_rotation_x(a * 0.8);
        pose.0[FOREARM[side]] = Quat::from_rotation_x(-(0.15 + 0.3 * s / (s + 1.0)));
    }
    // Lean into the stride and twist the chest against the hips a little.
    pose.0[PELVIS] = Quat::from_rotation_x(0.05 * s);
    pose.0[CHEST] = Quat::from_rotation_y(t.sin() * 0.08 * (s / (s + 0.3)));
    pose
}

/// How high the pelvis bobs over a stride, metres, and how long a stride
/// is at a speed: what a gait's root does.
pub fn stride(speed: f32) -> f32 {
    // Longer strides at speed, as people take them.
    0.9 + 0.35 * speed.clamp(0.0, 4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rest_pose_puts_each_joint_where_the_parts_say() {
        let pose = Pose::default();
        let world = pose.world(PARTS[0].joint, Quat::IDENTITY, 1.0);
        for (i, part) in PARTS.iter().enumerate() {
            assert!(world[i].1.distance(part.joint) < 1e-5, "{}", part.name);
        }
        // Turned half round, the left hand is on the right.
        let turned = pose.world(PARTS[0].joint, Quat::from_rotation_y(std::f32::consts::PI), 1.0);
        let hand = pose.point(&turned, FOREARM[0], PARTS[FOREARM[0]].to, 1.0);
        assert!(hand.x > 0.2, "{hand}");
    }

    #[test]
    fn walking_swings_one_foot_forward_as_the_other_goes_back() {
        let pose = gait(1.4, 0.25);
        let world = pose.world(PARTS[0].joint, Quat::IDENTITY, 1.0);
        let foot = |side: usize| pose.point(&world, SHIN[side], PARTS[SHIN[side]].to, 1.0);
        assert!(foot(0).z > 0.1 && foot(1).z < -0.1, "{} {}", foot(0), foot(1));
        // Standing still: both feet under the hips.
        let still = gait(0.0, 0.3);
        let world = still.world(PARTS[0].joint, Quat::IDENTITY, 1.0);
        let foot = |side: usize| still.point(&world, SHIN[side], PARTS[SHIN[side]].to, 1.0);
        assert!(foot(0).z.abs() < 1e-4 && foot(1).z.abs() < 1e-4);
    }
}
