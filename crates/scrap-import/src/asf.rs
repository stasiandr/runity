//! Acclaim skeleton and motion: an `.asf` file names the bones — each a
//! direction and a length, turned in its own axes — and an `.amc` file
//! gives, frame by frame, each bone's turn about those axes. The format of
//! the CMU motion capture database.
//!
//! What it becomes is what BVH becomes: a [`Skeleton`] and a [`Clip`] with
//! a key on every frame. A bone's joint sits where the bone starts — the
//! end of its parent — and turns the bone; the rest pose is every bone at
//! its written direction (CMU's is a T-pose). `scale` turns the file's
//! lengths into metres: CMU's `length 0.45` in inches is `0.0254 / 0.45`.
//! AMC keeps no frame rate: `rate` gives it (CMU's is 120, a few 60).

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use glam::{Quat, Vec3};
use scrap::animation::{Channel, Clip, Joint, Path, PoseTransform, Skeleton};

/// Which of a turn's three angles a value is, or a place's three.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Dof {
    Turn(usize),
    Move(usize),
}

struct Bone {
    name: String,
    /// Where the bone points and how long it is, in the world at rest.
    direction: Vec3,
    length: f32,
    /// The bone's own axes, in the world at rest.
    axes: Quat,
    dofs: Vec<Dof>,
    parent: Option<usize>,
}

/// A turn by angles in degrees about x, y and z, the first letter of
/// `order` turned first.
fn turn(angles: Vec3, order: &str) -> Quat {
    let mut q = Quat::IDENTITY;
    for axis in order.chars() {
        q = match axis.to_ascii_uppercase() {
            'X' => Quat::from_rotation_x(angles.x.to_radians()),
            'Y' => Quat::from_rotation_y(angles.y.to_radians()),
            'Z' => Quat::from_rotation_z(angles.z.to_radians()),
            _ => Quat::IDENTITY,
        } * q;
    }
    q
}

fn dof(word: &str) -> Option<Dof> {
    let axis = match word.to_ascii_lowercase().chars().last()? {
        'x' => 0,
        'y' => 1,
        'z' => 2,
        _ => return None,
    };
    match word.to_ascii_lowercase().chars().next()? {
        'r' => Some(Dof::Turn(axis)),
        't' => Some(Dof::Move(axis)),
        _ => None,
    }
}

fn three(words: &[&str]) -> Result<Vec3> {
    let v: Vec<f32> = words.iter().take(3).map(|w| w.parse()).collect::<Result<_, _>>()?;
    if v.len() < 3 {
        bail!("three numbers wanted: {words:?}");
    }
    Ok(Vec3::new(v[0], v[1], v[2]))
}

