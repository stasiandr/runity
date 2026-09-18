//! Villagers who decide what to do, find their way there, and carry it home.
//!
//! ```text
//! cargo run --release --example foragers
//! RUNITY_HEADLESS=1 cargo run --release --example foragers   # writes a PNG
//! ```
//!
//! Controls: arrows or WASD orbit, Q/E zoom, P pauses the world, 1–8 debug
//! views, Escape quits.
//!
//! This is the whole simulation stack in one scene. The land comes from noise;
//! the navigation grid is costed from the same noise, so what the AI walks on
//! and what you see cannot drift apart. Villagers score their options rather
//! than following a script, path around the rocks, steer out of each other's
//! way, and carry what they gather back to the storehouse.
//!
//! Note where each piece of work happens: decisions run on the world clock,
//! ten times a second, while movement runs on the frame. A faster machine
//! draws a smoother walk, not a faster village.

use runity::prelude::*;
use std::collections::HashMap;

const AREA: f32 = 44.0;
const CELL: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    /// Looking for food.
    Forage,
    /// Looking for wood.
    Chop,
    /// Taking what they carry back to the storehouse.
    Deliver,
    /// Nothing worth doing; standing about near home.
    Idle,
}

impl Job {
    fn colour(self) -> Color {
        match self {
            Job::Forage => Color::rgb(0.20, 0.70, 0.22),
            Job::Chop => Color::rgb(0.80, 0.35, 0.10),
            Job::Deliver => Color::rgb(0.95, 0.80, 0.15),
            Job::Idle => Color::rgb(0.55, 0.57, 0.62),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Wood,
    Food,
}

/// Something that can be gathered.
#[derive(Debug, Clone, Copy)]
struct Resource {
    kind: Kind,
    remaining: f32,
}

/// Where something is, on the ground.
#[derive(Debug, Clone, Copy)]
struct At(Vec3);

/// Somebody who works.
struct Villager {
    job: Job,
    carrying: f32,
    kind: Kind,
    velocity: Vec3,
    facing: Vec3,
    follower: Option<PathFollower>,
    target: Option<Entity>,
    /// Ticks spent going nowhere, so a stuck villager gives up and re-plans.
    stuck: u32,
}

struct Foragers {
    grid: NavGrid,
    finder: PathFinder,
    resources: SpatialGrid,
    villagers: Vec<Entity>,
    storehouse: Vec3,
    rng: Rng,
    wood: f32,
    food: f32,
    /// Meshes, built once.
    cube: Mesh,
    rock: Mesh,
    ground: Mesh,
    /// Scratch, so the per-tick systems do not allocate.
    nearby: Vec<u32>,
    positions: HashMap<u32, Vec3>,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Foragers {
    fn new(seed: u64) -> Self {
        Self {
            grid: NavGrid::new(
                vec2(-AREA * 0.5, -AREA * 0.5),
                CELL,
                AREA as usize,
                AREA as usize,
            ),
            finder: PathFinder::new(),
            resources: SpatialGrid::new(6.0),
            villagers: Vec::new(),
            storehouse: Vec3::ZERO,
            rng: Rng::named(seed, "foragers"),
            wood: 0.0,
            food: 0.0,
            cube: Mesh::cube(1.0),
            // A few hundred outcrops are drawn every frame, so they are boxes:
            // a sphere here costs ten times the triangles for a rock nobody
            // looks at closely.
            rock: Mesh::cube(1.0),
            ground: Mesh::plane(AREA, 1),
            nearby: Vec::new(),
            positions: HashMap::new(),
            yaw: 0.6,
            pitch: 0.42,
            distance: 26.0,
        }
    }

    /// Cost the ground from the same noise the terrain is drawn from.
    fn shape_the_land(&mut self, seed: u64) {
        let rocks = Noise::named(seed, "rocks");
        let mud = Noise::named(seed, "mud");
        let storehouse = self.storehouse;
        self.grid = NavGrid::from_fn(
            vec2(-AREA * 0.5, -AREA * 0.5),
            CELL,
            AREA as usize,
            AREA as usize,
            |centre| {
                let sample = vec2(centre.x * 0.06, centre.z * 0.06);
                // Keep the ground around the storehouse clear whatever the
                // noise says, so the village is never born inside a boulder.
                if runity::ai::ground_distance(centre, storehouse) < 5.0 {
                    return runity::ai::OPEN;
                }
                if rocks.ridged_2d(sample, Fbm::TERRAIN.at(0.8)) > 0.45 {
                    runity::ai::BLOCKED
                } else if mud.fbm_2d(sample, Fbm::TERRAIN) > 0.25 {
                    4 // boggy: passable, but a villager would rather not
                } else {
                    runity::ai::OPEN
                }
            },
        );
    }

    /// Scatter trees and berry bushes on open ground.
    fn sow(&mut self, world: &mut World, count: usize) {
        for index in 0..count {
            let Some(position) = self.open_spot() else {
                continue;
            };
            let kind = if index % 2 == 0 {
                Kind::Wood
            } else {
                Kind::Food
            };
            let entity = world.spawn();
            world.insert(entity, At(position));
            world.insert(
                entity,
                Resource {
                    kind,
                    remaining: self.rng.range(20.0, 60.0),
                },
            );
        }
    }

    /// A random walkable point, or nothing after a fair number of tries.
    fn open_spot(&mut self) -> Option<Vec3> {
        for _ in 0..40 {
            let point = vec3(
                self.rng.range(-AREA * 0.45, AREA * 0.45),
                0.0,
                self.rng.range(-AREA * 0.45, AREA * 0.45),
            );
            if let Some((x, y)) = self.grid.cell_at(point) {
                if self.grid.cost(x, y) == runity::ai::OPEN {
                    return Some(point);
                }
            }
        }
        None
    }

    // -------------------------------------------------------------- systems

    /// Once per world tick: everybody decides what they are doing.
    fn decide(&mut self, world: &mut World) {
        // The resource index is rebuilt every tick rather than maintained:
        // it cannot then hold a tree somebody already felled.
        self.positions.clear();
        let resources: Vec<(Entity, Vec3, Kind, f32)> = world
            .query2::<At, Resource>()
            .map(|(entity, at, resource)| (entity, at.0, resource.kind, resource.remaining))
            .collect();
        self.resources.rebuild(
            resources
                .iter()
                .map(|(entity, at, ..)| (entity.index(), *at)),
        );
        for (entity, at, ..) in &resources {
            self.positions.insert(entity.index(), *at);
        }

        for index in 0..self.villagers.len() {
            let entity = self.villagers[index];
            let Some(at) = world.get::<At>(entity).map(|at| at.0) else {
                continue;
            };
            let (job, carrying, has_path) = {
                let Some(villager) = world.get::<Villager>(entity) else {
                    continue;
                };
                (villager.job, villager.carrying, villager.follower.is_some())
            };

            // Busy and getting somewhere: leave them to it.
            if has_path && job != Job::Idle {
                continue;
            }

            let home = runity::ai::ground_distance(at, self.storehouse);
            let next = if carrying >= 10.0 {
                Job::Deliver
            } else {
                // Two scored options. The village needs food more than wood
                // when it is short of it, and a villager already holding
                // something would rather finish that errand.
                let wood_need = curve::inverse(self.wood, 0.0, 400.0);
                let food_need = curve::inverse(self.food, 0.0, 400.0);
                let options = [
                    (Job::Forage, score(&[food_need, curve::falloff(home, AREA)])),
                    (Job::Chop, score(&[wood_need, curve::falloff(home, AREA)])),
                ];
                choose_near_best(&options, 0.08, &mut self.rng)
                    .map(|(job, _)| job)
                    .unwrap_or(Job::Idle)
            };

            let kind = match next {
                Job::Chop => Kind::Wood,
                _ => Kind::Food,
            };
            let (goal, picked) = match next {
                Job::Deliver | Job::Idle => (Some(self.storehouse), None),
                _ => match self.pick_resource(at, kind, &resources) {
                    Some((entity, position)) => (Some(position), Some(entity)),
                    None => (None, None),
                },
            };

            let Some(goal) = goal else {
                if let Some(mut villager) = world.get_mut::<Villager>(entity) {
                    villager.job = Job::Idle;
                    villager.follower = None;
                }
                continue;
            };

            // Path from and to the nearest open ground: a villager standing on
            // a cell that was just built over still has to get home.
            let start = self
                .grid
                .nearest_walkable(at, 4)
                .map(|(x, y)| self.grid.cell_centre(x, y))
                .unwrap_or(at);
            let landing = self
                .grid
                .nearest_walkable(goal, 4)
                .map(|(x, y)| self.grid.cell_centre(x, y))
                .unwrap_or(goal);
            let path = self
                .finder
                .find(&self.grid, start, landing, &PathSettings::default());

            if let Some(mut villager) = world.get_mut::<Villager>(entity) {
                villager.job = next;
                // Only an outbound errand sets what they are after; on the way
                // home `kind` is what they are already carrying, and
                // overwriting it turns a load of timber into berries.
                if matches!(next, Job::Forage | Job::Chop) {
                    villager.kind = kind;
                }
                villager.stuck = 0;
                villager.target = picked;
                villager.follower = path.ok().map(|path| PathFollower::new(path, 0.6));
                if villager.follower.is_none() {
                    villager.job = Job::Idle;
                }
            }
        }
    }

    /// The nearest resource of a kind that still has something left.
    fn pick_resource(
        &mut self,
        from: Vec3,
        kind: Kind,
        resources: &[(Entity, Vec3, Kind, f32)],
    ) -> Option<(Entity, Vec3)> {
        self.resources.query_radius(from, AREA, &mut self.nearby);
        let mut best: Option<(Entity, Vec3, f32)> = None;
        for id in &self.nearby {
            let Some((entity, at, resource_kind, remaining)) =
                resources.iter().find(|(entity, ..)| entity.index() == *id)
            else {
                continue;
            };
            if *resource_kind != kind || *remaining <= 0.0 {
                continue;
            }
            let distance = runity::ai::ground_distance(*at, from);
            // `is_none_or` would read better, but it is newer than this
            // workspace's minimum Rust version.
            let closer = match best {
                Some((_, _, current)) => distance < current,
                None => true,
            };
            if closer {
                best = Some((*entity, *at, distance));
            }
        }
        best.map(|(entity, at, _)| (entity, at))
    }

    /// Every frame: walk, avoid each other, and work when they arrive.
    fn move_everyone(&mut self, world: &mut World, dt: f32) {
        // Villagers with somewhere to be: brisker than a stroll, so a
        // watching player sees the settlement work rather than shuffle.
        let locomotion = Locomotion::fast(2.4);
        let crowd: Vec<Vec3> = self
            .villagers
            .iter()
            .filter_map(|entity| world.get::<At>(*entity).map(|at| at.0))
            .collect();

        for index in 0..self.villagers.len() {
            let entity = self.villagers[index];
            let Some(at) = world.get::<At>(entity).map(|a| a.0) else {
                continue;
            };
            let Some(mut villager) = world.get_mut::<Villager>(entity) else {
                continue;
            };

            let mut position = at;
            let target = villager.follower.as_mut().and_then(|f| f.target(position));
            let mut acceleration = match target {
                Some(point) => arrive(position, villager.velocity, point, &locomotion),
                None => arrive(position, villager.velocity, position, &locomotion),
            };
            // Keep out of each other's way: the nearest neighbour matters
            // most, so a queue at the storehouse loosens rather than jams.
            let others = crowd
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, p)| *p);
            acceleration += separation(position, others, 1.2, 2.0);

            let mut velocity = villager.velocity;
            let moved =
                runity::ai::integrate(&mut position, &mut velocity, acceleration, &locomotion, dt);

            // Refuse to walk into a rock: the grid is the authority on where
            // the ground is, and steering only suggests.
            let walkable = self
                .grid
                .cell_at(position)
                .map(|(x, y)| self.grid.walkable(x, y))
                .unwrap_or(false);
            if walkable {
                villager.velocity = velocity;
                if moved > 1e-4 {
                    villager.facing = velocity.normalized();
                }
            } else {
                position = at;
                villager.velocity = Vec3::ZERO;
                villager.stuck += 1;
            }
            let arrived = villager
                .follower
                .as_ref()
                .is_some_and(PathFollower::is_finished);
            let job = villager.job;
            let kind = villager.kind;
            let target_entity = villager.target;
            if arrived {
                villager.follower = None;
            }

            if let Some(mut at) = world.get_mut::<At>(entity) {
                at.0 = position;
            }
            if arrived {
                self.finish_errand(world, entity, job, kind, target_entity, position);
            }
        }
    }

    /// What happens when somebody gets where they were going.
    fn finish_errand(
        &mut self,
        world: &mut World,
        entity: Entity,
        job: Job,
        kind: Kind,
        target: Option<Entity>,
        position: Vec3,
    ) {
        match job {
            Job::Deliver | Job::Idle => {
                if runity::ai::ground_distance(position, self.storehouse) < 3.0 {
                    let Some(mut villager) = world.get_mut::<Villager>(entity) else {
                        return;
                    };
                    match villager.kind {
                        Kind::Wood => self.wood += villager.carrying,
                        Kind::Food => self.food += villager.carrying,
                    }
                    villager.carrying = 0.0;
                    villager.job = Job::Idle;
                }
            }
            Job::Forage | Job::Chop => {
                let Some(target) = target else {
                    return;
                };
                let taken = {
                    let Some(mut resource) = world.get_mut::<Resource>(target) else {
                        return;
                    };
                    let taken = resource.remaining.min(12.0);
                    resource.remaining -= taken;
                    taken
                };
                // A patch that has been picked clean stops existing, and the
                // Despawned event tells anyone who cached it.
                if world
                    .get::<Resource>(target)
                    .is_some_and(|r| r.remaining <= 0.0)
                {
                    world.despawn(target);
                }
                if let Some(mut villager) = world.get_mut::<Villager>(entity) {
                    villager.carrying += taken;
                    villager.kind = kind;
                    villager.job = Job::Deliver;
                }
            }
        }
    }
}

impl Game for Foragers {
    fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
        engine.renderer.set_sky(Sky::new(SkyParams {
            sun_direction: vec3(0.5, 0.75, -0.4).normalized(),
            sun_irradiance: 4.0,
            ..SkyParams::default()
        }));
        engine.renderer.settings.shadows.extent = AREA;
        engine.renderer.settings.ambient_intensity = 0.6;
        // Nothing in this scene is shiny, and reflections are the most
        // expensive pass there is.
        engine.renderer.settings.ssr.enabled = false;
        engine.exposure = 0.95;
        engine.camera.target = Vec3::ZERO;
        // Ten world ticks a second: decisions are cheap, but not free, and a
        // villager who reconsiders every frame never finishes anything.
        engine.clock = WorldClock::new(10.0);

