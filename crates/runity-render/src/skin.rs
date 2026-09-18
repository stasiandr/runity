//! Skeletons, poses and skinning.
//!
//! A villager is a mesh that has to bend. The usual way is a skeleton — a
//! small tree of joints — plus, for each vertex, which joints move it and how
//! much. Animation then moves the joints, and the mesh follows.
//!
//! Skinning happens on the CPU here, which for this renderer is not a
//! compromise: everything else does too. It does mean the cost is per vertex
//! per frame, so the rest of the design pushes back on that — a pose is
//! sampled once and reused, and the skinned result is written into a buffer
//! the caller keeps rather than allocated anew.

use runity_math::{Mat4, Quat, Vec3};

use crate::mesh::Mesh;

/// How many joints may influence one vertex.
///
/// Four is what glTF stores and what every tool exports; a fifth influence is
/// almost always worth less than the arithmetic it costs.
pub const MAX_INFLUENCES: usize = 4;

/// One bone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joint {
    /// Index of the parent joint, or `None` for the root.
    pub parent: Option<u16>,
    /// The joint's transform in the rest pose, relative to its parent.
    pub rest: Transform,
    /// Takes a vertex from model space into this joint's space.
    pub inverse_bind: Mat4,
}

/// Position, rotation and scale, as a skeleton stores them.
///
/// The same shape as the engine's `Transform`, kept here so that the renderer
/// does not depend on the engine crate — animation is data, and data should
/// not need the main loop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// Offset from the parent.
    pub position: Vec3,
    /// Orientation.
    pub rotation: Quat,
    /// Scale along each axis.
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    /// The matrix this transform describes.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_translation(self.position)
            * self.rotation.to_mat4()
            * Mat4::from_scale(self.scale)
    }

    /// Interpolate, rotating the short way round.
    pub fn lerp(&self, other: &Transform, t: f32) -> Transform {
        let t = t.clamp(0.0, 1.0);
        Transform {
            position: self.position + (other.position - self.position) * t,
            rotation: self.rotation.slerp(other.rotation, t),
            scale: self.scale + (other.scale - self.scale) * t,
        }
    }
}

/// A tree of joints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skeleton {
    /// The joints, parents before children.
    pub joints: Vec<Joint>,
}

impl Skeleton {
    /// How many joints there are.
    pub fn len(&self) -> usize {
        self.joints.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }

    /// The pose the skeleton was built in.
    pub fn rest_pose(&self) -> Pose {
        Pose {
            locals: self.joints.iter().map(|joint| joint.rest).collect(),
        }
    }

    /// Whether every parent comes before its child.
    ///
    /// Assumed by [`Skeleton::world_matrices`], which computes in one pass
    /// rather than recursing — so it is worth being able to check.
    pub fn is_sorted(&self) -> bool {
        self.joints
            .iter()
            .enumerate()
            .all(|(index, joint)| match joint.parent {
                Some(parent) => (parent as usize) < index,
                None => true,
            })
    }

    /// Each joint's transform in model space.
    ///
    /// The usual case is one pass: parents come before their children, so a
    /// parent's world matrix is already computed by the time the child needs
    /// it. A skeleton listed in some other order still comes out right — glTF
    /// does not require the joints array to be topologically sorted, and a
    /// silently wrong pose is a worse answer than a slower one.
    pub fn world_matrices(&self, pose: &Pose, out: &mut Vec<Mat4>) {
        if self.is_sorted() {
            out.clear();
            for (index, joint) in self.joints.iter().enumerate() {
                let local = self.local_matrix(pose, index, joint);
                let world = match joint.parent {
                    Some(parent) => {
                        out.get(parent as usize).copied().unwrap_or(Mat4::IDENTITY) * local
                    }
                    None => local,
                };
                out.push(world);
            }
            return;
        }
        self.world_matrices_unsorted(pose, out);
    }

