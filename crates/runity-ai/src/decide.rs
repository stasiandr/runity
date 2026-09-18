//! Choosing what to do next.
//!
//! Not a behaviour tree and not a state machine: both encode the *order* of
//! decisions, and a villager's order changes constantly — hungry in winter,
//! ambitious in spring, terrified when a monster appears. Utility scoring
//! encodes the *reasons* instead. Every option scores itself from the world as
//! it is, and the best one wins. Adding a new thing villagers can do is then
//! one more scoring function, not a rewrite of the tree it would have hung in.
//!
//! Everything here is deterministic. Where a choice is deliberately varied it
//! draws from a seeded [`Rng`], never from wall-clock time or hashing order.

use runity_math::Rng;

/// Combine considerations into one score in `[0, 1]`.
///
/// Multiplying is the right instinct — an option that fails any of its
/// requirements should score near zero — but it is too harsh on its own: four
/// considerations at 0.8 multiply to 0.41, so an option that is good at
/// everything loses to one with a single mediocre consideration. The
/// compensation term pulls the product back up by how much the multiplication
/// cost it, which is Dave Mark's fix and the reason utility AI is usable with
/// more than two or three considerations.
pub fn score(considerations: &[f32]) -> f32 {
    if considerations.is_empty() {
        return 0.0;
    }
    let mut product = 1.0f32;
    for value in considerations {
        product *= value.clamp(0.0, 1.0);
        if product == 0.0 {
            // One veto is enough; nothing later can bring it back.
            return 0.0;
        }
    }
    let compensation = 1.0 - 1.0 / considerations.len() as f32;
    let adjustment = (1.0 - product) * compensation;
    (product + adjustment * product).clamp(0.0, 1.0)
}

/// The highest-scoring option, with its score.
///
/// Ties go to the earlier option, always — so a villager with two equally good
/// choices makes the same one on every machine, and the order you list options
/// in is a deliberate priority rather than an accident.
pub fn choose<A: Copy>(options: &[(A, f32)]) -> Option<(A, f32)> {
    let mut best: Option<(A, f32)> = None;
    for (action, score) in options {
        let score = *score;
        match best {
            Some((_, current)) if current >= score => {}
            _ => best = Some((*action, score)),
        }
    }
    best
}

/// Pick randomly among the options that score within `tolerance` of the best.
///
/// The cure for a hundred villagers who all decide to chop the same tree.
/// Because it draws from a seeded generator, the crowd is varied but the run
/// is still reproducible.
pub fn choose_near_best<A: Copy>(
    options: &[(A, f32)],
    tolerance: f32,
    rng: &mut Rng,
) -> Option<(A, f32)> {
    let (_, best) = choose(options)?;
    if best <= 0.0 {
        return None;
    }
    let cutoff = best - tolerance.max(0.0);
    let mut shortlist: Vec<(A, f32)> = Vec::new();
    for (action, score) in options {
        if *score >= cutoff && *score > 0.0 {
            shortlist.push((*action, *score));
        }
    }
    let index = rng.below(shortlist.len().max(1) as u32) as usize;
    shortlist.get(index).copied()
}

/// Pick an option with probability proportional to its score.
///
/// For choices where the *shape* of the preference matters — which tree to
/// fell, which of three routes to take — rather than which option is best.
pub fn choose_weighted<A: Copy>(options: &[(A, f32)], rng: &mut Rng) -> Option<(A, f32)> {
    let total: f32 = options.iter().map(|(_, score)| score.max(0.0)).sum();
    if total <= 0.0 {
        return None;
    }
    let mut roll = rng.range(0.0, total);
    for (action, score) in options {
        roll -= score.max(0.0);
        if roll <= 0.0 {
            return Some((*action, *score));
        }
    }
    options.last().copied()
}

/// Shapes for turning a measurement into a consideration in `[0, 1]`.
///
/// Kept as plain functions rather than a curve type with parameters: a
/// consideration is easier to read as `curve::falloff(distance, 20.0)` than as
/// a struct built three lines earlier.
pub mod curve {
    /// `value` mapped from the range `[low, high]` onto `[0, 1]`.
    pub fn ramp(value: f32, low: f32, high: f32) -> f32 {
        if (high - low).abs() < 1e-9 {
            return if value >= high { 1.0 } else { 0.0 };
        }
        ((value - low) / (high - low)).clamp(0.0, 1.0)
    }

    /// The same, the other way up: more means less.
    pub fn inverse(value: f32, low: f32, high: f32) -> f32 {
        1.0 - ramp(value, low, high)
    }

    /// Falls from 1 at zero distance to 0 at `range`, quickly at first.
    ///
    /// The right shape for "how much do I care about this, given how far away
    /// it is": twice as far should be much less than half as interesting.
    pub fn falloff(distance: f32, range: f32) -> f32 {
        if range <= 0.0 {
            return 0.0;
        }
        let normalized = (distance / range).clamp(0.0, 1.0);
        (1.0 - normalized) * (1.0 - normalized)
    }

