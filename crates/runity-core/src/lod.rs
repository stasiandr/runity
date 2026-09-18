//! Simulating a world larger than a machine can afford.
//!
//! The premise of this engine's game is that the settlement keeps developing
//! whether or not anyone is watching. Taken literally that means simulating
//! every villager everywhere, forever, which stops being affordable at about
//! the point the game gets interesting.
//!
//! The way out is not to stop simulating the distant parts, but to simulate
//! them *coarsely*: a region nobody is near ticks rarely, and when it does it
//! is told how long it has been — so instead of a hundred villagers each
//! deciding what to do, one calculation says "in the last five minutes this
//! village gathered roughly this much wood and grew by one". A player walking
//! back finds a village that has changed in the way it should have, having
//! cost almost nothing while they were away.
//!
//! Getting this right is a simulation design problem more than an engine one;
//! what the engine owes it is the bookkeeping. Which regions are near an
//! observer, which are due a tick, and — the part that is easy to get wrong —
//! exactly how many ticks each one has missed, counted once and only once.

use std::collections::HashMap;

use runity_math::Vec3;

/// How closely a region is being simulated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Detail {
    /// Nobody is near: not simulated at all, only accumulating missed ticks.
    Dormant,
    /// Simulated occasionally, in aggregate.
    Coarse,
    /// Simulated every tick, agent by agent.
    Full,
}

/// A square of the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Region {
    /// Column, in cells from the origin.
    pub x: i32,
    /// Row.
    pub z: i32,
}

/// A region that is due a tick, and how much time it owes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Due {
    /// Which region.
    pub region: Region,
    /// How closely it should be simulated.
    pub detail: Detail,
    /// Ticks since it was last simulated. One for a region keeping up; a
    /// great many for one that has just woken.
    pub elapsed: u64,
}

#[derive(Clone, Copy, Debug)]
struct State {
    detail: Detail,
    /// The tick it was last simulated on.
    ticked: u64,
    /// The tick an observer was last near it.
    observed: u64,
}

/// How often each level of detail is simulated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cadence {
    /// Ticks between simulations of a full-detail region. Normally one.
    pub full: u64,
    /// Ticks between simulations of a coarse one.
    pub coarse: u64,
    /// Ticks between simulations of a dormant one. Zero means never, until
    /// somebody comes near.
    pub dormant: u64,
}

impl Default for Cadence {
    fn default() -> Self {
        // At ten ticks a second: full every tick, coarse every ten seconds,
        // dormant only when woken.
        Self {
            full: 1,
            coarse: 100,
            dormant: 0,
        }
    }
}

/// Which parts of the world are being simulated how closely.
#[derive(Debug)]
pub struct Regions {
    cell_size: f32,
    states: HashMap<Region, State>,
    /// Distance within which a region is simulated in full.
    pub full_radius: f32,
    /// Distance within which it is simulated coarsely.
    pub coarse_radius: f32,
    /// How often each level is simulated.
    pub cadence: Cadence,
}

impl Regions {
    /// A world divided into squares `cell_size` across.
    ///
    /// Big enough that a player does not cross one every few seconds, small
    /// enough that a full-detail area is not most of the map: a few hundred
    /// metres suits a settlement.
    pub fn new(cell_size: f32) -> Self {
        Self {
            cell_size: if cell_size > 0.0 { cell_size } else { 1.0 },
            states: HashMap::new(),
            full_radius: 120.0,
            coarse_radius: 600.0,
            cadence: Cadence::default(),
        }
    }

    /// The region a position falls in.
    pub fn region_at(&self, position: Vec3) -> Region {
        Region {
            x: (position.x / self.cell_size).floor() as i32,
            z: (position.z / self.cell_size).floor() as i32,
        }
    }

    /// The centre of a region, in world units.
    pub fn centre_of(&self, region: Region) -> Vec3 {
        Vec3::new(
            (region.x as f32 + 0.5) * self.cell_size,
            0.0,
            (region.z as f32 + 0.5) * self.cell_size,
        )
    }

    /// How closely a region is simulated.
    pub fn detail(&self, region: Region) -> Detail {
        self.states
            .get(&region)
            .map_or(Detail::Dormant, |state| state.detail)
    }

