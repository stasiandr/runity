//! Something happening later: Unity's `Invoke`, `InvokeRepeating` and a
//! coroutine's `WaitForSeconds`, as a component.
//!
//! A [`Timer`] on an entity counts down in the fixed step; [`tick_timers`]
//! is the system, and returns which went off, by entity and name, for the
//! game's own systems to act on. No callbacks: what happens when the bomb's
//! fuse runs out is a system reading that list, like everything else. It is
//! plain data, so a scene can give one (`"fuse": (seconds: 3.0)` once the
//! game registers it) and a save can keep it.

use serde::{Deserialize, Serialize};

/// A countdown on an entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timer {
    /// What the game calls it when it goes off.
    #[serde(default)]
    pub name: String,
    /// Seconds left.
    pub seconds: f32,
    /// Go again this long after going off; `None` goes once and is taken
    /// off the entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<f32>,
}

impl Timer {
    /// Once, in `seconds`.
    pub fn once(name: &str, seconds: f32) -> Self {
        Self {
            name: name.to_string(),
            seconds,
            every: None,
        }
    }

    /// First in `seconds`, then every `every`.
    pub fn repeating(name: &str, seconds: f32, every: f32) -> Self {
        Self {
            name: name.to_string(),
            seconds,
            every: Some(every.max(1e-3)),
        }
    }
}

/// Count every timer down by `dt`: the ones that went off, by entity and
/// name — as many times as they went off in the step, for a fast repeat.
/// Call it in the fixed step.
pub fn tick_timers(world: &mut hecs::World, dt: f32) -> Vec<(hecs::Entity, String)> {
    let mut fired = Vec::new();
    let mut done = Vec::new();
    for (entity, timer) in world.query_mut::<(hecs::Entity, &mut Timer)>() {
        timer.seconds -= dt;
        while timer.seconds <= 0.0 {
            fired.push((entity, timer.name.clone()));
            match timer.every {
                Some(every) => timer.seconds += every,
                None => {
                    done.push(entity);
                    break;
                }
            }
        }
    }
    for entity in done {
        crate::world::take_off::<Timer>(world, entity);
    }
    fired
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fuse_goes_once_and_a_beacon_keeps_going() {
        let mut world = hecs::World::new();
        let bomb = world.spawn((Timer::once("boom", 0.25),));
        let beacon = world.spawn((Timer::repeating("blink", 0.1, 0.1),));
        let mut heard = Vec::new();
        for _ in 0..6 {
            heard.extend(tick_timers(&mut world, 0.05));
        }
        let booms: Vec<_> = heard.iter().filter(|(e, _)| *e == bomb).collect();
        assert_eq!(booms.len(), 1);
        assert!(world.get::<&Timer>(bomb).is_err(), "gone once it went off");
        let blinks = heard
            .iter()
            .filter(|(e, n)| *e == beacon && n == "blink")
            .count();
        assert_eq!(blinks, 3, "at 0.1, 0.2 and 0.3");
        // A step longer than the repeat fires it for each time round.
        assert_eq!(tick_timers(&mut world, 0.35).len(), 3);
    }
}
