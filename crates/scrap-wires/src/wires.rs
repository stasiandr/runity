//! A line's `wires`, and what runs them.
//!
//! ```ron
//! (id: "7a…", name: "porch", body: Trigger, collider: Box(half: (1.5, 1.0, 1.5)), wires: [
//!     (on: Enter, only: "player", to: "5f1c09aa3e7b2d10", do: Trigger("open")),
//!     (on: Empty, only: "player", to: "5f1c09aa3e7b2d10", do: Trigger("close")),
//!     (on: Enter, to: "c0ffee0000000001", do: Activate, once: true),
//! ])
//! ```
//!
//! A wire is data: when (something came in, went out, the last one went
//! out), to what (an entity by its ID — a picker in the editor, scoped
//! inside a prefab as a joint's `to` is), and which of the engine's
//! actions ([`Act`]). What a door does when it opens is its animator's —
//! content, like the wire. What the game does when the door opens is the
//! game's Rust: there is nothing here to write it in.
//!
//! [`run_wires`] reads the physics' [`Contacts`] — the last step's, which
//! a system running every step sees each change of exactly once — and
//! does what the wires say. A trigger that is someone else's on the
//! network ([`Replica`]) is left to them.

use std::collections::BTreeSet;

use hecs::{Entity, World};
use serde::{Deserialize, Serialize};

use scrap_core::id::EntityId;
use scrap_core::scene::{EntityDesc, Override, Transform};
use scrap_core::world::{Inactive, Layer, Parent, Replica, SceneId, SpawnedId, WorldTransform};
use scrap_core::AssetLink;
use scrap_physics::body::{Body, PhysicsLine};
use scrap_physics::physics::Contacts;

/// `wires: [(on: Enter, to: "5f1c…", do: Trigger("open"))]` — what the
/// line's body does to other things when something touches it: Hammer's
/// outputs, each wired to an input of a named entity.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Wires(pub Vec<Wire>);

/// One wire: when, to what, which action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wire {
    /// When it fires.
    #[serde(default)]
    pub on: On,
    /// Only for what is on this collision layer (`layers.ron`), or under a
    /// parent that is — the player, not a crate. Empty for anything.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub only: String,
    /// What it acts on: an entity of the scene or the prefab, by its ID.
    pub to: EntityId,
    /// What it does to it.
    #[serde(rename = "do")]
    pub act: Act,
    /// Fires the first time only: a trap that springs, a way that opens
    /// for good.
    #[serde(default, skip_serializing_if = "is_false")]
    pub once: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// When a wire fires. Several things coming in in one step fire it once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum On {
    /// Something came in: Hammer's OnStartTouch, Unity's OnTriggerEnter.
    #[default]
    Enter,
    /// Something went out: OnEndTouch.
    Leave,
    /// The last thing went out and nothing is left inside: OnEndTouchAll.
    /// A door closes when everyone has gone, not when the first one does.
    Empty,
}

/// What a wire does to its entity: the engine's actions, and only these.
/// Each is one thing a designer would otherwise ask a programmer for; none
/// of them asks anything, remembers anything or waits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Act {
    /// Pull a trigger of its animator: `Trigger("open")` swings a door
    /// whose graph goes to `open` on it.
    Trigger(String),
    /// Set a switch of its animator: `Set("lit", true)`.
    Set(String, bool),
    /// Switch it on, and what is under it.
    Activate,
    /// Switch it off: not drawn, no body, no sound.
    Deactivate,
    /// On if it was off, off if it was on.
    Toggle,
    /// A prefab where it stands, as it stands: a crate dropped from a
    /// hatch, a wave of enemies at a marker. Spawned by whoever holds the
    /// prefabs ([`SpawnOrder`]).
    Spawn { prefab: AssetLink },
}

