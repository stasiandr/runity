//! Who plays a cook: their peer number. The host seats the players; a cook
//! nobody plays has no seat and stands out of the kitchen.

use serde::{Deserialize, Serialize};

/// Sent to the other players by the host.
pub const NETWORKED: bool = true;

/// Which player plays a cook, by their peer number; a cook nobody plays
/// has none and stands out of the kitchen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seat(pub u32);

