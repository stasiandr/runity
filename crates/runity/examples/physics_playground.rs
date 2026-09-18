//! Crates and balls falling into a pile — the demo that shows the simulation
//! and the renderer working on the same scene.
//!
//! ```text
//! cargo run --release --example physics_playground
//! RUNITY_HEADLESS=1 cargo run --release --example physics_playground   # writes a PNG
//! ```
//!
//! Controls: arrows or WASD orbit, Q/E zoom, Space fires a ball from the
//! camera, R rebuilds the stack, P pauses, Escape quits.

use runity::physics::Material as Surface;
use runity::prelude::*;
use std::collections::HashMap;

struct Playground {
    cube: Mesh,
    ball: Mesh,
    floor: Mesh,
    yaw: f32,
    pitch: f32,
    distance: f32,
    paused: bool,
    /// How recently each body was hit, so impacts can be seen rather than
    /// merely believed.
    flash: HashMap<BodyHandle, f32>,
    rng: Rng,
}

impl Playground {
    fn new() -> Self {
        Self {
            cube: Mesh::cube(1.0),
            ball: Mesh::sphere(1.0, 24, 16),
            floor: Mesh::plane(30.0, 1),
            yaw: 0.7,
            pitch: 0.35,
            distance: 14.0,
            paused: false,
            flash: HashMap::new(),
            rng: Rng::named(1, "playground"),
        }
    }

    /// Ground plus a pyramid of crates, with a couple of heavy spheres on top.
    fn build(&mut self, engine: &mut Engine) {
        engine.physics = PhysicsWorld::new();
        self.flash.clear();
        self.rng = Rng::named(1, "playground");

        engine
            .physics
            .add(RigidBody::fixed(Shape::ground()).with_material(Surface {
                friction: 0.8,
                restitution: 0.0,
            }));

        let half = 0.5;
        for level in 0..5 {
            let count = 5 - level;
            for column in 0..count {
                let offset = (column as f32 - (count as f32 - 1.0) * 0.5) * 1.05;
                engine.physics.add(
                    RigidBody::dynamic(Shape::cuboid(Vec3::splat(half)))
                        .with_position(Vec3::new(
                            offset,
                            half + level as f32 * 1.01,
                            level as f32 * 0.02,
                        ))
                        .with_material(Surface {
                            friction: 0.6,
                            restitution: 0.05,
                        })
                        .with_user_data(1),
                );
            }
        }

        for i in 0..3 {
            engine.physics.add(
                RigidBody::dynamic(Shape::sphere(0.45))
                    .with_position(Vec3::new(
                        self.rng.range(-1.5, 1.5),
                        7.0 + i as f32 * 1.6,
                        self.rng.range(-1.5, 1.5),
                    ))
                    .with_density(3.0)
                    .with_material(Surface {
                        friction: 0.4,
                        restitution: 0.3,
                    })
                    .with_user_data(2),
            );
        }
    }

    /// Fire a ball along the camera's line of sight.
    fn shoot(&mut self, engine: &mut Engine) {
        let from = engine.camera.position;
        let direction = (engine.camera.target - from).normalized();
        engine.physics.add(
            RigidBody::dynamic(Shape::sphere(0.35))
                .with_position(from)
                .with_velocity(direction * 22.0)
                .with_density(6.0)
                .with_material(Surface {
                    friction: 0.3,
                    restitution: 0.4,
                })
                .with_user_data(3),
        );
    }

    fn material_for(&self, user_data: u64, lit: f32) -> Material<'static> {
        let base = match user_data {
            1 => Color::rgb(0.42, 0.20, 0.08),
            2 => Color::rgb(0.75, 0.76, 0.78),
            3 => Color::rgb(0.85, 0.35, 0.12),
            _ => Color::rgb(0.30, 0.31, 0.33),
        };
        // An impact brightens the body for a moment.
        let color = Color::rgb(base.r + lit * 0.8, base.g + lit * 0.7, base.b + lit * 0.5);
        match user_data {
            2 | 3 => Material::metal(color, 0.25),
            _ => Material {
                base_color: color,
                roughness: 0.8,
                ..Material::default()
            },
        }
    }
}

