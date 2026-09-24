//! A region of the world streamed in and out by the camera: `stream:
//! (scene: "oasis", radius: 80.0)` on an entity puts `scenes/oasis.ron`
//! into the world while the camera is within `radius` metres of it, and
//! takes it out again once it is farther than `radius + keep`. The field
//! is the core's — it is about what is in the world — and the loading is
//! the build's, which knows how to draw what it loads (`runity::streaming`).

use serde::{Deserialize, Serialize};

/// A streamed region, as a scene line writes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stream {
    /// The scene put in, by its name in `scenes/`: its own entities where
    /// they stand in it — the world's coordinates, as Unity's additive
    /// scenes.
    pub scene: String,
    /// Metres from the entity within which it is in.
    pub radius: f32,
    /// Metres more before it is taken out again: walking along the edge
    /// does not load and unload it every step.
    pub keep: f32,
}

impl Default for Stream {
    fn default() -> Self {
        Self {
            scene: String::new(),
            radius: 60.0,
            keep: 15.0,
        }
    }
}

crate::impl_parts! {
    Stream => "stream";
}
