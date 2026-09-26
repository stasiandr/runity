//! Destruction — the `destruction` module — where it meets the physics
//! and the render: the blows it breaks and dents by are read off the
//! physics' contacts ([`blows`]), the pieces become bodies knocked away and
//! are drawn in the material of what they broke off, what broke gives off
//! its own particles as debris, and a dented surface is drawn as it is
//! bent ([`step`], [`show`], [`DestructionLookDress`]).

pub use scrap_destruction::*;

use glam::Vec3;
use hecs::World;

use crate::material::Material;
use crate::scene::EntityDesc;
use crate::world::{Changed, Dress, LiveMesh, Surface, Unresolved, WorldTransform};

/// How fast a body was going at the end of the last step: what it struck
/// with, before the step that struck bounced it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moving(pub Vec3);

/// The blows of the last physics step: each thing that can break or dent
/// and each body that came to touch it, at the speed they met at, where
/// on its box it was struck and which way.
#[cfg(feature = "physics")]
pub fn blows(world: &World, physics: &crate::PhysicsWorld) -> Vec<Blow> {
    let mut out = Vec::new();
    for (entity, contacts, placed) in world
        .query::<(hecs::Entity, &crate::physics::Contacts, &WorldTransform)>()
        .with::<hecs::Or<&Breakable, &Dented>>()
        .iter()
    {
        let own = physics.velocity(world, entity).unwrap_or(Vec3::ZERO);
        for other in contacts.entered.iter().chain(&contacts.inside) {
            let Ok(there) = world.get::<&WorldTransform>(*other) else { continue };
            let theirs = world.get::<&Moving>(*other).map(|m| m.0).unwrap_or(Vec3::ZERO);
            let speed = (theirs - own).length();
            // Where on its box: the nearest point to the other's middle.
            let back = placed.0.inverse();
            let local = back.transform_point3(there.0.w_axis.truncate());
            let at = placed.0.transform_point3(local.clamp(Vec3::splat(-0.5), Vec3::splat(0.5)));
            let way = if theirs.length() > 1e-3 { theirs - own } else { at - there.0.w_axis.truncate() };
            out.push(Blow { entity, speed, at, way });
        }
    }
    out
}

/// Everything that can break or dent on by `seconds`, struck by what the
/// physics' last step brought together; the pieces made bodies of it,
/// knocked away. Call after the physics' step.
#[cfg(feature = "physics")]
pub fn step(world: &mut World, physics: &mut crate::PhysicsWorld, seconds: f32) {
    let blows = blows(world, physics);
    run_dents(world, &blows);
    let broken = run_fracture(world, seconds, &blows);
    let knocked = dress_broken(world, &broken);
    #[cfg(feature = "net")]
    crate::netsim::own_pieces(world, &knocked);
    if !knocked.is_empty() {
        physics.sync_from_world(world);
        for (piece, velocity) in knocked {
            physics.set_velocity(world, piece, velocity);
        }
    }
    // What each body is doing now, for the next step's blows.
    let moving: Vec<(hecs::Entity, Vec3)> = world
        .query::<(hecs::Entity, &crate::bodies::Physics)>()
        .iter()
        .filter(|(_, p)| p.0 == crate::body::Body::Dynamic)
        .filter_map(|(e, _)| physics.velocity(world, e).map(|v| (e, v)))
        .collect();
    for (e, v) in moving {
        let _ = world.insert_one(e, Moving(v));
    }
}

/// What broke put away, its debris given off, and its pieces given its
/// look: returns the pieces and how each should be moving.
pub fn dress_broken(world: &mut World, broken: &Broken) -> Vec<(hecs::Entity, Vec3)> {
    for thing in &broken.things {
        // Its particles, if it has any, left where it stood and played:
        // the debris of its breaking.
        let debris = world
            .get::<&crate::particles::Emitting>(*thing)
            .ok()
            .map(|e| (*e).clone())
            .zip(world.get::<&WorldTransform>(*thing).ok().map(|p| *p));
        if let Some((mut emitting, placed)) = debris {
            emitting.play();
            world.spawn((emitting, placed));
        }
        crate::world_core::set_active(world, *thing, false);
    }
    for (piece, _) in &broken.pieces {
        let (mesh, from) = match world.get::<&Piece>(*piece) {
            Ok(p) => (LiveMesh::new(p.vertices.clone(), p.indices.clone()), p.from),
            Err(_) => continue,
        };
        let surface = world.get::<&Surface>(from).map(|s| *s).unwrap_or(Surface(Material::default()));
        let _ = world.insert(*piece, (mesh, surface));
        #[cfg(feature = "physics")]
        let _ = world.insert_one(*piece, crate::physics::Contacts::default());
    }
    broken.pieces.clone()
}

