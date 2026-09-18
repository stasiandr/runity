//! Spatial queries, navigation and decision making.
//!
//! The renderer answers "what does this look like"; this crate answers "what
//! is near me, how do I get there, and what should I be doing". None of it
//! knows about entities or components — it takes positions and returns
//! answers, so the game decides how to store its agents.
//!
//! Everything here is deterministic on purpose, and in two places that cost
//! something: searches work in integer costs so priority-queue ties break the
//! same way on every machine, and every neighbour list is in a fixed order.
//! A world simulated on two machines cannot afford a villager who walks left
//! on one of them.

#![forbid(unsafe_code)]

pub mod nav;
pub mod path;
pub mod spatial;

pub use nav::{NavGrid, BLOCKED, OPEN};
pub use path::{find_path, FlowField, Path, PathError, PathFinder, PathSettings};
pub use spatial::{ground_distance, SpatialGrid};
