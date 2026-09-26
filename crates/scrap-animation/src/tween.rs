//! Tweens: one change from game code — the chest's lid flies open with a
//! bounce, the coin pops up and spins, the door slides in half a second
//! — DOTween's `DOMove`/`DORotate`/`DOScale` and Godot's `Tween`, as a
//! component (docs/feel.md).
//!
//! A tween drives the same [`Property`]s a motion clip's track does —
//! place, turn and scale by axis, `Active`, a sound's volume, particles'
//! rate — from where they are to where it says, over its seconds on its
//! [`Ease`], after a delay, once, several times or forever, there and
//! back if `yoyo`. It runs in the fixed step beside motion clips, on the
//! world's time: slow motion slows it and a hit-stop holds it.
//!
//! ```ignore
//! tween(world, lid, Tween::new("open", 0.4).turn(Vec3::new(-110.0, 0.0, 0.0)).ease(Ease::OutBack));
//! // A step later, when it is done:
//! if finished(world, lid, "open") { … }
//! ```
//!
//! No callbacks: what happens when it is done is a system reading
//! [`TweensDone`], as with a timer (`scrap_core::timers`).

use glam::Vec3;
use hecs::{Entity, World};
use serde::{Deserialize, Serialize};

use crate::ease::Ease;
use crate::motion::Property;

/// One change of some properties of an entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tween {
    /// What the game calls it when it is done.
    #[serde(default)]
    pub name: String,
    /// Where each property goes.
    pub to: Vec<(Property, f32)>,
    /// Seconds from where it is to where it goes, once.
    pub seconds: f32,
    #[serde(default)]
    pub ease: Ease,
    /// Seconds to wait before it starts; where it starts from is taken
    /// then.
    #[serde(default)]
    pub delay: f32,
    /// How many times it plays: 1 once, 0 forever.
    #[serde(default = "once")]
    pub times: u32,
    /// Every other play goes back rather than starting over.
    #[serde(default)]
    pub yoyo: bool,
    /// Where each property was when it started.
    #[serde(skip)]
    from: Option<Vec<f32>>,
    /// Seconds since it was given, delay and all.
    #[serde(skip)]
    elapsed: f32,
}

fn once() -> u32 {
    1
}

impl Tween {
    /// A tween called `name` taking `seconds`, of nothing yet: say where
    /// with [`Tween::position`], [`Tween::turn`], [`Tween::scale`] or
    /// [`Tween::set`].
    pub fn new(name: &str, seconds: f32) -> Self {
        Self {
            name: name.to_string(),
            to: Vec::new(),
            seconds,
            ease: Ease::Linear,
            delay: 0.0,
            times: 1,
            yoyo: false,
            from: None,
            elapsed: 0.0,
        }
    }

    /// And `what` to `value`.
    pub fn set(mut self, what: Property, value: f32) -> Self {
        self.to.retain(|(p, _)| *p != what);
        self.to.push((what, value));
        self
    }

    /// To this place, metres, in its parent's space.
    pub fn position(self, to: Vec3) -> Self {
        self.set(Property::X, to.x)
            .set(Property::Y, to.y)
            .set(Property::Z, to.z)
    }

    /// To this turn, degrees, as a line's `rotation` is written.
    pub fn turn(self, to_deg: Vec3) -> Self {
        self.set(Property::TurnX, to_deg.x)
            .set(Property::TurnY, to_deg.y)
            .set(Property::TurnZ, to_deg.z)
    }

    /// To this scale.
    pub fn scale(self, to: Vec3) -> Self {
        self.set(Property::ScaleX, to.x)
            .set(Property::ScaleY, to.y)
            .set(Property::ScaleZ, to.z)
    }

    pub fn ease(mut self, ease: Ease) -> Self {
        self.ease = ease;
        self
    }

    pub fn delay(mut self, seconds: f32) -> Self {
        self.delay = seconds;
        self
    }

