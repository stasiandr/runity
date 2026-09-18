//! Everything the renderer can do, in one frame: PBR materials under a
//! generated sky, shadows, ambient occlusion, screen-space reflections, bloom
//! and a lens treatment.
//!
//! ```text
//! cargo run --release --example showcase
//! RUNITY_HEADLESS=1 cargo run --release --example showcase       # writes showcase.png
//! RUNITY_HEADLESS=1 RUNITY_SUPERSAMPLE=2 cargo run --release --example showcase
//! ```
//!
//! Interactive: arrows or WASD orbit, Q/E zoom, 1-8 pick a debug view,
//! F1 toggles ambient occlusion, F2 reflections, F3 bloom, F4 the lens
//! treatment, F5 shadows.

use runity::prelude::*;

/// The material chart: roughness across, metal on the back row.
const ROUGHNESS_STEPS: usize = 7;

struct Showcase {
    sphere: Mesh,
    floor: Mesh,
    cube: Mesh,
    torus: Mesh,
    lamp: Mesh,
    floor_texture: Texture,
    bumps: Texture,
    crate_texture: Texture,
    angle: f32,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Showcase {
    fn new() -> Self {
        // A dark tiled floor: dark enough that reflections dominate, which is
        // what makes a polished surface read as polished.
        // Dark enough that reflections carry the surface, light enough that
        // the diffuse term — and therefore the shadows — are still visible.
        let mut floor_texture = Texture::checker(
            256,
            32,
            Color::rgb(0.045, 0.048, 0.056),
            Color::rgb(0.12, 0.125, 0.14),
        );
        floor_texture.wrap = Wrap::Repeat;

        // A procedural normal map: gentle dimples, so there is something for
        // the tangent frame to do.
        let bumps = Texture::from_fn(128, 128, |x, y| {
            let u = x as f32 / 128.0 * core::f32::consts::TAU * 6.0;
            let v = y as f32 / 128.0 * core::f32::consts::TAU * 6.0;
            let height = u.sin() * v.sin() * 0.5;
            // Store the analytic gradient as a tangent-space normal.
            let nx = -u.cos() * v.sin() * 0.5;
            let ny = -u.sin() * v.cos() * 0.5;
            let normal = Vec3::new(nx, ny, 1.0).normalized();
            let _ = height;
            Color::rgb(
                normal.x * 0.5 + 0.5,
                normal.y * 0.5 + 0.5,
                normal.z * 0.5 + 0.5,
            )
        });

        let crate_texture = Texture::from_fn(64, 64, |x, y| {
            let plank = (y / 16) % 2;
            let grain = ((x * 7 + y * 3) % 32) as f32 / 32.0;
            let shade = 0.55 + grain * 0.25;
            if x % 16 == 0 || y % 16 == 0 {
                Color::rgb(0.05, 0.025, 0.012)
            } else if plank == 0 {
                Color::rgb(0.32 * shade, 0.14 * shade, 0.05 * shade)
            } else {
                Color::rgb(0.24 * shade, 0.10 * shade, 0.04 * shade)
            }
        });

        Self {
            sphere: Mesh::sphere(0.42, 40, 28),
            floor: Mesh::plane(60.0, 1),
            cube: Mesh::cube(0.9),
            torus: Mesh::torus(0.65, 0.22, 48, 24),
            lamp: Mesh::sphere(0.18, 24, 16),
            floor_texture,
            bumps,
            crate_texture,
            angle: 0.0,
            yaw: 0.58,
            pitch: 0.31,
            distance: 9.2,
        }
    }
}

impl Game for Showcase {
    fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
        // A low sun: long shadows, warm key light, cool sky fill.
        engine.renderer.set_sky(Sky::new(SkyParams {
            // Low and off to the side, so the shadows stretch across the
            // frame instead of hiding behind what casts them.
            sun_direction: Vec3::new(0.72, 0.62, -0.42).normalized(),
            // A strong key against a held-back sky: that ratio is what makes
            // shadows read as shadows rather than as slightly darker floor.
            sun_irradiance: 4.6,
            intensity: 0.7,
            ..SkyParams::golden_hour()
        }));
        engine.renderer.settings.post = PostSettings::cinematic();
        engine.renderer.settings.shadows.extent = 14.0;
        engine.renderer.settings.ssao.radius = 0.35;
        // The sky fills in a lot at golden hour; hold it back so the sun's
        // shadows have somewhere to fall.
        engine.renderer.settings.ambient_intensity = 0.75;
        // The sky is the brightest thing in frame by a wide margin; expose for
        // the objects instead and let the horizon roll off.
        engine.exposure = 0.62;

