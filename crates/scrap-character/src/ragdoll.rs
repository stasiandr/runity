//! Ragdolls: a line's `ragdoll`. `ragdoll: (mode: Active, drive: Walk)` on
//! an entity puts a person made of physics where it stands: eleven
//! capsules — pelvis, chest, head, arms and forearms, thighs and shins —
//! joined at the hips, knees, shoulders, elbows, neck and waist (knees and
//! elbows as hinges).
//!
//! * `Limp`: a ragdoll — it falls, tumbles and lies as a body would.
//! * `Active`: an active ragdoll, physics-based animation — every joint
//!   pulled by a spring and a damper (a PD controller, a muscle) toward the
//!   pose its `drive` asks for, and its pelvis held up and after the
//!   entity by a stronger one (the "hand of God" active-ragdoll games
//!   use). The entity is the goal: move it by a route or by the game, and
//!   the body walks after it. Struck hard it goes limp, and over
//!   `recover` seconds finds its feet again.
//!
//! What it tries for (`drive`): `Stand`; `Walk`, the procedural gait at the
//! speed the entity goes; `Match`, motion matching over the gait's frames
//! ([`crate::matching`]) — the pose motion matching picks, held to by
//! physics. `reach: EntityRef(…)` bends the left arm by IK to reach for
//! another entity.
//!
//! The module says what to push and where; the facade pushes through the
//! physics ([`targets`]).

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use scrap_core::scene::Transform;
use scrap_core::world::{SceneId, WorldTransform};
use scrap_core::{EntityId, EntityRef};
use scrap_physics::bodies::{Jointed, Physics, Props, Shape};
use scrap_physics::body::{Body, BodyProps, Collider, Joint};

use crate::body::{gait, Pose, FOREARM, PARTS, PELVIS, UPPER_ARM};
use crate::ik::two_bone;
use crate::matching::{Database, Matcher};

/// A ragdoll, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ragdoll {
    pub mode: Mode,
    pub drive: Drive,
    /// How tall, metres.
    pub height: f32,
    /// How strong its muscles are, 0 to 1.
    pub strength: f32,
    /// Metres a second a blow must land at to knock it limp.
    pub toughness: f32,
    /// Seconds it takes to find its feet after.
    pub recover: f32,
    /// What its left hand reaches for.
    #[serde(skip_serializing_if = "unlinked")]
    pub reach: EntityRef,
    /// How it goes over the network (docs/netsim.md): `Local` unless
    /// the line says.
    #[serde(default, skip_serializing_if = "scrap_core::netsim::NetMode::is_local")]
    pub net: scrap_core::netsim::NetMode,
}

fn unlinked(r: &EntityRef) -> bool {
    r.0.is_none()
}

/// Whether a ragdoll holds itself up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mode {
    Limp,
    #[default]
    Active,
}

/// What an active ragdoll tries for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Drive {
    #[default]
    Stand,
    Walk,
    Match,
}

impl Default for Ragdoll {
    fn default() -> Self {
        Self { mode: Mode::Active, drive: Drive::Stand, height: 1.8, strength: 1.0, toughness: 5.0, recover: 3.0, reach: EntityRef::default(), net: scrap_core::netsim::NetMode::Local }
    }
}

scrap_core::impl_parts! {
    Ragdoll => "ragdoll", fractions ["strength"];
}

/// The ragdoll of a line, read off it.
pub trait RagdollLine {
    fn ragdoll(&self) -> Option<Ragdoll>;
}

impl RagdollLine for scrap_core::EntityDesc {
    fn ragdoll(&self) -> Option<Ragdoll> {
        self.part()
    }
}

impl RagdollLine for scrap_core::scene::Override {
    fn ragdoll(&self) -> Option<Ragdoll> {
        self.part()
    }
}

/// A part of a ragdoll: whose, and which.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RagdollPart {
    pub owner: hecs::Entity,
    pub index: usize,
}

/// A ragdoll as it moves: on the entity that is its goal.
#[derive(Debug, Clone)]
pub struct RagdollState {
    pub ragdoll: Ragdoll,
    /// Its parts, once put in the world.
    pub parts: Vec<hecs::Entity>,
    /// How much its muscles pull now, 0 (knocked limp) to 1.
    pub active: f32,
    phase: f32,
    speed: f32,
    facing: Quat,
    was: Option<Vec3>,
    matcher: Matcher,
    database: std::sync::Arc<Database>,
    /// The pose it is trying for now.
    pub pose: Pose,
}

