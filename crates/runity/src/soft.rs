//! Soft things — the `soft` module's ropes, cables, chains and cloth —
//! where they meet the modules they do not know: the physics' colliders
//! become what they lie on ([`obstacles`]), and the render draws them — a
//! rope's tube and a cloth as live meshes, a chain as copies of
//! `builtin:link` ([`show`]) — in the entity's material
//! ([`SoftLookDress`]).

pub use runity_soft::*;

use glam::Vec3;
use hecs::World;

use crate::material::Material;
use crate::render::MeshHandle;
use crate::scene::EntityDesc;
use crate::world::{Changed, Copies, Dress, LiveMesh, Surface, Unresolved, WorldTransform};

/// The physics' solid colliders, as what soft things lie on and wrap
/// round. Triggers are solid to nothing, and a model's own triangles are
/// left out: a soft thing meets primitives.
pub fn obstacles(world: &World) -> Vec<Obstacle> {
    obstacles_but(world, |_| false)
}

/// [`obstacles`], leaving out the entities `skip` says: water leaves out
/// what floats on it.
pub fn obstacles_but(world: &World, skip: impl Fn(hecs::Entity) -> bool) -> Vec<Obstacle> {
    use crate::bodies::{Physics, Shape};
    use crate::body::{Body, Collider};
    let mut out = Vec::new();
    for (entity, shape, placed, body) in world
        .query::<(hecs::Entity, &Shape, &WorldTransform, Option<&Physics>)>()
        .iter()
    {
        if matches!(body, Some(Physics(Body::Trigger))) || skip(entity) {
            continue;
        }
        let placed = placed.0;
        let (scale, turn, at) = placed.to_scale_rotation_translation();
        let wide = scale.abs().max_element();
        match shape.0 {
            Collider::Box { half, center } => out.push(Obstacle::placed_box(placed, center, half)),
            Collider::Ramp { half } => out.push(Obstacle::placed_ramp(placed, half)),
            Collider::Stairs { half, .. } => out.push(Obstacle::placed_ramp(placed, half)),
            Collider::Sphere { radius } => out.push(Obstacle::Sphere { center: at, radius: radius * wide }),
            Collider::Capsule { half_height, radius } | Collider::Cylinder { half_height, radius } => {
                let up = turn * Vec3::Y * (half_height * scale.y.abs());
                out.push(Obstacle::Capsule {
                    a: at - up,
                    b: at + up,
                    radius: radius * scale.x.abs().max(scale.z.abs()),
                })
            }
            Collider::None | Collider::Model => {}
        }
    }
    out
}

/// Every soft thing on by `seconds`, lying on the physics' colliders: the
/// module's fixed-step system.
pub fn step(world: &mut World, seconds: f32) {
    let ropes = world.query::<&RopeState>().iter().next().is_some();
    let cloth = world.query::<&ClothState>().iter().next().is_some();
    let hair = world.query::<&HairState>().iter().next().is_some();
    let bodies = world.query::<&SoftBodyState>().iter().next().is_some();
    let fluids = world.query::<&FluidState>().iter().next().is_some();
    let grains = world.query::<&GrainsState>().iter().next().is_some();
    // Jiggle bones meet nothing: they go first, and what hangs off them —
    // hair on a jiggling head — goes where they have gone.
    run_jiggle(world, seconds);
    if !ropes && !cloth && !hair && !bodies && !fluids && !grains {
        return;
    }
    let obstacles = Obstacles::new(obstacles(world));
    if ropes {
        run_ropes(world, seconds, &obstacles);
    }
    if cloth {
        run_cloth(world, seconds, &obstacles);
    }
    if hair {
        run_hair(world, seconds, &obstacles);
    }
    if bodies {
        run_soft_bodies(world, seconds, &obstacles);
    }
    if fluids {
        run_fluids(world, seconds, &obstacles);
    }
    if grains {
        run_grains(world, seconds, &obstacles);
    }
}

