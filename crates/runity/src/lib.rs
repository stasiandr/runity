//! # runity
//!
//! A small game engine in Rust with no third-party dependencies at all: the
//! whole tree builds from `std` and nothing else.
//!
//! * [`runity_math`] — vectors, matrices, quaternions.
//! * [`runity_render`] — a software rasterizer with programmable vertex and
//!   fragment stages: the reference, and what runs with no display.
//! * [`runity_gpu`] — the same pictures on Metal, and the differential tests
//!   that say they are the same.
//! * [`runity_platform`] — windows and input, straight from the OS (the X11
//!   wire protocol over a Unix socket; `user32`/`gdi32` on Windows).
//! * [`runity_core`] — entities, timing, input state and the main loop.
//!
//! ```no_run
//! use runity::prelude::*;
//!
//! struct Spin {
//!     cube: Mesh,
//!     angle: f32,
//! }
//!
//! impl Game for Spin {
//!     fn update(&mut self, engine: &mut Engine) {
//!         self.angle += engine.time.delta();
//!     }
//!
//!     fn render(&mut self, engine: &mut Engine) {
//!         let model = Mat4::from_rotation_y(self.angle);
//!         let shader = engine.lit_shader(model);
//!         engine.draw(&self.cube, &shader);
//!     }
//! }
//!
//! # fn main() -> std::io::Result<()> {
//! App::new(WindowConfig::new("spin", 960, 540))
//!     .run(Spin { cube: Mesh::cube(1.0), angle: 0.0 })?;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub use runity_core as core;
pub use runity_gpu as gpu;
pub use runity_math as math;
pub use runity_platform as platform;
pub use runity_render as render;

/// Everything you normally need, in one `use`.
pub mod prelude {
    pub use runity_core::headless;
    pub use runity_core::{
        App, Camera, DebugView, Engine, Entity, Game, Input, Renderer, RunOptions, Time,
        Transform, UnsupportedView, World,
    };
    pub use runity_gpu::{GpuShader, PulseUniforms, ENGINE_SOURCE, PULSE_FRAGMENT, PULSE_VERTEX};
    pub use runity_math::{Mat4, Quat, Vec2, Vec3, Vec4};
    pub use runity_platform::{Event, HeadlessWindow, Key, MouseButton, Window, WindowConfig};
    pub use runity_render::png::{decode_png, encode_png, load_png, save_png, save_ppm};
    pub use runity_render::{debug, golden};
    pub use runity_render::{
        BasicShader, Blend, Color, CullMode, DirectionalLight, DrawStats, Filter, Framebuffer,
        Image, Mesh, PolygonMode, Rasterizer, Shader, Texture, UnlitShader, Varying, Vertex,
        VertexOutput, Wrap,
    };
}
