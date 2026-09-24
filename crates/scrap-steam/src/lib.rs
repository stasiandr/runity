//! Steam (DNA, postulate 4: "Steam — модуль"): lobbies, invites through
//! friends and peer-to-peer through NAT, as one more transport of the
//! network module ([`steam::Steam`]).

pub mod steam;

pub use steam::Steam;

// The core and the network module, under the names this module's code
// knows them by.
#[allow(unused_imports)]
use scrap_core::player_prefs;
#[allow(unused_imports)]
use scrap_net::{net, relay};

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = scrap_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "steam");
        let problems = manifest.part_problems(&Vec::new());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
