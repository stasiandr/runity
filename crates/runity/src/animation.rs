//! Skeletons, poses and clips.
//!
//! A skeleton is a flat `Vec<Joint>` in which **a parent always appears
//! before its children**. That one rule removes half the problem: world
//! matrices are computed in a single left-to-right pass, because by the time
//! a joint comes up its parent's matrix is already in the array. No
//! recursion, no traversal stack, no `Rc<RefCell<_>>` tree walked every
//! frame.
//!
//! The rule is ours and not glTF's. glTF does not require joints to be
//! topologically ordered, and the indices in a mesh's `JOINTS_0` point into
//! that array, so they cannot simply be rearranged — [`Skeleton::is_sorted`]
//! is checked, and an unsorted skeleton takes a slower path that resolves
//! ancestors first. Slower beats a silently wrong pose.

use glam::{Mat4, Quat, Vec3};
use rkyv::{Archive, Deserialize, Serialize};

/// A joint's place in the hierarchy and its bind pose.
#[derive(Debug, Clone, PartialEq, Archive, Serialize, Deserialize)]
pub struct Joint {
    pub name: String,
    /// Index into the same array, or `None` for a root.
    pub parent: Option<u16>,
    /// Model space to this joint's space, in the pose the mesh was authored
    /// in. It is what turns a joint's animated matrix into the offset a
    /// vertex should actually move by.
    pub inverse_bind: [[f32; 4]; 4],
    /// The joint's own transform in the rest pose, relative to its parent.
    pub rest: PoseTransform,
}

/// One joint's local transform, in the form animation channels produce.
#[derive(Debug, Clone, Copy, PartialEq, Archive, Serialize, Deserialize)]
pub struct PoseTransform {
    pub translation: [f32; 3],
    /// xyzw.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Default for PoseTransform {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }
}

impl PoseTransform {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::from_array(self.scale),
            Quat::from_array(self.rotation),
            Vec3::from_array(self.translation),
        )
    }

    /// Blend toward `other`. Rotation goes the short way round, which is what
    /// keeps a limb from taking the long path between two nearby poses.
    pub fn lerp(&self, other: &PoseTransform, t: f32) -> PoseTransform {
        let a = Quat::from_array(self.rotation);
        let b = Quat::from_array(other.rotation);
        PoseTransform {
            translation: Vec3::from_array(self.translation)
                .lerp(Vec3::from_array(other.translation), t)
                .to_array(),
            rotation: a.slerp(if a.dot(b) < 0.0 { -b } else { b }, t).to_array(),
            scale: Vec3::from_array(self.scale)
                .lerp(Vec3::from_array(other.scale), t)
                .to_array(),
        }
    }
}

/// A flat hierarchy of joints.
#[derive(Debug, Clone, PartialEq, Default, Archive, Serialize, Deserialize)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
}

impl Skeleton {
    pub fn len(&self) -> usize {
        self.joints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }

    /// Whether every parent comes before its children.
    pub fn is_sorted(&self) -> bool {
        self.joints
            .iter()
            .enumerate()
            .all(|(i, joint)| match joint.parent {
                Some(parent) => (parent as usize) < i,
                None => true,
            })
    }

    /// The rest pose, as a starting point for animation.
    pub fn rest_pose(&self) -> Vec<PoseTransform> {
        self.joints.iter().map(|j| j.rest).collect()
    }

    /// Each joint's model-space matrix, given every joint's local transform.
    ///
    /// One pass when sorted. When not, ancestors are resolved first,
    /// iteratively — a deep skeleton must not overflow the stack — and a
    /// cycle, which a corrupt file can describe, stops rather than spins.
    pub fn world_matrices(&self, pose: &[PoseTransform]) -> Vec<Mat4> {
        let mut world = vec![Mat4::IDENTITY; self.joints.len()];
        let local = |i: usize| pose.get(i).copied().unwrap_or(self.joints[i].rest).matrix();

        if self.is_sorted() {
            for (i, joint) in self.joints.iter().enumerate() {
                world[i] = match joint.parent {
                    Some(parent) => world[parent as usize] * local(i),
                    None => local(i),
                };
            }
            return world;
        }

        let mut done = vec![false; self.joints.len()];
        for start in 0..self.joints.len() {
            if done[start] {
                continue;
            }
            // Collect the chain up to a root or an already-resolved joint,
            // then walk back down it.
            let mut chain = Vec::new();
            let mut current = Some(start as u16);
            let mut guard = 0;
            while let Some(index) = current {
                let i = index as usize;
                if done[i] || guard > self.joints.len() {
                    break;
                }
                chain.push(i);
                current = self.joints[i].parent;
                guard += 1;
            }
            for &i in chain.iter().rev() {
                world[i] = match self.joints[i].parent {
                    Some(parent) => world[parent as usize] * local(i),
                    None => local(i),
                };
                done[i] = true;
            }
        }
        world
    }

