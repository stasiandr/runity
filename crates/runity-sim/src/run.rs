//! Driving a simulation with no render loop.

use crate::{Simulation, Tick};

/// Step `sim` forward `ticks` times from [`Tick::ZERO`], with no commands.
/// This is what a headless replay or a determinism test drives instead of a
/// render loop — a game wires its own commands in by calling
/// [`Simulation::tick`] directly.
pub fn run<S: Simulation>(sim: &mut S, ticks: u64) {
    for i in 0..ticks {
        sim.tick(Tick(i), &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClientId;

    #[derive(Default)]
    struct Recorder {
        seen: Vec<Tick>,
    }

    impl Simulation for Recorder {
        type Command = ();

        fn tick(&mut self, tick: Tick, commands: &[(ClientId, Self::Command)]) {
            assert!(commands.is_empty(), "run() sends no commands");
            self.seen.push(tick);
        }
    }

    #[test]
    fn run_drives_the_requested_number_of_ticks_from_zero() {
        let mut sim = Recorder::default();
        run(&mut sim, 5);
        assert_eq!(sim.seen, vec![Tick(0), Tick(1), Tick(2), Tick(3), Tick(4)]);
    }

    #[test]
    fn running_zero_ticks_does_nothing() {
        let mut sim = Recorder::default();
        run(&mut sim, 0);
        assert!(sim.seen.is_empty());
    }
}
