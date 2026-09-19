//! Frame transport for runity's networking.
//!
//! One trait, [`Transport`], with two implementations: [`Loopback`], a pair
//! of in-memory queues connecting two ends in the same process with zero
//! threads, and [`Tcp`], built directly on `std::net`. Both speak the same
//! length-prefixed framing — a 4-byte little-endian length header followed by
//! that many bytes of body — so single-player and networked play run
//! identical serialization code; only what carries the bytes differs.

#![forbid(unsafe_code)]

mod framing;
mod loopback;
mod tcp;
mod transport;

pub use framing::MAX_FRAME_LEN;
pub use loopback::Loopback;
pub use tcp::Tcp;
pub use transport::Transport;
