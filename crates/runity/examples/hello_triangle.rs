//! The smallest interesting program: one triangle, one hand-written shader.
//!
//! ```text
//! cargo run --release --example hello_triangle
//! RUNITY_HEADLESS=1 cargo run --release --example hello_triangle
//! RUNITY_RENDERER=cpu cargo run --release --example hello_triangle
//! ```
//!
//! In a window it runs on the GPU, because the shader below is written twice —
//! once in Rust for the rasterizer, once in Metal Shading Language for the
//! hardware. Writing it twice is the whole price of admission, and a
//! differential test in `runity-gpu` is what holds the two halves together.

use runity::prelude::*;

/// A shader is just a pair of functions. `vertex` puts a vertex in clip space
/// and returns whatever the fragment stage needs; the rasterizer interpolates
/// that value with perspective correction and calls `fragment` per pixel.
///
/// The GPU runs the other half of it, `pulse_vertex` / `pulse_fragment` in
/// `runity-gpu`'s `shader.metal` — this shader is the engine's own pulse, so
/// the Metal side is already written and [`GpuShader`] only has to point at
/// it and say what goes in the uniform block.
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
        Some(color.scale_rgb(self.pulse()))
    }
}

impl Gradient {
    /// A slow pulse, to prove the fragment stage is really running per frame.
    fn pulse(&self) -> f32 {
        0.75 + 0.25 * (self.time * 2.0).sin()
    }
}

/// The half of `Gradient` that runs on the hardware. There is no code here
/// because the arithmetic lives in `shader.metal`; what an implementation
/// says is *which* pair of functions, and what they are handed.
impl GpuShader for Gradient {
    const SOURCE: &'static str = ENGINE_SOURCE;
    const VERTEX: &'static str = PULSE_VERTEX;
    const FRAGMENT: &'static str = PULSE_FRAGMENT;
    type Uniforms = PulseUniforms;

    fn uniforms(&self) -> PulseUniforms {
        PulseUniforms::new(self.mvp, self.pulse())
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
    engine.rasterizer.cull = CullMode::None;

    if headless {
        let path =
            std::env::var("RUNITY_SCREENSHOT").unwrap_or_else(|_| "triangle.png".to_string());
        save_png(&path, &engine.framebuffer)?;
        println!("wrote {path}");
    }
    Ok(())
}
