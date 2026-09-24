//! What the `runity` command does, as functions, so tests and other tools
//! call the same code the command line does.

pub mod add;
pub mod build;
pub mod check;
pub mod merge;
pub mod modules;
pub mod rebuild;
pub mod run;
pub mod template;

pub use check::{check, Finding, Severity};
pub use rebuild::{rebuild_time, RebuildTime};
