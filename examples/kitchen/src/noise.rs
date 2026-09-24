//! The kitchen's sounds, heard from what changes in it — the same on every
//! peer, since everyone has the round, the pots and the chopping as the
//! host sends them: a ding for a soup served, a buzz for points lost, a
//! pop for something picked up or a plate washed, a plop into a pot or onto
//! a plate, a knock as a knife goes. The pots' boiling, the pans' sizzle
//! and the music are the scene's own `sound`s.

use runity::hecs::World;

use crate::components::Item;
use crate::state::{Chop, Pot, Round, Served, Stack};

/// What was heard last frame.
#[derive(Debug, Default)]
pub struct Noise {
    heard: Option<Heard>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Heard {
    score: i32,
    served: u32,
    items: usize,
    in_pots: usize,
    on_plates: usize,
    racked: u32,
    chopped: f32,
}

impl Noise {
    /// The sounds this frame, by name.
    pub fn listen(&mut self, world: &World) -> Vec<&'static str> {
        let Some(round) = world.query::<&Round>().iter().next().cloned() else {
            self.heard = None;
            return Vec::new();
        };
        let now = Heard {
            score: round.score,
            served: round.served,
            items: world.query::<&Item>().iter().count(),
            in_pots: world.query::<&Pot>().iter().map(|p| p.foods.len()).sum(),
            on_plates: world.query::<&Served>().iter().map(|s| s.parts.len()).sum(),
            racked: world.query::<&Stack>().iter().map(|s| s.0).sum(),
            chopped: world.query::<&Chop>().iter().map(|c| c.0).sum(),
        };
        let mut out = Vec::new();
        if let Some(was) = self.heard {
            if now.served > was.served {
                out.push("ding");
            } else if now.score < was.score {
                out.push("buzz");
            }
            if now.items > was.items {
                out.push("pop");
            }
            if now.in_pots > was.in_pots || now.on_plates > was.on_plates {
                out.push("plop");
            }
            if now.racked > was.racked {
                out.push("pop");
            }
            // A knock every so far along a chop.
            if (now.chopped * 4.0).floor() > (was.chopped * 4.0).floor() {
                out.push("chop");
            }
        }
        // A knock is counted from where the chopping is, not from zero.
        self.heard = Some(now);
        out
    }
}
