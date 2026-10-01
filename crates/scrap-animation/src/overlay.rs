//! Editing a model's clip without leaving its model: a `*.clip.ron` with a
//! `base` is the changes to that clip, and only them.
//!
//! Unity's clips inside an FBX are read-only, and to change one you
//! duplicate it: the copy forgets the file it came from, its import
//! settings, and every fix made to the source after. Here the edit names
//! the clip it is over and keys only the tracks it changed:
//!
//! ```ron
//! (
//!     base: (model: "character-male-a", clip: "walk"),
//!     length: 1.0,
//!     tracks: [
//!         (path: "arm-left", what: TurnZ, keys: [(0.0, 12.0, Auto), (0.5, 40.0, Auto), (1.0, 12.0, Auto)]),
//!     ],
//! )
//! ```
//!
//! A track's `path` is a bone's name, and it replaces the base's curve of
//! that one property — `TurnZ` of `arm-left` — and nothing else: the other
//! bones, and the same bone's other turns, are still the model's, so the
//! model imported again brings its changes in under the edit. A file named
//! as its base clip (`walk.clip.ron` over `walk`) is the model's `walk`
//! from then on, in the game and in the editor; one named otherwise
//! (`walk-tired.clip.ron`) is one more clip of the model.
//!
//! A bone's turns are three angles in degrees, in the order a line's
//! `rotation` has (Y, then X, then Z), not the quaternions the model keeps:
//! what a person reads, and the curves Unity's and Blender's editors show.

#[allow(unused_imports)]
use crate::prelude::*;
use glam::{EulerRot, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::animation::{Channel, Clip, Path as Part, Skeleton};
use crate::motion::{Motion, Motions, Property, Track};
use crate::AssetLink;

/// The model's clip an edit is made over — or, with no `clip`, the
/// model's rest pose: a clip made in the editor for that rig, from
/// nothing (`base: (model: "character-male-a")`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Base {
    pub model: AssetLink,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub clip: String,
}

