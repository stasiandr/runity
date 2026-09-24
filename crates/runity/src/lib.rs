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

pub use runity_input::actions;
pub use runity_geometry::animation;
pub use runity_render::appearance;
pub use runity_animation::animator;
pub use runity_animation::animgraph;
/// The asset archive as the core has it: its header, its ID.
pub use runity_core::asset as asset_core;
pub use runity_geometry::mesh_asset;
mod asset_tests;

/// Assets: the core's archive and IDs, and every module's formats, under
/// one name as before they were cut apart.
pub mod asset {
    pub use crate::asset_core::*;
    pub use crate::material::{ArchivedMaterialAsset, MaterialAsset};
    pub use crate::mesh_asset::*;
    pub use crate::sound::{ArchivedSoundAsset, SoundAsset, LONG_SOUND_SECONDS};
}
pub use runity_render::atmosphere;
#[cfg(feature = "audio")]
pub use runity_audio::audio;
#[cfg(feature = "physics")]
pub use runity_net::bench;
pub use runity_geometry::builtin;
pub use runity_render::clouds;
pub use runity_core::components;
pub use runity_core::crash;
pub use runity_render::decals;
pub use runity_dialogue::dialogue;
#[cfg(feature = "discord")]
pub use runity_discord::discord;
pub use runity_core::edit;
pub use runity_render::exposure;
pub use runity_render::floaters;
pub use runity_render::foliage;
pub use runity_render::footprints;
pub mod gizmo;
pub use runity_gpu::gpu;
pub use runity_animation::graph_text;
pub use runity_core::id;
pub use runity_core::input;
pub use runity_net::lan;
pub use runity_core::layers;
pub use runity_render::lens;
pub use runity_core::library;
pub use runity_core::links;
pub use runity_render::lights;
pub mod live;
pub use runity_render::material;
pub use runity_core::merge;
pub use runity_render::moods;
/// Clips that move a scene's things: the animation module's, with the
/// sound and particles a clip turns written by the modules that own them.
pub mod motion {
    pub use runity_animation::motion::*;

    /// Every moving line one step on ([`run_with`]), a clip's volume and
    /// particle-rate tracks written to the entity's sound and particles.
    pub fn run(world: &mut hecs::World, dt: f32) {
        run_with(world, dt, &mut |world, entity, what, value| match what {
            Property::Volume => {
                if let Ok(mut s) = world.get::<&mut crate::world::Sounding>(entity) {
                    s.0.volume = value;
                }
            }
            Property::ParticleRate => {
                if let Ok(mut p) = world.get::<&mut crate::particles::Emitting>(entity) {
                    p.emitter.rate = value;
                }
            }
            _ => {}
        })
    }
}
#[cfg(feature = "navigation")]
pub use runity_navigation::navigation;
pub use runity_net::net;
pub use runity_render::particles;
pub use runity_core::parts;
pub use runity_net::party;
pub use runity_core::perf;
#[cfg(feature = "physics")]
pub use runity_physics::physics;
pub use runity_core::player_prefs;
pub use runity_render::post;
pub use runity_core::prefab;
pub use runity_core::project;
pub mod query;
pub use runity_render::ray;
pub use runity_render::reflections;
pub mod refs;
pub use runity_net::relay;
pub use runity_render::render;
#[cfg(feature = "reports")]
pub use runity_reports::reports;
pub use runity_core::ron_edit;
pub use runity_core::ron_text;
pub use runity_routes::routes;
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
pub use runity_physics::body;
pub use runity_core::defaults;
pub use runity_render::look;
pub use runity_audio::sound;
pub use runity_spline::spline;
mod scene_tests;

/// The scene file: the core's lines and scenes, and every module's types of
/// the fields on them, under one name as before they were cut apart.
pub mod scene {
    pub use crate::body::*;
    pub use crate::look::*;
    pub use runity_geometry::line::*;
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
        kinds.extend(runity_geometry::line::part_kinds());
        kinds.extend(runity_core::wind::part_kinds());
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
    pub use runity_geometry::line::{GeometryLine, GeometryOverride};
    pub use crate::material::MaterialLibrary;
    pub use crate::mesh_asset::{MeshLibrary, TextureLibrary};
    pub use crate::sound::SoundLibrary;
    pub use crate::look::{LookLine, LookOverride, SceneLook};
    pub use crate::motion::AnimationLine;
    pub use crate::routes::RouteLine;
    pub use crate::sound::SoundLine;
    pub use crate::spline::SplineLine;
}
pub use runity_overlay::screen;
pub use runity_core::shape;
#[cfg(feature = "desktop-shell")]
pub mod shell;
pub use runity_core::spelling;
pub use runity_render::ssao;
#[cfg(feature = "steam")]
pub use runity_steam::steam;
pub use runity_core::strings;
pub use runity_gpu::surface;
pub use runity_render::taa;
pub use runity_render::terrain;
pub use runity_core::time;
pub use runity_core::timers;
pub use runity_render::tour;
pub use runity_core::tuned;
pub use runity_overlay::ui;
pub use runity_overlay::ui_render;
pub use runity_render::volume;
pub use runity_render::weather;
pub use runity_overlay::widgets;
/// The world as the core has it: hierarchy, identity, spawning.
pub use runity_core::world as world_core;
pub use runity_render::world_look;
pub use runity_physics::bodies;
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
