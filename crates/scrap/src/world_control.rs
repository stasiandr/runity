//! The world of a running game, from outside it (docs/world-control.md):
//! its entities queried, read, written, spawned and despawned, its state
//! snapshotted and diffed, waited on — for any scrap game, through the
//! components it registered and the engine's own parts; and beside the
//! world, what a game keeps outside it ([`Registry`]: its resources read
//! and written, its commands called).
//!
//! It answers requests (a name and JSON arguments: [`Answer`]) — from the
//! control socket of a game in a window (`scrap::shell::control`,
//! `SCRAP_CONTROL`, `scrap world …`), from a test. A game hands its frame's
//! requests here with one call ([`WorldControl::answer`]) and to its
//! registry with another.
//!
//! An entity is named by its number (`hecs::Entity::to_bits`, as `query`
//! lists it) or by its scene line's name. A component is named as scenes
//! name it (`hero`), the engine's parts as `transform`, `name`, `parent`,
//! `active`. Components that derive `Serialize` and are registered saved,
//! networked or inspected read and write as JSON; the rest are listed as
//! write-only (put on from RON, taken off) with what makes them readable.

use std::collections::BTreeMap;

use hecs::{Entity, World};
pub use scrap_core::control::serde_json;
use serde_json::{json, Map};

pub use scrap_core::control::{Answer, Value};

use crate::Components;

/// The engine's parts the world control reads and writes besides the
/// game's components.
pub const PARTS: [&str; 4] = ["transform", "name", "parent", "active"];

/// One change made, to take back (`revert`).
struct Undo {
    entity: Entity,
    component: String,
    /// What it was: `None`, it was not there.
    before: Option<Value>,
}

/// Snapshots kept, the changes made, what waits.
#[derive(Default)]
pub struct WorldControl {
    snapshots: BTreeMap<String, Snapshot>,
    undo: Vec<Undo>,
}

/// The world as JSON: each entity's readable components.
pub type Snapshot = BTreeMap<u64, BTreeMap<String, Value>>;

fn arg_str<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str)
}

/// An entity by its number or its line's name.
pub fn entity_of(world: &World, text: &Value) -> Result<Entity, String> {
    let bits = match text {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse::<u64>().ok(),
        _ => None,
    };
    if let Some(bits) = bits {
        let e = Entity::from_bits(bits).ok_or_else(|| format!("no entity {bits}"))?;
        return if world.contains(e) { Ok(e) } else { Err(format!("entity {bits} is gone")) };
    }
    let name = text.as_str().ok_or("entity: a number or a name")?;
    world
        .query::<(Entity, &crate::world::LineName)>()
        .iter()
        .find(|(_, n)| n.0 == name)
        .map(|(e, _)| e)
        .ok_or_else(|| format!("no entity named `{name}`"))
}

/// An engine part of an entity as JSON (`None`: not there).
fn read_part(world: &World, e: Entity, part: &str) -> Option<Value> {
    match part {
        "transform" => world.get::<&crate::Transform>(e).ok().and_then(|t| serde_json::to_value(*t).ok()),
        "name" => world.get::<&crate::world::LineName>(e).ok().map(|n| json!(n.0)),
        "parent" => world.get::<&crate::world::Parent>(e).ok().map(|p| json!(p.0.to_bits().get())),
        "active" => Some(json!(world.get::<&crate::world::Inactive>(e).is_err())),
        _ => None,
    }
}

fn write_part(world: &mut World, e: Entity, part: &str, value: Value) -> Result<(), String> {
    match part {
        "transform" => {
            let t: crate::Transform = serde_json::from_value(value).map_err(|e| e.to_string())?;
            world.insert_one(e, t).map_err(|_| "the entity is gone".to_string())
        }
        "name" => {
            let n = value.as_str().ok_or("name: text")?.to_string();
            world.insert_one(e, crate::world::LineName(n)).map_err(|_| "the entity is gone".to_string())
        }
        "parent" => {
            if value.is_null() {
                let _ = world.remove_one::<crate::world::Parent>(e);
                return Ok(());
            }
            let p = entity_of(world, &value)?;
            world.insert_one(e, crate::world::Parent(p)).map_err(|_| "the entity is gone".to_string())
        }
        "active" => {
            if value.as_bool().ok_or("active: true or false")? {
                let _ = world.remove_one::<crate::world::Inactive>(e);
            } else {
                world.insert_one(e, crate::world::Inactive).map_err(|_| "the entity is gone".to_string())?;
            }
            Ok(())
        }
        other => Err(format!("no part `{other}`")),
    }
}

