//! Card #41, to be played rather than read: the valley with a person in it.
//!
//! ```text
//! cargo run --release --example demo
//! ```
//!
//! Opens a window at the campfire and waits. Everything the card added is on
//! the other side of a key, and the keys are listed in the corner of the
//! screen — `F` takes the list away once it is in the way:
//!
//! * **`W` `A` `S` `D`, mouse or arrow keys.** The walk goes where the head is
//!   looking, and the head turns every frame rather than every tick. Hold
//!   shift to run — 3.4 m/s against 1.7 — and watch the breath bar under the
//!   status panel empty; when it does, the run shortens back to a walk and the
//!   jump gets lower. Stand still and it fills again.
//! * **Space.** A jump, latched so that pressing it in a frame that runs no
//!   fixed tick still leaves the ground. At 15 Hz there are three such frames
//!   for every one that ticks, which is the point of the next key.
//! * **`T`.** Cycles the whole simulation — clock and physics together —
//!   between 60, 30, 20 and 15 Hz, live, with the rate in the corner. The walk
//!   stays the same speed at every one of them; what changes is how long the
//!   world waits before it agrees you pressed something. The head never waits:
//!   it turns on the frame, which is why 15 Hz feels sticky rather than slow.
//! * **`E`.** Fells the nearest trunk within reach. It comes down as a
//!   pendulum, shoves anyone standing under it aside without hurting them, and
//!   lies there as a log — 0.3 m, which the next pass steps over rather than
//!   walks around. The camera will say so: the eye rises over about 0.15 s
//!   instead of teleporting.
//! * **`B`.** Puts a wall down where you stand. It blocks immediately, and
//!   because you were standing inside it, it walks you out of the nearest face
//!   over half a second.
//! * **`M`.** The previous card's top-down debug view, straight down and at a
//!   fixed scale: the static grid, every footprint, and a red segment for each
//!   contact the body currently has. Useful for watching a settler slide along
//!   a trunk rather than find a path around it.
//!
//! Twelve settlers are wandering around the camp when it opens — different
//! heights and different shirts, no two the same — and they are the quickest
//! thing to try: run into them and they give way, and so do you.
//!
//! Thrown away when the card lands. Everything it drives is [`valley`], which
//! stays.

#[allow(dead_code)]
#[path = "valley.rs"]
mod valley;

use runity::prelude::*;
use std::io;

const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;

fn main() -> io::Result<()> {
    println!("valley — WASD to walk, shift to run, mouse or arrows to look.");
    println!("E fells a tree, B builds a wall, T changes the tick rate, Escape quits.");
    App::new(WindowConfig::new("runity — card #41", WIDTH, HEIGHT)).run(valley::Valley::live())?;
    Ok(())
}
