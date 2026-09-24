//! Splines: a line's `spline` (a curve through points) and `along` (copies
//! of a model set along it), and growing those copies when a scene's
//! prefabs expand ([`spline::grow_all`], which the engine passes to the
//! core's `prefab::instantiate_with`).

pub mod spline;

pub use spline::{grow_all, Along, Spline, SplineLine};

// The core, under the names this module's code knows it by.
#[allow(unused_imports)]
use runity_core::{defaults, id, impl_parts, AssetLink};

/// The scene's lines, with this module's fields and geometry's beside the
/// core's.
#[allow(unused_imports)]
mod scene {
    pub use crate::spline::*;
    pub use runity_core::scene::*;
    pub use runity_geometry::line::*;
}

/// The traits that read a line's fields.
#[allow(unused_imports)]
mod prelude {
    pub use crate::spline::SplineLine;
    pub use runity_geometry::line::GeometryLine;
}
