//! runity: a guest, never a host.
//!
//! The engine never creates a window. It is handed a surface, a size and a
//! stream of input events, and it draws. That one rule is what lets the same
//! code sit under the editor, a plain desktop shell, and later a
//! `UIViewController` on iOS or an `Activity` on Android — every one of
//! those owns its own surface and will not give it up.
//!
//! What is here so far is the part that depends on none of that: the scene
//! file both the editor and the game read, and the asset format the importer
//! writes and the runtime opens.

// Re-exported so that a game or a tool uses the engine's version rather
// than pinning its own and discovering the mismatch at a function boundary.
// `hecs` for the same reason and one more: an `Entity` from a different
// version of it is a different type, and the engine's world is the one that
// issues them.
pub use glam;
pub use hecs;
pub use ron;

pub mod actions;
pub mod animation;
pub mod animator;
pub mod animgraph;
pub mod asset;
#[cfg(feature = "audio")]
pub mod audio;
#[cfg(feature = "physics")]
pub mod bench;
pub mod builtin;
pub mod components;
pub mod decals;
pub mod edit;
pub mod gizmo;
pub mod gpu;
pub mod id;
pub mod input;
pub mod lan;
pub mod layers;
pub mod lens;
pub mod library;
pub mod lights;
pub mod live;
pub mod material;
pub mod merge;
#[cfg(feature = "physics")]
pub mod navigation;
pub mod net;
pub mod particles;
pub mod party;
pub mod perf;
#[cfg(feature = "physics")]
pub mod physics;
pub mod player_prefs;
pub mod post;
pub mod prefab;
pub mod project;
pub mod query;
pub mod ray;
pub mod reflections;
pub mod refs;
pub mod relay;
pub mod render;
pub mod ron_edit;
mod ron_text;
pub mod routes;
pub mod save;
pub mod scene;
pub mod screen;
pub mod shape;
#[cfg(feature = "desktop-shell")]
pub mod shell;
pub mod spelling;
pub mod ssao;
#[cfg(feature = "steam")]
pub mod steam;
pub mod strings;
pub mod surface;
pub mod time;
pub mod timers;
pub mod tuned;
pub mod ui;
pub mod ui_render;
pub mod volume;
pub mod widgets;
pub mod world;

pub use animation::{Channel, Clip, Joint, PoseTransform, Skeleton};
pub use animator::{advance_animations, Animator, Playing};
pub use asset::{
    AssetError, AssetId, Bounds, MaterialAsset, MeshAsset, SoundAsset, Submesh, Vertex,
};
#[cfg(feature = "audio")]
pub use audio::{Audio, Falloff};
pub use edit::History;
pub use gizmo::{Drag, GizmoStyle, Handle};
pub use gpu::{Gpu, GpuError, OffscreenTarget};
pub use id::{EntityId, EntityRef};
pub use input::{Input, InputEvent, Key, MouseButton};
pub use library::{Library, Reloaded};
pub use refs::{AssetLink, MaterialLink, ModelLink, PrefabLink, SceneLink, SoundLink, TextureLink};
// `Surface` is not re-exported at the root: `wgpu::Surface` and ours would
// read the same in a `use` list and mean different things.
pub use actions::{Actions, Binding};
pub use components::{ComponentProblem, Components};
pub use live::{Instance, LiveScene, Reload, Spawned};
pub use material::{Material, Shading};
pub use perf::{FrameSummary, FrameTimes};
#[cfg(feature = "physics")]
pub use physics::{BodyHandle, PhysicsWorld, RayHit};
pub use prefab::{instantiate, Instanced, Prefabs};
pub use project::{Project, ProjectError};
pub use render::{
    Camera, Draw, FogSettings, Frame, Lighting, MeshHandle, Renderer, ShadowSettings, TextureHandle,
};
pub use scene::{Along, Body, EntityDesc, Fog, Scene, Spline, Sun, Transform, View};
pub use surface::{AcquiredFrame, SurfaceError};
pub use time::{Time, TimeSettings};
pub use tuned::Tuned;
pub use ui::{Quad, TextRun, Ui};
pub use ui_render::UiRenderer;
pub use widgets::{Rect, Widgets};
pub use world::{
    build_frame, build_frame_where, captured_view, patch_scene, scene_camera, scene_fog,
    scene_lighting, spawn_scene, spawn_scene_with, Model, Patched, Posed, SceneId, Shape, Surface,
    Textured,
};
