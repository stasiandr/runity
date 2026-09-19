//! The three scoring terms of `12-minds.md` §3 — need, common work, own
//! business — and the one step of crafting inference that hangs off the
//! second of them.
//!
//! Everything here is a pure function of a [`Situation`] and a
//! [`MindTuning`]: no `World`, no clock, no randomness. The hysteresis that
//! turns a list of scores into a settler who is not twitching lives next
//! door, in [`super::Mind`].

use crate::economy::Tag;
use crate::needs::{NeedKind, Needs};
use crate::world::Entity;

use super::tuning::MindTuning;
use super::work::{AtHand, WorkCandidate};

/// What a settler can be doing, and the one thing each kind points at.
///
/// Comparing two of these is how [`super::Mind`] knows a settler changed
/// their mind, so the payloads are the identity of the activity and nothing
/// more: two ticks of carrying logs to the same stake are the same activity,
/// and the same work at a different stake is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Activity {
    /// Relieving one of the settler's own three needs.
    Need(NeedKind),
    /// Common work: a stake, or a standing chore like firewood to the hearth.
    Work {
        /// The [`WorkCandidate::target`] being worked on.
        target: Entity,
    },
    /// Making, in one action, the property `target` asks for — §3.5's single
    /// step of inference, and never more than one.
    Craft {
        /// The work this step would unlock.
        target: Entity,
        /// The property being made.
        tag: Tag,
    },
    /// The settler's own business: own lean-to, own tool, talking, sitting.
    OwnBusiness,
}

impl Activity {
    /// The work this activity is pointed at, if any — what
    /// [`super::Mind`] checks to notice a target has disappeared.
    pub fn target(self) -> Option<Entity> {
        match self {
            Activity::Work { target } | Activity::Craft { target, .. } => Some(target),
            Activity::Need(_) | Activity::OwnBusiness => None,
        }
    }
}

/// One candidate and what it scored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scored {
    /// The candidate.
    pub activity: Activity,
    /// Its score before any stickiness bonus.
    pub score: f32,
}

/// Everything a settler's mind reads at the moment it decides.
///
/// The mind of `02-settlers.md` keeps no plan, so this is deliberately not a
/// state the caller has to maintain: it is the current facts, gathered fresh,
/// and two settlers handed the same ones reach the same conclusion.
#[derive(Debug, Clone, Copy)]
pub struct Situation<'a> {
    /// Whose situation this is. Its [`Entity::index`] is what staggers the
    /// recompute in §3.7.
    pub settler: Entity,
    /// The cumulative tick count, as [`crate::Time::elapsed_ticks`] reports it.
    pub tick: u64,
    /// The settler's own three needs.
    pub needs: Needs,
    /// How bright it is, out of [`MindTuning::light_at_hour`].
    pub light: f32,
    /// Every piece of common work the settler can see.
    pub work: &'a [WorkCandidate],
    /// What they are carrying, and what they could make from it in one action.
    pub at_hand: &'a AtHand,
}

impl Situation<'_> {
    /// The need that has fallen through [`MindTuning::need_floor`], if one
    /// has — the worst of them when more than one did.
    ///
    /// This is the one thing in the whole module that overrides rather than
    /// competes: a settler this far down does not weigh eating against a
    /// stake, they eat.
    pub fn floor_breach(&self, tuning: &MindTuning) -> Option<NeedKind> {
        let worst = self.needs.lowest();
        (self.needs.level(worst) < tuning.need_floor).then_some(worst)
    }
}

/// How badly a need at `level` wants relieving: `(1 - level)^3`.
///
/// Cubed, not linear, so that a need at 0.8 is nearly silent (0.008) while
/// one at 0.2 shouts (0.512): a settler should not abandon work because they
/// are slightly peckish, and should abandon anything at all once they are
/// genuinely hungry. Below [`MindTuning::need_floor`] the curve stops
/// mattering and [`Situation::floor_breach`] takes over.
pub fn need_urgency(level: f32) -> f32 {
    let room = 1.0 - level.clamp(0.0, 1.0);
    room * room * room
}

/// What a piece of common work is worth to the settler who is looking at it:
/// `severity × (unskilled_share + (1 - unskilled_share)·skill) × disposition ×
/// light ÷ (1 + distance / distance_halving_metres)`.
///
/// `light` is a multiplier here and nowhere else. That single asymmetry with
/// [`MindTuning::own_business`] is the whole of the game's evening: as the
/// sun goes down every common-work score is scaled towards nothing while own
/// business holds still, and at some hour they cross. No rule anywhere says
/// "settlers stop working at dusk".
pub fn work_urgency(work: &WorkCandidate, light: f32, tuning: &MindTuning) -> f32 {
    let skill = work.skill.clamp(0.0, 1.0);
    let skill_term = tuning.unskilled_share + (1.0 - tuning.unskilled_share) * skill;
    let falloff = 1.0 + work.distance.max(0.0) / tuning.distance_halving_metres;
    work.severity * skill_term * work.disposition * light / falloff
}

