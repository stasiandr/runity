//! Soft things (docs/simulation.md): particles held by constraints and
//! solved by XPBD in small substeps — one solver for everything that bends,
//! stretches, twists and hangs. So far: ropes, cables and chains as
//! Cosserat rods ([`rope`]), lying on simple [`obstacle`]s.
//!
//! Headless: it hands out points, a tube's triangles and where a chain's
//! links are; drawing them is the render's, and the facade hands them over.
//! Colliders reach it the same way, as [`obstacle::Obstacle`]s: the module
//! does not know the physics.

pub mod obstacle;
pub mod particles;
pub mod rod;
pub mod rope;

pub use obstacle::{Obstacle, Obstacles};
pub use rope::{run_ropes, set_wind, Ends, Rope, RopeDress, RopeKind, RopeLine, RopeState};

/// This module's fields of a line, with how to check each one's text.
pub fn part_kinds() -> Vec<runity_core::parts::PartKind> {
    rope::part_kinds()
}

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = runity_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "soft");
        let problems = manifest.part_problems(&super::part_kinds());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
