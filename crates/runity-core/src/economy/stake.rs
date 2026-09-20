//! [`Stake`]: the one assignment a settlement can have for a single deficit.

use std::collections::BTreeMap;

use crate::world::Entity;

use super::tag::Tag;

/// One of the four settlement-wide shortages a stake can be raised against,
/// per `12-minds.md` §3 — a deficit no single settler's reactive mind sees,
/// because it is about the settlement as a whole rather than one settler's
/// own needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeficitKind {
    /// Кров — a hut.
    Shelter,
    /// Тепло — a hearth beyond the starting one.
    Warmth,
    /// Хранение — a storehouse.
    Storage,
    /// Сушка — a drying rack.
    Drying,
}

impl DeficitKind {
    /// All four kinds, in the order everything that indexes by kind uses.
    pub const ALL: [DeficitKind; 4] = [
        DeficitKind::Shelter,
        DeficitKind::Warmth,
        DeficitKind::Storage,
        DeficitKind::Drying,
    ];

    /// This kind's position in [`DeficitKind::ALL`], for anything that keeps
    /// a four-element array indexed by kind rather than a map of four
    /// entries.
    pub fn slot(self) -> usize {
        match self {
            DeficitKind::Shelter => 0,
            DeficitKind::Warmth => 1,
            DeficitKind::Storage => 2,
            DeficitKind::Drying => 3,
        }
    }
}

/// A stake's lifecycle. Only [`Stake::abandon`] leaves [`Active`](Self::Active)
/// for good; [`Frozen`](Self::Frozen) is round-trippable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Active,
    Frozen { since: u64 },
    Abandoned,
}

/// How many full sim-days a stake may go with zero deposits before it is
/// pulled, per this card's brief.
const ABANDON_AFTER_DAYS: u64 = 2;

/// What each contributor gets back when a stake is abandoned: their own
/// deposited units, in deposit order. Refunding a stake that was never spent
/// on anything is share math (`05-economy.md` §3.4) in its degenerate
/// case — nothing has left the total but what a contributor put in, so their
/// share of the total is exactly their own deposit.
pub type Refund = Vec<(Entity, u32)>;

/// The one assignment a settlement can have for a single deficit —
/// `05-economy.md` §3's share economy, pared to what a stake needs and
/// nothing else.
///
/// A stake names a property and an amount to reach — "hard, 30 units" —
/// never a specific material, exactly like a recipe step
/// (`05-economy.md` §6.1): what actually gets carried to it is the
/// depositor's business. Deposits accumulate per contributor
/// ([`Stake::deposit`]) and the running total decides each contributor's
/// share ([`Stake::share_of`]) by the same irreducible pair as
/// `05-economy.md` §3.2 — `units contributed / units total` — recomputed
/// from the current total every time, never frozen at the moment of
/// depositing.
///
/// A stake with no deposits for two full sim-days is abandoned
/// automatically ([`Stake::advance`]), refunding every contributor by share.
/// [`Stake::freeze`] and [`Stake::thaw`] pause and resume that clock without
/// disturbing deposits or shares — the ledger simply stops ageing while
/// frozen. Deciding *when* to open a stake, for which deficit, is a later
/// card's planner; this type only keeps the assignment once one exists.
#[derive(Debug, Clone, PartialEq)]
pub struct Stake {
    kind: DeficitKind,
    tag: Tag,
    target_units: u32,
    deposits: BTreeMap<Entity, u32>,
    total_units: u32,
    state: State,
    ticks_per_day: u64,
    /// The tick at which the idle clock now reads zero — either creation, or
    /// the tick of the most recent deposit, shifted forward by however long
    /// the stake has spent frozen since.
    idle_since: u64,
}

impl Stake {
    /// A fresh, active stake for `kind`, asking for `target_units` of
    /// `tag`, planted at `tick`.
    pub fn new(
        kind: DeficitKind,
        tag: Tag,
        target_units: u32,
        ticks_per_day: u64,
        tick: u64,
    ) -> Self {
        assert!(
            ticks_per_day > 0,
            "a sim-day must be at least one tick long"
        );
        assert!(target_units > 0, "a stake must ask for something");
        Self {
            kind,
            tag,
            target_units,
            deposits: BTreeMap::new(),
            total_units: 0,
            state: State::Active,
            ticks_per_day,
            idle_since: tick,
        }
    }

    /// Which deficit this stake was raised against.
    pub fn kind(&self) -> DeficitKind {
        self.kind
    }

    /// The property this stake asks for.
    pub fn tag(&self) -> Tag {
        self.tag
    }

    /// The amount of [`Stake::tag`] this stake asks for.
    pub fn target_units(&self) -> u32 {
        self.target_units
    }

