//! The round: how long it lasts, how often an order comes in and how long
//! one waits before it is lost. On one line of the scene; the orders, the
//! score and the clock are kept beside it while the game runs.

use serde::Deserialize;

use crate::components::item::{Dish, Food};

#[derive(Deserialize, Debug, Clone)]
pub struct Kitchen {
    pub round_seconds: f32,
    pub order_every: f32,
    pub order_seconds: f32,
    /// What is ordered here, picked from at random; soups alone unless
    /// the scene says.
    #[serde(default = "soups")]
    pub menu: Vec<Dish>,
    /// Clean plates on each rack as a round starts.
    #[serde(default = "plates")]
    pub plates: u32,
    /// The score for one star, two and three.
    #[serde(default = "stars")]
    pub stars: [i32; 3],
}

impl Kitchen {
    /// Stars a score earns, 0 to 3.
    pub fn stars_for(&self, score: i32) -> usize {
        self.stars.iter().filter(|s| score >= **s).count()
    }
}

fn stars() -> [i32; 3] {
    [60, 140, 220]
}

fn soups() -> Vec<Dish> {
    vec![Dish::Soup(Food::Tomato), Dish::Soup(Food::Onion)]
}

fn plates() -> u32 {
    3
}
