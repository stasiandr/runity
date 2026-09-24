//! Food in the air, on the host: the physics carries it (the item is a
//! dynamic body while it flies). Passing low over a pot with room it drops
//! in, if chopped; over the bin, it is gone. Once it lies still it lands —
//! onto a counter with nothing on it, or on the floor, where a cook can
//! pick it up again.

use runity::hecs::{Entity, World};
use runity::Transform;

use crate::components::item::Thing;
use crate::components::station::Kind;
use crate::components::{Item, Station};
use crate::state::*;

/// Seconds of lying still that make a landing.
const SETTLE: f32 = 0.2;

pub fn run(world: &mut World, seconds: f32) {
    let Some((_, true)) = kitchen(world) else {
        return;
    };
    let mut landed = Vec::new();
    for (item, t, flying) in world.query_mut::<(Entity, &Transform, &mut Flying)>() {
        if flying.launch.is_some() {
            continue;
        }
        flying.age += seconds;
        if (t.position - flying.last).length() < 0.004 {
            flying.still += seconds;
        } else {
            flying.still = 0.0;
        }
        flying.last = t.position;
        if (flying.still > SETTLE && flying.age > 0.3) || t.position.y < -2.0 || flying.age > 6.0 {
            landed.push(item);
        }
    }
    for item in landed {
        land(world, item);
    }
    // Caught in the air by a pot or the bin.
    let flying: Vec<(Entity, runity::glam::Vec3)> = world
        .query::<(Entity, &Transform, &Flying)>()
        .iter()
        .filter(|(_, _, f)| f.launch.is_none())
        .map(|(e, t, _)| (e, t.position))
        .collect();
    for (item, at) in flying {
        let low_over = station_under(world, at)
            .filter(|(_, kind)| at.y < 1.8 && matches!(kind, Kind::Stove | Kind::Bin));
        if let Some((station, kind)) = low_over {
            catch(world, item, station, kind);
        }
    }
}

/// The station whose tile `at` is over.
fn station_under(world: &World, at: runity::glam::Vec3) -> Option<(Entity, Kind)> {
    world
        .query::<(Entity, &Transform, &Station)>()
        .iter()
        .filter(|(_, t, _)| (t.position.x - at.x).abs() < 0.5 && (t.position.z - at.z).abs() < 0.5)
        .map(|(e, _, s)| (e, s.kind))
        .next()
}

/// Into the pot (chopped, with room) or the bin: `true` if it went.
fn catch(world: &mut World, item: Entity, station: Entity, kind: Kind) -> bool {
    let food = match world.get::<&Item>(item).map(|i| i.thing) {
        Ok(Thing::Food(food)) => food,
        _ => return false,
    };
    match kind {
        Kind::Bin => {
            despawn_tree(world, item);
            true
        }
        Kind::Stove => {
            let chopped = world.get::<&Chop>(item).is_ok_and(|c| c.0 >= 1.0);
            if world.get::<&Pot>(station).is_err() {
                let _ = world.insert_one(station, Pot::default());
            }
            let room = world
                .get::<&Pot>(station)
                .is_ok_and(|p| p.foods.len() < POT_HOLDS && !p.burnt());
            if !(chopped && room) {
                return false;
            }
            if let Ok(mut pot) = world.get::<&mut Pot>(station) {
                pot.foods.push(food);
            }
            despawn_tree(world, item);
            true
        }
        _ => false,
    }
}

/// Where it came down, what becomes of it.
fn land(world: &mut World, item: Entity) {
    let _ = world.remove::<(runity::world::Physics, runity::world::Shape, Flying)>(item);
    let Ok(at) = world.get::<&Transform>(item).map(|t| t.position) else {
        return;
    };
    if at.y < -1.0 {
        despawn_tree(world, item);
        return;
    }
    // Upright again, whatever the tumble.
    if let Ok(mut t) = world.get::<&mut Transform>(item) {
        t.rotation_deg = Default::default();
    }
    let Some((station, kind)) = station_under(world, at).filter(|_| at.y > 0.8) else {
        return; // On the floor.
    };
    if catch(world, item, station, kind) {
        return;
    }
    let top = world.get::<&Top>(station).ok().and_then(|t| t.0);
    match kind {
        Kind::Counter | Kind::Board | Kind::Crate(_) if top.is_none() => {
            let _ = world.insert_one(station, Top(Some(item)));
            let _ = world.insert_one(item, At(Place::On(station)));
        }
        _ => {}
    }
}
