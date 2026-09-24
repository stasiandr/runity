//! Fluids on grids — the `fluid` module — where they meet the rest: the
//! physics' colliders are what they flow round (the soft module's
//! [`obstacles`](crate::soft::obstacles)), bodies that `floats` are held up
//! by the water through the physics ([`float`]), and the render draws them —
//! surfaces as live meshes, snow and sand as copies of a small cube
//! ([`show`], [`FluidLookDress`]).

pub use runity_fluid::*;

use glam::{Mat4, Vec3};
use hecs::World;

use crate::material::Material;
use crate::render::MeshHandle;
use crate::scene::EntityDesc;
use crate::world::{Changed, Copies, Dress, LiveMesh, Surface, Unresolved, WorldTransform};

/// Every fluid on by `seconds`: the module's fixed-step system.
pub fn step(world: &mut World, seconds: f32) {
    let mpm = world.query::<&MpmState>().iter().next().is_some();
    let heights = world.query::<&ShallowState>().iter().next().is_some() || world.query::<&RipplesState>().iter().next().is_some();
    // What floats rides the water, it is not its bed.
    let obstacles = if mpm || heights {
        crate::soft::Obstacles::new(crate::soft::obstacles_but(world, |e| world.get::<&Floating>(e).is_ok()))
    } else {
        crate::soft::Obstacles::default()
    };
    if mpm {
        run_mpm(world, seconds, &obstacles);
    }
    if heights {
        run_heightfields(world, seconds, &obstacles);
    }
    run_oceans(world, seconds);
}

/// Hold up every body that `floats` by the water under it. Call before the
/// physics' step: the forces are for it.
#[cfg(feature = "physics")]
pub fn float(world: &World, physics: &mut crate::PhysicsWorld) {
    let floating: Vec<(hecs::Entity, Floating, Mat4)> = world
        .query::<(hecs::Entity, &Floating, &WorldTransform)>()
        .iter()
        .map(|(e, f, p)| (e, *f, p.0))
        .collect();
    for (entity, floating, placed) in floating {
        let Some(mass) = physics.mass(world, entity) else { continue };
        let height = placed.transform_vector3(Vec3::Y).length();
        let points = Floating::points(placed);
        let forces = floating.forces(
            mass,
            height,
            &points,
            |p| water_height(world, p),
            |p| physics.velocity_at(world, entity, p).unwrap_or(Vec3::ZERO),
        );
        for (at, force) in forces {
            physics.add_force_at(world, entity, force, at);
        }
    }
}

/// What each fluid looks like now, into what the render draws.
pub fn show(world: &mut World, _seconds: f32) {
    for (state, placed, live, copies) in
        world.query_mut::<(&MpmState, &WorldTransform, Option<&mut LiveMesh>, Option<&mut Copies>)>()
    {
        match (live, copies) {
            (_, Some(copies)) => copies.placed = state.grains_placed(),
            (Some(live), None) => {
                let (vertices, indices) = state.surface(placed.0);
                live.set(vertices, indices);
            }
            _ => {}
        }
    }
    for (state, placed, live) in world.query_mut::<(&ShallowState, &WorldTransform, &mut LiveMesh)>() {
        let (vertices, indices) = state.mesh(placed.0);
        live.set(vertices, indices);
    }
    for (state, placed, live) in world.query_mut::<(&RipplesState, &WorldTransform, &mut LiveMesh)>() {
        let (vertices, indices) = state.mesh(placed.0);
        live.set(vertices, indices);
    }
    for (state, placed, live) in world.query_mut::<(&OceanState, &WorldTransform, &mut LiveMesh)>() {
        let (vertices, indices) = state.mesh(placed.0);
        live.set(vertices, indices);
    }
}

/// What marks an entity drawn by this dresser, so taking the field off
/// takes off only what it put on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FluidLook;