impl Game for Playground {
    fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
        // A sun off to one side: the shadows are what make a pile of boxes
        // read as a pile rather than as sprites floating over a plane.
        engine.renderer.set_sky(Sky::new(SkyParams {
            sun_direction: Vec3::new(0.55, 0.70, -0.45).normalized(),
            sun_irradiance: 4.2,
            ..SkyParams::default()
        }));
        engine.renderer.settings.shadows.extent = 16.0;
        engine.renderer.settings.ambient_intensity = 0.8;
        engine.renderer.settings.ssao.radius = 0.4;
        engine.camera.target = Vec3::new(0.0, 1.5, 0.0);
        engine.exposure = 0.9;
        self.build(engine);
        Ok(())
    }

    fn update(&mut self, engine: &mut Engine) {
        let dt = engine.time.delta();

        if engine.input.key_pressed(Key::Escape) {
            engine.quit();
            return;
        }
        if engine.input.key_pressed(Key::R) {
            self.build(engine);
        }
        if engine.input.key_pressed(Key::Space) {
            self.shoot(engine);
        }
        if engine.input.key_pressed(Key::P) {
            self.paused = !self.paused;
            engine.auto_step_physics = !self.paused;
        }

        let input = &engine.input;
        self.yaw += (input.axis(Key::Left, Key::Right) + input.axis(Key::A, Key::D)) * dt * 1.5;
        self.pitch = (self.pitch
            + (input.axis(Key::Down, Key::Up) + input.axis(Key::S, Key::W)) * dt * 1.2)
            .clamp(-0.2, 1.3);
        self.distance = (self.distance + input.axis(Key::E, Key::Q) * dt * 12.0).clamp(5.0, 40.0);
        engine.camera.orbit(self.yaw, self.pitch, self.distance);

        // Contacts are events, not a state to poll: a hit that begins and
        // ends inside one step still gets its flash.
        for event in engine.physics.contacts() {
            if let ContactEvent::Started(a, b) = event {
                self.flash.insert(*a, 1.0);
                self.flash.insert(*b, 1.0);
            }
        }
        self.flash.retain(|_, remaining| {
            *remaining -= dt * 3.0;
            *remaining > 0.0
        });
    }

    fn render(&mut self, engine: &mut Engine) {
        engine.draw_pbr(
            &self.floor,
            Mat4::IDENTITY,
            &Material {
                base_color: Color::rgb(0.20, 0.21, 0.20),
                roughness: 0.9,
                ..Material::default()
            },
        );

        // Collect first: drawing borrows the engine, and so does the physics
        // world that lives inside it.
        let bodies: Vec<(Shape, Mat4, u64, f32)> = engine
            .physics
            .iter()
            .filter(|(_, body)| !matches!(body.shape, Shape::HalfSpace { .. }))
            .map(|(handle, body)| {
                let transform =
                    Transform::from_position(body.position()).with_rotation(body.rotation());
                let lit = self.flash.get(&handle).copied().unwrap_or(0.0);
                (body.shape, transform.matrix(), body.user_data, lit)
            })
            .collect();

        for (shape, transform, user_data, lit) in bodies {
            let material = self.material_for(user_data, lit * 0.6);
            match shape {
                Shape::Cuboid { half_extents } => {
                    let model = transform * Mat4::from_scale(half_extents * 2.0);
                    engine.draw_pbr(&self.cube, model, &material);
                }
                Shape::Sphere { radius } => {
                    let model = transform * Mat4::from_scale(Vec3::splat(radius));
                    engine.draw_pbr(&self.ball, model, &material);
                }
                _ => {}
            }
        }
    }
}

/// A headless run needs the shot fired for it, so the screenshot has
/// something happening in it.
struct Scripted {
    inner: Playground,
    fired: bool,
}

impl Game for Scripted {
    fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
        self.inner.start(engine)
    }
    fn update(&mut self, engine: &mut Engine) {
        self.inner.update(engine);
        if !self.fired && engine.time.frame() == 100 {
            self.inner.shoot(engine);
            self.fired = true;
        }
        engine.camera.orbit(0.9, 0.32, 13.0);
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
        "runity — physics playground",
        dimension("RUNITY_WIDTH", 960),
        dimension("RUNITY_HEIGHT", 540),
    );
    let headless = std::env::var("RUNITY_HEADLESS").is_ok();

    let mut app = App::new(config);
    if headless {
        app = app
            .with_max_frames(150)
            .with_frame_delta(1.0 / 60.0)
            .with_target_fps(None);
    }

    let engine = if headless {
        app.run(Scripted {
            inner: Playground::new(),
            fired: false,
        })?
    } else {
        app.run(Playground::new())?
    };

    if headless {
        let path = std::env::var("RUNITY_SCREENSHOT").unwrap_or_else(|_| "physics.png".to_string());
        save_png(&path, &engine.framebuffer)?;
        let stats = engine.physics.stats();
        println!(
            "wrote {path}; {} bodies, {} awake, {} contacts in the last step",
            stats.bodies, stats.awake, stats.contacts
        );
    }
    Ok(())
}
