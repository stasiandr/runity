//! Reports: the core's crash report sent to Sentry, and play reported to
//! GameAnalytics — the network, TLS and a player's consent, none of which
//! a game gets without asking for them.

pub mod reports;

// The core, under the names this module's code knows it by.
#[allow(unused_imports)]
use scrap_core::{crash, id};

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = scrap_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "reports");
        let problems = manifest.part_problems(&Vec::new());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
