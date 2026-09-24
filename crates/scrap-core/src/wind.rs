//! The scene's wind: what the render module sways foliage by and the
//! physics module blows `blown` bodies with.
//!
//! A deviation from the bare core, named (DNA, postulate 3): two numbers
//! read by two modules that do not depend on each other, and a module of
//! its own would be a crate for one struct. When weather becomes a module,
//! the wind moves there.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// The scene's wind.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wind {
    /// Which way it blows; only its level part counts.
    pub direction: Vec3,
    /// 0 is still air; 1 a breeze; 3 a gale.
    pub strength: f32,
}

impl Default for Wind {
    /// A breeze from the west: foliage is never quite still.
    fn default() -> Self {
        Self {
            direction: Vec3::new(1.0, 0.0, 0.3),
            strength: 1.0,
        }
    }
}

crate::impl_parts! {
    Wind => "wind";
}
