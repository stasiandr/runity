//! How long meat has lain on a hot pan, seconds: a patty after a while,
//! burnt after a while longer. Set by the host.

use serde::{Deserialize, Serialize};

/// Sent to the other players by the host.
pub const NETWORKED: bool = true;

/// Seconds on the pan for a patty, and for a burnt one.
pub const FRY_SECONDS: f32 = 5.0;
pub const FRY_BURN: f32 = 13.0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Fry(pub f32);

impl Fry {
    pub fn done(self) -> bool {
        self.0 >= FRY_SECONDS && !self.burnt()
    }

    pub fn burnt(self) -> bool {
        self.0 >= FRY_BURN
    }
}