/// `base: (model: …, clip: …)`, not `base: Some((…))`: an option written
/// as its value, absent when it is none.
pub(crate) mod bare {
    use super::Base;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(base: &Option<Base>, s: S) -> Result<S::Ok, S::Error> {
        match base {
            Some(b) => b.serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Base>, D::Error> {
        Base::deserialize(d).map(Some)
    }
}

/// The three properties of a part of a bone, x, y and z.
pub fn axes(part: Part) -> [Property; 3] {
    match part {
        Part::Translation => [Property::X, Property::Y, Property::Z],
        Part::Rotation => [Property::TurnX, Property::TurnY, Property::TurnZ],
        Part::Scale => [Property::ScaleX, Property::ScaleY, Property::ScaleZ],
    }
}

/// Which part of a bone a property is of, and which axis.
pub fn part_of(what: Property) -> Option<(Part, usize)> {
    [Part::Translation, Part::Rotation, Part::Scale]
        .into_iter()
        .find_map(|p| axes(p).iter().position(|a| *a == what).map(|i| (p, i)))
}

/// A rotation as a line's three angles, in degrees.
pub fn degrees(q: Quat) -> Vec3 {
    let (y, x, z) = q.to_euler(EulerRot::YXZ);
    Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI)
}

/// One channel as three tracks, one an axis, named by its bone. Turns are
/// unrolled: each angle the nearest to the one before by whole turns, so
/// a curve crossing 180° goes on to 190° instead of jumping to -170°.
pub fn channel_tracks(channel: &Channel, joint: &str) -> [Track; 3] {
    let stride = if channel.path == Part::Rotation { 4 } else { 3 };
    let mut columns: [Vec<crate::keys::Key>; 3] = Default::default();
    let mut last: Option<Vec3> = None;
    for (i, &t) in channel.times.iter().enumerate() {
        let Some(v) = channel.values.get(i * stride..(i + 1) * stride) else { break };
        let mut v = match channel.path {
            Part::Rotation => degrees(Quat::from_slice(v)),
            _ => Vec3::from_slice(v),
        };
        if let (Part::Rotation, Some(before)) = (channel.path, last) {
            for a in 0..3 {
                v[a] = before[a] + wrap(v[a] - before[a]);
            }
        }
        last = Some(v);
        for a in 0..3 {
            columns[a].push(crate::keys::Key::new(t, v[a]));
        }
    }
    let axes = axes(channel.path);
    columns.map(|keys| keys).into_iter().enumerate().map(|(a, keys)| Track {
        path: joint.to_string(),
        what: axes[a],
        keys,
        ease: Default::default(),
    }).collect::<Vec<_>>().try_into().expect("three axes")
}

/// An angle's difference to within half a turn.
fn wrap(d: f32) -> f32 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

/// Every track of a model's clip: each bone's channels, an axis a track,
/// in the skeleton's order — what the Animation window lists.
pub fn tracks_of(clip: &Clip, skeleton: &Skeleton) -> Vec<Track> {
    let mut channels: Vec<&Channel> = clip.channels.iter().collect();
    channels.sort_by_key(|c| (c.joint, c.path as u8));
    channels
        .into_iter()
        .filter_map(|c| {
            let joint = skeleton.joints.get(c.joint as usize)?;
            Some(channel_tracks(c, &joint.name))
        })
        .flatten()
        .collect()
}

/// The bone a track's path names: by its name, or its name less the rig's
/// prefix (`Hips` for `mixamorig:Hips`).
pub fn joint_named(skeleton: &Skeleton, name: &str) -> Option<usize> {
    let joints = &skeleton.joints;
    joints.iter().position(|j| j.name == name).or_else(|| {
        let bare = crate::animation::bare_joint_name(name);
        joints.iter().position(|j| crate::animation::bare_joint_name(&j.name) == bare)
    })
}

/// The clip an edit makes of its base: the base's channels, with every
/// part of a bone the edit keys made again from the edit's tracks and the
/// base's for the axes the edit leaves alone. Tracks of bones the skeleton
/// does not have are said in `problems` and skipped.
pub fn bake(name: &str, motion: &Motion, base: &Clip, skeleton: &Skeleton, problems: &mut Vec<String>) -> Clip {
    let mut channels = base.channels.clone();
    let mut parts: Vec<(usize, Part)> = Vec::new();
    for track in &motion.tracks {
        let Some((part, _)) = part_of(track.what) else {
            problems.push(format!("clip `{name}`: `{:?}` is not a bone's", track.what));
            continue;
        };
        let Some(joint) = joint_named(skeleton, &track.path) else {
            let nearest = scrap_core::spelling::closest(&track.path, skeleton.joints.iter().map(|j| j.name.as_str()));
            problems.push(match nearest {
                Some(n) => format!("clip `{name}`: no bone `{}` (did you mean `{n}`?)", track.path),
                None => format!("clip `{name}`: no bone `{}`", track.path),
            });
            continue;
        };
        if !parts.contains(&(joint, part)) {
            parts.push((joint, part));
        }
    }
    for (joint, part) in parts {
        let at = channels.iter().position(|c| c.joint as usize == joint && c.path == part);
        let joint_name = &skeleton.joints[joint].name;
        let from_base: Option<[Track; 3]> = at.map(|i| channel_tracks(&channels[i], joint_name));
        let mine = |what: Property| {
            motion
                .tracks
                .iter()
                .find(|t| t.what == what && joint_named(skeleton, &t.path) == Some(joint))
        };
        let tracks: Vec<Track> = axes(part)
            .iter()
            .enumerate()
            .filter_map(|(a, what)| mine(*what).cloned().or_else(|| from_base.as_ref().map(|b| b[a].clone())))
            .collect();
        let rest = &skeleton.joints[joint].rest;
        let rest = crate::Transform {
            position: Vec3::from_array(rest.translation),
            rotation_deg: degrees(Quat::from_array(rest.rotation)),
            scale: Vec3::from_array(rest.scale),
        };
        let refs: Vec<&Track> = tracks.iter().collect();
        let made = crate::motion::channels_for(joint as u16, &refs, &rest)
            .into_iter()
            .find(|c| c.path == part);
        match (at, made) {
            (Some(i), Some(c)) => channels[i] = c,
            (None, Some(c)) => channels.push(c),
            _ => {}
        }
    }
    Clip {
        name: name.to_string(),
        duration: if motion.length > 0.0 { motion.length } else { base.duration },
        channels,
    }
}

/// Whether an edit's base is this model.
fn over(base: &Base, model: &AssetLink) -> bool {
    match (base.model.id, model.id) {
        (Some(a), Some(b)) => a == b,
        _ => base.model.name == model.name,
    }
}

/// A model's clips with the project's edits of them made: an edit named as
/// its base replaces it, any other is one more clip. What is wrong with an
/// edit — a base clip the model does not have, a bone it does not — is
/// said, and the rest of it still made.
pub fn apply(motions: &Motions, model: &AssetLink, skeleton: &Skeleton, clips: &mut Vec<Clip>) -> Vec<String> {
    let mut problems = Vec::new();
    let mut edits: Vec<(&String, &Motion, &Base)> = motions
        .clips
        .iter()
        .filter_map(|(name, m)| Some((name, m, m.base.as_ref()?)))
        .filter(|(_, _, b)| over(b, model))
        .collect();
    edits.sort_by_key(|(name, _, _)| *name);
    for (name, motion, base) in edits {
        if base.clip.is_empty() {
            let rest = Clip { name: name.clone(), duration: motion.length, channels: Vec::new() };
            let made = bake(name, motion, &rest, skeleton, &mut problems);
            match clips.iter().position(|c| c.name == *name) {
                Some(i) => clips[i] = made,
                None => clips.push(made),
            }
            continue;
        }
        let Some(from) = clips.iter().position(|c| c.name == base.clip) else {
            let nearest = scrap_core::spelling::closest(&base.clip, clips.iter().map(|c| c.name.as_str()));
            problems.push(match nearest {
                Some(n) => format!("clip `{name}`: `{}` has no clip `{}` (did you mean `{n}`?)", model.name, base.clip),
                None => format!("clip `{name}`: `{}` has no clip `{}`", model.name, base.clip),
            });
            continue;
        };
        let made = bake(name, motion, &clips[from], skeleton, &mut problems);
        match clips.iter().position(|c| c.name == *name) {
            Some(i) => clips[i] = made,
            None => clips.push(made),
        }
    }
    problems
}

/// A clip's file as a person would write it: the base on its line, a
/// track a line with its keys on it.
pub fn text(motion: &Motion) -> String {
    let mut out = String::from("(\n");
    if let Some(base) = &motion.base {
        out.push_str(&format!("    base: {},\n", base_text(base)));
    }
    out.push_str(&format!("    length: {:?},\n    tracks: [\n", motion.length));
    for t in &motion.tracks {
        out.push_str(&format!("        {},\n", track_text(t)));
    }
    out.push_str("    ],\n)\n");
    out
}

/// The base as a person writes it, `(model: "body", clip: "walk")`.
fn base_text(base: &Base) -> String {
    let model = ron::to_string(&base.model).unwrap_or_default();
    if base.clip.is_empty() {
        format!("(model: {model})")
    } else {
        format!("(model: {model}, clip: {:?})", base.clip)
    }
}

/// One track as its line.
pub fn track_text(t: &Track) -> String {
    let keys: Vec<String> = t.keys.iter().map(key_text).collect();
    let path = if t.path.is_empty() { String::new() } else { format!("path: {:?}, ", t.path) };
    let ease = if t.ease == crate::ease::Ease::Linear { String::new() } else { format!(", ease: {:?}", t.ease) };
    format!("({path}what: {:?}, keys: [{}]{ease})", t.what, keys.join(", "))
}

fn key_text(k: &crate::keys::Key) -> String {
    let n = |v: f32| {
        // Three places after the point at most: a millimetre, a
        // thousandth of a degree, and a key that reads.
        let r = (v * 1000.0).round() / 1000.0;
        format!("{:?}", if r == 0.0 { 0.0 } else { r })
    };
    match k.tangent {
        crate::keys::Tangent::Eased => format!("({}, {})", n(k.time), n(k.value)),
        t => format!("({}, {}, {})", n(k.time), n(k.value), ron::to_string(&t).unwrap_or_default()),
    }
}

/// The file `old` with `motion` written into it, changed only where it
/// changed: a track replaced on its own line, the others as they were,
/// with their comments. A file that does not read is written whole.
pub fn write(old: &str, motion: &Motion) -> String {
    patched(old, motion).unwrap_or_else(|| text(motion))
}

fn patched(old: &str, motion: &Motion) -> Option<String> {
    use scrap_core::ron_edit::{self as patch, Change};
    let was: Motion = ron::from_str(old).ok()?;
    let mut text = old.to_string();
    if was.base != motion.base {
        let base = motion.base.as_ref().map(base_text);
        text = patch::set_field(&text, "base", base.as_deref())?;
    }
    if was.length != motion.length {
        text = patch::set_field(&text, "length", Some(&format!("{:?}", motion.length)))?;
    }
    let open = patch::value_start(&text, "tracks")?;
    let key = |t: &Track| (t.path.clone(), t.what);
    let mut changes = Vec::new();
    for (i, t) in was.tracks.iter().enumerate() {
        match motion.tracks.iter().find(|m| key(m) == key(t)) {
            None => changes.push(Change::Remove(i)),
            Some(m) if m != t => changes.push(Change::Replace(i, track_text(m))),
            _ => {}
        }
    }
    for m in &motion.tracks {
        if !was.tracks.iter().any(|t| key(t) == key(m)) {
            changes.push(Change::Append(track_text(m)));
        }
    }
    let out = patch::apply(&text, open, &changes)?;
    // What was written must read back as what was meant.
    let back: Motion = ron::from_str(&out).ok()?;
    (same(&back, motion)).then_some(out)
}

/// Equal as files are: tracks in any order, numbers as written.
fn same(a: &Motion, b: &Motion) -> bool {
    let canon = |m: &Motion| ron::from_str::<Motion>(&text(m)).ok().map(|mut m| {
        m.tracks.sort_by(|x, y| (&x.path, x.what as u8).cmp(&(&y.path, y.what as u8)));
        m
    });
    a.base == b.base && a.length == b.length && canon(a) == canon(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Joint, PoseTransform};

    fn skeleton() -> Skeleton {
        let joint = |name: &str, parent| Joint {
            name: name.into(),
            parent,
            inverse_bind: glam::Mat4::IDENTITY.to_cols_array_2d(),
            rest: PoseTransform::default(),
        };
        Skeleton { joints: vec![joint("hips", None), joint("arm", Some(0))] }
    }

    /// The arm turning 0 → 90° about z over a second, the hips rising.
    fn wave() -> Clip {
        let q = |deg: f32| Quat::from_rotation_z(deg.to_radians()).to_array();
        Clip {
            name: "wave".into(),
            duration: 1.0,
            channels: vec![
                Channel { joint: 1, path: Part::Rotation, times: vec![0.0, 1.0], values: [q(0.0), q(90.0)].concat() },
                Channel { joint: 0, path: Part::Translation, times: vec![0.0, 1.0], values: vec![0.0, 1.0, 0.0, 0.0, 2.0, 0.0] },
            ],
        }
    }

    fn edit(text: &str) -> Motions {
        let mut motions = Motions::default();
        motions.clips.insert("wave".into(), ron::from_str(text).unwrap());
        motions
    }

    #[test]
    fn a_clip_reads_as_tracks_of_bones_in_degrees() {
        let tracks = tracks_of(&wave(), &skeleton());
        let turn = tracks.iter().find(|t| t.path == "arm" && t.what == Property::TurnZ).unwrap();
        assert!((turn.keys[1].value - 90.0).abs() < 1e-3, "{:?}", turn.keys);
        let up = tracks.iter().find(|t| t.path == "hips" && t.what == Property::Y).unwrap();
        assert_eq!(up.keys[1], (1.0, 2.0));
        assert_eq!(tracks.len(), 6, "three axes each of two channels");
    }

    #[test]
    fn turns_unroll_past_half_a_circle() {
        let q = |deg: f32| Quat::from_rotation_z(deg.to_radians()).to_array();
        let c = Channel { joint: 0, path: Part::Rotation, times: vec![0.0, 1.0, 2.0], values: [q(170.0), q(190.0), q(250.0)].concat() };
        let [_, _, z] = channel_tracks(&c, "hips");
        let v: Vec<f32> = z.keys.iter().map(|k| k.value.round()).collect();
        assert_eq!(v, vec![170.0, 190.0, 250.0]);
    }

    #[test]
    fn an_edit_changes_only_the_axis_it_keys() {
        let motions = edit(r#"(base: (model: "body", clip: "wave"), length: 1.0, tracks: [
            (path: "arm", what: TurnX, keys: [(0.0, 0.0), (1.0, 45.0)]),
        ])"#);
        let mut clips = vec![wave()];
        let problems = apply(&motions, &AssetLink::named("body"), &skeleton(), &mut clips);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(clips.len(), 1, "named as its base: replaces it");
        let pose = clips[0].sample(&skeleton(), 1.0, false);
        let turned = degrees(Quat::from_array(pose[1].rotation));
        assert!((turned.x - 45.0).abs() < 0.1 && (turned.z - 90.0).abs() < 0.1, "{turned}");
        assert_eq!(pose[0].translation, [0.0, 2.0, 0.0], "the hips are the model's");
    }

