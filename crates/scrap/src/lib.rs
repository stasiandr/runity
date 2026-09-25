//! scrap: a guest, never a host.
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
pub use scrap_core::impl_parts;

#[cfg(feature = "animation")]
pub use scrap_animation::animator;
#[cfg(feature = "animation")]
pub use scrap_animation::animgraph;
/// IK on an animated skeleton: feet on the ground, a look (a line's `ik`).
#[cfg(feature = "animation")]
pub use scrap_animation::ik as pose_ik;
/// The asset archive as the core has it: its header, its ID.
pub use scrap_core::asset as asset_core;
pub use scrap_geometry::animation;
pub use scrap_geometry::mesh_asset;
#[cfg(feature = "input")]
pub use scrap_input::actions;
pub use scrap_render::appearance;
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
#[cfg(feature = "audio")]
pub use scrap_audio::audio;
pub use scrap_core::components;
pub mod console;
pub use scrap_core::crash;
pub use scrap_core::edit;
pub use scrap_core::embed;
pub use scrap_core::files;
pub use scrap_core::web_time;
#[cfg(feature = "dialogue")]
pub use scrap_dialogue::dialogue;
#[cfg(feature = "dialogue")]
pub use scrap_dialogue::quest;
/// A dialogue as the files are written by hand, patched where it changed.
#[cfg(feature = "dialogue")]
pub use scrap_dialogue::text as dialogue_text;
#[cfg(feature = "discord")]
pub use scrap_discord::discord;
pub use scrap_geometry::builtin;
pub use scrap_geometry::ease;
pub use scrap_geometry::solid;
#[cfg(feature = "physics")]
pub use scrap_net::bench;
pub use scrap_render::atmosphere;
pub use scrap_render::cameras;
pub use scrap_render::clouds;
pub use scrap_render::cluster;
pub use scrap_render::decals;
pub use scrap_render::distance;
pub use scrap_render::exposure;
pub use scrap_render::floaters;
pub use scrap_render::foliage;
pub use scrap_render::footprints;
#[cfg(feature = "physics")]
pub mod debug_overlay;
pub mod gizmo;
#[cfg(feature = "animation")]
pub use scrap_animation::graph_text;
pub use scrap_core::id;
pub use scrap_core::input;
pub use scrap_core::layers;
pub use scrap_core::library;
pub use scrap_core::links;
pub use scrap_gpu::gpu;
#[cfg(feature = "net")]
pub use scrap_net::lan;
pub use scrap_render::ddgi;
pub use scrap_render::graph;
pub use scrap_render::lens;
pub use scrap_render::lights;
pub use scrap_render::lod;
pub use scrap_render::occlusion;
pub use scrap_render::particles_gpu;
pub use scrap_render::upscale;
pub use scrap_render::vsm;
pub mod live;
pub use scrap_core::merge;
pub use scrap_render::material;
pub use scrap_render::moods;
/// Clips that move a scene's things: the animation module's, with the
/// sound and particles a clip turns written by the modules that own them.
#[cfg(feature = "animation")]
pub mod motion {
    pub use scrap_animation::motion::*;

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

/// Tweens from game code: the animation module's, with a sound's volume
/// and particles' rate read and written by the modules that own them, as
/// a motion clip's are (docs/feel.md).
#[cfg(feature = "animation")]
pub mod tween {
    pub use scrap_animation::tween::*;

    use scrap_animation::motion::Property;

    /// Every tween one step on ([`run_tweens`]).
    pub fn run(world: &mut hecs::World, dt: f32) {
        let get = |world: &hecs::World, entity: hecs::Entity, what: Property| match what {
            Property::Volume => world
                .get::<&crate::world::Sounding>(entity)
                .ok()
                .map(|s| s.0.volume),
            Property::ParticleRate => world
                .get::<&crate::particles::Emitting>(entity)
                .ok()
                .map(|p| p.emitter.rate),
            _ => None,
        };
        run_tweens(
            world,
            dt,
            &get,
            &mut |world, entity, what, value| match what {
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
            },
        )
    }
}
pub use scrap_core::parts;
pub use scrap_core::perf;
pub use scrap_core::player;
pub use scrap_core::player_prefs;
pub use scrap_core::prefab;
pub use scrap_core::project;
#[cfg(feature = "navigation")]
pub use scrap_navigation::navigation;
#[cfg(feature = "net")]
pub use scrap_net::net;
#[cfg(feature = "net")]
pub use scrap_net::party;
#[cfg(feature = "physics")]
pub use scrap_physics::physics;
pub use scrap_render::particles;
pub use scrap_render::passes;
pub use scrap_render::post;
pub mod query;

/// The loop by phases (Unity's PlayerLoop; `scrap_core::player_loop`),
/// with every module of this build's systems in it.
pub mod player_loop {
    pub use scrap_core::player_loop::*;

