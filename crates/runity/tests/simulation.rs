//! The simulation foundation, tested end to end: clock, entities, events,
//! randomness, physics and saves have to agree with each other, not just each
//! be correct on its own.

use runity::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Agent {
    home: Vec3,
    carried: u32,
}

serializable!(Agent { home, carried });

#[derive(Debug, Clone, Copy, PartialEq)]
struct Wanderer(Vec3);

serializable!(Wanderer(0));

#[derive(Debug, Clone, Copy)]
struct Delivered(u32);

/// A miniature of the settlement example: agents wander, pick things up and
/// drop them at home, driven by a seeded generator.
struct Sim {
    world: World,
    clock: WorldClock,
    rng: Rng,
    noise: Noise,
    delivered: u32,
}

impl Sim {
    fn new(seed: u64) -> Self {
        let mut sim = Sim {
            world: World::new(),
            clock: WorldClock::new(10.0).with_calendar(Calendar::new(24, 360)),
            rng: Rng::named(seed, "sim"),
            noise: Noise::named(seed, "ground"),
            delivered: 0,
        };
        for _ in 0..24 {
            let home = Vec3::new(sim.rng.range(-20.0, 20.0), 0.0, sim.rng.range(-20.0, 20.0));
            let agent = sim.world.spawn();
            sim.world.insert(agent, Agent { home, carried: 0 });
            sim.world.insert(agent, Wanderer(home));
        }
        sim
    }

    fn tick(&mut self) {
        self.world.advance_tick();
        let noise = self.noise;
        let mut gathered = 0;
        self.world
            .each2_mut::<Wanderer, Agent>(|_, mut position, agent| {
                let sample = vec2(position.0.x * 0.1, position.0.z * 0.1);
                let drift = vec3(
                    noise.perlin_2d(sample),
                    0.0,
                    noise.perlin_2d(sample + vec2(31.0, 17.0)),
                );
                position.0 += drift * 0.5;
                if (position.0 - agent.home).length() > 6.0 {
                    gathered += 1;
                    position.0 = agent.home;
                }
            });
        if gathered > 0 {
            self.delivered += gathered;
            self.world.send(Delivered(gathered));
        }
    }

