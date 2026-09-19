//! The engine's "does it all work" demo: a lit, textured cube over a
//! checkerboard floor, with an orbiting camera.
//!
//! ```text
//! cargo run --release --example spinning_cube
//! RUNITY_HEADLESS=1 cargo run --release --example spinning_cube   # writes a PNG
//! ```
//!
//! In a window it runs on Metal; `RUNITY_RENDERER=cpu` puts it back on the
//! software rasterizer, which is also what a headless run uses.
//!
//! Controls: arrows or WASD orbit the camera, Q/E zoom, Escape quits.
//! Debug views: 1 shaded, 2 wireframe, 3 depth buffer, 4 overdraw; N overlays
//! vertex normals and the world axes.
//!
//! Headless, the view is picked with `RUNITY_DEBUG_VIEW=shaded|wireframe|depth|overdraw`.

use runity::prelude::*;

struct Demo {
    cube: Mesh,
    floor: Mesh,
    sphere: Mesh,
    crate_texture: Texture,
    floor_texture: Texture,
    angle: f32,
    yaw: f32,
    pitch: f32,
    distance: f32,
    show_normals: bool,
}

impl Demo {
    fn new() -> Self {
        let crate_texture = Texture::from_fn(64, 64, |x, y| {
            // A plank-ish pattern, generated so the example needs no asset files.
            let plank = (y / 16) % 2;
            let grain = ((x * 7 + y * 3) % 32) as f32 / 32.0;
            let shade = 0.55 + grain * 0.25;
            let edge = x % 16 == 0 || y % 16 == 0;
            if edge {
                Color::rgb(0.25, 0.16, 0.10)
            } else if plank == 0 {
                Color::rgb(0.72 * shade, 0.45 * shade, 0.22 * shade)
            } else {
                Color::rgb(0.62 * shade, 0.38 * shade, 0.18 * shade)
            }
        });

        let mut floor_texture = Texture::checker(
            128,
            16,
            Color::rgb(0.20, 0.22, 0.26),
            Color::rgb(0.32, 0.34, 0.40),
        );
        floor_texture.wrap = Wrap::Repeat;

        Self {
            cube: Mesh::cube(1.4),
            floor: Mesh::plane(14.0, 1),
            sphere: Mesh::sphere(0.55, 28, 18),
            crate_texture,
            floor_texture,
            angle: 0.0,
            yaw: 0.6,
            pitch: 0.45,
            distance: 6.0,
            show_normals: false,
        }
    }
}

