//! Routes: a line's `route` — a way through points and how to travel it —
//! and the system that moves each travelling entity along its way in the
//! fixed step ([`routes::run_routes`]).

pub mod routes;

pub use routes::{run_routes, Route, RouteDress, RouteEnds, RouteLine};

// The core, under the names this module's code knows it by.
#[allow(unused_imports)]
use runity_core::{defaults, id, impl_parts, world};

/// The scene's lines, with this module's fields beside the core's.
#[allow(unused_imports)]
mod scene {
    pub use crate::routes::{Route, RouteEnds};
    pub use runity_core::scene::*;
}

/// The traits that read a line's fields.
#[allow(unused_imports)]
mod prelude {
    pub use crate::routes::RouteLine;
}
