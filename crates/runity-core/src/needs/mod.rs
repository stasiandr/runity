//! Settler needs and the settlement's day-log of deficits.
//!
//! Two things live here, and this module builds only their data and
//! bookkeeping — not the minds that read them:
//!
//! * [`Needs`] — hunger, warmth and rest on a single settler, decaying on
//!   their own and relieved only by a direct call ([`Needs::eat`],
//!   [`Needs::warm`], [`Needs::sleep`]), independent of any AI.
//! * [`DeficitLog`] — the settlement-wide record of four deficits actually
//!   lived through: nights spent cold or unsheltered, and days where eating
//!   found no food or a marked item aged into forgetting. Every entry reads
//!   `None` until the event it measures has happened at least once —
//!   undefined, not a defaulted `0.0` — per `12-minds.md` §2.1.
//! * [`Hurt`] — the same four deficits counted the other way round, per
//!   settler rather than per settlement. The log says how much of the
//!   village went cold; this says which of them it was, which is the half
//!   `12-minds.md` §2.4 needs to pick who plants a stake.
//!
//! [`Hearth`] and [`SleepingSpot`] are the two things a settler's needs are
//! relieved against, and [`tick_settlement`] is the one system that ties all
//! of it to a [`crate::World`]: it decays every settler's needs, relieves
//! warmth and rest for anyone within a hearth's or a sleeping spot's radius,
//! and keeps the log current.
//!
//! ```
//! use runity_core::needs::{DeficitLog, Needs, NightWatch, tick_settlement};
//! use runity_core::World;
//!
//! let mut world = World::new();
//! for _ in 0..3 {
//!     let settler = world.spawn();
//!     world.insert(settler, Needs::default());
//!     world.insert(settler, NightWatch::default());
//! }
//!
//! // No hearth, no sleeping spot: a full sim-day of ticks and dawn finds
//! // every settler cold and unsheltered.
//! let ticks_per_day = 20;
//! let mut log = DeficitLog::new(ticks_per_day);
//! for tick in 1..ticks_per_day {
//!     tick_settlement(&mut world, &mut log, tick, 1.0);
//!     assert_eq!(log.cold_last_night(), None, "the first night is not over yet");
//! }
//! tick_settlement(&mut world, &mut log, ticks_per_day, 1.0);
//! assert_eq!(log.cold_last_night(), Some(1.0));
//! assert_eq!(log.unsheltered_last_night(), Some(1.0));
//! ```

mod environment;
mod log;
mod settler;
mod system;

pub use environment::{Hearth, SleepingSpot};
pub use log::{DeficitLog, Hurt};
pub use settler::{NeedKind, Needs, NeedsRates};
pub use system::{tick_settlement, NightWatch};
