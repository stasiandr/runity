//! `--dacha DIR`: the Dacha robot walking and jumping by motion matching —
//! its Mixamo rig (`A_BreathingIdle`) with LAFAN1 retargeted onto it
//! through the two T-poses, and its pieces hung on the bones as the game's
//! pawn prefab hangs them (after `dacha-runity/src/own_body.rs`).

use std::path::Path;

use scrap::animation::{PoseTransform, Skeleton};
use scrap::asset::MeshLibrary;
use scrap::glam::{Mat4, Vec3};
use scrap::hecs::{Entity, World};
use scrap::matching::{Database, Setup};
use scrap::{EntityDesc, Transform};

/// The pawn prefab the pieces are read from, and the rig's model.
const PAWN: &str = "Prefabs_PlayerPawn";
const RIG_MODEL: &str = "A_BreathingIdle";
/// Which bone each slot of the rig hangs on (as Unity's prefab has it).
const ON_BONES: [(&str, &str); 5] = [
    ("BodySlot", "mixamorig:Spine2"),
    ("FootLeft", "mixamorig:LeftToeBase"),
    ("FootRight", "mixamorig:RightToeBase"),
    // The hands the game shows in first person instead; seen from outside,
    // on the forearms' ends.
    ("HandLeft", "mixamorig:LeftForeArm"),
    ("HandRight", "mixamorig:RightForeArm"),
];
/// The pawn's eye over its feet: where the head's pieces hang.
const EYE: f32 = 1.9;

/// Where a piece hangs: on a bone of the rig, or with the head.
#[derive(Debug, Clone, Copy)]
enum Frame {
    Bone(usize),
    Head,
}

/// The robot: its rig as a skeleton in metres, its pieces spawned, and
/// what places them.
pub struct Robot {
    pub skeleton: Skeleton,
    /// How tall it stands against LAFAN1's actors: its hips' height over
    /// theirs, set when the takes are retargeted.
    pub height_ratio: f32,
    /// The rig's scale: pieces hang in the rig's own units.
    scale: f32,
    rig_at: Mat4,
    head_bone: usize,
    head_rest: Vec3,
    pieces: Vec<(Entity, Frame, Mat4)>,
}

/// A bone's frame without scale.
fn unscaled(m: Mat4) -> Mat4 {
    let (_, turn, at) = m.to_scale_rotation_translation();
    Mat4::from_rotation_translation(turn, at)
}

/// A place in a bone's frame as the prefab gives it, in the model's frame
/// of that bone: a half turn about its y apart (own_body's `in_bone`).
fn in_bone(place: Mat4) -> Mat4 {
    Mat4::from_rotation_y(std::f32::consts::PI) * place
}

/// The rig as a skeleton standing in its bind pose (a T-pose), in metres,
/// the rig's scale taken in, rooted at the hips: the carrier the file hangs
/// the hips from — its hundredth and its turn — folded into them.
fn rig_skeleton(skin: &scrap::asset::MeshSkin, scale: f32) -> Option<(Skeleton, Vec<usize>)> {
    let joints = &skin.skeleton.joints;
    let hips = joints.iter().position(|j| j.name.ends_with("Hips"))?;
    // The hips and all below them, parents first.
    let mut keep = vec![hips];
    let mut i = 0;
    while i < keep.len() {
        let at = keep[i];
        keep.extend((0..joints.len()).filter(|&c| joints[c].parent == Some(at as u16)));
        i += 1;
    }
    let bind: Vec<Mat4> = keep
        .iter()
        .map(|&j| {
            let world = Mat4::from_cols_array_2d(&joints[j].inverse_bind).inverse();
            let (_, turn, at) = world.to_scale_rotation_translation();
            Mat4::from_rotation_translation(turn, at * scale)
        })
        .collect();
    let index = |j: usize| keep.iter().position(|&k| k == j);
    let skeleton = Skeleton {
        joints: keep
            .iter()
            .enumerate()
            .map(|(n, &j)| {
                let parent = joints[j].parent.and_then(|p| index(p as usize));
                let local = match parent {
                    Some(p) => bind[p].inverse() * bind[n],
                    None => bind[n],
                };
                let (_, turn, at) = local.to_scale_rotation_translation();
                scrap::animation::Joint {
                    name: joints[j].name.clone(),
                    parent: parent.map(|p| p as u16),
                    inverse_bind: bind[n].inverse().to_cols_array_2d(),
                    rest: PoseTransform { translation: at.to_array(), rotation: turn.to_array(), scale: [1.0; 3] },
                }
            })
            .collect(),
    };
    Some((skeleton, keep))
}