        self.shape_the_land(7);
        self.sow(&mut engine.world, 40);
        for _ in 0..12 {
            let spot = self.open_spot().unwrap_or(self.storehouse);
            let entity = engine.world.spawn();
            engine.world.insert(entity, At(spot));
            engine.world.insert(
                entity,
                Villager {
                    job: Job::Idle,
                    carrying: 0.0,
                    kind: Kind::Food,
                    velocity: Vec3::ZERO,
                    facing: Vec3::X,
                    follower: None,
                    target: None,
                    stuck: 0,
                },
            );
            self.villagers.push(entity);
        }
        Ok(())
    }

    fn update(&mut self, engine: &mut Engine) {
        let dt = engine.time.delta();
        if engine.input.key_pressed(Key::Escape) {
            engine.quit();
            return;
        }
        if engine.input.key_pressed(Key::P) {
            engine.clock.toggle_pause();
        }
        for (key, view) in [
            (Key::Num1, DebugView::Shaded),
            (Key::Num2, DebugView::Wireframe),
            (Key::Num3, DebugView::Depth),
            (Key::Num5, DebugView::Albedo),
            (Key::Num6, DebugView::Normals),
            (Key::Num8, DebugView::Occlusion),
        ] {
            if engine.input.key_pressed(key) {
                engine.debug_view = view;
            }
        }

        let input = &engine.input;
        self.yaw += (input.axis(Key::Left, Key::Right) + input.axis(Key::A, Key::D)) * dt * 1.2;
        self.pitch = (self.pitch
            + (input.axis(Key::Down, Key::Up) + input.axis(Key::S, Key::W)) * dt)
            .clamp(0.15, 1.45);
        self.distance = (self.distance + input.axis(Key::E, Key::Q) * dt * 25.0).clamp(12.0, 70.0);
        engine.camera.orbit(self.yaw, self.pitch, self.distance);

        // Movement is per frame, so it stays smooth whatever the tick rate —
        // but it runs on the world's time scale, so a paused world is still
        // and a sped-up one really is faster.
        if !engine.clock.is_paused() {
            let world_dt = dt * engine.clock.speed();
            let mut world = std::mem::take(&mut engine.world);
            self.move_everyone(&mut world, world_dt);
            engine.world = world;
        }
    }