        // A small warm lamp near the floor, to have something that is not the
        // sun and something for bloom to pick up.
        engine.renderer.lights.push(Light::point(
            Vec3::new(-3.4, 0.55, 2.4),
            Color::rgb(1.0, 0.45, 0.18),
            9.0,
            7.0,
        ));

        engine.camera.target = Vec3::new(0.0, 0.55, 0.0);
        engine.camera.fov_y = 45f32.to_radians();
        Ok(())
    }

    fn update(&mut self, engine: &mut Engine) {
        let dt = engine.time.delta();
        self.angle += dt * 0.5;

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
            (Key::Num8, DebugView::Occlusion),
        ] {
            if engine.input.key_pressed(key) {
                engine.debug_view = view;
            }
        }
        let settings = &mut engine.renderer.settings;
        if engine.input.key_pressed(Key::F1) {
            settings.ssao.enabled = !settings.ssao.enabled;
        }
        if engine.input.key_pressed(Key::F2) {
            settings.ssr.enabled = !settings.ssr.enabled;
        }
        if engine.input.key_pressed(Key::F3) {
            settings.bloom.enabled = !settings.bloom.enabled;
        }
        if engine.input.key_pressed(Key::F4) {
            settings.post = if settings.post.vignette > 0.0 {
                PostSettings::default()
            } else {
                PostSettings::cinematic()
            };
        }
        if engine.input.key_pressed(Key::F5) {
            settings.shadows.enabled = !settings.shadows.enabled;
        }

        let input = &engine.input;
        self.yaw += (input.axis(Key::Left, Key::Right) + input.axis(Key::A, Key::D)) * dt * 1.2;
        self.pitch = (self.pitch
            + (input.axis(Key::Down, Key::Up) + input.axis(Key::S, Key::W)) * dt)
            .clamp(0.02, 1.2);
        self.distance = (self.distance + input.axis(Key::E, Key::Q) * dt * 6.0).clamp(3.0, 24.0);
        engine.camera.orbit(self.yaw, self.pitch, self.distance);
    }

    fn render(&mut self, engine: &mut Engine) {
        // Floor: dark, polished, and large enough to run out of the frame.
        engine.draw_pbr(
            &self.floor,
            Mat4::IDENTITY,
            &Material {
                roughness: 0.26,
                base_color_texture: Some(&self.floor_texture),
                uv_scale: Vec2::splat(6.0),
                ..Material::default()
            },
        );

        // The chart: metals on the back row, dielectrics in front, roughness
        // rising left to right. This is the picture that tells you whether a
        // PBR implementation is correct.
        for step in 0..ROUGHNESS_STEPS {
            let t = step as f32 / (ROUGHNESS_STEPS - 1) as f32;
            let x = (t - 0.5) * 5.0;
            let roughness = 0.04 + t * 0.7;

            engine.draw_pbr(
                &self.sphere,
                Mat4::from_translation(Vec3::new(x, 0.42, -1.9)),
                &Material::metal(Color::rgb(0.94, 0.78, 0.42), roughness),
            );
            engine.draw_pbr(
                &self.sphere,
                Mat4::from_translation(Vec3::new(x, 0.42, -0.55)),
                &Material::dielectric(Color::rgb(0.55, 0.10, 0.09), roughness),
            );
        }

        // Hero objects: a chrome torus, and a wooden crate with a normal map.
        let spin = Mat4::from_rotation_y(self.angle);
        engine.draw_pbr(
            &self.torus,
            Mat4::from_translation(Vec3::new(2.3, 0.72, 1.6)) * spin * Mat4::from_rotation_x(0.9),
            &Material::metal(Color::rgb(0.95, 0.96, 0.98), 0.08),
        );
        engine.draw_pbr(
            &self.cube,
            Mat4::from_translation(Vec3::new(-2.2, 0.45, 1.5))
                * Mat4::from_rotation_y(self.angle * -0.6),
            &Material {
                roughness: 0.65,
                base_color_texture: Some(&self.crate_texture),
                normal_texture: Some(&self.bumps),
                ..Material::default()
            },
        );

        // The lamp itself: emissive, so it blooms and shows up in reflections.
        engine.draw_pbr(
            &self.lamp,
            Mat4::from_translation(Vec3::new(-3.4, 0.55, 2.4)),
            &Material {
                base_color: Color::BLACK,
                emissive: Color::rgb(7.0, 3.0, 1.1),
                ..Material::default()
            },
        );
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
        "occlusion" | "ao" => DebugView::Occlusion,
        _ => DebugView::Shaded,
    }
}

