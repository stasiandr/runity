//! A whole settlement of personal minds, run headless.
//!
//! [`Mind`](super::Mind) on its own is a pure decision; what a tuning is
//! actually judged by is a settlement of them left to run for a few sim-days.
//! [`Settlement`] is that run: a [`crate::Game`] that does nothing but decide,
//! driven by [`crate::headless::simulate`], reporting how often a settler
//! changed their mind ([`SwitchReport`]) rather than what they decided.
//!
//! The distribution is the point. "Eleven switches on day three" is a number
//! that means nothing and breaks on the next tuning change; a median, a p90
//! and a worst case say whether settlers are getting on with things or
//! dithering, and stay meaningful when the numbers move.

use std::cell::RefCell;
use std::fmt;
use std::io;
use std::rc::Rc;

use runity_math::Vec3;

use crate::app::{Engine, Game};
use crate::economy::Tag;
use crate::needs::Needs;
use crate::world::{Entity, World};

use super::decide::Mind;
use super::score::{Activity, Situation};
use super::tuning::{hour_of_day, MindTuning};
use super::work::{AtHand, WorkCandidate};

/// One settler as a run is set up: where they stand, what they are good at,
/// how they feel and what they are carrying.
#[derive(Debug, Clone)]
pub struct SettlerSpec {
    /// Where they stand. Nobody walks in this harness — distance to work is
    /// a fixed fact about a settler, which is enough to make two settlers
    /// score the same stake differently.
    pub position: Vec3,
    /// Skill at every piece of work, `[0, 1]`.
    pub skill: f32,
    /// Disposition towards every piece of work, `[0, 1]`.
    pub disposition: f32,
    /// Their three needs, rates included.
    pub needs: Needs,
    /// What they are carrying and can make in one action.
    pub at_hand: AtHand,
}

impl SettlerSpec {
    /// A settler standing at `position` with default needs, nothing in hand,
    /// and no skill worth mentioning.
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            skill: 0.0,
            disposition: 1.0,
            needs: Needs::default(),
            at_hand: AtHand::default(),
        }
    }

    /// The same settler, this good at work and this drawn to it.
    pub fn able(mut self, skill: f32, disposition: f32) -> Self {
        self.skill = skill;
        self.disposition = disposition;
        self
    }

    /// The same settler, with these needs.
    pub fn feeling(mut self, needs: Needs) -> Self {
        self.needs = needs;
        self
    }

    /// The same settler, carrying this.
    pub fn carrying(mut self, at_hand: AtHand) -> Self {
        self.at_hand = at_hand;
        self
    }
}

/// A standing piece of common work: a stake, or a chore like firewood to the
/// hearth. Its severity and the property it asks for are the same for
/// everyone; skill, disposition and distance are not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chore {
    /// Where it is.
    pub position: Vec3,
    /// How badly the settlement wants it, `[0, 1]`.
    pub severity: f32,
    /// The property it asks for, if any.
    pub wants: Option<Tag>,
}

impl Chore {
    /// A chore at `position` of this severity, asking for nothing but hands.
    pub fn new(position: Vec3, severity: f32) -> Self {
        Self {
            position,
            severity,
            wants: None,
        }
    }

    /// The same chore, asking for `tag`.
    pub fn wanting(mut self, tag: Tag) -> Self {
        self.wants = Some(tag);
        self
    }
}

/// How many times each settler changed their mind on each sim-day of a run.
///
/// One sample per settler per day, which is the unit the number is readable
/// in: a settlement of twelve over five days is sixty samples, and a p90 of
/// forty says something a total of two thousand does not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SwitchReport {
    samples: Vec<u32>,
}

impl SwitchReport {
    /// Every sample, sorted ascending.
    pub fn samples(&self) -> &[u32] {
        &self.samples
    }

    /// How many settler-days were recorded.
    pub fn count(&self) -> usize {
        self.samples.len()
    }

    /// Switches on the median settler-day, or `0` for an empty run.
    pub fn median(&self) -> u32 {
        self.percentile(0.5)
    }

