//! IK on an animated skeleton: after the clips and layers have posed it,
//! before it is skinned — feet put on the ground they stand over, a head
//! turned to look at something. Final IK's Grounder and LookAt, Unreal's
//! foot placement, for a skinned character rather than a ragdoll (the
//! ragdoll's IK is `scrap-character`'s; the solver is geometry's
//! [`crate::geometry_ik::two_bone`], the same for both).
//!
//! A line says it:
//!
//! ```ron
//! ik: (
//!     feet: (left: "LeftFoot", right: "RightFoot", max_adjust: 0.4),
//!     look_at: (joint: "Head", target: EntityRef("4f1c…"), weight: 0.8),
//! )
//! ```
//!
//! A foot is the end of a two-bone leg: its parent is the knee and the
//! knee's the hip, as every humanoid rig has them. The animation stands on
//! a flat floor at the entity's origin; each foot keeps its height above
//! that floor over the ground actually under it, found by the ground probe
//! the engine hands in (the scene's colliders). A foot that must go down
//! further than the leg reaches takes the hips down with it, so the other
//! leg bends rather than the foot floating; the foot is turned to the
//! slope. A look turns the joint toward the target, as far as `limit`
//! degrees from where the character faces (+Z, as glTF characters do).

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::animation::{bare_joint_name, PoseTransform, Skeleton};
use crate::id::EntityRef;

/// A line's IK: feet on the ground, a look at something.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ik {
    #[serde(skip_serializing_if = "Feet::is_off")]
    pub feet: Feet,
    #[serde(skip_serializing_if = "LookAt::is_off")]
    pub look_at: LookAt,
}

/// Two feet on the ground, each the end of a two-bone leg.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Feet {
    /// The feet's joints; empty is off.
    pub left: String,
    pub right: String,
    /// Metres a foot goes up or down at most, and the hips down.
    pub max_adjust: f32,
    /// How much a foot turns to the slope under it, 0..1.
    pub align: f32,
}

impl Default for Feet {
    fn default() -> Self {
        Self {
            left: String::new(),
            right: String::new(),
            max_adjust: 0.4,
            align: 1.0,
        }
    }
}

impl Feet {
    pub fn is_off(&self) -> bool {
        self.left.is_empty() && self.right.is_empty()
    }
}

/// A joint turned to look at another entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LookAt {
    /// The joint that turns (a head); empty is off.
    pub joint: String,
    pub target: EntityRef,
    /// 0..1.
    pub weight: f32,
    /// Degrees from where the character faces the look goes at most.
    pub limit: f32,
}

impl Default for LookAt {
    fn default() -> Self {
        Self {
            joint: String::new(),
            target: EntityRef::default(),
            weight: 1.0,
            limit: 70.0,
        }
    }
}

impl LookAt {
    pub fn is_off(&self) -> bool {
        self.joint.is_empty()
    }
}

/// The entity a look is at, found from its reference once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LookingAt(pub hecs::Entity);

/// The ground under a point: from `from` straight down at most `depth`
/// metres, where it is met and which way it faces. A physics raycast, the
/// scene's colliders, or a plane in a test.
pub type Ground<'a> = &'a dyn Fn(Vec3, f32) -> Option<(Vec3, Vec3)>;

/// A [`Ground`] made for one step, owning what it looks at.
pub type Probe = Box<dyn Fn(Vec3, f32) -> Option<(Vec3, Vec3)>>;

/// No ground: feet keep the animation's height.
pub fn no_ground(_: Vec3, _: f32) -> Option<(Vec3, Vec3)> {
    None
}

/// A joint by name, matched without a rig's prefix.
pub fn joint(skeleton: &Skeleton, name: &str) -> Option<usize> {
    let wanted = bare_joint_name(name);
    skeleton
        .joints
        .iter()
        .position(|j| j.name == name || bare_joint_name(&j.name) == wanted)
}