impl RagdollState {
    pub fn new(ragdoll: Ragdoll) -> Self {
        Self {
            active: if ragdoll.mode == Mode::Active { 1.0 } else { 0.0 },
            ragdoll,
            parts: Vec::new(),
            phase: 0.0,
            speed: 0.0,
            facing: Quat::IDENTITY,
            was: None,
            matcher: Matcher::default(),
            database: std::sync::Arc::new(Database::from_gait(&[0.0, 0.7, 1.3, 2.0, 3.0], 30.0)),
            pose: Pose::default(),
        }
    }

    fn scale(&self) -> f32 {
        self.ragdoll.height.max(0.3) / 1.8
    }

    /// Knocked limp by a blow at `speed`, if it is hard enough.
    pub fn strike(&mut self, speed: f32) {
        if speed >= self.ragdoll.toughness {
            self.active = 0.0;
        }
    }
}

/// The id a ragdoll's part goes by, from its owner's and its index: what
/// its joint names its parent by.
fn part_id(owner: EntityId, index: usize) -> EntityId {
    EntityId::from_raw(owner.raw() ^ (0x9e37_79b9_7f4a_7c15u64.wrapping_mul(index as u64 + 1)) | 1 << 62)
}

/// Put a ragdoll's parts in the world, standing at `placed`: dynamic
/// bodies, each a capsule, joined to its parent. `id` is its owner's.
pub fn spawn_parts(world: &mut hecs::World, owner: hecs::Entity, id: EntityId, placed: Mat4, scale: f32) -> Vec<hecs::Entity> {
    let (_, turn, at) = placed.to_scale_rotation_translation();
    let turn = Quat::from_rotation_y(turn.to_euler(glam::EulerRot::YXZ).0);
    PARTS
        .iter()
        .enumerate()
        .map(|(i, part)| {
            let middle = (part.from + part.to) * 0.5 * scale;
            let length = part.from.distance(part.to) * scale;
            let place = at + turn * middle;
            let mut transform = Transform { position: place, ..Transform::default() };
            transform.set_rotation(turn);
            let shape = Shape(Collider::Capsule { half_height: (length * 0.5).max(0.01), radius: part.radius * scale });
            let joint = part.parent.map(|p| {
                // Where it joins its parent, in its own space.
                let anchor = part.joint * scale - middle;
                let to = part_id(id, p);
                if part.hinge {
                    Joint::Hinge { to, anchor, axis: Vec3::X, limits_deg: None, motor: None }
                } else {
                    Joint::Ball { to, anchor }
                }
            });
            let entity = world.spawn((
                transform,
                WorldTransform(Mat4::from_rotation_translation(turn, place)),
                Physics(Body::Dynamic),
                shape,
                SceneId(part_id(id, i)),
                RagdollPart { owner, index: i },
                // Flesh: about as dense as water.
                Props(BodyProps { density: 1000.0, ..BodyProps::default() }),
            ));
            if let Some(joint) = joint {
                let _ = world.insert_one(entity, Jointed(joint));
            }
            entity
        })
        .collect()
}

/// What a ragdoll's muscles are after this step: each part's turn in the
/// world, where its pelvis should be, and how hard to pull (0 to 1).
#[derive(Debug, Clone, PartialEq)]
pub struct Targets {
    pub parts: Vec<hecs::Entity>,
    pub turns: [Quat; 11],
    pub pelvis: Vec3,
    pub pull: f32,
}

