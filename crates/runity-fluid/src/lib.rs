//! Fluids on grids (docs/simulation.md, items 6–8, 10): the material point
//! method for snow, sand, water and jelly ([`mpm`]); water as a height
//! field — shallow water that floods and ripples that ring
//! ([`heightfield`]); the open sea ([`ocean`]).
//!
//! It stands on the core, geometry (surfaces of fields) and the soft
//! module (what is solid, as obstacles). Headless: it hands out particles
//! and surfaces, and the water's height at a point ([`water_height`]); the
//! facade draws them and floats bodies on them.

pub mod heightfield;
pub mod mpm;
pub mod floats;
pub mod ocean;
pub mod smoke;

pub use heightfield::{run_heightfields, water_height, HeightfieldDress, HeightfieldLine, Ripples, RipplesState, ShallowState, ShallowWater, SnowCover, SnowState};
pub use mpm::{run_mpm, Mpm, MpmDress, MpmLine, MpmMaterial, MpmState, Transfer};
pub use ocean::{run_oceans, Ocean, OceanDress, OceanLine, OceanState};
pub use smoke::{count_smokes, run_smokes, Smoke, SmokeDress, SmokeLine, SmokeState};

/// Every ocean and smoke moved by this wind: the scene's.
pub fn set_wind(world: &mut hecs::World, wind: runity_core::wind::Wind) {
    ocean::set_wind(world, wind);
    smoke::set_wind(world, wind);
}
pub use floats::{Floats, FloatsDress, FloatsLine, Floating};

/// This module's fields of a line, with how to check each one's text.
pub fn part_kinds() -> Vec<runity_core::parts::PartKind> {
    let mut kinds = mpm::part_kinds();
    kinds.extend(heightfield::part_kinds());
    kinds.extend(ocean::part_kinds());
    kinds.extend(floats::part_kinds());
    kinds.extend(smoke::part_kinds());
    kinds
}

/// This module's manifest (`module.ron`).
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = runity_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "fluid");
        let problems = manifest.part_problems(&super::part_kinds());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
