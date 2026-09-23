//! Heat: a full pot cooks — soup after a while, burnt after a while longer,
//! on fire if left burning — and meat on a pan fries into a patty, burns,
//! catches fire. A fire costs points every second until it is put out
//! (scraped, hold F). The host's; the others see the pots and the meat as
//! it sends them.

use runity::hecs::{Entity, World};

use crate::components::station::Kind;
use crate::components::Station;
use crate::state::{kitchen, At, Fry, Place, Pot, Round, Service, POT_HOLDS};

/// Points a fire costs a second.
pub const FIRE_COST: i32 = 2;

pub fn run(world: &mut World, seconds: f32) {
    let Some((k, true)) = kitchen(world) else {
        return;
    };
    if world.get::<&Round>(k).map_or(true, |r| r.over) {
        return;
    }
    for pot in world.query_mut::<&mut Pot>() {
        if pot.foods.len() == POT_HOLDS {
            pot.cooked += seconds;
        }
    }
    let frying: Vec<Entity> = world
        .query::<(Entity, &At, &crate::components::Item)>()
        .iter()
        .filter(|(_, at, _)| match at.0 {
            Place::On(station) => world.get::<&Station>(station).is_ok_and(|s| s.kind == Kind::Pan),
            Place::Hand(_) => false,
        })
        .map(|(e, ..)| e)
        .collect();
    for meat in frying {
        let so_far = world.get::<&Fry>(meat).map_or(0.0, |f| f.0);
        let _ = world.insert_one(meat, Fry(so_far + seconds));
    }
    let fires = world.query::<&Pot>().iter().filter(|p| p.on_fire()).count()
        + world.query::<&Fry>().iter().filter(|f| f.on_fire()).count();
    if fires == 0 {
        return;
    }
    let Ok(mut service) = world.get::<&mut Service>(k) else {
        return;
    };
    let was = service.fire;
    service.fire += seconds * fires as f32;
    let whole = service.fire.floor() as i32 - was.floor() as i32;
    drop(service);
    if whole > 0 {
        if let Ok(mut round) = world.get::<&mut Round>(k) {
            round.score -= FIRE_COST * whole;
            round.say("Fire! Scrape it out (hold F)");
        }
    }
}
