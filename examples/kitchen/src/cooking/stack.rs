//! Clean plates on a rack, how many. Taking one takes one; the sink puts
//! them back as they are washed. The host's, sent to the others.

use serde::{Deserialize, Serialize};

/// Sent to the other players by the host.
pub const NETWORKED: bool = true;

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stack(pub u32);
