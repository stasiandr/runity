//! What a cook does with their hands at the station they face: `grab`
//! takes and puts things, `work` chops on a board and scrapes a burnt pot.
//!
//! A crate gives its food, the stack a plate. A counter, a board or a crate
//! holds one thing. Chopped food goes into a pot, three to a pot; a plate
//! takes a cooked soup; the window takes a plate of soup and an order that
//! wants it; the bin takes food and empties a plate. The host's: what the
//! others' hands do comes to it as acts.

use runity::glam::Vec3;
use runity::hecs::{Entity, World};
use runity::Transform;

use crate::components::item::Thing;
use crate::components::station::Kind;
use crate::components::{Item, Player, Station};
use crate::state::*;

pub fn run(world: &mut World, seconds: f32) {
    let Some((kitchen, true)) = kitchen(world) else {
        return;
    };
    if world.get::<&Round>(kitchen).map_or(true, |r| r.over) {
        return;
    }
    let cooks: Vec<(Entity, Vec3, Vec3, Controls)> = world
        .query::<(Entity, &Transform, &Player, &Controls)>()
        .iter()
        .map(|(e, t, _, c)| (e, t.position, facing(t), *c))
        .collect();
    for (cook, at, facing, controls) in cooks {
        let station = facing_station(world, at, facing);
        if controls.throw {
            throw(world, cook, at, facing);
            if let Ok(mut c) = world.get::<&mut Controls>(cook) {
                c.throw = false;
            }
        }
        if controls.grab {
            // Something lying on the floor in front comes first.
            match loose_ahead(world, at, facing).filter(|_| held_by(world, cook).is_none()) {
                Some(item) => {
                    let _ = world.insert_one(cook, Hands(Some(item)));
                    let _ = world.insert_one(item, At(Place::Hand(cook)));
                }
                None => {
                    if let Some(station) = station {
                        grab(world, kitchen, cook, station);
                    }
                }
            }
            if let Ok(mut c) = world.get::<&mut Controls>(cook) {
                c.grab = false;
            }
        }
        if controls.work {
            if let Some(station) = station {
                work(world, station, seconds);
            }
        }
    }
}

/// Food thrown: out of the hands, into the air, the way the cook faces.
/// Only food flies — a plate would break.
fn throw(world: &mut World, cook: Entity, at: Vec3, facing: Vec3) {
    let Some(item) = held_by(world, cook) else {
        return;
    };
    if !matches!(thing_of(world, item), Some(Thing::Food(_))) {
        return;
    }
    let _ = world.insert_one(cook, Hands(None));
    let _ = world.remove_one::<At>(item);
    let start = at + facing * 0.6 + Vec3::Y * 0.7;
    if let Ok(mut t) = world.get::<&mut Transform>(item) {
        t.position = start;
    }
    let _ = world.insert(
        item,
        (
            runity::world::Physics(runity::scene::Body::Dynamic),
            // A ball the size of the fruit under the item.
            runity::world::Shape(runity::scene::Collider::Sphere { radius: FOOD_RADIUS }),
            // Food does not roll far on a kitchen floor.
            runity::world::Props(runity::scene::BodyProps {
                drag: 0.4,
                spin_drag: 4.0,
                friction: 1.0,
                bounce: 0.2,
                ..Default::default()
            }),
            Flying {
                launch: Some(facing * THROW_SPEED + Vec3::Y * THROW_LIFT),
                last: start,
                still: 0.0,
                age: 0.0,
            },
        ),
    );
}

/// Food lying loose on the floor, just ahead of a cook.
fn loose_ahead(world: &World, at: Vec3, facing: Vec3) -> Option<Entity> {
    let ahead = at + facing * 0.7;
    world
        .query::<(Entity, &Transform, &Item)>()
        .without::<&At>()
        .without::<&Flying>()
        .iter()
        .map(|(e, t, _)| (e, Vec3::new(t.position.x - ahead.x, 0.0, t.position.z - ahead.z).length()))
        .filter(|(_, d)| *d < 0.6)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(e, _)| e)
}

/// The station in front of a cook: the nearest to a point just ahead.
fn facing_station(world: &World, at: Vec3, facing: Vec3) -> Option<Entity> {
    let ahead = at + facing * 0.8;
    world
        .query::<(Entity, &Transform, &Station)>()
        .iter()
        .map(|(e, t, _)| (e, Vec3::new(t.position.x - ahead.x, 0.0, t.position.z - ahead.z).length()))
        .filter(|(_, d)| *d < 0.8)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(e, _)| e)
}

fn kind_of(world: &World, station: Entity) -> Option<Kind> {
    world.get::<&Station>(station).ok().map(|s| s.kind)
}

fn top_of(world: &World, station: Entity) -> Option<Entity> {
    world.get::<&Top>(station).ok().and_then(|t| t.0)
}

fn held_by(world: &World, cook: Entity) -> Option<Entity> {
    world.get::<&Hands>(cook).ok().and_then(|h| h.0)
}

fn thing_of(world: &World, item: Entity) -> Option<Thing> {
    world.get::<&Item>(item).ok().map(|i| i.thing)
}

fn chopped(world: &World, item: Entity) -> bool {
    world.get::<&Chop>(item).is_ok_and(|c| c.0 >= 1.0)
}

fn say(world: &World, kitchen: Entity, words: &str) {
    if let Ok(mut r) = world.get::<&mut Round>(kitchen) {
        r.say(words);
    }
}

