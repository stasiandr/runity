//! A part of a station or a plate the game shows and hides by name: the
//! soup in a pot, the bar over a board, the soup on a plate.

use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct Mark {
    pub name: String,
}