/// A line's place under its parent: an instance's root part's where it
/// says one, or the line's.
fn place_of(line: &EntityDesc, root_of: &dyn Fn(&str) -> Option<scrap::EntityId>) -> Mat4 {
    let part = (!line.prefab.as_str().is_empty()).then(|| root_of(line.prefab.as_str())).flatten();
    part.and_then(|id| line.overrides.get(&id)?.transform).unwrap_or(line.transform).matrix()
}

/// The pawn prefab's pieces — the rig's slots' and the head's — each with
/// its place in its frame, and the rig's place under the pawn.
fn pieces_of(dir: &Path) -> Option<(Vec<(String, Result<String, ()>, Mat4)>, Mat4)> {
    let prefabs = dir.join("prefabs");
    let (_, pawn) = scrap::Prefabs::read(prefabs.join(format!("{PAWN}.prefab"))).ok()?;
    let root_of = |name: &str| scrap::Prefabs::read(prefabs.join(format!("{name}.prefab"))).ok().map(|(_, d)| d.id);
    fn walk(
        line: &EntityDesc,
        above: Mat4,
        frame: &Result<String, ()>,
        root_of: &dyn Fn(&str) -> Option<scrap::EntityId>,
        out: &mut Vec<(String, Result<String, ()>, Mat4)>,
    ) {
        let here = above * place_of(line, root_of);
        if !line.prefab.as_str().is_empty() {
            out.push((line.prefab.as_str().to_string(), frame.clone(), here));
            return;
        }
        for child in &line.children {
            walk(child, here, frame, root_of, out);
        }
    }
    let rig = pawn.children.iter().find(|c| c.name == "CharacterRig")?;
    let head = pawn.children.iter().find(|c| c.name == "Head")?;
    let mut pieces = Vec::new();
    for (slot, bone) in ON_BONES {
        let Some(line) = rig.children.iter().find(|c| c.name == slot) else { continue };
        let before = pieces.len();
        walk(line, Mat4::IDENTITY, &Ok(bone.to_string()), &root_of, &mut pieces);
        for piece in &mut pieces[before..] {
            piece.2 = in_bone(piece.2);
        }
    }
    for line in &head.children {
        walk(line, Mat4::IDENTITY, &Err(()), &root_of, &mut pieces);
    }
    Some((pieces, rig.transform.matrix()))
}

/// The Dacha project at `dir`: its scene opened for its library and
/// prefabs.
pub fn open(dir: &Path) -> anyhow::Result<scrap::LiveScene> {
    let scene = std::fs::read_dir(dir.join("scenes"))?
        .filter_map(|e| e.ok()?.path().to_str().map(String::from))
        .find(|p| p.ends_with(".ron"))
        .ok_or_else(|| anyhow::anyhow!("no scene in {}", dir.display()))?;
    let (live, _) = scrap::LiveScene::open(&scene)?;
    Ok(live)
}