    /// The sum of every contributor's deposits so far.
    pub fn total_units(&self) -> u32 {
        self.total_units
    }

    pub fn is_frozen(&self) -> bool {
        matches!(self.state, State::Frozen { .. })
    }

    pub fn is_abandoned(&self) -> bool {
        matches!(self.state, State::Abandoned)
    }

    /// `contributor`'s share, as the irreducible pair `05-economy.md` §3.2
    /// describes: their own units contributed, and the current total. `None`
    /// if they have never deposited into this stake.
    pub fn share_of(&self, contributor: Entity) -> Option<(u32, u32)> {
        let units = *self.deposits.get(&contributor)?;
        Some((units, self.total_units))
    }

    /// Deposit `units` of this stake's material on behalf of `contributor`,
    /// adding to any deposit they already made, and reset the idle clock —
    /// a stake that just received material has, by definition, not gone
    /// quiet. Panics on an abandoned stake: nothing is left to deposit into.
    pub fn deposit(&mut self, contributor: Entity, units: u32, tick: u64) {
        assert!(
            !self.is_abandoned(),
            "an abandoned stake takes no more deposits"
        );
        *self.deposits.entry(contributor).or_insert(0) += units;
        self.total_units += units;
        if !self.is_frozen() {
            self.idle_since = tick;
        }
    }

    /// Pause the abandonment clock. Deposits and shares are untouched; a
    /// frozen stake simply stops ageing until [`Stake::thaw`]. No-op on an
    /// already-frozen or abandoned stake.
    pub fn freeze(&mut self, tick: u64) {
        if matches!(self.state, State::Active) {
            self.state = State::Frozen { since: tick };
        }
    }

    /// Resume the abandonment clock exactly where [`Stake::freeze`] paused
    /// it: the ticks spent frozen do not count as idle time. No-op if the
    /// stake was not frozen.
    pub fn thaw(&mut self, tick: u64) {
        if let State::Frozen { since } = self.state {
            self.idle_since += tick.saturating_sub(since);
            self.state = State::Active;
        }
    }

    /// How many ticks this stake has gone without a deposit, as of `tick`.
    /// Frozen time never counts, whether the stake is still frozen (the
    /// clock is fixed at the tick it was paused) or has since thawed (that
    /// span was folded out of [`Stake::deposit`]/[`Stake::thaw`]'s
    /// bookkeeping already).
    fn idle_ticks(&self, tick: u64) -> u64 {
        match self.state {
            State::Frozen { since } => since.saturating_sub(self.idle_since),
            _ => tick.saturating_sub(self.idle_since),
        }
    }

    /// Let time pass to `tick`. If the stake is active and has gone two full
    /// sim-days without a deposit, it is pulled: every contributor is
    /// refunded their own deposit and the stake becomes
    /// [`Stake::is_abandoned`]. Returns the refund when that happens, and
    /// `None` otherwise — including while frozen, no matter how much time
    /// has passed.
    pub fn advance(&mut self, tick: u64) -> Option<Refund> {
        if !matches!(self.state, State::Active) {
            return None;
        }
        if self.idle_ticks(tick) >= ABANDON_AFTER_DAYS * self.ticks_per_day {
            return Some(self.abandon());
        }
        None
    }