/// Where each soft thing is, into what the render draws: a rope's tube
/// into its live mesh, a chain's links into its copies.
pub fn show(world: &mut World, _seconds: f32) {
    for (state, placed, live, copies) in
        world.query_mut::<(&RopeState, &WorldTransform, Option<&mut LiveMesh>, Option<&mut Copies>)>()
    {
        if state.points().is_empty() {
            continue;
        }
        if let Some(copies) = copies {
            copies.placed = state.links();
        } else if let Some(live) = live {
            let (vertices, indices) = state.tube(placed.0, TUBE_SIDES, TUBE_RINGS);
            live.set(vertices, indices);
        }
    }
    for (state, placed, live) in world.query_mut::<(&ClothState, &WorldTransform, &mut LiveMesh)>() {
        let (vertices, indices) = state.mesh(placed.0);
        live.set(vertices, indices);
    }
    for (state, placed, live) in world.query_mut::<(&HairState, &WorldTransform, &mut LiveMesh)>() {
        let (vertices, indices) = state.mesh(placed.0);
        live.set(vertices, indices);
    }
    for (state, placed, live, copies) in
        world.query_mut::<(&FluidState, &WorldTransform, Option<&mut LiveMesh>, Option<&mut Copies>)>()
    {
        match (live, copies) {
            (_, Some(copies)) => copies.placed = state.drops(),
            (Some(live), None) => {
                let (vertices, indices) = state.surface(placed.0);
                live.set(vertices, indices);
            }
            _ => {}
        }
    }
    for (state, copies) in world.query_mut::<(&GrainsState, &mut Copies)>() {
        copies.placed = state.placed_grains();
    }
    for (state, placed, live) in world.query_mut::<(&SoftBodyState, &WorldTransform, &mut LiveMesh)>() {
        let (vertices, indices) = state.mesh(placed.0);
        if !vertices.is_empty() {
            live.set(vertices, indices);
        }
    }
}

/// Sides round a rope's tube, and rings along each link of it.
const TUBE_SIDES: usize = 8;
const TUBE_RINGS: usize = 3;

/// What an entity shows its rope or cloth with: put on by
/// [`SoftLookDress`], so that taking them off takes off only what it put
/// on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RopeLook;

/// How a rope looks ([`crate::world::Dress`]): a tube drawn from a live
/// mesh, or a chain's links as copies of its line's model — `builtin:link`
/// when it names none — in the line's material. It runs after the look's
/// own dresser, which takes the surface off a line with no model and puts
/// a chain's model on it to be drawn once.
pub struct SoftLookDress<'a> {
    /// `builtin:link` and `builtin:sphere`, found before the look's
    /// dresser took the resolver.
    pub link: Option<MeshHandle>,
    pub sphere: Option<MeshHandle>,
    pub palette: &'a dyn Fn(&crate::AssetLink) -> Option<Material>,
}