    /// How closely the region containing a position is simulated.
    pub fn detail_at(&self, position: Vec3) -> Detail {
        self.detail(self.region_at(position))
    }

    /// How many regions are being tracked.
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// Whether nothing is being tracked.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// Set the level of detail from where the observers are.
    ///
    /// Call once per world tick with every player's position — and every
    /// other reason a region should stay awake: a caravan in transit, a fire
    /// burning, a siege. A region nobody mentions falls back to dormant.
    pub fn observe(&mut self, observers: &[Vec3], tick: u64) {
        // Everything starts the tick dormant, and is raised by whatever is
        // near it. Downgrading only on absence is what makes a region that has
        // just been left fall quiet without any explicit "forget" call.
        for state in self.states.values_mut() {
            state.detail = Detail::Dormant;
        }

        for observer in observers {
            let centre = self.region_at(*observer);
            let reach = (self.coarse_radius / self.cell_size).ceil() as i32 + 1;
            for dz in -reach..=reach {
                for dx in -reach..=reach {
                    let region = Region {
                        x: centre.x + dx,
                        z: centre.z + dz,
                    };
                    let distance = ground_distance(self.centre_of(region), *observer);
                    let detail = if distance <= self.full_radius {
                        Detail::Full
                    } else if distance <= self.coarse_radius {
                        Detail::Coarse
                    } else {
                        continue;
                    };
                    let entry = self.states.entry(region).or_insert(State {
                        detail,
                        // A region first seen now has missed nothing: the
                        // alternative is a brand-new region owing the whole
                        // history of the world.
                        ticked: tick,
                        observed: tick,
                    });
                    entry.detail = entry.detail.max(detail);
                    entry.observed = tick;
                }
            }
        }
    }

    /// Which regions are due a tick now, and how much time each owes.
    ///
    /// Comes back in a fixed order, because the order regions are simulated in
    /// changes their results the moment they interact at all.
    pub fn due(&self, tick: u64) -> Vec<Due> {
        let mut due: Vec<Due> = self
            .states
            .iter()
            .filter_map(|(region, state)| {
                let interval = match state.detail {
                    Detail::Full => self.cadence.full,
                    Detail::Coarse => self.cadence.coarse,
                    Detail::Dormant => self.cadence.dormant,
                };
                if interval == 0 {
                    return None;
                }
                let elapsed = tick.saturating_sub(state.ticked);
                (elapsed >= interval).then_some(Due {
                    region: *region,
                    detail: state.detail,
                    elapsed,
                })
            })
            .collect();
        due.sort_by_key(|entry| (entry.region.x, entry.region.z));
        due
    }

    /// Record that a region has been simulated up to `tick`.
    ///
    /// Must be called for everything [`Regions::due`] returned, or the same
    /// ticks are owed again next time — which is how a coarse simulation ends
    /// up applying a week's growth every ten seconds.
    pub fn mark_simulated(&mut self, region: Region, tick: u64) {
        if let Some(state) = self.states.get_mut(&region) {
            state.ticked = tick;
        }
    }

    /// Wake a region and take everything it owes, in one go.
    ///
    /// This is the moment a player walks back into a village that has been
    /// asleep for an hour: the caller applies the aggregate of those ticks,
    /// once.
    pub fn wake(&mut self, region: Region, tick: u64) -> u64 {
        match self.states.get_mut(&region) {
            Some(state) => {
                let elapsed = tick.saturating_sub(state.ticked);
                state.ticked = tick;
                state.detail = state.detail.max(Detail::Coarse);
                elapsed
            }
            None => {
                // Never tracked, so nothing is owed — a region begins the
                // moment it is first seen.
                self.states.insert(
                    region,
                    State {
                        detail: Detail::Coarse,
                        ticked: tick,
                        observed: tick,
                    },
                );
                0
            }
        }
    }

    /// Forget regions nobody has been near since `tick`.
    ///
    /// Without this the map grows by one entry per square ever visited, which
    /// for a world of any size is a slow leak.
    pub fn forget_unobserved_before(&mut self, tick: u64) -> usize {
        let before = self.states.len();
        self.states.retain(|_, state| state.observed >= tick);
        before - self.states.len()
    }

