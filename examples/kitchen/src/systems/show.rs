//! How it all looks, on every peer: the cooks nobody plays out of the
//! kitchen; on the host, what a cook holds in front of them and what is on
//! a station on top of it, chopped food flat (the others get that as the
//! items' transforms); everywhere, a bar over a board as the chopping goes
//! and over the stove as the soup cooks, the soup in a pot and on a plate —
//! from the pots, the chopping and the plates as the host sends them.

use scrap::glam::Vec3;
use scrap::hecs::{Entity, World};
use scrap::net::Owned;
use scrap::Transform;

use crate::components::item::{Dish, Food, Part, Thing};
use crate::components::station::Kind;
use crate::components::{Item, Player, Station};
use crate::state::*;

pub fn run(world: &mut World, seconds: f32) {
    cooks(world);
    held_by(world);
    hops(world, seconds);
    place_items(world);
    marks(world);
    boiling(world);
    music(world);
}

/// The last half minute of a round, the music hurries.
fn music(world: &mut World) {
    let hurry = world
        .query::<&Round>()
        .iter()
        .next()
        .is_some_and(|r| r.open && !r.over && r.time_left < 30.0);
    for (_, sound) in world.query_mut::<(&crate::components::Kitchen, &mut scrap::world::Sounding)>() {
        sound.0.pitch = if hurry { 1.12 } else { 1.0 };
    }
}

/// Seconds a hop lasts.
const HOP: f32 = 0.22;

/// An item's hop as it lands somewhere new or gets something on it:
/// seconds of it left, and where it was and how much it carried.
#[derive(Debug, Clone, Copy)]
pub struct Hop {
    left: f32,
    was: (Place, usize),
}

/// This peer's items hop when they move to a new place or a plate gets a
/// part (the others see it in the transforms).
fn hops(world: &mut World, seconds: f32) {
    let items: Vec<(Entity, Place, usize, Option<Hop>)> = world
        .query::<(Entity, &At, Option<&Served>, Option<&Hop>)>()
        .with::<&Owned>()
        .iter()
        .map(|(e, at, served, hop)| (e, at.0, served.map_or(0, |s| s.parts.len()), hop.copied()))
        .collect();
    for (item, place, parts, hop) in items {
        let now = (place, parts);
        let hop = match hop {
            Some(h) if h.was == now => Hop { left: (h.left - seconds).max(0.0), was: now },
            Some(_) => Hop { left: HOP, was: now },
            // First seen: no hop for being spawned into a hand.
            None => Hop { left: 0.0, was: now },
        };
        let _ = world.insert_one(item, hop);
    }
}

/// A full pot boils, louder once it is done: the stove's own `sound`,
/// turned up and down.
fn boiling(world: &mut World) {
    for (pot, sound) in world.query_mut::<(&Pot, &mut scrap::world::Sounding)>() {
        sound.0.volume = if pot.burnt() {
            0.2
        } else if pot.done() {
            0.9
        } else if pot.foods.len() == POT_HOLDS {
            0.6
        } else {
            0.0
        };
    }
}

/// A cook is in the kitchen while someone plays it.
fn cooks(world: &mut World) {
    let cooks: Vec<(Entity, bool, bool)> = world
        .query::<(Entity, &Player, Option<&Seat>, Option<&scrap::world::Inactive>)>()
        .iter()
        .map(|(e, _, seat, off)| (e, seat.is_some(), off.is_none()))
        .collect();
    for (cook, seated, on) in cooks {
        if seated != on {
            scrap::world::set_active(world, cook, seated);
        }
    }
}

/// Who holds each item, for the others (the host's).
fn held_by(world: &mut World) {
    let changes: Vec<(Entity, Option<u32>, Option<crate::components::HeldBy>)> = world
        .query::<(Entity, &At, Option<&crate::components::HeldBy>)>()
        .with::<&Owned>()
        .iter()
        .map(|(e, at, now)| {
            let by = match at.0 {
                Place::Hand(cook) => world.get::<&Player>(cook).ok().map(|p| p.index),
                Place::On(_) => None,
            };
            (e, by, now.copied())
        })
        .collect();
    for (item, by, now) in changes {
        match (by, now) {
            (Some(i), now) if now != Some(crate::components::HeldBy(i)) => {
                let _ = world.insert_one(item, crate::components::HeldBy(i));
            }
            (None, Some(_)) => {
                let _ = world.remove_one::<crate::components::HeldBy>(item);
            }
            _ => {}
        }
    }
    // Out of the hands some other way — thrown, dropped: no holder.
    let loose: Vec<Entity> = world
        .query::<(Entity, &crate::components::HeldBy)>()
        .without::<&At>()
        .with::<&Owned>()
        .iter()
        .map(|(e, _)| e)
        .collect();
    for item in loose {
        let _ = world.remove_one::<crate::components::HeldBy>(item);
    }
}