/// Every candidate this settler has, scored, in a fixed order: the three
/// needs, then the work in the order it was seen, then own business.
///
/// The order is part of the contract — [`super::Mind`] breaks ties by taking
/// the first of two equal scores, and a settlement of a thousand minds has to
/// make the same choice twice.
///
/// Work the settler cannot actually contribute to is not a candidate: a stake
/// asking for something `Sharp` is not work a settler with nothing sharp can
/// do. What they get instead, when one action would fix that, is a
/// [`Activity::Craft`] candidate worth [`MindTuning::craft_fraction`] of the
/// work — §3.5's single step of inference, and the only way such a settler
/// ever joins that stake.
pub fn candidates(situation: &Situation, tuning: &MindTuning) -> Vec<Scored> {
    let mut scored = Vec::with_capacity(NeedKind::ALL.len() + situation.work.len() + 1);

    for kind in NeedKind::ALL {
        scored.push(Scored {
            activity: Activity::Need(kind),
            score: need_urgency(situation.needs.level(kind)),
        });
    }

    for work in situation.work {
        let score = work_urgency(work, situation.light, tuning);
        match work.wants {
            Some(tag) if !situation.at_hand.has(tag) => {
                if situation.at_hand.one_step_from(tag) {
                    scored.push(Scored {
                        activity: Activity::Craft {
                            target: work.target,
                            tag,
                        },
                        score: score * tuning.craft_fraction,
                    });
                }
            }
            _ => scored.push(Scored {
                activity: Activity::Work {
                    target: work.target,
                },
                score,
            }),
        }
    }

    scored.push(Scored {
        activity: Activity::OwnBusiness,
        score: tuning.own_business,
    });

    scored
}

/// The score `activity` carries in `scored`, or `None` if it is not a
/// candidate at all any more.
pub(super) fn score_of(scored: &[Scored], activity: Activity) -> Option<f32> {
    scored
        .iter()
        .find(|s| s.activity == activity)
        .map(|s| s.score)
}

