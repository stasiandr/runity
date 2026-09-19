//! The settlement's day-log of deficits.

use std::collections::VecDeque;

/// One recorded attempt inside a day-bound rolling window: the tick it
/// happened on, and whether it counted against the settlement (no food found,
/// an item aged into forgetting).
#[derive(Debug, Clone, Copy, PartialEq)]
struct DayEvent {
    tick: u64,
    is_deficit: bool,
}

/// The settlement's rolling record of four deficits actually lived through,
/// per `12-minds.md` §2.1: a deficit's severity is undefined until the event
/// it measures has happened at least once, so every reading is an
/// `Option<f32>` rather than a value defaulted to `0.0` — "undefined" must be
/// able to exclude an entry from being chosen, which a `0.0` (the best
/// possible score) cannot.
///
/// Two entries are night-bound and roll over at dawn:
/// [`DeficitLog::cold_last_night`] and [`DeficitLog::unsheltered_last_night`].
/// Two are day-bound and use a trailing one-sim-day window that rolls over
/// continuously, tick by tick: [`DeficitLog::hunger_deficit`] and
/// [`DeficitLog::forgotten_items_deficit`].
///
/// This type only keeps the record. Deciding *when* a settler is cold or
/// unsheltered is [`super::tick_settlement`]'s job; deciding when an eat
/// attempt or an item's forgetting clock fires is a future card's — this log
/// just needs to be told, via [`DeficitLog::record_eat_attempt`] and
/// [`DeficitLog::record_item_aged`].
#[derive(Debug, Clone)]
pub struct DeficitLog {
    ticks_per_day: u64,
    next_dawn: u64,
    cold_last_night: Option<f32>,
    unsheltered_last_night: Option<f32>,
    eat_attempts: VecDeque<DayEvent>,
    item_agings: VecDeque<DayEvent>,
}

impl DeficitLog {
    /// A fresh log for a settlement whose sim-day is `ticks_per_day` fixed
    /// steps long — counted in ticks, not clock hours, exactly like the
    /// forgetting clock in `05-economy.md` §2.4, so it is a number a test can
    /// pin down rather than a feeling.
    pub fn new(ticks_per_day: u64) -> Self {
        assert!(
            ticks_per_day > 0,
            "a sim-day must be at least one tick long"
        );
        Self {
            ticks_per_day,
            next_dawn: ticks_per_day,
            cold_last_night: None,
            unsheltered_last_night: None,
            eat_attempts: VecDeque::new(),
            item_agings: VecDeque::new(),
        }
    }

    /// Fraction of settlers who spent last night outside every hearth's
    /// warmth radius. `None` until a first night has actually completed.
    pub fn cold_last_night(&self) -> Option<f32> {
        self.cold_last_night
    }

    /// Fraction of settlers who slept off a sleeping spot last night. `None`
    /// until a first night has actually completed.
    pub fn unsheltered_last_night(&self) -> Option<f32> {
        self.unsheltered_last_night
    }

    /// Fraction of the last sim-day's attempts to eat that found no food.
    /// `None` until at least one attempt has been recorded.
    pub fn hunger_deficit(&self) -> Option<f32> {
        fraction(&self.eat_attempts)
    }

    /// Fraction of the last sim-day's marked, away-from-home items that aged
    /// into forgetting. `None` until at least one such item has been checked.
    pub fn forgotten_items_deficit(&self) -> Option<f32> {
        fraction(&self.item_agings)
    }

    /// Record one settler's attempt to eat, and prune the trailing window.
    pub fn record_eat_attempt(&mut self, tick: u64, found_food: bool) {
        self.eat_attempts.push_back(DayEvent {
            tick,
            is_deficit: !found_food,
        });
        self.prune(tick);
    }

    /// Record one marked item's forgetting check, and prune the trailing
    /// window.
    pub fn record_item_aged(&mut self, tick: u64, forgotten: bool) {
        self.item_agings.push_back(DayEvent {
            tick,
            is_deficit: forgotten,
        });
        self.prune(tick);
    }

    /// Whether a full night has elapsed and the two night-bound entries are
    /// due to roll over.
    pub fn dawn_due(&self, tick: u64) -> bool {
        tick >= self.next_dawn
    }

