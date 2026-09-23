//! What the `runity` command does, as functions, so tests and other tools
//! call the same code the command line does.

pub mod check;

pub use check::{check, Finding, Severity};