    /// What the vertex shader wants: for each joint, the matrix that takes a
    /// vertex from its bind position to its posed one.
    pub fn skinning_matrices(&self, pose: &[PoseTransform]) -> Vec<Mat4> {
        self.world_matrices(pose)
            .into_iter()
            .zip(&self.joints)
            .map(|(world, joint)| world * Mat4::from_cols_array_2d(&joint.inverse_bind))
            .collect()
    }
}

/// Which part of a joint a channel animates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Archive, Serialize, Deserialize)]
pub enum Path {
    Translation,
    Rotation,
    Scale,
}

/// One joint's values over time for one path.
///
/// Times are seconds and strictly increasing. Values are flattened: three
/// floats per key for translation and scale, four for rotation.
#[derive(Debug, Clone, PartialEq, Archive, Serialize, Deserialize)]
pub struct Channel {
    pub joint: u16,
    pub path: Path,
    pub times: Vec<f32>,
    pub values: Vec<f32>,
}

/// A named animation.
#[derive(Debug, Clone, PartialEq, Default, Archive, Serialize, Deserialize)]
pub struct Clip {
    pub name: String,
    /// Seconds. Taken from the last key rather than recomputed at load.
    pub duration: f32,
    pub channels: Vec<Channel>,
}

impl Clip {
    /// The pose at a time, starting from the rest pose.
    ///
    /// `looping` wraps; otherwise the clip holds its last pose. Holding, not
    /// snapping back: a one-shot animation that returns to rest on its last
    /// frame is the classic twitch at the end of a death or a door.
    pub fn sample(&self, skeleton: &Skeleton, time: f32, looping: bool) -> Vec<PoseTransform> {
        let mut pose = skeleton.rest_pose();
        let t = if self.duration <= 0.0 {
            0.0
        } else if looping {
            time.rem_euclid(self.duration)
        } else {
            time.clamp(0.0, self.duration)
        };

        for channel in &self.channels {
            let Some(slot) = pose.get_mut(channel.joint as usize) else {
                // A channel naming a joint this skeleton does not have is a
                // mismatched pair of assets. Skipped, so the rest of the
                // animation still plays.
                continue;
            };
            let stride = match channel.path {
                Path::Rotation => 4,
                _ => 3,
            };
            if channel.times.is_empty() || channel.values.len() < stride {
                continue;
            }
            let (i, j, blend) = bracket(&channel.times, t);
            let a = &channel.values[i * stride..(i + 1) * stride];
            let b = channel
                .values
                .get(j * stride..(j + 1) * stride)
                .unwrap_or(a);
            match channel.path {
                Path::Translation => {
                    let v = Vec3::from_slice(a).lerp(Vec3::from_slice(b), blend);
                    slot.translation = v.to_array();
                }
                Path::Scale => {
                    let v = Vec3::from_slice(a).lerp(Vec3::from_slice(b), blend);
                    slot.scale = v.to_array();
                }
                Path::Rotation => {
                    let qa = Quat::from_xyzw(a[0], a[1], a[2], a[3]);
                    let qb = Quat::from_xyzw(b[0], b[1], b[2], b[3]);
                    let qb = if qa.dot(qb) < 0.0 { -qb } else { qb };
                    slot.rotation = qa.slerp(qb, blend).to_array();
                }
            }
        }
        pose
    }
}

