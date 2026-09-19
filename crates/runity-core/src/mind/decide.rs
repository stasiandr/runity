//! [`Mind`]: the hysteresis of `12-minds.md` §3.3 and the recompute triggers
//! of §3.7, wrapped around the pure scoring of [`super::score`].

use crate::world::Entity;

use super::score::{best, candidates, score_of, Activity, Scored, Situation};
use super::tuning::MindTuning;

/// Whether `settler` is due one of §3.7's staggered reconsiderations at
/// `tick`: `(entity index + tick) % recompute_period == 0`.
///
/// A counter and not a die roll, and that is the point. Spreading a thousand
/// minds over the period by hashing the tick would work too, right up until a
/// replay had to reproduce it; an entity's index and the tick are both
/// already part of the state a run is defined by.
///
/// ```
/// use runity_core::mind::{staggered_recompute, MindTuning};
/// use runity_core::World;
///
/// let mut world = World::new();
/// let first = world.spawn();
/// let second = world.spawn();
/// let tuning = MindTuning::default();
///
/// // Neighbouring settlers think on neighbouring ticks, never the same one.
/// assert!(staggered_recompute(first, 60, &tuning));
/// assert!(staggered_recompute(second, 59, &tuning));
/// assert!(!staggered_recompute(second, 60, &tuning));
/// ```
pub fn staggered_recompute(settler: Entity, tick: u64, tuning: &MindTuning) -> bool {
    if tuning.recompute_period == 0 {
        return true;
    }
    (settler.index() as u64 + tick) % tuning.recompute_period == 0
}

/// Why a settler reconsidered on a given tick — the four triggers of §3.7,
/// for anyone watching a run rather than a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The settler had nothing on, or [`Mind::complete`] said the last thing
    /// finished.
    Completion,
    /// A need crossed [`MindTuning::need_floor`], in either direction.
    NeedFloor,
    /// What the settler was working on is no longer in front of them.
    TargetGone,
    /// The staggered every-`recompute_period` sweep — see
    /// [`staggered_recompute`].
    Staggered,
}

/// One settler's mind: what they are doing, since when, and enough to know
/// when to think about it again.
///
/// This holds no plan — `02-settlers.md` is emphatic that nobody has one — but
/// it does hold hysteresis, which is a different thing: the current activity
/// gets [`MindTuning::stickiness_bonus`] added to its score and cannot be
/// dropped for [`MindTuning::min_activity_ticks`] at all. Without it, two
/// candidates whose scores drift past each other make a settler stand between
/// them vibrating.
///
/// Exactly two things pierce that hold: a need below
/// [`MindTuning::need_floor`], and the target of the work disappearing. A
/// third case is not a piercing at all — [`Mind::complete`] says the activity
/// is over, so there is nothing left to hold on to.
///
/// ```
/// use runity_core::mind::{Activity, AtHand, Mind, MindTuning, Situation, WorkCandidate};
/// use runity_core::needs::{NeedKind, Needs};
/// use runity_core::World;
///
/// let mut world = World::new();
/// let settler = world.spawn();
/// let stake = world.spawn();
/// let tuning = MindTuning::default();
///
/// let work = [WorkCandidate::new(stake, 1.0, 0.0).by(1.0, 1.0)];
/// let hand = AtHand::default();
/// let mut needs = Needs::default();
/// let mut mind = Mind::new();
///
/// let mut at = |tick, needs| Situation {
///     settler,
///     tick,
///     needs,
///     light: 1.0,
///     work: &work,
///     at_hand: &hand,
/// };
///
/// // Broad daylight, a stake underfoot: the settler works.
/// assert_eq!(mind.update(&at(0, needs), &tuning), Activity::Work { target: stake });
///
/// // Then hunger falls through the floor, mid-stake, well inside the
/// // minimum hold. Nothing about that hold survives it.
/// needs.hunger = 0.1;
/// assert_eq!(
///     mind.update(&at(1, needs), &tuning),
///     Activity::Need(NeedKind::Hunger),
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Mind {
    current: Option<Activity>,
    since: u64,
    below_floor: bool,
    last_trigger: Option<Trigger>,
}

impl Mind {
    /// A settler who has not decided anything yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// What the settler is doing, or `None` before the first decision and
    /// after [`Mind::complete`].
    pub fn current(&self) -> Option<Activity> {
        self.current
    }

    /// The tick [`Mind::current`] was taken up on.
    pub fn since(&self) -> u64 {
        self.since
    }

    /// How long the settler has been on their current activity, as of `tick`.
    pub fn held_ticks(&self, tick: u64) -> u64 {
        tick.saturating_sub(self.since)
    }