/// A component or part of an entity as JSON.
pub fn read(world: &World, components: &Components, e: Entity, name: &str) -> Result<Option<Value>, String> {
    if PARTS.contains(&name) {
        return Ok(read_part(world, e, name));
    }
    components.read_json(name, world, e)
}

/// Put a component or part on an entity from JSON.
pub fn write(world: &mut World, components: &Components, e: Entity, name: &str, value: Value) -> Result<(), String> {
    if PARTS.contains(&name) {
        return write_part(world, e, name, value);
    }
    if components.is_inspectable(name) {
        return components.write_json(name, value, world, e);
    }
    // Write-only: put on from RON text.
    let text = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
    components.insert_text(name, &text, world, e)
}

/// A field of `v` by a dotted path (`stats.hp`, `items.0`).
pub fn pick<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut at = v;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        at = match at {
            Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
            Value::Object(o) => o.get(part)?,
            _ => return None,
        };
    }
    Some(at)
}

/// `v` with the field at `path` set to `to`.
pub fn set_at(v: &mut Value, path: &str, to: Value) -> Result<(), String> {
    let mut at = v;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        at = match at {
            Value::Array(a) => a.get_mut(part.parse::<usize>().map_err(|_| format!("`{part}`: a list's index"))?).ok_or_else(|| format!("no `{part}`"))?,
            Value::Object(o) => {
                if !o.contains_key(part) {
                    return Err(format!("no field `{part}` (has {})", o.keys().cloned().collect::<Vec<_>>().join(", ")));
                }
                o.get_mut(part).expect("just checked")
            }
            _ => return Err(format!("`{part}`: not a struct or a list")),
        };
    }
    *at = to;
    Ok(())
}

/// A value given as text: JSON if it reads as JSON, else the text itself.
fn value_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

/// A condition on an entity: `hero.hp < 10`, `walker.speed >= 2`,
/// `name == door_1`, `transform.position.1 > 0.5` — a component (or part)
/// and a path in it, `== != < <= > >=`, a value.
pub struct Condition {
    component: String,
    path: String,
    op: String,
    value: Value,
}

impl Condition {
    pub fn parse(text: &str) -> Result<Self, String> {
        let ops = ["<=", ">=", "==", "!=", "<", ">", "="];
        let (at, op) = ops
            .iter()
            .filter_map(|op| text.find(op).map(|i| (i, *op)))
            .min_by_key(|(i, op)| (*i, std::cmp::Reverse(op.len())))
            .ok_or_else(|| format!("`{text}`: field op value, op one of == != < <= > >="))?;
        let field = text[..at].trim();
        let value = value_of(text[at + op.len()..].trim());
        let (component, path) = field.split_once('.').unwrap_or((field, ""));
        Ok(Self { component: component.to_string(), path: path.to_string(), op: if op == "=" { "==".into() } else { op.to_string() }, value })
    }

    /// Whether `e` meets it (an entity without the component does not).
    pub fn holds(&self, world: &World, components: &Components, e: Entity) -> bool {
        let Ok(Some(v)) = read(world, components, e, &self.component) else { return false };
        let Some(field) = pick(&v, &self.path) else { return false };
        compare(field, &self.op, &self.value)
    }
}

fn compare(a: &Value, op: &str, b: &Value) -> bool {
    let num = |v: &Value| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()));
    let ordering = match (num(a), num(b)) {
        (Some(x), Some(y)) => x.partial_cmp(&y),
        _ => match (a, b) {
            (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
            _ => None,
        },
    };
    let equal = a == b || ordering == Some(std::cmp::Ordering::Equal) || a.as_str().zip(b.as_str()).is_some_and(|(x, y)| x == y) || matches!((a, b), (Value::Bool(x), Value::String(y)) if x.to_string() == *y);
    match op {
        "==" => equal,
        "!=" => !equal,
        "<" => ordering == Some(std::cmp::Ordering::Less),
        "<=" => ordering.is_some_and(|o| o != std::cmp::Ordering::Greater),
        ">" => ordering == Some(std::cmp::Ordering::Greater),
        ">=" => ordering.is_some_and(|o| o != std::cmp::Ordering::Less),
        _ => false,
    }
}

