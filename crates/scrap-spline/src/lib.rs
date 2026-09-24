//! Splines: a line's `spline` (a curve through points) and `along` (copies
//! of a model set along it), and growing those copies when a scene's
//! prefabs expand ([`spline::grow_all`], which the engine passes to the
//! core's `prefab::instantiate_with`).

pub mod spline;

pub use spline::{grow_all, Along, Spline, SplineLine};

// The core, under the names this module's code knows it by.
#[allow(unused_imports)]
use scrap_core::{defaults, id, impl_parts, AssetLink};

/// The scene's lines, with this module's fields and geometry's beside the
/// core's.
#[allow(unused_imports)]
mod scene {
    pub use crate::spline::*;
    pub use scrap_core::scene::*;
    pub use scrap_geometry::line::*;
}

/// The traits that read a line's fields.
#[allow(unused_imports)]
mod prelude {
    pub use crate::spline::SplineLine;
    pub use scrap_geometry::line::GeometryLine;
}

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = scrap_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "spline");
        let problems = manifest.part_problems(&crate::spline::part_kinds());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
