//! What is on a plate: a soup from a pot, or the parts of a dish built on
//! it — a salad, a burger — as they are added.

use crate::components::item::{Dish, Food, Part, BUILT};
use serde::{Deserialize, Serialize};

/// Sent to the other players by the host.
pub const NETWORKED: bool = true;

/// A soup: of one food, or of several, which nobody ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Soup {
    Of(Food),
    Mixed,
}

/// The plate's load.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Served {
    pub soup: Option<Soup>,
    /// Sorted, so a dish is the same whatever went on first.
    pub parts: Vec<Part>,
}

impl Served {
    pub fn is_empty(&self) -> bool {
        self.soup.is_none() && self.parts.is_empty()
    }

    /// The dish it is, when it is one: a soup of one food, or every part
    /// of a built dish and nothing else.
    pub fn dish(&self) -> Option<Dish> {
        if let Some(soup) = self.soup {
            return match soup {
                Soup::Of(food) => Some(Dish::Soup(food)),
                Soup::Mixed => None,
            };
        }
        BUILT.into_iter().find(|d| {
            let mut want = d.parts().to_vec();
            want.sort();
            want == self.parts
        })
    }

    /// Whether `part` may go on: a plate of soup takes nothing, and the
    /// parts must stay on the way to some dish.
    pub fn takes(&self, part: Part) -> bool {
        if self.soup.is_some() {
            return false;
        }
        let mut with = self.parts.clone();
        with.push(part);
        BUILT.into_iter().any(|d| {
            let mut left = d.parts().to_vec();
            with.iter().all(|p| match left.iter().position(|l| l == p) {
                Some(i) => {
                    left.remove(i);
                    true
                }
                None => false,
            })
        })
    }

    pub fn add(&mut self, part: Part) {
        self.parts.push(part);
        self.parts.sort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dish_is_its_parts_in_any_order_and_nothing_else_goes_on() {
        let mut plate = Served::default();
        assert!(plate.takes(Part::Patty) && plate.takes(Part::Chopped(Food::Tomato)));
        assert!(!plate.takes(Part::Chopped(Food::Onion)), "no dish has chopped onion on a plate");
        plate.add(Part::Chopped(Food::Cabbage));
        plate.add(Part::Patty);
        assert!(!plate.takes(Part::Chopped(Food::Tomato)), "a burger has no tomato");
        assert_eq!(plate.dish(), None, "not yet");
        plate.add(Part::Bun);
        assert_eq!(plate.dish(), Some(Dish::Burger));
        assert!(!plate.takes(Part::Bun), "one bun");
        let salad = Served {
            soup: None,
            parts: {
                let mut p = vec![Part::Chopped(Food::Tomato), Part::Chopped(Food::Cabbage)];
                p.sort();
                p
            },
        };
        assert_eq!(salad.dish(), Some(Dish::Salad));
        let soup = Served { soup: Some(Soup::Of(Food::Onion)), parts: vec![] };
        assert!(!soup.takes(Part::Bun));
        assert_eq!(soup.dish(), Some(Dish::Soup(Food::Onion)));
    }
}
