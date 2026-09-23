//! A place in the kitchen a cook works at, one tile of it: a counter to
//! put things on, a crate of food, a board to chop on, a stove with its
//! pot, the stack of plates, the window orders go out of, the bin.
//! `components: { "station": (kind: Crate(Tomato)) }`.

use serde::Deserialize;

use crate::components::item::Food;

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Counter,
    Crate(Food),
    Board,
    Stove,
    Plates,
    Window,
    Bin,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct Station {
    pub kind: Kind,
}
