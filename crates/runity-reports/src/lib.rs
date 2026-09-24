//! Reports: the core's crash report sent to Sentry, and play reported to
//! GameAnalytics — the network, TLS and a player's consent, none of which
//! a game gets without asking for them.

pub mod reports;

// The core, under the names this module's code knows it by.
#[allow(unused_imports)]
use runity_core::{crash, id};
