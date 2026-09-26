//! The takes the database is made of, from one of two mocap sets kept
//! out of the repository:
//!
//! * LAFAN1 (Ubisoft La Forge, CC BY-NC-ND 4.0) — `$SCRAP_LAFAN1` or
//!   `~/.cache/scrap/lafan1`. Walking, running, obstacles, jumps; never
//!   in a build: non-commercial, no derivatives.
//! * 100STYLE (Ian Mason et al., CC BY 4.0) — `$SCRAP_100STYLE` or
//!   `~/.cache/scrap/100style`, `MM_SOURCE=100style`. A hundred gaits,
//!   each walked and run forwards, backwards and sideways, stood still
//!   and changed between; no jumps or climbs. Shippable with a credit.
//!
//! Both come out on LAFAN1's joint names, so the rest of the example —
//! the setup's feet and hands, the robot's aliases — reads either.

use std::path::PathBuf;

use scrap::animation::{Clip, PoseTransform, Skeleton};

/// Walking, running, and every take of getting over things: stairs,
/// boxes, ledges.
const LAFAN1: &[&str] = &[
    "walk1_subject1",
    "walk1_subject2",
    "walk1_subject5",
    "run1_subject2",
    "run1_subject5",
    "sprint1_subject2",
    "obstacles*",
    "jumps1*",
    "multipleActions1*",
];

/// 100STYLE's gaits taken by default: the plainest.
const STYLES: &[&str] = &["Neutral"];

/// Each gait's takes: forwards, backwards, sideways, walked and run,
/// standing, and the transitions between them.
const KINDS: &[&str] = &[
    "FW", "FR", "BW", "BR", "SW", "SR", "ID", "TR1", "TR2", "TR3", "TR4",
];

/// 100STYLE's joints by LAFAN1's names.
const NAMES: &[(&str, &str)] = &[
    ("Chest", "Spine"),
    ("Chest2", "Spine1"),
    ("Chest3", "Spine2"),
    ("Chest4", "Spine3"),
    ("Collar", "Shoulder"),
    ("Shoulder", "Arm"),
    ("Elbow", "ForeArm"),
    ("Wrist", "Hand"),
    ("Hip", "UpLeg"),
    ("Knee", "Leg"),
    ("Ankle", "Foot"),
];

pub struct Takes {
    pub skeleton: Skeleton,
    pub clips: Vec<Clip>,
    /// A pose every take's actor can be said to stand in, arms out: what
    /// retargeting goes through.
    pub t_pose: Vec<PoseTransform>,
    pub source: &'static str,
}

fn cache(var: &str, dir: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".cache/scrap")
            .join(dir)
    })
}

/// `MM_CLIPS=a,b*` picks takes by name, a trailing `*` for every take
/// starting so.
fn chosen(default: &[&str]) -> Vec<String> {
    match std::env::var("MM_CLIPS") {
        Ok(list) => list.split(',').map(str::to_string).collect(),
        Err(_) => default.iter().map(|s| s.to_string()).collect(),
    }
}

pub fn load() -> anyhow::Result<Takes> {
    match std::env::var("MM_SOURCE").as_deref() {
        Ok("100style") => hundred_styles(),
        _ => lafan1(),
    }
}

fn lafan1() -> anyhow::Result<Takes> {
    let dir = cache("SCRAP_LAFAN1", "lafan1");
    let mut names: Vec<String> = Vec::new();
    for name in chosen(LAFAN1) {
        match name.strip_suffix('*') {
            Some(prefix) => {
                let mut found: Vec<String> = std::fs::read_dir(&dir)?
                    .filter_map(|e| e.ok()?.file_name().into_string().ok())
                    .filter(|n| n.starts_with(prefix) && n.ends_with(".bvh"))
                    .map(|n| n.trim_end_matches(".bvh").to_string())
                    .collect();
                found.sort();
                names.extend(found);
            }
            None => names.push(name),
        }
    }
    let mut skeleton = None;
    let mut clips = Vec::new();
    for name in &names {
        let path = dir.join(format!("{name}.bvh"));
        let text = std::fs::read_to_string(&path).map_err(|e| {
            anyhow::anyhow!(
                "{}: {e} — LAFAN1 goes in ~/.cache/scrap/lafan1 (or $SCRAP_LAFAN1)",
                path.display()
            )
        })?;
        let (s, clip) = scrap_import::bvh::read(&text, name, 0.01)?;
        skeleton.get_or_insert(s);
        clips.push(clip);
    }
    let skeleton = skeleton.ok_or_else(|| anyhow::anyhow!("no takes"))?;
    // The takes open standing in a T-pose; the rest pose is no pose at all.
    let t_pose = clips[0].sample(&skeleton, 0.0, false);
    Ok(Takes {
        skeleton,
        clips,
        t_pose,
        source: "LAFAN1",
    })
}