/// Every ragdoll's goal read off its entity, its drive stepped on by
/// `seconds`, and its parts put in the world if they are not: what the
/// facade pulls toward. `reach` finds where a linked entity is.
pub fn targets(world: &mut hecs::World, seconds: f32) -> Vec<Targets> {
    // Parts first, for any ragdoll without.
    let fresh: Vec<(hecs::Entity, EntityId, Mat4, f32)> = world
        .query::<(hecs::Entity, &RagdollState, &WorldTransform, Option<&SceneId>)>()
        .iter()
        .filter(|(_, s, _, _)| s.parts.is_empty())
        .map(|(e, s, p, id)| (e, id.map_or(EntityId::from_raw(e.to_bits().get()), |i| i.0), p.0, s.scale()))
        .collect();
    for (owner, id, placed, scale) in fresh {
        let parts = spawn_parts(world, owner, id, placed, scale);
        // Over the network (docs/netsim.md): a `Full` ragdoll's parts are
        // bodies of the network's, driven by whoever drives it and taken
        // with it; any other's are every peer's own.
        let full = world.get::<&RagdollState>(owner).is_ok_and(|s| s.ragdoll.net == scrap_core::netsim::NetMode::Full);
        if full {
            let ids = (0..parts.len()).map(|i| part_id(id, i)).collect();
            let _ = world.insert_one(owner, scrap_core::netsim::Tied(ids));
        } else {
            for part in &parts {
                let _ = world.insert_one(*part, scrap_core::netsim::Unshared);
            }
        }
        if let Ok(mut s) = world.get::<&mut RagdollState>(owner) {
            s.parts = parts;
        }
    }
    let reaches: Vec<(hecs::Entity, Option<Vec3>)> = world
        .query::<(hecs::Entity, &RagdollState)>()
        .iter()
        .map(|(e, s)| (e, s.ragdoll.reach))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(e, r)| (e, r.get(world).and_then(|t| world.get::<&WorldTransform>(t).ok().map(|p| p.0.w_axis.truncate()))))
        .collect();
    let mut out = Vec::new();
    for (owner, reach) in reaches {
        let Ok(placed) = world.get::<&WorldTransform>(owner).map(|p| p.0) else { continue };
        let Ok(mut s) = world.get::<&mut RagdollState>(owner) else { continue };
        let goal = placed.w_axis.truncate();
        // How fast, and which way, the goal goes.
        if let Some(was) = s.was {
            if seconds > 0.0 {
                let v = (goal - was) / seconds;
                let level = Vec3::new(v.x, 0.0, v.z);
                s.speed = s.speed + (level.length() - s.speed) * 0.2;
                if level.length() > 0.2 {
                    let want = Quat::from_rotation_y(level.x.atan2(level.z));
                    s.facing = s.facing.slerp(want, 0.15);
                }
            }
        } else {
            let (_, turn, _) = placed.to_scale_rotation_translation();
            s.facing = Quat::from_rotation_y(turn.to_euler(glam::EulerRot::YXZ).0);
        }
        s.was = Some(goal);
        // Its muscles come back after a blow, or go if it is limp.
        let want = if s.ragdoll.mode == Mode::Active { 1.0 } else { 0.0 };
        s.active = if want > s.active { (s.active + seconds / s.ragdoll.recover.max(0.1)).min(want) } else { want };
        let speed = s.speed;
        let mut pose = match s.ragdoll.drive {
            Drive::Stand => Pose::default(),
            Drive::Walk => {
                let stride = crate::body::stride(speed);
                s.phase = (s.phase + seconds * speed / stride).fract();
                gait(speed, s.phase)
            }
            Drive::Match => {
                let db = s.database.clone();
                s.matcher.advance(&db, speed, seconds)
            }
        };
        let scale = s.scale();
        let pelvis = goal + Vec3::Y * (PARTS[PELVIS].joint.y * scale);
        // The left arm reaches by IK for what it is linked to.
        if let Some(target) = reach {
            let world_pose = pose.world(pelvis, s.facing, scale);
            let side = 0;
            let (chest_turn, _) = world_pose[crate::body::CHEST];
            let shoulder = world_pose[UPPER_ARM[side]].1;
            let (upper, lower) = (
                PARTS[UPPER_ARM[side]].joint.distance(PARTS[FOREARM[side]].joint) * scale,
                PARTS[FOREARM[side]].joint.distance(PARTS[FOREARM[side]].to) * scale,
            );
            let pole = shoulder + s.facing * Vec3::new(-0.3, -0.6, -0.4);
            let (elbow, hand) = two_bone(shoulder, upper, lower, target, pole);
            // The arm's rest runs straight down: each bone turned from down
            // to where it points now, relative to its parent.
            let upper_world = Quat::from_rotation_arc(Vec3::NEG_Y, (elbow - shoulder).normalize_or(Vec3::NEG_Y));
            let fore_world = Quat::from_rotation_arc(Vec3::NEG_Y, (hand - elbow).normalize_or(Vec3::NEG_Y));
            pose.0[UPPER_ARM[side]] = chest_turn.inverse() * upper_world;
            pose.0[FOREARM[side]] = upper_world.inverse() * fore_world;
        }
        s.pose = pose;
        let world_pose = pose.world(pelvis, s.facing, scale);
        out.push(Targets { parts: s.parts.clone(), turns: world_pose.map(|(t, _)| t), pelvis, pull: s.active * s.ragdoll.strength.clamp(0.0, 1.0) });
    }
    out
}

/// A capsule at each part, in the world, for `builtin:capsule`: how a
/// ragdoll is drawn.
pub fn capsules(world: &hecs::World, state: &RagdollState) -> Vec<Mat4> {
    let scale = state.scale();
    state
        .parts
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let placed = world.get::<&WorldTransform>(*e).ok()?.0;
            let part = PARTS[i];
            let length = part.from.distance(part.to) * scale;
            let r = part.radius * scale;
            // builtin:capsule is 2 m tall and 1 across.
            let size = Vec3::new(r * 2.0, (length + 2.0 * r) * 0.5, r * 2.0);
            let (_, turn, at) = placed.to_scale_rotation_translation();
            Some(Mat4::from_scale_rotation_translation(size, turn, at))
        })
        .collect()
}