    fn world_tick(&mut self, engine: &mut Engine) {
        // Decisions are on the world's clock, not the frame's.
        let mut world = std::mem::take(&mut engine.world);
        self.decide(&mut world);
        engine.world = world;
    }

    fn render(&mut self, engine: &mut Engine) {
        engine.draw_pbr(
            &self.ground,
            Mat4::IDENTITY,
            &Material {
                base_color: Color::rgb(0.22, 0.26, 0.18),
                roughness: 0.95,
                ..Material::default()
            },
        );

        // Rocks and bogs, straight from the navigation grid — what you see is
        // literally what the villagers are pathing around.
        let mut rocks = Vec::new();
        let mut bogs = Vec::new();
        for y in 0..self.grid.height() {
            for x in 0..self.grid.width() {
                match self.grid.cost(x, y) {
                    runity::ai::BLOCKED => rocks.push(self.grid.cell_centre(x, y)),
                    runity::ai::OPEN => {}
                    _ => bogs.push(self.grid.cell_centre(x, y)),
                }
            }
        }
        let stone = Material {
            base_color: Color::rgb(0.17, 0.17, 0.16),
            roughness: 0.95,
            ..Material::default()
        };
        for centre in rocks {
            // Vary the height from the position itself, so the outcrops look
            // like rock rather than cobbles — and so they look the same on
            // every run.
            let lump = Noise::named(7, "lumps").value_2d(vec2(centre.x, centre.z)) * 0.5 + 0.5;
            let height = 0.5 + lump * 0.8;
            let model = Mat4::from_translation(centre + vec3(0.0, height * 0.35, 0.0))
                * Mat4::from_scale(vec3(0.9, height * 0.7, 0.9));
            engine.draw_pbr(&self.rock, model, &stone);
        }
        let bog = Material {
            base_color: Color::rgb(0.16, 0.15, 0.12),
            roughness: 1.0,
            ..Material::default()
        };
        for centre in bogs {
            let model = Mat4::from_translation(centre + vec3(0.0, 0.02, 0.0))
                * Mat4::from_scale(vec3(1.0, 0.04, 1.0));
            engine.draw_pbr(&self.cube, model, &bog);
        }

        // The storehouse.
        engine.draw_pbr(
            &self.cube,
            Mat4::from_translation(self.storehouse + vec3(0.0, 1.0, 0.0))
                * Mat4::from_scale(Vec3::splat(2.0)),
            &Material {
                base_color: Color::rgb(0.50, 0.42, 0.30),
                roughness: 0.8,
                ..Material::default()
            },
        );

        let resources: Vec<(Vec3, Kind, f32)> = engine
            .world
            .query2::<At, Resource>()
            .map(|(_, at, resource)| (at.0, resource.kind, resource.remaining))
            .collect();
        for (position, kind, remaining) in resources {
            let scale = 0.4 + remaining / 90.0;
            let (colour, height) = match kind {
                Kind::Wood => (Color::rgb(0.28, 0.20, 0.10), 1.6),
                Kind::Food => (Color::rgb(0.35, 0.55, 0.18), 0.7),
            };
            let model = Mat4::from_translation(position + vec3(0.0, height * scale * 0.5, 0.0))
                * Mat4::from_scale(vec3(scale * 0.7, height * scale, scale * 0.7));
            engine.draw_pbr(
                &self.cube,
                model,
                &Material {
                    base_color: colour,
                    roughness: 0.9,
                    ..Material::default()
                },
            );
        }

        let people: Vec<(Vec3, Job, f32)> = self
            .villagers
            .iter()
            .filter_map(|entity| {
                let at = engine.world.get::<At>(*entity)?.0;
                let villager = engine.world.get::<Villager>(*entity)?;
                Some((at, villager.job, villager.carrying))
            })
            .collect();
        for (position, job, carrying) in people {
            let model = Mat4::from_translation(position + vec3(0.0, 0.85, 0.0))
                * Mat4::from_scale(vec3(0.55, 1.7, 0.55));
            engine.draw_pbr(
                &self.cube,
                model,
                &Material {
                    base_color: job.colour(),
                    roughness: 0.7,
                    ..Material::default()
                },
            );
            if carrying > 0.0 {
                let model = Mat4::from_translation(position + vec3(0.0, 1.9, 0.0))
                    * Mat4::from_scale(Vec3::splat(0.4));
                engine.draw_pbr(
                    &self.cube,
                    model,
                    &Material {
                        base_color: Color::rgb(0.8, 0.7, 0.3),
                        roughness: 0.6,
                        ..Material::default()
                    },
                );
            }
        }
    }
}

