//! The simulation's own clock.
//!
//! `runity_core::Time` measures frames and wall-clock seconds for
//! presentation; `Tick` measures simulation steps, and nothing in this crate
//! reads `Time`. A `Tick` only advances when [`crate::Simulation::tick`] is
//! called.

use std::ops::{Add, AddAssign, Sub};

/// One step of the simulation, counted from zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tick(pub u64);

impl Tick {
    /// The first tick.
    pub const ZERO: Tick = Tick(0);

    /// The tick after this one.
    #[inline]
    pub fn next(self) -> Tick {
        Tick(self.0 + 1)
    }
}

impl Add<u64> for Tick {
    type Output = Tick;
    #[inline]
    fn add(self, rhs: u64) -> Tick {
        Tick(self.0 + rhs)
    }
}

impl AddAssign<u64> for Tick {
    #[inline]
    fn add_assign(&mut self, rhs: u64) {
        self.0 += rhs;
    }
}

impl Sub<u64> for Tick {
    type Output = Tick;
    #[inline]
    fn sub(self, rhs: u64) -> Tick {
        Tick(self.0 - rhs)
    }
}

/// The number of ticks between two points in time: `later - earlier`.
impl Sub<Tick> for Tick {
    type Output = u64;
    #[inline]
    fn sub(self, rhs: Tick) -> u64 {
        self.0 - rhs.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_advances_by_one() {
        assert_eq!(Tick::ZERO.next(), Tick(1));
        assert_eq!(Tick(41).next(), Tick(42));
    }

    #[test]
    fn arithmetic_with_u64() {
        assert_eq!(Tick(10) + 5, Tick(15));
        assert_eq!(Tick(10) - 5, Tick(5));

        let mut t = Tick(10);
        t += 5;
        assert_eq!(t, Tick(15));
    }

    #[test]
    fn difference_between_ticks_is_a_tick_count() {
        assert_eq!(Tick(10) - Tick(3), 7);
    }

    #[test]
    fn ticks_are_distinct_from_time() {
        // Tick is not Time, and carries none of its wall-clock fields.
        assert_eq!(std::mem::size_of::<Tick>(), std::mem::size_of::<u64>());
    }

    #[test]
    fn ticks_order_like_their_inner_value() {
        assert!(Tick(1) < Tick(2));
        assert!(Tick(5) > Tick(2));
    }
}