    fn abandon(&mut self) -> Refund {
        self.state = State::Abandoned;
        let refund = self
            .deposits
            .iter()
            .map(|(&who, &units)| (who, units))
            .collect();
        self.deposits.clear();
        self.total_units = 0;
        refund
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    const DAY: u64 = 100;

    fn stake(tick: u64) -> Stake {
        Stake::new(DeficitKind::Shelter, Tag::Hard, 30, DAY, tick)
    }

    #[test]
    fn every_kind_has_its_own_slot_and_all_lists_them_in_slot_order() {
        for (expected, kind) in DeficitKind::ALL.into_iter().enumerate() {
            assert_eq!(kind.slot(), expected);
        }
    }

    #[test]
    fn repeated_partial_deposits_accumulate_the_total_and_each_share() {
        let mut world = World::new();
        let onega = world.spawn();
        let snovid = world.spawn();
        let zhdan = world.spawn();
        let goluba = world.spawn();

        // The barn example from 05-economy.md §3.2: 10 units needed, then an
        // extension adds 5 more, all from one contributor.
        let mut stake = Stake::new(DeficitKind::Storage, Tag::Hard, 10, DAY, 0);
        stake.deposit(onega, 3, 0);
        stake.deposit(snovid, 4, 0);
        stake.deposit(zhdan, 2, 0);
        stake.deposit(goluba, 1, 0);

        assert_eq!(stake.total_units(), 10);
        assert_eq!(stake.share_of(onega), Some((3, 10)));
        assert_eq!(stake.share_of(snovid), Some((4, 10)));
        assert_eq!(stake.share_of(zhdan), Some((2, 10)));
        assert_eq!(stake.share_of(goluba), Some((1, 10)));

        // Extension: five more units, all from Zhdan, deposited in two
        // partial calls.
        stake.deposit(zhdan, 3, 1);
        stake.deposit(zhdan, 2, 1);

        assert_eq!(stake.total_units(), 15);
        assert_eq!(stake.share_of(onega), Some((3, 15)), "Onega loses nothing");
        assert_eq!(
            stake.share_of(zhdan),
            Some((7, 15)),
            "but her voice weakens"
        );
    }

    #[test]
    fn a_contributor_with_no_deposit_has_no_share() {
        let mut world = World::new();
        let onega = world.spawn();
        let sitting_out = world.spawn();
        let mut stake = stake(0);
        stake.deposit(onega, 5, 0);
        assert_eq!(stake.share_of(sitting_out), None);
    }

    #[test]
    fn freezing_stops_the_clock_and_leaves_deposits_and_shares_untouched() {
        let mut world = World::new();
        let onega = world.spawn();
        let mut stake = stake(0);
        stake.deposit(onega, 5, 0);

        stake.freeze(10);
        // Way more than two days pass while frozen: still not abandoned.
        assert_eq!(stake.advance(10 + 10 * DAY), None);
        assert!(!stake.is_abandoned());
        assert_eq!(stake.total_units(), 5);
        assert_eq!(stake.share_of(onega), Some((5, 5)));
    }

    #[test]
    fn thawing_resumes_the_clock_from_where_it_paused_not_from_zero() {
        let mut world = World::new();
        let onega = world.spawn();
        let mut stake = stake(0);
        stake.deposit(onega, 5, 0);

        // 30 idle ticks pass, then freeze for a long stretch, then thaw.
        stake.freeze(30);
        stake.thaw(30 + 500);

        // Only 30 ticks of the idle clock had elapsed before the freeze, so
        // it should take (2*DAY - 30) more active ticks to abandon, not
        // 2*DAY more from the thaw point.
        let almost_there = 30 + 500 + (2 * DAY - 30) - 1;
        assert_eq!(stake.advance(almost_there), None);
        let there = 30 + 500 + (2 * DAY - 30);
        assert!(stake.advance(there).is_some());
    }

    #[test]
    fn two_full_idle_days_abandon_a_stake_with_no_deposits_and_refund_nothing() {
        let mut stake = stake(0);
        let refund = stake
            .advance(2 * DAY)
            .expect("two idle days should abandon");
        assert_eq!(refund, Vec::new());
        assert!(stake.is_abandoned());
        assert_eq!(stake.total_units(), 0);
    }

    #[test]
    fn two_full_idle_days_abandon_a_stake_with_partial_deposits_and_refund_by_share() {
        let mut world = World::new();
        let onega = world.spawn();
        let snovid = world.spawn();
        let mut stake = stake(0);
        stake.deposit(onega, 3, 0);
        stake.deposit(snovid, 4, 5);

        assert_eq!(
            stake.advance(5 + 2 * DAY - 1),
            None,
            "not quite two days yet"
        );
        let mut refund = stake
            .advance(5 + 2 * DAY)
            .expect("two idle days should abandon");
        refund.sort();
        let mut expected = vec![(onega, 3), (snovid, 4)];
        expected.sort();
        assert_eq!(refund, expected);
        assert!(stake.is_abandoned());
        assert_eq!(stake.total_units(), 0);
        assert_eq!(
            stake.share_of(onega),
            None,
            "the ledger is cleared on abandonment"
        );
    }

    #[test]
    fn a_deposit_resets_the_idle_clock() {
        let mut world = World::new();
        let onega = world.spawn();
        let mut stake = stake(0);
        stake.deposit(onega, 1, 2 * DAY - 1);
        // Without the deposit this tick would have abandoned the stake.
        assert_eq!(stake.advance(2 * DAY), None);
        assert_eq!(stake.advance(2 * DAY - 1 + 2 * DAY), Some(vec![(onega, 1)]));
    }

    #[test]
    #[should_panic(expected = "abandoned stake")]
    fn depositing_into_an_abandoned_stake_panics() {
        let mut world = World::new();
        let onega = world.spawn();
        let mut stake = stake(0);
        stake.advance(2 * DAY);
        stake.deposit(onega, 1, 2 * DAY);
    }
}
