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

pub mod asset;
pub mod gpu;
pub mod library;
pub mod render;
pub mod scene;
pub mod world;

pub use asset::{AssetError, AssetId, Bounds, MeshAsset, Submesh, Vertex};
pub use gpu::{Gpu, GpuError, OffscreenTarget};
pub use library::Library;
pub use render::{Camera, Draw, FogSettings, Frame, Lighting, MeshHandle, Renderer};
pub use scene::{Body, EntityDesc, Fog, Scene, Sun, Transform};
pub use world::{build_frame, spawn_scene, Model, Tint};
