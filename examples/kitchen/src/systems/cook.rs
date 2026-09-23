//! Heat: a full pot cooks — soup after a while, burnt after a while longer —
//! and meat on a pan fries into a patty, then burns. The host's; the others
//! see the pots and the meat as it sends them.

use runity::hecs::{Entity, World};

use crate::components::station::Kind;
use crate::components::Station;
use crate::state::{kitchen, At, Fry, Place, Pot, Round, POT_HOLDS};

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
}