/// What an entity is, in a line of `query`.
fn listing(world: &World, components: &Components, e: Entity) -> Value {
    let mut line = Map::new();
    line.insert("id".into(), json!(e.to_bits().get()));
    if let Some(n) = read_part(world, e, "name") {
        line.insert("name".into(), n);
    }
    if let Ok(p) = world.get::<&crate::world::Parent>(e) {
        line.insert("parent".into(), json!(p.0.to_bits().get()));
    }
    if let Ok(t) = world.get::<&crate::Transform>(e) {
        line.insert("at".into(), json!([t.position.x, t.position.y, t.position.z]));
    }
    line.insert("components".into(), json!(components.on(world, e)));
    Value::Object(line)
}

impl WorldControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// A world command (`help` lists them), or [`Answer::Unknown`].
    pub fn answer(&mut self, world: &mut World, components: &Components, cmd: &str, args: &Value) -> Answer {
        let result = match cmd {
            "components" => Ok(self.components(components)),
            "query" | "entities" => self.query(world, components, args),
            "get" => self.get(world, components, args),
            "set" => self.set(world, components, args),
            "add" => self.add(world, components, args),
            "remove" => self.remove(world, components, args),
            "spawn" => self.spawn(world, components, args),
            "despawn" => entity_of(world, args.get("entity").unwrap_or(&Value::Null)).and_then(|e| {
                crate::world::despawn_tree(world, e);
                Ok(json!({ "despawned": e.to_bits().get() }))
            }),
            "revert" => self.revert(world, components, args),
            "snapshot" => {
                let name = arg_str(args, "name").unwrap_or("last").to_string();
                let shot = snapshot(world, components);
                let n = shot.len();
                if let Some(path) = arg_str(args, "path") {
                    if let Err(e) = std::fs::write(path, serde_json::to_string_pretty(&shot).unwrap_or_default()) {
                        return Answer::Failed(format!("{path}: {e}"));
                    }
                }
                self.snapshots.insert(name.clone(), shot);
                Ok(json!({ "snapshot": name, "entities": n }))
            }
            "diff" => self.diff(world, components, args),
            "until" => return self.until(world, components, args),
            _ => return Answer::Unknown,
        };
        Answer::of(result)
    }

    /// Every component the game registered: name, readable or not, fields.
    pub fn components(&self, components: &Components) -> Value {
        let shapes = components.shapes();
        let list: Vec<Value> = components
            .names()
            .map(|name| {
                let shape = shapes.get(name).map(|s| serde_json::to_value(s).unwrap_or(Value::Null)).unwrap_or(Value::Null);
                json!({ "name": name, "access": if components.is_inspectable(name) { "read-write" } else { "write-only" }, "fields": shape })
            })
            .collect();
        json!({ "components": list, "parts": PARTS })
    }

    /// `query`: entities with every component of `with` (comma-separated
    /// or a list), meeting `where` (one condition or a list), named like
    /// `name`, in `near: [x, y, z, radius]`; `offset`/`limit` page it,
    /// `count: true` counts only.
    fn query(&self, world: &World, components: &Components, args: &Value) -> Result<Value, String> {
        let list = |name: &str| -> Vec<String> {
            match args.get(name) {
                Some(Value::String(s)) => s.split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect(),
                Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                _ => Vec::new(),
            }
        };
        let with = list("with");
        for w in &with {
            if !components.contains(w) && !PARTS.contains(&w.as_str()) {
                return Err(format!("no component `{w}` (components lists them)"));
            }
        }
        let conditions: Vec<Condition> = list("where").iter().map(|c| Condition::parse(c)).collect::<Result<_, _>>()?;
        let name = arg_str(args, "name");
        let near = args.get("near").and_then(Value::as_array).filter(|a| a.len() == 4).map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect::<Vec<_>>());
        let mut found: Vec<Entity> = world
            .iter()
            .map(|r| r.entity())
            .filter(|e| with.iter().all(|w| if PARTS.contains(&w.as_str()) { read_part(world, *e, w).is_some_and(|v| v != json!(false)) } else { components.has(w, world, *e) }))
            .filter(|e| conditions.iter().all(|c| c.holds(world, components, *e)))
            .filter(|e| name.is_none_or(|n| world.get::<&crate::world::LineName>(*e).is_ok_and(|l| l.0.contains(n))))
            .filter(|e| {
                near.as_ref().is_none_or(|n| world.get::<&crate::Transform>(*e).is_ok_and(|t| t.position.distance(glam::Vec3::new(n[0], n[1], n[2])) <= n[3]))
            })
            .collect();
        found.sort_by_key(|e| e.to_bits());
        let total = found.len();
        if args.get("count").and_then(Value::as_bool) == Some(true) {
            return Ok(json!({ "count": total }));
        }
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
        let page: Vec<Value> = found.iter().skip(offset).take(limit).map(|e| listing(world, components, *e)).collect();
        Ok(json!({ "total": total, "offset": offset, "entities": page }))
    }

    /// `get`: an entity's components (all readable ones, or `component`,
    /// a field by `path`); write-only ones named as such.
    fn get(&self, world: &World, components: &Components, args: &Value) -> Result<Value, String> {
        let e = entity_of(world, args.get("entity").unwrap_or(&Value::Null))?;
        if let Some(name) = arg_str(args, "component") {
            let v = read(world, components, e, name)?.ok_or_else(|| format!("entity {} has no `{name}`", e.to_bits().get()))?;
            return match arg_str(args, "path").filter(|p| !p.is_empty()) {
                Some(path) => pick(&v, path).cloned().ok_or_else(|| format!("no `{path}` in `{name}`")),
                None => Ok(v),
            };
        }
        let mut all = Map::new();
        for part in PARTS {
            if let Some(v) = read_part(world, e, part) {
                all.insert(part.into(), v);
            }
        }
        for name in components.on(world, e) {
            let v = match components.read_json(name, world, e) {
                Ok(Some(v)) => v,
                Ok(None) => continue,
                Err(_) => json!("(write-only: derive Serialize and register it saved or inspected to read it)"),
            };
            all.insert(name.to_string(), v);
        }
        Ok(json!({ "id": e.to_bits().get(), "components": all }))
    }

    fn remember(&mut self, world: &World, components: &Components, e: Entity, name: &str) {
        let before = read(world, components, e, name).ok().flatten();
        self.push_undo(Undo { entity: e, component: name.to_string(), before });
    }

    fn push_undo(&mut self, undo: Undo) {
        self.undo.push(undo);
        if self.undo.len() > 256 {
            self.undo.remove(0);
        }
    }

    /// `set`: a component (or part) whole from `value`, or one field of it
    /// by `path`; checked by putting it through the component's own type.
    fn set(&mut self, world: &mut World, components: &Components, args: &Value) -> Result<Value, String> {
        let e = entity_of(world, args.get("entity").unwrap_or(&Value::Null))?;
        let name = arg_str(args, "component").ok_or("set wants component")?;
        let given = args.get("value").cloned().ok_or("set wants value")?;
        let given = match given {
            Value::String(s) => value_of(&s),
            v => v,
        };
        let value = match arg_str(args, "path").filter(|p| !p.is_empty()) {
            Some(path) => {
                let mut whole = read(world, components, e, name)?.ok_or_else(|| format!("entity {} has no `{name}`", e.to_bits().get()))?;
                set_at(&mut whole, path, given)?;
                whole
            }
            None => given,
        };
        let before = read(world, components, e, name).ok().flatten();
        write(world, components, e, name, value)?;
        self.push_undo(Undo { entity: e, component: name.to_string(), before });
        Ok(json!({ "entity": e.to_bits().get(), name: read(world, components, e, name).ok().flatten() }))
    }

    /// `add`: a component put on (`value`, or its type's default from `()`).
    fn add(&mut self, world: &mut World, components: &Components, args: &Value) -> Result<Value, String> {
        let e = entity_of(world, args.get("entity").unwrap_or(&Value::Null))?;
        let name = arg_str(args, "component").ok_or("add wants component")?;
        let before = read(world, components, e, name).ok().flatten();
        match args.get("value") {
            Some(v) => {
                let v = match v {
                    Value::String(s) => value_of(s),
                    v => v.clone(),
                };
                write(world, components, e, name, v)?
            }
            // Its fields' defaults, as a scene line `"name": ()` gives them.
            None => components.insert_text(name, "()", world, e)?,
        }
        self.push_undo(Undo { entity: e, component: name.to_string(), before });
        Ok(json!({ "entity": e.to_bits().get(), "added": name }))
    }

    fn remove(&mut self, world: &mut World, components: &Components, args: &Value) -> Result<Value, String> {
        let e = entity_of(world, args.get("entity").unwrap_or(&Value::Null))?;
        let name = arg_str(args, "component").ok_or("remove wants component")?;
        self.remember(world, components, e, name);
        if name == "parent" {
            let _ = world.remove_one::<crate::world::Parent>(e);
        } else {
            components.remove_by_name(name, world, e);
        }
        Ok(json!({ "entity": e.to_bits().get(), "removed": name }))
    }

    /// `spawn`: a new entity with the components given
    /// (`components: {"hero": {...}, "transform": {...}}`).
    fn spawn(&mut self, world: &mut World, components: &Components, args: &Value) -> Result<Value, String> {
        let e = world.spawn(());
        if let Some(Value::Object(list)) = args.get("components") {
            for (name, v) in list {
                if let Err(err) = write(world, components, e, name, v.clone()) {
                    let _ = world.despawn(e);
                    return Err(format!("{name}: {err}"));
                }
            }
        }
        self.undo.push(Undo { entity: e, component: String::new(), before: None });
        Ok(json!({ "spawned": e.to_bits().get() }))
    }

    /// `revert`: the last `n` changes (1) taken back.
    fn revert(&mut self, world: &mut World, components: &Components, args: &Value) -> Result<Value, String> {
        let n = args.get("n").and_then(Value::as_u64).unwrap_or(1) as usize;
        let mut done = 0;
        for _ in 0..n {
            let Some(u) = self.undo.pop() else { break };
            if !world.contains(u.entity) {
                continue;
            }
            if u.component.is_empty() {
                let _ = world.despawn(u.entity);
            } else {
                match u.before {
                    Some(v) => write(world, components, u.entity, &u.component, v)?,
                    None if u.component == "parent" => {
                        let _ = world.remove_one::<crate::world::Parent>(u.entity);
                    }
                    None => components.remove_by_name(&u.component, world, u.entity),
                }
            }
            done += 1;
        }
        Ok(json!({ "reverted": done, "left": self.undo.len() }))
    }

    /// `diff`: snapshot `a` against snapshot `b`, or against now.
    fn diff(&self, world: &World, components: &Components, args: &Value) -> Result<Value, String> {
        let a = self.snapshots.get(arg_str(args, "a").unwrap_or("last")).ok_or("no such snapshot (snapshot makes one)")?;
        let now;
        let b = match arg_str(args, "b") {
            Some(name) => self.snapshots.get(name).ok_or_else(|| format!("no snapshot `{name}`"))?,
            None => {
                now = snapshot(world, components);
                &now
            }
        };
        Ok(diff(a, b))
    }

    /// `until`: answered when `where` holds for an entity (`entity`, or any
    /// with the component) — a wait without polling.
    fn until(&self, world: &World, components: &Components, args: &Value) -> Answer {
        let Some(text) = arg_str(args, "where") else { return Answer::Failed("until wants where: \"hero.hp < 10\"".into()) };
        let cond = match Condition::parse(text) {
            Ok(c) => c,
            Err(e) => return Answer::Failed(e),
        };
        let candidates: Vec<Entity> = match args.get("entity") {
            Some(v) => match entity_of(world, v) {
                Ok(e) => vec![e],
                Err(e) => return Answer::Failed(e),
            },
            None => world.iter().map(|r| r.entity()).collect(),
        };
        match candidates.into_iter().find(|e| cond.holds(world, components, *e)) {
            Some(e) => Answer::Done(json!({ "entity": e.to_bits().get(), "holds": text })),
            None => Answer::Pending,
        }
    }
}

