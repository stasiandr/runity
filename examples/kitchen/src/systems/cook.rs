//! A full pot cooks: soup after a while, burnt after a while longer. The
//! host's; the others see the pot as it sends it.

use runity::hecs::World;

use crate::state::{kitchen, Pot, Round, POT_HOLDS};

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
}
