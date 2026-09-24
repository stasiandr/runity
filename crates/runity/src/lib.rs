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
pub use runity_core::impl_parts;

pub mod actions;
pub mod animation;
pub mod appearance;
pub mod animator;
pub mod animgraph;
/// The asset archive as the core has it: its header, its ID.
pub use runity_core::asset as asset_core;
pub mod mesh_asset;
mod asset_tests;

/// Assets: the core's archive and IDs, and every module's formats, under
/// one name as before they were cut apart.
pub mod asset {
    pub use crate::asset_core::*;
    pub use crate::material::{ArchivedMaterialAsset, MaterialAsset};
    pub use crate::mesh_asset::*;
    pub use crate::sound::{ArchivedSoundAsset, SoundAsset, LONG_SOUND_SECONDS};
}
pub mod atmosphere;
#[cfg(feature = "audio")]
pub mod audio;
#[cfg(feature = "physics")]
pub mod bench;
pub mod builtin;
pub mod clouds;
pub use runity_core::components;
pub use runity_core::crash;
pub mod decals;
pub mod dialogue;
#[cfg(feature = "discord")]
pub mod discord;
pub use runity_core::edit;
pub mod exposure;
pub mod floaters;
pub mod foliage;
pub mod footprints;
pub mod gizmo;
pub mod gpu;
pub mod graph_text;
pub use runity_core::id;
pub use runity_core::input;
pub mod lan;
pub use runity_core::layers;
pub mod lens;
pub use runity_core::library;
pub use runity_core::links;
pub mod lights;
pub mod live;
pub mod material;
pub use runity_core::merge;
pub mod moods;
pub mod motion;
#[cfg(feature = "physics")]
pub mod navigation;
pub mod net;
pub mod particles;
pub use runity_core::parts;
pub mod party;
pub use runity_core::perf;
#[cfg(feature = "physics")]
pub mod physics;
pub use runity_core::player_prefs;
pub mod post;
pub use runity_core::prefab;
pub use runity_core::project;
pub mod query;
pub mod ray;
pub mod reflections;
pub mod refs;
pub mod relay;
pub mod render;
#[cfg(feature = "reports")]
pub mod reports;
pub use runity_core::ron_edit;
pub use runity_core::ron_text;
pub mod routes;
/// Saving a game in progress, as the core has it.
pub use runity_core::save as save_core;

/// Saving a game in progress, and the report a running game sends the
/// editor: the core's save with what the modules add to it.
pub mod save {
    pub use crate::animgraph::animator_trails;
    pub use crate::net::net_lines;
    pub use crate::save_core::*;

    /// Write down a world that started from `scene`, with each animated
    /// entity's graph state.
    pub fn capture(
        world: &hecs::World,
        components: &crate::components::Components,
        scene: &crate::scene::Scene,
    ) -> SaveGame {
        capture_with(world, components, scene, &crate::animgraph::animator_state)
    }
}
/// The scene file as the core has it: a line's identity, place and tree,
/// its modules' fields as parts (docs/modules.md).
pub use runity_core::scene as scene_core;
pub mod body;
pub use runity_core::defaults;
pub mod look;
pub mod sound;
pub mod spline;
mod scene_tests;

/// The scene file: the core's lines and scenes, and every module's types of
/// the fields on them, under one name as before they were cut apart.
pub mod scene {
    pub use crate::body::*;
    pub use crate::look::*;
    pub use crate::motion::{AnimatorRef, BoneName};
    pub use crate::routes::{Route, RouteEnds};
    pub use crate::scene_core::*;
    pub use crate::sound::*;
    pub use crate::spline::*;

    /// Every field of a line, an override or a scene's look the modules
    /// of this build read, with how to check its text: what `check` names
    /// a field by that no module reads.
    pub fn part_kinds() -> Vec<crate::parts::PartKind> {
        let mut kinds = Vec::new();
        kinds.extend(crate::scene_core::part_kinds());
        kinds.extend(crate::body::part_kinds());
        kinds.extend(crate::look::part_kinds());
        kinds.extend(crate::motion::part_kinds());
        kinds.extend(crate::routes::part_kinds());
        kinds.extend(crate::sound::part_kinds());
        kinds.extend(crate::spline::part_kinds());
        kinds
    }
}

/// What reads a line's fields: every module's trait, to `use
/// runity::prelude::*` once.
pub mod prelude {
    pub use crate::body::{PhysicsLine, PhysicsOverride};
    pub use crate::material::MaterialLibrary;
    pub use crate::mesh_asset::{MeshLibrary, TextureLibrary};
    pub use crate::sound::SoundLibrary;
    pub use crate::look::{LookLine, LookOverride, SceneLook};
    pub use crate::motion::AnimationLine;
    pub use crate::routes::RouteLine;
    pub use crate::sound::SoundLine;
    pub use crate::spline::SplineLine;
}
pub mod screen;
pub use runity_core::shape;
#[cfg(feature = "desktop-shell")]
pub mod shell;
pub use runity_core::spelling;
pub mod ssao;
#[cfg(feature = "steam")]
pub mod steam;
pub use runity_core::strings;
pub mod surface;
pub mod taa;
pub mod terrain;
pub use runity_core::time;
pub use runity_core::timers;
pub mod tour;
pub use runity_core::tuned;
pub mod ui;
pub mod ui_render;
pub mod volume;
pub mod weather;
pub mod widgets;
/// The world as the core has it: hierarchy, identity, spawning.
pub use runity_core::world as world_core;
pub mod world_look;
pub mod bodies;
pub mod spawning;
mod world_tests;
#[cfg(test)]
mod core_tests;

/// A world of entities: the core's hierarchy and spawning, and every
/// module's components of it, under one name as before they were cut
/// apart.
pub mod world {
    pub use crate::bodies::*;
    pub use crate::motion::OnBone;
    pub use crate::sound::Sounding;
    pub use crate::spawning::*;
    pub use crate::world_core::*;
    pub use crate::world_look::*;
}

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
pub use prefab::{Instanced, Prefabs};

/// Expand a scene's prefab instances, with what this build's modules grow
/// on it: [`prefab::instantiate_with`] and the copies set along splines.
pub fn instantiate(scene: &Scene, prefabs: &Prefabs) -> Instanced {
    prefab::instantiate_with(scene, prefabs, spline::grow_all)
}
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