/// Every entity's readable components and parts, by its number.
pub fn snapshot(world: &World, components: &Components) -> Snapshot {
    let mut out = Snapshot::new();
    for e in world.iter().map(|r| r.entity()) {
        let mut line = BTreeMap::new();
        for part in PARTS {
            if let Some(v) = read_part(world, e, part) {
                line.insert(part.to_string(), v);
            }
        }
        for name in components.on(world, e) {
            if let Ok(Some(v)) = components.read_json(name, world, e) {
                line.insert(name.to_string(), v);
            } else {
                line.insert(name.to_string(), Value::String("(write-only)".into()));
            }
        }
        out.insert(e.to_bits().get(), line);
    }
    out
}

/// What changed from `a` to `b`: entities come and gone, components put
/// on and taken off, fields changed (each with both values).
pub fn diff(a: &Snapshot, b: &Snapshot) -> Value {
    let added: Vec<u64> = b.keys().filter(|k| !a.contains_key(k)).copied().collect();
    let removed: Vec<u64> = a.keys().filter(|k| !b.contains_key(k)).copied().collect();
    let mut changed = Vec::new();
    for (id, before) in a {
        let Some(after) = b.get(id) else { continue };
        for (name, v) in before {
            match after.get(name) {
                None => changed.push(json!({ "entity": id, "component": name, "removed": true })),
                Some(w) if w != v => {
                    let mut fields = Vec::new();
                    leaves("", v, w, &mut fields);
                    changed.push(json!({ "entity": id, "component": name, "fields": fields }));
                }
                _ => {}
            }
        }
        for name in after.keys().filter(|n| !before.contains_key(*n)) {
            changed.push(json!({ "entity": id, "component": name, "added": true }));
        }
    }
    json!({ "added": added, "removed": removed, "changed": changed })
}

