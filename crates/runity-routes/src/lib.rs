//! Routes: a line's `route` — a way through points and how to travel it —
//! and the system that moves each travelling entity along its way in the
//! fixed step ([`routes::run_routes`]).

pub mod routes;

pub use routes::{run_routes, Route, RouteDress, RouteEnds, RouteLine};

// The core, under the names this module's code knows it by.
#[allow(unused_imports)]
use runity_core::{defaults, id, impl_parts, world};

/// The scene's lines, with this module's fields beside the core's.
#[allow(unused_imports)]
mod scene {
    pub use crate::routes::{Route, RouteEnds};
    pub use runity_core::scene::*;
}

/// The traits that read a line's fields.
#[allow(unused_imports)]
mod prelude {
    pub use crate::routes::RouteLine;
}

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = runity_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "routes");
        let problems = manifest.part_problems(&crate::routes::part_kinds());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