/// Applies the environment's choices before the first frame.
struct Configured {
    inner: Showcase,
    view: DebugView,
}

impl Game for Configured {
    fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
        self.inner.start(engine)?;
        engine.debug_view = self.view;
        let off = |name: &str| std::env::var(name).is_ok();
        if off("RUNITY_NO_SSAO") {
            engine.renderer.settings.ssao.enabled = false;
        }
        if off("RUNITY_NO_SSR") {
            engine.renderer.settings.ssr.enabled = false;
        }
        if off("RUNITY_NO_BLOOM") {
            engine.renderer.settings.bloom.enabled = false;
        }
        if off("RUNITY_NO_POST") {
            engine.renderer.settings.post = PostSettings::default();
        }
        if off("RUNITY_NO_SHADOWS") {
            engine.renderer.settings.shadows.enabled = false;
        }
        Ok(())
    }
    fn update(&mut self, engine: &mut Engine) {
        self.inner.update(engine);
    }
    fn render(&mut self, engine: &mut Engine) {
        self.inner.render(engine);
    }
}

fn main() -> std::io::Result<()> {
    let number = |name: &str, fallback: u32| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(fallback)
    };
    let headless = std::env::var("RUNITY_HEADLESS").is_ok();
    let config = WindowConfig::new(
        "runity — showcase",
        number("RUNITY_WIDTH", 1280),
        number("RUNITY_HEIGHT", 720),
    );

    let mut app = App::new(config).with_supersampling(number("RUNITY_SUPERSAMPLE", 1));
    if headless {
        // One frame, at a fixed time, so the image is reproducible.
        app = app
            .with_max_frames(1)
            .with_frame_delta(0.9)
            .with_target_fps(None);
    }

    let engine = app.run(Configured {
        inner: Showcase::new(),
        view: debug_view_from_env(),
    })?;

    if headless {
        let path =
            std::env::var("RUNITY_SCREENSHOT").unwrap_or_else(|_| "showcase.png".to_string());
        let supersample = number("RUNITY_SUPERSAMPLE", 1) as usize;
        let frame = engine.framebuffer.downsample(supersample);
        save_png(&path, &frame)?;
        let stats = engine.frame_stats();
        println!(
            "wrote {path} ({}x{}, {}x supersampled): {} triangles, {} fragments shaded",
            frame.width(),
            frame.height(),
            supersample,
            stats.triangles_in,
            stats.fragments_shaded,
        );
    }
    Ok(())
}