/// Where each of this peer's items goes: in the hands in front, or on top
/// of a station.
fn place_items(world: &mut World) {
    let places: Vec<(Entity, Vec3)> = world
        .query::<(Entity, &At)>()
        .with::<&Owned>()
        .iter()
        .filter_map(|(e, at)| {
            let spot = match at.0 {
                Place::Hand(cook) => {
                    let t = world.get::<&Transform>(cook).ok()?;
                    t.position + facing(&t) * 0.55 + Vec3::Y * 0.45
                }
                Place::On(station) => world.get::<&Transform>(station).ok()?.position + Vec3::Y * 0.66,
            };
            Some((e, spot))
        })
        .collect();
    for (item, spot) in places {
        let flat = world.get::<&Chop>(item).is_ok_and(|c| c.0 >= 1.0);
        // Squashed and back as it hops.
        let squash = world
            .get::<&Hop>(item)
            .map_or(0.0, |h| (std::f32::consts::PI * (1.0 - h.left / HOP)).sin() * (h.left > 0.0) as u8 as f32);
        if let Ok(mut t) = world.get::<&mut Transform>(item) {
            t.position = spot + Vec3::Y * 0.12 * squash;
            // The item's root is unscaled; chopped, it is flat.
            let tall = if flat { 0.35 } else { 1.0 };
            t.scale = Vec3::new(1.0 + 0.18 * squash, tall * (1.0 - 0.22 * squash), 1.0 + 0.18 * squash);
        }
    }
}

/// How far a food was chopped at the last step: juice flies while it rises.
#[derive(Debug, Clone, Copy)]
pub struct Cut(pub f32);

