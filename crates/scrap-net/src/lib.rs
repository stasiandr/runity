//! Networking (DNA, postulate 4): the network model — who owns an entity
//! and how it changes hands, state sync with prediction, the wire and its
//! transports (UDP by default, a relay, a loopback) — parties of players
//! and finding a game on the LAN ([`net`], [`party`], [`relay`],
//! [`lan`]). Who simulates an entity is the core's (`Owned`, `Replica`);
//! which peer does, and handing a body's speed over (`Takeover`), is here
//! and in the physics module. Steam is a transport of its own module.
//!
//! [`bench`], with the `physics` feature: a stretch of physics played
//! alone and then passed around over a worsening link, measured.

pub mod lan;
pub mod net;
pub mod party;
pub mod relay;
#[cfg(feature = "physics")]
pub mod bench;

// The core and physics, under the names this module's code knows them by.
#[allow(unused_imports)]
use scrap_core::{components, id, player_prefs, save as save_core, world as world_core, EntityId};
#[allow(unused_imports)]
use scrap_physics::bodies;
#[cfg(feature = "physics")]
#[allow(unused_imports)]
use scrap_physics::physics;

/// The scene's lines, with the physics module's fields beside the core's.
#[allow(unused_imports)]
mod scene {
    pub use scrap_core::scene::*;
    pub use scrap_physics::body::*;
}

/// The world, with the physics module's components beside the core's.
#[allow(unused_imports)]
mod world {
    pub use scrap_core::world::*;
    pub use scrap_physics::bodies::*;
}

/// The traits that read a line's fields.
#[allow(unused_imports)]
mod prelude {
    pub use scrap_physics::body::{PhysicsLine, PhysicsOverride};
}

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = scrap_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "net");
        let problems = manifest.part_problems(&Vec::new());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