    /// Smooth S-curve across `[low, high]` — no sudden change of mind at the
    /// edges, which is what makes scored behaviour look considered.
    pub fn smooth(value: f32, low: f32, high: f32) -> f32 {
        let t = ramp(value, low, high);
        t * t * (3.0 - 2.0 * t)
    }

    /// 1 above the threshold, 0 below it. A hard requirement, and a veto when
    /// multiplied with the rest.
    pub fn above(value: f32, threshold: f32) -> f32 {
        if value >= threshold {
            1.0
        } else {
            0.0
        }
    }

    /// 1 inside `[low, high]`, tapering to 0 over `edge` on either side.
    ///
    /// For preferences with a sweet spot: a resource neither too close to
    /// bother walking to nor too far to be worth it.
    pub fn band(value: f32, low: f32, high: f32, edge: f32) -> f32 {
        if value >= low && value <= high {
            return 1.0;
        }
        if edge <= 0.0 {
            return 0.0;
        }
        if value < low {
            ramp(value, low - edge, low)
        } else {
            inverse(value, high, high + edge)
        }
    }

    /// Urgency that rises sharply as a need approaches its limit.
    ///
    /// `need` is 0 when satisfied and 1 when critical; the curve stays low
    /// while there is slack and then takes over everything, which is how
    /// hunger should behave and how a linear consideration never does.
    pub fn urgency(need: f32) -> f32 {
        let n = need.clamp(0.0, 1.0);
        n * n * n
    }
}

#[cfg(test)]
mod tests {
    use super::curve::*;
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Task {
        Forage,
        Chop,
        Build,
        Flee,
    }

    #[test]
    fn a_score_stays_in_range_and_a_veto_wins() {
        assert_eq!(score(&[]), 0.0);
        assert_eq!(score(&[1.0, 1.0, 1.0]), 1.0);
        assert_eq!(
            score(&[0.9, 0.0, 0.9]),
            0.0,
            "one impossible requirement ends it"
        );
        assert_eq!(
            score(&[2.0, -1.0]),
            0.0,
            "values outside the range are clamped"
        );
        for values in [vec![0.5], vec![0.3, 0.8], vec![0.6, 0.6, 0.6, 0.6]] {
            let result = score(&values);
            assert!((0.0..=1.0).contains(&result), "{values:?} scored {result}");
        }
    }

    #[test]
    fn compensation_keeps_a_broadly_good_option_competitive() {
        // Four considerations at 0.8 multiply to 0.41 — worse than a single
        // 0.5, which is the wrong answer and the reason compensation exists.
        let broad = score(&[0.8, 0.8, 0.8, 0.8]);
        let narrow = score(&[0.5]);
        assert!(broad > 0.41 * 1.4, "raw product would be 0.41, got {broad}");
        assert!(
            broad > narrow,
            "good at everything should beat mediocre at one thing"
        );
        // But it is still a penalty: four 0.8s are not as good as four 1.0s.
        assert!(broad < score(&[1.0, 1.0, 1.0, 1.0]));
    }

    #[test]
    fn the_best_option_wins_and_ties_go_to_the_first() {
        let options = [(Task::Forage, 0.4), (Task::Chop, 0.9), (Task::Build, 0.9)];
        assert_eq!(choose(&options), Some((Task::Chop, 0.9)));
        assert_eq!(choose::<Task>(&[]), None);

        // The same list always decides the same way.
        for _ in 0..5 {
            assert_eq!(choose(&options), Some((Task::Chop, 0.9)));
        }
    }

    #[test]
    fn hunger_takes_over_when_it_gets_serious() {
        // The decision a settlement sim makes constantly: keep working, or eat.
        let decide = |hunger: f32, wood_needed: f32| {
            let options = [
                (Task::Forage, score(&[urgency(hunger), 0.9])),
                (Task::Chop, score(&[wood_needed, inverse(hunger, 0.3, 0.9)])),
            ];
            choose(&options).map(|(task, _)| task)
        };

        assert_eq!(
            decide(0.1, 0.8),
            Some(Task::Chop),
            "barely peckish: keep working"
        );
        assert_eq!(
            decide(0.95, 0.8),
            Some(Task::Forage),
            "starving: nothing else matters"
        );
        assert_eq!(decide(0.95, 0.0), Some(Task::Forage));
    }

    #[test]
    fn a_veto_beats_every_other_consideration() {
        let monster_nearby = 1.0;
        let options = [
            (Task::Chop, score(&[0.99, above(monster_nearby, 0.5) * 0.0])),
            (Task::Flee, score(&[0.4, above(monster_nearby, 0.5)])),
        ];
        assert_eq!(choose(&options).map(|(task, _)| task), Some(Task::Flee));
    }

