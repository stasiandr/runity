//! The Valley: the rules, and nothing else.
//!
//! This crate knows about settlers, work and the day; it does not know about
//! wgpu, windows or file formats. It is also the crate that gets hot-patched
//! while the game runs, which is why it is separate: the smaller and purer it
//! is, the faster a change to it lands on screen.
