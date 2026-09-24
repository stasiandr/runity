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
    components
        .register_state("soft.rope", scrap_soft::rope::gather_net, scrap_soft::rope::take_net)
        .register_state("soft.cloth", scrap_soft::cloth::gather_net, scrap_soft::cloth::take_net);
    #[cfg(feature = "character")]
    components.register_state("character.ragdoll", scrap_character::ragdoll::gather_net, scrap_character::ragdoll::take_net);
    #[cfg(feature = "destruction")]
    components
        .register_state("destruct.fracture", scrap_destruction::fracture::gather_net, scrap_destruction::fracture::take_net)
        .register_state("destruct.dents", scrap_destruction::dents::gather_net, scrap_destruction::dents::take_net);
    let _ = components;
}

/// The most a rope simulated elsewhere pulls a hand here, newtons.
pub const FELT_MOST: f32 = 5000.0;


/// The numbers of a rope held across machines ([`pull_bodies`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Across {
    /// How stiff it is at the hand of the machine that simulates it: of
    /// the stiffest spring on the hand's weight that stays steady at the
    /// physics' step.
    pub taut: f32,
    /// Its damping, of critical.
    pub damping: f32,
    /// Of the pull as its owner said it, the share of the last kept each
    /// thirtieth of a second at the far hand: a low-pass over bursts.
    pub keep: f32,
    /// At the far hand the said pull holds only while the hands are this
    /// share of the rope's length apart or more.
    pub gate: f32,
    /// The far hand's catch: how stiff, of the same stiffest spring…
    pub catch: f32,
    /// …its damping, of critical…
    pub catch_damping: f32,
    /// …and how far past the rope's length it lets the hand run first.
    pub slack: f32,
}

impl Default for Across {
    fn default() -> Self {
        Self { taut: 0.2, damping: 1.0, keep: 0.75, gate: 0.95, catch: 0.01, catch_damping: 0.3, slack: 0.03 }
    }
}

/// The numbers every rope held across machines goes by: a bench's to try.
/// Found by the bench's sweep (`sweep_one`): of 208 sets tried on two
/// links and several seeds each, these kept the rope within 5% of its
/// length where it is simulated, let the stronger player get 60–80% as
/// far as alone, and never swung or flung anything.
pub static ACROSS: std::sync::RwLock<Across> = std::sync::RwLock::new(Across {
    taut: 0.2,
    damping: 1.0,
    keep: 0.75,
    gate: 0.95,
    catch: 0.01,
    catch_damping: 0.3,
    slack: 0.03,
});

/// The pieces an `Event` fracture broke into, made the network's: each
/// gets an id made of what it broke off and its number — the same on
/// every peer, which broke it the same way — and what it broke off's
/// owner, who drives them from here as ordinary bodies (docs/netsim.md).
#[cfg(all(feature = "destruction", feature = "net"))]
pub fn own_pieces(world: &mut World, pieces: &[(hecs::Entity, glam::Vec3)]) {
    use scrap_destruction::{Breakable, Piece};
    for (piece, _) in pieces {
        let Some((from, index)) = world.get::<&Piece>(*piece).ok().map(|p| (p.from, p.index)) else { continue };
        let event = world
            .get::<&Breakable>(from)
            .is_ok_and(|b| b.fracture.net == crate::netsim::NetMode::Event);
        if !event {
            continue;
        }
        let Some(parent) = crate::net::network_id(world, from) else { continue };
        let id = piece_id(parent, index);
        let owner = world.get::<&crate::net::Owner>(from).map(|o| *o).unwrap_or(crate::net::Owner(crate::net::PeerId::HOST));
        let _ = world.insert(*piece, (crate::net::NetId(id), owner));
        // Broken here: asked for too, so the server has them as ours (it
        // would take a piece it has never heard of for the host's).
        if world.get::<&crate::net::Owned>(from).is_ok() {
            let _ = world.insert_one(*piece, crate::net::RequestOwnership);
        }
        // Pieces of pieces break by events too, and so on down.
        if let Ok(mut b) = world.get::<&mut Breakable>(*piece) {
            b.fracture.net = crate::netsim::NetMode::Event;
        }
    }
}

