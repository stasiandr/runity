//! What the `runity` command does, as functions, so tests and other tools
//! call the same code the command line does.

pub mod add;
pub mod build;
pub mod check;
pub mod merge;
pub mod rebuild;

pub use check::{check, Finding, Severity};
pub use rebuild::{rebuild_time, RebuildTime};
