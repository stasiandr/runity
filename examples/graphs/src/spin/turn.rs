//! Turns everything that has a `Spin` and this player drives: what someone
//! else drives turns on their machine and is shown turning here. Every
//! system that simulates asks for `Owned`; alone, everything is.

use scrap::hecs::World;
use scrap::world::Owned;
use scrap::Transform;

use crate::components::Spin;

pub fn run(world: &mut World, seconds: f32) {
    for (transform, spin) in world.query_mut::<(&mut Transform, &Spin)>().with::<&Owned>() {
        transform.rotation_deg.y += spin.degrees_per_second * seconds;
    }
}
