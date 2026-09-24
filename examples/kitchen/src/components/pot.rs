//! A stove's pot: what went in, and how long it has cooked. The host's,
//! sent to the others so everyone sees the soup.

use crate::components::item::Food;
use crate::components::served::Soup;
use serde::{Deserialize, Serialize};

/// Sent to the other players by the host.
pub const NETWORKED: bool = true;

/// A stove's pot: what went in, and how long it has cooked.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pot {
    pub foods: Vec<Food>,
    pub cooked: f32,
}

/// A pot takes three.
pub const POT_HOLDS: usize = 3;
/// Seconds for a full pot to be soup, and then to burn.
pub const COOK_SECONDS: f32 = 6.0;
pub const BURN_SECONDS: f32 = 14.0;
/// Seconds after which a burnt pot left on the heat catches fire.
pub const FIRE_SECONDS: f32 = 20.0;

impl Pot {
    pub fn soup(&self) -> Option<Soup> {
        let first = *self.foods.first()?;
        Some(if self.foods.iter().all(|f| *f == first) {
            Soup::Of(first)
        } else {
            Soup::Mixed
        })
    }

    pub fn done(&self) -> bool {
        self.foods.len() == POT_HOLDS && self.cooked >= COOK_SECONDS && !self.burnt()
    }

    pub fn burnt(&self) -> bool {
        self.cooked >= BURN_SECONDS
    }

    pub fn on_fire(&self) -> bool {
        self.cooked >= FIRE_SECONDS
    }
}

