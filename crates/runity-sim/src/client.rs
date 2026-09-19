//! Identifies a connected client.

/// A client connected to the simulation — the connection, not the entity it
/// controls. Stable across a disconnect and reconnect, so
/// [`crate::Simulation::on_leave`] and a later `on_join` can be matched to
/// the same player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(pub u32);
