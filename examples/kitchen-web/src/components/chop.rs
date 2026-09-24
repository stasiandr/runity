//! How far a food is chopped, 0 to 1; chopped at 1. Set by the host as a
//! cook works at a board.

use serde::{Deserialize, Serialize};

/// Sent to the other players by the host.
pub const NETWORKED: bool = true;

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Chop(pub f32);

