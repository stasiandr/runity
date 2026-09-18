//! The engine layer: entities, timing, input and the main loop that ties the
//! renderer to a window.

#![forbid(unsafe_code)]

pub mod app;
pub mod input;
pub mod time;
pub mod transform;
pub mod world;

pub use app::{App, Engine, Game, RunOptions};
pub use input::Input;
pub use time::Time;
pub use transform::{Camera, Transform};
pub use world::{Entity, World};
