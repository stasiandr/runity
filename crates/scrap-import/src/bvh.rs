//! BioVision Hierarchy: the plain-text mocap format most public capture
//! comes in (LAFAN1, CMU, Bandai). A skeleton written as nested `JOINT`
//! blocks with offsets from the parent, then one line of channel values
//! per frame.
//!
//! What it becomes is what a glTF skin becomes: a [`Skeleton`] whose
//! rest pose is every rotation at zero, and a [`Clip`] with a key on
//! every frame. The unit is the file's — capture is usually centimetres,
//! so `scale` of 0.01 makes it metres. `End Site` blocks carry no
//! channels and are left out.

use anyhow::{bail, Context, Result};
use glam::{Mat4, Quat, Vec3};
use scrap::animation::{Channel, Clip, Joint, Path, PoseTransform, Skeleton};

/// One of the six values a joint's frame can hold.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Value {
    Position(usize),
    Rotation(usize),
}

/// A skeleton and its one clip, read from BVH text. `scale` turns the
/// file's unit into metres; `name` names the clip.
pub fn read(text: &str, name: &str, scale: f32) -> Result<(Skeleton, Clip)> {
    let mut words = text.split_whitespace().peekable();
    let mut joints: Vec<Joint> = Vec::new();
    // Each joint's channels, in the order the frame lines give them.
    let mut channels: Vec<Vec<Value>> = Vec::new();
    // The joint each open `{` belongs to; `None` for an end site's.
    let mut open: Vec<Option<u16>> = Vec::new();
    let expect = |word: Option<&str>, what: &str| -> Result<()> {
        match word {
            Some(w) if w == what => Ok(()),
            other => bail!("expected {what}, found {other:?}"),
        }
    };
    expect(words.next(), "HIERARCHY")?;
    loop {
        let Some(word) = words.next() else { bail!("the file ends inside HIERARCHY") };
        match word {
            "ROOT" | "JOINT" => {
                let joint_name = words.next().context("a joint without a name")?;
                expect(words.next(), "{")?;
                let parent = open.iter().rev().find_map(|j| *j);
                if word == "JOINT" && parent.is_none() {
                    bail!("joint {joint_name} outside a root");
                }
                let index = u16::try_from(joints.len()).context("too many joints")?;
                joints.push(Joint {
                    name: joint_name.to_string(),
                    parent: if word == "ROOT" { None } else { parent },
                    inverse_bind: Mat4::IDENTITY.to_cols_array_2d(),
                    rest: PoseTransform::default(),
                });
                channels.push(Vec::new());
                open.push(Some(index));
            }
            "End" => {
                words.next();
                expect(words.next(), "{")?;
                open.push(None);
            }
            "OFFSET" => {
                let mut offset = [0.0f32; 3];
                for v in &mut offset {
                    *v = words.next().context("a short OFFSET")?.parse::<f32>()? * scale;
                }
                if let Some(Some(joint)) = open.last() {
                    joints[*joint as usize].rest.translation = offset;
                }
            }
            "CHANNELS" => {
                let count: usize = words.next().context("CHANNELS without a count")?.parse()?;
                let Some(Some(joint)) = open.last().copied() else { bail!("CHANNELS outside a joint") };
                for _ in 0..count {
                    let channel = words.next().context("a short CHANNELS")?;
                    let axis = match channel.as_bytes().first() {
                        Some(b'X') => 0,
                        Some(b'Y') => 1,
                        Some(b'Z') => 2,
                        _ => bail!("channel {channel}"),
                    };
                    channels[joint as usize].push(if channel.ends_with("position") {
                        Value::Position(axis)
                    } else if channel.ends_with("rotation") {
                        Value::Rotation(axis)
                    } else {
                        bail!("channel {channel}")
                    });
                }
            }
            "}" => {
                open.pop().context("an unmatched }")?;
            }
            "MOTION" => break,
            other => bail!("unexpected {other} in HIERARCHY"),
        }
    }
    if joints.is_empty() {
        bail!("no joints");
    }
    expect(words.next(), "Frames:")?;
    let frames: usize = words.next().context("Frames: without a count")?.parse()?;
    expect(words.next(), "Frame")?;
    expect(words.next(), "Time:")?;
    let step: f32 = words.next().context("Frame Time: without a value")?.parse()?;

    let joints_len = joints.len();
    let mut translations: Vec<Vec<f32>> = vec![Vec::new(); joints_len];
    let mut rotations: Vec<Vec<f32>> = vec![Vec::new(); joints_len];
    let mut values = [0.0f32; 6];
    for frame in 0..frames {
        for (joint, list) in channels.iter().enumerate() {
            let mut position = Vec3::from_array(joints[joint].rest.translation);
            let mut rotation = Quat::IDENTITY;
            for (k, value) in list.iter().enumerate() {
                let word = words.next().with_context(|| format!("frame {frame} is short"))?;
                values[k] = word.parse().with_context(|| format!("frame {frame}: {word}"))?;
                match *value {
                    Value::Position(axis) => position[axis] = values[k] * scale,
                    // Intrinsic, in the order written: `Z Y X` is Rz·Ry·Rx.
                    Value::Rotation(axis) => {
                        let radians = values[k].to_radians();
                        rotation *= match axis {
                            0 => Quat::from_rotation_x(radians),
                            1 => Quat::from_rotation_y(radians),
                            _ => Quat::from_rotation_z(radians),
                        };
                    }
                }
            }
            if list.iter().any(|v| matches!(v, Value::Position(_))) {
                translations[joint].extend(position.to_array());
            }
            if list.iter().any(|v| matches!(v, Value::Rotation(_))) {
                rotations[joint].extend(rotation.normalize().to_array());
            }
        }
    }

    let mut skeleton = Skeleton { joints };
    let world = skeleton.world_matrices(&skeleton.rest_pose());
    for (joint, world) in skeleton.joints.iter_mut().zip(world) {
        joint.inverse_bind = world.inverse().to_cols_array_2d();
    }
    let times: Vec<f32> = (0..frames).map(|i| i as f32 * step).collect();
    let mut clip_channels = Vec::new();
    for joint in 0..joints_len {
        for (path, values) in [(Path::Translation, &mut translations[joint]), (Path::Rotation, &mut rotations[joint])] {
            if !values.is_empty() {
                clip_channels.push(Channel {
                    joint: joint as u16,
                    path,
                    times: times.clone(),
                    values: std::mem::take(values),
                });
            }
        }
    }
    let clip = Clip {
        name: name.to_string(),
        duration: times.last().copied().unwrap_or(0.0),
        channels: clip_channels,
    };
    Ok((skeleton, clip))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_BONES: &str = "HIERARCHY
ROOT Hips
{
	OFFSET 0 100 0
	CHANNELS 6 Xposition Yposition Zposition Zrotation Yrotation Xrotation
	JOINT Knee
	{
		OFFSET 0 -50 0
		CHANNELS 3 Zrotation Yrotation Xrotation
		End Site
		{
			OFFSET 0 -50 0
		}
	}
}
MOTION
Frames: 2
Frame Time: 0.5
0 100 0 0 0 0 0 0 0
10 100 0 90 0 0 0 0 90
";

    #[test]
    fn a_bvh_reads_as_a_skeleton_and_a_clip_in_metres() {
        let (skeleton, clip) = read(TWO_BONES, "two", 0.01).unwrap();
        assert_eq!(skeleton.joints.len(), 2, "the end site is not a joint");
        assert_eq!(skeleton.joints[1].parent, Some(0));
        assert_eq!(skeleton.joints[1].rest.translation, [0.0, -0.5, 0.0]);
        assert_eq!(clip.duration, 0.5);
        let pose = clip.sample(&skeleton, 0.5, false);
        assert!((Vec3::from_array(pose[0].translation) - Vec3::new(0.1, 1.0, 0.0)).length() < 1e-5);
        let hips = Quat::from_array(pose[0].rotation);
        assert!((hips * Vec3::X - Vec3::Y).length() < 1e-5, "Zrotation 90 turns x to y");
        let knee = Quat::from_array(pose[1].rotation);
        assert!((knee * Vec3::Y - Vec3::Z).length() < 1e-5, "Xrotation 90 turns y to z");
        // The rest pose's world matrices undo its inverse binds.
        let skin = skeleton.skinning_matrices(&skeleton.rest_pose());
        assert!(skin.iter().all(|m| m.abs_diff_eq(Mat4::IDENTITY, 1e-5)));
    }
}