    /// Switches on the 90th-percentile settler-day.
    pub fn p90(&self) -> u32 {
        self.percentile(0.9)
    }

    /// The worst settler-day of the run.
    pub fn max(&self) -> u32 {
        self.samples.last().copied().unwrap_or(0)
    }

    /// Nearest-rank percentile: no interpolation, so every value reported is
    /// a day that actually happened.
    pub fn percentile(&self, fraction: f32) -> u32 {
        if self.samples.is_empty() {
            return 0;
        }
        let rank = (fraction.clamp(0.0, 1.0) * self.samples.len() as f32).ceil() as usize;
        self.samples[rank.max(1) - 1]
    }
}

impl fmt::Display for SwitchReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} settler-days: median {}, p90 {}, max {}",
            self.count(),
            self.median(),
            self.p90(),
            self.max()
        )
    }
}

/// A handle on the switch counts a [`Settlement`] is filling in.
///
/// [`crate::headless::simulate`] takes the game by value, so the way to read
/// anything out of a run is to hold a handle to it from before the run
/// starts — the same shape as [`crate::headless::record`]'s recorder.
#[derive(Debug, Clone, Default)]
pub struct SwitchLog(Rc<RefCell<Vec<u32>>>);

impl SwitchLog {
    fn push(&self, counts: impl IntoIterator<Item = u32>) {
        self.0.borrow_mut().extend(counts);
    }

    /// The run so far, as a distribution.
    pub fn report(&self) -> SwitchReport {
        let mut samples = self.0.borrow().clone();
        samples.sort_unstable();
        SwitchReport { samples }
    }
}

/// One settler, mid-run.
#[derive(Debug)]
struct Settler {
    entity: Entity,
    spec: SettlerSpec,
    mind: Mind,
    switches_today: u32,
}

/// A settlement of minds deciding for themselves, tick by tick, with no
/// window and no rendering.
///
/// Everything tunable arrives as the one [`MindTuning`] handed to
/// [`Settlement::new`]; nothing in the run reaches for a constant of its own.
///
/// ```
/// use runity_core::mind::{MindTuning, Chore, Settlement, SettlerSpec};
/// use runity_math::Vec3;
///
/// let mut settlement = Settlement::new(MindTuning::default(), 480, 1.0 / 20.0);
/// settlement.add_chore(Chore::new(Vec3::ZERO, 0.7));
/// for i in 0..12 {
///     settlement.add_settler(SettlerSpec::new(Vec3::new(i as f32 * 2.0, 0.0, 0.0)));
/// }
///
/// let report = settlement.run_days(4);
/// assert_eq!(report.count(), 12 * 4, "one sample per settler per day");
/// assert!(report.p90() >= report.median());
/// ```
pub struct Settlement {
    world: World,
    tuning: MindTuning,
    ticks_per_day: u64,
    seconds_per_tick: f32,
    settlers: Vec<Settler>,
    chores: Vec<(Entity, Chore)>,
    switches: SwitchLog,
}

impl Settlement {
    /// An empty settlement whose sim-day is `ticks_per_day` fixed steps long,
    /// each step worth `seconds_per_tick` of simulated time.
    pub fn new(tuning: MindTuning, ticks_per_day: u64, seconds_per_tick: f32) -> Self {
        assert!(
            ticks_per_day > 0,
            "a sim-day must be at least one tick long"
        );
        Self {
            world: World::new(),
            tuning,
            ticks_per_day,
            seconds_per_tick,
            settlers: Vec::new(),
            chores: Vec::new(),
            switches: SwitchLog::default(),
        }
    }

    /// Add a piece of standing common work and return its entity — the
    /// [`WorkCandidate::target`] the settlers will see.
    pub fn add_chore(&mut self, chore: Chore) -> Entity {
        let entity = self.world.spawn();
        self.chores.push((entity, chore));
        entity
    }

    /// Add a settler and return their entity. Their [`Entity::index`] is what
    /// staggers their recompute, so the order settlers are added in is part
    /// of the run.
    pub fn add_settler(&mut self, spec: SettlerSpec) -> Entity {
        let entity = self.world.spawn();
        self.settlers.push(Settler {
            entity,
            spec,
            mind: Mind::new(),
            switches_today: 0,
        });
        entity
    }