    #[test]
    fn an_edit_of_another_name_is_one_more_clip_and_a_wrong_bone_is_said() {
        let mut motions = edit(r#"(base: (model: "body", clip: "wave"), length: 0.0, tracks: [
            (path: "amr", what: TurnX, keys: [(0.0, 10.0)]),
        ])"#);
        let m = motions.clips.remove("wave").unwrap();
        motions.clips.insert("wave-big".into(), m);
        let mut clips = vec![wave()];
        let problems = apply(&motions, &AssetLink::named("body"), &skeleton(), &mut clips);
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[1].duration, 1.0, "the base's length when it says none");
        assert!(problems[0].contains("did you mean `arm`"), "{problems:?}");
        // Another model's clips are not touched.
        let mut other = vec![wave()];
        assert!(apply(&motions, &AssetLink::named("rock"), &skeleton(), &mut other).is_empty());
        assert_eq!(other, vec![wave()]);
    }

    #[test]
    fn a_clip_of_the_rig_alone_starts_from_the_rest_pose() {
        let motions = edit(r#"(base: (model: "body"), length: 2.0, tracks: [
            (path: "arm", what: TurnZ, keys: [(0.0, 0.0, Auto), (1.0, 30.0, Auto), (2.0, 0.0, Auto)]),
        ])"#);
        let mut clips = Vec::new();
        assert!(apply(&motions, &AssetLink::named("body"), &skeleton(), &mut clips).is_empty());
        assert_eq!((clips[0].name.as_str(), clips[0].duration), ("wave", 2.0));
        let pose = clips[0].sample(&skeleton(), 1.0, false);
        assert!((degrees(Quat::from_array(pose[1].rotation)).z - 30.0).abs() < 0.1);
        assert_eq!(pose[0], PoseTransform::default(), "the hips at rest");
    }

    #[test]
    fn writing_changes_only_the_track_that_changed() {
        let old = "// a wave, bigger\n(\n    base: (model: \"body\", clip: \"wave\"),\n    length: 1.0,\n    tracks: [\n        // the elbow\n        (path: \"arm\", what: TurnX, keys: [(0.0, 0.0), (1.0, 45.0)]),\n        (path: \"hips\", what: Y, keys: [(0.0, 1.0)]),\n    ],\n)\n";
        let mut m: Motion = ron::from_str(old).unwrap();
        m.tracks[1].keys[0].value = 1.5;
        m.tracks.push(Track { path: "arm".into(), what: Property::TurnZ, keys: vec![crate::keys::Key::shaped(0.5, 3.0, crate::keys::Tangent::Auto)], ease: Default::default() });
        let new = write(old, &m);
        assert!(new.contains("// a wave, bigger") && new.contains("// the elbow"), "{new}");
        assert!(new.contains("(path: \"hips\", what: Y, keys: [(0.0, 1.5)])"), "{new}");
        assert!(new.contains("keys: [(0.5, 3.0, Auto)]"), "{new}");
        let back: Motion = ron::from_str(&new).unwrap();
        assert_eq!(back.tracks.len(), 3);
        assert_eq!(write(&new, &back), new, "nothing changed, nothing written");
    }
}