/// A skeleton and its one clip, read from an `.asf` and an `.amc`.
pub fn read(asf: &str, amc: &str, name: &str, scale: f32, rate: f32) -> Result<(Skeleton, Clip)> {
    // The root: where and how it starts, its values' order, its axes.
    let mut root = Bone {
        name: "root".into(),
        direction: Vec3::ZERO,
        length: 0.0,
        axes: Quat::IDENTITY,
        dofs: Vec::new(),
        parent: None,
    };
    let mut root_at = Vec3::ZERO;
    let mut root_axes_order = "XYZ".to_string();
    let mut root_orientation = Vec3::ZERO;
    let mut bones: Vec<Bone> = Vec::new();
    let mut section = "";
    let mut open: Option<Bone> = None;
    let mut children: Vec<(String, Vec<String>)> = Vec::new();
    for line in asf.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix(':') {
            section = match rest.split_whitespace().next().unwrap_or("") {
                "root" => "root",
                "bonedata" => "bonedata",
                "hierarchy" => "hierarchy",
                _ => "",
            };
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        match section {
            "root" => match words[0] {
                "order" => root.dofs = words[1..].iter().filter_map(|w| dof(w)).collect(),
                "axis" => root_axes_order = words.get(1).unwrap_or(&"XYZ").to_string(),
                "position" => root_at = three(&words[1..])?,
                "orientation" => root_orientation = three(&words[1..])?,
                _ => {}
            },
            "bonedata" => match words[0] {
                "begin" => {
                    open = Some(Bone {
                        name: String::new(),
                        direction: Vec3::Y,
                        length: 0.0,
                        axes: Quat::IDENTITY,
                        dofs: Vec::new(),
                        parent: None,
                    })
                }
                "end" => bones.extend(open.take()),
                word => {
                    let Some(bone) = open.as_mut() else { continue };
                    match word {
                        "name" => bone.name = words.get(1).context("a bone without a name")?.to_string(),
                        "direction" => bone.direction = three(&words[1..])?.normalize_or_zero(),
                        "length" => bone.length = words.get(1).context("length without a value")?.parse()?,
                        "axis" => bone.axes = turn(three(&words[1..])?, words.get(4).unwrap_or(&"XYZ")),
                        "dof" => bone.dofs = words[1..].iter().filter_map(|w| dof(w)).collect(),
                        _ => {}
                    }
                }
            },
            "hierarchy" => {
                if words[0] != "begin" && words[0] != "end" {
                    children.push((words[0].to_string(), words[1..].iter().map(|w| w.to_string()).collect()));
                }
            }
            _ => {}
        }
    }
    root.axes = turn(root_orientation, &root_axes_order);
    // The root first, then the bones in the order the hierarchy reaches
    // them, so a parent always comes before its children.
    let mut by_name: HashMap<String, Bone> = bones.into_iter().map(|b| (b.name.clone(), b)).collect();
    let mut ordered = vec![root];
    let mut index: HashMap<String, usize> = HashMap::from([("root".to_string(), 0)]);
    let mut queue = std::collections::VecDeque::from(["root".to_string()]);
    while let Some(parent) = queue.pop_front() {
        let Some((_, kids)) = children.iter().find(|(p, _)| *p == parent) else { continue };
        for kid in kids {
            let mut bone = by_name.remove(kid).with_context(|| format!("{kid} in the hierarchy has no bone"))?;
            bone.parent = Some(index[&parent]);
            index.insert(kid.clone(), ordered.len());
            ordered.push(bone);
            queue.push_back(kid.clone());
        }
    }

    // A joint's turn under its parent is the parent's axes undone, its
    // own axes, then the frame's turn about them.
    let local_axes = |b: &Bone| match b.parent {
        Some(p) => ordered[p].axes.inverse() * b.axes,
        None => b.axes,
    };
    let joints: Vec<Joint> = ordered
        .iter()
        .map(|b| {
            let at = match b.parent {
                Some(p) => ordered[p].axes.inverse() * ordered[p].direction * ordered[p].length * scale,
                None => root_at * scale,
            };
            Joint {
                name: b.name.clone(),
                parent: b.parent.map(|p| p as u16),
                inverse_bind: glam::Mat4::IDENTITY.to_cols_array_2d(),
                rest: PoseTransform { translation: at.to_array(), rotation: local_axes(b).to_array(), scale: [1.0; 3] },
            }
        })
        .collect();

    // The frames: a number, then a line a bone.
    let mut frames: Vec<HashMap<&str, Vec<f32>>> = Vec::new();
    for line in amc.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(':') {
            continue;
        }
        let mut words = line.split_whitespace();
        let first = words.next().unwrap_or("");
        if first.parse::<usize>().is_ok() && words.clone().next().is_none() {
            frames.push(HashMap::new());
            continue;
        }
        let Some(frame) = frames.last_mut() else { continue };
        let values: Vec<f32> = words.map(|w| w.parse()).collect::<Result<_, _>>().with_context(|| format!("{line}"))?;
        frame.insert(first, values);
    }
    if frames.is_empty() {
        bail!("no frames");
    }

    let mut translations: Vec<f32> = Vec::with_capacity(frames.len() * 3);
    let mut rotations: Vec<Vec<f32>> = vec![Vec::with_capacity(frames.len() * 4); ordered.len()];
    for frame in &frames {
        for (j, bone) in ordered.iter().enumerate() {
            if bone.dofs.is_empty() {
                continue;
            }
            let values = frame.get(bone.name.as_str()).map(Vec::as_slice).unwrap_or(&[]);
            let (mut angles, mut moved) = (Vec3::ZERO, Vec3::ZERO);
            let mut order = String::new();
            for (k, d) in bone.dofs.iter().enumerate() {
                let v = values.get(k).copied().unwrap_or(0.0);
                match *d {
                    Dof::Turn(axis) => {
                        angles[axis] = v;
                        order.push(['X', 'Y', 'Z'][axis]);
                    }
                    Dof::Move(axis) => moved[axis] = v,
                }
            }
            // A bone's turns go x, then y, then z — as its axes do — and
            // the root's in the order it names.
            let spin = if j == 0 { turn(angles, &order) } else { turn(angles, "XYZ") };
            rotations[j].extend((local_axes(bone) * spin).normalize().to_array());
            if j == 0 {
                translations.extend(((root_at + moved) * scale).to_array());
            }
        }
    }

    let mut skeleton = Skeleton { joints };
    let world = skeleton.world_matrices(&skeleton.rest_pose());
    for (joint, world) in skeleton.joints.iter_mut().zip(world) {
        joint.inverse_bind = world.inverse().to_cols_array_2d();
    }
    let times: Vec<f32> = (0..frames.len()).map(|i| i as f32 / rate).collect();
    let mut channels = Vec::new();
    if !translations.is_empty() {
        channels.push(Channel { joint: 0, path: Path::Translation, times: times.clone(), values: translations });
    }
    for (j, values) in rotations.into_iter().enumerate() {
        if !values.is_empty() {
            channels.push(Channel { joint: j as u16, path: Path::Rotation, times: times.clone(), values });
        }
    }
    let clip = Clip { name: name.to_string(), duration: times.last().copied().unwrap_or(0.0), channels };
    Ok((skeleton, clip))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A leg: the root, a thigh pointing down, a shin pointing down whose
    /// axes are turned a quarter about y.
    const LEG_ASF: &str = ":units
  length 1
