//! How the characters move, on every peer: each one's speed, from how far
//! it went since the last step, to its animator (`speed` in
//! animators/cook.ron and walker.ron) — so a cook stands, walks or runs,
//! whoever drives it; dust from a running cook's feet; and the customers,
//! whom a route carries without turning them, turned the way they go.

use runity::animgraph::Controller;
use runity::glam::Vec3;
use runity::hecs::{Entity, World};
use runity::Transform;

use crate::components::Player;
use crate::state::marked;

/// Where a character was at the last step, and how fast it goes, smoothed.
#[derive(Debug, Clone, Copy)]
pub struct Stride {
    pub last: Vec3,
    pub speed: f32,
}

pub fn run(world: &mut World, seconds: f32) {
    if seconds <= 0.0 {
        return;
    }
    let mut dusty = Vec::new();
    let movers: Vec<(Entity, Vec3, bool)> = world
        .query::<(Entity, &Transform, &Controller, Option<&Player>)>()
        .iter()
        .map(|(e, t, _, p)| (e, t.position, p.is_some()))
        .collect();
    for (entity, at, cook) in movers {
        let stride = world.get::<&Stride>(entity).map(|s| *s).ok();
        let Some(mut stride) = stride else {
            let _ = world.insert_one(entity, Stride { last: at, speed: 0.0 });
            continue;
        };
        let moved = Vec3::new(at.x - stride.last.x, 0.0, at.z - stride.last.z);
        let now = moved.length() / seconds;
        // Smoothed: a network snapshot arriving in steps is not a stumble.
        stride.speed += (now - stride.speed) * (seconds * 10.0).min(1.0);
        stride.last = at;
        let _ = world.insert_one(entity, stride);
        if let Ok(mut controller) = world.get::<&mut Controller>(entity) {
            controller.set("speed", stride.speed);
        }
        // A route turns nobody: a customer faces the way it walks.
        if !cook && moved.length() > 1e-4 {
            if let Ok(mut t) = world.get::<&mut Transform>(entity) {
                t.rotation_deg.y = moved.x.atan2(moved.z).to_degrees();
            }
        }
        if cook {
            dusty.push((entity, stride.speed > 2.5));
        }
    }
    for (cook, running) in dusty {
        for (mark, name) in marked(world, cook) {
            if name == "dust" {
                let off = world.get::<&runity::world::Inactive>(mark).is_ok();
                if off == running {
                    runity::world::set_active(world, mark, running);
                }
            }
        }
    }
}