    /// A joint's local transform: the pose's, or its rest pose if the pose is
    /// short.
    fn local_matrix(&self, pose: &Pose, index: usize, joint: &Joint) -> Mat4 {
        pose.locals
            .get(index)
            .copied()
            .unwrap_or(joint.rest)
            .matrix()
    }

    /// The same result without assuming an order: each joint's ancestors are
    /// resolved first, iteratively so that a deep skeleton cannot overflow the
    /// stack. A parent cycle — which a malformed file can describe — stops at
    /// the joint that closes it rather than looping forever.
    fn world_matrices_unsorted(&self, pose: &Pose, out: &mut Vec<Mat4>) {
        out.clear();
        out.resize(self.joints.len(), Mat4::IDENTITY);
        let mut done = vec![false; self.joints.len()];
        let mut chain: Vec<usize> = Vec::new();

        for start in 0..self.joints.len() {
            if done[start] {
                continue;
            }
            // Walk up to the first ancestor that is already resolved (or is a
            // root), collecting the way back down.
            chain.clear();
            let mut cursor = Some(start);
            while let Some(index) = cursor {
                if done[index] || chain.contains(&index) {
                    break;
                }
                chain.push(index);
                cursor = self.joints[index]
                    .parent
                    .map(|parent| parent as usize)
                    .filter(|parent| *parent < self.joints.len());
            }
            // Then unwind: every parent in the chain is resolved before the
            // child that named it.
            for &index in chain.iter().rev() {
                let joint = &self.joints[index];
                let local = self.local_matrix(pose, index, joint);
                out[index] = match joint.parent {
                    Some(parent)
                        if (parent as usize) < self.joints.len() && done[parent as usize] =>
                    {
                        out[parent as usize] * local
                    }
                    _ => local,
                };
                done[index] = true;
            }
        }
    }

    /// The matrices skinning actually uses: world times inverse bind.
    pub fn skinning_matrices(&self, pose: &Pose, out: &mut Vec<Mat4>) {
        self.world_matrices(pose, out);
        for (matrix, joint) in out.iter_mut().zip(&self.joints) {
            *matrix = *matrix * joint.inverse_bind;
        }
    }
}

/// Where every joint is, relative to its parent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pose {
    /// One local transform per joint.
    pub locals: Vec<Transform>,
}

impl Pose {
    /// A pose of `count` joints, all at rest.
    pub fn identity(count: usize) -> Self {
        Self {
            locals: vec![Transform::default(); count],
        }
    }

    /// Mix two poses, joint by joint.
    ///
    /// The whole of blending: a character halfway between walking and standing
    /// is every joint halfway between the two, which is why rotations are
    /// stored as quaternions and interpolated as such — matrices blended
    /// element by element produce shear.
    pub fn blend(&self, other: &Pose, weight: f32) -> Pose {
        let weight = weight.clamp(0.0, 1.0);
        let locals = self
            .locals
            .iter()
            .zip(&other.locals)
            .map(|(a, b)| a.lerp(b, weight))
            .collect();
        Pose { locals }
    }

    /// Mix in another pose for a subset of joints only.
    ///
    /// For a character who waves while walking: the arm comes from one clip,
    /// everything else from the other.
    pub fn blend_masked(&self, other: &Pose, weight: f32, mask: &[bool]) -> Pose {
        let weight = weight.clamp(0.0, 1.0);
        let locals = self
            .locals
            .iter()
            .zip(&other.locals)
            .enumerate()
            .map(|(index, (a, b))| {
                if mask.get(index).copied().unwrap_or(false) {
                    a.lerp(b, weight)
                } else {
                    *a
                }
            })
            .collect();
        Pose { locals }
    }
}

/// Which joints move a vertex, and by how much.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Influence {
    /// Joint indices.
    pub joints: [u16; MAX_INFLUENCES],
    /// Their weights, which should sum to one.
    pub weights: [f32; MAX_INFLUENCES],
}

