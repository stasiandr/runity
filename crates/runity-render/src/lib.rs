//! The 3D render (DNA, "Видеокарта — модуль `gpu`": `render` stands on
//! it): a line's look (`model`'s material, `camera`, `light`, `particles`,
//! …) dressed onto its entity ([`appearance::LookDress`]), a frame built
//! from the world ([`world_look::build_frame`]) and drawn — materials and
//! lights, shadows, sky and fog, post-processing, reflections, particles,
//! terrain, foliage — with the gizmos an editor draws over it.
//!
//! It stands on the core, geometry (the meshes it uploads) and the GPU,
//! and draws the overlay's UI where the world shows it. The render path is
//! one (DNA, postulate 7); its passes are switched, not swapped.

pub mod appearance;
pub mod atmosphere;
pub mod bindless;
pub mod clouds;
pub mod cluster;
pub mod ddgi;
pub mod decals;
pub mod distance;
pub mod exposure;
pub mod floaters;
pub mod foliage;
pub mod footprints;
pub mod gizmo;
pub mod graph;
pub mod lens;
pub mod lights;
pub mod lod;
pub mod look;
pub mod material;
pub mod moods;
pub mod occlusion;
pub mod particles;
pub mod particles_gpu;
pub(crate) mod smoke_gpu;
pub mod passes;
pub mod post;
pub mod quality;
pub mod ray;
pub mod reflections;
pub mod render;
pub mod restir;
pub mod ssao;
pub mod streaming_textures;
pub mod taa;
pub mod terrain;
pub mod tour;
pub mod upscale;
pub mod volume;
pub mod vsm;
pub mod weather;
pub mod world_look;

pub use material::{Material, Shading};
pub use render::{
    Camera, Draw, FogSettings, Frame, Lighting, MeshHandle, Renderer, ShadowSettings, TextureHandle,
};
pub use passes::Passes;
pub use world_look::build_frame;

/// This module's systems in the loop: cameras that follow keep after their
/// targets once the frame's moves are done, and sparks and footprints move
/// on the frame's time — they are for the eye.
pub fn systems(player_loop: &mut runity_core::player_loop::PlayerLoop) {
    use runity_core::player_loop::Phase;
    player_loop
        .add(Phase::LateUpdate, "cameras", world_look::follow_cameras)
        .add(Phase::PostLateUpdate, "particles", particles::run_particles)
        .add(Phase::PostLateUpdate, "footprints", footprints::run_footprints);
}

// The core, geometry, the GPU and the overlay, under the names this
// module's code knows them by.
#[allow(unused_imports)]
use runity_core::{defaults, id, impl_parts, input, library, AssetLink, Library, Tuned};
#[allow(unused_imports)]
use runity_geometry::{animation, builtin};
#[allow(unused_imports)]
use runity_gpu::{gpu, gpu_timer, surface};
#[allow(unused_imports)]
use runity_overlay::{ui, ui_render};

/// The scene's lines, with this module's fields and geometry's beside the
/// core's.
#[allow(unused_imports)]
mod scene {
    pub use crate::look::*;
    pub use runity_core::scene::*;
    pub use runity_geometry::line::*;
}

/// The world, with this module's components beside the core's.
#[allow(unused_imports)]
mod world {
    pub use crate::world_look::*;
    pub use runity_core::world::*;
}

/// The core's archive with geometry's formats and this module's.
#[allow(unused_imports)]
mod asset {
    pub use crate::material::{ArchivedMaterialAsset, MaterialAsset, MATERIAL};
    pub use runity_core::asset::*;
    pub use runity_geometry::mesh_asset::*;
}

/// The traits that read a line's fields.
#[allow(unused_imports)]
mod prelude {
    pub use crate::look::{LookLine, LookOverride, SceneLook};
    pub use crate::material::MaterialLibrary;
    pub use runity_geometry::line::{GeometryLine, GeometryOverride};
    pub use runity_geometry::mesh_asset::{MeshLibrary, TextureLibrary};
}

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = runity_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "render");
        let problems = manifest.part_problems(&crate::look::part_kinds());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