    /// The build's modules' systems, in the order a frame needs them: in
    /// the fixed step the wires first, so what they pull moves in the same
    /// step, then routes, motion clips, tweens and characters' animation, then
    /// the hierarchy placed; cameras following in LateUpdate; particles
    /// and footprints as the frame is built. A game runs a phase where its
    /// own systems want it.
    pub fn modules() -> PlayerLoop {
        let mut player_loop = PlayerLoop::new();
        #[cfg(feature = "wires")]
        scrap_wires::systems(&mut player_loop);
        #[cfg(feature = "routes")]
        scrap_routes::systems(&mut player_loop);
        #[cfg(feature = "animation")]
        player_loop
            .add(Phase::FixedUpdate, "motion", crate::motion::run)
            .add(Phase::FixedUpdate, "tweens", crate::tween::run);
        // Feet of animated skeletons stand on the scene's colliders, as a
        // crawler's do.
        #[cfg(all(feature = "animation", feature = "soft"))]
        scrap_animation::systems_on(&mut player_loop, crate::soft::feet_ground);
        #[cfg(all(feature = "animation", not(feature = "soft")))]
        scrap_animation::systems(&mut player_loop);
        scrap_core::player_loop::systems(&mut player_loop);
        // Ropes swing once everything they hang from is placed, and are
        // drawn as the frame is built.
        #[cfg(feature = "soft")]
        player_loop
            .add(Phase::FixedUpdate, "soft", crate::soft::step)
            .add(Phase::PostLateUpdate, "soft_look", crate::soft::show);
        #[cfg(feature = "destruction")]
        player_loop.add(
            Phase::PostLateUpdate,
            "dents_look",
            crate::destruction::show,
        );
        #[cfg(feature = "fluid")]
        player_loop
            .add(Phase::FixedUpdate, "fluid", crate::fluid::step)
            .add(Phase::PostLateUpdate, "fluid_look", crate::fluid::show);
        #[cfg(feature = "character")]
        player_loop
            .add(Phase::FixedUpdate, "crawl", crate::character::crawl)
            .add(
                Phase::PostLateUpdate,
                "character_look",
                crate::character::show,
            );
        scrap_render::systems(&mut player_loop);
        player_loop
    }

    #[cfg(test)]
    mod tests {
        #[test]
        #[cfg(all(feature = "routes", feature = "animation"))]
        fn the_modules_put_their_systems_where_unity_would() {
            let player_loop = super::modules();
            use super::Phase;
            let mut fixed = vec!["routes", "motion", "tweens", "animation", "hierarchy"];
            if cfg!(feature = "wires") {
                fixed.insert(0, "wires");
            }
            if cfg!(feature = "soft") {
                fixed.push("soft");
            }
            if cfg!(feature = "fluid") {
                fixed.push("fluid");
            }
            if cfg!(feature = "character") {
                fixed.push("crawl");
            }
            assert_eq!(player_loop.names(Phase::FixedUpdate), fixed);
            assert_eq!(player_loop.names(Phase::LateUpdate), ["cameras"]);
        }
    }
}

/// The engine's modules by their manifests (`module.ron`, docs/modules.md):
/// what a project's `scrap.ron` lists, and what `scrap check` holds the
/// list against.
pub mod modules {
    pub use scrap_core::module::{features, list_problems, with_depends, Manifest};

    /// The modules' features `scrap` builds with when a game does not say
    /// `default-features = false`: `default` in its Cargo.toml, less what
    /// is not a module's (a render pass's, as `ray-tracing`).
    pub const DEFAULT_FEATURES: &[&str] = &[
        "animation",
        "character",
        "destruction",
        "dialogue",
        "fluid",
        "input",
        "navigation",
        "net",
        "physics",
        "routes",
        "soft",
        "spline",
        "wires",
    ];

