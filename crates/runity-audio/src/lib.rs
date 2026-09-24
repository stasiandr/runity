//! Sound: what a line says it sounds like (`sound`), the sound asset the
//! importer writes, and the mixer that plays sounds in 3D round the
//! listener, on kira (docs/modules.md). Without the `kira` feature the
//! fields and the asset stay, so a scene with sounds opens and saves in a
//! build with no mixer.

#[cfg(feature = "kira")]
pub mod audio;
pub mod sound;

#[cfg(feature = "kira")]
pub use audio::{Audio, Falloff, SoundDress};
pub use sound::{SoundAsset, SoundLibrary, SoundLine, SoundSource, Sounding};

// The core, under the names this module's code knows it by.
#[allow(unused_imports)]
use runity_core::{defaults, impl_parts, library, AssetId, AssetLink};

/// The scene's lines, with this module's fields beside the core's.
#[allow(unused_imports)]
mod scene {
    pub use crate::sound::*;
    pub use runity_core::scene::*;
}

/// The world, with this module's components beside the core's.
#[allow(unused_imports)]
mod world {
    pub use crate::sound::Sounding;
    pub use runity_core::world::*;
}

/// The core's archive with this module's format.
#[allow(unused_imports)]
mod asset {
    pub use crate::sound::{ArchivedSoundAsset, SoundAsset};
    pub use runity_core::asset::*;
}

/// The traits that read a line's fields.
#[allow(unused_imports)]
mod prelude {
    pub use crate::sound::{SoundLibrary, SoundLine};
}
