//! A number inside any value serde can write, by its path: `intensity`,
//! `color.1`, `steps.0.speed` — what an animation track names when it
//! moves a field of a part or a component (`Property::Field`).
//!
//! The value is written as RON text, the number found there by its path
//! and changed in place, and the value read back from the text: every
//! other field keeps what it had — an enum's variant, an option, a name —
//! so a component the game changes while it runs keeps its state under a
//! clip that moves one of its numbers. A round trip through text is not
//! free, a few microseconds a field, and a track or two a thing is what it
//! is for; the transform and a material's numbers have their own fast
//! paths.
//!
//! A game's component that derives `Serialize` is registered here as it is
//! registered with [`crate::Components::register_saved`], so a clip can
//! move `components.player.speed` without the animation module knowing the
//! game's types.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::RwLock;

use serde::{de::DeserializeOwned, Serialize};

use crate::ron_edit::{items, value_span};

/// Where a value begins at or after `i`, past spaces.
fn skip_space(text: &str, mut i: usize) -> usize {
    let b = text.as_bytes();
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// The items inside the value at `span`: past a name before its bracket
/// (`Some(`, `Spot(`), as `(key, span of value)` — `None` for a list's.
fn inside(text: &str, span: Range<usize>) -> Option<Vec<(Option<String>, Range<usize>)>> {
    let value = &text[span.clone()];
    let at = value.find(['(', '['])?;
    if !value[..at].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let found = items(text, span.start + at)?;
    Some(
        found
            .items
            .into_iter()
            .map(|r| {
                let item = &text[r.clone()];
                // `key: value` — a key is a name, not a string's colon.
                match item.split_once(':') {
                    Some((k, _)) if !k.trim().is_empty() && k.trim().chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
                        let colon = r.start + item.find(':').unwrap_or(0) + 1;
                        let start = skip_space(text, colon);
                        (Some(k.trim().to_string()), value_span(text, start).unwrap_or(start..r.end))
                    }
                    _ => (None, r),
                }
            })
            .collect(),
    )
}

/// The span of the value at a path in RON text: a struct's field by name,
/// a list's or a tuple's item by number. An option's `Some(…)` and a
/// variant's name before its fields are gone through.
fn at(text: &str, path: &str) -> Option<Range<usize>> {
    let mut span = value_span(text, skip_space(text, 0))?;
    for step in path.split('.').filter(|s| !s.is_empty()) {
        let mut found = inside(text, span.clone())?;
        // `Some(x)`: one item and no key — what is in it.
        while found.len() == 1 && found[0].0.is_none() && text[span.clone()].starts_with("Some(") {
            span = found[0].1.clone();
            found = inside(text, span.clone())?;
        }
        span = match step.parse::<usize>() {
            Ok(i) if found.iter().all(|(k, _)| k.is_none()) => found.get(i)?.1.clone(),
            _ => found.into_iter().find(|(k, _)| k.as_deref() == Some(step))?.1,
        };
    }
    // An option's number: what is in it.
    while text[span.clone()].starts_with("Some(") {
        span = inside(text, span.clone())?.first()?.1.clone();
    }
    Some(span)
}

/// A number written as RON: a float, a whole number, a switch.
fn number(word: &str) -> Option<f32> {
    match word.trim() {
        "true" => Some(1.0),
        "false" => Some(0.0),
        w => w.parse::<f32>().ok(),
    }
}

/// The number at `path` in `value`.
pub fn get<T: Serialize>(value: &T, path: &str) -> Option<f32> {
    let text = ron::to_string(value).ok()?;
    number(&text[at(&text, path)?])
}

