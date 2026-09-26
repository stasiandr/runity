//! Characters — the `character` module — where they meet the physics and
//! the render: a ragdoll's muscles pull its parts through the physics each
//! step, PD controllers toward the pose it wants, its pelvis held up and
//! after its entity ([`step`]); a crawler's feet go down on the colliders
//! ([`crawl`]); both are drawn as capsules in their line's material
//! ([`show`], [`CharacterLookDress`]).

pub use scrap_character::*;

use glam::{Mat4, Quat, Vec3};
use hecs::World;

use crate::material::Material;
use crate::render::MeshHandle;
use crate::scene::EntityDesc;
use crate::world::{Changed, Copies, Dress, Surface, Unresolved, WorldTransform};

/// How fast a part was going at the end of the last step: a sudden change
/// is a blow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartWas(pub Vec3);

/// Every ragdoll's muscles pulled for the coming step, and the blows of
/// the last knocked in. Call before the physics' step.
#[cfg(feature = "physics")]
pub fn step(world: &mut World, physics: &mut crate::PhysicsWorld, seconds: f32) {
    let fresh = world.query::<&RagdollState>().iter().any(|s| s.parts.is_empty());
    // Blows: the pelvis, chest or head whose speed jumped more than its
    // owner can take — not a foot meeting the ground as it walks.
    let jumps: Vec<(hecs::Entity, f32)> = world
        .query::<(hecs::Entity, &RagdollPart, Option<&PartWas>)>()
        .iter()
        .filter(|(_, part, _)| part.index <= 2)
        .filter_map(|(e, part, was)| {
            let now = physics.velocity(world, e)?;
            Some((part.owner, was.map_or(0.0, |w| (now - w.0).length())))
        })
        .collect();
    for (owner, jump) in jumps {
        if let Ok(mut s) = world.get::<&mut RagdollState>(owner) {
            s.strike(jump);
        }
    }
    let targets = targets(world, seconds);
    if fresh {
        physics.sync_from_world(world);
    }
    for t in targets {
        if t.pull <= 0.0 || t.parts.len() != 11 {
            continue;
        }
        // Someone else's: its parts are shown as their owner has them.
        if t.parts.iter().any(|p| world.get::<&crate::world::Replica>(*p).is_ok()) {
            continue;
        }
        let mut total = 0.0;
        for (i, part) in t.parts.iter().enumerate() {
            let Ok(current) = world.get::<&WorldTransform>(*part).map(|p| p.0.to_scale_rotation_translation().1) else { continue };
            let mass = physics.mass(world, *part).unwrap_or(1.0);
            total += mass;
            let inertia = physics.inertia(world, *part).map_or(mass * 0.05, |v| v.max_element());
            let mut error = t.turns[i] * current.inverse();
            if error.w < 0.0 {
                error = -error;
            }
            let (axis, angle) = error.to_axis_angle();
            let spin = physics.spin(world, *part).unwrap_or(Vec3::ZERO);
            let parent = scrap_character::body::PARTS[i].parent.map(|p| t.parts[p]);
            let parent_spin = parent.and_then(|p| physics.spin(world, p)).unwrap_or(Vec3::ZERO);
            // A muscle: a spring toward the pose, a damper against turning
            // away from the parent; the pelvis, with no parent, held by
            // the hand of God.
            // Stable at the physics' step: kd · dt well under 1.
            let (kp, kd) = if parent.is_some() { (300.0, 20.0) } else { (500.0, 30.0) };
            let torque = (axis * angle * kp - (spin - parent_spin) * kd) * inertia * t.pull;
            physics.add_torque(world, *part, torque);
            if let Some(parent) = parent {
                physics.add_torque(world, parent, -torque);
            }
            // The chest kept upright in the world too: what keeps a walker
            // from folding over its hips.
            if i == 1 {
                let upright = (axis * angle * 200.0 - spin * 16.0) * inertia * t.pull;
                physics.add_torque(world, *part, upright);
            }
        }
        // The chest held over the pelvis by a spring too: what lifts a
        // body that is sitting or lying back onto its feet.
        let chest = t.parts[1];
        let parts = &scrap_character::body::PARTS;
        let scale = (t.pelvis.y / parts[0].joint.y).max(0.1);
        let chest_goal = t.pelvis
            + t.turns[0] * ((parts[1].joint - parts[0].joint) * scale)
            + t.turns[1] * (((parts[1].from + parts[1].to) * 0.5 - parts[1].joint) * scale);
        if let (Ok(at), Some(v), Some(m)) = (
            world.get::<&WorldTransform>(chest).map(|p| p.0.w_axis.truncate()),
            physics.velocity(world, chest),
            physics.mass(world, chest),
        ) {
            let force = ((chest_goal - at) * 80.0 - v * 12.0) * (m * 2.0) * t.pull;
            physics.add_force(world, chest, force);
        }
        // The pelvis held up and after its entity: a spring toward where it
        // should be, with the body's weight taken.
        let pelvis = t.parts[0];
        if let (Ok(at), Some(v)) = (world.get::<&WorldTransform>(pelvis).map(|p| p.0.w_axis.truncate()), physics.velocity(world, pelvis)) {
            let force = ((t.pelvis - at) * 100.0 - v * 14.0 + Vec3::Y * 9.81) * total * t.pull;
            physics.add_force(world, pelvis, force);
        }
    }
    // What each part is doing now, for the next step's blows — only of
    // parts simulated here: one just taken over from another peer starts
    // at the speed it had there, which is no blow.
    let now: Vec<(hecs::Entity, Option<Vec3>)> = world
        .query::<(hecs::Entity, &RagdollPart, Option<&crate::world::Replica>)>()
        .iter()
        .map(|(e, _, replica)| (e, if replica.is_some() { None } else { physics.velocity(world, e) }))
        .collect();
    for (e, v) in now {
        match v {
            Some(v) => {
                let _ = world.insert_one(e, PartWas(v));
            }
            None => {
                let _ = world.remove_one::<PartWas>(e);
            }
        }
    }
}

