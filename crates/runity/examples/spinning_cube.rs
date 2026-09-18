//! A lit, textured crate over a checkerboard floor, with an orbiting camera —
//! the "does it all still work" demo.
//!
//! ```text
//! cargo run --release --example spinning_cube
//! RUNITY_HEADLESS=1 cargo run --release --example spinning_cube   # writes a PNG
//! ```
//!
//! Controls: arrows or WASD orbit the camera, Q/E zoom, Escape quits.
//! Debug views: 1 shaded, 2 wireframe, 3 depth, 4 overdraw, 5 albedo,
//! 6 normals, 7 material; N toggles the normal and axis overlay.
//!
//! Headless, the view is picked with `RUNITY_DEBUG_VIEW=shaded|wireframe|depth|
//! overdraw|albedo|normals|material`.

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
    show_overlay: bool,
}

impl Demo {
    fn new() -> Self {
        let crate_texture = Texture::from_fn(64, 64, |x, y| {
            // Plank-ish, generated so the example needs no asset files.
            let plank = (y / 16) % 2;
            let grain = ((x * 7 + y * 3) % 32) as f32 / 32.0;
            let shade = 0.55 + grain * 0.25;
            let edge = x % 16 == 0 || y % 16 == 0;
            if edge {
                Color::rgb(0.06, 0.03, 0.02)
            } else if plank == 0 {
                Color::rgb(0.45 * shade, 0.20 * shade, 0.07 * shade)
            } else {
                Color::rgb(0.34 * shade, 0.15 * shade, 0.05 * shade)
            }
        });

        let mut floor_texture = Texture::checker(
            128,
            16,
            Color::rgb(0.06, 0.065, 0.075),
            Color::rgb(0.22, 0.23, 0.25),
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
            show_overlay: false,
        }
    }

    fn cube_transform(&self) -> Mat4 {
        Mat4::from_rotation_y(self.angle) * Mat4::from_rotation_x(self.angle * 0.6)
    }
}

impl Game for Demo {
    fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
        engine.renderer.set_sky(Sky::new(SkyParams::default()));
        engine.camera.target = Vec3::new(0.0, 0.5, 0.0);
        Ok(())
    }

    fn update(&mut self, engine: &mut Engine) {
        let dt = engine.time.delta();
        self.angle += dt * 0.8;

        if engine.input.key_pressed(Key::Escape) {
            engine.quit();
            return;
        }
        for (key, view) in [
            (Key::Num1, DebugView::Shaded),
            (Key::Num2, DebugView::Wireframe),
            (Key::Num3, DebugView::Depth),
            (Key::Num4, DebugView::Overdraw),
            (Key::Num5, DebugView::Albedo),
            (Key::Num6, DebugView::Normals),
            (Key::Num7, DebugView::Material),
        ] {
            if engine.input.key_pressed(key) {
                engine.debug_view = view;
            }
        }
        if engine.input.key_pressed(Key::N) {
            self.show_overlay = !self.show_overlay;
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
        let floor = Material {
            roughness: 0.55,
            base_color_texture: Some(&self.floor_texture),
            ..Material::default()
        };
        engine.draw_pbr(
            &self.floor,
            Mat4::from_translation(Vec3::new(0.0, -0.75, 0.0)),
            &floor,
        );

        let wood = Material {
            roughness: 0.75,
            base_color_texture: Some(&self.crate_texture),
            ..Material::default()
        };
        engine.draw_pbr(&self.cube, self.cube_transform(), &wood);

        // Two spheres orbiting it: one polished metal, one painted.
        for (i, material) in [
            Material::metal(Color::rgb(0.95, 0.75, 0.35), 0.12),
            Material::dielectric(Color::rgb(0.06, 0.20, 0.45), 0.25),
        ]
        .into_iter()
        .enumerate()
        {
            let phase = self.angle * 1.6 + i as f32 * std::f32::consts::PI;
            let position = Vec3::new(
                phase.cos() * 2.3,
                0.35 + (phase * 2.0).sin() * 0.45,
                phase.sin() * 2.3,
            );
            engine.draw_pbr(&self.sphere, Mat4::from_translation(position), &material);
        }
    }

    fn overlay(&mut self, engine: &mut Engine) {
        if self.show_overlay {
            engine.draw_normals(
                &self.cube,
                self.cube_transform(),
                0.35,
                Color::rgb(0.2, 1.0, 0.4),
            );
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
        "albedo" => DebugView::Albedo,
        "normals" => DebugView::Normals,
        "material" => DebugView::Material,
        _ => DebugView::Shaded,
    }
}

/// Wraps the demo so a headless run can script what an interactive one does
/// with the keyboard.
struct Scripted {
    demo: Demo,
    view: DebugView,
    overlay: bool,
}

impl Game for Scripted {
    fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
        engine.debug_view = self.view;
        self.demo.show_overlay = self.overlay;
        self.demo.start(engine)
    }
    fn update(&mut self, engine: &mut Engine) {
        self.demo.update(engine);
    }
    fn render(&mut self, engine: &mut Engine) {
        self.demo.render(engine);
    }
    fn overlay(&mut self, engine: &mut Engine) {
        self.demo.overlay(engine);
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
        // Deterministic run, so the screenshot is always the same frame.
        app = app
            .with_max_frames(90)
            .with_frame_delta(1.0 / 60.0)
            .with_target_fps(None);
    }

    let engine = if headless {
        app.run(Scripted {
            demo: Demo::new(),
            view: debug_view_from_env(),
            overlay: std::env::var("RUNITY_SHOW_NORMALS").is_ok(),
        })?
    } else {
        app.run(Demo::new())?
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
