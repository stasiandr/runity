//! Walking around in first person: the player half of the game.
//!
//! ```text
//! cargo run --release --example first_person
//! RUNITY_HEADLESS=1 cargo run --release --example first_person   # writes a PNG
//! ```
//!
//! Controls: WASD or arrows to walk, mouse or Q/E to look, Space to jump,
//! Shift to run, F to shove the nearest crate, Escape to quit.
//!
//! The player is a character controller rather than a rigid body, on purpose.
//! A simulated body is pushed around by impulses, which is exactly the wrong
//! feel: sliding on ice nobody asked for, bouncing off a step, tipping over
//! into a crate. The controller moves where it is told and then refuses the
//! parts of that motion the world will not allow.

use runity::physics::{Character, CharacterSettings};
use runity::prelude::*;

struct Player {
    character: Character,
    yaw: f32,
    pitch: f32,
    /// Meshes and the props drawn around the scene.
    cube: Mesh,
    ramp: Mesh,
    ground: Mesh,
    props: Vec<(Vec3, Vec3, Color)>,
    crates: Vec<BodyHandle>,
    /// Scripted walk for a headless run, in seconds of (forward, turn).
    script: Vec<(f32, Vec3, f32)>,
    elapsed: f32,
}

impl Player {
    fn new() -> Self {
        Self {
            character: Character::new(CharacterSettings::default(), vec3(0.0, 0.2, 6.0)),
            yaw: 0.0,
            pitch: -0.18,
            cube: Mesh::cube(1.0),
            ramp: Mesh::cube(1.0),
            ground: Mesh::plane(60.0, 1),
            props: Vec::new(),
            crates: Vec::new(),
            // (until this many seconds, wish direction in local space, yaw):
            // walk up the steps, then turn and look back over the scene.
            script: vec![
                (2.6, vec3(0.0, 0.0, -1.0), 0.0),
                (3.4, Vec3::ZERO, 1.6),
                (9.0, Vec3::ZERO, 2.5),
            ],
            elapsed: 0.0,
        }
    }

    /// Where the player is looking.
    fn forward(&self) -> Vec3 {
        vec3(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
        .normalized()
    }

    /// A box that is both a physics body and something to draw.
    fn add_block(&mut self, engine: &mut Engine, centre: Vec3, half: Vec3, colour: Color) {
        engine
            .physics
            .add(RigidBody::fixed(Shape::cuboid(half)).with_position(centre));
        self.props.push((centre, half * 2.0, colour));
    }
}

impl Game for Player {
    fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
        engine.renderer.set_sky(Sky::new(SkyParams {
            sun_direction: vec3(0.45, 0.72, -0.53).normalized(),
            sun_irradiance: 4.2,
            ..SkyParams::default()
        }));
        engine.renderer.settings.shadows.extent = 30.0;
        engine.renderer.settings.ambient_intensity = 0.7;
        engine.camera.near = 0.05;
        engine.exposure = 0.9;

        engine.physics.add(RigidBody::fixed(Shape::ground()));

        // A few things to walk into, over and around: a kerb to step onto, a
        // stair, a low wall, and a ramp that is walkable at the bottom and
        // too steep at the top.
        self.add_block(
            engine,
            vec3(0.0, 0.15, 0.0),
            vec3(3.0, 0.15, 2.0),
            Color::rgb(0.45, 0.42, 0.38),
        );
        self.add_block(
            engine,
            vec3(0.0, 0.45, -2.6),
            vec3(3.0, 0.15, 0.6),
            Color::rgb(0.42, 0.39, 0.35),
        );
        self.add_block(
            engine,
            vec3(0.0, 0.75, -4.0),
            vec3(3.0, 0.15, 0.8),
            Color::rgb(0.39, 0.36, 0.33),
        );
        self.add_block(
            engine,
            vec3(-4.5, 1.0, -2.0),
            vec3(0.4, 1.0, 4.0),
            Color::rgb(0.30, 0.33, 0.36),
        );
        self.add_block(
            engine,
            vec3(5.0, 1.2, -1.0),
            vec3(1.6, 1.2, 1.6),
            Color::rgb(0.33, 0.30, 0.28),
        );

