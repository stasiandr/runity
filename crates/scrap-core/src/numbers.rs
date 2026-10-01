//! A number inside any value serde can write, by its path: `intensity`,
//! `color.1`, `steps.0.speed` — what an animation track names when it
//! moves a field of a part or a component (`Property::Field`).
//!
//! The value is written to RON's tree, the number found and changed there,
//! and the value read back: every other field keeps what it had, so a
//! component the game changes while it runs keeps its state under a clip
//! that moves one of its numbers. A round trip through text is not free,
//! a few microseconds a field, and a track or two a thing is what it is
//! for; the transform and a material's numbers have their own fast paths.
//!
//! A game's component that derives `Serialize` is registered here as it is
//! registered with [`crate::Components::register_saved`], so a clip can
//! move `components.player.speed` without the animation module knowing the
//! game's types.

use std::collections::BTreeMap;
use std::sync::RwLock;

use ron::value::{Number, Value};
use serde::{de::DeserializeOwned, Serialize};

/// The value at a path of a tree: a struct's field by name, a list's item
/// by number; `Some` is gone through.
fn at<'a>(mut value: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    for step in path.split('.').filter(|s| !s.is_empty()) {
        while let Value::Option(Some(inner)) = value {
            value = inner;
        }
        value = match value {
            Value::Map(map) => map.iter_mut().find(|(k, _)| matches!(k, Value::String(s) if s == step)).map(|(_, v)| v)?,
            Value::Seq(items) => items.get_mut(step.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    while let Value::Option(Some(inner)) = value {
        value = inner;
    }
    Some(value)
}

fn tree<T: Serialize>(value: &T) -> Option<Value> {
    ron::from_str(&ron::to_string(value).ok()?).ok()
}

/// The number at `path` in `value`.
pub fn get<T: Serialize>(value: &T, path: &str) -> Option<f32> {
    let mut tree = tree(value)?;
    match at(&mut tree, path)? {
        Value::Number(n) => Some(n.into_f64() as f32),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Set the number at `path` in `value`; whether there was one. A whole
/// number field takes the nearest whole number, a switch is on above a
/// half.
pub fn set<T: Serialize + DeserializeOwned>(value: &mut T, path: &str, to: f32) -> bool {
    let Some(mut tree) = tree(value) else { return false };
    let Some(slot) = at(&mut tree, path) else { return false };
    *slot = match slot {
        Value::Number(Number::F32(_)) | Value::Number(Number::F64(_)) => Value::Number(Number::from(to)),
        Value::Number(_) => Value::Number(Number::from(to.round() as i64)),
        Value::Bool(_) => Value::Bool(to > 0.5),
        _ => return false,
    };
    match tree.into_rust::<T>() {
        Ok(back) => {
            *value = back;
            true
        }
        Err(_) => false,
    }
}

/// Every number in a value written as RON text, by its path, with what it
/// is: the fields a track could move, for a list to pick from.
pub fn all(text: &str) -> Vec<(String, f32)> {
    fn walk(value: &Value, path: &str, out: &mut Vec<(String, f32)>) {
        let join = |s: &str| if path.is_empty() { s.to_string() } else { format!("{path}.{s}") };
        match value {
            Value::Number(n) => out.push((path.to_string(), n.into_f64() as f32)),
            Value::Option(Some(inner)) => walk(inner, path, out),
            Value::Map(map) => {
                for (k, v) in map.iter() {
                    if let Value::String(k) = k {
                        walk(v, &join(k), out);
                    }
                }
            }
            Value::Seq(items) if items.len() <= 16 => {
                for (i, v) in items.iter().enumerate() {
                    walk(v, &join(&i.to_string()), out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Ok(value) = ron::from_str::<Value>(text) {
        walk(&value, "", &mut out);
    }
    out
}

/// Gets (`to: None`) or sets a number of a component of an entity. A
/// shared world is enough: hecs borrows one component at a time.
pub type Access = fn(&hecs::World, hecs::Entity, &str, Option<f32>) -> Option<f32>;

fn access<T: hecs::Component + Serialize + DeserializeOwned>(
    world: &hecs::World,
    entity: hecs::Entity,
    path: &str,
    to: Option<f32>,
) -> Option<f32> {
    let mut c = world.get::<&mut T>(entity).ok()?;
    let now = get(&*c, path)?;
    if let Some(to) = to {
        set(&mut *c, path, to);
    }
    Some(now)
}

static COMPONENTS: RwLock<BTreeMap<String, Access>> = RwLock::new(BTreeMap::new());

/// Let clips move the numbers of the component scenes call `name`.
pub fn register<T: hecs::Component + Serialize + DeserializeOwned>(name: &str) {
    if let Ok(mut all) = COMPONENTS.write() {
        all.insert(name.to_string(), access::<T>);
    }
}

/// A number of a game's component on an entity, by the component's name
/// and the number's path in it; with `to`, set as well. `None` when the
/// component is not on it, not registered, or has no number there.
pub fn component(world: &hecs::World, entity: hecs::Entity, name: &str, path: &str, to: Option<f32>) -> Option<f32> {
    let access = *COMPONENTS.read().ok()?.get(name)?;
    access(world, entity, path, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
    struct Lamp {
        intensity: f32,
        color: [f32; 3],
        count: u32,
        on: bool,
        name: String,
        glow: Option<f32>,
    }

    fn lamp() -> Lamp {
        Lamp { intensity: 2.0, color: [1.0, 0.5, 0.0], count: 3, on: true, name: "a".into(), glow: Some(0.25) }
    }

    #[test]
    fn a_number_is_found_and_changed_by_its_path_and_the_rest_stays() {
        let mut l = lamp();
        assert_eq!(get(&l, "intensity"), Some(2.0));
        assert_eq!(get(&l, "color.1"), Some(0.5));
        assert_eq!(get(&l, "glow"), Some(0.25));
        assert!(set(&mut l, "color.2", 0.75));
        assert!(set(&mut l, "count", 4.6));
        assert!(set(&mut l, "glow", 1.0));
        assert_eq!(l, Lamp { color: [1.0, 0.5, 0.75], count: 5, glow: Some(1.0), ..lamp() });
        assert!(!set(&mut l, "name", 1.0), "not a number");
        assert!(!set(&mut l, "nope", 1.0));
    }

    #[test]
    fn every_number_of_a_text_is_listed_by_path() {
        let paths: Vec<String> = all("(intensity: 2.0, color: (1.0, 0.5, 0.0), name: \"a\", inner: (speed: 4))").into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, ["color.0", "color.1", "color.2", "inner.speed", "intensity"], "by name");
    }

    #[test]
    fn a_registered_component_is_moved_on_its_entity() {
        register::<Lamp>("lamp");
        let mut world = hecs::World::new();
        let e = world.spawn((lamp(),));
        assert_eq!(component(&mut world, e, "lamp", "intensity", Some(5.0)), Some(2.0));
        assert_eq!(world.get::<&Lamp>(e).unwrap().intensity, 5.0);
        assert_eq!(component(&mut world, e, "nothing", "intensity", Some(5.0)), None);
    }
}