impl Ik {
    /// What will not work on this skeleton: a joint it does not have (with
    /// the nearest it does), a foot with no knee and hip above it.
    pub fn problems(&self, skeleton: &Skeleton) -> Vec<String> {
        let names: Vec<&str> = skeleton
            .joints
            .iter()
            .map(|j| bare_joint_name(&j.name))
            .collect();
        let mut out = Vec::new();
        let mut named = |field: &str, name: &str| -> Option<usize> {
            if name.is_empty() {
                return None;
            }
            let found = joint(skeleton, name);
            if found.is_none() {
                let near = crate::spelling::closest(bare_joint_name(name), names.iter().copied())
                    .map(|n| format!(" — did you mean `{n}`?"))
                    .unwrap_or_default();
                out.push(format!(
                    "ik: {field} `{name}` is not a joint of the skeleton{near}"
                ));
            }
            found
        };
        let feet = [
            ("feet.left", named("feet.left", &self.feet.left)),
            ("feet.right", named("feet.right", &self.feet.right)),
        ];
        named("look_at.joint", &self.look_at.joint);
        for (field, foot) in feet {
            if let Some(foot) = foot {
                if leg(skeleton, foot).is_none() {
                    out.push(format!("ik: {field} has no knee and hip above it to bend"));
                }
            }
        }
        out
    }
}

/// A foot's knee and hip: its parent and its parent's.
pub(crate) fn leg(skeleton: &Skeleton, foot: usize) -> Option<(usize, usize)> {
    let knee = skeleton.joints.get(foot)?.parent? as usize;
    let hip = skeleton.joints.get(knee)?.parent? as usize;
    Some((knee, hip))
}

/// Each joint's place in the world: the skeleton's model-space matrices
/// under the entity's.
pub(crate) fn placed_joints(skeleton: &Skeleton, pose: &[PoseTransform], placed: Mat4) -> Vec<Mat4> {
    skeleton
        .world_matrices(pose)
        .into_iter()
        .map(|m| placed * m)
        .collect()
}

pub(crate) fn turn_of(m: Mat4) -> Quat {
    m.to_scale_rotation_translation().1
}

/// The matrix a joint hangs from in the world: its parent's, or the
/// entity's for a root.
fn parent_of(skeleton: &Skeleton, world: &[Mat4], placed: Mat4, j: usize) -> Mat4 {
    skeleton.joints[j]
        .parent
        .and_then(|p| world.get(p as usize).copied())
        .unwrap_or(placed)
}

/// Turn joint `j` by `turn` in the world, written into its local pose.
pub(crate) fn turn_joint(
    skeleton: &Skeleton,
    pose: &mut [PoseTransform],
    world: &[Mat4],
    placed: Mat4,
    j: usize,
    turn: Quat,
) {
    let parent = turn_of(parent_of(skeleton, world, placed, j));
    let local = Quat::from_array(pose[j].rotation);
    pose[j].rotation = (parent.inverse() * turn * parent * local)
        .normalize()
        .to_array();
}

/// Bend the skeleton's pose by `ik`: feet on `ground`, the look toward
/// `look` (a point in the world). `placed` is the entity's place.
pub fn solve(
    skeleton: &Skeleton,
    pose: &mut [PoseTransform],
    placed: Mat4,
    ik: &Ik,
    look: Option<Vec3>,
    ground: Ground,
) {
    if pose.len() != skeleton.joints.len() {
        return;
    }
    if !ik.feet.is_off() {
        feet(skeleton, pose, placed, &ik.feet, ground);
    }
    if let (Some(target), Some(head)) = (look, joint(skeleton, &ik.look_at.joint)) {
        look_at(skeleton, pose, placed, head, target, &ik.look_at);
    }
}

