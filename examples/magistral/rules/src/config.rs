//! The game's numbers, as `content/magistral/core/rules.ron` writes them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Card;

/// The file as the game ships it; `Config::default()` is this.
pub const RULES: &str = include_str!("../../content/magistral/core/rules.ron");

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CardStats {
    pub cost: u8,
    pub cells: u8,
    pub speed: u8,
    pub capacity: u8,
    pub draw: u8,
    pub energy: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub turns_per_stage: u8,
    pub payments: Vec<i32>,
    pub start_money: i32,
    pub hand: u8,
    pub energy: u8,
    pub remove_cost: i32,
    pub stock_cap: u8,
    pub warehouse_cap: u8,
    pub factory_rate: u8,
    pub max_trains: u8,
    pub price: Vec<i32>,
    pub distance_bonus_every: i32,
    pub subsidy_money: i32,
    pub subsidy_debt: i32,
    pub shares_price: i32,
    pub shares_income: i32,
    pub cards: BTreeMap<Card, CardStats>,
    pub start_deck: Vec<Card>,
    pub pool: Vec<Card>,
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, ron::error::SpannedError> {
        ron::from_str(text)
    }

    pub fn card(&self, card: Card) -> &CardStats {
        static NONE: CardStats = CardStats {
            cost: 0,
            cells: 0,
            speed: 0,
            capacity: 0,
            draw: 0,
            energy: 0,
        };
        self.cards.get(&card).unwrap_or(&NONE)
    }
}

impl Default for Config {
    fn default() -> Config {
        Config::parse(RULES).expect("rules.ron as shipped parses")
    }
}
