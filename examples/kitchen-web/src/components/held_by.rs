//! Which cook holds an item, by the cook's index: set by the host from who
//! holds what, so every peer can put the item in the hands of its own copy
//! of that cook — where the player driving it sees it at once, not a round
//! trip behind.

use serde::{Deserialize, Serialize};

/// Sent to the other players by the host.
pub const NETWORKED: bool = true;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldBy(pub u32);