fn feet(
    skeleton: &Skeleton,
    pose: &mut [PoseTransform],
    placed: Mat4,
    feet: &Feet,
    ground: Ground,
) {
    let up = Vec3::Y;
    let floor = placed.w_axis.truncate();
    let reach = feet.max_adjust.max(0.0);
    let legs: Vec<(usize, usize, usize)> = [&feet.left, &feet.right]
        .into_iter()
        .filter_map(|name| {
            let foot = joint(skeleton, name)?;
            let (knee, hip) = leg(skeleton, foot)?;
            Some((foot, knee, hip))
        })
        .collect();
    if legs.is_empty() {
        return;
    }
    let mut world = placed_joints(skeleton, pose, placed);
    // Where each foot should go: as high over the ground under it as the
    // animation has it over the floor, and which way that ground faces.
    let aims: Vec<Option<(Vec3, Vec3)>> = legs
        .iter()
        .map(|&(foot, _, _)| {
            let at = world[foot].w_axis.truncate();
            let height = (at - floor).dot(up);
            let on_floor = at - up * height;
            let (hit, normal) = ground(on_floor + up * reach, 2.0 * reach)?;
            let lift = (hit - on_floor).dot(up).clamp(-reach, reach);
            Some((at + up * lift, normal.normalize_or(up)))
        })
        .collect();
    // The hips go down as far as the lowest foot has to, so its leg reaches.
    let drop = legs
        .iter()
        .zip(&aims)
        .filter_map(|(&(foot, _, _), aim)| {
            aim.map(|(to, _)| (to - world[foot].w_axis.truncate()).dot(up))
        })
        .fold(0.0f32, f32::min);
    let hips = skeleton.joints[legs[0].2].parent.map(|p| p as usize);
    if let (Some(hips), true) = (hips, drop < 0.0) {
        let parent = parent_of(skeleton, &world, placed, hips);
        let local = parent.inverse().transform_vector3(up * drop);
        let t = Vec3::from_array(pose[hips].translation) + local;
        pose[hips].translation = t.to_array();
        world = placed_joints(skeleton, pose, placed);
    }
    let forward = placed.transform_vector3(Vec3::Z).normalize_or(Vec3::Z);
    for (&(foot, knee, hip), aim) in legs.iter().zip(&aims) {
        let Some((target, normal)) = *aim else {
            continue;
        };
        // The foot keeps the animation's turn through the leg's, then
        // turns to the slope.
        let kept = turn_of(world[foot]);
        reach_leg(skeleton, pose, placed, &mut world, (foot, knee, hip), target, forward);
        let slope = Quat::IDENTITY.slerp(
            Quat::from_rotation_arc(up, normal),
            feet.align.clamp(0.0, 1.0),
        );
        let now = turn_of(world[foot]);
        turn_joint(
            skeleton,
            pose,
            &world,
            placed,
            foot,
            slope * kept * now.inverse(),
        );
        world = placed_joints(skeleton, pose, placed);
    }
}

/// Bend a leg — `(foot, knee, hip)` — so its foot is at `target`, the knee
/// bending the way it already does (forward, `forward`, when straight).
/// The foot's own turn is the leg's; `world` is refreshed.
pub(crate) fn reach_leg(
    skeleton: &Skeleton,
    pose: &mut [PoseTransform],
    placed: Mat4,
    world: &mut Vec<Mat4>,
    (foot, knee, hip): (usize, usize, usize),
    target: Vec3,
    forward: Vec3,
) {
    let up = Vec3::Y;
    let (h, k, f) = (
        world[hip].w_axis.truncate(),
        world[knee].w_axis.truncate(),
        world[foot].w_axis.truncate(),
    );
    // The knee bends the way it already does, leaning forward a little: a
    // leg near straight has almost no bend to say which way, and without
    // the lean the knee could flip between steps.
    let along = (f - h).normalize_or(-up);
    let out = (k - h) - along * (k - h).dot(along);
    let pole = k + out + forward.normalize_or_zero() * 0.03;
    let (knee_to, foot_to) = crate::geometry_ik::two_bone(h, h.distance(k), k.distance(f), target, pole);
    turn_joint(
        skeleton,
        pose,
        world,
        placed,
        hip,
        Quat::from_rotation_arc((k - h).normalize_or(up), (knee_to - h).normalize_or(up)),
    );
    *world = placed_joints(skeleton, pose, placed);
    let (k, f) = (world[knee].w_axis.truncate(), world[foot].w_axis.truncate());
    turn_joint(
        skeleton,
        pose,
        world,
        placed,
        knee,
        Quat::from_rotation_arc((f - k).normalize_or(up), (foot_to - k).normalize_or(up)),
    );
    *world = placed_joints(skeleton, pose, placed);
}