    /// Play `times` times (0 forever), going back every other time if
    /// `yoyo`.
    pub fn repeat(mut self, times: u32, yoyo: bool) -> Self {
        self.times = times;
        self.yoyo = yoyo;
        self
    }

    /// Where it is, 0 to 1 along its curve, and whether it is done.
    fn at(&self) -> (f32, bool) {
        let played = (self.elapsed - self.delay).max(0.0);
        let one = self.seconds.max(1e-6);
        let plays = played / one;
        let done = self.times > 0 && plays >= self.times as f32;
        let (play, into) = if done {
            (self.times - 1, 1.0)
        } else {
            (plays.floor() as u32, plays.fract())
        };
        let back = self.yoyo && play % 2 == 1;
        let t = if back { 1.0 - into } else { into };
        (self.ease.at(t), done)
    }
}

/// The tweens running on an entity. [`tween`] gives one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tweens(pub Vec<Tween>);

/// The names of an entity's tweens that finished in the last step: there
/// for the game's systems of the next step, then gone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TweensDone(pub Vec<String>);

/// Start `tween` on `entity`. A tween already running on the same
/// properties is stopped where it is — two tweens pulling one lid are a
/// bug, and the newer is the one meant.
pub fn tween(world: &mut World, entity: Entity, tween: Tween) {
    if let Ok(mut running) = world.get::<&mut Tweens>(entity) {
        running.0.retain(|t| {
            !t.to
                .iter()
                .any(|(p, _)| tween.to.iter().any(|(q, _)| p == q))
        });
        running.0.push(tween);
        return;
    }
    let _ = world.insert_one(entity, Tweens(vec![tween]));
}

/// Whether a tween of this name is running on `entity`.
pub fn tweening(world: &World, entity: Entity, name: &str) -> bool {
    world
        .get::<&Tweens>(entity)
        .is_ok_and(|t| t.0.iter().any(|t| t.name == name))
}

/// Whether a tween of this name finished on `entity` in the last step.
pub fn finished(world: &World, entity: Entity, name: &str) -> bool {
    world
        .get::<&TweensDone>(entity)
        .is_ok_and(|d| d.0.iter().any(|n| n == name))
}

/// A property's value on an entity, as a tween starts from it.
fn read(
    world: &World,
    entity: Entity,
    what: Property,
    get: &dyn Fn(&World, Entity, Property) -> Option<f32>,
) -> f32 {
    let transform = world.get::<&crate::Transform>(entity).ok().map(|t| *t);
    let axis = |v: Vec3, i: usize| v[i];
    match (what, transform) {
        (Property::X, Some(t)) => axis(t.position, 0),
        (Property::Y, Some(t)) => axis(t.position, 1),
        (Property::Z, Some(t)) => axis(t.position, 2),
        (Property::TurnX, Some(t)) => axis(t.rotation_deg, 0),
        (Property::TurnY, Some(t)) => axis(t.rotation_deg, 1),
        (Property::TurnZ, Some(t)) => axis(t.rotation_deg, 2),
        (Property::ScaleX, Some(t)) => axis(t.scale, 0),
        (Property::ScaleY, Some(t)) => axis(t.scale, 1),
        (Property::ScaleZ, Some(t)) => axis(t.scale, 2),
        (Property::Active, _) => {
            if world.get::<&crate::world::Inactive>(entity).is_ok() {
                0.0
            } else {
                1.0
            }
        }
        _ => get(world, entity, what).unwrap_or(0.0),
    }
}