/// The id of piece `index` of what had id `parent`: the same on every
/// peer, and never one a scene or a spawn would give.
#[cfg(all(feature = "destruction", feature = "net"))]
pub fn piece_id(parent: crate::EntityId, index: u32) -> crate::EntityId {
    let mixed = parent.raw().wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(17) ^ (index as u64 + 1).wrapping_mul(0x2545_f491_4f6c_dd1d);
    crate::EntityId::from_raw(mixed | (1 << 62))
}

/// How far ahead a body closing on another is claimed: a quarter of a
/// second, as the dacha simulator's `ApproachClaimSystem`.
pub const AHEAD: f32 = 0.25;

/// Claims on approach (docs/netsim.md; DNA, open question 3, closed by
/// the engine): a loose body of someone else's that a body of ours will
/// meet within [`AHEAD`] is asked for now, so that the impact is solved
/// on one machine — a body someone else drives is kinematic here, and two
/// machines each solving one impact against an immovable copy of the
/// other get two answers, neither the one a single machine gets (two
/// crates shoved head-on at 9 m/s came out at 17.9 against 8.5 alone, in
/// the dacha simulator's bench). Two closing on each other: the lower
/// peer's number gathers, or both would take it from each other for
/// ever. A player's body is never taken, and what is already being
/// asked for is left alone. Call before the physics' step.
#[cfg(all(feature = "net", feature = "physics"))]
pub fn claim_approaching(world: &mut World, physics: &crate::physics::PhysicsWorld) {
    use crate::bodies::{Physics, Shape};
    use crate::body::Body;
    use crate::net::{Owned, Owner, OwnershipPending, PeerId, Replica, RequestOwnership};
    use crate::world::WorldTransform;
    let reach = |world: &World, e: hecs::Entity| -> f32 {
        let scale = world.get::<&WorldTransform>(e).map(|p| p.0.to_scale_rotation_translation().0.abs().max_element()).unwrap_or(1.0);
        let r = match world.get::<&Shape>(e).map(|s| s.0) {
            Ok(crate::body::Collider::Box { half, center }) => half.length() + center.length(),
            Ok(crate::body::Collider::Sphere { radius }) => radius,
            Ok(crate::body::Collider::Capsule { half_height, radius }) | Ok(crate::body::Collider::Cylinder { half_height, radius }) => half_height + radius,
            _ => 0.5,
        };
        r * scale
    };
    let owner = |world: &World, e: hecs::Entity| world.get::<&Owner>(e).map(|o| o.0).unwrap_or(PeerId::HOST);
    let dynamic = |world: &World, e: hecs::Entity| matches!(world.get::<&Physics>(e).map(|p| p.0), Ok(Body::Dynamic));
    let at = |world: &World, e: hecs::Entity| world.get::<&WorldTransform>(e).map(|p| p.0.w_axis.truncate()).unwrap_or_default();
    let mine: Vec<(hecs::Entity, glam::Vec3, glam::Vec3, f32, PeerId)> = world
        .query::<(hecs::Entity, &Owned)>()
        .without::<&OwnershipPending>()
        .iter()
        .map(|(e, _)| e)
        .filter(|e| dynamic(world, *e))
        .filter_map(|e| {
            let v = physics.velocity(world, e)?;
            (v.length() > 0.5).then(|| (e, at(world, e), v, reach(world, e), owner(world, e)))
        })
        .collect();
    if mine.is_empty() {
        return;
    }
    let theirs: Vec<hecs::Entity> = world
        .query::<(hecs::Entity, &Replica)>()
        .without::<(&OwnershipPending, &RequestOwnership, &Pawn)>()
        .iter()
        .map(|(e, _)| e)
        .filter(|e| dynamic(world, *e))
        .collect();
    let mut ask = Vec::new();
    for other in theirs {
        let (there, radius, them) = (at(world, other), reach(world, other), owner(world, other));
        let v_there = predicted_velocity(world, other);
        for (_, here, v_here, r, me) in &mine {
            let d = there - *here;
            let v = *v_here - v_there;
            let speed2 = v.length_squared();
            if speed2 < 1e-6 {
                continue;
            }
            let t = (d.dot(v) / speed2).clamp(0.0, AHEAD);
            let miss = (d - v * t).length();
            if miss > r + radius + 0.05 {
                continue;
            }
            // Coming at us too: the lower number gathers.
            let closing = v_there.dot(-d.normalize_or_zero()) > 0.5;
            if closing && me.0 > them.0 {
                continue;
            }
            ask.push(other);
            break;
        }
    }
    for e in ask {
        let _ = world.insert_one(e, RequestOwnership);
    }
}