impl Act {
    /// The animator parameter it names, for the ones that name one.
    pub fn parameter(&self) -> Option<&str> {
        match self {
            Act::Trigger(p) | Act::Set(p, _) => Some(p),
            _ => None,
        }
    }
}

crate::impl_parts! {
    Wires => "wires", default if |w| w.0.is_empty();
}

/// A line's wires, read off it.
pub trait WireLine {
    fn wires(&self) -> Vec<Wire>;
}

impl WireLine for EntityDesc {
    fn wires(&self) -> Vec<Wire> {
        self.part::<Wires>().map(|w| w.0).unwrap_or_default()
    }
}

impl WireLine for Override {
    fn wires(&self) -> Vec<Wire> {
        self.part::<Wires>().map(|w| w.0).unwrap_or_default()
    }
}

/// An entity's wires in the world, with how many times each has fired.
#[derive(Debug, Clone, PartialEq)]
pub struct Wired {
    pub wires: Vec<Wire>,
    pub fired: Vec<u32>,
}

impl Wired {
    pub fn new(wires: Vec<Wire>) -> Self {
        let fired = vec![0; wires.len()];
        Self { wires, fired }
    }
}

/// A prefab asked for by a wire, to be spawned by whoever holds the
/// prefabs and the models — `LiveScene` in a game, which spawns these and
/// takes them away as it polls. An entity of its own: hecs has no
/// resources.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnOrder {
    pub prefab: AssetLink,
    pub at: Transform,
}

/// The wires module's dresser ([`crate::world::Dress`]): a line's wires on
/// its entity, with the [`Contacts`] they read — a trigger has them, any
/// other body gets them here. A reload that changes the wires keeps the
/// count of each one it did not change: a spent `once` stays spent, and
/// nothing fires again for being read again.
pub struct WireDress;

impl crate::world::Dress for WireDress {
    fn parts(&self) -> &[&'static str] {
        &["wires"]
    }

    fn dress(
        &mut self,
        line: &EntityDesc,
        entity: Entity,
        world: &mut World,
        _: crate::world::Changed,
        _: &mut Vec<crate::world::Unresolved>,
    ) {
        let wires = line.wires();
        if wires.is_empty() {
            let _ = world.remove_one::<Wired>(entity);
            return;
        }
        let mut wired = Wired::new(wires);
        if let Ok(old) = world.get::<&Wired>(entity) {
            for (i, wire) in wired.wires.iter().enumerate() {
                if old.wires.get(i) == Some(wire) {
                    wired.fired[i] = old.fired[i];
                }
            }
        }
        let _ = world.insert_one(entity, wired);
        if world.get::<&Contacts>(entity).is_err() {
            let _ = world.insert_one(entity, Contacts::default());
        }
    }
}

/// Whether `entity`, or the nearest thing above it with a layer, is on
/// `layer`; anything, for an empty one.
fn on_layer(world: &World, entity: Entity, layer: &str) -> bool {
    if layer.is_empty() {
        return true;
    }
    let mut at = Some(entity);
    while let Some(e) = at {
        if let Ok(l) = world.get::<&Layer>(e) {
            return l.0 == layer;
        }
        at = world.get::<&Parent>(e).ok().map(|p| p.0);
    }
    layer == "default"
}

/// The entity a wire names: a line of the scene, or of a prefab spawned
/// at run time, in that spawn's scope. Looked up only when a wire fires,
/// which is seldom.
fn find(world: &World, id: EntityId) -> Option<Entity> {
    if id.is_unassigned() {
        return None;
    }
    let scene = world
        .query::<(Entity, &SceneId)>()
        .iter()
        .find(|(_, s)| s.0 == id)
        .map(|(e, _)| e);
    scene.or_else(|| {
        world
            .query::<(Entity, &SpawnedId)>()
            .iter()
            .find(|(_, s)| s.0 == id)
            .map(|(e, _)| e)
    })
}

