//! Networking for runity: UDP, reliability and replication.
//!
//! The game this engine is for is multiplayer, and its world keeps developing
//! whether or not anyone is watching — which makes the network layer part of
//! the simulation's design rather than a thing bolted on at the end.
//!
//! Three deliberate choices:
//!
//! * **UDP, with reliability chosen per message.** TCP makes a position
//!   update wait behind a lost chat line; a game wants the position dropped
//!   and the chat line resent.
//! * **The protocol knows nothing about sockets.** [`Connection`] takes bytes
//!   and hands back bytes, so every reliability behaviour is tested against a
//!   simulated link with loss, reordering and duplication — deterministically,
//!   with no network involved.
//! * **The same encoding as saves.** A snapshot on the wire and a snapshot on
//!   disk are the same bytes, which means the network path is exercised by
//!   every save test and vice versa.

#![forbid(unsafe_code)]

pub mod connection;
pub mod host;
pub mod packet;
pub mod transport;

pub use connection::{
    Connection, ConnectionSettings, ConnectionState, ConnectionStats, MAX_MESSAGE_CHUNK,
};
pub use host::{Host, HostEvent};
pub use packet::{Header, PacketKind, Reception, MAX_PACKET, PROTOCOL};
pub use transport::{Transport, UdpTransport};