fn look_at(
    skeleton: &Skeleton,
    pose: &mut [PoseTransform],
    placed: Mat4,
    head: usize,
    target: Vec3,
    look: &LookAt,
) {
    let world = placed_joints(skeleton, pose, placed);
    let at = world[head].w_axis.truncate();
    let facing = placed.transform_vector3(Vec3::Z).normalize_or(Vec3::Z);
    let mut want = (target - at).normalize_or(facing);
    // No further round than the limit from where the character faces.
    let limit = look.limit.to_radians().max(0.0);
    let off = facing.angle_between(want);
    if off > limit {
        let round = Quat::from_rotation_arc(facing, want);
        want = Quat::IDENTITY.slerp(round, limit / off) * facing;
    }
    // Which way the joint looks: where the character faces, as the rest
    // pose has the joint.
    let rest = skeleton.world_matrices(&skeleton.rest_pose());
    let own = turn_of(rest[head]).inverse() * Vec3::Z;
    let now = (turn_of(world[head]) * own).normalize_or(facing);
    let turn = Quat::IDENTITY.slerp(
        Quat::from_rotation_arc(now, want),
        look.weight.clamp(0.0, 1.0),
    );
    turn_joint(skeleton, pose, &world, placed, head, turn);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Joint;

    /// Hips a metre up, two legs of two half-metre bones down to feet on
    /// the floor, a spine and a head: every joint by its offset from its
    /// parent.
    fn legs() -> Skeleton {
        let joint = |name: &str, parent: Option<u16>, at: Vec3| Joint {
            name: format!("mixamorig:{name}"),
            parent,
            inverse_bind: Mat4::IDENTITY.to_cols_array_2d(),
            rest: PoseTransform {
                translation: at.to_array(),
                ..Default::default()
            },
        };
        // The knees a little forward, as a walk has them.
        Skeleton {
            joints: vec![
                joint("Hips", None, Vec3::new(0.0, 1.0, 0.0)),
                joint("LeftUpLeg", Some(0), Vec3::new(0.1, 0.0, 0.0)),
                joint("LeftLeg", Some(1), Vec3::new(0.0, -0.5, 0.05)),
                joint("LeftFoot", Some(2), Vec3::new(0.0, -0.5, -0.05)),
                joint("RightUpLeg", Some(0), Vec3::new(-0.1, 0.0, 0.0)),
                joint("RightLeg", Some(4), Vec3::new(0.0, -0.5, 0.05)),
                joint("RightFoot", Some(5), Vec3::new(0.0, -0.5, -0.05)),
                joint("Head", Some(0), Vec3::new(0.0, 0.7, 0.0)),
            ],
        }
    }

    fn ik() -> Ik {
        Ik {
            feet: Feet {
                left: "LeftFoot".into(),
                right: "RightFoot".into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn at(skeleton: &Skeleton, pose: &[PoseTransform], placed: Mat4, j: usize) -> Vec3 {
        placed_joints(skeleton, pose, placed)[j].w_axis.truncate()
    }

    /// A plane through `point` facing `normal`, as a ground probe.
    fn plane(point: Vec3, normal: Vec3) -> impl Fn(Vec3, f32) -> Option<(Vec3, Vec3)> {
        move |from: Vec3, depth: f32| {
            let n = normal.normalize();
            // Down from `from` to the plane.
            let t = (from - point).dot(n) / n.y;
            (t >= 0.0 && t <= depth).then(|| (from - Vec3::Y * t, n))
        }
    }

    fn bone_lengths(skeleton: &Skeleton, pose: &[PoseTransform]) -> Vec<f32> {
        let world = placed_joints(skeleton, pose, Mat4::IDENTITY);
        (1..world.len())
            .filter_map(|j| {
                let p = skeleton.joints[j].parent? as usize;
                Some(
                    world[j]
                        .w_axis
                        .truncate()
                        .distance(world[p].w_axis.truncate()),
                )
            })
            .collect()
    }

    #[test]
    fn feet_on_a_flat_floor_stay_where_the_animation_put_them() {
        let skeleton = legs();
        let mut pose = skeleton.rest_pose();
        let before = pose.clone();
        solve(
            &skeleton,
            &mut pose,
            Mat4::IDENTITY,
            &ik(),
            None,
            &plane(Vec3::ZERO, Vec3::Y),
        );
        for (a, b) in pose.iter().zip(&before) {
            assert!(
                Quat::from_array(a.rotation).angle_between(Quat::from_array(b.rotation)) < 1e-3
            );
            assert!(
                Vec3::from_array(a.translation).distance(Vec3::from_array(b.translation)) < 1e-4
            );
        }
    }

    #[test]
    fn a_foot_over_a_step_stands_on_it_and_one_over_a_hole_takes_the_hips_down() {
        let skeleton = legs();
        let lengths = bone_lengths(&skeleton, &skeleton.rest_pose());
        // The left foot over a step 15 cm up, the right over ground 20 cm
        // down: the right leg cannot reach without the hips going down.
        let ground = |from: Vec3, depth: f32| {
            let y = if from.x > 0.0 { 0.15 } else { -0.2 };
            (from.y - y <= depth && from.y >= y).then(|| (Vec3::new(from.x, y, from.z), Vec3::Y))
        };
        let mut pose = skeleton.rest_pose();
        solve(&skeleton, &mut pose, Mat4::IDENTITY, &ik(), None, &ground);
        let left = at(&skeleton, &pose, Mat4::IDENTITY, 3);
        let right = at(&skeleton, &pose, Mat4::IDENTITY, 6);
        assert!((left.y - 0.15).abs() < 1e-3, "on the step: {left}");
        assert!((right.y + 0.2).abs() < 1e-3, "down in the hole: {right}");
        let hips = at(&skeleton, &pose, Mat4::IDENTITY, 0);
        assert!((hips.y - 0.8).abs() < 1e-3, "the hips down 20 cm: {hips}");
        let now = bone_lengths(&skeleton, &pose);
        for (a, b) in now.iter().zip(&lengths) {
            assert!((a - b).abs() < 1e-4, "bones keep their lengths: {now:?}");
        }
        let knee = at(&skeleton, &pose, Mat4::IDENTITY, 2);
        assert!(knee.z > 0.0, "the knee still bends forward: {knee}");
    }

    #[test]
    fn a_foot_on_a_slope_stands_on_it_turned_to_it_within_reach() {
        let skeleton = legs();
        // Rising 20° toward +x, through the origin; the character stands a
        // metre along the slope and a little up, where the ground is.
        let normal = Vec3::new(-(20f32.to_radians().sin()), 20f32.to_radians().cos(), 0.0);
        let ground = plane(Vec3::ZERO, normal);
        let placed = Mat4::from_translation(Vec3::new(1.0, 20f32.to_radians().tan(), 0.0));
        let mut pose = skeleton.rest_pose();
        let mut far = ik();
        far.feet.max_adjust = 0.1;
        solve(&skeleton, &mut pose, placed, &far, None, &ground);
        let floor_y = |x: f32| x * 20f32.to_radians().tan();
        let left = at(&skeleton, &pose, placed, 3);
        let right = at(&skeleton, &pose, placed, 6);
        assert!(
            (left.y - floor_y(left.x)).abs() < 2e-3,
            "uphill foot on the slope: {left}"
        );
        assert!(
            (right.y - floor_y(right.x)).abs() < 2e-3,
            "downhill foot on it: {right}"
        );
        let foot_up = turn_of(placed_joints(&skeleton, &pose, placed)[3]) * Vec3::Y;
        assert!(
            foot_up.angle_between(normal) < 1e-3,
            "the foot turned to the slope"
        );
        // Past its reach, a foot goes only as far as max_adjust.
        let mut steep = skeleton.rest_pose();
        let cliff = |from: Vec3, depth: f32| {
            let y = if from.x > 0.0 { 0.5 } else { 0.0 };
            (from.y - y <= depth && from.y >= y).then(|| (Vec3::new(from.x, y, from.z), Vec3::Y))
        };
        solve(&skeleton, &mut steep, Mat4::IDENTITY, &far, None, &cliff);
        let left = at(&skeleton, &steep, Mat4::IDENTITY, 3);
        assert!(
            left.y.abs() < 1e-3,
            "half a metre up is out of its reach: {left}"
        );
    }

    #[test]
    fn a_head_turns_to_look_and_no_further_than_its_limit() {
        let skeleton = legs();
        let look = Ik {
            look_at: LookAt {
                joint: "Head".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let head_facing = |pose: &[PoseTransform]| {
            turn_of(placed_joints(&skeleton, pose, Mat4::IDENTITY)[7]) * Vec3::Z
        };
        let mut pose = skeleton.rest_pose();
        let target = Vec3::new(3.0, 1.7, 3.0);
        solve(
            &skeleton,
            &mut pose,
            Mat4::IDENTITY,
            &look,
            Some(target),
            &no_ground,
        );
        let want = (target - Vec3::new(0.0, 1.7, 0.0)).normalize();
        assert!(head_facing(&pose).angle_between(want) < 1e-3, "looks at it");
        // Behind: as far round as the limit.
        let mut pose = skeleton.rest_pose();
        solve(
            &skeleton,
            &mut pose,
            Mat4::IDENTITY,
            &look,
            Some(Vec3::new(0.0, 1.7, -5.0)),
            &no_ground,
        );
        let off = head_facing(&pose).angle_between(Vec3::Z).to_degrees();
        assert!((off - 70.0).abs() < 0.1, "turned {off}°");
    }

    #[test]
    fn a_lines_ik_bends_its_skeleton_between_the_clips_and_the_skinning() {
        use crate::animator::{advance_animations_on, Animator};
        use crate::world::{Posed, WorldTransform};
        let scene: crate::scene::Scene = ron::from_str(
            r#"(entities: [(id: "0000000000000001", name: "hero",
                ik: (feet: (left: "LeftFoot", right: "RightFoot"), look_at: (joint: "Head", target: EntityRef("0000000000000002")))),
               (id: "0000000000000002", name: "bird", transform: (position: (0.0, 1.7, 5.0)))])"#,
        )
        .unwrap();
        let mut world = hecs::World::new();
        crate::world::spawn_scene_dressed(
            &scene,
            &mut world,
            &mut [Box::new(crate::motion::MotionDress)],
        );
        let ids = crate::world::addressable(&world);
        let hero = ids[&crate::id::EntityId::from_raw(1)];
        let bird = ids[&crate::id::EntityId::from_raw(2)];
        assert!(
            world.get::<&Ik>(hero).is_ok(),
            "the line's `ik` is on the entity"
        );
        let skeleton = std::sync::Arc::new(legs());
        world
            .insert(
                hero,
                (
                    Animator::new(skeleton.clone(), Default::default()),
                    WorldTransform(Mat4::IDENTITY),
                ),
            )
            .unwrap();
        world
            .insert_one(
                bird,
                WorldTransform(Mat4::from_translation(Vec3::new(0.0, 1.7, 5.0))),
            )
            .unwrap();
        let ground = plane(Vec3::new(0.0, 0.1, 0.0), Vec3::Y);
        advance_animations_on(&mut world, 1.0 / 60.0, &ground);
        assert!(
            world.get::<&LookingAt>(hero).is_ok(),
            "the look's target found and kept"
        );
        let posed = world.get::<&Posed>(hero).unwrap();
        // Skinning is place times inverse bind, and the binds here are
        // identity: the foot's place.
        let foot = posed.0[3].w_axis.truncate();
        assert!((foot.y - 0.1).abs() < 1e-3, "on the raised floor: {foot}");
    }

    #[test]
    fn a_joint_the_skeleton_does_not_have_is_said_with_the_nearest() {
        let mut wrong = ik();
        wrong.feet.left = "LeftFot".into();
        wrong.look_at.joint = "Hips".into();
        let problems = wrong.problems(&legs());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0]
                .contains("`LeftFot` is not a joint of the skeleton — did you mean `LeftFoot`?"),
            "{problems:?}"
        );
        let mut rootless = ik();
        rootless.feet.left = "Hips".into();
        assert!(rootless.problems(&legs())[0].contains("no knee and hip"));
    }
}
