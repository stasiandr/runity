//! Physics: what a line says about its body — `body`, `collider`,
//! `physics`, `joint` — the components the world keeps of them, and the
//! simulation that moves them, on rapier (docs/modules.md).
//!
//! It stands on the core and on geometry: a `collider: Model` is made of
//! the line's model or terrain, which are geometry's. It knows nothing of
//! drawing; the network module moves bodies between peers through the
//! core's `Owned` and `Replica` and this module's `Takeover`.

pub mod bodies;
pub mod body;
#[cfg(feature = "rapier")]
mod shapes;
#[cfg(feature = "rapier")]
pub mod physics;

#[cfg(feature = "rapier")]
pub use physics::{BodyHandle, PhysicsDress, PhysicsWorld, RayHit};

// The core and geometry, under the names this module's code knows them by.
#[allow(unused_imports)]
use scrap_core::{defaults, id, impl_parts, layers, AssetLink, EntityDesc, EntityId, Library, Scene};
#[allow(unused_imports)]
use scrap_geometry::builtin;

/// The scene's lines, with this module's fields and geometry's beside the
/// core's.
#[allow(unused_imports)]
mod scene {
    pub use crate::body::*;
    pub use scrap_core::scene::*;
    pub use scrap_geometry::line::*;
}

/// The world, with this module's components beside the core's.
#[allow(unused_imports)]
mod world {
    pub use crate::bodies::*;
    pub use scrap_core::world::*;
}

/// The core's archive with geometry's formats.
#[allow(unused_imports)]
mod asset {
    pub use scrap_core::asset::*;
    pub use scrap_geometry::mesh_asset::*;
}

/// The traits that read a line's fields.
#[allow(unused_imports)]
mod prelude {
    pub use crate::body::{PhysicsLine, PhysicsOverride};
    pub use scrap_geometry::line::{GeometryLine, GeometryOverride};
    pub use scrap_geometry::mesh_asset::MeshLibrary;
}

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = scrap_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "physics");
        let problems = manifest.part_problems(&crate::body::part_kinds());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