/// Every crawler on by `seconds`, its feet going down on the colliders
/// under them.
pub fn crawl(world: &mut World, seconds: f32) {
    if world.query::<&CrawlerState>().iter().next().is_none() {
        return;
    }
    let obstacles = crate::soft::obstacles(world);
    let ground = |p: Vec3| {
        let solid = |y: f32| obstacles.iter().any(|o| o.contact(Vec3::new(p.x, y, p.z), 0.0).is_some());
        let (mut lo, mut hi) = (p.y - 6.0, p.y + 1.5);
        if solid(hi) {
            return hi;
        }
        if !solid(lo) {
            return lo.max(0.0);
        }
        for _ in 0..20 {
            let mid = (lo + hi) * 0.5;
            if solid(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    };
    for (state, placed) in world.query_mut::<(&mut CrawlerState, &WorldTransform)>() {
        state.advance(placed.0, ground, seconds);
    }
}

/// A capsule from `a` to `b`, `radius` thick, for `builtin:capsule`.
fn capsule(a: Vec3, b: Vec3, radius: f32) -> Mat4 {
    let length = a.distance(b);
    let turn = Quat::from_rotation_arc(Vec3::Y, (b - a).normalize_or(Vec3::Y));
    Mat4::from_scale_rotation_translation(Vec3::new(radius * 2.0, (length + 2.0 * radius) * 0.5, radius * 2.0), turn, (a + b) * 0.5)
}

/// Ragdolls and crawlers as capsules, into what the render draws.
pub fn show(world: &mut World, _seconds: f32) {
    let ragdolls: Vec<(hecs::Entity, Vec<Mat4>)> =
        world.query::<(hecs::Entity, &RagdollState)>().iter().map(|(e, s)| (e, capsules(world, s))).collect();
    for (e, placed) in ragdolls {
        if let Ok(mut copies) = world.get::<&mut Copies>(e) {
            copies.placed = placed;
        }
    }
    for (state, copies) in world.query_mut::<(&CrawlerState, &mut Copies)>() {
        let r = state.crawler.thickness * 0.5;
        copies.placed = state.bones().into_iter().map(|(a, b)| capsule(a, b, r)).collect();
    }
}

/// How ragdolls and crawlers are drawn ([`crate::world::Dress`]): copies of
/// `builtin:capsule` in the line's material — a ragdoll's parts instead of
/// the entity's model (the entity is only its goal), a crawler's legs
/// beside its body.
pub struct CharacterLookDress<'a> {
    pub capsule: Option<MeshHandle>,
    pub palette: &'a dyn Fn(&crate::AssetLink) -> Option<Material>,
}

impl Dress for CharacterLookDress<'_> {
    fn parts(&self) -> &[&'static str] {
        &["ragdoll", "crawler", "model", "material"]
    }

    fn dress(&mut self, line: &EntityDesc, entity: hecs::Entity, world: &mut World, _: Changed, _: &mut Vec<Unresolved>) {
        use crate::prelude::*;
        let (ragdoll, crawler) = (line.ragdoll(), line.crawler());
        if ragdoll.is_none() && crawler.is_none() {
            return;
        }
        let Some(mesh) = self.capsule else { return };
        if ragdoll.is_some() {
            scrap_core::world::take_off::<crate::world::Model>(world, entity);
            let _ = world.insert_one(entity, Surface(line.material_from(self.palette)));
            let _ = world.insert_one(entity, Copies { mesh, placed: Vec::new() });
        } else {
            // The legs in the body's material; the body its model.
            if world.get::<&Surface>(entity).is_err() {
                let _ = world.insert_one(entity, Surface(line.material_from(self.palette)));
            }
            let _ = world.insert_one(entity, Copies { mesh, placed: Vec::new() });
        }
    }
}

