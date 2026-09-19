//! Stakes: the one assignment a settlement can have, its material progress
//! and shares.
//!
//! This is the minimal slice of `05-economy.md`'s share economy that
//! `12-minds.md`'s stakes need, not the whole thing:
//!
//! * [`Tag`] — a material property (`05-economy.md` §6.2). A stake names a
//!   tag and an amount, never a specific material.
//! * [`Stake`] — a settlement's one assignment for a single
//!   [`DeficitKind`]: per-contributor deposits, the share they buy
//!   (`05-economy.md` §3.2-3.4), and a two-sim-day abandonment clock that
//!   [`Stake::freeze`]/[`Stake::thaw`] can pause.
//! * [`StrikeBoard`] — the per-deficit-kind count of abandoned stakes: three
//!   abandonments of the same kind mark it un-treatable and refuse a fourth
//!   attempt to open one.
//!
//! Nothing here decides *when* a stake gets created or which deficit is
//! worth treating first — that is `12-minds.md`'s settlement mind, a later
//! card. This module only keeps the assignment once one exists.
//!
//! ```
//! use runity_core::economy::{DeficitKind, StrikeBoard, Tag};
//! use runity_core::World;
//!
//! let mut world = World::new();
//! let onega = world.spawn();
//!
//! let ticks_per_day = 100;
//! let board = StrikeBoard::new();
//! let mut stake = board
//!     .open_stake(DeficitKind::Warmth, Tag::Hard, 10, ticks_per_day, 0)
//!     .expect("Warmth has no strikes yet");
//!
//! stake.deposit(onega, 4, 0);
//! assert_eq!(stake.share_of(onega), Some((4, 4)));
//!
//! // Gone quiet for two full sim-days: the stake is pulled and Onega's
//! // four units come back to her.
//! let refund = stake.advance(2 * ticks_per_day).expect("two idle days");
//! assert_eq!(refund, vec![(onega, 4)]);
//! ```

mod stake;
mod strikes;
mod tag;

pub use stake::{DeficitKind, Refund, Stake};
pub use strikes::{StrikeBoard, Untreatable};
pub use tag::Tag;