:root
   order TX TY TZ RX RY RZ
   axis XYZ
   position 0 0 0
   orientation 0 0 0
:bonedata
  begin
     id 1
     name thigh
     direction 0 -1 0
     length 4
     axis 0 0 0 XYZ
    dof rx ry rz
  end
  begin
     id 2
     name shin
     direction 0 -1 0
     length 5
     axis 0 90 0 XYZ
    dof rx
  end
:hierarchy
  begin
    root thigh
    thigh shin
  end
";

    const LEG_AMC: &str = ":FULLY-SPECIFIED
:DEGREES
1
root 0 9 0 0 0 0
thigh 0 0 0
shin 0
2
root 1 9 0 0 90 0
thigh 90 0 0
shin 90
";

    #[test]
    fn an_asf_and_amc_read_as_a_skeleton_and_a_clip() {
        let (skeleton, clip) = read(LEG_ASF, LEG_AMC, "leg", 0.1, 2.0).unwrap();
        let names: Vec<&str> = skeleton.joints.iter().map(|j| j.name.as_str()).collect();
        assert_eq!(names, ["root", "thigh", "shin"]);
        assert_eq!(clip.duration, 0.5);
        let at = |t: f32| {
            let world = skeleton.world_matrices(&clip.sample(&skeleton, t, false));
            world.iter().map(|m| m.w_axis.truncate()).collect::<Vec<_>>()
        };
        // At rest the knee is 0.4 m under the hips, standing 0.9 m up.
        let first = at(0.0);
        assert!((first[0] - Vec3::new(0.0, 0.9, 0.0)).length() < 1e-5);
        assert!((first[2] - Vec3::new(0.0, 0.5, 0.0)).length() < 1e-5, "{:?}", first[2]);
        // Then the thigh turns a quarter about x, which takes down to -z:
        // the knee comes up level with the hips, behind them; the root
        // turned a quarter about y carries -z round to -x.
        let second = at(0.5);
        let knee = second[2] - second[0];
        assert!((knee - Vec3::new(-0.4, 0.0, 0.0)).length() < 1e-4, "{knee:?}");
        assert!((second[0] - Vec3::new(0.1, 0.9, 0.0)).length() < 1e-5);
        // The rest pose's world matrices undo its inverse binds.
        let skin = skeleton.skinning_matrices(&skeleton.rest_pose());
        assert!(skin.iter().all(|m| m.abs_diff_eq(glam::Mat4::IDENTITY, 1e-5)));
    }
}
