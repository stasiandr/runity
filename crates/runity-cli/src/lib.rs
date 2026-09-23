//! What the `runity` command does, as functions, so tests and other tools
//! call the same code the command line does.

pub mod check;
pub mod rebuild;

pub use check::{check, Finding, Severity};
pub use rebuild::{rebuild_time, RebuildTime};