impl Robot {
    /// The robot's rig and pieces, spawned into `world`.
    pub fn spawn(
        dir: &Path,
        live: &mut scrap::LiveScene,
        world: &mut World,
        gpu: &scrap::Gpu,
        renderer: &mut scrap::render::Renderer,
    ) -> anyhow::Result<Robot> {
        let skin = live
            .library()
            .and_then(|l| l.mesh_by_name(RIG_MODEL))
            .and_then(|m| m.skin_owned())
            .ok_or_else(|| anyhow::anyhow!("no skin in {RIG_MODEL}"))?;
        let (pieces, rig_at) = pieces_of(dir).ok_or_else(|| anyhow::anyhow!("no {PAWN} prefab"))?;
        let (scale, _, _) = rig_at.to_scale_rotation_translation();
        let (skeleton, _) = rig_skeleton(&skin, scale.x).ok_or_else(|| anyhow::anyhow!("no hips in {RIG_MODEL}"))?;
        let head_bone = skeleton.joints.iter().position(|j| j.name.ends_with(":Head")).unwrap_or(0);
        let head_rest = skeleton.world_matrices(&skeleton.rest_pose())[head_bone].w_axis.truncate();
        let mut spawned = Vec::new();
        for (prefab, frame, local) in pieces {
            let frame = match frame {
                Ok(bone) => match skeleton.joints.iter().position(|j| j.name == bone) {
                    Some(j) => Frame::Bone(j),
                    None => continue,
                },
                Err(()) => Frame::Head,
            };
            match live.spawn_prefab(&prefab, Transform::default(), None, world, gpu, renderer) {
                Ok(done) => spawned.push((done.root, frame, local)),
                Err(why) => eprintln!("{prefab}: {why}"),
            }
        }
        eprintln!("rig under the pawn: {rig_at:?}");
        // The rig's scale, and none of its turn: the database already faces
        // the character ahead.
        let rig_at = Mat4::from_scale(scale);
        Ok(Robot { skeleton, height_ratio: 1.0, scale: scale.x, rig_at, head_bone, head_rest, pieces: spawned })
    }

    /// Every piece where the pose puts it: `placed` is the character's place,
    /// `posed` its joints in its own frame (metres).
    pub fn pose(&self, world: &mut World, placed: Mat4, posed: &[Mat4]) {
        let rig = placed * self.rig_at;
        let head_moved = posed[self.head_bone].w_axis.truncate() - self.head_rest;
        for &(piece, frame, local) in &self.pieces {
            let at = match frame {
                Frame::Bone(j) => {
                    // The bone in the rig's own units, as the pieces are.
                    let (_, turn, at) = posed[j].to_scale_rotation_translation();
                    rig * unscaled(Mat4::from_rotation_translation(turn, at / self.scale)) * local
                }
                Frame::Head => placed * Mat4::from_translation(Vec3::Y * EYE + head_moved) * local,
            };
            if let Ok(mut t) = world.get::<&mut Transform>(piece) {
                let (scale, turn, position) = at.to_scale_rotation_translation();
                t.position = position;
                t.set_rotation(turn);
                t.scale = scale;
            }
            let _ = world.insert_one(piece, scrap::world::WorldTransform(at));
        }
    }
}

/// LAFAN1's clips on the robot's rig: each retargeted through the takes'
/// opening T-pose and the rig's bind, and the database of them.
pub fn database(robot: &mut Robot, from: &Skeleton, clips: &[scrap::animation::Clip]) -> anyhow::Result<Database> {
    let started = std::time::Instant::now();
    // The takes open standing in a T-pose, as the rig binds.
    let t_pose = clips.first().map(|c| c.sample(from, 0.0, false)).ok_or_else(|| anyhow::anyhow!("no clips"))?;
    let hips = |s: &Skeleton, pose: &[PoseTransform]| {
        let root = s.joints.iter().position(|j| j.parent.is_none()).unwrap_or(0);
        s.world_matrices(pose)[root].w_axis.y
    };
    robot.height_ratio = hips(&robot.skeleton, &robot.skeleton.rest_pose()) / hips(from, &t_pose);
    eprintln!("the robot stands {:.2} of the takes' height", robot.height_ratio);
    let robot = &robot.skeleton;
    let aliases = [("LeftToe", "LeftToeBase"), ("RightToe", "RightToeBase")];
    let rest = robot.rest_pose();
    let retargeted: Vec<_> = clips.iter().map(|c| c.retarget_through(from, &t_pose, robot, &rest, &aliases, 30.0)).collect();
    let db = Database::build(robot, &retargeted, Setup::default()).map_err(anyhow::Error::msg)?;
    eprintln!("{} frames retargeted onto the robot in {:.1?}", db.len(), started.elapsed());
    Ok(db)
}
