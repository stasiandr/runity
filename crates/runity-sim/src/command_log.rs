//! Every command applied to the simulation, in the order it was applied —
//! the tape a network layer or a save file replays.

use crate::{ClientId, Tick};

/// `(tick, client, command)` entries, in application order.
#[derive(Debug, Clone)]
pub struct CommandLog<C> {
    entries: Vec<(Tick, ClientId, C)>,
}

impl<C> CommandLog<C> {
    /// An empty log.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Record a command as applied, after everything already logged.
    pub fn push(&mut self, tick: Tick, client: ClientId, command: C) {
        self.entries.push((tick, client, command));
    }

    /// Every entry, in the order [`CommandLog::push`] was called.
    pub fn entries(&self) -> &[(Tick, ClientId, C)] {
        &self.entries
    }

    /// The commands recorded for one tick, in application order.
    pub fn commands_at(&self, tick: Tick) -> impl Iterator<Item = (ClientId, &C)> {
        self.entries
            .iter()
            .filter(move |(t, _, _)| *t == tick)
            .map(|(_, client, command)| (*client, command))
    }

    /// The number of entries logged.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no command has been logged yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<C> Default for CommandLog<C> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_replay_in_application_order() {
        let mut log = CommandLog::new();
        log.push(Tick(2), ClientId(1), "second-tick");
        log.push(Tick(1), ClientId(2), "pushed-second-but-earlier-tick");
        log.push(Tick(2), ClientId(3), "third-push");

        assert_eq!(
            log.entries(),
            [
                (Tick(2), ClientId(1), "second-tick"),
                (Tick(1), ClientId(2), "pushed-second-but-earlier-tick"),
                (Tick(2), ClientId(3), "third-push"),
            ]
        );
    }

    #[test]
    fn commands_at_a_tick_keep_their_relative_order() {
        let mut log = CommandLog::new();
        log.push(Tick(0), ClientId(1), 1);
        log.push(Tick(1), ClientId(2), 2);
        log.push(Tick(0), ClientId(3), 3);

        let at_zero: Vec<_> = log.commands_at(Tick(0)).map(|(c, cmd)| (c, *cmd)).collect();
        assert_eq!(at_zero, vec![(ClientId(1), 1), (ClientId(3), 3)]);
    }

    #[test]
    fn new_log_is_empty() {
        let log: CommandLog<()> = CommandLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert_eq!(log.entries(), []);
    }
}
