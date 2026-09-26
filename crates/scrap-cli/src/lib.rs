//! What the `scrap` command does, as functions, so tests and other tools
//! call the same code the command line does.

pub mod add;
pub mod build;
pub mod check;
pub mod lines;
pub mod merge;
pub mod migrate;
pub mod modules;
pub mod perf;
pub mod rebuild;
pub mod run;
pub mod template;

pub use check::{check, Finding, Severity};
pub use rebuild::{rebuild_time, RebuildTime};
