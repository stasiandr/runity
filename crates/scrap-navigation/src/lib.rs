//! Navigation: Unity's NavMesh as a grid baked from the physics module's
//! static world, and paths over it ([`navigation::NavGrid`]).

pub mod navigation;

/// The scene's lines, with the physics module's fields beside the core's.
#[allow(unused_imports)]
mod scene {
    pub use scrap_core::scene::*;
    pub use scrap_physics::body::*;
}

#[allow(unused_imports)]
use scrap_core::world;
#[allow(unused_imports)]
use scrap_physics::physics;

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = scrap_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "navigation");
        let problems = manifest.part_problems(&Vec::new());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
