//! The sink: the plates that came back from the window dirty, and how far
//! the one being washed is. The host's, sent to the others.

use serde::{Deserialize, Serialize};

/// Sent to the other players by the host.
pub const NETWORKED: bool = true;

/// Seconds of scrubbing a plate takes.
pub const WASH_SECONDS: f32 = 1.6;
/// Seconds after a plate goes out of the window that it is back, dirty.
pub const RETURN_SECONDS: f32 = 5.0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sink {
    pub dirty: u32,
    /// 0 to 1, for the plate on top.
    pub washed: f32,
}
