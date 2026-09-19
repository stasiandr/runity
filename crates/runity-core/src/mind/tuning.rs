//! [`MindTuning`]: every number the personal mind weighs, in one place.

/// Every constant the personal mind of `12-minds.md` §3 reads, in a single
/// struct, in the spirit of [`crate::physics::Tuning`].
///
/// The point of collecting them is not tidiness — it is that a headless run
/// can be handed a different set without recompiling anything and without
/// hunting for a second copy of `0.15` somewhere else in the tree. Nothing in
/// this module holds a scoring constant of its own; if a number decides what
/// a settler does next, it is a field here.
///
/// The defaults are placeholder tuning, exactly like [`crate::needs::NeedsRates`]:
/// the real numbers come from watching a settlement run, and
/// [`crate::mind::Settlement`] is how it gets watched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MindTuning {
    /// A need below this level overrides everything — every work candidate,
    /// own business, and stickiness with it. Above it a need is just another
    /// candidate, scored like the rest.
    pub need_floor: f32,
    /// What a settler with no skill at all still brings to a piece of work:
    /// the skill term is `unskilled_share + (1 - unskilled_share) * skill`,
    /// which at the default is the `0.5 + 0.5·skill` of §3.2.
    pub unskilled_share: f32,
    /// Distance, in metres, at which a piece of work scores half what it
    /// would underfoot: the term is `1 / (1 + distance / this)`.
    pub distance_halving_metres: f32,
    /// Light at midnight. Common work is multiplied by light; own business is
    /// not, which is the whole reason a settler drifts off a stake at dusk
    /// without anything in the code mentioning dusk.
    pub night_light: f32,
    /// Light at noon.
    pub noon_light: f32,
    /// Own business — own lean-to, own tool, talking, sitting — scores this
    /// flat, whatever the hour and wherever the settler stands.
    pub own_business: f32,
    /// What one step of crafting inference (§3.5) is worth, as a fraction of
    /// the score of the work it would unlock.
    pub craft_fraction: f32,
    /// Score added to whatever the settler is already doing, so two
    /// candidates drifting past each other do not make them twitch.
    pub stickiness_bonus: f32,
    /// Ticks an activity must be held before it can be dropped at all.
    /// Pierced only by [`MindTuning::need_floor`] or by the target
    /// disappearing.
    pub min_activity_ticks: u64,
    /// The staggered recompute of §3.7: a settler reconsiders when
    /// `(entity index + tick) % this == 0`. A counter, not a die roll — at
    /// twelve settlers or twelve hundred, the same tick produces the same
    /// decisions.
    pub recompute_period: u64,
}

impl Default for MindTuning {
    fn default() -> Self {
        Self {
            need_floor: 0.15,
            unskilled_share: 0.5,
            distance_halving_metres: 50.0,
            night_light: 0.15,
            noon_light: 1.0,
            own_business: 0.15,
            craft_fraction: 0.5,
            // Roughly a third of own business: enough to hold a settler
            // through the noise of two candidates crossing, not enough to
            // hold them on work that has genuinely gone worse.
            stickiness_bonus: 0.05,
            // Two seconds at a 20 Hz fixed step.
            min_activity_ticks: 40,
            recompute_period: 60,
        }
    }
}

impl MindTuning {
    /// How bright it is at `hour` of the day (`0..24`, wraps), from
    /// [`MindTuning::night_light`] at midnight to [`MindTuning::noon_light`]
    /// at noon and back.
    ///
    /// Linear on either side of noon, so dawn and dusk — the hours
    /// `07-look.md` anchors the sky on — land exactly halfway between the
    /// two, and no third and fourth constant has to be kept in step with the
    /// first two. Linear also means no `sin`: the same reasoning as
    /// [`crate::physics`], since a settlement is meant to run identically on
    /// every machine.
    pub fn light_at_hour(&self, hour: f32) -> f32 {
        let hour = hour.rem_euclid(24.0);
        let noonness = 1.0 - (12.0 - hour).abs() / 12.0;
        self.night_light + (self.noon_light - self.night_light) * noonness
    }
}

/// Hour of day at `tick`, on the same sim-day
/// [`crate::needs::DeficitLog`] counts: a day is `ticks_per_day` fixed steps
/// long and tick zero is dawn, the boundary the log rolls the night over on.
///
/// ```
/// use runity_core::mind::hour_of_day;
///
/// let day = 240;
/// assert_eq!(hour_of_day(0, day), 6.0, "tick zero is dawn");
/// assert_eq!(hour_of_day(day / 4, day), 12.0, "a quarter day later, noon");
/// assert_eq!(hour_of_day(3 * day / 4, day), 0.0, "and midnight before dawn");
/// ```
pub fn hour_of_day(tick: u64, ticks_per_day: u64) -> f32 {
    assert!(
        ticks_per_day > 0,
        "a sim-day must be at least one tick long"
    );
    const DAWN: f32 = 6.0;
    let through = (tick % ticks_per_day) as f32 / ticks_per_day as f32;
    (DAWN + through * 24.0).rem_euclid(24.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_runs_from_night_at_midnight_to_noon_at_noon() {
        let tuning = MindTuning::default();
        assert!((tuning.light_at_hour(0.0) - 0.15).abs() < 1e-6);
        assert!((tuning.light_at_hour(12.0) - 1.0).abs() < 1e-6);
        assert!((tuning.light_at_hour(24.0) - 0.15).abs() < 1e-6);
    }

    #[test]
    fn dawn_and_dusk_sit_halfway_and_need_no_constants_of_their_own() {
        let tuning = MindTuning::default();
        let halfway = (tuning.night_light + tuning.noon_light) / 2.0;
        assert!((tuning.light_at_hour(6.0) - halfway).abs() < 1e-6);
        assert!((tuning.light_at_hour(18.0) - halfway).abs() < 1e-6);
    }

    #[test]
    fn light_falls_monotonically_from_noon_to_midnight() {
        let tuning = MindTuning::default();
        let mut previous = tuning.light_at_hour(12.0);
        for step in 1..=48 {
            let hour = 12.0 + step as f32 * 0.25;
            let light = tuning.light_at_hour(hour);
            assert!(light <= previous, "light rose again at hour {hour}");
            previous = light;
        }
        assert!((previous - tuning.night_light).abs() < 1e-6);
    }

    #[test]
    fn light_wraps_through_midnight_either_direction() {
        let tuning = MindTuning::default();
        assert!((tuning.light_at_hour(-1.0) - tuning.light_at_hour(23.0)).abs() < 1e-6);
        assert!((tuning.light_at_hour(25.0) - tuning.light_at_hour(1.0)).abs() < 1e-6);
    }

    #[test]
    fn the_hour_walks_a_full_circle_once_per_sim_day() {
        let day = 120;
        assert_eq!(hour_of_day(0, day), 6.0);
        assert_eq!(hour_of_day(day, day), 6.0, "the next dawn, same hour");
        assert_eq!(hour_of_day(day / 2, day), 18.0);
        assert!(hour_of_day(day - 1, day) < 6.0, "still before dawn");
    }

    #[test]
    #[should_panic(expected = "at least one tick long")]
    fn a_zero_length_day_is_a_caller_bug() {
        hour_of_day(1, 0);
    }
}
