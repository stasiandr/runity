//! # runity
//!
//! A small game engine in Rust with no third-party dependencies at all: the
//! whole tree builds from `std` and nothing else.
//!
//! * [`runity_math`] — vectors, matrices, quaternions.
//! * [`runity_render`] — a software rasterizer with programmable vertex and
//!   fragment stages.
//! * [`runity_platform`] — windows and input, straight from the OS (the X11
//!   wire protocol over a Unix socket; `user32`/`gdi32` on Windows).
//! * [`runity_physics`] — rigid bodies, collision and the simulation step.
//! * [`runity_ai`] — spatial queries, navigation, steering and decisions.
//! * [`runity_serialize`] — versioned binary encoding shared by saves, the
//!   network and snapshot tests.
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
//!         engine.draw_pbr(&self.cube, model, &Material::metal(Color::WHITE, 0.25));
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

pub use runity_ai as ai;
pub use runity_core as core;
pub use runity_math as math;
pub use runity_physics as physics;
pub use runity_platform as platform;
pub use runity_render as render;
pub use runity_serialize as serialize;

/// Everything you normally need, in one `use`.
pub mod prelude {
    pub use runity_ai::{
        arrive, choose, choose_near_best, curve, score, seek, separation, Awareness, FlowField,
        Locomotion, NavGrid, Path, PathFinder, PathFollower, PathSettings, Senses, SpatialGrid,
    };
    pub use runity_core::headless;
    pub use runity_core::{
        App, Calendar, Camera, Date, DebugView, Despawned, Engine, Entity, Game, Input, Mut,
        RunOptions, Season, Time, Transform, World, WorldClock,
    };
    pub use runity_math::{vec2, vec3, vec4, Fbm, Mat4, Noise, Quat, Rng, Vec2, Vec3, Vec4};
    pub use runity_physics::{
        BodyHandle, BodyType, ContactEvent, PhysicsStats, PhysicsWorld, Ray, RayCast, RigidBody,
        Shape,
    };
    pub use runity_platform::{Event, HeadlessWindow, Key, MouseButton, Window, WindowConfig};
    pub use runity_render::png::{decode_png, encode_png, load_png, save_png, save_ppm};
    pub use runity_render::{debug, golden};
    pub use runity_render::{
        Blend, Bloom, BloomSettings, CameraView, Color, CullMode, DrawStats, Filter, Framebuffer,
        GBuffer, GeometryShader, Image, Light, LightKind, Material, Mesh, OcclusionBuffer,
        PolygonMode, PostSettings, Rasterizer, RenderSettings, Renderer, Shader, ShadowSettings,
        Sky, SkyParams, SsaoSettings, SsrSettings, Surface, Texture, ToneMap, UnlitShader, Varying,
        Vertex, VertexOutput, Wrap,
    };
    pub use runity_serialize::{
        from_bytes, serializable, serializable_enum, to_bytes, Archive, Deserialize, Reader,
        Serialize, Writer,
    };
}
