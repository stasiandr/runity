//! Input actions (DNA, "Ввод — как Unity Input System"): the core knows
//! device events; this module names what they mean to a game — `jump`,
//! `move` — and binds each to keys, buttons and sticks, from `input.ron`,
//! reloaded while the game runs. Action maps and control schemes grow
//! here, `input.ron` staying their simplest case.

pub mod actions;

pub use actions::{Actions, Binding};

// The core, under the names this module's code knows it by.
#[allow(unused_imports)]
use runity_core::{input, spelling};

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = runity_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "input");
        let problems = manifest.part_problems(&Vec::new());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
