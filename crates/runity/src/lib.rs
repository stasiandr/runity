//! runity: a guest, never a host.
//!
//! The engine never creates a window. It is handed a surface, a size and a
//! stream of input events, and it draws. That one rule is what lets the same
//! code sit under a native Swift editor on macOS, a `UIViewController` on
//! iOS, an `Activity` on Android and a plain desktop shell on Windows —
//! every one of those owns its own surface and will not give it up.
//!
//! What is here so far is the part that depends on none of that: the scene
//! file both the editor and the game read, and the asset format the importer
//! writes and the runtime opens.

// Re-exported so that a game or a tool uses the engine's version rather
// than pinning its own and discovering the mismatch at a function boundary.
pub use glam;

pub mod animation;
pub mod asset;
pub mod builtin;
pub mod gpu;
pub mod input;
pub mod library;
pub mod material;
pub mod physics;
pub mod render;
pub mod scene;
#[cfg(feature = "desktop-shell")]
pub mod shell;
pub mod surface;
pub mod time;
pub mod ui;
pub mod ui_render;
pub mod world;

pub use animation::{Channel, Clip, Joint, PoseTransform, Skeleton};
pub use asset::{AssetError, AssetId, Bounds, MeshAsset, Submesh, Vertex};
pub use gpu::{Gpu, GpuError, OffscreenTarget};
pub use input::{Input, InputEvent, Key, MouseButton};
pub use library::Library;
// `Surface` is not re-exported at the root: `wgpu::Surface` and ours would
// read the same in a `use` list and mean different things.
pub use material::{Material, Shading};
pub use physics::{BodyHandle, CharacterMove, CharacterSettings, PhysicsWorld, RayHit};
pub use render::{
    Camera, Draw, FogSettings, Frame, Lighting, MeshHandle, Renderer, ShadowSettings, TextureHandle,
};
pub use scene::{Body, EntityDesc, Fog, Scene, Sun, Transform};
pub use surface::{AcquiredFrame, SurfaceError};
pub use time::{Time, TimeSettings};
pub use ui::{Quad, TextRun, Ui};
pub use ui_render::UiRenderer;
pub use world::{build_frame, spawn_scene, Model, Shape, Surface, Textured};
