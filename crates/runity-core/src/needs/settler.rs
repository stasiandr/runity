//! A settler's biological needs.

/// Which of the three needs a level or an action is about.
///
/// [`NeedKind::ALL`] fixes the order — hunger, warmth, rest — and that order
/// is the tie break wherever two needs are equally bad. A mind that broke such
/// a tie by iteration order of a map would not be deterministic, and
/// `08-scale.md` asks that it be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NeedKind {
    /// Голод.
    Hunger,
    /// Холод.
    Warmth,
    /// Отдых.
    Rest,
}

impl NeedKind {
    /// All three, in the order every tie is broken by.
    pub const ALL: [NeedKind; 3] = [NeedKind::Hunger, NeedKind::Warmth, NeedKind::Rest];
}

/// How fast [`Needs`] decays and how fast each relieving action restores it.
///
/// Placeholder tuning, in the spirit of [`crate::physics::Tuning`]: the real
/// numbers come from playtesting once a mind (`12-minds.md`) is actually
/// deciding when to eat, warm up or sleep. Rates are per simulated second, not
/// per tick, so they read the same whether the fixed step is 20 Hz or 60 Hz.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NeedsRates {
    /// How much [`Needs::hunger`] drops per second.
    pub hunger_decay: f32,
    /// How much [`Needs::warmth`] drops per second.
    pub warmth_decay: f32,
    /// How much [`Needs::rest`] drops per second.
    pub rest_decay: f32,
    /// How much [`Needs::warm`] restores warmth per second spent near a hearth.
    pub warm_gain: f32,
    /// How much [`Needs::sleep`] restores rest per second spent asleep.
    pub sleep_gain: f32,
}

impl Default for NeedsRates {
    fn default() -> Self {
        Self {
            hunger_decay: 1.0 / 600.0,
            warmth_decay: 1.0 / 300.0,
            rest_decay: 1.0 / 900.0,
            warm_gain: 1.0 / 30.0,
            sleep_gain: 1.0 / 120.0,
        }
    }
}

/// A settler's hunger, warmth and rest — a component attached to the settler's
/// entity.
///
/// Each level lives in `[0, 1]`, where `1.0` is fully satisfied and `0.0` is
/// critical. All three decay on their own as simulated time passes
/// ([`Needs::decay`]) and are relieved only by a direct call — [`Needs::eat`],
/// [`Needs::warm`], [`Needs::sleep`] — so a test, or a system with no mind
/// behind it at all, can drive a settler's needs by itself. Nothing here reads
/// `World` or knows what a hearth or a bed looks like; that wiring lives in
/// [`super::tick_settlement`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Needs {
    pub hunger: f32,
    pub warmth: f32,
    pub rest: f32,
    pub rates: NeedsRates,
}

impl Needs {
    /// A settler that starts fully satisfied on all three needs.
    pub fn new(rates: NeedsRates) -> Self {
        Self {
            hunger: 1.0,
            warmth: 1.0,
            rest: 1.0,
            rates,
        }
    }

    /// One need's level, `[0, 1]`.
    pub fn level(&self, kind: NeedKind) -> f32 {
        match kind {
            NeedKind::Hunger => self.hunger,
            NeedKind::Warmth => self.warmth,
            NeedKind::Rest => self.rest,
        }
    }

    /// The need in the worst shape, ties broken by [`NeedKind::ALL`]'s order.
    pub fn lowest(&self) -> NeedKind {
        NeedKind::ALL
            .into_iter()
            .reduce(|worst, kind| {
                if self.level(kind) < self.level(worst) {
                    kind
                } else {
                    worst
                }
            })
            .expect("NeedKind::ALL is never empty")
    }

    /// Relieve `kind` by `dt` seconds of the action that treats it. Eating is
    /// the odd one out — it finishes in a single call, no matter the `dt`.
    pub fn relieve(&mut self, kind: NeedKind, dt: f32) {
        match kind {
            NeedKind::Hunger => self.eat(),
            NeedKind::Warmth => self.warm(dt),
            NeedKind::Rest => self.sleep(dt),
        }
    }

    /// Let `dt` seconds of simulated time pass, decaying all three needs.
    pub fn decay(&mut self, dt: f32) {
        self.hunger = (self.hunger - self.rates.hunger_decay * dt).max(0.0);
        self.warmth = (self.warmth - self.rates.warmth_decay * dt).max(0.0);
        self.rest = (self.rest - self.rates.rest_decay * dt).max(0.0);
    }

    /// A meal, finished in a single tick: hunger goes straight to full.
    pub fn eat(&mut self) {
        self.hunger = 1.0;
    }

