//! An Animation Clip (`.anim`) as a motion clip in `clips/`
//! (`scrap::motion`): position, rotation and scale curves by the path of
//! the thing they move, and the float curves scrap has a place for —
//! switching a thing on (`m_IsActive`), its sound's volume, its particles'
//! rate. Keys come over linear; Unity's tangents do not.

use std::path::Path;

use anyhow::{Context, Result};
use scrap::motion::{Motion, Property, Track};
use yaml_rust2::Yaml;

use super::yaml::{self, Get};
use super::Report;

/// Read one `.anim` file.
pub fn convert(path: &Path, report: &mut Report) -> Result<Motion> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let docs = yaml::documents(&text);
    let clip = docs
        .iter()
        .find(|d| d.kind == "AnimationClip")
        .context("no AnimationClip in it")?;
    Ok(from_yaml(&clip.body, report))
}

fn from_yaml(b: &Yaml, report: &mut Report) -> Motion {
    let settings = &b["m_AnimationClipSettings"];
    let mut motion = Motion {
        length: (settings.f32("m_StopTime").unwrap_or(0.0)
            - settings.f32("m_StartTime").unwrap_or(0.0))
        .max(0.0),
        tracks: Vec::new(),
    };
    // Unity is left-handed, scrap right-handed: z mirrored, as a scene's
    // positions are, and so the turns about x and y run the other way.
    let vectors: [(&str, [(Property, f32); 3]); 3] = [
        (
            "m_PositionCurves",
            [(Property::X, 1.0), (Property::Y, 1.0), (Property::Z, -1.0)],
        ),
        (
            "m_EulerCurves",
            [
                (Property::TurnX, -1.0),
                (Property::TurnY, -1.0),
                (Property::TurnZ, 1.0),
            ],
        ),
        (
            "m_ScaleCurves",
            [
                (Property::ScaleX, 1.0),
                (Property::ScaleY, 1.0),
                (Property::ScaleZ, 1.0),
            ],
        ),
    ];
    for (list, axes) in vectors {
        for c in b.list(list) {
            let path = c.str("path").unwrap_or("").to_string();
            let keys = c["curve"].list("m_Curve");
            for (i, (what, sign)) in axes.into_iter().enumerate() {
                let axis = ["x", "y", "z"][i];
                let keys: Vec<(f32, f32)> = keys
                    .iter()
                    .filter_map(|k| Some((k.f32("time")?, k["value"].f32(axis)? * sign)))
                    .collect();
                if keys.is_empty() {
                    continue;
                }
                motion.tracks.push(Track {
                    path: path.clone(),
                    what,
                    keys,
                    ease: Default::default(),
                });
            }
        }
    }
    if !b.list("m_RotationCurves").is_empty() {
        report.skip("an animation's quaternion rotation curves (Euler ones come over)");
    }
    for c in b.list("m_FloatCurves") {
        let attribute = c.str("attribute").unwrap_or("");
        // A place written as floats, one axis a curve.
        let (what, sign) = match attribute {
            "m_LocalPosition.x" => (Property::X, 1.0),
            "m_LocalPosition.y" => (Property::Y, 1.0),
            "m_LocalPosition.z" => (Property::Z, -1.0),
            "localEulerAnglesRaw.x" | "m_LocalEulerAngles.x" => (Property::TurnX, -1.0),
            "localEulerAnglesRaw.y" | "m_LocalEulerAngles.y" => (Property::TurnY, -1.0),
            "localEulerAnglesRaw.z" | "m_LocalEulerAngles.z" => (Property::TurnZ, 1.0),
            "m_LocalScale.x" => (Property::ScaleX, 1.0),
            "m_LocalScale.y" => (Property::ScaleY, 1.0),
            "m_LocalScale.z" => (Property::ScaleZ, 1.0),
            _ => (Property::Active, 1.0),
        };
        let what = match attribute {
            _ if what != Property::Active => what,
            "m_IsActive" => Property::Active,
            "m_Volume" => Property::Volume,
            "EmissionModule.rateOverTime.scalar" => Property::ParticleRate,
            other => {
                report.skip(format!("an animation's `{other}` curve"));
                continue;
            }
        };
        let keys: Vec<(f32, f32)> = c["curve"]
            .list("m_Curve")
            .iter()
            .filter_map(|k| Some((k.f32("time")?, k.f32("value")? * sign)))
            .collect();
        if keys.is_empty() {
            continue;
        }
        motion.tracks.push(Track {
            path: c.str("path").unwrap_or("").to_string(),
            what,
            keys,
            ease: Default::default(),
        });
    }
    if !b.list("m_PPtrCurves").is_empty() {
        report.skip("an animation's object curves (sprites, materials)");
    }
    motion
}

/// The clip as its file: a track a line, its keys on it.
pub fn text(motion: &Motion) -> String {
    let mut out = format!("(\n    length: {:?},\n    tracks: [\n", motion.length);
    for t in &motion.tracks {
        let keys: Vec<String> = t
            .keys
            .iter()
            .map(|(time, v)| format!("({time:?}, {v:?})"))
            .collect();
        let path = if t.path.is_empty() {
            String::new()
        } else {
            format!("path: {:?}, ", t.path)
        };
        out.push_str(&format!(
            "        ({path}what: {:?}, keys: [{}]),\n",
            t.what,
            keys.join(", ")
        ));
    }
    out.push_str("    ],\n)\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOLOGRAM: &str = "%YAML 1.1
--- !u!74 &7400000
AnimationClip:
  m_Name: hologram
  m_RotationCurves: []
  m_EulerCurves:
  - curve:
      m_Curve:
      - time: 0
        value: {x: 0, y: 30, z: 0}
      - time: 1
        value: {x: 0, y: 90, z: 0}
    path: Lid
  m_PositionCurves:
  - curve:
      m_Curve:
      - time: 0
        value: {x: 1, y: 2, z: 3}
    path:
  m_ScaleCurves: []
  m_FloatCurves:
  - curve:
      m_Curve:
      - time: 0
        value: 0
      - time: 0.25
        value: 1
    attribute: m_IsActive
    path: GameObject
    classID: 1
  - curve:
      m_Curve:
      - time: 0
        value: 0.5
    attribute: material._Glow
    path: GameObject
    classID: 23
  m_PPtrCurves: []
  m_AnimationClipSettings:
    m_StartTime: 0
    m_StopTime: 0.5
";

    #[test]
    fn an_anim_file_becomes_tracks_by_path() {
        let docs = yaml::documents(HOLOGRAM);
        let mut report = Report::default();
        let motion = from_yaml(&docs[0].body, &mut report);
        assert_eq!(motion.length, 0.5);
        let find = |path: &str, what| {
            motion
                .tracks
                .iter()
                .find(|t| t.path == path && t.what == what)
                .map(|t| t.keys.clone())
        };
        assert_eq!(find("Lid", Property::TurnY), Some(vec![(0.0, -30.0), (1.0, -90.0)]));
        assert_eq!(find("", Property::Z), Some(vec![(0.0, -3.0)]), "mirrored");
        assert_eq!(find("GameObject", Property::Active), Some(vec![(0.0, 0.0), (0.25, 1.0)]));
        assert!(format!("{report:?}").contains("material._Glow"), "{report:?}");
        // Its file reads back.
        let back: Motion = ron::from_str(&text(&motion)).unwrap();
        assert_eq!(back, motion);
    }
}