/// Wraps the game so a headless run can report what the village achieved,
/// which is the only way to tell from a screenshot whether the AI worked.
struct Reported {
    inner: Foragers,
    report: std::rc::Rc<std::cell::RefCell<(f32, f32)>>,
}

impl Game for Reported {
    fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
        self.inner.start(engine)?;
        // A screenshot is one moment; run the world faster so that moment has
        // something in it.
        engine.clock.set_speed(4.0);
        Ok(())
    }
    fn update(&mut self, engine: &mut Engine) {
        self.inner.update(engine);
        *self.report.borrow_mut() = (self.inner.wood, self.inner.food);
    }
    fn world_tick(&mut self, engine: &mut Engine) {
        self.inner.world_tick(engine);
    }
    fn render(&mut self, engine: &mut Engine) {
        self.inner.render(engine);
    }
}

fn main() -> std::io::Result<()> {
    let dimension = |name: &str, fallback: u32| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(fallback)
    };
    let config = WindowConfig::new(
        "runity — foragers",
        dimension("RUNITY_WIDTH", 960),
        dimension("RUNITY_HEIGHT", 540),
    );
    let headless = std::env::var("RUNITY_HEADLESS").is_ok();

    let mut app = App::new(config);
    if headless {
        app = app
            .with_max_frames(120)
            .with_frame_delta(1.0 / 30.0)
            .with_target_fps(None);
    }

    let mut game = Foragers::new(3);
    game.yaw = 0.8;
    let report = std::rc::Rc::new(std::cell::RefCell::new((0.0f32, 0.0f32)));
    let engine = app.run(Reported {
        inner: game,
        report: report.clone(),
    })?;

    if headless {
        let path =
            std::env::var("RUNITY_SCREENSHOT").unwrap_or_else(|_| "foragers.png".to_string());
        save_png(&path, &engine.framebuffer)?;
        let (wood, food) = *report.borrow();
        println!("the storehouse holds {wood:.0} wood and {food:.0} food");
        println!("wrote {path} after {} world ticks", engine.clock.tick());
    }
    Ok(())
}
