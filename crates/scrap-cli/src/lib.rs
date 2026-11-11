//! What the `scrap` command does, as functions, so tests and other tools
//! call the same code the command line does.

pub mod add;
pub mod build;
pub mod check;
pub mod hub;
pub mod lines;
pub mod merge;
pub mod migrate;
pub mod modules;
pub mod perf;
pub mod rebuild;
pub mod run;
pub mod template;
/// `scrap world`: a running game's world from a shell (docs/world-control.md).
#[cfg(unix)]
pub mod world;

pub use check::{check, Finding, Severity};
pub use rebuild::{rebuild_time, RebuildTime};