    /// Roll the two night-bound entries over at dawn, and schedule the next
    /// one. Fractions are clamped to `[0, 1]` — the caller counts settlers, so
    /// anything outside that range is a caller bug, not a value worth
    /// propagating.
    pub fn record_night(&mut self, cold_fraction: f32, unsheltered_fraction: f32, tick: u64) {
        self.cold_last_night = Some(cold_fraction.clamp(0.0, 1.0));
        self.unsheltered_last_night = Some(unsheltered_fraction.clamp(0.0, 1.0));
        while self.next_dawn <= tick {
            self.next_dawn += self.ticks_per_day;
        }
    }

    /// Prune the day-bound windows without recording a new event. Call this
    /// once a tick even when nothing happened, so an entry whose events have
    /// all aged out falls back to `None` instead of freezing at its last
    /// value.
    pub fn advance(&mut self, tick: u64) {
        self.prune(tick);
    }

    fn prune(&mut self, tick: u64) {
        let cutoff = tick.saturating_sub(self.ticks_per_day);
        while matches!(self.eat_attempts.front(), Some(e) if e.tick < cutoff) {
            self.eat_attempts.pop_front();
        }
        while matches!(self.item_agings.front(), Some(e) if e.tick < cutoff) {
            self.item_agings.pop_front();
        }
    }
}

fn fraction(events: &VecDeque<DayEvent>) -> Option<f32> {
    if events.is_empty() {
        return None;
    }
    let hits = events.iter().filter(|e| e.is_deficit).count();
    Some(hits as f32 / events.len() as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_four_entries_start_unknown() {
        let log = DeficitLog::new(100);
        assert_eq!(log.cold_last_night(), None);
        assert_eq!(log.unsheltered_last_night(), None);
        assert_eq!(log.hunger_deficit(), None);
        assert_eq!(log.forgotten_items_deficit(), None);
    }

    #[test]
    fn night_entries_stay_unknown_until_dawn_is_actually_due() {
        let mut log = DeficitLog::new(10);
        assert!(!log.dawn_due(5));
        assert!(!log.dawn_due(9));
        assert!(log.dawn_due(10));

        log.record_night(1.0, 1.0, 10);
        assert_eq!(log.cold_last_night(), Some(1.0));
        assert_eq!(log.unsheltered_last_night(), Some(1.0));
    }

    #[test]
    fn dawn_recurs_every_ticks_per_day() {
        let mut log = DeficitLog::new(10);
        log.record_night(1.0, 1.0, 10);
        assert!(!log.dawn_due(19));
        assert!(log.dawn_due(20));
        log.record_night(0.0, 0.0, 20);
        assert_eq!(log.cold_last_night(), Some(0.0));
        assert!(!log.dawn_due(29));
        assert!(log.dawn_due(30));
    }

    #[test]
    fn fractions_are_clamped_to_a_sane_range() {
        let mut log = DeficitLog::new(10);
        log.record_night(1.5, -0.5, 10);
        assert_eq!(log.cold_last_night(), Some(1.0));
        assert_eq!(log.unsheltered_last_night(), Some(0.0));
    }

    #[test]
    fn hunger_deficit_is_unknown_until_an_eat_attempt_happens() {
        let mut log = DeficitLog::new(100);
        assert_eq!(log.hunger_deficit(), None);
        log.record_eat_attempt(1, true);
        assert_eq!(log.hunger_deficit(), Some(0.0));
        log.record_eat_attempt(2, false);
        assert_eq!(log.hunger_deficit(), Some(0.5));
    }

    #[test]
    fn the_day_bound_window_rolls_over_continuously_as_ticks_pass() {
        let mut log = DeficitLog::new(100);
        log.record_eat_attempt(10, true); // found food: not a deficit
        log.record_eat_attempt(20, false); // no food: a deficit
        assert_eq!(log.hunger_deficit(), Some(0.5));

        // Tick 10's event is now more than one day (100 ticks) old; tick 20's
        // is not yet. Advancing with no new event still prunes it.
        log.advance(111);
        assert_eq!(log.hunger_deficit(), Some(1.0));

        // Once every event has aged out, the reading goes back to unknown —
        // a window with nothing in it is not "zero deficit".
        log.advance(121);
        assert_eq!(log.hunger_deficit(), None);
    }

    #[test]
    fn forgotten_items_deficit_behaves_like_hunger_but_independently() {
        let mut log = DeficitLog::new(50);
        log.record_item_aged(5, false);
        log.record_item_aged(5, true);
        assert_eq!(log.forgotten_items_deficit(), Some(0.5));
        // Recording hunger events must not disturb this window.
        log.record_eat_attempt(5, true);
        assert_eq!(log.forgotten_items_deficit(), Some(0.5));
    }
}