/// Before the soft step: each rope's end held by a body this peer
/// simulates — a dynamic body, not someone else's replica — is anchored
/// to it, as heavy as it is and as fast as the point it holds.
#[cfg(all(feature = "soft", feature = "physics"))]
pub fn anchor_ropes(world: &mut World, physics: &mut crate::physics::PhysicsWorld) {
    use crate::bodies::Physics;
    use crate::body::Body;
    use crate::world::{Replica, WorldTransform};
    use scrap_soft::{Anchor, Ends, RopeState};
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
            (Some(e), Some(at)) if both && movable(world, e) && !movable(world, entity) && world.get::<&Replica>(entity).is_err() => {
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
        // Held at both ends, and one of them someone else's body: the rope
        // couples the two bodies across machines as a spring, each side
        // worked out by its own owner ([`pull_bodies`]), and the solver
        // here only gives it its shape between the hands. Held by a body
        // here against a post, or by two bodies here, it holds them itself.
        let remote = |e: hecs::Entity| world.get::<&Replica>(e).is_ok();
        let across = both && (remote(entity) || end.is_some_and(remote));
        let first = if across { None } else { anchor(entity, start_at) };
        let last = match (end, end_at) {
            (Some(e), Some(at)) if both && !across => anchor(e, at),
            _ => None,
        };
        // An end held by someone else's body, on a rope simulated here: held
        // where its owner has it now, not where the picture of it is — the
        // picture is a couple of ticks and the way here late, and a rope
        // pulled by a hand that late tugs behind the hand.
        let predicted = |e: hecs::Entity, to: glam::Vec3| predicted_point(world, e, to);
        let pins = if world.get::<&Replica>(entity).is_ok() {
            [None, None]
        } else {
            let to = world.get::<&RopeState>(entity).map(|s| s.rope.to).unwrap_or_default();
            [
                if first.is_none() { predicted(entity, glam::Vec3::ZERO) } else { None },
                match end {
                    Some(e) if both && last.is_none() => predicted(e, to),
                    _ => None,
                },
            ]
        };
        if let Ok(mut state) = world.get::<&mut RopeState>(entity) {
            state.anchors = [first, last];
            state.pins = pins;
            // Tied into what holds it, not lying on it.
            state.ignores = [Some(entity), end].into_iter().flatten().map(|e| e.to_bits().get()).collect();
        }
    }
}

/// Where someone else's entity is now at its owner's, by its buffer: the
/// point `to` in its space. `None` for what is not shown from a buffer.
#[cfg(all(feature = "soft", feature = "physics"))]
fn predicted_point(world: &World, entity: hecs::Entity, to: glam::Vec3) -> Option<glam::Vec3> {
    #[cfg(feature = "net")]
    {
        let presented = world.get::<&crate::net::sync::Presented>(entity).ok()?;
        let one_way = crate::netsim::link_delay(world);
        let (pose, _, _) = presented.takeover(one_way)?;
        Some(pose.position + pose.rotation() * (to * pose.scale))
    }
    #[cfg(not(feature = "net"))]
    {
        let _ = (world, entity, to);
        None
    }
}



/// How fast someone else's entity goes at its owner's, by its buffer. The
/// ropes ask it, and so do approach claims, which a game without ropes has.
#[cfg(all(feature = "physics", any(feature = "soft", feature = "net")))]
fn predicted_velocity(world: &World, entity: hecs::Entity) -> glam::Vec3 {
    #[cfg(feature = "net")]
    {
        let one_way = crate::netsim::link_delay(world);
        if let Ok(presented) = world.get::<&crate::net::sync::Presented>(entity) {
            if let Some((_, v, _)) = presented.takeover(one_way) {
                return v;
            }
        }
    }
    let _ = (world, entity);
    glam::Vec3::ZERO
}

