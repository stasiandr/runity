//! A cook. `index` is whose keys move it (0: WASD, 1: the arrows), `speed`
//! metres a second.

use serde::Deserialize;

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct Player {
    pub index: u32,
    pub speed: f32,
}
