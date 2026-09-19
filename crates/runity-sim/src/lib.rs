//! The simulation contract: the step a game world takes that knows nothing
//! about pixels, windows or sockets.
//!
//! [`Tick`] is the simulation's own clock, distinct from
//! `runity_core::Time` (frame timing and presentation — not used here).
//! [`Simulation`] is the step itself; [`Wire`] round-trips entity state as
//! text, the shape shared by save files and network snapshots; [`CommandLog`]
//! records what was applied and when; [`run`] drives a fixed number of ticks
//! with no render loop involved.

#![forbid(unsafe_code)]

mod client;
mod command_log;
mod run;
mod simulation;
mod tick;
mod wire;

pub use client::ClientId;
pub use command_log::CommandLog;
pub use run::run;
pub use simulation::Simulation;
pub use tick::Tick;
pub use wire::{read, write, Header, Wire, WireError, WIRE_FORMAT_VERSION};
