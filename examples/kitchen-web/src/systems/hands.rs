//! What a cook does with their hands at the station they face: `grab`
//! takes and puts things, `work` chops on a board, scrapes a burnt pot and
//! washes plates at the sink.
//!
//! A crate gives its food, a rack a clean plate while it has one. A
//! counter, a board or a crate holds one thing; a pan holds meat, and fries
//! it. Chopped tomato or onion goes into a pot, three to a pot; a plate
//! takes a cooked soup, or the parts of a dish one by one — put on it, or
//! picked up with it; the window takes a plate with a dish on it and the
//! order that wants it, and the plate comes back to the sink dirty; the bin
//! takes food and empties a plate. The host's: what the others' hands do
//! comes to it as acts.

use scrap::glam::Vec3;
use scrap::hecs::{Entity, World};
use scrap::Transform;

use crate::components::fry::Fry;
use crate::components::item::{Food, Part, Thing};
use crate::components::sink::{Sink, RETURN_SECONDS, WASH_SECONDS};
use crate::components::stack::Stack;
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
            scrap::world::Physics(scrap::scene::Body::Dynamic),
            // A ball the size of the fruit under the item.
            scrap::world::Shape(scrap::scene::Collider::Sphere { radius: FOOD_RADIUS }),
            // Food does not roll far on a kitchen floor.
            scrap::world::Props(scrap::scene::BodyProps {
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

/// What `item` would be on a plate, if anything: chopped food, a bun, a
/// fried patty.
fn part_of(world: &World, item: Entity) -> Option<Part> {
    match thing_of(world, item)? {
        Thing::Food(Food::Bun) => Some(Part::Bun),
        Thing::Food(Food::Meat) => world.get::<&Fry>(item).ok().filter(|f| f.done()).map(|_| Part::Patty),
        Thing::Food(food) if food.chops() && chopped(world, item) => Some(Part::Chopped(food)),
        _ => None,
    }
}

/// Put `part` on `plate` if it goes: said when it does not.
fn onto(world: &mut World, kitchen: Entity, plate: Entity, part: Part) -> bool {
    let mut load = world.get::<&Served>(plate).map(|s| (*s).clone()).unwrap_or_default();
    if !load.takes(part) {
        say(world, kitchen, "That does not go on this plate");
        return false;
    }
    load.add(part);
    let _ = world.insert_one(plate, load);
    true
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
            (Kind::Plates, None) => {
                let left = world.get::<&Stack>(station).map_or(0, |s| s.0);
                if left == 0 {
                    say(world, kitchen, "No clean plates: wash some at the sink");
                } else {
                    let _ = world.insert_one(station, Stack(left - 1));
                    spawn(world, kitchen, "plate", cook);
                }
            }
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
            // A plate held, food on the station: onto the plate with it.
            if let (Thing::Plate, Some(food)) = (thing, top) {
                if let Some(part) = part_of(world, food) {
                    if onto(world, kitchen, item, part) {
                        let _ = world.insert_one(station, Top(None));
                        despawn_tree(world, food);
                    }
                    return;
                }
            }
            // A bun straight from its crate onto a held plate.
            if let (Thing::Plate, Kind::Crate(Food::Bun), None) = (thing, kind, top) {
                onto(world, kitchen, item, Part::Bun);
                return;
            }
            // Food held, a plate on the station: onto it.
            if let (Thing::Food(_), Some(plate)) = (thing, top) {
                if thing_of(world, plate) == Some(Thing::Plate) {
                    match part_of(world, item) {
                        Some(part) => {
                            if onto(world, kitchen, plate, part) {
                                used_up(world, cook, item);
                            }
                        }
                        None => say(world, kitchen, "Chop it or fry it first"),
                    }
                    return;
                }
            }
            match kind {
                Kind::Counter | Kind::Board | Kind::Crate(_) if top.is_none() => {
                    put(world, cook, station, item)
                }
                Kind::Pan if top.is_none() => match thing {
                    Thing::Food(Food::Meat) => put(world, cook, station, item),
                    _ => say(world, kitchen, "The pan is for meat"),
                },
                Kind::Bin => match thing {
                    Thing::Food(_) => used_up(world, cook, item),
                    Thing::Plate => {
                        let _ = world.insert_one(item, Served::default());
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
        Thing::Food(food) if !food.soups() => say(world, kitchen, "Soup is tomato or onion"),
        Thing::Food(_) if !chopped(world, item) => say(world, kitchen, "Chop it first"),
        Thing::Food(food) if pot.foods.len() < POT_HOLDS && !pot.burnt() => {
            if let Ok(mut p) = world.get::<&mut Pot>(stove) {
                p.foods.push(food);
            }
            used_up(world, cook, item);
        }
        Thing::Food(_) => say(world, kitchen, "The pot is full"),
        Thing::Plate => {
            let empty = world.get::<&Served>(item).map_or(true, |s| s.is_empty());
            if pot.burnt() {
                say(world, kitchen, "Burnt! Scrape the pot (hold F)");
            } else if !pot.done() {
                say(world, kitchen, "Not cooked yet");
            } else if empty {
                let served = Served {
                    soup: pot.soup(),
                    parts: Vec::new(),
                };
                let _ = world.insert_one(item, served);
                let _ = world.insert_one(stove, Pot::default());
            }
        }
    }
}

/// A plate with a dish out of the window, to the order that wants it most;
/// the plate is back at the sink, dirty, a little later.
fn serve(world: &mut World, kitchen: Entity, cook: Entity, item: Entity, thing: Thing) {
    let load = world.get::<&Served>(item).map(|s| (*s).clone()).unwrap_or_default();
    if thing != Thing::Plate || load.is_empty() {
        say(world, kitchen, "The window takes a plate with a dish on it");
        return;
    }
    let dish = load.dish();
    used_up(world, cook, item);
    if let Ok(mut service) = world.get::<&mut Service>(kitchen) {
        service.returning.push(RETURN_SECONDS);
    }
    let Ok(mut s) = world.get::<&mut Round>(kitchen) else {
        return;
    };
    let wanted = dish.and_then(|dish| s.orders.iter().position(|o| o.dish == dish));
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
            if !matches!(thing_of(world, item), Some(Thing::Food(food)) if food.chops()) {
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
        Some(Kind::Pan) => {
            // Burnt meat scraped off the pan, fire and all.
            let Some(meat) = top_of(world, station) else {
                return;
            };
            if world.get::<&Fry>(meat).is_ok_and(|f| f.burnt()) {
                let _ = world.insert_one(station, Top(None));
                despawn_tree(world, meat);
            }
        }
        Some(Kind::Sink) => {
            let Ok(mut sink) = world.get::<&mut Sink>(station).map(|s| *s) else {
                return;
            };
            if sink.dirty == 0 {
                return;
            }
            sink.washed += seconds / WASH_SECONDS;
            if sink.washed >= 1.0 {
                sink.washed = 0.0;
                sink.dirty -= 1;
                rack(world, station);
            }
            let _ = world.insert_one(station, sink);
        }
        _ => {}
    }
}

/// A washed plate onto the rack nearest the sink.
fn rack(world: &mut World, sink: Entity) {
    let Some(at) = world.get::<&Transform>(sink).ok().map(|t| t.position) else {
        return;
    };
    let nearest = world
        .query::<(Entity, &Transform, &Station)>()
        .iter()
        .filter(|(_, _, s)| s.kind == Kind::Plates)
        .min_by(|a, b| a.1.position.distance(at).total_cmp(&b.1.position.distance(at)))
        .map(|(e, ..)| e);
    if let Some(rack) = nearest {
        let n = world.get::<&Stack>(rack).map_or(0, |s| s.0);
        let _ = world.insert_one(rack, Stack(n + 1));
    }
}