/// Put `item` in `cook`'s hands, off wherever it was.
fn take(world: &mut World, cook: Entity, station: Entity, item: Entity) {
    let _ = world.insert_one(station, Top(None));
    let _ = world.insert_one(cook, Hands(Some(item)));
    let _ = world.insert_one(item, At(Place::Hand(cook)));
}

/// Put what `cook` holds down on `station`.
fn put(world: &mut World, cook: Entity, station: Entity, item: Entity) {
    let _ = world.insert_one(station, Top(Some(item)));
    let _ = world.insert_one(cook, Hands(None));
    let _ = world.insert_one(item, At(Place::On(station)));
}

/// Take what `cook` holds out of the game.
fn used_up(world: &mut World, cook: Entity, item: Entity) {
    let _ = world.insert_one(cook, Hands(None));
    despawn_tree(world, item);
}

fn grab(world: &mut World, kitchen: Entity, cook: Entity, station: Entity) {
    let Some(kind) = kind_of(world, station) else {
        return;
    };
    let top = top_of(world, station);
    let pending = world
        .get::<&Service>(kitchen)
        .is_ok_and(|s| s.spawns.iter().any(|sp| sp.place == Place::Hand(cook)));
    match held_by(world, cook) {
        None if pending => {}
        None => match (kind, top) {
            (_, Some(item)) => take(world, cook, station, item),
            (Kind::Crate(food), None) => spawn(world, kitchen, food.name(), cook),
            (Kind::Plates, None) => spawn(world, kitchen, "plate", cook),
            (Kind::Stove, None) => {
                let done = world.get::<&Pot>(station).is_ok_and(|p| p.done());
                if done {
                    say(world, kitchen, "Bring a plate for the soup");
                }
            }
            _ => {}
        },
        Some(item) => {
            let Some(thing) = thing_of(world, item) else {
                return;
            };
            match kind {
                Kind::Counter | Kind::Board | Kind::Crate(_) if top.is_none() => {
                    put(world, cook, station, item)
                }
                Kind::Bin => match thing {
                    Thing::Food(_) => used_up(world, cook, item),
                    Thing::Plate => {
                        let _ = world.insert_one(item, Served(None));
                    }
                },
                Kind::Stove => stove(world, kitchen, cook, station, item, thing),
                Kind::Window => serve(world, kitchen, cook, item, thing),
                _ => {}
            }
        }
    }
}

fn spawn(world: &World, kitchen: Entity, prefab: &'static str, cook: Entity) {
    if let Ok(mut s) = world.get::<&mut Service>(kitchen) {
        s.spawns.push(Spawn {
            prefab,
            place: Place::Hand(cook),
        });
    }
}

/// Food into the pot, or the soup onto a plate.
fn stove(world: &mut World, kitchen: Entity, cook: Entity, stove: Entity, item: Entity, thing: Thing) {
    if world.get::<&Pot>(stove).is_err() {
        let _ = world.insert_one(stove, Pot::default());
    }
    let pot = world.get::<&Pot>(stove).map(|p| (*p).clone()).unwrap_or_default();
    match thing {
        Thing::Food(_) if !chopped(world, item) => say(world, kitchen, "Chop it first"),
        Thing::Food(food) if pot.foods.len() < POT_HOLDS && !pot.burnt() => {
            if let Ok(mut p) = world.get::<&mut Pot>(stove) {
                p.foods.push(food);
            }
            used_up(world, cook, item);
        }
        Thing::Food(_) => say(world, kitchen, "The pot is full"),
        Thing::Plate => {
            let empty = world.get::<&Served>(item).map_or(true, |s| s.0.is_none());
            if pot.burnt() {
                say(world, kitchen, "Burnt! Scrape the pot (hold F)");
            } else if !pot.done() {
                say(world, kitchen, "Not cooked yet");
            } else if empty {
                let _ = world.insert_one(item, Served(pot.soup()));
                let _ = world.insert_one(stove, Pot::default());
            }
        }
    }
}

/// A plate of soup out of the window, to the order that wants it most.
fn serve(world: &mut World, kitchen: Entity, cook: Entity, item: Entity, thing: Thing) {
    let soup = world.get::<&Served>(item).ok().and_then(|s| s.0);
    let Some(soup) = soup.filter(|_| thing == Thing::Plate) else {
        say(world, kitchen, "The window takes a plate of soup");
        return;
    };
    used_up(world, cook, item);
    let Ok(mut s) = world.get::<&mut Round>(kitchen) else {
        return;
    };
    let wanted = s.orders.iter().position(|o| Soup::Of(o.food) == soup);
    match wanted {
        Some(i) => {
            let order = s.orders.remove(i);
            // Quicker is worth more.
            let tip = (10.0 * order.left / order.total).round() as i32;
            s.score += 20 + tip;
            s.served += 1;
            s.say(format!("Served! +{}", 20 + tip));
        }
        None => {
            s.score -= 5;
            s.say("Nobody ordered that: −5");
        }
    }
}

fn work(world: &mut World, station: Entity, seconds: f32) {
    match kind_of(world, station) {
        Some(Kind::Board) => {
            let Some(item) = top_of(world, station) else {
                return;
            };
            if !matches!(thing_of(world, item), Some(Thing::Food(_))) {
                return;
            }
            let so_far = world.get::<&Chop>(item).map_or(0.0, |c| c.0);
            let _ = world.insert_one(item, Chop((so_far + seconds / CHOP_SECONDS).min(1.0)));
        }
        Some(Kind::Stove) => {
            let burnt = world.get::<&Pot>(station).is_ok_and(|p| p.burnt());
            if burnt {
                let _ = world.insert_one(station, Pot::default());
            }
        }
        _ => {}
    }
}