/// The fields at which `a` and `b` differ, by path.
fn leaves(path: &str, a: &Value, b: &Value, out: &mut Vec<Value>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                match y.get(k) {
                    Some(w) => leaves(&p, v, w, out),
                    None => out.push(json!({ "path": p, "was": v, "now": null })),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (v, w)) in x.iter().zip(y).enumerate() {
                leaves(&if path.is_empty() { i.to_string() } else { format!("{path}.{i}") }, v, w, out);
            }
        }
        _ if a != b => out.push(json!({ "path": path, "was": a, "now": b })),
        _ => {}
    }
}

/// What a game keeps outside its world and lets be looked at: resources
/// (named values read as JSON, some written) and commands (named, with
/// JSON arguments) — its run, its fight, its settings, its cheats. `G` is
/// whatever the game hands in (itself, usually).
pub struct Registry<G> {
    resources: BTreeMap<String, Resource<G>>,
    commands: BTreeMap<String, (String, fn(&mut G, &Value) -> Answer)>,
}

struct Resource<G> {
    help: String,
    get: fn(&G) -> Value,
    set: Option<fn(&mut G, Value) -> Result<(), String>>,
}

impl<G> Default for Registry<G> {
    fn default() -> Self {
        Self { resources: BTreeMap::new(), commands: BTreeMap::new() }
    }
}