    /// The sets `scrap new` offers (DNA, postulate 8), by name: `bare`,
    /// the core alone; `basic`, what a game as hard as Flappy Bird needs —
    /// a picture, input, a score on screen, sound, collisions — played
    /// together, since a game alone is a session of one (postulate 4), in
    /// a window; `full`, every official module. What they stand on comes
    /// with them.
    pub fn set(name: &str) -> Option<Vec<String>> {
        let names: Vec<&str> = match name {
            "bare" => Vec::new(),
            "basic" => vec![
                "render", "input", "overlay", "audio", "physics", "net", "shell",
            ],
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
            include_str!("../../scrap-geometry/module.ron"),
            include_str!("../../scrap-physics/module.ron"),
            include_str!("../../scrap-navigation/module.ron"),
            include_str!("../../scrap-animation/module.ron"),
            include_str!("../../scrap-audio/module.ron"),
            include_str!("../../scrap-gpu/module.ron"),
            include_str!("../../scrap-render/module.ron"),
            include_str!("../../scrap-overlay/module.ron"),
            include_str!("../../scrap-ui/module.ron"),
            include_str!("../../scrap-net/module.ron"),
            include_str!("../../scrap-steam/module.ron"),
            include_str!("../../scrap-input/module.ron"),
            include_str!("../../scrap-spline/module.ron"),
            include_str!("../../scrap-routes/module.ron"),
            include_str!("../../scrap-wires/module.ron"),
            include_str!("../../scrap-dialogue/module.ron"),
            include_str!("../../scrap-reports/module.ron"),
            include_str!("../../scrap-discord/module.ron"),
            include_str!("../../scrap-shell/module.ron"),
            include_str!("../../scrap-soft/module.ron"),
            include_str!("../../scrap-destruction/module.ron"),
            include_str!("../../scrap-fluid/module.ron"),
            include_str!("../../scrap-character/module.ron"),
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
            "wires" => cfg!(feature = "wires"),
            "dialogue" => cfg!(feature = "dialogue"),
            "soft" => cfg!(feature = "soft"),
            "destruction" => cfg!(feature = "destruction"),
            "fluid" => cfg!(feature = "fluid"),
            "character" => cfg!(feature = "character"),
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
            assert_eq!(
                built.iter().any(|n| n == "physics"),
                cfg!(feature = "physics")
            );
            for set in super::SETS {
                let listed = super::set(set).unwrap();
                assert!(super::list_problems(&listed, &all, env!("CARGO_PKG_VERSION")).is_empty());
            }
        }
    }
}
pub use scrap_render::quality;
pub use scrap_render::ray;
pub use scrap_render::reflections;
pub mod refs;
pub mod shot;
pub mod streaming;
pub use scrap_core::records;
pub use scrap_core::ron_edit;
pub use scrap_core::ron_text;
/// Saving a game in progress, as the core has it.
pub use scrap_core::save as save_core;
#[cfg(feature = "net")]
pub use scrap_net::relay;
pub use scrap_render::render;
#[cfg(feature = "reports")]
pub use scrap_reports::reports;
#[cfg(feature = "routes")]
pub use scrap_routes::routes;
#[cfg(feature = "wires")]
pub use scrap_wires::wires;

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
pub use scrap_audio::sound;
pub use scrap_core::defaults;
/// The scene file as the core has it: a line's identity, place and tree,
/// its modules' fields as parts (docs/modules.md).
pub use scrap_core::scene as scene_core;
pub use scrap_physics::body;
pub use scrap_render::look;
#[cfg(feature = "spline")]
pub use scrap_spline::spline;
mod scene_tests;

/// The scene file: the core's lines and scenes, and every module's types of
/// the fields on them, under one name as before they were cut apart.
pub mod scene {
    pub use crate::body::*;
    #[cfg(feature = "character")]
    pub use crate::character::{Crawler, Ragdoll};
    #[cfg(feature = "destruction")]
    pub use crate::destruction::{Dents, Fracture};
    #[cfg(feature = "fluid")]
    pub use crate::fluid::{Floats, Mpm, Ocean, Ripples, ShallowWater, Smoke, SnowCover};
    pub use crate::look::*;
    #[cfg(feature = "animation")]
    pub use crate::motion::{AnimatorRef, BoneName};
    #[cfg(feature = "routes")]
    pub use crate::routes::{Route, RouteEnds};
    pub use crate::scene_core::*;
    #[cfg(feature = "soft")]
    pub use crate::soft::{
        Cloth, DistanceField, Fluid, Grains, Hair, Jiggle, Rope, RopeKind, SoftBody,
    };
    pub use crate::sound::*;
    #[cfg(feature = "spline")]
    pub use crate::spline::*;
    #[cfg(feature = "wires")]
    pub use crate::wires::{Act, On, Wire, Wires};
    pub use scrap_geometry::line::*;