/// A dented surface drawn as it is bent.
pub fn show(world: &mut World, _seconds: f32) {
    for (dented, live) in world.query_mut::<(&mut Dented, &mut LiveMesh)>() {
        if dented.fresh {
            live.set(dented.vertices.clone(), dented.indices.clone());
            dented.fresh = false;
        }
    }
}

/// How what breaks and dents is dressed ([`crate::world::Dress`]): each
/// asks the physics for its contacts, and a dented surface is drawn from
/// its own mesh instead of its model. After the look's dresser.
pub struct DestructionLookDress;

impl Dress for DestructionLookDress {
    fn parts(&self) -> &[&'static str] {
        &["fracture", "dents", "model"]
    }

    fn dress(&mut self, line: &EntityDesc, entity: hecs::Entity, world: &mut World, _: Changed, _: &mut Vec<Unresolved>) {
        let (fracture, dents) = (line.fracture(), line.dents());
        #[cfg(feature = "physics")]
        if fracture.is_some() || dents.is_some() {
            let _ = world.insert_one(entity, crate::physics::Contacts::default());
        }
        let _ = fracture;
        if dents.is_some() && world.get::<&Dented>(entity).is_ok() {
            scrap_core::world::take_off::<crate::world::Model>(world, entity);
            if let Ok(mut d) = world.get::<&mut Dented>(entity) {
                d.fresh = true;
            }
            if world.get::<&LiveMesh>(entity).is_err() {
                let _ = world.insert_one(entity, LiveMesh::new(Vec::new(), Vec::new()));
            }
        }
    }
}

#[cfg(all(test, feature = "physics"))]
mod tests {
    use super::*;
    use crate::render::MeshHandle;
    use crate::scene::Scene;

    #[test]
    fn a_ball_thrown_at_a_wall_breaks_it_and_one_thrown_at_a_car_dents_it() {
        let scene: Scene = ron::from_str(
            r#"(entities: [
                (name: "ground", transform: (scale: (20.0, 1.0, 20.0)), body: Static, collider: Box(half: (0.5, 0.05, 0.5), center: (0.0, -0.05, 0.0))),
                (name: "wall", model: "builtin:cube", transform: (position: (0.0, 1.0, 0.0), scale: (3.0, 2.0, 0.3)),
                 material: "builtin:stone", body: Static, collider: Box(half: (0.5, 0.5, 0.5)),
                 fracture: (pieces: 10, strength: 5.0)),
                (name: "ball", model: "builtin:sphere", transform: (position: (0.0, 1.0, 3.0)),
                 body: Dynamic, collider: Sphere(radius: 0.5), physics: (density: 20.0)),
                (name: "car", model: "builtin:cube", transform: (position: (6.0, 0.6, 0.0), scale: (2.0, 1.0, 4.0)),
                 body: Static, collider: Box(half: (0.5, 0.5, 0.5)), dents: (depth: 0.1)),
                (name: "rock", model: "builtin:sphere", transform: (position: (6.0, 4.0, 0.5), scale: (0.4, 0.4, 0.4)),
                 body: Dynamic, collider: Sphere(radius: 0.5), physics: (density: 10.0)),
            ])"#,
        )
        .unwrap();
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        crate::world::apply_hierarchy(&mut world);
        let mut physics = crate::PhysicsWorld::new(1.0 / 60.0);
        physics.sync_from_world(&mut world);
        let ball = world.query::<(hecs::Entity, &crate::world_core::SceneId)>().iter().nth(2).map(|(e, _)| e).unwrap();
        physics.set_velocity(&world, ball, Vec3::new(0.0, 0.0, -12.0));
        let car = world.query::<(hecs::Entity, &Dented)>().iter().next().map(|(e, _)| e).unwrap();
        let before = world.get::<&Dented>(car).unwrap().vertices.clone();
        for _ in 0..90 {
            physics.run(&mut world);
            step(&mut world, &mut physics, 1.0 / 60.0);
            show(&mut world, 0.0);
        }
        let pieces = world.query::<&Piece>().iter().count();
        assert_eq!(pieces, 10, "the wall broke");
        let wall = world.query::<(hecs::Entity, &Breakable)>().iter().find(|(_, b)| b.broken).map(|(e, _)| e).unwrap();
        assert!(world.get::<&crate::world_core::Inactive>(wall).is_ok(), "and is put away");
        // The pieces are drawn in the wall's stone and fell.
        let stone = crate::material::builtin::by_name("stone").unwrap();
        for (surface, placed, _) in world.query::<(&Surface, &WorldTransform, &Piece)>().iter() {
            assert_eq!(surface.0, stone);
            assert!(placed.0.w_axis.y < 2.2);
        }
        let dented = world.get::<&Dented>(car).unwrap();
        assert!(dented.blows >= 1, "the rock dented the car");
        assert_ne!(before, dented.vertices);
        assert!(!world.get::<&LiveMesh>(car).unwrap().vertices().is_empty());
    }
}
