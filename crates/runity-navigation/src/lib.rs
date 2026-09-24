//! Navigation: Unity's NavMesh as a grid baked from the physics module's
//! static world, and paths over it ([`navigation::NavGrid`]).

pub mod navigation;

/// The scene's lines, with the physics module's fields beside the core's.
#[allow(unused_imports)]
mod scene {
    pub use runity_core::scene::*;
    pub use runity_physics::body::*;
}

#[allow(unused_imports)]
use runity_core::world;
#[allow(unused_imports)]
use runity_physics::physics;