impl Default for Influence {
    fn default() -> Self {
        // Rigid: entirely controlled by the first joint.
        Self {
            joints: [0; MAX_INFLUENCES],
            weights: [1.0, 0.0, 0.0, 0.0],
        }
    }
}

impl Influence {
    /// Scale the weights so they sum to one.
    ///
    /// Exporters round weights, and weights that sum to 0.98 shrink the mesh
    /// by two percent — which looks like a modelling mistake and is not one.
    pub fn normalized(mut self) -> Self {
        let total: f32 = self.weights.iter().sum();
        if total > 1e-6 {
            for weight in &mut self.weights {
                *weight /= total;
            }
        } else {
            self.weights = [1.0, 0.0, 0.0, 0.0];
        }
        self
    }
}

/// A mesh that bends: the rest-pose geometry plus its influences.
#[derive(Clone, Debug, Default)]
pub struct SkinnedMesh {
    /// The mesh as modelled, in the bind pose.
    pub rest: Mesh,
    /// One influence per vertex.
    pub influences: Vec<Influence>,
}

impl SkinnedMesh {
    /// Deform the rest mesh into `out`, reusing its allocation.
    ///
    /// Writing into a buffer the caller keeps is the difference between
    /// skinning a crowd and allocating a mesh per character per frame.
    pub fn apply(&self, matrices: &[Mat4], out: &mut Mesh) {
        out.vertices.clear();
        out.vertices.extend_from_slice(&self.rest.vertices);
        out.indices.clear();
        out.indices.extend_from_slice(&self.rest.indices);

        for (index, vertex) in out.vertices.iter_mut().enumerate() {
            let Some(influence) = self.influences.get(index) else {
                continue;
            };
            let mut position = Vec3::ZERO;
            let mut normal = Vec3::ZERO;
            let mut total = 0.0;
            for slot in 0..MAX_INFLUENCES {
                let weight = influence.weights[slot];
                if weight <= 0.0 {
                    continue;
                }
                let Some(matrix) = matrices.get(influence.joints[slot] as usize) else {
                    continue;
                };
                let moved = matrix.transform_point(self.rest.vertices[index].position);
                position += Vec3::new(moved.x, moved.y, moved.z) * weight;
                normal += matrix.transform_vector(self.rest.vertices[index].normal) * weight;
                total += weight;
            }
            if total > 1e-6 {
                vertex.position = position / total;
                // Renormalizing matters: blended normals are shorter than one,
                // and a short normal is a surface that reads as too dark.
                vertex.normal = normal.normalized();
            }
        }
    }

    /// How many vertices bend.
    pub fn len(&self) -> usize {
        self.rest.vertices.len()
    }

    /// Whether there is nothing to skin.
    pub fn is_empty(&self) -> bool {
        self.rest.vertices.is_empty()
    }
}

/// How a value changes between keyframes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    /// Straight lines between keys.
    Linear,
    /// Hold each key until the next.
    Step,
}

/// What one channel of an animation drives.
#[derive(Clone, Debug, PartialEq)]
pub enum Track {
    /// Movement.
    Translation(Vec<(f32, Vec3)>),
    /// Rotation, interpolated the short way round.
    Rotation(Vec<(f32, Quat)>),
    /// Scale.
    Scale(Vec<(f32, Vec3)>),
}

impl Track {
    /// When the last keyframe is.
    pub fn duration(&self) -> f32 {
        let last = match self {
            Track::Translation(keys) | Track::Scale(keys) => keys.last().map(|(time, _)| *time),
            Track::Rotation(keys) => keys.last().map(|(time, _)| *time),
        };
        last.unwrap_or(0.0)
    }

    /// How many keyframes it holds.
    pub fn len(&self) -> usize {
        match self {
            Track::Translation(keys) | Track::Scale(keys) => keys.len(),
            Track::Rotation(keys) => keys.len(),
        }
    }