        // Crates that can be shoved about, so the controller's effect on the
        // simulation is visible rather than theoretical.
        for (index, offset) in [
            vec3(1.6, 0.0, 2.0),
            vec3(2.4, 0.0, 1.2),
            vec3(-2.2, 0.0, 1.6),
        ]
        .into_iter()
        .enumerate()
        {
            let handle = engine.physics.add(
                RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.35)))
                    .with_position(offset + vec3(0.0, 0.35 + index as f32 * 0.02, 0.0))
                    .with_mass(6.0),
            );
            self.crates.push(handle);
        }
        Ok(())
    }

    fn update(&mut self, engine: &mut Engine) {
        let dt = engine.time.delta();
        self.elapsed += dt;

        if engine.input.key_pressed(Key::Escape) {
            engine.quit();
            return;
        }

        // Looking: mouse when there is one, keys when there is not.
        let look = engine.input.mouse_delta();
        self.yaw += look.x * 0.004 + engine.input.axis(Key::Q, Key::E) * dt * 2.0;
        self.pitch = (self.pitch - look.y * 0.004).clamp(-1.4, 1.4);

        // Walking is in the plane, whatever the pitch: looking down does not
        // push the player into the floor.
        let forward = vec3(self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = vec3(forward.z, 0.0, -forward.x) * -1.0;
        let input = &engine.input;
        let mut wish = forward * (input.axis(Key::S, Key::W) + input.axis(Key::Down, Key::Up))
            + right * (input.axis(Key::A, Key::D) + input.axis(Key::Left, Key::Right));
        if wish.length_squared() > 1.0 {
            wish = wish.normalized();
        }
        let running = input.key_down(Key::LeftShift) || input.key_down(Key::RightShift);
        let speed = if running { 5.5 } else { 3.0 };

        if input.key_pressed(Key::Space) {
            self.character.jump();
        }

        // The controller runs on the fixed step, like everything else that
        // touches the physics world.
        let steps = ((dt / engine.time.fixed_delta).round() as u32).clamp(1, 4);
        let step_dt = dt / steps as f32;
        for _ in 0..steps {
            self.character
                .step(&mut engine.physics, wish * speed, step_dt);
        }

        // Shove the nearest crate: a ray from the eyes, the same way a game
        // would decide what the player is pointing at.
        if input.key_pressed(Key::F) {
            let ray = Ray::new(self.character.eyes(), self.forward(), 3.0);
            if let Some(hit) = engine.physics.cast_ray(&ray) {
                let direction = self.forward();
                if let Some(body) = engine.physics.get_mut(hit.handle) {
                    body.apply_impulse_at(direction * 12.0, hit.hit.point);
                }
            }
        }

        engine.camera.position = self.character.eyes();
        engine.camera.target = self.character.eyes() + self.forward();
    }

    fn render(&mut self, engine: &mut Engine) {
        engine.draw_pbr(
            &self.ground,
            Mat4::IDENTITY,
            &Material {
                base_color: Color::rgb(0.18, 0.21, 0.16),
                roughness: 0.95,
                ..Material::default()
            },
        );

        for (centre, size, colour) in &self.props {
            let model = Mat4::from_translation(*centre) * Mat4::from_scale(*size);
            engine.draw_pbr(
                &self.cube,
                model,
                &Material {
                    base_color: *colour,
                    roughness: 0.85,
                    ..Material::default()
                },
            );
        }

        let crates: Vec<(Mat4, bool)> = self
            .crates
            .iter()
            .filter_map(|handle| {
                let body = engine.physics.get(*handle)?;
                let transform =
                    Transform::from_position(body.position()).with_rotation(body.rotation());
                Some((
                    transform.matrix() * Mat4::from_scale(Vec3::splat(0.7)),
                    body.is_sleeping(),
                ))
            })
            .collect();
        for (model, sleeping) in crates {
            let colour = if sleeping {
                Color::rgb(0.42, 0.26, 0.12)
            } else {
                Color::rgb(0.62, 0.38, 0.16)
            };
            engine.draw_pbr(
                &self.ramp,
                model,
                &Material {
                    base_color: colour,
                    roughness: 0.8,
                    ..Material::default()
                },
            );
        }
    }
}

/// Plays the scripted walk, so a headless run produces a frame with the
/// player somewhere interesting.
struct Scripted {
    inner: Player,
}

impl Game for Scripted {
    fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
        self.inner.start(engine)
    }

    fn update(&mut self, engine: &mut Engine) {
        let dt = engine.time.delta();
        self.inner.elapsed += dt;
        let elapsed = self.inner.elapsed;

        let (wish, yaw) = self
            .inner
            .script
            .iter()
            .find(|(until, _, _)| elapsed < *until)
            .map(|(_, wish, yaw)| (*wish, *yaw))
            .unwrap_or((Vec3::ZERO, -1.4));
        self.inner.yaw = yaw;

        let forward = vec3(yaw.sin(), 0.0, -yaw.cos());
        let right = vec3(forward.z, 0.0, -forward.x) * -1.0;
        let direction = forward * -wish.z + right * wish.x;
        let steps = ((dt / engine.time.fixed_delta).round() as u32).clamp(1, 4);
        let step_dt = dt / steps as f32;
        for _ in 0..steps {
            self.inner
                .character
                .step(&mut engine.physics, direction * 3.0, step_dt);
        }

        engine.camera.position = self.inner.character.eyes();
        engine.camera.target = self.inner.character.eyes() + self.inner.forward();
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
        "runity — first person",
        dimension("RUNITY_WIDTH", 960),
        dimension("RUNITY_HEIGHT", 540),
    );
    let headless = std::env::var("RUNITY_HEADLESS").is_ok();

    let mut app = App::new(config);
    if headless {
        app = app
            .with_max_frames(150)
            .with_frame_delta(1.0 / 30.0)
            .with_target_fps(None);
    }

    let engine = if headless {
        app.run(Scripted {
            inner: Player::new(),
        })?
    } else {
        app.run(Player::new())?
    };

    if headless {
        let path =
            std::env::var("RUNITY_SCREENSHOT").unwrap_or_else(|_| "first_person.png".to_string());
        save_png(&path, &engine.framebuffer)?;
        println!(
            "wrote {path}; the camera ended at {:?}",
            engine.camera.position
        );
    }
    Ok(())
}
