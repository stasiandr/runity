//! The bookkeeping that ties [`super::Needs`] and [`super::DeficitLog`] to a
//! [`World`]: decay, radius checks against [`super::Hearth`] and
//! [`super::SleepingSpot`], and log rollover.

use super::environment::{Hearth, SleepingSpot};
use super::log::DeficitLog;
use super::settler::Needs;
use crate::transform::Transform;
use crate::world::World;
use runity_math::Vec3;

/// Per-settler bookkeeping for the two night-bound deficits: whether this
/// settler has been warmed or sheltered at all so far tonight. Attach it
/// alongside [`Needs`] on any settler that should count towards
/// [`DeficitLog::cold_last_night`] and [`DeficitLog::unsheltered_last_night`].
///
/// [`tick_settlement`] only ever sets these flags to `true`; they are reset to
/// `false` for everyone the moment dawn rolls the log over, so "spent the
/// night" means the whole night, not a single lucky tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NightWatch {
    warmed: bool,
    sheltered: bool,
}

/// Decay every settler's needs by `dt` seconds, relieve warmth and rest for
/// anyone within a [`Hearth`] or [`SleepingSpot`]'s radius, and keep `log`
/// current: prune its day-bound windows, and — once `current_tick` reaches
/// the next dawn — roll the two night-bound entries over.
///
/// This is purely mechanical, the same way gravity does not ask a settler's
/// permission before pulling them down: whether a settler is near a hearth or
/// a sleeping spot is a fact about the world, not a decision a mind makes.
/// Eating is not, which is why it has no place here — see [`Needs::eat`].
///
/// `current_tick` is meant to be the cumulative tick count (for example
/// `Engine::time`'s `elapsed_ticks()`), not a per-call index: the log's dawn
/// schedule is expressed against it directly.
pub fn tick_settlement(world: &mut World, log: &mut DeficitLog, current_tick: u64, dt: f32) {
    let hearths = positioned_radii::<Hearth>(world, |h| h.radius);
    let spots = positioned_radii::<SleepingSpot>(world, |s| s.radius);

    for entity in world.entities_with::<Needs>() {
        let position = world
            .get::<Transform>(entity)
            .map(|t| t.position)
            .unwrap_or(Vec3::ZERO);
        let warmed = near_any(&hearths, position);
        let sheltered = near_any(&spots, position);

        if let Some(needs) = world.get_mut::<Needs>(entity) {
            needs.decay(dt);
            if warmed {
                needs.warm(dt);
            }
            if sheltered {
                needs.sleep(dt);
            }
        }
        if let Some(watch) = world.get_mut::<NightWatch>(entity) {
            watch.warmed |= warmed;
            watch.sheltered |= sheltered;
        }
    }

    log.advance(current_tick);

    if log.dawn_due(current_tick) {
        let watches: Vec<NightWatch> = world.iter::<NightWatch>().map(|(_, w)| *w).collect();
        if !watches.is_empty() {
            let population = watches.len() as f32;
            let cold = watches.iter().filter(|w| !w.warmed).count() as f32;
            let unsheltered = watches.iter().filter(|w| !w.sheltered).count() as f32;
            log.record_night(cold / population, unsheltered / population, current_tick);
            for (_, watch) in world.iter_mut::<NightWatch>() {
                *watch = NightWatch::default();
            }
        }
    }
}

/// Every entity carrying a `T`, paired with its position and a radius read
/// out of `T` — `T` has no position of its own, so this looks it up on the
/// same entity's [`Transform`], exactly like [`crate::physics::Blocker`].
fn positioned_radii<T: 'static>(world: &World, radius_of: impl Fn(&T) -> f32) -> Vec<(Vec3, f32)> {
    world
        .iter::<T>()
        .filter_map(|(entity, component)| {
            world
                .get::<Transform>(entity)
                .map(|t| (t.position, radius_of(component)))
        })
        .collect()
}

