//! The personal mind: need against common work against own business.
//!
//! `12-minds.md` §3. A settler keeps no plan — every tick they might look at
//! the facts in front of them and pick the next thing. This module is the
//! picking: three scoring terms, one step of crafting inference, and just
//! enough hysteresis that a settler who is nearly indifferent between two
//! candidates does not stand between them vibrating.
//!
//! * [`need_urgency`] — `(1 - level)^3` for each of the three needs, with a
//!   hard floor at [`MindTuning::need_floor`] below which the need stops
//!   competing and simply overrides.
//! * [`work_urgency`] — common work, `severity × skill × disposition × light
//!   ÷ distance falloff`. Light is a factor here and nowhere else.
//! * [`MindTuning::own_business`] — own lean-to, own tool, talking, sitting,
//!   at a flat score that no hour of the day touches.
//! * [`Activity::Craft`] — §3.5, one step: work asking for a property the
//!   settler has not got but could make in one action becomes a candidate at
//!   [`MindTuning::craft_fraction`] of the work's score.
//! * [`Mind`] — §3.3 and §3.7: the stickiness bonus, the minimum hold, and
//!   the four triggers that decide when a settler thinks at all.
//! * [`Settlement`] — a whole village of the above, run headless, reporting
//!   how much its settlers dithered.
//!
//! The one asymmetry worth pointing at, because everything the evening does
//! falls out of it: common work is multiplied by light and own business is
//! not. Nothing in this module knows what dusk is, and yet a settler carrying
//! logs to a stake will put them down and go sit by their own fire as the sun
//! goes.
//!
//! Two things are deliberately absent. There is no randomness: the staggered
//! recompute is `(entity index + tick) % period`, so a thousand minds spread
//! themselves over the period and a replay reproduces them exactly
//! (`08-scale.md`). And there are no constants outside [`MindTuning`] — if a
//! number decides what a settler does, it is a field of that one struct, and
//! a headless run can be handed a different one.
//!
//! ```
//! use runity_core::mind::{Activity, AtHand, Mind, MindTuning, Situation, WorkCandidate};
//! use runity_core::needs::Needs;
//! use runity_core::World;
//!
//! let mut world = World::new();
//! let settler = world.spawn();
//! let stake = world.spawn();
//!
//! let tuning = MindTuning::default();
//! let work = [WorkCandidate::new(stake, 0.7, 6.0).by(0.4, 0.9)];
//! let hand = AtHand::default();
//! let needs = Needs::default();
//! let situation = |light| Situation {
//!     settler,
//!     tick: 0,
//!     needs,
//!     light,
//!     work: &work,
//!     at_hand: &hand,
//! };
//!
//! // At noon the stake is worth more than sitting about.
//! let mut noon = Mind::new();
//! assert_eq!(
//!     noon.update(&situation(tuning.light_at_hour(12.0)), &tuning),
//!     Activity::Work { target: stake },
//! );
//!
//! // Deep in the night the same stake, unchanged, is not.
//! let mut night = Mind::new();
//! assert_eq!(
//!     night.update(&situation(tuning.light_at_hour(0.0)), &tuning),
//!     Activity::OwnBusiness,
//! );
//! ```

mod decide;
mod score;
mod settlement;
mod tuning;
mod work;

pub use decide::{staggered_recompute, Mind, Trigger};
pub use score::{candidates, need_urgency, work_urgency, Activity, Scored, Situation};
pub use settlement::{Chore, Settlement, SettlerSpec, SwitchLog, SwitchReport};
pub use tuning::{hour_of_day, MindTuning};
pub use work::{AtHand, OneStep, WorkCandidate};
