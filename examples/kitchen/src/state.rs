//! What the game keeps while it runs, beside the scene's components.
//!
//! The kitchen is the host's: it keeps who holds what, what is on which
//! counter, the round's clock and its orders, and runs the rules. What the
//! others need to see goes to them as networked components — what is in a
//! pot ([`Pot`]), how far a food is chopped ([`Chop`]) or fried ([`Fry`]),
//! what is on a plate ([`Served`]), the clean plates on a rack ([`Stack`])
//! and the dirty ones in the sink ([`Sink`]), the round ([`Round`]), who
//! plays which cook ([`Seat`]) —
//! and every item's place as its transform. Each player drives their own
//! cook's walk; what their hands do goes to the host as an [`Act`].

use runity::glam::Vec3;
use runity::hecs::{Entity, World};
use runity::Transform;
use serde::{Deserialize, Serialize};

pub use crate::components::pot::{BURN_SECONDS, COOK_SECONDS, POT_HOLDS};
pub use crate::components::round::Order;
pub use crate::components::fry::{Fry, FRY_BURN, FRY_SECONDS};
pub use crate::components::served::Soup;
pub use crate::components::sink::Sink;
pub use crate::components::stack::Stack;
pub use crate::components::{Chop, Pot, Round, Seat, Served};

/// Seconds of work to chop one.
pub const CHOP_SECONDS: f32 = 1.2;

/// What a cook's keys say this step: where to go, and the buttons. `grab`
/// stays down until a step has used it, so a press between two steps is
/// not lost.
#[derive(Debug, Default, Clone, Copy)]
pub struct Controls {
    pub x: f32,
    pub z: f32,
    pub grab: bool,
    pub work: bool,
    pub throw: bool,
}

/// What a player's hands did, sent to the host: grab once, work held or
/// let go, or start the round again.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Act {
    Grab { cook: u32 },
    /// Throw what is held, the way the cook faces.
    Throw { cook: u32 },
    Work { cook: u32, on: bool },
    Restart,
}

/// The message kind an [`Act`] travels as.
pub const ACT: &str = "act";

/// What a cook is holding (the host's).
#[derive(Debug, Default, Clone, Copy)]
pub struct Hands(pub Option<Entity>);

/// The way a cook faces, from how it is turned.
pub fn facing(t: &Transform) -> Vec3 {
    let yaw = t.rotation_deg.y.to_radians();
    Vec3::new(yaw.sin(), 0.0, yaw.cos())
}

/// Where an item is (the host's).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    Hand(Entity),
    On(Entity),
}

#[derive(Debug, Clone, Copy)]
pub struct At(pub Place);

/// What is on a station (the host's).
#[derive(Debug, Default, Clone, Copy)]
pub struct Top(pub Option<Entity>);

/// Something to spawn from a prefab, which only the frame can: it has the
/// renderer (and the party, which tells the others).
#[derive(Debug, Clone)]
pub struct Spawn {
    pub prefab: &'static str,
    pub place: Place,
}

/// The host's side of the round, beside [`Round`]: what to spawn, when the
/// next order comes, and the generator that picks it.
#[derive(Debug, Default)]
pub struct Service {
    pub spawns: Vec<Spawn>,
    pub next_order: f32,
    pub seed: u32,
    /// Seconds until each plate that went out of the window is back in
    /// the sink, dirty.
    pub returning: Vec<f32>,
}

/// An entity and everything under it, gone.
pub fn despawn_tree(world: &mut World, root: Entity) {
    let mut gone = vec![root];
    let mut i = 0;
    while i < gone.len() {
        let parent = gone[i];
        gone.extend(
            world
                .query::<(Entity, &runity::world::Parent)>()
                .iter()
                .filter(|(_, p)| p.0 == parent)
                .map(|(e, _)| e),
        );
        i += 1;
    }
    for e in gone {
        let _ = world.despawn(e);
    }
}

/// The children of `parent` with a mark, by its name.
pub fn marked(world: &World, parent: Entity) -> Vec<(Entity, String)> {
    world
        .query::<(Entity, &runity::world::Parent, &crate::components::Mark)>()
        .iter()
        .filter(|(_, p, _)| p.0 == parent)
        .map(|(e, _, m)| (e, m.name.clone()))
        .collect()
}

/// The kitchen's line, and whether this peer runs the rules on it: the
/// host does (alone, that is this one).
pub fn kitchen(world: &World) -> Option<(Entity, bool)> {
    world
        .query::<(Entity, &crate::components::Kitchen, Option<&runity::net::Owned>)>()
        .iter()
        .next()
        .map(|(e, _, owned)| (e, owned.is_some()))
}

/// The cook with this index.
pub fn cook(world: &World, index: u32) -> Option<Entity> {
    world
        .query::<(Entity, &crate::components::Player)>()
        .iter()
        .find(|(_, p)| p.index == index)
        .map(|(e, _)| e)
}

/// What an act does on the host: a cook's buttons for the next steps.
pub fn apply(world: &mut World, act: Act) {
    match act {
        Act::Grab { cook: index } => {
            if let Some(c) = cook(world, index) {
                let mut controls = world.get::<&Controls>(c).map(|c| *c).unwrap_or_default();
                controls.grab = true;
                let _ = world.insert_one(c, controls);
            }
        }
        Act::Throw { cook: index } => {
            if let Some(c) = cook(world, index) {
                let mut controls = world.get::<&Controls>(c).map(|c| *c).unwrap_or_default();
                controls.throw = true;
                let _ = world.insert_one(c, controls);
            }
        }
        Act::Work { cook: index, on } => {
            if let Some(c) = cook(world, index) {
                let mut controls = world.get::<&Controls>(c).map(|c| *c).unwrap_or_default();
                controls.work = on;
                let _ = world.insert_one(c, controls);
            }
        }
        Act::Restart => {
            if let Some((k, true)) = kitchen(world) {
                // Over, or not yet open: the lobby's start is this too.
                if world.get::<&Round>(k).is_ok_and(|r| r.over || !r.open) {
                    let _ = world.insert_one(k, Restart);
                }
            }
        }
    }
}

/// A thing in the air (the host's): the speed to start with, until the
/// step has given it to the physics, and how long it has lain still.
#[derive(Debug, Clone, Copy)]
pub struct Flying {
    pub launch: Option<Vec3>,
    pub last: Vec3,
    pub still: f32,
    pub age: f32,
}

/// How hard a cook throws: along the way they face, and up.
pub const THROW_SPEED: f32 = 6.5;
pub const THROW_LIFT: f32 = 2.2;
/// How big food is to the physics, metres: the fruit under the item.
pub const FOOD_RADIUS: f32 = 0.17;

/// Asked for on the kitchen: start the round again at the next step.
#[derive(Debug, Clone, Copy)]
pub struct Restart;