fn near_any(sources: &[(Vec3, f32)], point: Vec3) -> bool {
    sources
        .iter()
        .any(|(source, radius)| (point - *source).length_squared() <= radius * radius)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Entity;

    fn settler_with_watch(world: &mut World) -> Entity {
        let entity = world.spawn();
        world.insert(entity, Needs::default());
        world.insert(entity, NightWatch::default());
        entity
    }

    #[test]
    fn needs_decay_every_tick_with_no_hearth_or_spot_required() {
        let mut world = World::new();
        let settler = settler_with_watch(&mut world);
        let mut log = DeficitLog::new(1000);

        tick_settlement(&mut world, &mut log, 1, 10.0);

        let needs = world.get::<Needs>(settler).unwrap();
        assert!(needs.hunger < 1.0);
        assert!(needs.warmth < 1.0);
        assert!(needs.rest < 1.0);
    }

    #[test]
    fn no_hearth_or_sleeping_spot_leaves_everyone_cold_and_unsheltered_after_one_night() {
        let mut world = World::new();
        for _ in 0..4 {
            settler_with_watch(&mut world);
        }
        let ticks_per_day = 20;
        let mut log = DeficitLog::new(ticks_per_day);

        for tick in 1..ticks_per_day {
            tick_settlement(&mut world, &mut log, tick, 1.0);
            assert_eq!(log.cold_last_night(), None, "night {tick} is not over");
            assert_eq!(log.unsheltered_last_night(), None);
        }

        tick_settlement(&mut world, &mut log, ticks_per_day, 1.0);
        assert_eq!(log.cold_last_night(), Some(1.0));
        assert_eq!(log.unsheltered_last_night(), Some(1.0));
    }

    #[test]
    fn a_hearth_and_a_sleeping_spot_in_range_clear_both_night_deficits() {
        let mut world = World::new();
        for _ in 0..3 {
            settler_with_watch(&mut world);
            // Everyone defaults to the origin, so a hearth and a sleeping
            // spot there cover all of them.
        }
        let hearth = world.spawn();
        world.insert(hearth, Transform::from_position(Vec3::ZERO));
        world.insert(hearth, Hearth::new(5.0));

        let spot = world.spawn();
        world.insert(spot, Transform::from_position(Vec3::ZERO));
        world.insert(spot, SleepingSpot::new(5.0));

        let ticks_per_day = 10;
        let mut log = DeficitLog::new(ticks_per_day);
        for tick in 1..=ticks_per_day {
            tick_settlement(&mut world, &mut log, tick, 1.0);
        }

        assert_eq!(log.cold_last_night(), Some(0.0));
        assert_eq!(log.unsheltered_last_night(), Some(0.0));
    }

    #[test]
    fn only_settlers_outside_every_radius_count_towards_the_deficit() {
        let mut world = World::new();
        let warm_settler = settler_with_watch(&mut world);
        world.insert(warm_settler, Transform::from_position(Vec3::ZERO));
        let cold_settler = settler_with_watch(&mut world);
        world.insert(
            cold_settler,
            Transform::from_position(Vec3::new(100.0, 0.0, 0.0)),
        );

        let hearth = world.spawn();
        world.insert(hearth, Transform::from_position(Vec3::ZERO));
        world.insert(hearth, Hearth::new(2.0));

        let ticks_per_day = 5;
        let mut log = DeficitLog::new(ticks_per_day);
        for tick in 1..=ticks_per_day {
            tick_settlement(&mut world, &mut log, tick, 1.0);
        }

        assert_eq!(
            log.cold_last_night(),
            Some(0.5),
            "one of two was never warmed"
        );
    }

    #[test]
    fn night_watch_resets_at_dawn_so_a_new_night_starts_clean() {
        let mut world = World::new();
        settler_with_watch(&mut world);
        let ticks_per_day = 5;
        let mut log = DeficitLog::new(ticks_per_day);

        for tick in 1..=ticks_per_day {
            tick_settlement(&mut world, &mut log, tick, 1.0);
        }
        assert_eq!(log.cold_last_night(), Some(1.0));

        // Add a hearth mid-second-day: the deficit for the *next* dawn
        // reflects only what happens after the reset, not the first night's
        // exposure carrying over.
        let hearth = world.spawn();
        world.insert(hearth, Transform::from_position(Vec3::ZERO));
        world.insert(hearth, Hearth::new(5.0));

        for tick in (ticks_per_day + 1)..=(2 * ticks_per_day) {
            tick_settlement(&mut world, &mut log, tick, 1.0);
        }
        assert_eq!(log.cold_last_night(), Some(0.0));
    }
}
