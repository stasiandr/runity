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

#[cfg(feature = "input")]
pub use runity_input::actions;
pub use runity_geometry::animation;
pub use runity_render::appearance;
#[cfg(feature = "animation")]
pub use runity_animation::animator;
#[cfg(feature = "animation")]
pub use runity_animation::animgraph;
/// The asset archive as the core has it: its header, its ID.
pub use runity_core::asset as asset_core;
pub use runity_geometry::mesh_asset;
mod asset_tests;

/// Assets: the core's archive and IDs, and every module's formats, under
/// one name as before they were cut apart.
pub mod asset {
    pub use crate::asset_core::*;
    pub use crate::material::{ArchivedMaterialAsset, MaterialAsset, MATERIAL};
    pub use crate::mesh_asset::*;
    pub use crate::sound::{ArchivedSoundAsset, SoundAsset, LONG_SOUND_SECONDS, SOUND};

    /// Every kind of asset this build's modules name: what a library can
    /// hold. Each module picks its byte; two picking one is caught here.
    pub fn kinds() -> Vec<AssetKind> {
        vec![MESH, TEXTURE, SOUND, MATERIAL]
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn no_two_modules_name_one_kind() {
            let kinds = super::kinds();
            for (i, a) in kinds.iter().enumerate() {
                for b in &kinds[i + 1..] {
                    assert_ne!(a, b, "{} and {} share byte {}", a.name, b.name, a.byte);
                }
            }
        }
    }
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
#[cfg(feature = "dialogue")]
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
#[cfg(feature = "animation")]
pub use runity_animation::graph_text;
pub use runity_core::id;
pub use runity_core::input;
#[cfg(feature = "net")]
pub use runity_net::lan;
pub use runity_core::layers;
pub use runity_render::lens;
pub use runity_core::library;
pub use runity_core::links;
pub use runity_render::lod;
pub use runity_render::lights;
pub mod live;
pub use runity_render::material;
pub use runity_core::merge;
pub use runity_render::moods;
/// Clips that move a scene's things: the animation module's, with the
/// sound and particles a clip turns written by the modules that own them.
#[cfg(feature = "animation")]
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
#[cfg(feature = "net")]
pub use runity_net::net;
pub use runity_render::occlusion;
pub use runity_render::particles;
pub use runity_render::particles_gpu;
pub use runity_render::passes;
pub use runity_core::parts;
#[cfg(feature = "net")]
pub use runity_net::party;
pub use runity_core::perf;
#[cfg(feature = "physics")]
pub use runity_physics::physics;
pub use runity_core::player_prefs;
pub use runity_render::post;
pub use runity_core::prefab;
pub use runity_core::project;
pub mod query;

/// The loop by phases (Unity's PlayerLoop; `runity_core::player_loop`),
/// with every module of this build's systems in it.
pub mod player_loop {
    pub use runity_core::player_loop::*;

    /// The build's modules' systems, in the order a frame needs them: in
    /// the fixed step routes, motion clips and characters' animation, then
    /// the hierarchy placed; cameras following in LateUpdate; particles
    /// and footprints as the frame is built. A game runs a phase where its
    /// own systems want it.
    pub fn modules() -> PlayerLoop {
        let mut player_loop = PlayerLoop::new();
        #[cfg(feature = "routes")]
        runity_routes::systems(&mut player_loop);
        #[cfg(feature = "animation")]
        player_loop.add(Phase::FixedUpdate, "motion", crate::motion::run);
        #[cfg(feature = "animation")]
        runity_animation::systems(&mut player_loop);
        runity_core::player_loop::systems(&mut player_loop);
        runity_render::systems(&mut player_loop);
        player_loop
    }

    #[cfg(test)]
    mod tests {
        #[test]
        #[cfg(all(feature = "routes", feature = "animation"))]
        fn the_modules_put_their_systems_where_unity_would() {
            let player_loop = super::modules();
            use super::Phase;
            assert_eq!(
                player_loop.names(Phase::FixedUpdate),
                ["routes", "motion", "animation", "hierarchy"]
            );
            assert_eq!(player_loop.names(Phase::LateUpdate), ["cameras"]);
        }
    }
}

/// The engine's modules by their manifests (`module.ron`, docs/modules.md):
/// what a project's `runity.ron` lists, and what `runity check` holds the
/// list against.
pub mod modules {
    pub use runity_core::module::{features, list_problems, with_depends, Manifest};