/// After the soft step: what each rope did to the bodies holding it, into
/// the physics for its next step.
#[cfg(all(feature = "soft", feature = "physics"))]
pub fn pull_bodies(world: &mut World, physics: &mut crate::physics::PhysicsWorld) {
    use crate::world::WorldTransform;
    use scrap_soft::RopeState;
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
    // A rope held across machines — one end's body here, the other's
    // someone else's. The machine that simulates the rope holds its own
    // hand to the other end as a stiff spring: the hand is dragged after
    // the other body as it is seen now (predicted), and the pull is sent,
    // in the rope's frame, to the other body's owner — one force for both
    // hands, so what one gains the other loses (the saw's rule of the
    // dacha simulator: the effect is applied by whoever owns the target).
    // It arrives late, so the far hand takes it only while the rope is
    // taut there too — a pull from a moment that is over would sling the
    // hand back when the players let go — and meanwhile is caught at once
    // by a spring of its own if it runs off past the rope's length at its
    // owner's. The numbers are [`Across`]'s.
    let tune = *ACROSS.read().unwrap_or_else(|e| e.into_inner());
    let dt = physics.dt().max(1e-4);
    let mut held = Vec::new();
    for (entity, state, placed) in world.query::<(hecs::Entity, &RopeState, &WorldTransform)>().iter() {
        if state.rope.ends != scrap_soft::Ends::Both {
            continue;
        }
        let Some(length) = state.rod().map(|r| r.length()) else { continue };
        held.push((entity, placed.0.w_axis.truncate(), state.rope.to, state.rope.end, length));
    }
    for (entity, start, to, end, length) in held {
        let Some(end_entity) = end.get(world) else { continue };
        let remote = |e: hecs::Entity| world.get::<&crate::world::Replica>(e).is_ok();
        if !(remote(entity) || remote(end_entity)) {
            continue;
        }
        let rope_here = !remote(entity);
        let holders = [(entity, glam::Vec3::ZERO), (end_entity, to)];
        for k in 0..2 {
            let (holder, offset) = holders[k];
            let (other, other_offset) = holders[1 - k];
            let here = matches!(world.get::<&crate::bodies::Physics>(holder).map(|p| p.0), Ok(crate::body::Body::Dynamic))
                && !remote(holder);
            if !here || !remote(other) {
                continue;
            }
            let at = if k == 0 {
                start
            } else {
                match world.get::<&WorldTransform>(holder) {
                    Ok(p) => p.0.transform_point3(offset),
                    Err(_) => continue,
                }
            };
            let there = predicted_point(world, other, other_offset)
                .or_else(|| world.get::<&WorldTransform>(other).ok().map(|p| p.0.transform_point3(other_offset)));
            let Some(there) = there else { continue };
            let apart = at - there;
            let far = apart.length();
            if far < 1e-6 {
                continue;
            }
            let out = apart / far;
            let mass = physics.mass(world, holder).unwrap_or(1.0).max(1e-3);
            let v_here = physics.velocity_at(world, holder, at).unwrap_or_default();
            let v_there = predicted_velocity(world, other);
            let opening = (v_here - v_there).dot(out);
            let spring = |taut: f32, damping: f32, beyond: f32| {
                if far <= beyond {
                    return 0.0;
                }
                let stiffness = mass * taut / (dt * dt);
                let damping = 2.0 * damping * (stiffness * mass).sqrt();
                stiffness * (far - beyond) + damping * opening.max(0.0)
            };
            let force = if rope_here {
                // The spring, and its pull said to the other hand's owner.
                let force = (-out * spring(tune.taut, tune.damping, length)).clamp_length_max(FELT_MOST);
                if let Ok(mut state) = world.get::<&mut RopeState>(entity) {
                    state.pulls[k] = force;
                    state.pulls[1 - k] = -force;
                }
                force
            } else {
                let long = world.get::<&RopeState>(entity).ok().and_then(|s| s.owner_length).unwrap_or(length).max(length);
                let said = world.get::<&RopeState>(entity).map(|s| s.pulls[k].length()).unwrap_or(0.0);
                let felt = world.get::<&RopeState>(entity).map(|s| s.felt[k].length()).unwrap_or(0.0);
                // `keep` is per thirtieth of a second, whatever the step.
                let keep = tune.keep.powf(dt * 30.0);
                let felt = felt * keep + said * (1.0 - keep);
                let taut = far >= long * tune.gate;
                let pull = if taut { felt } else { 0.0 };
                let catch = spring(tune.catch, tune.catch_damping, long * (1.0 + tune.slack));
                if let Ok(mut state) = world.get::<&mut RopeState>(entity) {
                    state.felt[k] = -out * felt;
                }
                (-out * (pull + catch)).clamp_length_max(FELT_MOST)
            };
            physics.add_force_at(world, holder, force, at);
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
/// The bench's scenes.
#[cfg(all(feature = "net", feature = "physics", feature = "soft"))]
pub mod scenes;

pub use scrap_core::netsim::*;
