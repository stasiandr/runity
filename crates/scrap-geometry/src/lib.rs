//! Geometry: what a model is made of, before anything draws it — vertices
//! and submeshes, textures, the built-in shapes, convex solids and brush
//! CSG, a line's model and terrain, and a rigged model's skeleton and clips as data. The render module uploads them, physics
//! makes colliders of them, animation plays the clips; none of that is
//! here (docs/modules.md).

pub mod animation;
pub mod builtin;
pub mod field;
pub mod line;
pub mod sdf;
pub mod mesh_asset;
pub mod solid;
pub mod terrain;

pub use animation::{Channel, Clip, Joint, PoseTransform, Posed, Skeleton};
pub use line::{GeometryLine, GeometryOverride, ModelRef};

// The core, under the names this module's code knows it by.
use scrap_core::{library, AssetLink};

/// The core's archive with this module's formats beside it.
mod asset {
    pub use crate::mesh_asset::*;
    pub use scrap_core::asset::*;
}

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = scrap_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "geometry");
        let problems = manifest.part_problems(&crate::line::part_kinds());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
