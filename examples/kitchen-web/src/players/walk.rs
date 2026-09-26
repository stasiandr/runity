//! Cooks walk where their keys say, turn to face the way they go, and stop
//! at the counters: every station is a tile a metre square they cannot
//! stand in, and the room has walls. Each player walks their own cook —
//! the one this peer owns; the others' come over the network.

use scrap::glam::Vec3;
use scrap::hecs::World;
use scrap::net::Owned;
use scrap::Transform;

use crate::components::{Player, Station};
use crate::state::Controls;

/// How far a cook's middle stays from a counter's edge.
pub const RADIUS: f32 = 0.35;

pub fn run(world: &mut World, seconds: f32) {
    let tiles: Vec<Vec3> = world
        .query::<(&Transform, &Station)>()
        .iter()
        .map(|(t, _)| t.position)
        .collect();
    // The room: as far as its stations go.
    let (lo, hi) = tiles.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    for (transform, player, controls) in world
        .query_mut::<(&mut Transform, &Player, &Controls)>()
        .with::<&Owned>()
    {
        let wish = Vec3::new(controls.x, 0.0, controls.z);
        if wish.length_squared() < 0.01 {
            continue;
        }
        let dir = wish.normalize_or_zero() * wish.length().min(1.0);
        let facing = dir.normalize_or_zero();
        transform.rotation_deg.y = facing.x.atan2(facing.z).to_degrees();
        // Each axis on its own, so a cook slides along a counter.
        let step = dir * player.speed * seconds;
        for axis in [0usize, 2] {
            let mut next = transform.position;
            next[axis] += step[axis];
            let blocked = tiles.iter().any(|t| {
                (next.x - t.x).abs() < 0.5 + RADIUS && (next.z - t.z).abs() < 0.5 + RADIUS
            });
            let inside = next.x > lo.x && next.x < hi.x && next.z > lo.z && next.z < hi.z;
            if !blocked && inside {
                transform.position = next;
            }
        }
    }
}