    /// The switch counts this run is filling in. Take a copy before handing
    /// the settlement to [`crate::headless::simulate`].
    pub fn switches(&self) -> SwitchLog {
        self.switches.clone()
    }

    /// What `entity` is doing right now, if it is a settler here.
    pub fn activity_of(&self, entity: Entity) -> Option<Activity> {
        self.settlers
            .iter()
            .find(|settler| settler.entity == entity)
            .and_then(|settler| settler.mind.current())
    }

    /// Run `days` sim-days through [`crate::headless::simulate`] and report
    /// the distribution of daily switches.
    pub fn run_days(self, days: u64) -> SwitchReport {
        let switches = self.switches();
        let ticks = days * self.ticks_per_day;
        crate::headless::simulate(self, ticks);
        switches.report()
    }

    /// Every piece of work as `settler` sees it.
    fn work_for(&self, settler: &Settler) -> Vec<WorkCandidate> {
        self.chores
            .iter()
            .map(|(entity, chore)| {
                let distance = (chore.position - settler.spec.position).length();
                let mut candidate = WorkCandidate::new(*entity, chore.severity, distance)
                    .by(settler.spec.skill, settler.spec.disposition);
                candidate.wants = chore.wants;
                candidate
            })
            .collect()
    }
}

impl Game for Settlement {
    fn start(&mut self, engine: &mut Engine) -> io::Result<()> {
        engine.time.fixed_delta = self.seconds_per_tick;
        Ok(())
    }