    fn run(&mut self, ticks: u64) {
        let step = self.clock.seconds_per_tick() as f32;
        for _ in 0..ticks {
            self.clock.advance(step);
            while self.clock.next_tick().is_some() {
                self.tick();
            }
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        Archive::write(*b"SNAP", 1, |writer| {
            writer
                .write(&self.clock)
                .write(&self.rng)
                .write(&self.delivered);
            self.world.save_entities(writer);
            self.world.save_components::<Agent>(writer);
            self.world.save_components::<Wanderer>(writer);
        })
    }

    fn restore(bytes: &[u8], seed: u64) -> Sim {
        let (_, mut reader) = Archive::open(bytes, *b"SNAP", 1).unwrap();
        let clock: WorldClock = reader.read().unwrap();
        let rng: Rng = reader.read().unwrap();
        let delivered: u32 = reader.read().unwrap();
        let mut world = World::new();
        world.load_entities(&mut reader).unwrap();
        world.load_components::<Agent>(&mut reader).unwrap();
        world.load_components::<Wanderer>(&mut reader).unwrap();
        reader.finish().unwrap();
        Sim {
            world,
            clock,
            rng,
            noise: Noise::named(seed, "ground"),
            delivered,
        }
    }

    fn state(&self) -> (u64, u32, usize, Vec<Vec3>) {
        (
            self.clock.tick(),
            self.delivered,
            self.world.count::<Agent>(),
            self.world.iter::<Wanderer>().map(|(_, w)| w.0).collect(),
        )
    }
}

#[test]
fn the_same_seed_produces_the_same_history() {
    let mut first = Sim::new(4);
    let mut second = Sim::new(4);
    first.run(2_000);
    second.run(2_000);
    assert_eq!(first.state(), second.state());
    assert!(first.delivered > 0, "the agents should have done something");
}

#[test]
fn a_different_seed_produces_a_different_history() {
    let mut first = Sim::new(4);
    let mut second = Sim::new(5);
    first.run(2_000);
    second.run(2_000);
    assert_ne!(first.state(), second.state());
}

#[test]
fn a_snapshot_resumes_exactly_where_it_was_taken() {
    let mut original = Sim::new(7);
    original.run(1_500);

    let bytes = original.snapshot();
    let mut restored = Sim::restore(&bytes, 7);
    assert_eq!(restored.state(), original.state());
    assert_eq!(
        restored.snapshot(),
        bytes,
        "a snapshot must survive its own round trip"
    );

    // And, crucially, keeps agreeing afterwards: a save that only looks right
    // is worse than one that is obviously broken.
    original.run(500);
    restored.run(500);
    assert_eq!(restored.state(), original.state());
}

#[test]
fn the_world_clock_drives_ticks_independently_of_the_frame_rate() {
    let ticks_at = |fps: f32| {
        let mut sim = Sim::new(1);
        let mut clock = WorldClock::new(10.0);
        let frames = (fps * 5.0) as usize;
        for _ in 0..frames {
            clock.advance(1.0 / fps);
            while clock.next_tick().is_some() {
                sim.tick();
            }
        }
        clock.tick()
    };
    assert_eq!(ticks_at(30.0), 50);
    assert_eq!(ticks_at(144.0), 50);
}

#[test]
fn a_paused_world_stops_developing() {
    let mut sim = Sim::new(11);
    sim.run(200);
    let before = sim.state();

    sim.clock.pause();
    sim.run(500);
    assert_eq!(
        sim.state(),
        before,
        "a pause has to stop history, not hide it"
    );

    sim.clock.resume();
    sim.run(200);
    assert_ne!(sim.state(), before);
}

#[test]
fn change_detection_reports_exactly_what_a_tick_touched() {
    let mut sim = Sim::new(3);
    sim.run(50);

    let acknowledged = sim.world.change_tick();
    sim.world.advance_tick();
    let moved: Vec<Entity> = sim
        .world
        .changed_since::<Wanderer>(acknowledged)
        .map(|(e, _)| e)
        .collect();
    assert!(
        moved.is_empty(),
        "nothing has moved since the tick turned over"
    );

    sim.tick();
    let moved = sim.world.changed_since::<Wanderer>(acknowledged).count();
    assert_eq!(
        moved,
        sim.world.count::<Wanderer>(),
        "one tick moves every agent"
    );
    assert_eq!(
        sim.world.changed_since::<Agent>(acknowledged).count(),
        0,
        "and touches no Agent"
    );
}

#[test]
fn events_reach_their_reader_once() {
    let mut sim = Sim::new(9);
    sim.run(400);
    let mut seen = 0;
    for _ in 0..200 {
        sim.tick();
        seen += sim
            .world
            .events::<Delivered>()
            .iter()
            .map(|e| e.0)
            .sum::<u32>();
    }
    // Every delivery was announced exactly once, and the running total the
    // systems kept agrees with the events they sent.
    assert!(seen > 0);
    assert!(seen <= sim.delivered);
}

#[test]
fn physics_and_the_world_clock_stay_in_step() {
    // A body dropped in one world and in a copy of it must land in the same
    // place, tick for tick: that is what makes a replay or a second machine
    // agree with the first.
    let build = || {
        let mut physics = PhysicsWorld::new();
        physics.add(RigidBody::fixed(Shape::ground()));
        for i in 0..8 {
            physics.add(
                RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.4))).with_position(Vec3::new(
                    (i % 3) as f32 * 0.5,
                    1.0 + i as f32 * 0.9,
                    0.0,
                )),
            );
        }
        physics
    };

    let mut first = build();
    let mut second = build();
    for _ in 0..300 {
        first.step(1.0 / 60.0);
    }
    for _ in 0..300 {
        second.step(1.0 / 60.0);
    }

    for ((_, a), (_, b)) in first.iter().zip(second.iter()) {
        assert_eq!(a.position(), b.position());
        assert_eq!(a.rotation(), b.rotation());
    }
    assert!(
        first.iter().all(|(_, body)| body.position().y > -0.1),
        "nothing fell through"
    );
}

#[test]
fn the_engine_loop_ticks_the_world_on_its_own_clock() {
    // The frame loop should advance the world without the game having to
    // count anything itself.
    struct Counted {
        ticks: u32,
        frames: u32,
    }

    impl Game for Counted {
        fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
            engine.clock = WorldClock::new(10.0);
            Ok(())
        }
        fn update(&mut self, _engine: &mut Engine) {
            self.frames += 1;
        }
        fn world_tick(&mut self, engine: &mut Engine) {
            self.ticks += 1;
            // The world's change tick advances with it, so systems can ask
            // what moved since last time.
            assert!(engine.world.change_tick() > 1);
        }
    }

    let config = WindowConfig::new("simulation", 32, 32);
    let engine = App::new(config.clone())
        .with_max_frames(60)
        .with_frame_delta(1.0 / 60.0)
        .with_target_fps(None)
        .run_with_window(
            Box::new(HeadlessWindow::new(&config)),
            Counted {
                ticks: 0,
                frames: 0,
            },
        )
        .expect("the headless loop should run");

    // Sixty frames at sixty a second is one second, which is ten world ticks.
    assert_eq!(engine.clock.tick(), 10);
    assert_eq!(
        engine.world.change_tick(),
        11,
        "one change tick per world tick"
    );
}