/// A 100STYLE take's text with its joints under LAFAN1's names, and its
/// frames cut to `keep` — the set's own cuts, less the actor walking on
/// and off and standing in a T-pose.
fn renamed(text: &str, keep: Option<(usize, usize)>) -> String {
    let mut out = String::with_capacity(text.len());
    let motion = text.find("MOTION").unwrap_or(text.len());
    let (head, frames) = text.split_at(motion);
    let mut lines = frames.lines();
    let mut frames_out = String::new();
    if let (Some(title), Some(_count), Some(time)) = (lines.next(), lines.next(), lines.next()) {
        let body: Vec<&str> = lines.filter(|l| !l.trim().is_empty()).collect();
        let (from, to) = keep.unwrap_or((0, body.len()));
        let body = &body[from.min(body.len())..to.min(body.len())];
        frames_out = format!(
            "{title}\nFrames: {}\n{time}\n{}\n",
            body.len(),
            body.join("\n")
        );
    }
    for line in head.lines() {
        let trimmed = line.trim_start();
        let named = ["ROOT ", "JOINT "]
            .iter()
            .find_map(|k| trimmed.strip_prefix(k).map(|n| (k, n.trim())));
        match named {
            Some((keyword, name)) => {
                let side = ["Left", "Right"]
                    .into_iter()
                    .find(|s| name.starts_with(s))
                    .unwrap_or("");
                let bare = &name[side.len()..];
                let bare = NAMES
                    .iter()
                    .find(|(a, _)| *a == bare)
                    .map_or(bare, |(_, b)| b);
                out.push_str(&line[..line.len() - trimmed.len()]);
                out.push_str(&format!("{keyword}{side}{bare}"));
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out + &frames_out
}

/// Each take's frames worth keeping, from the set's `Frame_Cuts.csv`:
/// `Style_KIND` → (first, last).
fn cuts(dir: &std::path::Path) -> std::collections::HashMap<String, (usize, usize)> {
    let mut cuts = std::collections::HashMap::new();
    let Ok(text) = std::fs::read_to_string(dir.join("Frame_Cuts.csv")) else {
        return cuts;
    };
    let mut lines = text.lines();
    let header: Vec<&str> = lines
        .next()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .collect();
    for line in lines {
        let cells: Vec<&str> = line.split(',').map(str::trim).collect();
        for (i, h) in header.iter().enumerate() {
            let Some(kind) = h.strip_suffix("_START") else {
                continue;
            };
            let (Some(a), Some(b)) = (
                cells.get(i).and_then(|v| v.parse().ok()),
                cells.get(i + 1).and_then(|v| v.parse().ok()),
            ) else {
                continue;
            };
            cuts.insert(format!("{}_{kind}", cells[0]), (a, b));
        }
    }
    cuts
}

fn hundred_styles() -> anyhow::Result<Takes> {
    let dir = cache("SCRAP_100STYLE", "100style").join("100STYLE");
    let styles: Vec<String> = match std::env::var("MM_STYLES") {
        Ok(list) => list.split(',').map(str::to_string).collect(),
        Err(_) => STYLES.iter().map(|s| s.to_string()).collect(),
    };
    let cuts = cuts(&dir);
    let mut skeleton = None;
    let mut clips = Vec::new();
    for style in &styles {
        for kind in KINDS {
            let name = format!("{style}_{kind}");
            let path = dir.join(style).join(format!("{name}.bvh"));
            let Ok(text) = std::fs::read_to_string(&path) else {
                if !path.exists() && !dir.join(style).exists() {
                    anyhow::bail!("{}: no such gait — 100STYLE goes in ~/.cache/scrap/100style (or $SCRAP_100STYLE)", dir.join(style).display());
                }
                continue;
            };
            let (s, clip) =
                scrap_import::bvh::read(&renamed(&text, cuts.get(&name).copied()), &name, 0.01)?;
            skeleton.get_or_insert(s);
            clips.push(clip);
        }
    }
    let skeleton = skeleton.ok_or_else(|| anyhow::anyhow!("no takes"))?;
    // Every rotation at zero is a T-pose here, but with the hips on the
    // floor: stand them as high as the first take's.
    let mut t_pose = skeleton.rest_pose();
    let root = skeleton
        .joints
        .iter()
        .position(|j| j.parent.is_none())
        .unwrap_or(0);
    let height = clips[0].sample(&skeleton, 0.0, false)[root].translation[1];
    t_pose[root].translation = [0.0, height, 0.0];
    Ok(Takes {
        skeleton,
        clips,
        t_pose,
        source: "100STYLE",
    })
}
