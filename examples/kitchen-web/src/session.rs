//! What a frame of the kitchen does around the party, the same in the
//! window and in the tests: the host acts on what everyone's hands did,
//! seats the players, and makes what the steps asked for — spawned from its
//! prefab, told to everyone, and put where it goes.

use scrap::hecs::{Entity, World};
use scrap::party::{Event, Party};

use crate::state::{self, Act, Hands, Place, Top, ACT};

/// The acts among a frame's events, carried out — on the host, and only
/// what the sender may: their own cook's hands, and the next round only
/// when the host itself asked.
pub fn act(world: &mut World, party: &Party, events: &[Event]) {
    if !party.is_host() {
        return;
    }
    for event in events {
        let Some(act) = event.decode::<Act>(ACT) else {
            continue;
        };
        let Event::Message { from, .. } = event else {
            continue;
        };
        let allowed = match act {
            // The next round is the host's to start, whoever else asks.
            Act::Restart => *from == party.me(),
            // Hands are only for the cook seated for whoever sent them.
            Act::Grab { cook } | Act::Throw { cook } | Act::Work { cook, .. } => {
                state::cook(world, cook)
                    .and_then(|c| world.get::<&state::Seat>(c).ok().map(|s| s.0))
                    == Some(from.0)
            }
        };
        if allowed {
            state::apply(world, act);
        }
    }
}

/// Seat, claim, and spawn: the kitchen's part of a frame after the party's.
/// `make` spawns a prefab by name, with a renderer or without.
pub fn frame(
    world: &mut World,
    party: &mut Party,
    make: impl FnMut(&mut World, &str) -> Option<Entity>,
) {
    crate::front::hold(world);
    crate::front::seat(world, party);
    crate::front::claim(world, party);
    spawns(world, party, make);
}

/// What the steps asked to be made.
pub fn spawns(
    world: &mut World,
    party: &mut Party,
    mut make: impl FnMut(&mut World, &str) -> Option<Entity>,
) {
    let asked: Vec<state::Spawn> = world
        .query_mut::<&mut state::Service>()
        .into_iter()
        .flat_map(|s| std::mem::take(&mut s.spawns))
        .collect();
    for spawn in asked {
        let Some(item) = make(world, spawn.prefab) else {
            continue;
        };
        // Everyone else gets it too, from the same prefab.
        party.spawn(world, item, spawn.prefab);
        let _ = world.insert_one(item, state::At(spawn.place));
        match spawn.place {
            Place::Hand(cook) => {
                let _ = world.insert_one(cook, Hands(Some(item)));
            }
            Place::On(station) => {
                let _ = world.insert_one(station, Top(Some(item)));
            }
        }
    }
}