    /// The modules' features `runity` builds with when a game does not say
    /// `default-features = false`: `default` in its Cargo.toml, less what
    /// is not a module's (a render pass's, as `ray-tracing`).
    pub const DEFAULT_FEATURES: &[&str] = &[
        "animation", "dialogue", "input", "navigation", "net", "physics", "routes", "spline",
    ];

    /// The sets `runity new` offers (DNA, postulate 8), by name: `bare`,
    /// the core alone; `basic`, what a game as hard as Flappy Bird needs —
    /// a picture, input, a score on screen, sound, collisions — played
    /// together, since a game alone is a session of one (postulate 4), in
    /// a window; `full`, every official module. What they stand on comes
    /// with them.
    pub fn set(name: &str) -> Option<Vec<String>> {
        let names: Vec<&str> = match name {
            "bare" => Vec::new(),
            "basic" => vec!["render", "input", "overlay", "audio", "physics", "net", "shell"],
            "full" => return Some(official().into_iter().map(|m| m.name).collect()),
            _ => return None,
        };
        Some(names.into_iter().map(String::from).collect())
    }

    /// The names of the sets, for a message.
    pub const SETS: &[&str] = &["bare", "basic", "full"];

    /// Every official module's manifest, whether this build has it or not.
    pub fn official() -> Vec<Manifest> {
        [
        include_str!("../../runity-geometry/module.ron"),
        include_str!("../../runity-physics/module.ron"),
        include_str!("../../runity-navigation/module.ron"),
        include_str!("../../runity-animation/module.ron"),
        include_str!("../../runity-audio/module.ron"),
        include_str!("../../runity-gpu/module.ron"),
        include_str!("../../runity-render/module.ron"),
        include_str!("../../runity-overlay/module.ron"),
        include_str!("../../runity-ui/module.ron"),
        include_str!("../../runity-net/module.ron"),
        include_str!("../../runity-steam/module.ron"),
        include_str!("../../runity-input/module.ron"),
        include_str!("../../runity-spline/module.ron"),
        include_str!("../../runity-routes/module.ron"),
        include_str!("../../runity-dialogue/module.ron"),
        include_str!("../../runity-reports/module.ron"),
        include_str!("../../runity-discord/module.ron"),
        include_str!("../../runity-shell/module.ron"),
        ]
        .iter()
        .map(|text| Manifest::parse(text).expect("an official module's manifest reads"))
        .collect()
    }

    /// The modules this build of the engine has: every one without a
    /// feature, and those whose feature is on.
    pub fn built() -> Vec<Manifest> {
        official()
            .into_iter()
            .filter(|m| m.feature.as_deref().is_none_or(feature_on))
            .collect()
    }

    fn feature_on(feature: &str) -> bool {
        match feature {
            "physics" => cfg!(feature = "physics"),
            "navigation" => cfg!(feature = "navigation"),
            "audio" => cfg!(feature = "audio"),
            "steam" => cfg!(feature = "steam"),
            "reports" => cfg!(feature = "reports"),
            "discord" => cfg!(feature = "discord"),
            "desktop-shell" => cfg!(feature = "desktop-shell"),
            "animation" => cfg!(feature = "animation"),
            "net" => cfg!(feature = "net"),
            "input" => cfg!(feature = "input"),
            "spline" => cfg!(feature = "spline"),
            "routes" => cfg!(feature = "routes"),
            "dialogue" => cfg!(feature = "dialogue"),
            _ => false,
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn the_official_modules_stand_on_each_other_and_on_nothing_else() {
            let all = super::official();
            let names: Vec<String> = all.iter().map(|m| m.name.clone()).collect();
            assert!(super::list_problems(&names, &all, env!("CARGO_PKG_VERSION")).is_empty());
            // Physics, navigation and the rest of the default set are built.
            let built: Vec<String> = super::built().into_iter().map(|m| m.name).collect();
            assert!(built.iter().any(|n| n == "render"));
            assert_eq!(built.iter().any(|n| n == "physics"), cfg!(feature = "physics"));
            for set in super::SETS {
                let listed = super::set(set).unwrap();
                assert!(super::list_problems(&listed, &all, env!("CARGO_PKG_VERSION")).is_empty());
            }
        }
    }
}
pub use runity_render::ray;
pub use runity_render::reflections;
pub mod refs;
#[cfg(feature = "net")]
pub use runity_net::relay;
pub use runity_render::render;
#[cfg(feature = "reports")]
pub use runity_reports::reports;
pub use runity_core::ron_edit;
pub use runity_core::ron_text;
#[cfg(feature = "routes")]
pub use runity_routes::routes;
/// Saving a game in progress, as the core has it.
pub use runity_core::save as save_core;

/// Saving a game in progress, and the report a running game sends the
/// editor: the core's save with what the modules add to it.
pub mod save {
    #[cfg(feature = "animation")]
    pub use crate::animgraph::animator_trails;
    #[cfg(feature = "net")]
    pub use crate::net::net_lines;

