//! The simulation contract itself.

use crate::{ClientId, Tick};

/// A game's simulation step, decoupled from rendering, windowing and
/// networking — the contract the rest of the network layer is built on.
///
/// [`Simulation::tick`] is the only place state changes; everything else
/// (rendering, input capture, the network transport) happens around it, not
/// inside it, which is what makes it replayable from a [`crate::CommandLog`]
/// and safe to run identically on a server and every client.
pub trait Simulation {
    /// The input a client can send for one tick.
    type Command;

    /// Advance the simulation by one tick, applying every command received
    /// for it, in application order.
    fn tick(&mut self, tick: Tick, commands: &[(ClientId, Self::Command)]);

    /// A client joined the simulation. The default does nothing.
    fn on_join(&mut self, _client: ClientId) {}

    /// A client left the simulation — disconnected, not merely idle. The
    /// default does nothing; a game that needs to freeze or despawn the
    /// client's body overrides this.
    fn on_leave(&mut self, _client: ClientId) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Counter {
        ticks: Vec<Tick>,
        applied: Vec<(ClientId, i32)>,
        joined: Vec<ClientId>,
        left: Vec<ClientId>,
    }

    impl Simulation for Counter {
        type Command = i32;

        fn tick(&mut self, tick: Tick, commands: &[(ClientId, Self::Command)]) {
            self.ticks.push(tick);
            self.applied.extend_from_slice(commands);
        }

        fn on_join(&mut self, client: ClientId) {
            self.joined.push(client);
        }

        fn on_leave(&mut self, client: ClientId) {
            self.left.push(client);
        }
    }

    #[test]
    fn tick_receives_the_tick_and_its_commands() {
        let mut sim = Counter::default();
        sim.tick(Tick(3), &[(ClientId(1), 7), (ClientId(2), -1)]);
        assert_eq!(sim.ticks, vec![Tick(3)]);
        assert_eq!(sim.applied, vec![(ClientId(1), 7), (ClientId(2), -1)]);
    }

    #[test]
    fn lifecycle_hooks_default_to_a_no_op() {
        struct Silent;
        impl Simulation for Silent {
            type Command = ();
            fn tick(&mut self, _tick: Tick, _commands: &[(ClientId, ())]) {}
        }

        // Neither hook needs an override to compile or to run.
        let mut sim = Silent;
        sim.on_join(ClientId(1));
        sim.on_leave(ClientId(1));
    }

    #[test]
    fn overridden_hooks_observe_join_and_leave() {
        let mut sim = Counter::default();
        sim.on_join(ClientId(5));
        sim.on_leave(ClientId(5));
        assert_eq!(sim.joined, vec![ClientId(5)]);
        assert_eq!(sim.left, vec![ClientId(5)]);
    }
}
