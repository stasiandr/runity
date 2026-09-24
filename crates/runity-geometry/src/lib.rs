//! Geometry: what a model is made of, before anything draws it — vertices
//! and submeshes, textures, the built-in shapes, a line's model and
//! terrain, and a rigged model's skeleton and clips as data. The render module uploads them, physics
//! makes colliders of them, animation plays the clips; none of that is
//! here (docs/modules.md).

pub mod animation;
pub mod builtin;
pub mod line;
pub mod mesh_asset;
pub mod terrain;

pub use animation::{Channel, Clip, Joint, PoseTransform, Posed, Skeleton};
pub use line::{GeometryLine, GeometryOverride, ModelRef};

// The core, under the names this module's code knows it by.
use runity_core::{library, AssetLink};

/// The core's archive with this module's formats beside it.
mod asset {
    pub use crate::mesh_asset::*;
    pub use runity_core::asset::*;
}