    /// Why the last [`Mind::update`] reconsidered, or `None` if it simply
    /// carried on.
    pub fn last_trigger(&self) -> Option<Trigger> {
        self.last_trigger
    }

    /// The current activity is finished. The settler is left holding nothing,
    /// so the next [`Mind::update`] decides afresh — §3.7's first trigger.
    pub fn complete(&mut self) {
        self.current = None;
    }

    /// One tick of thought. Returns what the settler is doing when it ends,
    /// which on most ticks is exactly what they were doing when it began:
    /// [`Mind::update`] is cheap to call every tick precisely because it
    /// usually decides nothing.
    pub fn update(&mut self, situation: &Situation, tuning: &MindTuning) -> Activity {
        let breach = situation.floor_breach(tuning);
        let crossed = breach.is_some() != self.below_floor;
        self.below_floor = breach.is_some();

        // The hard floor. Not a candidate with a large score — an override,
        // taken before stickiness, the minimum hold, or the work list are
        // even looked at.
        if let Some(kind) = breach {
            self.last_trigger = Some(Trigger::NeedFloor);
            return self.settle_on(Activity::Need(kind), situation.tick);
        }

        let target_gone = self
            .current
            .and_then(Activity::target)
            .is_some_and(|target| !situation.work.iter().any(|work| work.target == target));

        let trigger = if self.current.is_none() {
            Some(Trigger::Completion)
        } else if crossed {
            Some(Trigger::NeedFloor)
        } else if target_gone {
            Some(Trigger::TargetGone)
        } else if staggered_recompute(situation.settler, situation.tick, tuning) {
            Some(Trigger::Staggered)
        } else {
            None
        };
        self.last_trigger = trigger;

        let Some(current) = self.current else {
            return self.decide(situation, tuning);
        };
        if trigger.is_none() {
            return current;
        }
        // Too soon to drop it. Work that has vanished is the exception: there
        // is nothing left to be faithful to.
        if !target_gone && self.held_ticks(situation.tick) < tuning.min_activity_ticks {
            return current;
        }
        self.decide(situation, tuning)
    }

    /// Score everything and take the best, with the current activity carrying
    /// [`MindTuning::stickiness_bonus`] and winning every tie.
    fn decide(&mut self, situation: &Situation, tuning: &MindTuning) -> Activity {
        let scored = candidates(situation, tuning);
        let mut chosen = best(&scored).expect("own business is always a candidate");
        if let Some(current) = self.current {
            if let Some(held) = score_of(&scored, current) {
                if held + tuning.stickiness_bonus >= chosen.score {
                    chosen = Scored {
                        activity: current,
                        score: held,
                    };
                }
            }
        }
        self.settle_on(chosen.activity, situation.tick)
    }