    /// Each animated entity's recent transitions: none in a build without
    /// animation.
    #[cfg(not(feature = "animation"))]
    pub fn animator_trails(_: &hecs::World) -> Vec<(crate::id::EntityId, Vec<String>)> {
        Vec::new()
    }

    /// Who owns what on the network: nothing in a build without it.
    #[cfg(not(feature = "net"))]
    pub fn net_lines(_: &hecs::World) -> Vec<NetLine> {
        Vec::new()
    }

    /// An entity's animator state for a save: its graph's, or nothing in a
    /// build without animation.
    fn animator_state(world: &hecs::World, entity: hecs::Entity) -> String {
        #[cfg(feature = "animation")]
        return crate::animgraph::animator_state(world, entity);
        #[cfg(not(feature = "animation"))]
        {
            let _ = (world, entity);
            String::new()
        }
    }
    pub use crate::save_core::*;

    /// Write down a world that started from `scene`, with each animated
    /// entity's graph state.
    pub fn capture(
        world: &hecs::World,
        components: &crate::components::Components,
        scene: &crate::scene::Scene,
    ) -> SaveGame {
        capture_with(world, components, scene, &animator_state)
    }
}
/// The scene file as the core has it: a line's identity, place and tree,
/// its modules' fields as parts (docs/modules.md).
pub use runity_core::scene as scene_core;
pub use runity_physics::body;
pub use runity_core::defaults;
pub use runity_render::look;
pub use runity_audio::sound;
#[cfg(feature = "spline")]
pub use runity_spline::spline;
mod scene_tests;

/// The scene file: the core's lines and scenes, and every module's types of
/// the fields on them, under one name as before they were cut apart.
pub mod scene {
    pub use crate::body::*;
    pub use crate::look::*;
    pub use runity_geometry::line::*;
    #[cfg(feature = "animation")]
    pub use crate::motion::{AnimatorRef, BoneName};
    #[cfg(feature = "routes")]
    pub use crate::routes::{Route, RouteEnds};
    pub use crate::scene_core::*;
    pub use crate::sound::*;
    #[cfg(feature = "spline")]
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
        #[cfg(feature = "animation")]
        kinds.extend(crate::motion::part_kinds());
        #[cfg(feature = "routes")]
        kinds.extend(crate::routes::part_kinds());
        kinds.extend(crate::sound::part_kinds());
        #[cfg(feature = "spline")]
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
    #[cfg(feature = "animation")]
    pub use crate::motion::AnimationLine;
    #[cfg(feature = "routes")]
    pub use crate::routes::RouteLine;
    pub use crate::sound::SoundLine;
    #[cfg(feature = "spline")]
    pub use crate::spline::SplineLine;
}
pub use runity_overlay::screen;
pub use runity_core::shape;
#[cfg(feature = "desktop-shell")]
pub use runity_shell::shell;
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
pub use runity_render::upscale;
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
    #[cfg(feature = "animation")]
    pub use crate::motion::OnBone;
    pub use crate::sound::Sounding;
    pub use crate::spawning::*;
    pub use crate::world_core::*;
    pub use crate::world_look::*;
}

pub use animation::{Channel, Clip, Joint, PoseTransform, Skeleton};
#[cfg(feature = "animation")]
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
#[cfg(feature = "input")]
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
    prefab::instantiate_with(scene, prefabs, grow)
}

/// What grows on a scene as it expands: copies along splines, in a build
/// that has them.
fn grow(lines: &mut [EntityDesc]) {
    #[cfg(feature = "spline")]
    spline::grow_all(lines);
    #[cfg(not(feature = "spline"))]
    let _ = lines;
}
pub use project::{Project, ProjectError};
pub use render::{
    Camera, Draw, FogSettings, Frame, Lighting, MeshHandle, Renderer, ShadowSettings, TextureHandle,
};
pub use scene::{Body, EntityDesc, Fog, Scene, Sun, Transform, View};
#[cfg(feature = "spline")]
pub use scene::{Along, Spline};
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