impl<G> Registry<G> {
    pub fn new() -> Self {
        Self::default()
    }

    /// A value read as JSON (`res get NAME`).
    pub fn resource(&mut self, name: &str, help: &str, get: fn(&G) -> Value) -> &mut Self {
        self.resources.insert(name.into(), Resource { help: help.into(), get, set: None });
        self
    }

    /// A value read and written as JSON (`res set NAME …`).
    pub fn resource_mut(&mut self, name: &str, help: &str, get: fn(&G) -> Value, set: fn(&mut G, Value) -> Result<(), String>) -> &mut Self {
        self.resources.insert(name.into(), Resource { help: help.into(), get, set: Some(set) });
        self
    }

    /// A command (`call NAME`, or by its name): JSON arguments, an answer —
    /// [`Answer::Pending`] to be asked again next frame.
    pub fn command(&mut self, name: &str, help: &str, run: fn(&mut G, &Value) -> Answer) -> &mut Self {
        self.commands.insert(name.into(), (help.into(), run));
        self
    }

    /// What it holds, for `help`.
    pub fn help(&self) -> Value {
        json!({
            "resources": self.resources.iter().map(|(n, r)| json!({ "name": n, "help": r.help, "writable": r.set.is_some() })).collect::<Vec<_>>(),
            "commands": self.commands.iter().map(|(n, c)| json!({ "name": n, "help": c.0 })).collect::<Vec<_>>(),
        })
    }