/// How a fluid looks ([`crate::world::Dress`]): water and jelly as their
/// surface, snow and sand as their grains — copies of a small cube — in
/// the line's material. After the look's dresser.
pub struct FluidLookDress<'a> {
    /// `builtin:cube`, found before the look's dresser took the resolver.
    pub cube: Option<MeshHandle>,
    pub palette: &'a dyn Fn(&crate::AssetLink) -> Option<Material>,
}

impl Dress for FluidLookDress<'_> {
    fn parts(&self) -> &[&'static str] {
        &["mpm", "shallow_water", "ripples", "ocean", "model", "material"]
    }

    fn dress(&mut self, line: &EntityDesc, entity: hecs::Entity, world: &mut World, _: Changed, _: &mut Vec<Unresolved>) {
        let mpm = line.mpm();
        let surfaces = line.shallow_water().is_some() || line.ripples().is_some() || line.ocean().is_some();
        if mpm.is_none() && !surfaces {
            if world.remove_one::<FluidLook>(entity).is_ok() {
                let _ = world.remove::<(LiveMesh, Copies)>(entity);
            }
            return;
        }
        let _ = world.remove_one::<crate::world::Model>(entity);
        let _ = world.insert(entity, (FluidLook, Surface(line.material_from(self.palette))));
        let grains = mpm.is_some_and(|m| matches!(m.material, MpmMaterial::Snow | MpmMaterial::Sand));
        if grains {
            let _ = world.remove_one::<LiveMesh>(entity);
            if let Some(mesh) = self.cube {
                let _ = world.insert_one(entity, Copies { mesh, placed: Vec::new() });
            }
        } else {
            let _ = world.remove_one::<Copies>(entity);
            if world.get::<&LiveMesh>(entity).is_err() {
                let _ = world.insert_one(entity, LiveMesh::new(Vec::new(), Vec::new()));
            }
        }
    }
}

use crate::prelude::*;

#[cfg(all(test, feature = "physics"))]
mod tests {
    use super::*;
    use crate::scene::Scene;

    #[test]
    fn a_crate_floats_on_a_pond_and_snow_falls_as_grains() {
        let scene: Scene = ron::from_str(
            r#"(entities: [
                (name: "ground", transform: (scale: (20.0, 1.0, 20.0)), body: Static, collider: Box(half: (0.5, 0.05, 0.5), center: (0.0, -0.05, 0.0))),
                (name: "pond", material: (base_color: (0.1, 0.3, 0.4)), shallow_water: (size: (6.0, 6.0), cells: 30, depth: 1.0)),
                (name: "crate", model: "builtin:cube", transform: (position: (0.0, 2.0, 0.0), scale: (0.8, 0.8, 0.8)),
                 body: Dynamic, collider: Box(half: (0.5, 0.5, 0.5)), floats: (share: 0.4)),
                (name: "snow", transform: (position: (8.0, 0.0, 0.0)), mpm: (material: Snow, size: (0.3, 0.3, 0.3), domain: (1.0, 1.0, 1.0), resolution: 12.0)),
            ])"#,
        )
        .unwrap();
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        crate::world::apply_hierarchy(&mut world);
        let mut physics = crate::PhysicsWorld::new(1.0 / 60.0);
        physics.sync_from_world(&mut world);
        for _ in 0..240 {
            step(&mut world, 1.0 / 60.0);
            float(&world, &mut physics);
            physics.run(&mut world);
        }
        show(&mut world, 0.0);
        let crate_ = world.query::<(&Floating, &WorldTransform)>().iter().map(|(_, p)| p.0.w_axis.truncate()).next().unwrap();
        // Floating 0.4 under: its middle about 0.1 × 0.8 above the water at
        // 1 m, not on the floor.
        assert!((crate_.y - 1.08).abs() < 0.12, "floats: {crate_}");
        let pond = world.query::<(&ShallowState, &LiveMesh)>().iter().map(|(_, l)| l.vertices().len()).next().unwrap();
        assert!(pond > 0);
        let grains = world.query::<(&MpmState, &Copies)>().iter().map(|(s, c)| (s.grains.len(), c.placed.len())).next().unwrap();
        assert!(grains.0 > 0 && grains.0 == grains.1);
    }
}
