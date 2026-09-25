//! Jiggle bones: a line's `jiggle`. `jiggle: ()` on a child makes it
//! trail its parent as a spring does — an antenna, a ponytail, a tail, a
//! belly, the cards of a head of hair (hair cards with physics on bones),
//! a lantern on a cart. Secondary motion: the parent is moved by whatever
//! moves it — a route, a clip, the physics — and the child follows late,
//! overshoots and settles.
//!
//! Each jiggling entity is the tip of a bone from its parent: the tip is a
//! point carried on by where it was going (Verlet), pulled back toward
//! where the bone rests as `stiffness` says, pulled down by `gravity`, and
//! held the bone's length from the parent; the entity is put there, turned
//! as the bone has turned. A chain of them — each a child of the last —
//! swings as one, root first; whatever hangs off a jiggling entity (a card,
//! a model) goes with it (Unity's spring bones, VRM's).

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use scrap_core::world::{Parent, WorldTransform};

/// A jiggle bone, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Jiggle {
    /// How hard it springs back to rest, 0 (hangs loose) to 1 (stiff).
    pub stiffness: f32,
    /// How much of its swing it loses, 0 (rings on) to 1 (dead).
    pub damping: f32,
    /// How much it sags under gravity, 0 (not at all) to 1.
    pub gravity: f32,
    /// How it goes over the network (docs/netsim.md): `Local` unless
    /// the line says.
    #[serde(default, skip_serializing_if = "scrap_core::netsim::NetMode::is_local")]
    pub net: scrap_core::netsim::NetMode,
}

impl Default for Jiggle {
    fn default() -> Self {
        Self {
            stiffness: 0.3,
            damping: 0.15,
            gravity: 0.3,
            net: scrap_core::netsim::NetMode::Local,
        }
    }
}

scrap_core::impl_parts! {
    Jiggle => "jiggle", fractions ["stiffness", "damping", "gravity"];
}

/// The jiggle of a line, read off it.
pub trait JiggleLine {
    fn jiggle(&self) -> Option<Jiggle>;
}

impl JiggleLine for scrap_core::EntityDesc {
    fn jiggle(&self) -> Option<Jiggle> {
        self.part()
    }
}

impl JiggleLine for scrap_core::scene::Override {
    fn jiggle(&self) -> Option<Jiggle> {
        self.part()
    }
}

pub const STEP: f32 = 1.0 / 60.0;

/// A jiggling bone's tip as it moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JiggleState {
    pub jiggle: Jiggle,
    tip: Option<(Vec3, Vec3)>,
    owed: f32,
}

impl JiggleState {
    pub fn new(jiggle: Jiggle) -> Self {
        Self { jiggle, tip: None, owed: 0.0 }
    }

    /// Where its tip is, once it has moved.
    pub fn tip(&self) -> Option<Vec3> {
        self.tip.map(|t| t.0)
    }

    /// The tip on by `steps` fixed steps, the bone from `anchor` to where
    /// it would rest, `rest`.
    fn swing(&mut self, anchor: Vec3, rest: Vec3, steps: u32) -> Vec3 {
        let length = anchor.distance(rest);
        let (mut now, mut was) = self.tip.unwrap_or((rest, rest));
        let j = self.jiggle;
        for _ in 0..steps {
            let keep = 1.0 - j.damping.clamp(0.0, 1.0) * 0.5;
            let mut next = now + (now - was) * keep + Vec3::new(0.0, -9.81, 0.0) * (j.gravity.max(0.0) * STEP * STEP);
            next += (rest - next) * (j.stiffness.clamp(0.0, 1.0) * 0.25);
            // Held its length from the parent.
            next = anchor + (next - anchor).normalize_or(rest - anchor) * length;
            was = now;
            now = next;
        }
        self.tip = Some((now, was));
        now
    }
}

