//! Something a cook carries: food, whole or chopped, raw or fried, or a
//! plate. The prefabs `tomato`, `onion`, `cabbage`, `bun`, `meat` and
//! `plate` have one each.
//!
//! The menu, and what a plate has to carry for each: a soup (three of one
//! chopped food, cooked in a pot), a salad (chopped tomato and cabbage), a
//! burger (a bun, a fried patty and chopped cabbage).

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Food {
    Tomato,
    Onion,
    Cabbage,
    /// Goes on a plate as it is.
    Bun,
    /// Fried on a pan into a patty.
    Meat,
}

impl Food {
    pub fn name(self) -> &'static str {
        match self {
            Food::Tomato => "tomato",
            Food::Onion => "onion",
            Food::Cabbage => "cabbage",
            Food::Bun => "bun",
            Food::Meat => "meat",
        }
    }

    /// Chopped on a board before it is any use.
    pub fn chops(self) -> bool {
        matches!(self, Food::Tomato | Food::Onion | Food::Cabbage)
    }

    /// A soup is made of it.
    pub fn soups(self) -> bool {
        matches!(self, Food::Tomato | Food::Onion)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Thing {
    Food(Food),
    Plate,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct Item {
    pub thing: Thing,
}

/// What goes on a plate to build a dish.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    Chopped(Food),
    Bun,
    Patty,
}

/// What someone orders.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dish {
    Soup(Food),
    Salad,
    Burger,
}

impl Dish {
    /// What goes on the plate for it, when it is built there: a soup is
    /// not, it comes out of a pot.
    pub fn parts(self) -> &'static [Part] {
        match self {
            Dish::Soup(_) => &[],
            Dish::Salad => &[Part::Chopped(Food::Tomato), Part::Chopped(Food::Cabbage)],
            Dish::Burger => &[Part::Bun, Part::Patty, Part::Chopped(Food::Cabbage)],
        }
    }

    /// Its name for the screens, as a strings key.
    pub fn key(self) -> &'static str {
        match self {
            Dish::Soup(Food::Onion) => "@order.onion",
            Dish::Soup(_) => "@order.tomato",
            Dish::Salad => "@order.salad",
            Dish::Burger => "@order.burger",
        }
    }
}

/// The dishes built on a plate.
pub const BUILT: [Dish; 2] = [Dish::Salad, Dish::Burger];
