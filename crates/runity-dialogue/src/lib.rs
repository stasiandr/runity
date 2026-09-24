//! Dialogue: a conversation as a graph of lines and choices, written in
//! RON, played by the game and checked for what it names that is not
//! there.

pub mod dialogue;

// The core, under the names this module's code knows it by.
#[allow(unused_imports)]
use runity_core::spelling;

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = runity_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "dialogue");
        let problems = manifest.part_problems(&Vec::new());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
