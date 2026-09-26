//! Fluids on grids — the `fluid` module — where they meet the rest: the
//! physics' colliders are what they flow round (the soft module's
//! [`obstacles`](crate::soft::obstacles)), bodies that `floats` are held up
//! by the water through the physics ([`float`]), and the render draws them —
//! surfaces as live meshes, snow and sand as copies of a small cube
//! ([`show`], [`FluidLookDress`]).

pub use scrap_fluid::*;

use glam::{Mat4, Vec3};
use hecs::World;

use crate::material::Material;
use crate::render::MeshHandle;
use crate::scene::EntityDesc;
use crate::world::{Changed, Copies, Dress, LiveMesh, Surface, Unresolved, WorldTransform};

static CPU_SMOKE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether smokes are stepped by the renderer on the GPU (the default) —
/// the fixed step keeps only their clocks — or here on the CPU.
pub fn gpu_smoke() -> bool {
    !CPU_SMOKE.load(std::sync::atomic::Ordering::Relaxed) && std::env::var_os("SCRAP_CPU_SMOKE").is_none()
}

/// Step smokes on the CPU (`false`) or on the GPU where they are drawn:
/// for a tool or test that reads their grids.
pub fn set_gpu_smoke(on: bool) {
    CPU_SMOKE.store(!on, std::sync::atomic::Ordering::Relaxed);
}

/// Every fluid on by `seconds`: the module's fixed-step system.
pub fn step(world: &mut World, seconds: f32) {
    let mpm = world.query::<&MpmState>().iter().next().is_some();
    let heights = world.query::<&ShallowState>().iter().next().is_some()
        || world.query::<&RipplesState>().iter().next().is_some()
        || world.query::<&SnowState>().iter().next().is_some();
    let smoke = world.query::<&SmokeState>().iter().next().is_some();
    // What floats rides the water, it is not its bed.
    let obstacles = if mpm || heights || smoke {
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
    if smoke {
        if gpu_smoke() {
            scrap_fluid::count_smokes(world, seconds, &obstacles);
        } else {
            run_smokes(world, seconds, &obstacles);
        }
    }
    run_oceans(world, seconds);
}

/// A smoke the renderer steps: the box, its look, and the steps owed.
fn smoke_on_gpu(entity: hecs::Entity, state: &mut SmokeState) -> crate::volume::Smoke {
    let (solid, solid_version) = state.solid();
    let solid: Vec<u32> = solid.iter().map(|s| u32::from(*s)).collect();
    let n = [state.n[0] as u32, state.n[1] as u32, state.n[2] as u32];
    let stride = crate::volume::smoke_stride(n);
    let size = [n[0] / stride[0], n[1] / stride[1], n[2] / stride[2]];
    let s = state.smoke;
    let wind = Vec3::new(state.wind.direction.x, 0.0, state.wind.direction.z).normalize_or_zero() * state.wind.strength * 1.5;
    let (low, high) = state.bounds();
    let linear = |c: f32| crate::material::srgb_to_linear(c.clamp(0.0, 1.0));
    crate::volume::Smoke {
        low,
        high,
        size,
        cells: std::sync::Arc::new(Vec::new()),
        color: [linear(s.color[0]), linear(s.color[1]), linear(s.color[2])],
        density: 3.0,
        glow: if s.fire { 16.0 } else { 0.0 },
        gpu: Some(crate::volume::GpuSmoke {
            key: entity.to_bits().get(),
            n,
            dx: state.dx,
            source: s.source,
            rate: s.rate,
            heat: s.heat,
            weight: s.weight,
            curl: s.curl,
            fade: s.fade,
            wind,
            clock: state.clock(),
            solid: std::sync::Arc::new(solid),
            solid_version,
        }),
    }
}

/// A smoke's grid as the render's fog takes it: density and heat a byte
/// each, thinned to fit.
pub fn smoke_volume(state: &SmokeState) -> crate::volume::Smoke {
    let most = crate::volume::SMOKE_MOST;
    let n = state.n;
    // Every so many cells along each way, to fit.
    let stride = [(n[0] as u32).div_ceil(most[0]).max(1), (n[1] as u32).div_ceil(most[1]).max(1), (n[2] as u32).div_ceil(most[2]).max(1)];
    let size = [n[0] as u32 / stride[0], n[1] as u32 / stride[1], n[2] as u32 / stride[2]];
    // Heat settles far under what the source gives off: a fifth of it is
    // white-hot.
    let heat_most = (state.smoke.heat * 0.2).max(0.05);
    let mut cells = Vec::with_capacity((size[0] * size[1] * size[2]) as usize);
    for k in 0..size[2] as usize {
        for j in 0..size[1] as usize {
            for i in 0..size[0] as usize {
                let at = ((k * stride[2] as usize) * n[1] + j * stride[1] as usize) * n[0] + i * stride[0] as usize;
                let d = (state.density[at] / 3.0 * 255.0).clamp(0.0, 255.0) as u8;
                let h = (state.heat[at] / heat_most * 255.0).clamp(0.0, 255.0) as u8;
                cells.push([d, h, 0, 255]);
            }
        }
    }
    let (low, high) = state.bounds();
    let linear = |c: f32| crate::material::srgb_to_linear(c.clamp(0.0, 1.0));
    let c = state.smoke.color;
    crate::volume::Smoke {
        low,
        high,
        size,
        cells: std::sync::Arc::new(cells),
        color: [linear(c[0]), linear(c[1]), linear(c[2])],
        density: 3.0,
        glow: if state.smoke.fire { 16.0 } else { 0.0 },
        gpu: None,
    }
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
        world.query_mut::<(&mut MpmState, &WorldTransform, Option<&mut LiveMesh>, Option<&mut Copies>)>()
    {
        match (live, copies) {
            // Asleep, it looks as it did: nothing to make again.
            (_, Some(copies)) if state.asleep() && !copies.placed.is_empty() => {}
            (Some(live), None) if state.asleep() && !live.is_empty() => {}
            (_, Some(copies)) => copies.placed = state.grains_placed(),
            // As it was when its surface was last made (and that is still
            // what is shown): not made — nor uploaded — again.
            (Some(live), None) => {
                if let Some((vertices, indices)) = state.surface_if_moved(placed.0, live.is_empty()) {
                    live.set(vertices, indices);
                }
            }
            _ => {}
        }
    }
    for (state, placed, live) in world.query_mut::<(&ShallowState, &WorldTransform, &mut LiveMesh)>() {
        let (vertices, indices) = state.mesh(placed.0);
        live.set(vertices, indices);
    }
    for (state, placed, live) in world.query_mut::<(&SnowState, &WorldTransform, &mut LiveMesh)>() {
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
    let gpu = gpu_smoke();
    let smokes: Vec<(hecs::Entity, crate::volume::Smoke)> = world
        .query_mut::<(hecs::Entity, &mut SmokeState)>()
        .into_iter()
        .map(|(e, s)| (e, if gpu { smoke_on_gpu(e, s) } else { smoke_volume(s) }))
        .collect();
    for (entity, volume) in smokes {
        let _ = world.insert_one(entity, crate::world::SmokeVolume(volume));
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
        &["mpm", "shallow_water", "ripples", "ocean", "snow_cover", "model", "material"]
    }

    fn dress(&mut self, line: &EntityDesc, entity: hecs::Entity, world: &mut World, _: Changed, _: &mut Vec<Unresolved>) {
        let mpm = line.mpm();
        let surfaces = line.shallow_water().is_some() || line.ripples().is_some() || line.ocean().is_some() || line.snow_cover().is_some();
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
