//! How it all looks, on every peer: the cooks nobody plays out of the
//! kitchen; on the host, what a cook holds in front of them and what is on
//! a station on top of it, chopped food flat (the others get that as the
//! items' transforms); everywhere, a bar over a board as the chopping goes
//! and over the stove as the soup cooks, the soup in a pot and on a plate —
//! from the pots, the chopping and the plates as the host sends them.

use runity::glam::Vec3;
use runity::hecs::{Entity, World};
use runity::net::Owned;
use runity::Transform;

use crate::components::item::{Food, Thing};
use crate::components::{Item, Player, Station};
use crate::state::*;

pub fn run(world: &mut World, _seconds: f32) {
    cooks(world);
    held_by(world);
    place_items(world);
    marks(world);
    boiling(world);
}

/// A full pot boils, louder once it is done: the stove's own `sound`,
/// turned up and down.
fn boiling(world: &mut World) {
    for (pot, sound) in world.query_mut::<(&Pot, &mut runity::world::Sounding)>() {
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
        .query::<(Entity, &Player, Option<&Seat>, Option<&runity::world::Inactive>)>()
        .iter()
        .map(|(e, _, seat, off)| (e, seat.is_some(), off.is_none()))
        .collect();
    for (cook, seated, on) in cooks {
        if seated != on {
            runity::world::set_active(world, cook, seated);
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
        if let Ok(mut t) = world.get::<&mut Transform>(item) {
            t.position = spot;
            // The item's root is unscaled; chopped, it is flat.
            t.scale.y = if flat { 0.35 } else { 1.0 };
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
    let mut shown: Vec<(Entity, Vec<(String, f32)>)> = Vec::new();
    for (station, _, at, pot) in world
        .query::<(Entity, &Station, &Transform, Option<&Pot>)>()
        .iter()
    {
        let mut on = Vec::new();
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
        let soup = served.and_then(|s| s.0).map(|soup| match soup {
            Soup::Of(Food::Tomato) => "soup tomato",
            Soup::Of(Food::Onion) => "soup onion",
            Soup::Mixed => "soup mixed",
        });
        shown.push((plate, soup.map(|s| (s.to_string(), 1.0)).into_iter().collect()));
    }
    for (owner, on) in shown {
        for (mark, name) in marked(world, owner) {
            let amount = on.iter().find(|(n, _)| *n == name).map(|(_, a)| *a);
            let off = world.get::<&runity::world::Inactive>(mark).is_ok();
            if amount.is_some() == off {
                runity::world::set_active(world, mark, amount.is_some());
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