    fn fixed_update(&mut self, engine: &mut Engine) {
        let tick = engine.time.elapsed_ticks();
        let dt = self.seconds_per_tick;
        let light = self
            .tuning
            .light_at_hour(hour_of_day(tick, self.ticks_per_day));

        for index in 0..self.settlers.len() {
            let work = self.work_for(&self.settlers[index]);
            let settler = &mut self.settlers[index];
            settler.spec.needs.decay(dt);

            let before = settler.mind.current();
            let situation = Situation {
                settler: settler.entity,
                tick,
                needs: settler.spec.needs,
                light,
                work: &work,
                at_hand: &settler.spec.at_hand,
            };
            let activity = settler.mind.update(&situation, &self.tuning);
            if before != Some(activity) {
                settler.switches_today += 1;
            }

            // Doing the thing. Only the two activities that can finish do
            // anything here; work is a standing chore that never runs out.
            match activity {
                Activity::Need(kind) => {
                    settler.spec.needs.relieve(kind, dt);
                    if settler.spec.needs.level(kind) >= 1.0 {
                        settler.mind.complete();
                    }
                }
                Activity::Craft { tag, .. } => {
                    settler.spec.at_hand.take(tag);
                    settler.mind.complete();
                }
                Activity::Work { .. } | Activity::OwnBusiness => {}
            }
        }

        if tick % self.ticks_per_day == 0 {
            self.switches
                .push(self.settlers.iter().map(|s| s.switches_today));
            for settler in &mut self.settlers {
                settler.switches_today = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind::work::OneStep;
    use crate::needs::NeedsRates;

    const TICKS_PER_DAY: u64 = 480;
    const SECONDS_PER_TICK: f32 = 1.0 / 20.0;

    /// Needs that fall fast enough to matter inside a four-day run: a full
    /// sim-day here is 24 seconds of simulated time, so the default rates
    /// (which are written for a day of real minutes) would never move.
    fn hurried() -> Needs {
        Needs::new(NeedsRates {
            hunger_decay: 1.0 / 40.0,
            warmth_decay: 1.0 / 30.0,
            rest_decay: 1.0 / 60.0,
            warm_gain: 1.0 / 4.0,
            sleep_gain: 1.0 / 6.0,
        })
    }

    fn settlement(tuning: MindTuning) -> Settlement {
        let mut settlement = Settlement::new(tuning, TICKS_PER_DAY, SECONDS_PER_TICK);
        settlement.add_chore(Chore::new(Vec3::new(4.0, 0.0, 0.0), 0.8));
        settlement.add_chore(Chore::new(Vec3::new(-30.0, 0.0, 12.0), 0.6));
        for i in 0..12 {
            settlement.add_settler(
                SettlerSpec::new(Vec3::new(i as f32 * 3.0, 0.0, (i % 4) as f32 * 5.0))
                    .able(i as f32 / 12.0, 0.6 + (i % 3) as f32 * 0.2)
                    .feeling(hurried()),
            );
        }
        settlement
    }

    #[test]
    fn a_recorded_run_reports_a_distribution_not_a_number() {
        let report = settlement(MindTuning::default()).run_days(4);
        assert_eq!(report.count(), 12 * 4);
        assert!(report.median() <= report.p90());
        assert!(report.p90() <= report.max());
        // Settlers get on with things: a day is 480 ticks, and nobody should
        // be spending a meaningful fraction of them changing their mind.
        assert!(report.max() < 50, "someone dithered all day: {report}");
    }

    #[test]
    fn the_same_run_twice_reports_the_same_numbers() {
        let first = settlement(MindTuning::default()).run_days(4);
        let second = settlement(MindTuning::default()).run_days(4);
        assert_eq!(first, second, "no RNG anywhere in the mind");
    }

    #[test]
    fn a_shorter_minimum_hold_makes_a_settlement_dither_more() {
        // Both reconsider every tick, so the difference is hysteresis alone.
        let every_tick = MindTuning {
            recompute_period: 1,
            ..MindTuning::default()
        };
        let patient = settlement(every_tick).run_days(4);
        let twitchy = settlement(MindTuning {
            min_activity_ticks: 1,
            stickiness_bonus: 0.0,
            ..every_tick
        })
        .run_days(4);
        assert!(
            twitchy.p90() > patient.p90(),
            "hysteresis is supposed to cost switches: {patient} vs {twitchy}"
        );
    }

    #[test]
    fn the_log_can_be_read_while_the_run_is_still_being_set_up() {
        let settlement = settlement(MindTuning::default());
        let log = settlement.switches();
        assert_eq!(log.report().count(), 0);
        settlement.run_days(2);
        assert_eq!(log.report().count(), 12 * 2);
    }

    #[test]
    fn crafting_puts_the_material_in_hand_and_then_the_stake_is_plain_work() {
        let mut settlement =
            Settlement::new(MindTuning::default(), TICKS_PER_DAY, SECONDS_PER_TICK);
        let stake = settlement.add_chore(Chore::new(Vec3::ZERO, 1.0).wanting(Tag::Sharp));
        let settler = settlement.add_settler(SettlerSpec::new(Vec3::ZERO).able(1.0, 1.0).carrying(
            AtHand::new([Tag::Hard], [OneStep::new(Tag::Hard, Tag::Sharp)]),
        ));

        let mut engine = Engine::new(1, 1);
        settlement.start(&mut engine).unwrap();

        engine.time.advance_tick();
        settlement.fixed_update(&mut engine);
        assert_eq!(
            settlement.activity_of(settler),
            None,
            "the craft was taken up and finished inside one tick"
        );

        engine.time.advance_tick();
        settlement.fixed_update(&mut engine);
        assert_eq!(
            settlement.activity_of(settler),
            Some(Activity::Work { target: stake }),
            "with something sharp in hand the stake is work at last"
        );
    }

    #[test]
    fn percentiles_are_nearest_rank_and_an_empty_run_is_all_zeroes() {
        let empty = SwitchReport::default();
        assert_eq!((empty.median(), empty.p90(), empty.max()), (0, 0, 0));

        let log = SwitchLog::default();
        log.push([5, 1, 4, 2, 3, 9, 7, 6, 8, 0]);
        let report = log.report();
        assert_eq!(report.samples(), &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(report.median(), 4, "the fifth of ten, no interpolation");
        assert_eq!(report.p90(), 8);
        assert_eq!(report.max(), 9);
        assert_eq!(report.percentile(0.0), 0);
        assert_eq!(report.percentile(1.0), 9);
    }
}
