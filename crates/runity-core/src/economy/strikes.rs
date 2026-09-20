//! [`StrikeBoard`]: the per-deficit-kind count of abandoned stakes.

use std::collections::HashMap;

use super::stake::{DeficitKind, Stake};
use super::tag::Tag;

/// How many abandonments of the same [`DeficitKind`] mark it un-treatable.
const STRIKES_UNTIL_UNTREATABLE: u8 = 3;

/// Returned by [`StrikeBoard::open_stake`] when a deficit has already
/// collected three abandonments and is refused a fourth attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Untreatable(pub DeficitKind);

/// Counts, per [`DeficitKind`], how many stakes raised against it have been
/// abandoned. Three abandonments of the same kind mark that deficit
/// un-treatable: [`StrikeBoard::open_stake`] refuses a fourth attempt.
///
/// "Until state changes" — this card's own words for how un-treatable
/// eventually lifts — is deliberately left open here: this board only
/// counts and gates, and a later card decides what event should call
/// [`StrikeBoard::reset`].
#[derive(Debug, Clone, Default)]
pub struct StrikeBoard {
    strikes: HashMap<DeficitKind, u8>,
}

impl StrikeBoard {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many abandonments have been recorded for `kind` so far.
    pub fn strikes(&self, kind: DeficitKind) -> u8 {
        self.strikes.get(&kind).copied().unwrap_or(0)
    }

    /// Whether `kind` still has room for another attempt.
    pub fn is_treatable(&self, kind: DeficitKind) -> bool {
        self.strikes(kind) < STRIKES_UNTIL_UNTREATABLE
    }

    /// Record one more abandonment for `kind`.
    pub fn record_abandonment(&mut self, kind: DeficitKind) {
        let count = self.strikes.entry(kind).or_insert(0);
        *count = count.saturating_add(1);
    }

    /// Clear `kind`'s count, making it treatable again regardless of how
    /// many strikes it had.
    pub fn reset(&mut self, kind: DeficitKind) {
        self.strikes.remove(&kind);
    }

    /// Open a fresh stake for `kind`, unless it has already collected three
    /// abandonments.
    pub fn open_stake(
        &self,
        kind: DeficitKind,
        tag: Tag,
        target_units: u32,
        ticks_per_day: u64,
        tick: u64,
    ) -> Result<Stake, Untreatable> {
        if !self.is_treatable(kind) {
            return Err(Untreatable(kind));
        }
        Ok(Stake::new(kind, tag, target_units, ticks_per_day, tick))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_board_treats_every_kind() {
        let board = StrikeBoard::new();
        assert!(board.is_treatable(DeficitKind::Warmth));
        assert_eq!(board.strikes(DeficitKind::Warmth), 0);
    }

    #[test]
    fn three_abandonments_of_the_same_kind_mark_it_untreatable() {
        let mut board = StrikeBoard::new();
        board.record_abandonment(DeficitKind::Warmth);
        board.record_abandonment(DeficitKind::Warmth);
        assert!(
            board.is_treatable(DeficitKind::Warmth),
            "two strikes is not three"
        );
        board.record_abandonment(DeficitKind::Warmth);
        assert!(!board.is_treatable(DeficitKind::Warmth));
    }

    #[test]
    fn other_kinds_are_unaffected_by_one_kind_s_strikes() {
        let mut board = StrikeBoard::new();
        for _ in 0..3 {
            board.record_abandonment(DeficitKind::Warmth);
        }
        assert!(!board.is_treatable(DeficitKind::Warmth));
        assert!(board.is_treatable(DeficitKind::Shelter));
    }

    #[test]
    fn a_fourth_attempt_to_open_a_stake_is_rejected_until_state_changes() {
        let mut board = StrikeBoard::new();
        for _ in 0..3 {
            board.record_abandonment(DeficitKind::Storage);
        }
        assert_eq!(
            board.open_stake(DeficitKind::Storage, Tag::Hard, 10, 100, 0),
            Err(Untreatable(DeficitKind::Storage)),
        );

        board.reset(DeficitKind::Storage);
        assert!(board
            .open_stake(DeficitKind::Storage, Tag::Hard, 10, 100, 0)
            .is_ok());
    }
}