/// Every tween one step on: each property set to where its curve is now,
/// what finished taken off and named in [`TweensDone`]. A sound's volume
/// and particles' rate are read with `get` and written with `set`, as
/// motion clips hand them over ([`crate::motion::run_with`]).
/// In the fixed step, before the hierarchy is placed.
pub fn run_tweens(
    world: &mut World,
    dt: f32,
    get: &dyn Fn(&World, Entity, Property) -> Option<f32>,
    set: &mut dyn FnMut(&mut World, Entity, Property, f32),
) {
    let said: Vec<Entity> = world
        .query::<(Entity, &TweensDone)>()
        .iter()
        .map(|(e, _)| e)
        .collect();
    for entity in said {
        scrap_core::world::take_off::<TweensDone>(world, entity);
    }
    let running: Vec<Entity> = world
        .query::<(Entity, &Tweens)>()
        .iter()
        .map(|(e, _)| e)
        .collect();
    for entity in running {
        // Taken out and put back rather than the component removed: an
        // entity moved between archetypes every step costs more than the
        // tween.
        let Some(mut tweens) = world
            .get::<&mut Tweens>(entity)
            .ok()
            .map(|mut t| Tweens(std::mem::take(&mut t.0)))
        else {
            continue;
        };
        let mut done = Vec::new();
        let mut values = Vec::new();
        tweens.0.retain_mut(|tween| {
            tween.elapsed += dt;
            if tween.elapsed < tween.delay {
                return true;
            }
            if tween.from.is_none() {
                let from = tween
                    .to
                    .iter()
                    .map(|(what, _)| read(&*world, entity, *what, get))
                    .collect();
                tween.from = Some(from);
            }
            let (w, finished) = tween.at();
            let from = tween.from.as_deref().unwrap_or_default();
            for ((what, to), from) in tween.to.iter().zip(from.iter()) {
                values.push((*what, from + (to - from) * w));
            }
            if finished {
                done.push(tween.name.clone());
            }
            !finished
        });
        write(world, entity, &values, set);
        if tweens.0.is_empty() {
            scrap_core::world::take_off::<Tweens>(world, entity);
        } else if let Ok(mut left) = world.get::<&mut Tweens>(entity) {
            // Any given by `set` meanwhile go after.
            let given = std::mem::replace(&mut left.0, tweens.0);
            left.0.extend(given);
        }
        if !done.is_empty() {
            let _ = world.insert_one(entity, TweensDone(done));
        }
    }
}

