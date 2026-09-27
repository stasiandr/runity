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
//! * CMU (Carnegie Mellon's motion capture database: free for commercial
//!   products, the data itself not to be resold) — `$SCRAP_CMU` or
//!   `~/.cache/scrap/cmu/subjects`, ASF/AMC. Jumps, stops, turns, steps
//!   and ledges, one actor's bones per subject. `MM_SOURCE=mix` puts the
//!   takes `picked.txt` lists (or `CMU`, below) onto 100STYLE's
//!   skeleton beside its gaits: what a game can ship.
//!
//! All come out on LAFAN1's joint names, so the rest of the example —
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
    /// Takes, by the start of their name, looked in only for jumps.
    pub jump_only: Vec<String>,
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
        Ok("mix") => mix(),
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
        jump_only: vec!["jumps".into(), "multipleActions".into()],
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
        jump_only: Vec::new(),
    })
}

/// CMU's takes worth having when `picked.txt` is not there: jumps, stops,
/// turns, steps and ledges.
const CMU: &[&str] = &[
    "13_11", "13_13", "13_19", "13_32", "13_35", "13_36", "13_37", "13_38", "13_39", "13_40", "13_41", "13_42",
    "16_01", "16_02", "16_05", "16_06", "16_07", "16_08", "16_09", "16_10", "16_33", "16_34", "16_57",
    "49_02", "49_03", "75_01", "75_02", "75_03", "75_12", "82_02", "82_03", "82_04",
    "83_02", "83_03", "83_27", "83_28", "83_29", "83_30", "83_31", "83_32", "83_33", "83_34", "83_35",
    "91_39", "91_40", "91_41", "91_42", "91_43", "91_44", "91_45",
];

/// CMU's takes that are a jump, by the start of their trial's name.
const CMU_JUMPS: &[&str] = &[
    "13_11", "13_13", "13_19", "13_32", "13_39", "13_40", "13_41", "13_42", "16_01", "16_02", "16_05", "16_06", "16_07",
    "16_09", "16_10", "49_", "75_", "82_", "91_",
];

/// CMU's bones by LAFAN1's names, less the side.
const CMU_NAMES: &[(&str, &str)] = &[
    ("root", "Hips"),
    ("hipjoint", "HipJoint"),
    ("femur", "UpLeg"),
    ("tibia", "Leg"),
    ("foot", "Foot"),
    ("toes", "Toe"),
    ("lowerback", "Spine"),
    ("upperback", "Spine1"),
    ("thorax", "Spine2"),
    ("lowerneck", "Neck"),
    ("upperneck", "Neck1"),
    ("head", "Head"),
    ("clavicle", "Shoulder"),
    ("humerus", "Arm"),
    ("radius", "ForeArm"),
    ("wrist", "Hand"),
    ("hand", "Palm"),
    ("fingers", "Fingers"),
    ("thumb", "Thumb"),
];

/// A CMU take on its actor's skeleton, under LAFAN1's names, and the pose
/// it goes through to another rig: the T-pose the skeleton rests in, the
/// thighs straightened from the 20° the rest spreads them, the hips as
/// high as the take opens standing.
fn cmu_take(dir: &std::path::Path, trial: &str, rate: f32) -> anyhow::Result<(Skeleton, Clip, Vec<PoseTransform>)> {
    use scrap::glam::{Quat, Vec3};
    let subject = trial.split('_').next().unwrap_or("");
    let read = |path: std::path::PathBuf| {
        std::fs::read_to_string(&path).map_err(|e| {
            anyhow::anyhow!("{}: {e} — CMU goes in ~/.cache/scrap/cmu/subjects (or $SCRAP_CMU)", path.display())
        })
    };
    let asf = read(dir.join(format!("{subject}.asf")))?;
    let amc = read(dir.join(format!("{trial}.amc")))?;
    let (mut skeleton, clip) = scrap_import::asf::read(&asf, &amc, &format!("cmu_{trial}"), 0.0254 / 0.45, rate)?;
    for joint in &mut skeleton.joints {
        let (side, bare) = match joint.name.split_at(1) {
            ("l", rest) if CMU_NAMES.iter().any(|(a, _)| *a == rest) => ("Left", rest),
            ("r", rest) if CMU_NAMES.iter().any(|(a, _)| *a == rest) => ("Right", rest),
            _ => ("", joint.name.as_str()),
        };
        if let Some((_, name)) = CMU_NAMES.iter().find(|(a, _)| *a == bare) {
            joint.name = format!("{side}{name}");
        }
    }
    let mut t_pose = skeleton.rest_pose();
    let world = skeleton.world_matrices(&t_pose);
    let turn_of = |m: &scrap::glam::Mat4| m.to_scale_rotation_translation().1;
    for side in ["Left", "Right"] {
        let find = |n: &str| skeleton.joints.iter().position(|j| j.name == format!("{side}{n}"));
        let (Some(thigh), Some(knee)) = (find("UpLeg"), find("Leg")) else { continue };
        let Some(parent) = skeleton.joints[thigh].parent.map(|p| p as usize) else { continue };
        let down = (world[knee].w_axis - world[thigh].w_axis).truncate().normalize_or_zero();
        let straight = Quat::from_rotation_arc(down, -Vec3::Y) * turn_of(&world[thigh]);
        t_pose[thigh].rotation = (turn_of(&world[parent]).inverse() * straight).normalize().to_array();
    }
    let root = skeleton.joints.iter().position(|j| j.parent.is_none()).unwrap_or(0);
    let height = clip.sample(&skeleton, 0.0, false)[root].translation[1];
    t_pose[root].translation = [0.0, height, 0.0];
    Ok((skeleton, clip, t_pose))
}

/// 100STYLE's gaits and CMU's picked takes, all on 100STYLE's skeleton.
fn mix() -> anyhow::Result<Takes> {
    let mut takes = hundred_styles()?;
    let dir = cache("SCRAP_CMU", "cmu/subjects");
    let root = dir.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    // Each trial's frame rate, from the index when there is one.
    let mut rates = std::collections::HashMap::new();
    if let Ok(index) = std::fs::read_to_string(root.join("index.tsv")) {
        for line in index.lines() {
            let mut cells = line.split('\t');
            if let (Some(trial), Some(rate)) = (cells.next(), cells.next().and_then(|r| r.parse::<f32>().ok())) {
                rates.insert(trial.to_string(), rate);
            }
        }
    }
    let trials: Vec<String> = match std::fs::read_to_string(root.join("picked.txt")) {
        Ok(list) => list.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect(),
        Err(_) => CMU.iter().map(|s| s.to_string()).collect(),
    };
    let rest = takes.t_pose.clone();
    let aliases: [(&str, &str); 0] = [];
    for trial in &trials {
        let rate = rates.get(trial).copied().unwrap_or(120.0);
        let (skeleton, clip, t_pose) = cmu_take(&dir, trial, rate)?;
        takes.clips.push(clip.retarget_through(&skeleton, &t_pose, &takes.skeleton, &rest, &aliases, 60.0));
    }
    takes.source = "100STYLE + CMU";
    // Their run-ups end in a leap: never run on in them unasked.
    takes.jump_only = CMU_JUMPS.iter().map(|t| format!("cmu_{t}")).collect();
    Ok(takes)
}
