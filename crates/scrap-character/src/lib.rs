//! Characters in physics (docs/simulation.md, item 11): a humanoid ragdoll
//! from capsules and joints, limp or active — its muscles PD controllers
//! pulling toward a pose ([`ragdoll`]) — the pose from a procedural gait
//! ([`body`]) or from motion matching over it ([`matching`]); IK for
//! limbs ([`ik`]); and many-legged crawlers stepping by IK
//! ([`crawler`]).
//!
//! Headless: it builds the bodies and says what the muscles want; the
//! facade pulls through the physics and draws them.

pub mod body;
pub mod crawler;
pub mod ik;
pub mod matching;
pub mod ragdoll;

pub use crawler::{Crawler, CrawlerLine, CrawlerState};
pub use ragdoll::{capsules, spawn_parts, targets, CrawlerDress, Drive, Mode, Ragdoll, RagdollDress, RagdollLine, RagdollPart, RagdollState, Targets};

/// This module's fields of a line, with how to check each one's text.
pub fn part_kinds() -> Vec<scrap_core::parts::PartKind> {
    let mut kinds = ragdoll::part_kinds();
    kinds.extend(crawler::part_kinds());
    kinds
}

/// This module's manifest (`module.ron`).
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = scrap_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "character");
        let problems = manifest.part_problems(&super::part_kinds());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