#[cfg(all(test, feature = "physics"))]
mod tests {
    use super::*;
    use crate::scene::Scene;

    fn run(scene: &str, seconds: f32, mut each: impl FnMut(&mut World, &mut crate::PhysicsWorld, usize)) -> World {
        let scene: Scene = ron::from_str(scene).unwrap();
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        crate::world::apply_hierarchy(&mut world);
        let mut physics = crate::PhysicsWorld::new(1.0 / 60.0);
        physics.sync_from_world(&mut world);
        for frame in 0..(seconds * 60.0) as usize {
            crate::routes::run_routes(&mut world, 1.0 / 60.0);
            crate::world::apply_hierarchy(&mut world);
            step(&mut world, &mut physics, 1.0 / 60.0);
            physics.run(&mut world);
            crawl(&mut world, 1.0 / 60.0);
            each(&mut world, &mut physics, frame);
        }
        show(&mut world, 0.0);
        world
    }

    fn head(world: &World) -> Vec3 {
        let s = world.query::<&RagdollState>().iter().next().map(|s| s.parts[2]).unwrap();
        world.get::<&WorldTransform>(s).unwrap().0.w_axis.truncate()
    }

    const GROUND: &str = r#"(name: "ground", transform: (scale: (40.0, 1.0, 40.0)), body: Static, collider: Box(half: (0.5, 0.05, 0.5), center: (0.0, -0.05, 0.0)))"#;

    #[test]
    fn a_limp_ragdoll_falls_down_and_an_active_one_stands() {
        let limp = run(&format!("(entities: [{GROUND}, (name: \"guy\", ragdoll: (mode: Limp))])"), 3.0, |_, _, _| {});
        assert!(head(&limp).y < 0.5, "the limp one lies down: {}", head(&limp));
        let active = run(&format!("(entities: [{GROUND}, (name: \"guy\", ragdoll: (mode: Active))])"), 3.0, |_, _, _| {});
        assert!(head(&active).y > 1.4, "the active one stands: {}", head(&active));
        let copies = active.query::<&Copies>().iter().map(|c| c.placed.len()).next();
        assert_eq!(copies, Some(11));
    }

    #[test]
    fn an_active_ragdoll_walks_after_its_goal_and_is_knocked_down_by_a_blow() {
        let walked = run(
            &format!("(entities: [{GROUND}, (name: \"guy\", ragdoll: (drive: Walk), route: (points: [(0.0, 0.0, 0.0), (0.0, 0.0, 4.0)], speed: 1.2, ends: Stop))])"),
            5.0,
            |_, _, _| {},
        );
        let h = head(&walked);
        assert!(h.z > 3.0 && h.y > 1.3, "walked after it, upright: {h}");
        // Knocked with a shove: down, and back up after a few seconds.
        let mut lowest = f32::MAX;
        let knocked = run(&format!("(entities: [{GROUND}, (name: \"guy\", ragdoll: (recover: 2.0))])"), 6.0, |world, physics, frame| {
            if frame == 60 {
                let chest = world.query::<&RagdollState>().iter().next().map(|s| s.parts[1]).unwrap();
                physics.set_velocity(world, chest, Vec3::new(8.0, 0.0, 0.0));
            }
            if frame > 60 {
                lowest = lowest.min(head(world).y);
            }

        });
        assert!(lowest < 1.0, "knocked down: {lowest}");
        assert!(head(&knocked).y > 1.3, "and up again: {}", head(&knocked));
    }

    #[test]
    fn a_crawler_walks_over_a_crate_its_feet_on_the_colliders() {
        let world = run(
            &format!("(entities: [{GROUND},
                (name: \"crate\", transform: (position: (0.0, 0.2, 2.0), scale: (2.0, 0.4, 1.0)), body: Static, collider: Box(half: (0.5, 0.5, 0.5))),
                (name: \"spider\", transform: (position: (0.0, 0.8, 0.0)), crawler: (), route: (points: [(0.0, 0.0, 0.0), (0.0, 0.0, 4.0)], speed: 1.0, ends: Stop))])"),
            5.0,
            |_, _, _| {},
        );
        let state = world.query::<&CrawlerState>().iter().next().map(|s| (s.stepped, s.feet.clone())).unwrap();
        assert!(state.0 > 20, "stepped: {}", state.0);
        for f in &state.1 {
            let on_crate = f.x.abs() < 1.0 && (f.z - 2.0).abs() < 0.5;
            let expected = if on_crate { 0.4 } else { 0.0 };
            assert!((f.y - expected).abs() < 0.02, "foot at {f}");
        }
    }
}