    /// Every field of a line, an override or a scene's look the modules
    /// of this build read, with how to check its text: what `check` names
    /// a field by that no module reads.
    pub fn part_kinds() -> Vec<crate::parts::PartKind> {
        let mut kinds = Vec::new();
        kinds.extend(crate::scene_core::part_kinds());
        kinds.extend(crate::body::part_kinds());
        kinds.extend(scrap_geometry::line::part_kinds());
        kinds.extend(scrap_core::wind::part_kinds());
        kinds.extend(scrap_core::stream::part_kinds());
        kinds.extend(scrap_core::player::part_kinds());
        kinds.extend(crate::look::part_kinds());
        #[cfg(feature = "animation")]
        kinds.extend(crate::motion::part_kinds());
        #[cfg(feature = "routes")]
        kinds.extend(crate::routes::part_kinds());
        #[cfg(feature = "wires")]
        kinds.extend(crate::wires::part_kinds());
        kinds.extend(crate::sound::part_kinds());
        #[cfg(feature = "spline")]
        kinds.extend(crate::spline::part_kinds());
        #[cfg(feature = "soft")]
        kinds.extend(crate::soft::part_kinds());
        #[cfg(feature = "destruction")]
        kinds.extend(crate::destruction::part_kinds());
        #[cfg(feature = "fluid")]
        kinds.extend(crate::fluid::part_kinds());
        #[cfg(feature = "character")]
        kinds.extend(crate::character::part_kinds());
        kinds
    }
}

/// What reads a line's fields: every module's trait, to `use
/// scrap::prelude::*` once.
pub mod prelude {
    pub use crate::body::{PhysicsLine, PhysicsOverride};
    #[cfg(feature = "character")]
    pub use crate::character::{CrawlerLine, RagdollLine};
    #[cfg(feature = "destruction")]
    pub use crate::destruction::{DentsLine, FractureLine};
    #[cfg(feature = "fluid")]
    pub use crate::fluid::{FloatsLine, HeightfieldLine, MpmLine, OceanLine, SmokeLine};
    pub use crate::look::{LookLine, LookOverride, SceneLook};
    pub use crate::material::MaterialLibrary;
    pub use crate::mesh_asset::{MeshLibrary, TextureLibrary};
    #[cfg(feature = "animation")]
    pub use crate::motion::AnimationLine;
    #[cfg(feature = "routes")]
    pub use crate::routes::RouteLine;
    #[cfg(feature = "soft")]
    pub use crate::soft::{
        ClothLine, DistanceFieldLine, FluidLine, GrainsLine, HairLine, JiggleLine, RopeLine,
        SoftBodyLine,
    };
    pub use crate::sound::SoundLibrary;
    pub use crate::sound::SoundLine;
    #[cfg(feature = "spline")]
    pub use crate::spline::SplineLine;
    #[cfg(feature = "wires")]
    pub use crate::wires::WireLine;
    pub use scrap_geometry::line::{GeometryLine, GeometryOverride};
}
pub use scrap_core::shape;
pub mod netsim;
pub use scrap_overlay::screen;
#[cfg(feature = "character")]
pub mod character;
#[cfg(feature = "destruction")]
pub mod destruction;
#[cfg(feature = "fluid")]
pub mod fluid;
#[cfg(feature = "soft")]
pub mod soft;
pub use scrap_core::spelling;
pub use scrap_core::strings;
pub use scrap_core::time;
pub use scrap_core::timers;
pub use scrap_core::tuned;
/// The world as the core has it: hierarchy, identity, spawning.
pub use scrap_core::world as world_core;
pub use scrap_gpu::surface;
pub use scrap_overlay::ui;
pub use scrap_overlay::ui_render;
pub use scrap_overlay::widgets;
pub use scrap_physics::bodies;
pub use scrap_render::ssao;
pub use scrap_render::taa;
pub use scrap_render::terrain;
pub use scrap_render::tour;
pub use scrap_render::volume;
pub use scrap_render::weather;
pub use scrap_render::world_look;
#[cfg(feature = "desktop-shell")]
pub use scrap_shell::shell;
#[cfg(feature = "steam")]
pub use scrap_steam::steam;
#[cfg(test)]
mod core_tests;
pub mod spawning;
mod world_tests;

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
#[cfg(feature = "spline")]
pub use scene::{Along, Spline};
pub use scene::{Body, EntityDesc, Fog, Scene, Sun, Transform, View};
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