/// The character module's dresser for ragdolls: the state on the entity;
/// its parts come on its first step.
pub struct RagdollDress;

impl scrap_core::world::Dress for RagdollDress {
    fn parts(&self) -> &[&'static str] {
        &["ragdoll"]
    }

    fn dress(
        &mut self,
        line: &scrap_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: scrap_core::world::Changed,
        _: &mut Vec<scrap_core::world::Unresolved>,
    ) {
        // A ragdoll dressed again starts again: its old parts go.
        if let Ok(old) = world.get::<&RagdollState>(entity).map(|s| s.parts.clone()) {
            for part in old {
                let _ = world.despawn(part);
            }
        }
        match line.ragdoll() {
            Some(r) => {
                let _ = world.insert_one(entity, RagdollState::new(r));
            }
            None => {
                scrap_core::world::take_off::<RagdollState>(world, entity);
            }
        }
    }
}

/// The character module's dresser for crawlers.
pub struct CrawlerDress;

impl scrap_core::world::Dress for CrawlerDress {
    fn parts(&self) -> &[&'static str] {
        &["crawler"]
    }

    fn dress(
        &mut self,
        line: &scrap_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: scrap_core::world::Changed,
        _: &mut Vec<scrap_core::world::Unresolved>,
    ) {
        use crate::crawler::{CrawlerLine, CrawlerState};
        match line.crawler() {
            Some(c) => {
                let _ = world.insert_one(entity, CrawlerState::new(c));
            }
            None => {
                scrap_core::world::take_off::<CrawlerState>(world, entity);
            }
        }
    }
}


/// A ragdoll's state for the network (`Components::register_state`): how
/// much its muscles pull — knocked limp or back on its feet — from its
/// owner, when it is `Full`. Its parts go as the bodies they are.
pub fn gather_net(world: &hecs::World, entity: hecs::Entity) -> Option<Vec<u8>> {
    let state = world.get::<&RagdollState>(entity).ok()?;
    if state.ragdoll.net != scrap_core::netsim::NetMode::Full || world.get::<&scrap_core::world::Replica>(entity).is_ok() {
        return None;
    }
    // In tenths: a figure that changes while it recovers, not every step.
    Some(vec![(state.active.clamp(0.0, 1.0) * 10.0).round() as u8])
}

/// The owner's figure onto everyone else's.
pub fn take_net(world: &mut hecs::World, entity: hecs::Entity, _sender: u32, _tick: u64, bytes: &[u8]) {
    if let (Ok(mut state), Some(b)) = (world.get::<&mut RagdollState>(entity), bytes.first()) {
        state.active = *b as f32 / 10.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ragdoll_puts_eleven_jointed_capsules_where_it_stands_and_asks_for_its_pose() {
        let mut world = hecs::World::new();
        let owner = world.spawn((
            WorldTransform(Mat4::from_translation(Vec3::new(2.0, 0.0, 1.0))),
            SceneId(EntityId::from_raw(42)),
            RagdollState::new(Ragdoll { drive: Drive::Walk, ..Ragdoll::default() }),
        ));
        let targets = targets(&mut world, 1.0 / 60.0);
        assert_eq!(targets.len(), 1);
        let parts = &world.get::<&RagdollState>(owner).unwrap().parts.clone();
        assert_eq!(parts.len(), 11);
        // The head above the pelvis above the feet, where the owner stands.
        let at = |i: usize| world.get::<&WorldTransform>(parts[i]).unwrap().0.w_axis.truncate();
        assert!(at(2).y > at(0).y && at(0).y > at(8).y);
        assert!((at(0).x - 2.0).abs() < 1e-4 && (at(0).z - 1.0).abs() < 1e-4);
        // Each joined to its parent by the parent's id.
        let joined = parts.iter().filter(|p| world.get::<&Jointed>(**p).is_ok()).count();
        assert_eq!(joined, 10);
        let pelvis_id = world.get::<&SceneId>(parts[0]).unwrap().0;
        match world.get::<&Jointed>(parts[1]).unwrap().0 {
            Joint::Ball { to, .. } => assert_eq!(to, pelvis_id),
            other => panic!("{other:?}"),
        }
        assert!((targets[0].pelvis.y - 0.97).abs() < 1e-4);
        assert_eq!(targets[0].pull, 1.0);
        // Struck hard: limp, and it comes back over its recover time.
        world.get::<&mut RagdollState>(owner).unwrap().strike(9.0);
        let t = super::targets(&mut world, 1.0);
        assert!(t[0].pull > 0.0 && t[0].pull < 0.5, "{}", t[0].pull);
        let capsules = capsules(&world, &world.get::<&RagdollState>(owner).unwrap());
        assert_eq!(capsules.len(), 11);
    }
}