    /// `res` (`list`, `get NAME [path]`, `set NAME [path] value`), `call
    /// NAME`, or a command by its own name; [`Answer::Unknown`] otherwise.
    pub fn answer(&self, game: &mut G, cmd: &str, args: &Value) -> Answer {
        match cmd {
            "res" => {
                let op = arg_str(args, "op").unwrap_or("list");
                let name = arg_str(args, "name").unwrap_or_default();
                match op {
                    "list" => Answer::Done(self.help()["resources"].clone()),
                    "get" => {
                        let Some(r) = self.resources.get(name) else { return Answer::Failed(format!("no resource `{name}` (res list)")) };
                        let v = (r.get)(game);
                        match arg_str(args, "path").filter(|p| !p.is_empty()) {
                            Some(path) => pick(&v, path).cloned().map_or_else(|| Answer::Failed(format!("no `{path}` in `{name}`")), Answer::Done),
                            None => Answer::Done(v),
                        }
                    }
                    "set" => {
                        let Some(r) = self.resources.get(name) else { return Answer::Failed(format!("no resource `{name}`")) };
                        let Some(set) = r.set else { return Answer::Failed(format!("`{name}` is read-only")) };
                        let given = match args.get("value").cloned().unwrap_or(Value::Null) {
                            Value::String(s) => value_of(&s),
                            v => v,
                        };
                        let value = match arg_str(args, "path").filter(|p| !p.is_empty()) {
                            Some(path) => {
                                let mut whole = (r.get)(game);
                                if let Err(e) = set_at(&mut whole, path, given) {
                                    return Answer::Failed(e);
                                }
                                whole
                            }
                            None => given,
                        };
                        Answer::of(set(game, value).map(|_| (r.get)(game)))
                    }
                    other => Answer::Failed(format!("res list | get | set, not {other}")),
                }
            }
            "call" => {
                let name = arg_str(args, "command").unwrap_or_default();
                let inner = args.get("args").cloned().unwrap_or(Value::Null);
                match self.commands.get(name) {
                    Some((_, run)) => run(game, &inner),
                    None => Answer::Failed(format!("no command `{name}` (help lists them)")),
                }
            }
            name => match self.commands.get(name) {
                Some((_, run)) => run(game, args),
                None => Answer::Unknown,
            },
        }
    }
}