impl Dress for SoftLookDress<'_> {
    fn parts(&self) -> &[&'static str] {
        &["rope", "cloth", "hair", "soft_body", "fluid", "grains", "model", "material"]
    }

    fn dress(&mut self, line: &EntityDesc, entity: hecs::Entity, world: &mut World, _: Changed, _: &mut Vec<Unresolved>) {
        use crate::prelude::*;
        let (rope, cloth, hair, body) = (line.rope(), line.cloth(), line.hair(), line.soft_body());
        let fluid = line.fluid();
        if line.grains().is_some() {
            // Each grain is a small ball in the line's material.
            let _ = world.remove::<(crate::world::Model, LiveMesh)>(entity);
            let _ = world.insert(entity, (RopeLook, Surface(line.material_from(self.palette))));
            if let Some(mesh) = self.sphere {
                let _ = world.insert_one(entity, Copies { mesh, placed: Vec::new() });
            }
            return;
        }
        if let Some(fluid) = fluid {
            // Water is its surface or its drops, not the model it names.
            let _ = world.remove_one::<crate::world::Model>(entity);
            let _ = world.insert(entity, (RopeLook, Surface(line.material_from(self.palette))));
            if fluid.look == FluidLook::Drops {
                let _ = world.remove_one::<LiveMesh>(entity);
                if let Some(mesh) = self.sphere {
                    let _ = world.insert_one(entity, Copies { mesh, placed: Vec::new() });
                }
            } else {
                let _ = world.remove_one::<Copies>(entity);
                if world.get::<&LiveMesh>(entity).is_err() {
                    let _ = world.insert_one(entity, LiveMesh::new(Vec::new(), Vec::new()));
                }
            }
            return;
        }
        if body.is_some() {
            // The model itself is soft: drawn deformed, not as it is.
            let _ = world.remove_one::<crate::world::Model>(entity);
        }
        if rope.is_none() && cloth.is_none() && hair.is_none() && body.is_none() {
            if world.remove_one::<RopeLook>(entity).is_ok() {
                let _ = world.remove::<(LiveMesh, Copies)>(entity);
            }
            return;
        }
        // Hair is drawn in its line's material, like the rest: a head of
        // hair is a line of its own, a child of the head, so that it and
        // the head are each their own colour.
        let _ = world.insert(entity, (RopeLook, Surface(line.material_from(self.palette))));
        if rope.is_some_and(|r| r.kind == RopeKind::Chain) && cloth.is_none() && hair.is_none() && body.is_none() {
            let _ = world.remove_one::<LiveMesh>(entity);
            // The line's model is its link, drawn at each link and not
            // once at the entity.
            let own = world.remove_one::<crate::world::Model>(entity).ok().map(|m| m.0);
            match own.or(self.link) {
                Some(mesh) => {
                    let placed = world.get::<&Copies>(entity).map(|c| c.placed.clone()).unwrap_or_default();
                    let _ = world.insert_one(entity, Copies { mesh, placed });
                }
                None => {
                    let _ = world.remove_one::<Copies>(entity);
                }
            }
        } else {
            let _ = world.remove_one::<Copies>(entity);
            if world.get::<&LiveMesh>(entity).is_err() {
                let _ = world.insert_one(entity, LiveMesh::new(Vec::new(), Vec::new()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Scene;

    #[test]
    fn a_rope_on_a_line_hangs_over_a_crate_and_is_drawn_in_its_material() {
        let scene: Scene = ron::from_str(
            r#"(entities: [
                (name: "line", transform: (position: (-2.0, 2.0, 0.0)),
                 material: "builtin:bark", rope: (to: (4.0, 0.0, 0.0), slack: 0.45, segments: 40)),
                (name: "crate", transform: (position: (0.0, 0.5, 0.0)),
                 model: "builtin:cube", body: Static, collider: Box(half: (0.5, 0.5, 0.5))),
                (name: "chain", transform: (position: (3.0, 3.0, 0.0)),
                 rope: (to: (0.0, -1.0, 0.0), ends: Start, kind: Chain, segments: 8)),
                (name: "flag", transform: (position: (-3.0, 3.0, 0.0)), cloth: (pinned: Left)),
            ])"#,
        )
        .unwrap();
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        crate::world::apply_hierarchy(&mut world);
        set_wind(&mut world, runity_core::wind::Wind { direction: Vec3::X, strength: 0.0 });
        for _ in 0..90 {
            step(&mut world, 1.0 / 30.0);
        }
        show(&mut world, 0.0);
        let (line, middle) = world
            .query::<(hecs::Entity, &RopeState)>()
            .iter()
            .find(|(_, s)| s.rope.kind == RopeKind::Rope)
            .map(|(e, s)| (e, s.points()[s.points().len() / 2]))
            .unwrap();
        // A rope 45% longer than 4 m would sag 1.6 m in its middle, to 0.4
        // above the ground — through the crate, had it started there; it
        // falls onto the crate's top instead, which holds it up at 1 m.
        assert!(middle.x.abs() < 0.5 && (1.0..1.05).contains(&middle.y), "lies on the crate: {middle}");
        let live = world.get::<&LiveMesh>(line).unwrap();
        assert!(!live.vertices().is_empty());
        let bark = crate::material::builtin::by_name("bark").unwrap();
        assert_eq!(world.get::<&Surface>(line).unwrap().0, bark);
        let links = world.query::<&Copies>().iter().map(|c| (c.mesh, c.placed.len())).next();
        let flag = world.query::<(hecs::Entity, &ClothState)>().iter().next().map(|(e, _)| e).unwrap();
        assert!(!world.get::<&LiveMesh>(flag).unwrap().vertices().is_empty());
        assert_eq!(links, Some((MeshHandle::TEST, 8)));
        // The frame draws the crate, and the chain's links as eight copies
        // of the link — not the link once at the chain's entity.
        let frame = crate::world::build_frame(&world, Default::default(), Default::default(), Default::default());
        assert_eq!(frame.draws.len(), 1 + 8);
        assert_eq!(frame.live_meshes.len(), 2);
    }
}