    #[test]
    fn choosing_near_the_best_spreads_a_crowd_without_losing_the_thread() {
        let options = [
            (Task::Forage, 0.80),
            (Task::Chop, 0.78),
            (Task::Build, 0.30),
            (Task::Flee, 0.00),
        ];
        let mut rng = Rng::named(1, "crowd");
        let mut counts = [0; 4];
        for _ in 0..400 {
            let (task, _) = choose_near_best(&options, 0.05, &mut rng).unwrap();
            counts[task as usize] += 1;
        }
        assert!(counts[Task::Forage as usize] > 100 && counts[Task::Chop as usize] > 100);
        assert_eq!(counts[Task::Build as usize], 0, "0.30 is not near 0.80");
        assert_eq!(counts[Task::Flee as usize], 0, "and zero is never chosen");

        // The same seed gives the same crowd.
        let mut first = Rng::named(2, "crowd");
        let mut second = Rng::named(2, "crowd");
        for _ in 0..50 {
            assert_eq!(
                choose_near_best(&options, 0.05, &mut first),
                choose_near_best(&options, 0.05, &mut second)
            );
        }
        assert_eq!(choose_near_best::<Task>(&[], 0.1, &mut rng), None);
        assert_eq!(choose_near_best(&[(Task::Chop, 0.0)], 0.1, &mut rng), None);
    }

    #[test]
    fn weighted_choice_follows_the_weights() {
        let options = [(Task::Forage, 3.0), (Task::Chop, 1.0)];
        let mut rng = Rng::named(3, "weights");
        let mut forage = 0;
        for _ in 0..2_000 {
            if choose_weighted(&options, &mut rng).unwrap().0 == Task::Forage {
                forage += 1;
            }
        }
        let share = forage as f32 / 2_000.0;
        assert!(
            (share - 0.75).abs() < 0.04,
            "three to one should come out near 0.75, got {share}"
        );
        assert_eq!(choose_weighted(&[(Task::Chop, 0.0)], &mut rng), None);
        assert_eq!(choose_weighted::<Task>(&[], &mut rng), None);
    }

    #[test]
    fn the_curves_have_the_shapes_their_names_claim() {
        assert_eq!(ramp(5.0, 0.0, 10.0), 0.5);
        assert_eq!(ramp(-5.0, 0.0, 10.0), 0.0);
        assert_eq!(ramp(50.0, 0.0, 10.0), 1.0);
        assert_eq!(
            ramp(1.0, 1.0, 1.0),
            1.0,
            "a degenerate range does not divide by zero"
        );

        assert_eq!(inverse(0.0, 0.0, 10.0), 1.0);
        assert_eq!(inverse(10.0, 0.0, 10.0), 0.0);

        assert_eq!(falloff(0.0, 20.0), 1.0);
        assert_eq!(falloff(20.0, 20.0), 0.0);
        assert!(
            falloff(10.0, 20.0) < 0.5,
            "distance should hurt faster than linearly"
        );
        assert_eq!(falloff(1.0, 0.0), 0.0);

        assert!((smooth(5.0, 0.0, 10.0) - 0.5).abs() < 1e-6);
        assert!(
            smooth(2.0, 0.0, 10.0) < ramp(2.0, 0.0, 10.0),
            "the S-curve starts slowly"
        );

        assert_eq!(above(0.6, 0.5), 1.0);
        assert_eq!(above(0.4, 0.5), 0.0);

        assert_eq!(band(5.0, 2.0, 8.0, 1.0), 1.0);
        assert_eq!(band(1.0, 2.0, 8.0, 1.0), 0.0);
        assert!((band(1.5, 2.0, 8.0, 1.0) - 0.5).abs() < 1e-6);
        assert!((band(8.5, 2.0, 8.0, 1.0) - 0.5).abs() < 1e-6);
        assert_eq!(band(9.0, 2.0, 8.0, 0.0), 0.0);

        assert_eq!(urgency(0.0), 0.0);
        assert_eq!(urgency(1.0), 1.0);
        assert!(
            urgency(0.5) < 0.2,
            "a need with slack left should stay quiet"
        );
        assert!(urgency(0.9) > 0.7, "and take over near the limit");
    }

    #[test]
    fn a_decision_is_stable_while_the_world_is() {
        // Scored behaviour must not flicker: the same inputs have to give the
        // same answer, or a villager spends the day turning around.
        let world = |hunger: f32| {
            [
                (Task::Forage, score(&[urgency(hunger), falloff(12.0, 40.0)])),
                (Task::Chop, score(&[0.7, inverse(hunger, 0.4, 1.0)])),
                (Task::Build, score(&[0.6, above(1.0, 0.5)])),
            ]
        };
        let first = choose(&world(0.3));
        for _ in 0..10 {
            assert_eq!(choose(&world(0.3)), first);
        }
    }
}
