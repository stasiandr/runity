//! The soup on a plate, if any.

use crate::components::item::Food;
use serde::{Deserialize, Serialize};

/// Sent to the other players by the host.
pub const NETWORKED: bool = true;

/// A soup: of one food, or of several, which nobody ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Soup {
    Of(Food),
    Mixed,
}

/// The soup on a plate.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Served(pub Option<Soup>);