impl Game for Demo {
    fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
        engine.clear_color = Color::rgb(0.04, 0.05, 0.08);
        engine.light = DirectionalLight {
            direction: Vec3::new(-0.5, -0.85, -0.35).normalized(),
            color: Color::rgb(1.0, 0.96, 0.88),
            intensity: 1.15,
        };
        engine.camera.target = Vec3::new(0.0, 0.5, 0.0);
        Ok(())
    }

    fn update(&mut self, engine: &mut Engine) {
        let dt = engine.time.delta();
        self.angle += dt * 0.8;

        let input = &engine.input;
        if input.key_pressed(Key::Escape) {
            engine.quit();
            return;
        }
        // One switch drives every debug view; the render code below is unaware.
        let requested = [
            (Key::Num1, DebugView::Shaded),
            (Key::Num2, DebugView::Wireframe),
            (Key::Num3, DebugView::Depth),
            (Key::Num4, DebugView::Overdraw),
        ]
        .into_iter()
        .find(|(key, _)| input.key_pressed(*key))
        .map(|(_, view)| view);
        let toggle_normals = input.key_pressed(Key::N);
        if let Some(view) = requested {
            // Two of the four are CPU-only; on the GPU the switch says so and
            // the view stays where it was.
            if let Err(refused) = engine.set_debug_view(view) {
                println!("{refused}");
            }
        }
        if toggle_normals {
            self.show_normals = !self.show_normals;
        }

        let input = &engine.input;
        self.yaw += (input.axis(Key::Left, Key::Right) + input.axis(Key::A, Key::D)) * dt * 1.5;
        self.pitch = (self.pitch
            + (input.axis(Key::Down, Key::Up) + input.axis(Key::S, Key::W)) * dt * 1.2)
            .clamp(-1.2, 1.4);
        self.distance = (self.distance + input.axis(Key::E, Key::Q) * dt * 6.0).clamp(2.5, 20.0);

        engine.camera.orbit(self.yaw, self.pitch, self.distance);
    }

    fn render(&mut self, engine: &mut Engine) {
        // Floor.
        let model = Mat4::from_translation(Vec3::new(0.0, -0.75, 0.0));
        let mut shader = engine.lit_shader(model);
        shader.texture = Some(&self.floor_texture);
        shader.specular_strength = 0.05;
        engine.draw(&self.floor, &shader);

        // Spinning crate.
        let model = Mat4::from_rotation_y(self.angle)
            * Mat4::from_rotation_x(self.angle * 0.6)
            * Mat4::from_translation(Vec3::ZERO);
        let mut shader = engine.lit_shader(model);
        shader.texture = Some(&self.crate_texture);
        engine.draw(&self.cube, &shader);

        // Two spheres orbiting it, to show depth sorting and specular highlights.
        for (i, tint) in [Color::rgb(0.9, 0.3, 0.35), Color::rgb(0.35, 0.65, 0.95)]
            .into_iter()
            .enumerate()
        {
            let phase = self.angle * 1.6 + i as f32 * std::f32::consts::PI;
            let position = Vec3::new(
                phase.cos() * 2.3,
                0.35 + (phase * 2.0).sin() * 0.45,
                phase.sin() * 2.3,
            );
            let mut shader = engine.lit_shader(Mat4::from_translation(position));
            shader.base_color = tint;
            shader.specular_strength = 0.6;
            shader.shininess = 64.0;
            engine.draw(&self.sphere, &shader);
        }

        // Overlays go on last: they ignore depth and sit on top of the frame.
        if self.show_normals {
            let model = Mat4::from_rotation_y(self.angle) * Mat4::from_rotation_x(self.angle * 0.6);
            engine.draw_normals(&self.cube, model, 0.35, Color::rgb(0.2, 1.0, 0.4));
            engine.draw_axes(1.5);
        }
    }
}

fn debug_view_from_env() -> DebugView {
    match std::env::var("RUNITY_DEBUG_VIEW")
        .unwrap_or_default()
        .as_str()
    {
        "wireframe" => DebugView::Wireframe,
        "depth" => DebugView::Depth,
        "overdraw" => DebugView::Overdraw,
        _ => DebugView::Shaded,
    }
}

fn main() -> std::io::Result<()> {
    // The size is overridable so the same example can produce documentation
    // screenshots at whatever resolution is wanted.
    let dimension = |name: &str, fallback: u32| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(fallback)
    };
    let config = WindowConfig::new(
        "runity — spinning cube",
        dimension("RUNITY_WIDTH", 960),
        dimension("RUNITY_HEIGHT", 540),
    );
    let headless = std::env::var("RUNITY_HEADLESS").is_ok();

    let mut app = App::new(config);
    if headless {
        // Deterministic run so the screenshot is always the same frame.
        app = app
            .with_max_frames(90)
            .with_frame_delta(1.0 / 60.0)
            .with_target_fps(None);
    }

    let mut demo = Demo::new();
    demo.show_normals = std::env::var("RUNITY_SHOW_NORMALS").is_ok();

    let engine = if headless {
        // Headless runs are scripted rather than interactive.
        let view = debug_view_from_env();
        struct Configured(Demo, DebugView);
        impl Game for Configured {
            fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
                if let Err(refused) = engine.set_debug_view(self.1) {
                    println!("{refused}");
                }
                self.0.start(engine)
            }
            fn update(&mut self, engine: &mut Engine) {
                self.0.update(engine);
            }
            fn render(&mut self, engine: &mut Engine) {
                self.0.render(engine);
            }
        }
        app.run(Configured(demo, view))?
    } else {
        app.run(demo)?
    };

    if headless {
        let path = std::env::var("RUNITY_SCREENSHOT").unwrap_or_else(|_| "cube.png".to_string());
        save_png(&path, &engine.framebuffer)?;
        let stats = engine.frame_stats();
        println!(
            "wrote {path} ({}x{}) after {} frames; last frame: {} triangles in, {} rasterized, {} fragments",
            engine.framebuffer.width(),
            engine.framebuffer.height(),
            engine.time.frame(),
            stats.triangles_in,
            stats.triangles_rasterized,
            stats.fragments_written,
        );
    }
    Ok(())
}