/// Everything wired one step on: each wire whose trigger saw its moment
/// in the last physics step does its action. In the fixed step, before
/// what it pulls moves.
pub fn run_wires(world: &mut World, _: f32) {
    let mut fired: Vec<(Entity, usize)> = Vec::new();
    for (entity, wired, contacts) in world
        .query::<(Entity, &Wired, &Contacts)>()
        .without::<&Replica>()
        .without::<&Inactive>()
        .iter()
    {
        if contacts.entered.is_empty() && contacts.left.is_empty() {
            continue;
        }
        for (i, wire) in wired.wires.iter().enumerate() {
            if wire.once && wired.fired[i] > 0 {
                continue;
            }
            let counts = |e: &Entity| on_layer(world, *e, &wire.only);
            let now = match wire.on {
                On::Enter => contacts.entered.iter().any(counts),
                On::Leave => contacts.left.iter().any(counts),
                On::Empty => {
                    contacts.left.iter().any(counts) && !contacts.inside.iter().any(counts)
                }
            };
            if now {
                fired.push((entity, i));
            }
        }
    }
    for (entity, i) in fired {
        let wire = {
            let Ok(mut wired) = world.get::<&mut Wired>(entity) else {
                continue;
            };
            wired.fired[i] += 1;
            wired.wires[i].clone()
        };
        if let Some(target) = find(world, wire.to) {
            act(world, target, &wire.act);
        }
    }
}

/// Do one action to one entity.
fn act(world: &mut World, target: Entity, act: &Act) {
    match act {
        Act::Activate => scrap_core::world::set_active(world, target, true),
        Act::Deactivate => scrap_core::world::set_active(world, target, false),
        Act::Toggle => {
            let off = world.get::<&Inactive>(target).is_ok();
            scrap_core::world::set_active(world, target, off);
        }
        Act::Trigger(name) => animate(world, target, name, None),
        Act::Set(name, value) => animate(world, target, name, Some(*value)),
        Act::Spawn { prefab } => {
            let placed = world
                .get::<&WorldTransform>(target)
                .map(|w| w.0)
                .unwrap_or_default();
            let (scale, rotation, position) = placed.to_scale_rotation_translation();
            let mut at = Transform {
                position,
                scale,
                ..Default::default()
            };
            at.set_rotation(rotation);
            world.spawn((SpawnOrder {
                prefab: prefab.clone(),
                at,
            },));
        }
    }
}

/// Pull a trigger (`None`) or set a switch of the entity's animator: the
/// graph moving the things under a line, or a skeleton's.
#[cfg(feature = "animation")]
fn animate(world: &mut World, target: Entity, name: &str, value: Option<bool>) {
    use scrap_animation::animgraph::Controller;
    use scrap_animation::motion::Moving;
    let pull = |c: &mut Controller| match value {
        None => c.trigger(name),
        Some(v) => c.set_bool(name, v),
    };
    if let Ok(mut moving) = world.get::<&mut Moving>(target) {
        pull(&mut moving.controller);
    } else if let Ok(mut controller) = world.get::<&mut Controller>(target) {
        pull(&mut controller);
    }
}

/// Without the animation module, there is no animator to pull.
#[cfg(not(feature = "animation"))]
fn animate(_: &mut World, _: Entity, _: &str, _: Option<bool>) {}