    /// `dt` seconds spent near a hearth.
    pub fn warm(&mut self, dt: f32) {
        self.warmth = (self.warmth + self.rates.warm_gain * dt).min(1.0);
    }

    /// `dt` seconds spent asleep.
    pub fn sleep(&mut self, dt: f32) {
        self.rest = (self.rest + self.rates.sleep_gain * dt).min(1.0);
    }
}

impl Default for Needs {
    fn default() -> Self {
        Self::new(NeedsRates::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_rates() -> NeedsRates {
        NeedsRates {
            hunger_decay: 0.1,
            warmth_decay: 0.2,
            rest_decay: 0.3,
            warm_gain: 0.5,
            sleep_gain: 0.25,
        }
    }

    #[test]
    fn a_fresh_settler_starts_fully_satisfied() {
        let needs = Needs::default();
        assert_eq!(needs.hunger, 1.0);
        assert_eq!(needs.warmth, 1.0);
        assert_eq!(needs.rest, 1.0);
    }

    #[test]
    fn decay_reduces_every_need_and_stops_at_zero() {
        let mut needs = Needs::new(fast_rates());
        needs.decay(1.0);
        assert!((needs.hunger - 0.9).abs() < 1e-6);
        assert!((needs.warmth - 0.8).abs() < 1e-6);
        assert!((needs.rest - 0.7).abs() < 1e-6);

        needs.decay(100.0);
        assert_eq!(needs.hunger, 0.0, "decay never goes negative");
        assert_eq!(needs.warmth, 0.0);
        assert_eq!(needs.rest, 0.0);
    }

    #[test]
    fn eat_is_a_direct_call_that_fully_relieves_hunger_at_once() {
        let mut needs = Needs::new(fast_rates());
        needs.decay(1.0);
        assert!(needs.hunger < 1.0);
        needs.eat();
        assert_eq!(needs.hunger, 1.0);
    }

    #[test]
    fn warm_and_sleep_restore_gradually_and_clamp_at_one() {
        let mut needs = Needs::new(fast_rates());
        needs.decay(2.0);
        let warmth_before = needs.warmth;
        needs.warm(1.0);
        assert!(needs.warmth > warmth_before);
        assert!(needs.warmth <= 1.0);

        needs.warm(100.0);
        assert_eq!(needs.warmth, 1.0, "warming never overshoots full");

        let rest_before = needs.rest;
        needs.sleep(1.0);
        assert!(needs.rest > rest_before);
        needs.sleep(100.0);
        assert_eq!(needs.rest, 1.0);
    }

    #[test]
    fn the_lowest_need_is_the_worst_one_with_a_fixed_tie_break() {
        let mut needs = Needs {
            warmth: 0.2,
            rest: 0.5,
            ..Needs::default()
        };
        assert_eq!(needs.lowest(), NeedKind::Warmth);

        // All three equal: the first of NeedKind::ALL wins, every time.
        needs.hunger = 0.4;
        needs.warmth = 0.4;
        needs.rest = 0.4;
        assert_eq!(needs.lowest(), NeedKind::Hunger);
    }

    #[test]
    fn relieve_routes_to_the_action_that_treats_that_need() {
        let mut needs = Needs::new(fast_rates());
        needs.decay(2.0);
        let (warmth, rest) = (needs.warmth, needs.rest);

        needs.relieve(NeedKind::Hunger, 0.0);
        assert_eq!(needs.hunger, 1.0, "a meal ignores dt entirely");
        assert_eq!(needs.warmth, warmth, "and touches nothing else");

        needs.relieve(NeedKind::Warmth, 1.0);
        assert!(needs.warmth > warmth);
        needs.relieve(NeedKind::Rest, 1.0);
        assert!(needs.rest > rest);
    }

    #[test]
    fn level_reads_the_field_the_kind_names() {
        let needs = Needs {
            hunger: 0.1,
            warmth: 0.2,
            rest: 0.3,
            ..Needs::default()
        };
        assert_eq!(needs.level(NeedKind::Hunger), 0.1);
        assert_eq!(needs.level(NeedKind::Warmth), 0.2);
        assert_eq!(needs.level(NeedKind::Rest), 0.3);
    }

    #[test]
    fn these_actions_need_nothing_but_the_component_itself() {
        // No World, no Engine, no mind: this is the whole point of a direct
        // API — a test can drive it standalone.
        let mut needs = Needs::default();
        needs.decay(0.5);
        needs.eat();
        needs.warm(0.5);
        needs.sleep(0.5);
    }
}
