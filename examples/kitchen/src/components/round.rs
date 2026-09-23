//! The round as everyone sees it — the orders, the score, the clock — kept
//! on the kitchen's line by the host.

use crate::components::item::Food;
use serde::{Deserialize, Serialize};

/// Sent to the other players by the host.
pub const NETWORKED: bool = true;

/// A soup someone wants, and how long they will still wait.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Order {
    pub food: Food,
    pub left: f32,
    pub total: f32,
}

/// The round as everyone sees it: kept on the kitchen's line by the host.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Round {
    pub orders: Vec<Order>,
    pub score: i32,
    pub served: u32,
    pub time_left: f32,
    pub over: bool,
    /// The doors are open: the host started the round. Until then the
    /// kitchen is the lobby — cooks walk about, nothing is ordered and
    /// the clock waits.
    #[serde(default)]
    pub open: bool,
    /// A word for the screen, and how long it stays.
    pub note: Option<(String, f32)>,
}

impl Round {
    pub fn say(&mut self, words: impl Into<String>) {
        self.note = Some((words.into(), 2.0));
    }
}