fn write(
    world: &mut World,
    entity: Entity,
    values: &[(Property, f32)],
    set: &mut dyn FnMut(&mut World, Entity, Property, f32),
) {
    for &(what, value) in values {
        match what {
            Property::Active => {
                let on = value > 0.5;
                let off_now = world.get::<&crate::world::Inactive>(entity).is_ok();
                if off_now == on {
                    crate::world::set_active(world, entity, on);
                }
            }
            Property::Volume | Property::ParticleRate => set(world, entity, what, value),
            _ => {
                if let Ok(mut t) = world.get::<&mut crate::Transform>(entity) {
                    match what {
                        Property::X => t.position.x = value,
                        Property::Y => t.position.y = value,
                        Property::Z => t.position.z = value,
                        Property::TurnX => t.rotation_deg.x = value,
                        Property::TurnY => t.rotation_deg.y = value,
                        Property::TurnZ => t.rotation_deg.z = value,
                        Property::ScaleX => t.scale.x = value,
                        Property::ScaleY => t.scale.y = value,
                        Property::ScaleZ => t.scale.z = value,
                        _ => {}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(world: &mut World, dt: f32) {
        run_tweens(world, dt, &|_, _, _| None, &mut |_, _, _, _| {});
    }

    fn position(world: &World, e: Entity) -> Vec3 {
        world.get::<&crate::Transform>(e).unwrap().position
    }

    #[test]
    fn a_tween_goes_from_where_it_is_to_where_it_says_and_says_when_it_is_done() {
        let mut world = World::new();
        let e = world.spawn((crate::Transform {
            position: Vec3::new(1.0, 0.0, 0.0),
            ..Default::default()
        },));
        tween(
            &mut world,
            e,
            Tween::new("slide", 1.0)
                .position(Vec3::new(3.0, 2.0, 0.0))
                .ease(Ease::InQuad)
                .delay(0.5),
        );
        run(&mut world, 0.25);
        assert_eq!(
            position(&world, e),
            Vec3::new(1.0, 0.0, 0.0),
            "waits out its delay"
        );
        run(&mut world, 0.25);
        run(&mut world, 0.5);
        let half = position(&world, e);
        assert!(
            (half.x - 1.5).abs() < 1e-5,
            "a quarter of the way at half time: {half}"
        );
        assert!(tweening(&world, e, "slide"));
        assert!(!finished(&world, e, "slide"));
        run(&mut world, 0.5);
        assert_eq!(
            position(&world, e),
            Vec3::new(3.0, 2.0, 0.0),
            "exactly there"
        );
        assert!(finished(&world, e, "slide"), "said for the next step");
        assert!(!tweening(&world, e, "slide"));
        run(&mut world, 0.5);
        assert!(!finished(&world, e, "slide"), "and only for it");
        assert!(world.get::<&Tweens>(e).is_err(), "taken off");
    }

    #[test]
    fn a_yoyo_goes_there_and_back_and_forever_does_not_finish() {
        let mut world = World::new();
        let e = world.spawn((crate::Transform::default(),));
        tween(
            &mut world,
            e,
            Tween::new("bob", 1.0).set(Property::Y, 1.0).repeat(2, true),
        );
        run(&mut world, 1.0);
        assert_eq!(position(&world, e).y, 1.0, "there");
        run(&mut world, 0.5);
        assert_eq!(position(&world, e).y, 0.5);
        run(&mut world, 0.5);
        assert_eq!(position(&world, e).y, 0.0, "and back");
        assert!(finished(&world, e, "bob"));

        tween(
            &mut world,
            e,
            Tween::new("spin", 1.0)
                .set(Property::TurnY, 360.0)
                .repeat(0, false),
        );
        for _ in 0..100 {
            run(&mut world, 0.25);
        }
        assert!(tweening(&world, e, "spin"), "forever");
        let turn = world.get::<&crate::Transform>(e).unwrap().rotation_deg.y;
        assert!(turn < 360.0, "starts over each time: {turn}");
    }

    #[test]
    fn a_newer_tween_of_the_same_property_takes_over_and_others_carry_on() {
        let mut world = World::new();
        let e = world.spawn((crate::Transform::default(),));
        tween(&mut world, e, Tween::new("up", 1.0).set(Property::Y, 10.0));
        tween(
            &mut world,
            e,
            Tween::new("grow", 1.0).scale(Vec3::splat(2.0)),
        );
        run(&mut world, 0.5);
        tween(&mut world, e, Tween::new("down", 0.5).set(Property::Y, 0.0));
        assert!(!tweening(&world, e, "up"));
        assert!(tweening(&world, e, "grow"));
        run(&mut world, 0.5);
        let t = *world.get::<&crate::Transform>(e).unwrap();
        assert_eq!(t.position.y, 0.0, "down from where up left it");
        assert_eq!(t.scale, Vec3::splat(2.0));
    }

    #[test]
    fn a_tween_hands_other_modules_properties_to_them_and_switches_things() {
        let mut world = World::new();
        let e = world.spawn((crate::Transform::default(),));
        tween(
            &mut world,
            e,
            Tween::new("fade", 1.0)
                .set(Property::Volume, 0.0)
                .set(Property::Active, 0.0),
        );
        let mut volumes = Vec::new();
        run_tweens(
            &mut world,
            1.0,
            &|_, _, _| Some(0.8),
            &mut |_, _, what, v| {
                if what == Property::Volume {
                    volumes.push(v);
                }
            },
        );
        assert_eq!(volumes, [0.0]);
        assert!(
            world.get::<&crate::world::Inactive>(e).is_ok(),
            "switched off"
        );
    }
}