/// Every jiggle bone on by `seconds`, root first, and what hangs off each
/// moved with it.
pub fn run_jiggle(world: &mut hecs::World, seconds: f32) {
    if world.query::<&JiggleState>().iter().next().is_none() {
        return;
    }
    // How deep each entity with a parent is: roots first.
    let parents: scrap_core::hash::FastMap<hecs::Entity, hecs::Entity> =
        world.query::<(hecs::Entity, &Parent)>().iter().map(|(e, p)| (e, p.0)).collect();
    let depth = |mut e: hecs::Entity| {
        let mut d = 0;
        while let Some(p) = parents.get(&e) {
            e = *p;
            d += 1;
            if d > 64 {
                break;
            }
        }
        d
    };
    let mut order: Vec<(usize, hecs::Entity)> = parents.keys().map(|e| (depth(*e), *e)).collect();
    order.sort_by_key(|(d, e)| (*d, e.to_bits()));
    // What each entity is to its parent: its own transform, or, for one
    // without, as the hierarchy placed it.
    let mut local: scrap_core::hash::FastMap<hecs::Entity, Mat4> = Default::default();
    for (_, e) in &order {
        if let Ok(own) = world.get::<&scrap_core::scene::Transform>(*e) {
            local.insert(*e, own.matrix());
            continue;
        }
        let (Ok(me), Ok(parent)) = (world.get::<&WorldTransform>(*e), world.get::<&WorldTransform>(parents[e])) else {
            continue;
        };
        local.insert(*e, parent.0.inverse() * me.0);
    }
    let mut moved: scrap_core::hash::FastSet<hecs::Entity> = Default::default();
    for (_, e) in order {
        let Some(offset) = local.get(&e).copied() else { continue };
        let parent = parents[&e];
        let Ok(parent_placed) = world.get::<&WorldTransform>(parent).map(|p| p.0) else { continue };
        let rest = parent_placed * offset;
        let jiggling = world.get::<&JiggleState>(e).is_ok();
        let placed = if jiggling {
            let mut state = world.get::<&mut JiggleState>(e).unwrap();
            state.owed = (state.owed + seconds.max(0.0)).min(0.25);
            let steps = (state.owed / STEP) as u32;
            state.owed -= steps as f32 * STEP;
            let anchor = parent_placed.w_axis.truncate();
            let rest_tip = rest.w_axis.truncate();
            let tip = state.swing(anchor, rest_tip, steps);
            // Turned as the bone has turned, from its rest to its tip.
            let bend = Quat::from_rotation_arc(
                (rest_tip - anchor).normalize_or(Vec3::Y),
                (tip - anchor).normalize_or(Vec3::Y),
            );
            let (scale, turn, _) = rest.to_scale_rotation_translation();
            Some(Mat4::from_scale_rotation_translation(scale, bend * turn, tip))
        } else if moved.contains(&parent) {
            // Hanging off one that jiggled: it goes with it.
            Some(rest)
        } else {
            None
        };
        if let Some(placed) = placed {
            if let Ok(mut world_placed) = world.get::<&mut WorldTransform>(e) {
                world_placed.0 = placed;
            }
            moved.insert(e);
        }
    }
}

/// The soft module's dresser for jiggle bones.
pub struct JiggleDress;

impl scrap_core::world::Dress for JiggleDress {
    fn parts(&self) -> &[&'static str] {
        &["jiggle"]
    }

    fn dress(
        &mut self,
        line: &scrap_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: scrap_core::world::Changed,
        _: &mut Vec<scrap_core::world::Unresolved>,
    ) {
        match line.jiggle() {
            Some(jiggle) => {
                let _ = world.insert_one(entity, JiggleState::new(jiggle));
            }
            None => {
                let _ = world.remove_one::<JiggleState>(entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cart at `x`, with a two-bone antenna standing up on it and a
    /// card hanging off the top.
    fn cart() -> (hecs::World, [hecs::Entity; 4]) {
        let mut world = hecs::World::new();
        let base = world.spawn((WorldTransform(Mat4::IDENTITY),));
        let a = world.spawn((WorldTransform(Mat4::from_translation(Vec3::Y)), Parent(base), JiggleState::new(Jiggle::default())));
        let b = world.spawn((WorldTransform(Mat4::from_translation(Vec3::Y * 2.0)), Parent(a), JiggleState::new(Jiggle::default())));
        let card = world.spawn((WorldTransform(Mat4::from_translation(Vec3::new(0.2, 2.0, 0.0))), Parent(b)));
        (world, [base, a, b, card])
    }

    fn drive(world: &mut hecs::World, entities: [hecs::Entity; 4], x: f32) {
        // The hierarchy places everything where it rests: the base at x.
        let rests = [0.0, 1.0, 2.0, 2.0];
        for (i, e) in entities.iter().enumerate() {
            let side = if i == 3 { 0.2 } else { 0.0 };
            world.get::<&mut WorldTransform>(*e).unwrap().0 = Mat4::from_translation(Vec3::new(x + side, rests[i], 0.0));
        }
        run_jiggle(world, 1.0 / 30.0);
    }

    #[test]
    fn an_antenna_trails_as_the_cart_moves_off_and_settles_when_it_stops() {
        let (mut world, e) = cart();
        for _ in 0..10 {
            drive(&mut world, e, 0.0);
        }
        // Still: it stands as it rests, sagging a little.
        let top = world.get::<&WorldTransform>(e[2]).unwrap().0.w_axis.truncate();
        assert!(top.x.abs() < 0.05 && top.y > 1.9, "{top}");
        // The cart jerks off to +x: the top is left behind at first.
        for i in 1..=3 {
            drive(&mut world, e, i as f32 * 0.15);
        }
        let top = world.get::<&WorldTransform>(e[2]).unwrap().0.w_axis.truncate();
        assert!(top.x < 0.45 - 0.1, "trails: {top}");
        for i in 4..=8 {
            drive(&mut world, e, i as f32 * 0.15);
        }
        let top = world.get::<&WorldTransform>(e[2]).unwrap().0.w_axis.truncate();
        // Its length from the bone below it holds.
        let middle = world.get::<&WorldTransform>(e[1]).unwrap().0.w_axis.truncate();
        assert!((top.distance(middle) - 1.0).abs() < 1e-3, "{top} {middle}");
        // The card hangs off the top, where it is.
        let card = world.get::<&WorldTransform>(e[3]).unwrap().0.w_axis.truncate();
        assert!(card.distance(top) < 0.21 && card.distance(top) > 0.19, "{card} off {top}");
        // Stopped: it swings past and settles upright over the cart.
        for _ in 0..240 {
            drive(&mut world, e, 1.2);
        }
        let top = world.get::<&WorldTransform>(e[2]).unwrap().0.w_axis.truncate();
        assert!((top.x - 1.2).abs() < 0.05, "settles: {top}");
    }
}
