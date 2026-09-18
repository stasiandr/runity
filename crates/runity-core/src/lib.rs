//! The engine layer: entities, timing, input and the main loop that ties the
//! renderer to a window.

#![forbid(unsafe_code)]

pub mod app;
pub mod authority;
pub mod clock;
pub mod headless;
pub mod input;
pub mod lod;
pub mod predict;
pub mod replicate;
pub mod scene;
pub mod time;
pub mod transform;
pub mod world;

pub use app::{App, DebugView, Engine, Game, RunOptions};
pub use authority::{Authority, Command, CommandQueue, Lease, PeerId};
pub use clock::{Calendar, Date, Season, WorldClock};
pub use input::Input;
pub use lod::{Cadence, Detail, Due, Region, Regions};
pub use predict::{Correction, Predictable, Prediction};
pub use replicate::{Everything, Interest, Nearby, Replica, Replication, SnapshotStats};
pub use scene::{Node, Scene};
pub use time::Time;
pub use transform::{Camera, Transform};
pub use world::{Despawned, Entity, Mut, World};
