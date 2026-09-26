//! Turning in place. A scene line gives it as
//! `components: { "spin": (degrees_per_second: 45.0) }` — the file's name is
//! the name — and changing the number while the game runs changes the speed.

use serde::Deserialize;

#[derive(Deserialize)]
pub struct Spin {
    pub degrees_per_second: f32,
}
