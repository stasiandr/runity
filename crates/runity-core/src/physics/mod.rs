//! Deterministic simulation primitives.
//!
//! Nothing in this module (or its submodules) calls a transcendental function
//! (`sin`, `cos`, `tan`, `atan2`, `powf`, `to_radians`, ...). Addition,
//! subtraction, multiplication, division and `sqrt` round identically on
//! every IEEE-754 platform; transcendental functions do not carry the same
//! guarantee, and a bit of drift after a million physics ticks is not
//! acceptable. See `docs/ARCHITECTURE.md` for why golden-image tests get a
//! tolerance and this module does not.

pub mod heightfield;

pub use heightfield::Heightfield;
