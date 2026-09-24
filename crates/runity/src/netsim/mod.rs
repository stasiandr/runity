//! Simulations over the network, where the modules meet
//! (docs/netsim.md): the states that go over the wire as their own bytes,
//! registered with the game's components, and what couples a soft thing
//! to the physics' bodies — a rope's end in a hand, a load on a cable.
//!
//! The modules do not know each other: `soft` knows ropes, `physics`
//! bodies, `net` blobs. This is the glue.

use hecs::World;

use crate::components::Components;

/// Register every simulation's network state this build has: a game
/// calls it where it registers its own networked components, and every
/// peer must, or the builds' fingerprints differ.
pub fn register(components: &mut Components) {
    #[cfg(feature = "soft")]
    components.register_state("soft.rope", runity_soft::rope::gather_net, runity_soft::rope::take_net);
    let _ = components;
}

/// Before the soft step: each rope's end held by a body this peer
/// simulates — a dynamic body, not someone else's replica — is anchored
/// to it, as heavy as it is and as fast as the point it holds.
#[cfg(all(feature = "soft", feature = "physics"))]
pub fn anchor_ropes(world: &mut World, physics: &mut crate::physics::PhysicsWorld) {
    use crate::bodies::Physics;
    use crate::body::Body;
    use crate::world::{Replica, WorldTransform};
    use runity_soft::{Anchor, Ends, RopeState};
    let movable = |world: &World, e: hecs::Entity| {
        matches!(world.get::<&Physics>(e).map(|p| p.0), Ok(Body::Dynamic)) && world.get::<&Replica>(e).is_err()
    };
    let mut todo = Vec::new();
    for (entity, state, placed) in world.query::<(hecs::Entity, &RopeState, &WorldTransform)>().iter() {
        let end = state.rope.end.get(world);
        let end_at = end
            .and_then(|e| world.get::<&WorldTransform>(e).ok().map(|p| p.0.transform_point3(state.rope.to)));
        todo.push((entity, placed.0.w_axis.truncate(), end, end_at, state.rope.ends == Ends::Both));
    }
    for (entity, start_at, end, end_at, both) in todo {
        // A body on the end of a rope pinned at its start is never further
        // than the rope reaches: one carried past it — at a handover, by a
        // shove — is put back on the reach at once, moving out no more, as
        // a stiff chain would hold it, rather than yanked back by the rope
        // over the next steps.
        let end_at = match (end, end_at) {
            (Some(e), Some(at)) if both && movable(world, e) && !movable(world, entity) => {
                let reach = world.get::<&RopeState>(entity).ok().and_then(|s| s.rod().map(|r| r.length()));
                match reach {
                    Some(reach) if at.distance(start_at) > reach * 1.001 => {
                        let out = (at - start_at).normalize();
                        let shift = start_at + out * reach - at;
                        let v = physics.velocity(world, e).unwrap_or_default();
                        let spin = physics.spin(world, e).unwrap_or_default();
                        let (position, rotation) = match world.get::<&crate::scene::Transform>(e) {
                            Ok(t) => (t.position + shift, t.rotation()),
                            Err(_) => continue,
                        };
                        physics.teleport(world, e, position, rotation);
                        physics.set_velocity(world, e, v - out * v.dot(out).max(0.0));
                        physics.set_spin(world, e, spin);
                        if let Ok(mut placed) = world.get::<&mut WorldTransform>(e) {
                            placed.0.w_axis += shift.extend(0.0);
                        }
                        Some(at + shift)
                    }
                    _ => Some(at),
                }
            }
            _ => end_at,
        };
        let anchor = |e: hecs::Entity, at: glam::Vec3| {
            if !movable(world, e) {
                return None;
            }
            Some(Anchor {
                mass: physics.mass(world, e)?,
                velocity: physics.velocity_at(world, e, at)?,
                impulse: glam::Vec3::ZERO,
                fresh: true,
            })
        };
        let first = anchor(entity, start_at);
        let last = match (end, end_at) {
            (Some(e), Some(at)) if both => anchor(e, at),
            _ => None,
        };
        if let Ok(mut state) = world.get::<&mut RopeState>(entity) {
            state.anchors = [first, last];
            // Tied into what holds it, not lying on it.
            state.ignores = [Some(entity), end].into_iter().flatten().map(|e| e.to_bits().get()).collect();
        }
    }
}

/// After the soft step: what each rope did to the bodies holding it, into
/// the physics for its next step.
#[cfg(all(feature = "soft", feature = "physics"))]
pub fn pull_bodies(world: &mut World, physics: &mut crate::physics::PhysicsWorld) {
    use crate::world::WorldTransform;
    use runity_soft::RopeState;
    let mut pulls = Vec::new();
    for (entity, state, placed) in world.query_mut::<(hecs::Entity, &mut RopeState, &WorldTransform)>() {
        let start = placed.0.w_axis.truncate();
        let to = state.rope.to;
        let end = state.rope.end;
        for (k, anchor) in state.anchors.iter_mut().enumerate() {
            if let Some(anchor) = anchor {
                if anchor.impulse != glam::Vec3::ZERO {
                    pulls.push((entity, k, anchor.impulse, start, to, end));
                    anchor.impulse = glam::Vec3::ZERO;
                }
            }
        }
    }
    for (entity, k, impulse, start, to, end) in pulls {
        let (body, at) = if k == 0 {
            (entity, start)
        } else {
            let Some(e) = end.get(world) else { continue };
            let Ok(at) = world.get::<&WorldTransform>(e).map(|p| p.0.transform_point3(to)) else { continue };
            (e, at)
        };
        physics.add_impulse_at(world, body, impulse, at);
    }
}

/// The simulations' bench: sessions of several peers in one process.
#[cfg(all(feature = "net", feature = "physics"))]
pub mod bench;

pub use runity_core::netsim::*;
