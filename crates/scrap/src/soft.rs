//! Soft things — the `soft` module's ropes, cables, chains and cloth —
//! where they meet the modules they do not know: the physics' colliders
//! become what they lie on ([`obstacles`]), and the render draws them — a
//! rope's tube and a cloth as live meshes, a chain as copies of
//! `builtin:link` ([`show`]) — in the entity's material
//! ([`SoftLookDress`]).

pub use scrap_soft::*;

use glam::Vec3;
use hecs::World;

use crate::material::Material;
use crate::render::MeshHandle;
use crate::scene::EntityDesc;
use crate::world::{Changed, Copies, Dress, LiveMesh, Surface, Unresolved, WorldTransform};

/// The ground under a point for the feet of animated skeletons
/// (`scrap_animation::ik`): the scene's solid colliders as soft things
/// meet them, less the character's own — the entity with the IK and those
/// it hangs from, whose capsule its feet are inside. Straight down from
/// `from` at most `depth`, in 5 cm steps to the first solid, then halved
/// to a millimetre: a step's top, not the far side of it.
#[cfg(feature = "animation")]
pub fn feet_ground(world: &World) -> scrap_animation::ik::Probe {
    use crate::world::Parent;
    let mut own = std::collections::HashSet::new();
    for (entity, _) in world
        .query::<(hecs::Entity, &scrap_animation::ik::Ik)>()
        .iter()
    {
        let mut at = Some(entity);
        while let Some(e) = at.filter(|e| own.insert(*e)) {
            at = world.get::<&Parent>(e).ok().map(|p| p.0);
        }
    }
    let all = Obstacles::new(obstacles_but(world, |e| own.contains(&e)));
    Box::new(move |from: Vec3, depth: f32| {
        let mut near = Vec::new();
        all.near(
            from - Vec3::Y * (depth + 0.1),
            from + Vec3::splat(0.1),
            &mut near,
        );
        let solid = |p: Vec3| near.iter().find_map(|o| o.contact(p, 0.0));
        if near.is_empty() || solid(from).is_some() {
            return None;
        }
        let steps = (depth / 0.05).ceil().max(1.0) as usize;
        let mut above = from;
        for i in 1..=steps {
            let below = from - Vec3::Y * (depth * i as f32 / steps as f32);
            if solid(below).is_some() {
                let (mut hi, mut lo) = (above, below);
                while hi.y - lo.y > 1e-3 {
                    let mid = (hi + lo) * 0.5;
                    if solid(mid).is_some() {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let normal = solid(lo).map_or(Vec3::Y, |(n, _)| n);
                return Some((hi, normal));
            }
            above = below;
        }
        None
    })
}

/// The physics' solid colliders, as what soft things lie on and wrap
/// round. Triggers are solid to nothing, and a model's own triangles are
/// left out: a soft thing meets primitives.
pub fn obstacles(world: &World) -> Vec<Obstacle> {
    obstacles_but(world, |_| false)
}

/// [`obstacles`], leaving out the entities `skip` says: water leaves out
/// what floats on it.
pub fn obstacles_but(world: &World, skip: impl Fn(hecs::Entity) -> bool) -> Vec<Obstacle> {
    tagged_obstacles_but(world, skip).0
}

/// [`obstacles_but`], each with the bits of the entity it is (0 for the
/// distance fields): what a rope leaves out of what it meets.
pub fn tagged_obstacles_but(world: &World, skip: impl Fn(hecs::Entity) -> bool) -> (Vec<Obstacle>, Vec<u64>) {
    let (mut out, mut tags) = primitives(world, |e, _| skip(e));
    // The scene's distance fields: its meshes, baked.
    for field in world.query::<&DistanceFieldState>().iter().filter_map(|f| f.baked.clone()) {
        out.push(Obstacle::Field(field));
        tags.push(0);
    }
    (out, tags)
}

/// The colliders that are simple shapes, but those `skip` says.
fn primitives(world: &World, skip: impl Fn(hecs::Entity, Option<crate::body::Body>) -> bool) -> (Vec<Obstacle>, Vec<u64>) {
    use crate::bodies::{Physics, Shape};
    use crate::body::{Body, Collider};
    let mut out = Vec::new();
    let mut tags = Vec::new();
    for (entity, shape, placed, body) in world
        .query::<(hecs::Entity, &Shape, &WorldTransform, Option<&Physics>)>()
        .iter()
    {
        if matches!(body, Some(Physics(Body::Trigger))) || skip(entity, body.map(|b| b.0)) {
            continue;
        }
        let placed = placed.0;
        let (scale, turn, at) = placed.to_scale_rotation_translation();
        let wide = scale.abs().max_element();
        let before = out.len();
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
        tags.resize(before, 0);
        tags.resize(out.len(), entity.to_bits().get());
    }
    (out, tags)
}

/// Metres either side of a surface the render's copy of a distance field
/// tells apart.
const FIELD_RANGE: f32 = 2.0;

/// Bake each distance field not yet baked, from what stands still: the
/// physics' simple shapes and its mesh colliders. The render gets it too,
/// when the line asks.
pub fn bake_fields(world: &mut World) {
    use crate::bodies::Physics;
    use crate::body::Body;
    let waiting: Vec<(hecs::Entity, DistanceField, glam::Mat4)> = world
        .query::<(hecs::Entity, &DistanceFieldState, &WorldTransform)>()
        .iter()
        .filter(|(_, s, _)| s.baked.is_none())
        .map(|(e, s, placed)| (e, s.field, placed.0))
        .collect();
    if waiting.is_empty() {
        return;
    }
    let moves = |b: Option<Body>| matches!(b, Some(Body::Dynamic | Body::Kinematic));
    let still = primitives(world, |_, body| moves(body)).0;
    let meshes: Vec<(crate::physics::CollisionMesh, glam::Mat4)> = world
        .query::<(&crate::physics::CollisionMesh, &WorldTransform, Option<&Physics>)>()
        .iter()
        .filter(|(_, _, body)| !moves(body.map(|b| b.0)))
        .map(|(mesh, placed, _)| (mesh.clone(), placed.0))
        .collect();
    let meshes: Vec<WorldMesh> = meshes
        .iter()
        .map(|(m, placed)| WorldMesh { vertices: &m.vertices, triangles: &m.triangles, placed: *placed })
        .collect();
    for (entity, field, placed) in waiting {
        let sdf = std::sync::Arc::new(bake(&field, placed, &still, &meshes));
        if field.draw {
            let [x, y, z] = sdf.size;
            let look = crate::distance::DistanceField {
                low: sdf.low,
                high: sdf.high(),
                size: [x as u32, y as u32, z as u32],
                range: FIELD_RANGE,
                cells: std::sync::Arc::new(sdf.bytes(FIELD_RANGE)),
            };
            let _ = world.insert_one(entity, crate::world_look::DistanceFieldLook(look));
        }
        if let Ok(mut state) = world.get::<&mut DistanceFieldState>(entity) {
            state.baked = Some(sdf);
        }
    }
}

/// Every soft thing on by `seconds`, lying on the physics' colliders: the
/// module's fixed-step system.
pub fn step(world: &mut World, seconds: f32) {
    bake_fields(world);
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
    // Where everything is before it moves: what went through what.
    let starts = frame_starts(world);
    let (solid, tags) = tagged_obstacles_but(world, |_| false);
    // Kept for the second grid only when there are cloths to add to it.
    let for_sheets = cloth.then(|| (solid.clone(), tags.clone()));
    let obstacles = Obstacles::tagged(solid, tags);
    if cloth {
        run_cloth(world, seconds, &obstacles);
    }
    if hair {
        run_hair(world, seconds, &obstacles);
    }
    // The rest meet the cloths as they now are, within their own steps.
    // Without cloths it is the same grid, lent rather than copied.
    let sheets;
    let with_sheets = match for_sheets {
        Some((mut all, tags)) => {
            all.extend(sheet_obstacles(world));
            sheets = Obstacles::tagged(all, tags);
            &sheets
        }
        None => &obstacles,
    };
    if ropes {
        run_ropes(world, seconds, with_sheets);
    }
    if bodies {
        run_soft_bodies(world, seconds, with_sheets);
    }
    if fluids {
        run_fluids(world, seconds, with_sheets);
    }
    if grains {
        run_grains(world, seconds, with_sheets);
    }
    // Then each kind against the others: one solver's contacts.
    run_contacts(world, &starts, &obstacles);
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
            // Asleep, it looks as it did: nothing to make again.
            (_, Some(copies)) if state.asleep() && !copies.placed.is_empty() => {}
            (Some(live), None) if state.asleep() && !live.is_empty() => {}
            (_, Some(copies)) => copies.placed = state.drops(),
            (Some(live), None) => {
                let (vertices, indices) = state.surface(placed.0);
                if !vertices.is_empty() {
                    live.set(vertices, indices);
                }
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
    #[cfg(feature = "animation")]
    fn feet_find_a_crates_top_and_look_through_their_own_capsule() {
        let scene: Scene = ron::from_str(
            r#"(entities: [
                (name: "crate", transform: (position: (0.0, 0.5, 0.0)),
                 body: Static, collider: Box(half: (0.5, 0.5, 0.5))),
                (name: "slope", transform: (position: (5.0, 0.0, 0.0), rotation_deg: (0.0, 0.0, 20.0)),
                 body: Static, collider: Box(half: (2.0, 0.5, 2.0))),
                (name: "hero", transform: (position: (0.0, 1.0, 0.0)), body: Kinematic,
                 collider: Capsule(half_height: 0.6, radius: 0.3),
                 ik: (feet: (left: "LeftFoot", right: "RightFoot"))),
            ])"#,
        )
        .unwrap();
        let mut world = World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        crate::world::apply_hierarchy(&mut world);
        let ground = feet_ground(&world);
        let (top, up) = ground(Vec3::new(0.1, 1.4, 0.0), 0.8).expect("the crate, through the hero");
        assert!((top.y - 1.0).abs() < 2e-3 && up.y > 0.99, "{top} {up}");
        let (_, tilted) = ground(Vec3::new(5.0, 1.5, 0.0), 2.0).expect("the slope");
        assert!(
            (tilted.angle_between(Vec3::Y).to_degrees() - 20.0).abs() < 1.0,
            "{tilted}"
        );
        assert!(
            ground(Vec3::new(20.0, 1.0, 0.0), 0.8).is_none(),
            "nothing there"
        );
    }

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
        set_wind(&mut world, scrap_core::wind::Wind { direction: Vec3::X, strength: 0.0 });
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