/// A joint's name without its rig's prefix: `mixamorig:Hips` and
/// `mixamorig1:Hips` are both `Hips`, the same bone of two downloads.
pub fn bare_joint_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

impl Clip {
    /// This clip, made on `from`, for `to`: joints matched by name (less
    /// the rig's prefix), each rotation carried as its turn away from the
    /// rest pose, so two rigs posed a little differently at rest still
    /// agree. Only the root moves, scaled by how much taller one rig's
    /// root stands than the other's: the other bones keep `to`'s lengths,
    /// or a short character would be stretched to a tall one's arms.
    /// Scale is left to `to`. Joints `to` does not have are dropped.
    ///
    /// What Mixamo's clips need to play on a character that is not the
    /// one each was downloaded with: Unity's Humanoid retargeting, for
    /// rigs that share bone names.
    pub fn retarget(&self, from: &Skeleton, to: &Skeleton) -> Clip {
        let by_name: std::collections::HashMap<&str, usize> = to
            .joints
            .iter()
            .enumerate()
            .map(|(i, j)| (bare_joint_name(&j.name), i))
            .collect();
        let mut channels = Vec::new();
        for channel in &self.channels {
            let Some(source) = from.joints.get(channel.joint as usize) else {
                continue;
            };
            let Some(&target) = by_name.get(bare_joint_name(&source.name)) else {
                continue;
            };
            let aim = &to.joints[target];
            match channel.path {
                Path::Rotation => {
                    let from_rest = Quat::from_array(source.rest.rotation);
                    let to_rest = Quat::from_array(aim.rest.rotation);
                    let values = channel
                        .values
                        .chunks_exact(4)
                        .flat_map(|q| {
                            let turn = from_rest.inverse() * Quat::from_slice(q);
                            (to_rest * turn).normalize().to_array()
                        })
                        .collect();
                    channels.push(Channel {
                        joint: target as u16,
                        path: Path::Rotation,
                        times: channel.times.clone(),
                        values,
                    });
                }
                Path::Translation if aim.parent.is_none() || source.parent.is_none() => {
                    let from_height = Vec3::from_array(source.rest.translation).length();
                    let to_height = Vec3::from_array(aim.rest.translation).length();
                    let ratio = if from_height > 1e-5 {
                        to_height / from_height
                    } else {
                        1.0
                    };
                    channels.push(Channel {
                        joint: target as u16,
                        path: Path::Translation,
                        times: channel.times.clone(),
                        values: channel.values.iter().map(|v| v * ratio).collect(),
                    });
                }
                _ => {}
            }
        }
        Clip {
            name: self.name.clone(),
            duration: self.duration,
            channels,
        }
    }
}

