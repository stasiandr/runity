//! The engine layer: entities, timing, input and the main loop that ties the
//! renderer to a window.

#![forbid(unsafe_code)]

pub mod app;
pub mod assets;
pub mod economy;
pub mod headless;
pub mod input;
pub mod needs;
pub mod physics;
pub mod rng;
pub mod time;
pub mod transform;
pub mod world;

pub use app::{App, DebugView, Engine, Game, Renderer, RunOptions, UnsupportedView};
pub use assets::AssetCache;
pub use input::Input;
pub use rng::Rng;
pub use time::Time;
pub use transform::{Camera, Transform};
pub use world::{Entity, World};