/// The world commands, for `help`.
pub fn help() -> Value {
    json!([
        ["components", "every registered component: read-write or write-only, its fields"],
        ["query", "entities: with=hero,walker where=\"hero.hp < 10\" name=… near=[x,y,z,r] offset limit count"],
        ["get", "entity=N [component=hero [path=hp]]"],
        ["set", "entity=N component=hero [path=hp] value=5"],
        ["add", "entity=N component=hero [value={…}] (default from `()`)"],
        ["remove", "entity=N component=hero"],
        ["spawn", "components={\"transform\": {…}, \"hero\": {…}}"],
        ["despawn", "entity=N (and its children)"],
        ["revert", "n=1: take back the last set/add/remove/spawn"],
        ["snapshot", "name=… [path=file.json]"],
        ["diff", "a=… [b=…]: what changed (b: now)"],
        ["until", "where=\"hero.hp < 10\" [entity=N] timeout=…: answered when it holds"],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    struct Hero {
        name: String,
        hp: i32,
        bag: Vec<String>,
    }

    #[derive(Debug, Clone, Default, Deserialize)]
    struct Opaque {
        #[allow(dead_code)]
        n: i32,
    }

    fn world() -> (World, Components, Entity, Entity) {
        let mut components = Components::new();
        components.register_saved::<Hero>("hero");
        components.register::<Opaque>("opaque");
        let mut world = World::new();
        let a = world.spawn((Hero { name: "Ingrid".into(), hp: 55, bag: vec!["potion".into()] }, crate::Transform::default(), crate::world::LineName("ingrid".into())));
        let b = world.spawn((Hero { name: "Boris".into(), hp: 4, bag: vec![] }, Opaque { n: 1 }));
        (world, components, a, b)
    }

    fn done(a: Answer) -> Value {
        match a {
            Answer::Done(v) => v,
            other => panic!("{other:?}"),
        }
    }

    /// A game's own component is listed, queried by a field, read, written
    /// by a field's path, put on and taken off by name — no glue.
    #[test]
    fn a_games_component_is_queried_read_and_written_by_name() {
        let (mut world, components, a, b) = world();
        let mut wc = WorldControl::new();
        let listed = done(wc.answer(&mut world, &components, "components", &Value::Null));
        assert!(listed.to_string().contains("\"access\":\"write-only\"") && listed.to_string().contains("\"hp\""), "{listed}");
        let low = done(wc.answer(&mut world, &components, "query", &json!({ "with": "hero", "where": "hero.hp < 10" })));
        assert_eq!(low["total"], 1);
        assert_eq!(low["entities"][0]["id"], b.to_bits().get());
        let named = done(wc.answer(&mut world, &components, "get", &json!({ "entity": "ingrid", "component": "hero", "path": "name" })));
        assert_eq!(named, "Ingrid");
        done(wc.answer(&mut world, &components, "set", &json!({ "entity": b.to_bits().get(), "component": "hero", "path": "hp", "value": "40" })));
        assert_eq!(world.get::<&Hero>(b).unwrap().hp, 40);
        let wrong = wc.answer(&mut world, &components, "set", &json!({ "entity": b.to_bits().get(), "component": "hero", "path": "hp", "value": "lots" }));
        assert!(matches!(wrong, Answer::Failed(_)), "checked by the type: {wrong:?}");
        assert!(matches!(wc.answer(&mut world, &components, "get", &json!({ "entity": b.to_bits().get(), "component": "opaque" })), Answer::Failed(e) if e.contains("derive serde::Serialize")));
        done(wc.answer(&mut world, &components, "remove", &json!({ "entity": a.to_bits().get(), "component": "hero" })));
        assert!(world.get::<&Hero>(a).is_err());
        done(wc.answer(&mut world, &components, "revert", &json!({ "n": 2 })));
        assert_eq!(world.get::<&Hero>(a).unwrap().hp, 55, "put back");
        assert_eq!(world.get::<&Hero>(b).unwrap().hp, 4, "set taken back");
        done(wc.answer(&mut world, &components, "add", &json!({ "entity": a.to_bits().get(), "component": "opaque", "value": "(n: 3)" })));
        assert!(components.has("opaque", &world, a));
    }

    /// A snapshot and the world after a change: what changed, field by
    /// field; an entity spawned and one despawned; a wait that holds.
    #[test]
    fn a_snapshot_diffs_against_the_world_now() {
        let (mut world, components, a, b) = world();
        let mut wc = WorldControl::new();
        done(wc.answer(&mut world, &components, "snapshot", &json!({ "name": "before" })));
        world.get::<&mut Hero>(a).unwrap().hp = 50;
        world.get::<&mut Hero>(a).unwrap().bag.push("sword".into());
        let spawned = done(wc.answer(&mut world, &components, "spawn", &json!({ "components": { "hero": { "name": "Grumash", "hp": 30 }, "transform": { "position": [1.0, 0.0, 2.0] } } })));
        done(wc.answer(&mut world, &components, "despawn", &json!({ "entity": b.to_bits().get() })));
        let d = done(wc.answer(&mut world, &components, "diff", &json!({ "a": "before" })));
        assert_eq!(d["added"], json!([spawned["spawned"]]));
        assert_eq!(d["removed"], json!([b.to_bits().get()]));
        let fields = &d["changed"][0]["fields"];
        assert!(fields.as_array().unwrap().contains(&json!({ "path": "hp", "was": 55, "now": 50 })), "{fields}");
        assert!(fields.to_string().contains("sword"), "{fields}");
        assert!(matches!(wc.answer(&mut world, &components, "until", &json!({ "where": "hero.hp > 100" })), Answer::Pending));
        world.get::<&mut Hero>(a).unwrap().hp = 120;
        assert!(matches!(wc.answer(&mut world, &components, "until", &json!({ "where": "hero.hp > 100" })), Answer::Done(_)));
    }

    /// A game's resources and commands, read, written and called.
    #[test]
    fn a_registry_reads_writes_and_calls() {
        struct Game {
            gold: i32,
        }
        let mut r = Registry::<Game>::new();
        r.resource_mut("run", "the run", |g| json!({ "gold": g.gold }), |g, v| {
            g.gold = v["gold"].as_i64().ok_or("gold")? as i32;
            Ok(())
        });
        r.command("double", "gold ×2", |g, _| {
            g.gold *= 2;
            Answer::Done(json!(g.gold))
        });
        let mut g = Game { gold: 5 };
        assert_eq!(done(r.answer(&mut g, "res", &json!({ "op": "set", "name": "run", "path": "gold", "value": "7" }))), json!({ "gold": 7 }));
        assert_eq!(done(r.answer(&mut g, "double", &Value::Null)), json!(14));
        assert_eq!(done(r.answer(&mut g, "call", &json!({ "command": "double" }))), json!(28));
        assert!(r.answer(&mut g, "nothing", &Value::Null).is_unknown());
        assert!(r.help().to_string().contains("gold ×2"));
    }
}
