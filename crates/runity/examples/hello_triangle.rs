//! The smallest interesting program: one triangle, one hand-written shader.
//!
//! ```text
//! cargo run --release --example hello_triangle
//! RUNITY_HEADLESS=1 cargo run --release --example hello_triangle
//! ```

use runity::prelude::*;

/// A shader is just a pair of functions. `vertex` puts a vertex in clip space
/// and returns whatever the fragment stage needs; the rasterizer interpolates
/// that value with perspective correction and calls `fragment` per pixel.
struct Gradient {
    mvp: Mat4,
    time: f32,
}

impl Shader for Gradient {
    /// Our interpolant: the vertex color.
    type Varying = Color;

    fn vertex(&self, vertex: &Vertex) -> VertexOutput<Color> {
        VertexOutput {
            clip_position: self.mvp.transform_point(vertex.position),
            varying: vertex.color,
        }
    }

    fn fragment(&self, color: &Color) -> Option<Color> {
        // A slow pulse, to prove the fragment stage is really running per frame.
        let pulse = 0.75 + 0.25 * (self.time * 2.0).sin();
        Some(color.scale_rgb(pulse))
    }
}

struct Demo {
    triangle: Mesh,
}

impl Game for Demo {
    fn update(&mut self, engine: &mut Engine) {
        if engine.input.key_pressed(Key::Escape) {
            engine.quit();
        }
    }

    fn render(&mut self, engine: &mut Engine) {
        let model = Mat4::from_rotation_y(engine.time.elapsed());
        let shader = Gradient {
            mvp: engine.view_projection() * model,
            time: engine.time.elapsed(),
        };
        engine.draw(&self.triangle, &shader);
    }
}

fn main() -> std::io::Result<()> {
    let vertices = vec![
        Vertex::new(Vec3::new(-1.0, -0.8, 0.0), Vec3::Z, Vec2::new(0.0, 1.0))
            .with_color(Color::rgb(0.95, 0.25, 0.30)),
        Vertex::new(Vec3::new(1.0, -0.8, 0.0), Vec3::Z, Vec2::new(1.0, 1.0))
            .with_color(Color::rgb(0.25, 0.85, 0.40)),
        Vertex::new(Vec3::new(0.0, 1.0, 0.0), Vec3::Z, Vec2::new(0.5, 0.0))
            .with_color(Color::rgb(0.30, 0.45, 0.98)),
    ];
    let triangle = Mesh::new(vertices, vec![0, 1, 2]);

    let mut app = App::new(WindowConfig::new("runity — hello triangle", 800, 600));
    let headless = std::env::var("RUNITY_HEADLESS").is_ok();
    if headless {
        app = app
            .with_max_frames(1)
            .with_frame_delta(0.25)
            .with_target_fps(None);
    }

    let mut engine = app.run(Demo { triangle })?;
    // A triangle has no back, so let both sides show.
    engine.renderer.rasterizer.cull = CullMode::None;

    if headless {
        let path =
            std::env::var("RUNITY_SCREENSHOT").unwrap_or_else(|_| "triangle.png".to_string());
        save_png(&path, &engine.framebuffer)?;
        println!("wrote {path}");
    }
    Ok(())
}
