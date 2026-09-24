//! Animation: a skeleton posed from its clips ([`animator`]), graphs of
//! states and transitions that pick the clip ([`animgraph`], written as
//! text by [`graph_text`]), and motion clips that move, switch and turn a
//! scene's things rather than bones ([`motion`]) — Unity's Animator and
//! Animation Clips (docs/modules.md).
//!
//! It plays what geometry holds — skeletons and clips as data — and
//! leaves the pose on the entity ([`Posed`]) for whoever draws. A clip's
//! track of another module's property (a sound's volume, particles' rate)
//! is handed to that module through [`motion::run_with`].

pub mod animator;
pub mod animgraph;
pub mod graph_text;
pub mod motion;

pub use animator::{advance_animations, Animator, Playing};
pub use runity_geometry::animation::Posed;

// The core and geometry, under the names this module's code knows them by.
#[allow(unused_imports)]
use runity_core::{id, impl_parts, project, ron_edit, spelling, AssetLink, Transform};
#[allow(unused_imports)]
use runity_geometry::animation;

/// The scene's lines, with this module's fields and geometry's beside the
/// core's.
#[allow(unused_imports)]
mod scene {
    pub use crate::motion::{AnimatorRef, BoneName};
    pub use runity_core::scene::*;
    pub use runity_geometry::line::*;
}

/// The world, with this module's components beside the core's.
#[allow(unused_imports)]
mod world {
    pub use crate::motion::OnBone;
    pub use runity_core::world::*;
    pub use runity_geometry::animation::Posed;
}

/// The core's archive with geometry's formats.
#[allow(unused_imports)]
mod asset {
    pub use runity_core::asset::*;
    pub use runity_geometry::mesh_asset::*;
}

/// The traits that read a line's fields.
#[allow(unused_imports)]
mod prelude {
    pub use crate::motion::AnimationLine;
    pub use runity_geometry::line::{GeometryLine, GeometryOverride};
}