/// What each station and plate shows, by the marks on its parts.
fn marks(world: &mut World) {
    // Food being chopped, and where: the station under it.
    let chopping: Vec<(Vec3, f32)> = world
        .query::<(&Transform, &Chop)>()
        .iter()
        .filter(|(_, c)| c.0 > 0.0 && c.0 < 1.0)
        .map(|(t, c)| (t.position, c.0))
        .collect();
    // Food being cut this very moment: its chop went up since the last step.
    let mut cutting: Vec<(Vec3, f32)> = Vec::new();
    let chops: Vec<(Entity, Vec3, f32)> = world
        .query::<(Entity, &Transform, &Chop)>()
        .iter()
        .map(|(e, t, c)| (e, t.position, c.0))
        .collect();
    for (item, at, chop) in chops {
        let before = world.get::<&Cut>(item).map_or(chop, |c| c.0);
        if chop > before && chop < 1.0 {
            cutting.push((at, chop));
        }
        let _ = world.insert_one(item, Cut(chop));
    }
    // Meat, and how fried: on a pan, it shows there.
    let meats: Vec<(Entity, Vec3, Fry)> = world
        .query::<(Entity, &Transform, &Item, Option<&Fry>)>()
        .iter()
        .filter(|(_, _, item, _)| item.thing == Thing::Food(Food::Meat))
        .map(|(e, t, _, f)| (e, t.position, f.copied().unwrap_or_default()))
        .collect();
    let mut shown: Vec<(Entity, Vec<(String, f32)>)> = Vec::new();
    let mut sizzling: Vec<(Entity, f32)> = Vec::new();
    for (station, kind, at, pot, stack, sink) in world
        .query::<(Entity, &Station, &Transform, Option<&Pot>, Option<&Stack>, Option<&Sink>)>()
        .iter()
    {
        let mut on = Vec::new();
        if kind.kind == Kind::Pan {
            let here = at.position;
            let frying = meats
                .iter()
                .find(|(_, p, _)| (p.x - here.x).abs() < 0.3 && (p.z - here.z).abs() < 0.3 && p.y > here.y);
            sizzling.push((station, if frying.is_some_and(|(_, _, f)| !f.burnt()) { 0.5 } else { 0.0 }));
            if let Some((_, _, fry)) = frying {
                if fry.on_fire() {
                    on.push(("fire".into(), 1.0));
                }
                if fry.burnt() {
                    on.push(("smoke".into(), 1.0));
                } else if fry.done() {
                    let left = 1.0 - (fry.0 - FRY_SECONDS) / (FRY_BURN - FRY_SECONDS);
                    on.push(("warn".into(), left));
                    on.push(("sizzle".into(), 1.0));
                } else {
                    on.push(("bar".into(), fry.0 / FRY_SECONDS));
                    on.push(("sizzle".into(), 1.0));
                }
            }
        }
        // A plate a mark, as many as the rack has; the dirty ones in the sink.
        if let Some(stack) = stack {
            for n in 1..=stack.0 {
                on.push((format!("stack {n}"), 1.0));
            }
        }
        if let Some(sink) = sink {
            for n in 1..=sink.dirty {
                on.push((format!("dirty {n}"), 1.0));
            }
            if sink.washed > 0.0 {
                on.push(("bar".into(), sink.washed));
                on.push(("suds".into(), 1.0));
            }
        }
        if let Some(pot) = pot {
            if !pot.foods.is_empty() {
                let name = match pot.soup() {
                    _ if pot.burnt() => "burnt".to_string(),
                    Some(Soup::Of(food)) => food.name().to_string(),
                    _ => "mixed".to_string(),
                };
                on.push((format!("soup {name}"), pot.foods.len() as f32 / POT_HOLDS as f32));
            }
            if pot.foods.len() == POT_HOLDS && !pot.burnt() {
                // The burner lit under a full pot.
                on.push(("flame".into(), 1.0));
            }
            if pot.foods.len() == POT_HOLDS && !pot.done() && !pot.burnt() {
                on.push(("bar".into(), pot.cooked / COOK_SECONDS));
            } else if pot.done() {
                // Done, and heading for burnt: the bar turns to a warning,
                // and it steams.
                let left = 1.0 - (pot.cooked - COOK_SECONDS) / (BURN_SECONDS - COOK_SECONDS);
                on.push(("warn".into(), left));
                on.push(("steam".into(), 1.0));
            } else if pot.burnt() {
                on.push(("smoke".into(), 1.0));
            }
            if pot.on_fire() {
                on.push(("fire".into(), 1.0));
            }
        }
        let here = at.position;
        if let Some((_, chop)) = chopping
            .iter()
            .find(|(p, _)| (p.x - here.x).abs() < 0.3 && (p.z - here.z).abs() < 0.3)
        {
            on.push(("bar".into(), *chop));
        }
        if let Some((_, chop)) = cutting
            .iter()
            .find(|(p, _)| (p.x - here.x).abs() < 0.3 && (p.z - here.z).abs() < 0.3)
        {
            on.push(("chop fx".into(), *chop));
        }
        shown.push((station, on));
    }
    for (plate, item, served) in world.query::<(Entity, &Item, Option<&Served>)>().iter() {
        if item.thing != Thing::Plate {
            continue;
        }
        let load = served.cloned().unwrap_or_default();
        let mut on: Vec<(String, f32)> = Vec::new();
        match (load.soup, load.dish()) {
            (Some(Soup::Of(Food::Onion)), _) => on.push(("soup onion".into(), 1.0)),
            (Some(Soup::Of(_)), _) => on.push(("soup tomato".into(), 1.0)),
            (Some(Soup::Mixed), _) => on.push(("soup mixed".into(), 1.0)),
            // A whole dish shows as itself, the parts as they go on.
            (None, Some(Dish::Salad)) => on.push(("salad".into(), 1.0)),
            (None, Some(Dish::Burger)) => on.push(("burger".into(), 1.0)),
            (None, _) => {
                for part in &load.parts {
                    let name = match part {
                        Part::Chopped(food) => format!("part {}", food.name()),
                        Part::Bun => "part bun".into(),
                        Part::Patty => "part patty".into(),
                    };
                    on.push((name, 1.0));
                }
            }
        }
        shown.push((plate, on));
    }
    // A pan with meat on it sizzles: its own `sound`, turned up.
    for (pan, volume) in sizzling {
        if let Ok(mut sound) = world.get::<&mut scrap::world::Sounding>(pan) {
            sound.0.volume = volume;
        }
    }
    // Meat raw, fried or burnt.
    for (meat, _, fry) in &meats {
        let look = if fry.burnt() {
            "burnt"
        } else if fry.done() {
            "cooked"
        } else {
            "raw"
        };
        shown.push((*meat, vec![(look.to_string(), 1.0)]));
    }
    for (owner, on) in shown {
        for (mark, name) in marked(world, owner) {
            let amount = on.iter().find(|(n, _)| *n == name).map(|(_, a)| *a);
            let off = world.get::<&scrap::world::Inactive>(mark).is_ok();
            if amount.is_some() == off {
                scrap::world::set_active(world, mark, amount.is_some());
            }
            let Some(amount) = amount else { continue };
            if let Ok(mut t) = world.get::<&mut Transform>(mark) {
                if name == "bar" || name == "warn" {
                    // A bar filling from the left.
                    let full = 0.9;
                    t.scale.x = (full * amount.clamp(0.0, 1.0)).max(0.02);
                    t.position.x = -full / 2.0 + t.scale.x / 2.0;
                } else if name.starts_with("soup") {
                    t.scale.y = 0.05 + 0.15 * amount;
                }
            }
        }
    }
}