    /// Every tracked region and its detail, in a fixed order.
    pub fn tracked(&self) -> Vec<(Region, Detail)> {
        let mut all: Vec<(Region, Detail)> = self
            .states
            .iter()
            .map(|(region, state)| (*region, state.detail))
            .collect();
        all.sort_by_key(|(region, _)| (region.x, region.z));
        all
    }
}

/// Distance on the ground plane.
fn ground_distance(a: Vec3, b: Vec3) -> f32 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::vec3;

    fn regions() -> Regions {
        let mut regions = Regions::new(100.0);
        regions.full_radius = 150.0;
        regions.coarse_radius = 500.0;
        regions
    }

    #[test]
    fn detail_follows_the_observer() {
        let mut regions = regions();
        regions.observe(&[Vec3::ZERO], 1);

        assert_eq!(regions.detail_at(vec3(10.0, 0.0, 10.0)), Detail::Full);
        assert_eq!(regions.detail_at(vec3(300.0, 0.0, 0.0)), Detail::Coarse);
        assert_eq!(
            regions.detail_at(vec3(5_000.0, 0.0, 0.0)),
            Detail::Dormant,
            "the far side"
        );
    }

    #[test]
    fn a_region_the_observer_leaves_goes_quiet() {
        let mut regions = regions();
        let home = vec3(0.0, 0.0, 0.0);
        regions.observe(&[home], 1);
        assert_eq!(regions.detail_at(home), Detail::Full);

        // The player walks a long way off.
        regions.observe(&[vec3(4_000.0, 0.0, 0.0)], 2);
        assert_eq!(
            regions.detail_at(home),
            Detail::Dormant,
            "nobody is near it now"
        );
        assert_eq!(regions.detail_at(vec3(4_000.0, 0.0, 0.0)), Detail::Full);
    }

    #[test]
    fn two_observers_both_count_and_the_closer_one_wins() {
        let mut regions = regions();
        let middle = vec3(300.0, 0.0, 0.0);
        regions.observe(&[Vec3::ZERO], 1);
        assert_eq!(
            regions.detail_at(middle),
            Detail::Coarse,
            "only distantly watched"
        );

        // A second player standing right there raises it.
        regions.observe(&[Vec3::ZERO, middle], 2);
        assert_eq!(regions.detail_at(middle), Detail::Full);
        assert_eq!(
            regions.detail_at(Vec3::ZERO),
            Detail::Full,
            "and the first is unaffected"
        );
    }

    #[test]
    fn full_regions_tick_every_tick_and_coarse_ones_rarely() {
        let mut regions = regions();
        regions.cadence = Cadence {
            full: 1,
            coarse: 10,
            dormant: 0,
        };
        regions.observe(&[Vec3::ZERO], 0);

        let near = regions.region_at(Vec3::ZERO);
        let far = regions.region_at(vec3(300.0, 0.0, 0.0));

        // One tick later the full region is due and the coarse one is not.
        let due = regions.due(1);
        assert!(due.iter().any(|entry| entry.region == near));
        assert!(!due.iter().any(|entry| entry.region == far));

        // Ten ticks later, both are.
        let due = regions.due(10);
        assert!(due
            .iter()
            .any(|entry| entry.region == far && entry.elapsed == 10));
    }

    #[test]
    fn a_dormant_region_is_never_due_until_it_is_woken() {
        let mut regions = regions();
        regions.observe(&[Vec3::ZERO], 0);
        let far = regions.region_at(vec3(300.0, 0.0, 0.0));

        // Everyone leaves.
        regions.observe(&[vec3(9_000.0, 0.0, 0.0)], 1);
        assert_eq!(regions.detail(far), Detail::Dormant);
        assert!(regions.due(10_000).iter().all(|entry| entry.region != far));

        // Somebody comes back, and the whole gap is owed exactly once.
        let owed = regions.wake(far, 10_000);
        assert_eq!(
            owed, 10_000,
            "the village has had ten thousand ticks to itself"
        );
        assert_eq!(regions.wake(far, 10_000), 0, "and does not owe them twice");
    }

    #[test]
    fn simulated_ticks_are_counted_once() {
        // The bug this guards against: a coarse region that is never marked
        // applies its accumulated growth again on every pass.
        let mut regions = regions();
        regions.cadence = Cadence {
            full: 1,
            coarse: 10,
            dormant: 0,
        };
        regions.observe(&[vec3(300.0, 0.0, 0.0)], 0);
        let region = regions.region_at(vec3(300.0, 0.0, 0.0));

        let mut applied = 0;
        for tick in 1..=100 {
            for entry in regions.due(tick) {
                applied += entry.elapsed;
                regions.mark_simulated(entry.region, tick);
            }
        }
        // Whatever the cadence, the total time simulated is the time passed.
        let ticks_for_region: u64 = applied;
        assert!(
            (99..=101).contains(&(ticks_for_region / regions.tracked().len() as u64)),
            "simulated {ticks_for_region} ticks across {} regions in 100",
            regions.tracked().len()
        );
        assert_eq!(
            regions.due(100).iter().find(|e| e.region == region),
            None,
            "nothing outstanding"
        );
    }

    #[test]
    fn a_new_region_owes_nothing() {
        // A region first seen at tick ten thousand must not owe ten thousand
        // ticks of history it never had.
        let mut regions = regions();
        regions.observe(&[Vec3::ZERO], 10_000);
        let due = regions.due(10_000);
        assert!(due.iter().all(|entry| entry.elapsed == 0), "{due:?}");
    }

    #[test]
    fn regions_come_back_in_a_fixed_order() {
        let mut regions = regions();
        regions.observe(&[Vec3::ZERO], 0);
        let first = regions.due(50);
        for _ in 0..5 {
            assert_eq!(regions.due(50), first, "a HashMap's order is not an order");
        }
        assert!(first.len() > 1);
        assert!(first.windows(2).all(|pair| {
            (pair[0].region.x, pair[0].region.z) <= (pair[1].region.x, pair[1].region.z)
        }));
    }

    #[test]
    fn forgetting_keeps_the_map_from_growing_forever() {
        let mut regions = regions();
        // A player walks a long way, touching a great many regions.
        for step in 0..20 {
            regions.observe(&[vec3(step as f32 * 400.0, 0.0, 0.0)], step as u64);
        }
        let tracked = regions.len();
        assert!(
            tracked > 50,
            "a long walk touches a lot of ground: {tracked}"
        );

        let forgotten = regions.forget_unobserved_before(19);
        assert!(forgotten > 0);
        assert!(regions.len() < tracked);
        // What is still under the player is kept.
        assert_eq!(
            regions.detail_at(vec3(19.0 * 400.0, 0.0, 0.0)),
            Detail::Full
        );
    }

    #[test]
    fn positions_and_regions_agree_with_each_other() {
        let regions = Regions::new(50.0);
        assert_eq!(
            regions.region_at(vec3(10.0, 0.0, 10.0)),
            Region { x: 0, z: 0 }
        );
        assert_eq!(
            regions.region_at(vec3(-10.0, 0.0, -10.0)),
            Region { x: -1, z: -1 }
        );
        for region in [
            Region { x: 0, z: 0 },
            Region { x: 7, z: -3 },
            Region { x: -12, z: 40 },
        ] {
            assert_eq!(regions.region_at(regions.centre_of(region)), region);
        }
    }

    #[test]
    fn a_world_left_alone_costs_almost_nothing() {
        // The whole point: a hundred regions, nobody watching, and the
        // simulation does no work at all until somebody returns.
        let mut regions = regions();
        for step in 0..10 {
            regions.observe(&[vec3(step as f32 * 300.0, 0.0, 0.0)], step as u64);
        }
        regions.observe(&[], 100); // everyone logs off

        let mut work = 0;
        for tick in 100..10_000 {
            work += regions.due(tick).len();
        }
        assert_eq!(work, 0, "an unwatched world should not be simulated at all");

        // And when a player returns, the time is still there to be applied.
        let village = regions.region_at(vec3(600.0, 0.0, 0.0));
        assert!(regions.wake(village, 10_000) > 9_000);
    }
}