/// The two keys a time falls between, and how far along it is.
///
/// Before the first key or after the last, both indices are the same one and
/// the blend is zero — a clip holds its ends rather than extrapolating past
/// them, which is how a limb ends up somewhere no animator put it.
fn bracket(times: &[f32], t: f32) -> (usize, usize, f32) {
    if t <= times[0] {
        return (0, 0, 0.0);
    }
    let last = times.len() - 1;
    if t >= times[last] {
        return (last, last, 0.0);
    }
    // Linear rather than binary: a channel has tens of keys, and the search
    // is not where the time goes.
    let mut i = 0;
    while i + 1 < times.len() && times[i + 1] <= t {
        i += 1;
    }
    let span = times[i + 1] - times[i];
    let blend = if span > 1e-9 {
        (t - times[i]) / span
    } else {
        0.0
    };
    (i, i + 1, blend)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Root at the origin, then two joints a metre apart up the chain —
    /// a shoulder, an elbow, a hand.
    fn arm() -> Skeleton {
        let joint = |name: &str, parent: Option<u16>, y: f32| Joint {
            name: name.into(),
            parent,
            inverse_bind: Mat4::from_translation(Vec3::new(0.0, -y, 0.0)).to_cols_array_2d(),
            rest: PoseTransform {
                translation: [0.0, if parent.is_none() { 0.0 } else { 1.0 }, 0.0],
                ..Default::default()
            },
        };
        Skeleton {
            joints: vec![
                joint("shoulder", None, 0.0),
                joint("elbow", Some(0), 1.0),
                joint("hand", Some(1), 2.0),
            ],
        }
    }

    #[test]
    fn a_clip_retargets_by_bone_name_as_turns_from_rest() {
        let mut from = arm();
        for j in &mut from.joints {
            j.name = format!("mixamorig:{}", j.name);
        }
        from.joints[0].rest.translation = [0.0, 2.0, 0.0];
        from.joints.push(Joint {
            name: "mixamorig:tail".into(),
            ..from.joints[2].clone()
        });
        let mut to = arm();
        to.joints[0].rest.translation = [0.0, 1.0, 0.0];
        let bent = Quat::from_rotation_z(1.0);
        to.joints[1].rest.rotation = bent.to_array();
        let wave = Quat::from_rotation_x(0.5);
        let key = |joint: u16, path: Path, values: Vec<f32>| Channel {
            joint,
            path,
            times: vec![0.0],
            values,
        };
        let clip = Clip {
            name: "mixamo.com".into(),
            duration: 1.0,
            channels: vec![
                key(0, Path::Translation, vec![0.0, 2.2, 0.4]),
                key(1, Path::Rotation, wave.to_array().to_vec()),
                key(1, Path::Translation, vec![0.0, 5.0, 0.0]),
                key(3, Path::Rotation, wave.to_array().to_vec()),
            ],
        };
        let moved = clip.retarget(&from, &to);
        assert_eq!(
            moved.channels.len(),
            2,
            "the tail and the elbow's stretch go"
        );
        let root = &moved.channels[0];
        assert_eq!((root.joint, root.path), (0, Path::Translation));
        assert!(
            (root.values[1] - 1.1).abs() < 1e-5,
            "half as tall: half the lift"
        );
        let elbow = &moved.channels[1];
        let q = Quat::from_slice(&elbow.values);
        assert!(
            q.angle_between(bent * wave) < 1e-4,
            "its own rest, the same turn"
        );
    }

    #[test]
    fn a_sorted_skeleton_resolves_in_one_pass() {
        let skeleton = arm();
        assert!(skeleton.is_sorted());
        let world = skeleton.world_matrices(&skeleton.rest_pose());
        assert_eq!(world[0].w_axis.y, 0.0);
        assert_eq!(world[1].w_axis.y, 1.0);
        assert_eq!(world[2].w_axis.y, 2.0, "each joint stacks on its parent");
    }

    #[test]
    fn an_unsorted_skeleton_gives_the_same_answer_the_slow_way() {
        // glTF does not require joints to be ordered, and the indices in a
        // mesh point into this array, so they cannot just be rearranged.
        let sorted = arm();
        let mut shuffled = Skeleton {
            joints: vec![
                sorted.joints[2].clone(),
                sorted.joints[1].clone(),
                sorted.joints[0].clone(),
            ],
        };
        // hand(0) <- elbow(1) <- shoulder(2)
        shuffled.joints[0].parent = Some(1);
        shuffled.joints[1].parent = Some(2);
        shuffled.joints[2].parent = None;
        assert!(!shuffled.is_sorted());

        let world = shuffled.world_matrices(&shuffled.rest_pose());
        assert_eq!(world[2].w_axis.y, 0.0, "shoulder");
        assert_eq!(world[1].w_axis.y, 1.0, "elbow");
        assert_eq!(world[0].w_axis.y, 2.0, "hand");
    }

    #[test]
    fn a_cycle_stops_instead_of_spinning() {
        // A corrupt file can describe one, and hanging is the worst
        // possible answer.
        let mut skeleton = arm();
        skeleton.joints[0].parent = Some(2);
        let world = skeleton.world_matrices(&skeleton.rest_pose());
        assert_eq!(world.len(), 3);
    }

    #[test]
    fn turning_a_parent_swings_everything_below_it() {
        let skeleton = arm();
        let mut pose = skeleton.rest_pose();
        // A quarter turn about Z at the shoulder lays the arm along -X.
        pose[0].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
        let world = skeleton.world_matrices(&pose);
        assert!(
            world[2].w_axis.x < -1.9,
            "the hand swung: {}",
            world[2].w_axis.x
        );
        assert!(world[2].w_axis.y.abs() < 0.01);
    }

    #[test]
    fn a_rest_pose_skins_to_no_movement_at_all() {
        // The bind matrices undo the rest pose exactly, so a mesh in its
        // bind position must not shift by a millimetre. If it does, every
        // model twitches the moment a skeleton is attached.
        let skeleton = arm();
        for matrix in skeleton.skinning_matrices(&skeleton.rest_pose()) {
            let drift = (matrix - Mat4::IDENTITY).to_cols_array();
            assert!(
                drift.iter().all(|v| v.abs() < 1e-5),
                "rest pose should skin to identity, got {matrix:?}"
            );
        }
    }

    fn wave() -> Clip {
        Clip {
            name: "wave".into(),
            duration: 2.0,
            channels: vec![Channel {
                joint: 1,
                path: Path::Translation,
                times: vec![0.0, 1.0, 2.0],
                values: vec![0.0, 1.0, 0.0, 0.0, 3.0, 0.0, 0.0, 1.0, 0.0],
            }],
        }
    }

    #[test]
    fn a_clip_interpolates_between_its_keys() {
        let skeleton = arm();
        let clip = wave();
        assert_eq!(clip.sample(&skeleton, 0.0, false)[1].translation[1], 1.0);
        assert_eq!(clip.sample(&skeleton, 1.0, false)[1].translation[1], 3.0);
        assert_eq!(
            clip.sample(&skeleton, 0.5, false)[1].translation[1],
            2.0,
            "halfway between 1 and 3"
        );
    }

    #[test]
    fn a_clip_holds_its_ends_rather_than_extrapolating() {
        // Running past the last key and carrying on puts a limb somewhere no
        // animator ever placed it.
        let skeleton = arm();
        let clip = wave();
        assert_eq!(clip.sample(&skeleton, 9.0, false)[1].translation[1], 1.0);
        assert_eq!(clip.sample(&skeleton, -9.0, false)[1].translation[1], 1.0);
    }

    #[test]
    fn looping_wraps_instead_of_clamping() {
        let skeleton = arm();
        let clip = wave();
        assert_eq!(
            clip.sample(&skeleton, 2.5, true)[1].translation[1],
            clip.sample(&skeleton, 0.5, true)[1].translation[1]
        );
        // And backwards, which a plain modulo gets wrong for negative time.
        assert_eq!(
            clip.sample(&skeleton, -1.5, true)[1].translation[1],
            clip.sample(&skeleton, 0.5, true)[1].translation[1]
        );
    }

    #[test]
    fn a_joint_the_clip_does_not_touch_keeps_its_rest_transform() {
        let skeleton = arm();
        let pose = wave().sample(&skeleton, 0.5, false);
        assert_eq!(pose[0], skeleton.joints[0].rest);
        assert_eq!(pose[2], skeleton.joints[2].rest);
    }

    #[test]
    fn a_channel_naming_a_joint_that_does_not_exist_is_skipped_not_fatal() {
        // Two assets that do not match is a real situation, and losing one
        // channel beats losing the animation.
        let skeleton = arm();
        let mut clip = wave();
        clip.channels.push(Channel {
            joint: 99,
            path: Path::Rotation,
            times: vec![0.0],
            values: vec![0.0, 0.0, 0.0, 1.0],
        });
        assert_eq!(clip.sample(&skeleton, 0.5, false).len(), 3);
    }

    #[test]
    fn rotation_takes_the_short_way_round() {
        // Two rotations 350 degrees apart are 10 degrees apart the other
        // way. Blending the long way spins a limb the wrong direction
        // through the whole arc.
        let a = PoseTransform {
            rotation: Quat::from_rotation_y(0.05).to_array(),
            ..Default::default()
        };
        let b = PoseTransform {
            rotation: (-Quat::from_rotation_y(-0.05)).to_array(),
            ..Default::default()
        };
        let middle = Quat::from_array(a.lerp(&b, 0.5).rotation);
        let angle = middle.to_euler(glam::EulerRot::YXZ).0;
        assert!(angle.abs() < 0.02, "expected near zero, got {angle}");
    }
}
