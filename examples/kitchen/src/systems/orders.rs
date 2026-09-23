//! The round, on the host: orders come in and wait, the clock runs down,
//! and at the end the kitchen stops until someone asks to start again —
//! which clears it.

use runity::hecs::{Entity, World};

use crate::components::item::Food;
use crate::components::Kitchen;
use crate::state::*;

/// Orders waiting at once, at most.
pub const MOST_ORDERS: usize = 4;

pub fn run(world: &mut World, seconds: f32) {
    let Some((kitchen, true)) = kitchen(world) else {
        return;
    };
    let Ok(rules) = world.get::<&Kitchen>(kitchen).map(|k| *k) else {
        return;
    };
    if world.get::<&Round>(kitchen).is_err() {
        start(world, kitchen, &rules);
    }
    if world.remove_one::<Restart>(kitchen).is_ok() {
        clear(world);
        start(world, kitchen, &rules);
        return;
    }
    let Ok((round, service)) = world.query_one_mut::<(&mut Round, &mut Service)>(kitchen) else {
        return;
    };
    if let Some((_, left)) = &mut round.note {
        *left -= seconds;
        if *left <= 0.0 {
            round.note = None;
        }
    }
    if round.over {
        return;
    }
    round.time_left -= seconds;
    if round.time_left <= 0.0 {
        round.time_left = 0.0;
        round.over = true;
        return;
    }
    service.next_order -= seconds;
    if service.next_order <= 0.0 && round.orders.len() < MOST_ORDERS {
        service.next_order = rules.order_every;
        // A little generator of its own: every round asks the same.
        service.seed = service.seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let food = if (service.seed >> 16) % 2 == 0 { Food::Tomato } else { Food::Onion };
        round.orders.push(Order {
            food,
            left: rules.order_seconds,
            total: rules.order_seconds,
        });
    }
    let mut lost = 0;
    for order in &mut round.orders {
        order.left -= seconds;
        if order.left <= 0.0 {
            lost += 1;
        }
    }
    if lost > 0 {
        round.orders.retain(|o| o.left > 0.0);
        round.score -= 10 * lost;
        round.say("An order walked out: −10");
    }
}

fn start(world: &mut World, kitchen: Entity, rules: &Kitchen) {
    let _ = world.insert(
        kitchen,
        (
            Round {
                time_left: rules.round_seconds,
                ..Default::default()
            },
            Service {
                next_order: 1.0,
                seed: 7,
                ..Default::default()
            },
        ),
    );
}

/// Everything a round left: food and plates, what is in the pots and on
/// the counters, what the cooks hold.
fn clear(world: &mut World) {
    let items: Vec<Entity> = world
        .query::<(Entity, &crate::components::Item)>()
        .iter()
        .map(|(e, _)| e)
        .collect();
    for item in items {
        despawn_tree(world, item);
    }
    for top in world.query_mut::<&mut Top>() {
        top.0 = None;
    }
    for pot in world.query_mut::<&mut Pot>() {
        *pot = Pot::default();
    }
    for hands in world.query_mut::<&mut Hands>() {
        hands.0 = None;
    }
}
