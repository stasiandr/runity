//! The round: how long it lasts, how often an order comes in and how long
//! one waits before it is lost. On one line of the scene; the orders, the
//! score and the clock are kept beside it while the game runs.

use serde::Deserialize;

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct Kitchen {
    pub round_seconds: f32,
    pub order_every: f32,
    pub order_seconds: f32,
}
