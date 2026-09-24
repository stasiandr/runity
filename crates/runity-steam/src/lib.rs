//! Steam (DNA, postulate 4: "Steam — модуль"): lobbies, invites through
//! friends and peer-to-peer through NAT, as one more transport of the
//! network module ([`steam::Steam`]).

pub mod steam;

pub use steam::Steam;

// The core and the network module, under the names this module's code
// knows them by.
#[allow(unused_imports)]
use runity_core::player_prefs;
#[allow(unused_imports)]
use runity_net::{net, relay};
