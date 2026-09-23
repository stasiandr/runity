//! Something a cook carries: food, whole or chopped, or a plate, empty or
//! with soup. The prefabs `tomato`, `onion` and `plate` have one each.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Food {
    Tomato,
    Onion,
}

impl Food {
    pub fn name(self) -> &'static str {
        match self {
            Food::Tomato => "tomato",
            Food::Onion => "onion",
        }
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