    /// Take up `activity`, restarting the hold clock only if it is actually a
    /// change of mind.
    fn settle_on(&mut self, activity: Activity, tick: u64) -> Activity {
        if self.current != Some(activity) {
            self.current = Some(activity);
            self.since = tick;
        }
        activity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::Tag;
    use crate::mind::work::{AtHand, OneStep, WorkCandidate};
    use crate::needs::{NeedKind, Needs, NeedsRates};
    use crate::world::World;

    fn still() -> Needs {
        Needs::new(NeedsRates {
            hunger_decay: 0.0,
            warmth_decay: 0.0,
            rest_decay: 0.0,
            ..NeedsRates::default()
        })
    }

    struct Bench {
        settler: Entity,
        stake: Entity,
        tuning: MindTuning,
        needs: Needs,
        hand: AtHand,
    }

    fn bench() -> (World, Bench) {
        let mut world = World::new();
        let settler = world.spawn();
        let stake = world.spawn();
        (
            world,
            Bench {
                settler,
                stake,
                tuning: MindTuning::default(),
                needs: still(),
                hand: AtHand::default(),
            },
        )
    }

    fn situation<'a>(
        bench: &'a Bench,
        tick: u64,
        light: f32,
        work: &'a [WorkCandidate],
    ) -> Situation<'a> {
        Situation {
            settler: bench.settler,
            tick,
            needs: bench.needs,
            light,
            work,
            at_hand: &bench.hand,
        }
    }

    #[test]
    fn a_need_below_the_floor_beats_the_best_work_there_is_mid_activity() {
        let (_world, mut bench) = bench();
        let work = [WorkCandidate::new(bench.stake, 1.0, 0.0).by(1.0, 1.0)];
        let mut mind = Mind::new();

        // A stake underfoot at noon: the very best score the formula gives.
        assert_eq!(
            mind.update(&situation(&bench, 0, 1.0, &work), &bench.tuning),
            Activity::Work {
                target: bench.stake
            }
        );

        // One tick later — deep inside min_activity_ticks, with the
        // stickiness bonus on top — warmth falls through the floor.
        bench.needs.warmth = 0.14;
        assert_eq!(
            mind.update(&situation(&bench, 1, 1.0, &work), &bench.tuning),
            Activity::Need(NeedKind::Warmth),
            "the hard floor overrides everything, stickiness included"
        );
        assert_eq!(mind.last_trigger(), Some(Trigger::NeedFloor));

        // And it keeps overriding for as long as the need is down there.
        for tick in 2..10 {
            assert_eq!(
                mind.update(&situation(&bench, tick, 1.0, &work), &bench.tuning),
                Activity::Need(NeedKind::Warmth)
            );
        }
    }

    #[test]
    fn a_settler_on_a_stake_drifts_to_own_business_as_the_light_goes() {
        let (_world, bench) = bench();
        // Modest work: severity 0.5, no skill, disposition 0.6. At noon it is
        // worth 0.15 * ... — comfortably more than own business; at night it
        // is not.
        let work = [WorkCandidate::new(bench.stake, 0.6, 5.0).by(0.2, 0.9)];
        let mut mind = Mind::new();

        // Walk a whole day of ticks from dawn, one hour at a time, and let
        // the light do the deciding. Nothing here mentions an hour.
        let ticks_per_day = 24 * 60;
        let mut switched_at = None;
        for tick in 0..ticks_per_day {
            let hour = crate::mind::hour_of_day(tick, ticks_per_day);
            let light = bench.tuning.light_at_hour(hour);
            let activity = mind.update(&situation(&bench, tick, light, &work), &bench.tuning);
            if switched_at.is_none() && activity == Activity::OwnBusiness {
                switched_at = Some(hour);
            }
        }

        let hour = switched_at.expect("the settler must give up on the stake at some point");
        assert!(
            (17.0..=23.0).contains(&hour),
            "the switch should land in the evening, not at {hour}"
        );
    }

    #[test]
    fn crafting_is_how_a_settler_without_the_material_joins_the_stake() {
        let (_world, mut bench) = bench();
        bench.hand = AtHand::new([Tag::Hard], [OneStep::new(Tag::Hard, Tag::Sharp)]);
        let work = [WorkCandidate::new(bench.stake, 1.0, 0.0)
            .wanting(Tag::Sharp)
            .by(1.0, 1.0)];
        let mut mind = Mind::new();

        assert_eq!(
            mind.update(&situation(&bench, 0, 1.0, &work), &bench.tuning),
            Activity::Craft {
                target: bench.stake,
                tag: Tag::Sharp
            },
            "half of a full-score stake still beats own business"
        );

        // Half of a *dim* stake does not, and then the settler does not
        // bother — "sometimes", not "always".
        let mut other = Mind::new();
        assert_eq!(
            other.update(&situation(&bench, 0, 0.25, &work), &bench.tuning),
            Activity::OwnBusiness
        );
    }

    #[test]
    fn stickiness_holds_a_settler_between_two_candidates_that_are_neck_and_neck() {
        let (mut world, bench) = bench();
        let other = world.spawn();
        // Two pieces of work a hair apart, both recomputed on every tick.
        let tuning = MindTuning {
            recompute_period: 1,
            min_activity_ticks: 40,
            ..bench.tuning
        };
        let mut mind = Mind::new();

        let held = WorkCandidate::new(bench.stake, 0.50, 0.0).by(1.0, 1.0);
        let rival = WorkCandidate::new(other, 0.52, 0.0).by(1.0, 1.0);

        // First tick: the settler takes the better of the two.
        let work = [rival, held];
        assert_eq!(
            mind.update(&situation(&bench, 0, 1.0, &work), &tuning),
            Activity::Work { target: other }
        );

        // Now the two swap places by a whisker, every tick.
        let swapped = [
            WorkCandidate::new(other, 0.50, 0.0).by(1.0, 1.0),
            WorkCandidate::new(bench.stake, 0.52, 0.0).by(1.0, 1.0),
        ];
        for tick in 1..tuning.min_activity_ticks {
            assert_eq!(
                mind.update(&situation(&bench, tick, 1.0, &swapped), &tuning),
                Activity::Work { target: other },
                "dropped at tick {tick}, before the minimum hold elapsed"
            );
        }

        // Past the hold the bonus still covers a two-hundredth of a point, so
        // the settler stays put even now: close is not better.
        assert_eq!(
            mind.update(
                &situation(&bench, tuning.min_activity_ticks, 1.0, &swapped),
                &tuning
            ),
            Activity::Work { target: other }
        );

        // A rival that is genuinely better does win.
        let clearly_better = [
            WorkCandidate::new(other, 0.20, 0.0).by(1.0, 1.0),
            WorkCandidate::new(bench.stake, 0.90, 0.0).by(1.0, 1.0),
        ];
        assert_eq!(
            mind.update(
                &situation(&bench, tuning.min_activity_ticks + 1, 1.0, &clearly_better),
                &tuning
            ),
            Activity::Work {
                target: bench.stake
            }
        );
    }

    #[test]
    fn work_that_disappears_pierces_the_minimum_hold() {
        let (_world, bench) = bench();
        let work = [WorkCandidate::new(bench.stake, 1.0, 0.0).by(1.0, 1.0)];
        let mut mind = Mind::new();
        mind.update(&situation(&bench, 0, 1.0, &work), &bench.tuning);

        // One tick in — nowhere near min_activity_ticks — the stake is gone.
        assert_eq!(
            mind.update(&situation(&bench, 1, 1.0, &[]), &bench.tuning),
            Activity::OwnBusiness
        );
        assert_eq!(mind.last_trigger(), Some(Trigger::TargetGone));
    }

    #[test]
    fn completion_leaves_nothing_to_hold_on_to() {
        let (_world, bench) = bench();
        let work = [WorkCandidate::new(bench.stake, 1.0, 0.0).by(1.0, 1.0)];
        let mut mind = Mind::new();
        mind.update(&situation(&bench, 0, 1.0, &work), &bench.tuning);
        assert!(mind.current().is_some());

        mind.complete();
        assert_eq!(mind.current(), None);
        // The next tick decides afresh, minimum hold or no minimum hold.
        assert_eq!(
            mind.update(&situation(&bench, 1, 1.0, &[]), &bench.tuning),
            Activity::OwnBusiness
        );
        assert_eq!(mind.last_trigger(), Some(Trigger::Completion));
    }

    #[test]
    fn between_triggers_a_settler_does_not_reconsider_at_all() {
        let (mut world, mut bench) = bench();
        let other = world.spawn();
        let mut mind = Mind::new();
        let work = [WorkCandidate::new(bench.stake, 0.6, 0.0).by(1.0, 1.0)];
        mind.update(&situation(&bench, 0, 1.0, &work), &bench.tuning);

        // Far better work appears, but this settler's stagger slot is 60
        // ticks away and nothing else has changed: they carry on regardless.
        let tempting = [
            WorkCandidate::new(bench.stake, 0.6, 0.0).by(1.0, 1.0),
            WorkCandidate::new(other, 1.0, 0.0).by(1.0, 1.0),
        ];
        bench.needs.hunger = 0.5;
        for tick in 1..bench.tuning.recompute_period {
            if staggered_recompute(bench.settler, tick, &bench.tuning) {
                continue;
            }
            assert_eq!(
                mind.update(&situation(&bench, tick, 1.0, &tempting), &bench.tuning),
                Activity::Work {
                    target: bench.stake
                }
            );
            assert_eq!(mind.last_trigger(), None, "no trigger, no thought");
        }
    }

    #[test]
    fn the_stagger_spreads_settlers_across_the_period_without_a_die_roll() {
        let mut world = World::new();
        let tuning = MindTuning::default();
        let settlers: Vec<Entity> = (0..tuning.recompute_period)
            .map(|_| world.spawn())
            .collect();

        // On any one tick exactly one of sixty settlers reconsiders...
        for tick in 0..3 * tuning.recompute_period {
            let due = settlers
                .iter()
                .filter(|s| staggered_recompute(**s, tick, &tuning))
                .count();
            assert_eq!(due, 1, "tick {tick} woke {due} settlers, not one");
        }
        // ...and every settler gets exactly one turn per period.
        for settler in &settlers {
            let turns = (0..tuning.recompute_period)
                .filter(|tick| staggered_recompute(*settler, *tick, &tuning))
                .count();
            assert_eq!(turns, 1);
        }
    }

    #[test]
    fn the_hold_clock_only_restarts_on_an_actual_change_of_mind() {
        let (_world, bench) = bench();
        let tuning = MindTuning {
            recompute_period: 1,
            ..bench.tuning
        };
        let work = [WorkCandidate::new(bench.stake, 0.6, 0.0).by(1.0, 1.0)];
        let mut mind = Mind::new();
        mind.update(&situation(&bench, 0, 1.0, &work), &tuning);
        assert_eq!(mind.since(), 0);

        for tick in 1..100 {
            mind.update(&situation(&bench, tick, 1.0, &work), &tuning);
        }
        assert_eq!(mind.since(), 0, "the same activity all along");
        assert_eq!(mind.held_ticks(99), 99);
    }
}
