//! The round, on the host: shut until the host opens the doors, then
//! orders come in and wait, the clock runs down, and at the end the kitchen
//! stops until the host asks to start again. Opening and starting again are
//! the same act, `Restart`: it clears the kitchen and opens it.

use runity::hecs::{Entity, World};

use crate::components::station::Kind;
use crate::components::{Kitchen, Station};
use crate::state::*;

/// Orders waiting at once, at most.
pub const MOST_ORDERS: usize = 4;

pub fn run(world: &mut World, seconds: f32) {
    let Some((kitchen, true)) = kitchen(world) else {
        return;
    };
    let Ok(rules) = world.get::<&Kitchen>(kitchen).map(|k| (*k).clone()) else {
        return;
    };
    if world.get::<&Round>(kitchen).is_err() {
        start(world, kitchen, &rules, false);
    }
    if world.remove_one::<Restart>(kitchen).is_ok() {
        clear(world);
        start(world, kitchen, &rules, true);
        return;
    }
    plates_back(world, kitchen, seconds);
    let Ok((round, service)) = world.query_one_mut::<(&mut Round, &mut Service)>(kitchen) else {
        return;
    };
    if let Some((_, left)) = &mut round.note {
        *left -= seconds;
        if *left <= 0.0 {
            round.note = None;
        }
    }
    if round.over || !round.open {
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
        let menu = &rules.menu;
        let Some(dish) = menu.get((service.seed >> 16) as usize % menu.len().max(1)).copied() else {
            return;
        };
        round.orders.push(Order {
            dish,
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

/// The plates that went out of the window, back in the sink dirty when
/// their time is up.
fn plates_back(world: &mut World, kitchen: Entity, seconds: f32) {
    let back = match world.get::<&mut Service>(kitchen) {
        Ok(mut service) => {
            service.returning.iter_mut().for_each(|t| *t -= seconds);
            let before = service.returning.len();
            service.returning.retain(|t| *t > 0.0);
            before - service.returning.len()
        }
        Err(_) => 0,
    };
    if back == 0 {
        return;
    }
    let sink = world
        .query::<(Entity, &Station)>()
        .iter()
        .find(|(_, s)| s.kind == Kind::Sink)
        .map(|(e, _)| e);
    if let Some(sink) = sink {
        let mut now = world.get::<&Sink>(sink).map(|s| *s).unwrap_or_default();
        now.dirty += back as u32;
        let _ = world.insert_one(sink, now);
    }
}

fn start(world: &mut World, kitchen: Entity, rules: &Kitchen, open: bool) {
    // Full racks, an empty sink.
    let stations: Vec<(Entity, Kind)> = world.query::<(Entity, &Station)>().iter().map(|(e, s)| (e, s.kind)).collect();
    for (station, kind) in stations {
        match kind {
            Kind::Plates => {
                let _ = world.insert_one(station, Stack(rules.plates));
            }
            Kind::Sink => {
                let _ = world.insert_one(station, Sink::default());
            }
            _ => {}
        }
    }
    let _ = world.insert(
        kitchen,
        (
            Round {
                time_left: rules.round_seconds,
                open,
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
