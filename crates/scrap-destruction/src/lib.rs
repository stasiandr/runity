//! Destruction (docs/simulation.md, item 5): things that break into
//! Voronoi pieces ([`fracture`]) — cut ahead of time or at the moment of
//! the blow, pieces that break again — and surfaces that dent where they
//! are struck ([`dents`]).
//!
//! It stands on the core, geometry and the physics' fields: the pieces
//! are dynamic bodies. It does not know who struck what or how hard; the
//! facade reads that off the physics and hands it the blows ([`Blow`]),
//! and draws the pieces and the dents.

pub mod dents;
pub mod fracture;
pub mod voronoi;

pub use dents::{run_dents, Dented, Dents, DentsDress, DentsLine};
pub use fracture::{run_fracture, Blow, Breakable, Broken, Fracture, FractureDress, FractureLine, Pattern, Piece};

/// This module's fields of a line, with how to check each one's text.
pub fn part_kinds() -> Vec<scrap_core::parts::PartKind> {
    let mut kinds = fracture::part_kinds();
    kinds.extend(dents::part_kinds());
    kinds
}

/// This module's manifest (`module.ron`).
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = scrap_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "destruction");
        let problems = manifest.part_problems(&super::part_kinds());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