/// What is wrong with a line's wires, a sentence each, without who: the
/// caller says whose. `line` finds a line by its ID where the wire looks
/// for it (the file, the scene as expanded); `parameters` an animator
/// graph's parameters by its name (`None` when it cannot tell);
/// `prefab` whether a prefab is there. What `check` and the editor's
/// problems say.
pub fn problems<'a>(
    of: &EntityDesc,
    line: impl Fn(EntityId) -> Option<&'a EntityDesc>,
    parameters: impl Fn(&str) -> Option<BTreeSet<String>>,
    prefab: impl Fn(&AssetLink) -> bool,
) -> Vec<String> {
    let wires = of.wires();
    let mut out = Vec::new();
    if wires.is_empty() {
        return out;
    }
    if of.body() == Body::None {
        out.push(
            "it has wires and no body for anything to touch — give it `body: Trigger` and a collider"
                .into(),
        );
    }
    for (i, wire) in wires.iter().enumerate() {
        let n = i + 1;
        if wire.to.is_unassigned() {
            out.push(format!("wire {n} goes to nothing — pick what it acts on"));
            continue;
        }
        let Some(target) = line(wire.to) else {
            out.push(format!("wire {n} goes to {}, which is not there", wire.to));
            continue;
        };
        if let Some(name) = wire.act.parameter() {
            let graph = target
                .parts
                .raw("animator")
                .and_then(|text| ron::from_str::<String>(text).ok())
                .unwrap_or_default();
            if graph.is_empty() {
                // An instance's animator is its prefab's, which the file
                // does not say.
                if target.prefab.is_empty() {
                    out.push(format!(
                        "wire {n} pulls `{name}` on `{}`, which has no animator",
                        target.name
                    ));
                }
            } else if let Some(known) = parameters(&graph) {
                if !known.contains(name) {
                    let near =
                        scrap_core::spelling::closest(name, known.iter().map(String::as_str))
                            .map(|p| format!(" — did you mean `{p}`?"))
                            .unwrap_or_default();
                    out.push(format!(
                        "wire {n} pulls `{name}` on `{}`, and its animator `{graph}` has no `{name}`{near}",
                        target.name
                    ));
                }
            }
        }
        if let Act::Spawn { prefab: link } = &wire.act {
            if link.is_empty() {
                out.push(format!("wire {n} spawns nothing — name a prefab"));
            } else if !prefab(link) {
                out.push(format!(
                    "wire {n} spawns `{link}`, and there is no such prefab"
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Changed, Dress};
    use scrap_core::scene::Scene;
    use scrap_physics::physics::{PhysicsDress, PhysicsWorld};

    const FLOOR: &str =
        r#"(id: "f1", name: "floor", body: Static, collider: Box(half: (20.0, 0.1, 20.0)))"#;
    const LAMP: &str = r#"(id: "1a", name: "lamp")"#;
    const BULB: &str = r#"(id: "b0", name: "bulb")"#;

    fn scene(zone_wires: &str, balls: &[(f32, &str)]) -> Scene {
        let mut lines = vec![FLOOR.to_string(), LAMP.to_string(), BULB.to_string()];
        lines.push(format!(
            r#"(id: "20", name: "zone", transform: (position: (0.0, 3.0, 0.0)), body: Trigger, collider: Box(half: (1.0, 0.5, 1.0)), wires: {zone_wires})"#
        ));
        for (i, (y, layer)) in balls.iter().enumerate() {
            lines.push(format!(
                r#"(id: "3{i}", name: "ball {i}", transform: (position: (0.0, {y:.1}, 0.0)), body: Dynamic, collider: Sphere(radius: 0.25), layer: "{layer}")"#
            ));
        }
        ron::from_str(&format!("(entities: [{}])", lines.join(",\n"))).unwrap()
    }

    fn spawn(scene: &Scene) -> World {
        let mut world = World::new();
        crate::world::spawn_scene_dressed(
            scene,
            &mut world,
            &mut [Box::new(PhysicsDress), Box::new(WireDress)],
        );
        world
    }

    fn named(world: &World, id: &str) -> Entity {
        let id: EntityId = id.parse().unwrap();
        find(world, id).unwrap()
    }

    fn off(world: &World, id: &str) -> bool {
        world.get::<&Inactive>(named(world, id)).is_ok()
    }

    /// Steps of physics, then the wires, as the loop has them; a note of
    /// every step at which `watch` said yes.
    fn play(world: &mut World, steps: usize, watch: impl Fn(&World) -> bool) -> Vec<bool> {
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        let mut seen = Vec::new();
        for _ in 0..steps {
            run_wires(world, 1.0 / 60.0);
            crate::world::apply_hierarchy(world);
            physics.run(world);
            seen.push(watch(world));
        }
        seen
    }

    #[test]
    fn balls_through_a_zone_switch_a_lamp_off_while_inside_and_a_bulb_once() {
        let wires = r#"[
            (on: Enter, to: "1a", do: Deactivate),
            (on: Empty, to: "1a", do: Activate),
            (on: Enter, to: "b0", do: Toggle, once: true),
        ]"#;
        let mut world = spawn(&scene(wires, &[(6.0, ""), (12.0, "")]));
        let lamp_off = play(&mut world, 240, |w| off(w, "1a"));
        // Off twice — a ball in the zone each time — and on again after.
        let spells = lamp_off.windows(2).filter(|w| !w[0] && w[1]).count();
        assert_eq!(spells, 2, "{lamp_off:?}");
        assert!(!off(&world, "1a"), "on again once both have gone through");
        assert!(off(&world, "b0"), "toggled once, by the first ball only");
        let wired = world.get::<&Wired>(named(&world, "20")).unwrap();
        assert_eq!(wired.fired, [2, 2, 1]);
    }

    #[test]
    fn a_wire_only_for_a_layer_lets_anything_else_through() {
        let wires = r#"[(on: Enter, only: "player", to: "1a", do: Deactivate)]"#;
        let mut world = spawn(&scene(wires, &[(6.0, "crate")]));
        play(&mut world, 120, |_| false);
        assert!(!off(&world, "1a"), "a crate is not the player");

        let mut world = spawn(&scene(wires, &[(6.0, "player")]));
        play(&mut world, 120, |_| false);
        assert!(off(&world, "1a"), "the player is");
    }

    #[test]
    fn a_reload_keeps_what_fired_for_the_wires_it_did_not_change() {
        let wires = r#"[(on: Enter, to: "1a", do: Deactivate, once: true), (on: Leave, to: "b0", do: Toggle)]"#;
        let scene = scene(wires, &[(6.0, "")]);
        let mut world = spawn(&scene);
        play(&mut world, 120, |_| false);
        let zone = named(&world, "20");
        assert_eq!(world.get::<&Wired>(zone).unwrap().fired, [1, 1]);

        // The file is read again with its second wire changed.
        let mut line = scene.find("zone").unwrap().clone();
        line.parts.set_raw("wires", r#"[(on: Enter, to: "1a", do: Deactivate, once: true), (on: Leave, to: "b0", do: Activate)]"#).unwrap();
        let changed = ["wires".to_string()];
        WireDress.dress(
            &line,
            zone,
            &mut world,
            Changed::Only(&changed),
            &mut Vec::new(),
        );
        assert_eq!(
            world.get::<&Wired>(zone).unwrap().fired,
            [1, 0],
            "the spent one stays spent; the new one has not fired"
        );
    }

    #[test]
    fn a_spawn_is_ordered_where_its_marker_stands() {
        let wires = r#"[(on: Enter, to: "1a", do: Spawn(prefab: "crate"))]"#;
        let mut scene = scene(wires, &[(6.0, "")]);
        scene.entities[1].transform.position = glam::Vec3::new(4.0, 2.0, 0.0);
        let mut world = spawn(&scene);
        play(&mut world, 120, |_| false);
        let orders: Vec<SpawnOrder> = world.query::<&SpawnOrder>().iter().cloned().collect();
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].prefab.as_str(), "crate");
        assert!((orders[0].at.position - glam::Vec3::new(4.0, 2.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn problems_name_a_wire_to_nothing_a_missing_parameter_and_no_body() {
        let text = r#"(entities: [
            (id: "d0", name: "door", animator: "door"),
            (id: "e0", name: "sign"),
            (id: "20", name: "mat", wires: [
                (to: "d0", do: Trigger("opne")),
                (to: "e0", do: Trigger("open")),
                (to: "99", do: Activate),
                (to: "d0", do: Spawn(prefab: "crat")),
            ]),
        ])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        let mat = scene.find("mat").unwrap();
        let said = problems(
            mat,
            |id| scene.get(id),
            |graph| (graph == "door").then(|| ["open".to_string(), "close".to_string()].into()),
            |p| p.as_str() == "crate",
        );
        assert_eq!(said.len(), 5, "{said:#?}");
        assert!(said[0].contains("no body"), "{}", said[0]);
        assert!(said[1].contains("did you mean `open`"), "{}", said[1]);
        assert!(
            said[2].contains("`sign`, which has no animator"),
            "{}",
            said[2]
        );
        assert!(said[3].contains("not there"), "{}", said[3]);
        assert!(said[4].contains("no such prefab"), "{}", said[4]);
    }

    #[test]
    fn a_wire_reads_and_writes_as_a_person_would_write_it() {
        let text = r#"[(on: Empty, only: "player", to: "5f1c09aa3e7b2d10", do: Set("lit", false), once: true)]"#;
        let wires: Wires = ron::from_str(text).unwrap();
        assert_eq!(scrap_core::parts::to_text(&wires), text);
        // What is left out is the usual: on Enter, for anything, every time.
        let plain: Wires = ron::from_str(r#"[(to: "a1", do: Activate)]"#).unwrap();
        assert_eq!(plain.0[0].on, On::Enter);
        assert_eq!(
            scrap_core::parts::to_text(&plain),
            r#"[(on: Enter, to: "00000000000000a1", do: Activate)]"#
        );
        // A prefab's scope finds the target as it finds a joint's.
        assert_eq!(
            scrap_core::prefab::links_in(text),
            ["5f1c09aa3e7b2d10".parse::<EntityId>().unwrap()]
        );
    }

    #[cfg(feature = "animation")]
    #[test]
    fn a_door_swings_open_when_something_walks_into_its_porch() {
        use scrap_animation::motion::{self, Motion, MotionDress, Motions, Moving};
        let graph = r#"(start: "shut", states: {
            "shut": (clip: "door_shut", transitions: [(to: "open", when: [Trigger("open")])]),
            "open": (clip: "door_open", looping: false),
        })"#;
        let mut motions = Motions::default();
        motions
            .graphs
            .insert("door".into(), ron::from_str(graph).unwrap());
        for (clip, turn) in [("door_shut", 0.0), ("door_open", -90.0)] {
            let clip_text =
                format!("(length: 0.5, tracks: [(what: TurnY, keys: [(0.0, {turn:.1})])])");
            motions
                .clips
                .insert(clip.into(), ron::from_str::<Motion>(&clip_text).unwrap());
        }
        let mut scene = scene(
            r#"[(on: Enter, to: "d0", do: Trigger("open"))]"#,
            &[(6.0, "")],
        );
        scene
            .entities
            .push(ron::from_str(r#"(id: "d0", name: "door", animator: "door")"#).unwrap());
        let mut world = World::new();
        crate::world::spawn_scene_dressed(
            &scene,
            &mut world,
            &mut [
                Box::new(PhysicsDress),
                Box::new(WireDress),
                Box::new(MotionDress),
            ],
        );
        assert!(motion::attach(&mut world, &motions, |_| None).is_empty());
        let door = named(&world, "d0");
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        for _ in 0..120 {
            run_wires(&mut world, 1.0 / 60.0);
            motion::run_with(&mut world, 1.0 / 60.0, &mut |_, _, _, _| {});
            crate::world::apply_hierarchy(&mut world);
            physics.run(&mut world);
        }
        let state = world
            .get::<&Moving>(door)
            .unwrap()
            .controller
            .state()
            .map(str::to_string);
        assert_eq!(state.as_deref(), Some("open"));
    }
}