/// The best of `scored`, first of any tie wins. `None` only for an empty
/// slice, which [`candidates`] never produces.
pub(super) fn best(scored: &[Scored]) -> Option<Scored> {
    scored
        .iter()
        .copied()
        .reduce(|best, next| if next.score > best.score { next } else { best })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::needs::NeedsRates;
    use crate::world::World;

    fn still_needs() -> NeedsRates {
        NeedsRates {
            hunger_decay: 0.0,
            warmth_decay: 0.0,
            rest_decay: 0.0,
            ..NeedsRates::default()
        }
    }

    fn situation<'a>(
        settler: Entity,
        needs: Needs,
        work: &'a [WorkCandidate],
        at_hand: &'a AtHand,
        light: f32,
    ) -> Situation<'a> {
        Situation {
            settler,
            tick: 0,
            needs,
            light,
            work,
            at_hand,
        }
    }

    #[test]
    fn need_urgency_is_a_cube_that_stays_quiet_until_it_does_not() {
        assert_eq!(need_urgency(1.0), 0.0);
        assert!((need_urgency(0.0) - 1.0).abs() < 1e-6);
        assert!((need_urgency(0.5) - 0.125).abs() < 1e-6);
        assert!(
            need_urgency(0.8) < 0.01,
            "slightly peckish is nearly silent"
        );
        assert!(need_urgency(0.2) > 0.5, "genuinely hungry shouts");
        // Out-of-range levels are clamped, not extrapolated.
        assert_eq!(need_urgency(1.5), 0.0);
        assert!((need_urgency(-1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn work_urgency_matches_the_formula_term_for_term() {
        let mut world = World::new();
        let stake = world.spawn();
        let tuning = MindTuning::default();
        let work = WorkCandidate::new(stake, 0.8, 50.0).by(1.0, 0.5);
        // 0.8 * (0.5 + 0.5*1.0) * 0.5 * 1.0 / (1 + 50/50) = 0.2
        assert!((work_urgency(&work, 1.0, &tuning) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn work_falls_off_with_distance_and_rises_with_skill() {
        let mut world = World::new();
        let stake = world.spawn();
        let tuning = MindTuning::default();

        let underfoot = WorkCandidate::new(stake, 1.0, 0.0);
        let far = WorkCandidate::new(stake, 1.0, 50.0);
        assert!(
            (work_urgency(&far, 1.0, &tuning) * 2.0 - work_urgency(&underfoot, 1.0, &tuning)).abs()
                < 1e-6,
            "fifty metres is exactly half"
        );

        let unskilled = WorkCandidate::new(stake, 1.0, 0.0).by(0.0, 1.0);
        let skilled = WorkCandidate::new(stake, 1.0, 0.0).by(1.0, 1.0);
        assert!(
            (work_urgency(&unskilled, 1.0, &tuning) * 2.0 - work_urgency(&skilled, 1.0, &tuning))
                .abs()
                < 1e-6,
            "a settler with no skill still brings half"
        );
    }

    #[test]
    fn light_scales_common_work_and_leaves_own_business_alone() {
        let mut world = World::new();
        let settler = world.spawn();
        let stake = world.spawn();
        let tuning = MindTuning::default();
        let work = [WorkCandidate::new(stake, 1.0, 0.0).by(1.0, 1.0)];
        let hand = AtHand::default();
        let needs = Needs::new(still_needs());

        let noon = candidates(&situation(settler, needs, &work, &hand, 1.0), &tuning);
        let night = candidates(&situation(settler, needs, &work, &hand, 0.15), &tuning);

        let work_at =
            |scored: &[Scored]| score_of(scored, Activity::Work { target: stake }).unwrap();
        let own_at = |scored: &[Scored]| score_of(scored, Activity::OwnBusiness).unwrap();

        assert!(work_at(&night) < work_at(&noon));
        assert!((work_at(&night) - work_at(&noon) * 0.15).abs() < 1e-6);
        assert_eq!(
            own_at(&night),
            own_at(&noon),
            "own business does not care what time it is"
        );
    }

    #[test]
    fn work_asking_for_something_not_in_hand_becomes_a_craft_at_half_score() {
        let mut world = World::new();
        let settler = world.spawn();
        let stake = world.spawn();
        let tuning = MindTuning::default();
        let work = [WorkCandidate::new(stake, 1.0, 0.0)
            .wanting(Tag::Sharp)
            .by(1.0, 1.0)];
        let hand = AtHand::new(
            [Tag::Hard],
            [super::super::OneStep::new(Tag::Hard, Tag::Sharp)],
        );
        let needs = Needs::new(still_needs());

        let scored = candidates(&situation(settler, needs, &work, &hand, 1.0), &tuning);
        assert_eq!(
            score_of(&scored, Activity::Work { target: stake }),
            None,
            "nothing sharp in hand: the stake itself is not work this settler can do"
        );
        let craft = score_of(
            &scored,
            Activity::Craft {
                target: stake,
                tag: Tag::Sharp,
            },
        )
        .expect("one step away, so crafting is a candidate");
        let full = work_urgency(&work[0], 1.0, &tuning);
        assert!((craft - full * 0.5).abs() < 1e-6);
    }

    #[test]
    fn work_asking_for_something_out_of_reach_is_no_candidate_at_all() {
        let mut world = World::new();
        let settler = world.spawn();
        let stake = world.spawn();
        let tuning = MindTuning::default();
        let work = [WorkCandidate::new(stake, 1.0, 0.0).wanting(Tag::Sharp)];
        let hand = AtHand::holding([Tag::Warm]);
        let needs = Needs::new(still_needs());

        let scored = candidates(&situation(settler, needs, &work, &hand, 1.0), &tuning);
        assert!(
            scored.iter().all(|s| s.activity.target().is_none()),
            "neither the stake nor a craft for it"
        );
    }

    #[test]
    fn holding_what_the_work_asks_for_makes_it_plain_work_again() {
        let mut world = World::new();
        let settler = world.spawn();
        let stake = world.spawn();
        let tuning = MindTuning::default();
        let work = [WorkCandidate::new(stake, 1.0, 0.0).wanting(Tag::Sharp)];
        let hand = AtHand::holding([Tag::Sharp]);
        let needs = Needs::new(still_needs());

        let scored = candidates(&situation(settler, needs, &work, &hand, 1.0), &tuning);
        assert!(score_of(&scored, Activity::Work { target: stake }).is_some());
    }

    #[test]
    fn the_candidate_order_is_fixed_so_two_runs_break_ties_the_same_way() {
        let mut world = World::new();
        let settler = world.spawn();
        let first = world.spawn();
        let second = world.spawn();
        let tuning = MindTuning::default();
        let work = [
            WorkCandidate::new(first, 1.0, 0.0),
            WorkCandidate::new(second, 1.0, 0.0),
        ];
        let hand = AtHand::default();
        let needs = Needs::new(still_needs());

        let scored = candidates(&situation(settler, needs, &work, &hand, 1.0), &tuning);
        let order: Vec<Activity> = scored.iter().map(|s| s.activity).collect();
        assert_eq!(
            order,
            vec![
                Activity::Need(NeedKind::Hunger),
                Activity::Need(NeedKind::Warmth),
                Activity::Need(NeedKind::Rest),
                Activity::Work { target: first },
                Activity::Work { target: second },
                Activity::OwnBusiness,
            ]
        );
        assert_eq!(
            best(&scored).unwrap().activity,
            Activity::Work { target: first },
            "equal scores: the first one seen wins"
        );
    }

    #[test]
    fn the_floor_is_read_off_the_worst_need_and_only_when_it_is_breached() {
        let tuning = MindTuning::default();
        let mut world = World::new();
        let settler = world.spawn();
        let hand = AtHand::default();

        let mut needs = Needs::new(still_needs());
        needs.warmth = 0.2;
        assert_eq!(
            situation(settler, needs, &[], &hand, 1.0).floor_breach(&tuning),
            None,
            "0.2 is low, but it is above the floor"
        );

        needs.warmth = 0.1;
        needs.hunger = 0.05;
        assert_eq!(
            situation(settler, needs, &[], &hand, 1.0).floor_breach(&tuning),
            Some(NeedKind::Hunger),
            "two below the floor: the worse one"
        );
    }
}
