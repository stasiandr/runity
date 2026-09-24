//! Wires (docs/wires.md): a line's `wires` — what happens to which entity
//! when something comes into its body or goes out — and the system that
//! does it in the fixed step ([`wires::run_wires`]). Hammer's outputs
//! wired to inputs, UEFN's devices: a door opens when the player walks up,
//! a lamp goes out behind them, without a line of the game's code.
//!
//! The set of things a wire can do is closed and the engine's own
//! ([`Act`]): an animator's trigger or switch, a thing switched on or off,
//! a prefab spawned. No conditions, no variables, no sequence — logic is
//! the game's Rust (DNA, postulate 5).

pub mod wires;

pub use wires::{
    problems, run_wires, Act, On, SpawnOrder, Wire, WireDress, WireLine, Wired, Wires,
};

/// This module's systems in the loop: the wires, first in the fixed step,
/// so what they pull moves in the same step (the physics' contacts they
/// read are the last step's).
pub fn systems(player_loop: &mut scrap_core::player_loop::PlayerLoop) {
    player_loop.add(
        scrap_core::player_loop::Phase::FixedUpdate,
        "wires",
        run_wires,
    );
}

// The core, under the names this module's code knows it by.
#[allow(unused_imports)]
use scrap_core::{defaults, id, impl_parts, world};

/// The scene's lines, with this module's fields beside the core's.
#[allow(unused_imports)]
mod scene {
    pub use crate::wires::Wires;
    pub use scrap_core::scene::*;
}

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = scrap_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "wires");
        let problems = manifest.part_problems(&crate::wires::part_kinds());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