    /// Whether it has no keyframes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One joint's worth of animation.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    /// Which joint it drives.
    pub joint: u16,
    /// What it drives.
    pub track: Track,
    /// How it moves between keys.
    pub interpolation: Interpolation,
}

/// A clip: everything that moves, and for how long.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animation {
    /// Name from the file.
    pub name: String,
    /// The channels, in file order.
    pub channels: Vec<Channel>,
    /// How long the clip lasts, in seconds.
    pub duration: f32,
}

impl Animation {
    /// Work out the duration from the channels.
    pub fn recompute_duration(&mut self) {
        self.duration = self.channels.iter().fold(0.0f32, |longest, channel| {
            longest.max(channel.track.duration())
        });
    }

    /// Write the pose at `time` into `pose`.
    ///
    /// Joints the clip says nothing about are left alone, which is what makes
    /// a clip that only moves an arm usable on top of one that walks.
    pub fn sample(&self, time: f32, pose: &mut Pose) {
        for channel in &self.channels {
            let Some(local) = pose.locals.get_mut(channel.joint as usize) else {
                continue;
            };
            match &channel.track {
                Track::Translation(keys) => {
                    if let Some(value) = sample_vec(keys, time, channel.interpolation) {
                        local.position = value;
                    }
                }
                Track::Scale(keys) => {
                    if let Some(value) = sample_vec(keys, time, channel.interpolation) {
                        local.scale = value;
                    }
                }
                Track::Rotation(keys) => {
                    if let Some(value) = sample_quat(keys, time, channel.interpolation) {
                        local.rotation = value;
                    }
                }
            }
        }
    }

    /// The pose at `time`, starting from a skeleton's rest pose.
    pub fn pose_at(&self, skeleton: &Skeleton, time: f32) -> Pose {
        let mut pose = skeleton.rest_pose();
        self.sample(time, &mut pose);
        pose
    }

    /// Wrap a time into the clip's length, for looping playback.
    pub fn wrap(&self, time: f32) -> f32 {
        if self.duration <= 0.0 {
            return 0.0;
        }
        let wrapped = time % self.duration;
        if wrapped < 0.0 {
            wrapped + self.duration
        } else {
            wrapped
        }
    }
}

/// Find the pair of keyframes around a time, and how far between them it is.
fn bracket<T>(keys: &[(f32, T)], time: f32) -> Option<(usize, usize, f32)> {
    if keys.is_empty() {
        return None;
    }
    if time <= keys[0].0 {
        return Some((0, 0, 0.0));
    }
    let last = keys.len() - 1;
    if time >= keys[last].0 {
        return Some((last, last, 0.0));
    }
    // Binary search rather than a scan: a clip can have hundreds of keys and
    // is sampled once per character per frame.
    let mut low = 0;
    let mut high = last;
    while high - low > 1 {
        let middle = (low + high) / 2;
        if keys[middle].0 <= time {
            low = middle;
        } else {
            high = middle;
        }
    }
    let span = keys[high].0 - keys[low].0;
    let fraction = if span > 1e-9 {
        (time - keys[low].0) / span
    } else {
        0.0
    };
    Some((low, high, fraction))
}

fn sample_vec(keys: &[(f32, Vec3)], time: f32, interpolation: Interpolation) -> Option<Vec3> {
    let (low, high, fraction) = bracket(keys, time)?;
    Some(match interpolation {
        Interpolation::Step => keys[low].1,
        Interpolation::Linear => keys[low].1 + (keys[high].1 - keys[low].1) * fraction,
    })
}

fn sample_quat(keys: &[(f32, Quat)], time: f32, interpolation: Interpolation) -> Option<Quat> {
    let (low, high, fraction) = bracket(keys, time)?;
    Some(match interpolation {
        Interpolation::Step => keys[low].1,
        Interpolation::Linear => keys[low].1.slerp(keys[high].1, fraction),
    })
}
