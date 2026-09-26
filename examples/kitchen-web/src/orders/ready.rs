//! A player says they are ready, in the lobby, on their own cook: the
//! player owns the cook, so it is theirs to send, not the host's.

use serde::{Deserialize, Serialize};

/// Sent to the other players by whoever plays the cook.
pub const NETWORKED: bool = true;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ready(pub bool);