/// Set the number at `path` in `value`; whether there was one. A whole
/// number field takes the nearest whole number, a switch is on above a
/// half.
pub fn set<T: Serialize + DeserializeOwned>(value: &mut T, path: &str, to: f32) -> bool {
    let Ok(mut text) = ron::to_string(value) else { return false };
    let Some(span) = at(&text, path) else { return false };
    let word = text[span.clone()].trim().to_string();
    let written = match word.as_str() {
        "true" | "false" => (to > 0.5).to_string(),
        w if w.parse::<f32>().is_err() => return false,
        w if !w.contains(['.', 'e', 'E']) => format!("{}", to.round() as i64),
        _ => format!("{to:?}"),
    };
    text.replace_range(span, &written);
    match ron::from_str::<T>(&text) {
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
    fn walk(text: &str, span: Range<usize>, path: &str, out: &mut Vec<(String, f32)>) {
        let word = text[span.clone()].trim();
        if let Ok(v) = word.parse::<f32>() {
            out.push((path.to_string(), v));
            return;
        }
        let Some(found) = inside(text, span.clone()) else { return };
        if found.len() > 16 && found.iter().all(|(k, _)| k.is_none()) {
            return;
        }
        let some = word.starts_with("Some(") && found.len() == 1 && found[0].0.is_none();
        for (i, (key, span)) in found.into_iter().enumerate() {
            let step = key.unwrap_or_else(|| i.to_string());
            let next = if some {
                path.to_string()
            } else if path.is_empty() {
                step
            } else {
                format!("{path}.{step}")
            };
            walk(text, span, &next, out);
        }
    }
    let mut out = Vec::new();
    if let Some(span) = value_span(text, skip_space(text, 0)) {
        walk(text, span, "", &mut out);
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

/// Gets or sets a number of an engine part as an entity holds it.
type PartAccess = Box<dyn Fn(&hecs::World, hecs::Entity, &str, Option<f32>) -> Option<f32> + Send + Sync>;

static PARTS: RwLock<BTreeMap<String, PartAccess>> = RwLock::new(BTreeMap::new());

/// Let clips move the numbers of the part a line calls `name`, as the
/// module that dresses it keeps it on the entity: in component `C`, the
/// part's value where `pick` finds it — `|l: &mut LightSource| &mut l.0`.
/// What a module re-reads every frame moves as the clip does; what it
/// built once at spawn (a body, a cloth) does not, and is not registered.
pub fn register_part<C, T>(name: &str, pick: fn(&mut C) -> &mut T)
where
    C: hecs::Component,
    T: Serialize + DeserializeOwned + 'static,
{
    let access = move |world: &hecs::World, entity: hecs::Entity, path: &str, to: Option<f32>| {
        let mut c = world.get::<&mut C>(entity).ok()?;
        let value = pick(&mut c);
        let now = get(&*value, path)?;
        if let Some(to) = to {
            set(value, path, to);
        }
        Some(now)
    };
    if let Ok(mut all) = PARTS.write() {
        all.insert(name.to_string(), Box::new(access));
    }
}

/// A number of an engine part on an entity, by the part's name and the
/// number's path in it; with `to`, set as well.
pub fn part(world: &hecs::World, entity: hecs::Entity, name: &str, path: &str, to: Option<f32>) -> Option<f32> {
    let all = PARTS.read().ok()?;
    all.get(name)?(world, entity, path, to)
}

/// Whether clips can move the numbers of the part called `name`.
pub fn has_part(name: &str) -> bool {
    PARTS.read().is_ok_and(|all| all.contains_key(name))
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

    #[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
    enum Shape {
        Point,
        Spot { angle: f32 },
    }

    #[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
    struct Shaped {
        shape: Shape,
        other: Shape,
        size: f32,
    }

    #[test]
    fn a_value_with_enums_keeps_them_when_one_number_moves() {
        let mut v = Shaped { shape: Shape::Point, other: Shape::Spot { angle: 30.0 }, size: 1.0 };
        assert!(set(&mut v, "size", 2.0));
        assert_eq!(v, Shaped { shape: Shape::Point, other: Shape::Spot { angle: 30.0 }, size: 2.0 });
        assert_eq!(get(&v, "other.angle").or(get(&v, "other.Spot.angle")), Some(30.0));
    }

    #[test]
    fn every_number_of_a_text_is_listed_by_path() {
        let paths: Vec<String> = all("(intensity: 2.0, color: (1.0, 0.5, 0.0), name: \"a\", inner: (speed: 4))").into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, ["intensity", "color.0", "color.1", "color.2", "inner.speed"], "as written");
    }

    #[test]
    fn a_registered_component_is_moved_on_its_entity() {
        register::<Lamp>("lamp");
        let mut world = hecs::World::new();
        let e = world.spawn((lamp(),));
        assert_eq!(component(&world, e, "lamp", "intensity", Some(5.0)), Some(2.0));
        assert_eq!(world.get::<&Lamp>(e).unwrap().intensity, 5.0);
        assert_eq!(component(&world, e, "nothing", "intensity", Some(5.0)), None);
    }
}
