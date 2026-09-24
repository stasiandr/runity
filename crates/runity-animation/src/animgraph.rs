//! Which animation plays when: states and transitions, as a file.
//!
//! Unity's Animator Controller, as RON a diff shows and a designer edits
//! while the game runs:
//!
//! ```text
//! (
//!     start: "idle",
//!     states: {
//!         "idle": (clip: "idle", transitions: [
//!             (to: "walk", when: [Above("speed", 0.1)]),
//!         ]),
//!         "walk": (clip: "walk", speed_from: "speed", transitions: [
//!             (to: "idle", when: [Below("speed", 0.1)]),
//!         ]),
//!         "move": (blend_by: "speed", blend: [(0.0, "idle"), (2.0, "walk")]),
//!         "jump": (clip: "jump", looping: false, transitions: [
//!             (to: "idle", when: [Finished]),
//!         ]),
//!     },
//!     any: [
//!         (to: "jump", when: [Trigger("jump")], fade: 0.1),
//!     ],
//! )
//! ```
//!
//! A transition is written inside the state it leaves, and `any` holds
//! Unity's Any State transitions. Adding a transition touches one state's
//! entry, so two people adding one each to different states merge without
//! a conflict (DNA, postulate 2), and an agent never has to match a `from`
//! at one end of the file to a state at the other. Order is priority: the
//! first transition whose conditions hold is taken, Any State's before a
//! state's own, as in Unity. A file of the older shape — every transition
//! in one list at the end, with `from:` — still reads, and is written in
//! this shape the next time the editor changes it.
//!
//! In memory a [`Graph`] keeps its transitions as one list with `from`, in
//! priority order ([`Graph::normalize`]); the shape above is only the file.
//!
//! The game sets parameters — `set("speed", 3.2)`, `trigger("jump")` — and
//! the graph picks the clip; the game never says "play the walk". Clips are
//! named as the model's file names them. A [`Controller`] sits on an entity
//! beside its [`Animator`], and [`run_controllers`] is the system.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::animator::Animator;

/// A state: which clip, and how it plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// The clip; empty for a blend state.
    #[serde(default)]
    pub clip: String,
    /// A 1D blend tree instead of one clip: clips at values of the
    /// `blend_by` parameter, in rising order —
    /// `[(0.0, "idle"), (1.5, "walk"), (5.0, "run")]` — and between two of
    /// them a mix of both, in step. Unity's Blend Tree.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blend: Vec<(f32, String)>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blend_by: String,
    /// A 2D blend tree: clips at points of `blend_by` (x) and `blend_by_y`
    /// (y) — `[(0.0, 0.0, "idle"), (0.0, 1.0, "walk"), (1.0, 0.0,
    /// "strafe_right"), …]` — mixed by direction and by how far out: a
    /// character walking any way while facing one. Unity's 2D Freeform
    /// Directional blend tree.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub directional: Vec<(f32, f32, String)>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blend_by_y: String,
    /// Named moments of the cycle, 0..1 — `[(0.1, "footstep"), (0.6,
    /// "footstep")]` — that [`Controller::fired`] reports as they pass:
    /// Unity's animation events, for a sound or a hit on the right frame.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<(f32, String)>,
    #[serde(default = "yes")]
    pub looping: bool,
    /// Play speed, or `1.0`.
    #[serde(default = "one")]
    pub speed: f32,
    /// A parameter to multiply the speed by — a walk that plays faster the
    /// faster the character goes.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub speed_from: Option<String>,
    /// A parameter that says where in the clip it stands, 0 at its start
    /// and 1 at its end, instead of it playing: a house going up as its
    /// health fills. Unity's Motion Time.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub time_from: Option<String>,
}

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}
fn fade() -> f32 {
    0.2
}

mod plain {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<T: Serialize, S: Serializer>(v: &Option<T>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => v.serialize(s),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<T>, D::Error> {
        T::deserialize(d).map(Some)
    }
}

/// What has to be true for a transition to be taken.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    /// A parameter is above a value.
    Above(String, f32),
    /// A parameter is below a value.
    Below(String, f32),
    /// A parameter is set to true (anything above one half).
    Is(String),
    /// A parameter is false.
    Not(String),
    /// A trigger was pulled since the last update; taking the transition
    /// uses it up.
    Trigger(String),
    /// The state's clip played to its end (a clip that does not loop).
    Finished,
}

/// From one state to another, when every condition holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    /// A state's name, or `"*"` for any state.
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub when: Vec<Condition>,
    /// Seconds of crossfade.
    #[serde(default = "fade")]
    pub fade: f32,
}

/// A whole graph. In memory the transitions are one list in priority
/// order; in the file each sits in the state it leaves (see the module).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "file::Graph", into = "file::Graph")]
pub struct Graph {
    pub start: String,
    pub states: BTreeMap<String, State>,
    pub transitions: Vec<Transition>,
}

/// The graph as it is written: transitions inside their states.
mod file {
    use super::*;

    /// A transition as written in the state it leaves, or in `any`.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Exit {
        pub to: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub when: Vec<Condition>,
        #[serde(default = "fade")]
        pub fade: f32,
    }

    /// [`super::State`] and the transitions that leave it.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct State {
        #[serde(default)]
        pub clip: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub blend: Vec<(f32, String)>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub blend_by: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub directional: Vec<(f32, f32, String)>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub blend_by_y: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub events: Vec<(f32, String)>,
        #[serde(default = "yes")]
        pub looping: bool,
        #[serde(default = "one")]
        pub speed: f32,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
        pub speed_from: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
        pub time_from: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub transitions: Vec<Exit>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Graph {
        pub start: String,
        pub states: BTreeMap<String, State>,
        /// Any State's transitions.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub any: Vec<Exit>,
        /// The older shape: every transition here, with `from`. Read, never
        /// written.
        #[serde(default, skip_serializing)]
        pub transitions: Vec<Transition>,
    }

    impl From<Graph> for super::Graph {
        fn from(f: Graph) -> Self {
            let exit = |from: &str, e: Exit| Transition {
                from: from.to_string(),
                to: e.to,
                when: e.when,
                fade: e.fade,
            };
            let mut transitions: Vec<Transition> =
                f.any.into_iter().map(|e| exit(ANY, e)).collect();
            let mut states = BTreeMap::new();
            for (name, s) in f.states {
                transitions.extend(s.transitions.into_iter().map(|e| exit(&name, e)));
                states.insert(
                    name,
                    super::State {
                        clip: s.clip,
                        blend: s.blend,
                        blend_by: s.blend_by,
                        directional: s.directional,
                        blend_by_y: s.blend_by_y,
                        events: s.events,
                        looping: s.looping,
                        speed: s.speed,
                        speed_from: s.speed_from,
                        time_from: s.time_from,
                    },
                );
            }
            transitions.extend(f.transitions);
            let mut graph = super::Graph {
                start: f.start,
                states,
                transitions,
            };
            graph.normalize();
            graph
        }
    }

    impl From<super::Graph> for Graph {
        /// A transition from a state that is not there has nowhere to be
        /// written and is left out; [`super::Graph::problems`] names it
        /// before it comes to that.
        fn from(g: super::Graph) -> Self {
            let exit = |t: &Transition| Exit {
                to: t.to.clone(),
                when: t.when.clone(),
                fade: t.fade,
            };
            let from = |name: &str| -> Vec<Exit> {
                g.transitions
                    .iter()
                    .filter(|t| t.from == name)
                    .map(exit)
                    .collect()
            };
            Graph {
                start: g.start.clone(),
                states: g
                    .states
                    .iter()
                    .map(|(name, s)| {
                        (
                            name.clone(),
                            State {
                                clip: s.clip.clone(),
                                blend: s.blend.clone(),
                                blend_by: s.blend_by.clone(),
                                directional: s.directional.clone(),
                                blend_by_y: s.blend_by_y.clone(),
                                events: s.events.clone(),
                                looping: s.looping,
                                speed: s.speed,
                                speed_from: s.speed_from.clone(),
                                time_from: s.time_from.clone(),
                                transitions: from(name),
                            },
                        )
                    })
                    .collect(),
                any: from(ANY),
                transitions: Vec::new(),
            }
        }
    }
}

/// The name Any State's transitions leave from.
pub const ANY: &str = "*";

impl Graph {
    /// Put the transitions in the order the file gives them — Any State's
    /// first, then each state's in name order — keeping the order within
    /// each, which is their priority. What the file says and what runs are
    /// then the same list.
    pub fn normalize(&mut self) {
        let rank = |from: &str| -> usize {
            if from == ANY {
                0
            } else {
                self.states
                    .keys()
                    .position(|k| k == from)
                    .map_or(usize::MAX, |i| i + 1)
            }
        };
        let mut ranked: Vec<(usize, Transition)> = self
            .transitions
            .drain(..)
            .map(|t| (rank(&t.from), t))
            .collect();
        ranked.sort_by_key(|(r, _)| *r);
        self.transitions = ranked.into_iter().map(|(_, t)| t).collect();
    }

    /// What cannot work, in words: a start or a transition naming a state
    /// that is not there, a state naming a clip the model does not have.
    /// What is wrong with the graph's shape, whatever the model: a state
    /// nothing leads to, a state nothing leaves that is not meant to hold
    /// (one playing once), and a transition that can never be taken
    /// because one before it from the same state always is.
    pub fn shape_problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        // Reachable from the start, by any transition (Any State's from
        // every state).
        let mut reached: std::collections::BTreeSet<&str> = Default::default();
        let mut next = vec![self.start.as_str()];
        while let Some(state) = next.pop() {
            if !reached.insert(state) {
                continue;
            }
            for t in &self.transitions {
                if t.from == state || t.from == ANY {
                    next.push(t.to.as_str());
                }
            }
        }
        for (name, state) in &self.states {
            if !reached.contains(name.as_str()) {
                out.push(format!(
                    "state `{name}` is never reached: no transition from `{}` leads there",
                    self.start
                ));
            }
            let leaves = self
                .transitions
                .iter()
                .any(|t| (t.from == *name || t.from == ANY) && t.to != *name);
            if !leaves && !state.looping && self.states.len() > 1 {
                out.push(format!(
                    "state `{name}` plays once and nothing leaves it: the character stops there"
                ));
            }
        }
        // First match wins: after an unconditional exit from a state, the
        // rest from it are never looked at.
        let mut open: std::collections::BTreeMap<&str, &str> = Default::default();
        for t in &self.transitions {
            if let Some(first) = open.get(t.from.as_str()) {
                out.push(format!(
                    "the transition `{}` → `{}` is never taken: `{}` → `{first}` before it always is",
                    t.from, t.to, t.from
                ));
                continue;
            }
            if t.when.is_empty() {
                open.insert(t.from.as_str(), t.to.as_str());
            }
        }
        out
    }

    /// Every parameter the graph reads: in conditions, blends and speeds.
    pub fn parameters(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        for t in &self.transitions {
            for c in &t.when {
                match c {
                    Condition::Above(p, _)
                    | Condition::Below(p, _)
                    | Condition::Is(p)
                    | Condition::Not(p)
                    | Condition::Trigger(p) => {
                        out.insert(p.clone());
                    }
                    Condition::Finished => {}
                }
            }
        }
        for state in self.states.values() {
            for p in [&state.blend_by, &state.blend_by_y] {
                if !p.is_empty() {
                    out.insert(p.clone());
                }
            }
            for p in [&state.speed_from, &state.time_from].into_iter().flatten() {
                out.insert(p.clone());
            }
        }
        out
    }

    pub fn problems(&self, clips: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        let states: Vec<&str> = self.states.keys().map(String::as_str).collect();
        let near = |name: &str, known: &[&str]| {
            crate::spelling::closest(name, known.iter().copied())
                .map(|n| format!(" — did you mean `{n}`?"))
                .unwrap_or_default()
        };
        if !self.states.contains_key(&self.start) {
            out.push(format!(
                "start `{}` is not a state{}",
                self.start,
                near(&self.start, &states)
            ));
        }
        for t in &self.transitions {
            for name in [&t.from, &t.to] {
                if name != ANY && !self.states.contains_key(name) {
                    out.push(format!(
                        "a transition names `{name}`, which is not a state{}",
                        near(name, &states)
                    ));
                }
            }
        }
        for (name, state) in &self.states {
            for (at, event) in &state.events {
                if !(0.0..=1.0).contains(at) {
                    out.push(format!(
                        "state `{name}`: event `{event}` at {at}, and the cycle runs 0 to 1"
                    ));
                }
            }
            if !state.blend.is_empty() {
                if state.blend_by.is_empty() {
                    out.push(format!(
                        "state `{name}` blends by no parameter: say blend_by"
                    ));
                }
                if state.blend.windows(2).any(|w| w[1].0 <= w[0].0) {
                    out.push(format!("state `{name}`'s blend values do not rise"));
                }
                for (_, clip) in &state.blend {
                    if !clips.contains(&clip.as_str()) {
                        out.push(format!(
                            "state `{name}` blends `{clip}`, which the model does not have{}",
                            near(clip, clips)
                        ));
                    }
                }
                continue;
            }
            if !state.directional.is_empty() {
                if state.blend_by.is_empty() || state.blend_by_y.is_empty() {
                    out.push(format!(
                        "state `{name}` blends in 2D: say blend_by and blend_by_y"
                    ));
                }
                for (_, _, clip) in &state.directional {
                    if !clips.contains(&clip.as_str()) {
                        out.push(format!(
                            "state `{name}` blends `{clip}`, which the model does not have{}",
                            near(clip, clips)
                        ));
                    }
                }
                continue;
            }
            if !clips.contains(&state.clip.as_str()) {
                out.push(format!(
                    "state `{name}` plays `{}`, which the model does not have{}",
                    state.clip,
                    near(&state.clip, clips)
                ));
            }
        }
        out
    }
}

/// A graph running on one entity: its state, and the game's parameters.
#[derive(Debug, Clone)]
pub struct Controller {
    pub graph: Graph,
    state: Option<String>,
    params: HashMap<String, f32>,
    triggers: HashSet<String>,
    /// Where in its cycle the state was at the last update.
    phase: Option<f32>,
    fired: Vec<String>,
    /// Updates so far: the clock the trail is told by.
    updates: u64,
    /// The last few transitions taken, oldest first.
    trail: std::collections::VecDeque<Passage>,
}

/// A transition taken: when (the controller's update count), from which
/// state to which, and what made it — for an agent reading a running game
/// to see why a character is where it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Passage {
    pub update: u64,
    /// Empty for the start.
    pub from: String,
    pub to: String,
    /// The conditions that held, in words; `start` for the first state.
    pub when: String,
}

impl std::fmt::Display for Passage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.from.is_empty() {
            write!(f, "#{} → {} ({})", self.update, self.to, self.when)
        } else {
            write!(
                f,
                "#{} {} → {} ({})",
                self.update, self.from, self.to, self.when
            )
        }
    }
}

/// Conditions in words: `speed > 0.1, jump pulled`.
pub fn describe(when: &[Condition]) -> String {
    if when.is_empty() {
        return "always".into();
    }
    when.iter()
        .map(|c| match c {
            Condition::Above(p, v) => format!("{p} > {v}"),
            Condition::Below(p, v) => format!("{p} < {v}"),
            Condition::Is(p) => p.clone(),
            Condition::Not(p) => format!("not {p}"),
            Condition::Trigger(p) => format!("{p} pulled"),
            Condition::Finished => "clip finished".into(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// How many transitions a controller remembers.
pub const TRAIL: usize = 16;

impl Controller {
    pub fn new(graph: Graph) -> Self {
        Self {
            graph,
            state: None,
            params: HashMap::new(),
            triggers: HashSet::new(),
            phase: None,
            fired: Vec::new(),
            updates: 0,
            trail: Default::default(),
        }
    }

    /// The last transitions taken, oldest first.
    pub fn trail(&self) -> impl Iterator<Item = &Passage> {
        self.trail.iter()
    }

    /// The state it is in; `None` before the first update.
    pub fn state(&self) -> Option<&str> {
        self.state.as_deref()
    }

    pub fn set(&mut self, name: &str, value: f32) {
        self.params.insert(name.to_string(), value);
    }

    pub fn set_bool(&mut self, name: &str, value: bool) {
        self.set(name, if value { 1.0 } else { 0.0 });
    }

    /// Pull a trigger. It lasts until the next update: a jump pressed while
    /// nothing can jump is not remembered for later.
    pub fn trigger(&mut self, name: &str) {
        self.triggers.insert(name.to_string());
    }

    fn param(&self, name: &str) -> f32 {
        self.params.get(name).copied().unwrap_or(0.0)
    }

    /// Which two of a blend state's clips, and how far between them, for
    /// the parameter's value now: below the first is all the first, above
    /// the last all the last.
    fn mix(&self, state: &State, animator: &Animator) -> Option<(usize, usize, f32)> {
        let index = |name: &str| animator.clips.iter().position(|c| c.name == name);
        let value = self.param(&state.blend_by);
        let points = &state.blend;
        let (first, last) = (points.first()?, points.last()?);
        if value <= first.0 {
            let a = index(&first.1)?;
            return Some((a, a, 0.0));
        }
        if value >= last.0 {
            let a = index(&last.1)?;
            return Some((a, a, 0.0));
        }
        let pair = points.windows(2).find(|w| value <= w[1].0)?;
        let weight = (value - pair[0].0) / (pair[1].0 - pair[0].0).max(1e-6);
        Some((index(&pair[0].1)?, index(&pair[1].1)?, weight))
    }

    /// A 2D blend state's mix for the parameters now: the two points either
    /// side of the direction, how far between them, and the middle point
    /// (if there is one) by how far short of their ring it is.
    #[allow(clippy::type_complexity)]
    fn mix_2d(
        &self,
        state: &State,
        animator: &Animator,
    ) -> Option<(usize, usize, f32, Option<(usize, f32)>)> {
        let index = |clip: &str| animator.clips.iter().position(|c| c.name == clip);
        let q = glam::Vec2::new(self.param(&state.blend_by), self.param(&state.blend_by_y));
        let points: Vec<(glam::Vec2, usize)> = state
            .directional
            .iter()
            .filter_map(|(x, y, clip)| Some((glam::Vec2::new(*x, *y), index(clip)?)))
            .collect();
        let middle = points
            .iter()
            .find(|(p, _)| p.length() < 1e-4)
            .map(|(_, c)| *c);
        let ring: Vec<(f32, f32, usize)> = points
            .iter()
            .filter(|(p, _)| p.length() >= 1e-4)
            .map(|(p, c)| (p.y.atan2(p.x), p.length(), *c))
            .collect();
        if ring.is_empty() || q.length() < 1e-4 {
            let c = middle.or(ring.first().map(|r| r.2))?;
            return Some((c, c, 0.0, None));
        }
        let heading = q.y.atan2(q.x);
        let turn = |a: f32| {
            let d = (a - heading).rem_euclid(std::f32::consts::TAU);
            if d > std::f32::consts::PI {
                d - std::f32::consts::TAU
            } else {
                d
            }
        };
        // The nearest on each side; of several at one heading, the one
        // whose distance is nearest the parameters'.
        let pick = |before: bool| {
            ring.iter()
                .map(|r| (turn(r.0), r))
                .filter(|(d, _)| if before { *d <= 0.0 } else { *d > 0.0 })
                .min_by(|(d1, r1), (d2, r2)| {
                    d1.abs().total_cmp(&d2.abs()).then(
                        (r1.1 - q.length())
                            .abs()
                            .total_cmp(&(r2.1 - q.length()).abs()),
                    )
                })
                .map(|(d, r)| (d, *r))
        };
        let (from, to) = match (pick(true), pick(false)) {
            (Some(a), Some(b)) => (a, b),
            (Some(a), None) | (None, Some(a)) => (a, a),
            (None, None) => return None,
        };
        let t = if to.1 .2 == from.1 .2 {
            0.0
        } else {
            -from.0 / (to.0 - from.0)
        };
        let edge = from.1 .1 + (to.1 .1 - from.1 .1) * t;
        let out = (q.length() / edge.max(1e-4)).min(1.0);
        let third = middle.map(|c| (c, 1.0 - out));
        Some((from.1 .2, to.1 .2, t, third))
    }

    /// Take the first transition whose conditions hold, if any, and play
    /// what the state says on `animator`. Returns the state entered.
    pub fn update(&mut self, animator: &mut Animator) -> Option<String> {
        self.updates += 1;
        let mut because = String::from("start");
        let entered = match &self.state {
            None => Some((self.graph.start.clone(), 0.0)),
            Some(now) => {
                let finished = animator.finished();
                self.graph
                    .transitions
                    .iter()
                    .filter(|t| (t.from == *now || t.from == ANY) && t.to != *now)
                    .find(|t| {
                        t.when.iter().all(|c| match c {
                            Condition::Above(p, v) => self.param(p) > *v,
                            Condition::Below(p, v) => self.param(p) < *v,
                            Condition::Is(p) => self.param(p) > 0.5,
                            Condition::Not(p) => self.param(p) <= 0.5,
                            Condition::Trigger(name) => self.triggers.contains(name),
                            Condition::Finished => finished,
                        })
                    })
                    .map(|t| {
                        because = describe(&t.when);
                        (t.to.clone(), t.fade)
                    })
            }
        };
        if let Some((to, _)) = &entered {
            self.trail.push_back(Passage {
                update: self.updates,
                from: self.state.clone().unwrap_or_default(),
                to: to.clone(),
                when: because,
            });
            while self.trail.len() > TRAIL {
                self.trail.pop_front();
            }
        }
        self.triggers.clear();
        if let Some((name, fade)) = &entered {
            if let Some(state) = self
                .graph
                .states
                .get(name)
                .filter(|s| !s.directional.is_empty())
            {
                if let Some((a, b, w, third)) = self.mix_2d(state, animator) {
                    animator.blend_three(a, b, w, third, *fade);
                }
            } else if let Some(state) = self.graph.states.get(name).filter(|s| !s.blend.is_empty())
            {
                if let Some((a, b, w)) = self.mix(state, animator) {
                    // From a plain clip: fade in. Between blend states:
                    // the same cycle, other clips.
                    animator.blend(a, b, w, *fade);
                }
            } else if let Some(state) = self.graph.states.get(name) {
                if let Some(clip) = animator.clips.iter().position(|c| c.name == state.clip) {
                    if state.looping {
                        animator.play(clip, *fade);
                    } else {
                        animator.play_once(clip, *fade);
                    }
                }
            }
            self.state = Some(name.clone());
        }
        if let Some(state) = self.state.as_ref().and_then(|s| self.graph.states.get(s)) {
            if entered.is_none() && !state.directional.is_empty() {
                if let Some((a, b, w, third)) = self.mix_2d(state, animator) {
                    animator.blend_three(a, b, w, third, 0.0);
                }
            }
            if entered.is_none() && !state.blend.is_empty() {
                if let Some((a, b, w)) = self.mix(state, animator) {
                    animator.blend(a, b, w, 0.0);
                }
            }
            if let Some(p) = state.time_from.as_deref() {
                animator.set_speed(0.0);
                animator.set_fraction(self.param(p));
            } else {
                let factor = state.speed_from.as_deref().map_or(1.0, |p| self.param(p));
                animator.set_speed(state.speed * factor);
            }
        }
        self.fire_events(animator, entered.is_some());
        entered.map(|(name, _)| name)
    }

    /// The events of the state that passed since the last update, in order.
    pub fn fired(&self) -> &[String] {
        &self.fired
    }

    fn fire_events(&mut self, animator: &Animator, entered: bool) {
        self.fired.clear();
        let now = match (animator.blending(), animator.playing()) {
            (Some(blend), _) => Some(blend.phase),
            (None, Some(playing)) => animator
                .clips
                .get(playing.clip)
                .filter(|c| c.duration > 0.0)
                .map(|c| {
                    let t = playing.time / c.duration;
                    if playing.looping {
                        t.rem_euclid(1.0)
                    } else {
                        t.min(1.0)
                    }
                }),
            _ => None,
        };
        let before = if entered { None } else { self.phase };
        self.phase = now;
        let (Some(state), Some(now)) = (
            self.state.as_ref().and_then(|s| self.graph.states.get(s)),
            now,
        ) else {
            return;
        };
        // From where it was to where it is, round the end of the cycle if it
        // went round; a state just entered starts from its beginning.
        let from = before.unwrap_or(-f32::EPSILON);
        let passed = |at: f32| {
            if now >= from {
                at > from && at <= now
            } else {
                at > from || at <= now
            }
        };
        self.fired.extend(
            state
                .events
                .iter()
                .filter(|(at, _)| passed(*at))
                .map(|(_, name)| name.clone()),
        );
    }
}

/// One difference between two versions of a graph: what an agent's edit
/// did, for a person to look over and take back piece by piece.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    StateAdded(String),
    StateRemoved(String, State),
    /// The state is there in both, and plays or behaves otherwise.
    StateChanged(String, State),
    TransitionAdded(Transition),
    TransitionRemoved(Transition),
    /// The start was the first.
    StartMoved(String),
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let edge = |t: &Transition| format!("{} → {} when {}", t.from, t.to, describe(&t.when));
        match self {
            Change::StateAdded(n) => write!(f, "+ state {n}"),
            Change::StateRemoved(n, _) => write!(f, "− state {n}"),
            Change::StateChanged(n, _) => write!(f, "~ state {n}"),
            Change::TransitionAdded(t) => write!(f, "+ {}", edge(t)),
            Change::TransitionRemoved(t) => write!(f, "− {}", edge(t)),
            Change::StartMoved(was) => write!(f, "~ start (was {was})"),
        }
    }
}

/// What changed from `before` to `after`, states first.
pub fn diff(before: &Graph, after: &Graph) -> Vec<Change> {
    let mut out = Vec::new();
    for (name, state) in &after.states {
        match before.states.get(name) {
            None => out.push(Change::StateAdded(name.clone())),
            Some(was) if was != state => out.push(Change::StateChanged(name.clone(), was.clone())),
            _ => {}
        }
    }
    for (name, state) in &before.states {
        if !after.states.contains_key(name) {
            out.push(Change::StateRemoved(name.clone(), state.clone()));
        }
    }
    if before.start != after.start {
        out.push(Change::StartMoved(before.start.clone()));
    }
    for t in &after.transitions {
        if !before.transitions.contains(t) {
            out.push(Change::TransitionAdded(t.clone()));
        }
    }
    for t in &before.transitions {
        if !after.transitions.contains(t) {
            out.push(Change::TransitionRemoved(t.clone()));
        }
    }
    out
}

impl Change {
    /// Take this change back in `graph`, as it was before.
    pub fn revert(&self, graph: &mut Graph) {
        match self {
            Change::StateAdded(n) => {
                graph.states.remove(n);
                graph.transitions.retain(|t| t.from != *n && t.to != *n);
            }
            Change::StateRemoved(n, state) | Change::StateChanged(n, state) => {
                graph.states.insert(n.clone(), state.clone());
            }
            Change::TransitionAdded(t) => {
                if let Some(i) = graph.transitions.iter().position(|x| x == t) {
                    graph.transitions.remove(i);
                }
            }
            Change::TransitionRemoved(t) => graph.transitions.push(t.clone()),
            Change::StartMoved(was) => graph.start = was.clone(),
        }
    }

    /// The state this change is about, if it is one: what the canvas marks.
    pub fn state(&self) -> Option<&str> {
        match self {
            Change::StateAdded(n) | Change::StateRemoved(n, _) | Change::StateChanged(n, _) => {
                Some(n)
            }
            _ => None,
        }
    }
}

/// A graph's cases: what it should do, played without the game — as
/// `animators/<name>.cases.ron` beside it, which `runity check` plays.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Cases {
    /// How long each clip is, seconds; any not named is a second.
    #[serde(default)]
    pub clips: BTreeMap<String, f32>,
    pub cases: Vec<Case>,
}

/// One case: steps from the start state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub name: String,
    pub steps: Vec<Step>,
}

/// Parameters set, triggers pulled, time let pass — then where the graph
/// should stand.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Step {
    #[serde(default)]
    pub set: BTreeMap<String, f32>,
    #[serde(default)]
    pub trigger: Vec<String>,
    /// Seconds, a thirtieth at a time.
    #[serde(default)]
    pub wait: f32,
    pub expect: String,
}

impl Cases {
    /// Play every case on `graph`: what went otherwise, in words.
    pub fn run(&self, graph: &Graph) -> Vec<String> {
        use crate::animation::{Channel, Clip, Joint, Path, PoseTransform, Skeleton};
        use std::sync::Arc;
        let skeleton = Arc::new(Skeleton {
            joints: vec![Joint {
                name: "root".into(),
                parent: None,
                inverse_bind: glam::Mat4::IDENTITY.to_cols_array_2d(),
                rest: PoseTransform::default(),
            }],
        });
        let mut names: HashSet<String> = HashSet::new();
        for state in graph.states.values() {
            names.insert(state.clip.clone());
            names.extend(state.blend.iter().map(|(_, c)| c.clone()));
            names.extend(state.directional.iter().map(|(_, _, c)| c.clone()));
        }
        let clips: Vec<Clip> = names
            .into_iter()
            .filter(|n| !n.is_empty())
            .map(|name| {
                let duration = self.clips.get(&name).copied().unwrap_or(1.0);
                Clip {
                    name,
                    duration,
                    channels: vec![Channel {
                        joint: 0,
                        path: Path::Translation,
                        times: vec![0.0, duration],
                        values: vec![0.0; 6],
                    }],
                }
            })
            .collect();
        let clips = Arc::new(clips);
        let mut out = Vec::new();
        for case in &self.cases {
            let mut animator = Animator::new(skeleton.clone(), clips.clone());
            let mut controller = Controller::new(graph.clone());
            controller.update(&mut animator);
            for (i, step) in case.steps.iter().enumerate() {
                for (name, value) in &step.set {
                    controller.set(name, *value);
                }
                for name in &step.trigger {
                    controller.trigger(name);
                }
                controller.update(&mut animator);
                let mut left = step.wait;
                while left > 1e-6 {
                    let dt = left.min(1.0 / 30.0);
                    animator.advance(dt);
                    controller.update(&mut animator);
                    left -= dt;
                }
                let now = controller.state().unwrap_or("");
                if now != step.expect {
                    out.push(format!(
                        "case `{}`, step {}: in `{now}`, expected `{}`",
                        case.name,
                        i + 1,
                        step.expect
                    ));
                    break;
                }
            }
        }
        out
    }
}

/// Update every entity's [`Controller`] against its [`Animator`]: call it
/// in the fixed step before [`crate::advance_animations`].
pub fn run_controllers(world: &mut hecs::World) {
    for (animator, controller) in world.query_mut::<(&mut Animator, &mut Controller)>() {
        controller.update(animator);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Channel, Clip, Joint, Path, PoseTransform, Skeleton};
    use std::sync::Arc;

    const GRAPH: &str = r#"(
        start: "idle",
        states: {
            "idle": (clip: "idle", transitions: [
                (to: "walk", when: [Above("speed", 0.1)]),
            ]),
            "walk": (clip: "walk", speed_from: "speed", transitions: [
                (to: "idle", when: [Below("speed", 0.1)]),
            ]),
            "jump": (clip: "jump", looping: false, transitions: [
                (to: "idle", when: [Finished]),
            ]),
        },
        any: [(to: "jump", when: [Trigger("jump")], fade: 0.1)],
    )"#;

    #[test]
    fn transitions_are_written_in_the_state_they_leave() {
        let graph: Graph = ron::from_str(GRAPH).unwrap();
        // Any State's first, then by state; each keeps its priority.
        let order: Vec<(&str, &str)> = graph
            .transitions
            .iter()
            .map(|t| (t.from.as_str(), t.to.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                ("*", "jump"),
                ("idle", "walk"),
                ("jump", "idle"),
                ("walk", "idle")
            ]
        );
        let text = ron::ser::to_string_pretty(&graph, Default::default()).unwrap();
        assert!(
            text.lines().all(|l| !l.trim_start().starts_with("from:")),
            "no `from` in the file: {text}"
        );
        assert!(text.contains("any:"), "{text}");
        assert_eq!(ron::from_str::<Graph>(&text).unwrap(), graph);
    }

    #[test]
    fn a_file_of_the_older_shape_still_reads() {
        let old: Graph = ron::from_str(
            r#"(
            start: "idle",
            states: {"idle": (clip: "idle"), "walk": (clip: "walk")},
            transitions: [
                (from: "walk", to: "idle", when: [Below("speed", 0.1)]),
                (from: "idle", to: "walk", when: [Above("speed", 0.1)]),
                (from: "*", to: "idle", when: [Trigger("reset")]),
            ],
        )"#,
        )
        .unwrap();
        let new: Graph = ron::from_str(
            r#"(
            start: "idle",
            states: {
                "idle": (clip: "idle", transitions: [(to: "walk", when: [Above("speed", 0.1)])]),
                "walk": (clip: "walk", transitions: [(to: "idle", when: [Below("speed", 0.1)])]),
            },
            any: [(to: "idle", when: [Trigger("reset")])],
        )"#,
        )
        .unwrap();
        assert_eq!(old, new);
    }

    fn animator() -> Animator {
        let skeleton = Arc::new(Skeleton {
            joints: vec![Joint {
                name: "root".into(),
                parent: None,
                inverse_bind: glam::Mat4::IDENTITY.to_cols_array_2d(),
                rest: PoseTransform::default(),
            }],
        });
        let clip = |name: &str, duration: f32| Clip {
            name: name.into(),
            duration,
            channels: vec![Channel {
                joint: 0,
                path: Path::Translation,
                times: vec![0.0, duration],
                values: vec![0.0; 6],
            }],
        };
        Animator::new(
            skeleton,
            Arc::new(vec![
                clip("idle", 1.0),
                clip("walk", 1.0),
                clip("jump", 0.5),
            ]),
        )
    }

    fn playing(animator: &Animator) -> String {
        animator.clips[animator.playing().unwrap().clip]
            .name
            .clone()
    }

    #[test]
    fn a_report_of_the_world_says_which_state_each_controller_is_in() {
        let graph: Graph = ron::from_str(GRAPH).unwrap();
        let mut animator = animator();
        let mut controller = Controller::new(graph);
        controller.update(&mut animator);
        let mut world = hecs::World::new();
        let id = runity_core::EntityId::fresh();
        world.spawn((
            crate::world::SceneId(id),
            crate::scene::Transform::default(),
            controller,
        ));
        let report = runity_core::save::capture_with(
            &world,
            &runity_core::components::Components::new(),
            &runity_core::Scene::default(),
            &animator_state,
        );
        assert_eq!(report.entities[0].animator, "idle");
        let text = ron::to_string(&report).unwrap();
        let back: runity_core::save::SaveGame = ron::from_str(&text).unwrap();
        assert_eq!(back, report);
    }

    #[test]
    fn parameters_pick_the_clip_and_the_game_never_names_one() {
        let graph: Graph = ron::from_str(GRAPH).unwrap();
        assert!(graph.problems(&["idle", "walk", "jump"]).is_empty());
        let mut animator = animator();
        let mut controller = Controller::new(graph);

        assert_eq!(controller.update(&mut animator).as_deref(), Some("idle"));
        assert_eq!(playing(&animator), "idle");

        controller.set("speed", 2.0);
        assert_eq!(controller.update(&mut animator).as_deref(), Some("walk"));
        assert_eq!(animator.playing().unwrap().speed, 2.0, "faster when faster");
        assert_eq!(controller.update(&mut animator), None, "stays walking");

        // A jump from anywhere, once, and back when it has played out.
        controller.trigger("jump");
        assert_eq!(controller.update(&mut animator).as_deref(), Some("jump"));
        assert_eq!(
            controller.update(&mut animator),
            None,
            "the trigger was used"
        );
        animator.advance(0.6);
        assert_eq!(controller.update(&mut animator).as_deref(), Some("idle"));
    }

    #[test]
    fn a_parameter_scrubs_a_clip_that_does_not_play_on_its_own() {
        let graph: Graph = ron::from_str(
            r#"(start: "build", states: {"build": (clip: "jump", time_from: "health")})"#,
        )
        .unwrap();
        let mut animator = animator();
        let mut controller = Controller::new(graph);
        controller.set("health", 0.5);
        controller.update(&mut animator);
        animator.advance(1.0);
        controller.update(&mut animator);
        let at = animator.playing().unwrap();
        assert_eq!(at.time, 0.25, "half of a half-second clip, time or no time");
        assert_eq!(at.speed, 0.0);
        controller.set("health", 1.0);
        controller.update(&mut animator);
        assert_eq!(animator.playing().unwrap().time, 0.5);
    }

    #[test]
    fn a_2d_blend_mixes_by_direction_and_by_how_far_out() {
        let graph: Graph = ron::from_str(
            r#"(start: "move", states: {"move": (
                blend_by: "x", blend_by_y: "y",
                directional: [(0.0, 0.0, "idle"), (0.0, 1.0, "walk"), (1.0, 0.0, "jump")],
            )})"#,
        )
        .unwrap();
        assert!(graph.problems(&["idle", "walk", "jump"]).is_empty());
        let mut animator = animator();
        let mut controller = Controller::new(graph);
        controller.set("x", 0.5);
        controller.set("y", 0.5);
        controller.update(&mut animator);
        let blend = animator.blending().expect("a blend");
        // Between right (jump, 0°) and forward (walk, 90°), halfway round;
        // 0.71 of the way out, so 0.29 standing.
        assert_eq!((blend.a, blend.b), (2, 1), "{blend:?}");
        assert!((blend.weight - 0.5).abs() < 1e-5, "{blend:?}");
        let (middle, w) = blend.third.unwrap();
        assert_eq!(middle, 0);
        assert!((w - (1.0 - 0.5f32.hypot(0.5))).abs() < 1e-5, "{blend:?}");
        // All the way forward: walk only.
        controller.set("x", 0.0);
        controller.set("y", 2.0);
        controller.update(&mut animator);
        let blend = animator.blending().unwrap();
        let clip = if blend.weight < 0.5 { blend.a } else { blend.b };
        assert_eq!(clip, 1, "{blend:?}");
        assert_eq!(blend.third.unwrap().1, 0.0);
        animator.advance(0.1);
    }

    #[test]
    fn a_blend_state_mixes_the_two_clips_either_side_of_the_parameter() {
        let graph: Graph = ron::from_str(
            r#"(
            start: "move",
            states: {
                "move": (blend_by: "speed", blend: [(0.0, "idle"), (2.0, "walk")], transitions: [
                    (to: "jump", when: [Trigger("jump")], fade: 0.1),
                ]),
                "jump": (clip: "jump", looping: false, transitions: [(to: "move", when: [Finished])]),
            },
        )"#,
        )
        .unwrap();
        assert!(graph.problems(&["idle", "walk", "jump"]).is_empty());
        assert!(graph
            .problems(&["idle", "jump"])
            .iter()
            .any(|p| p.contains("blends `walk`")));
        let mut animator = animator();
        let mut controller = Controller::new(graph);

        controller.set("speed", 0.5);
        controller.update(&mut animator);
        let blend = animator.blending().expect("a blend");
        assert_eq!((blend.a, blend.b), (0, 1));
        assert!((blend.weight - 0.25).abs() < 1e-6, "{blend:?}");

        // The weight follows the parameter and the cycle keeps going.
        animator.advance(0.3);
        controller.set("speed", 1.5);
        controller.update(&mut animator);
        let later = animator.blending().unwrap();
        assert!((later.weight - 0.75).abs() < 1e-6);
        assert!(later.phase > 0.0, "the cycle kept going: {later:?}");
        controller.set("speed", 9.0);
        controller.update(&mut animator);
        assert_eq!(animator.blending().unwrap().a, 1, "all walk past the end");

        // Out to a plain clip and back.
        controller.trigger("jump");
        assert_eq!(controller.update(&mut animator).as_deref(), Some("jump"));
        assert!(animator.blending().is_none());
        animator.advance(0.6);
        assert_eq!(controller.update(&mut animator).as_deref(), Some("move"));
        assert!(animator.blending().is_some());
        assert_eq!(animator.advance(0.05).len(), 1, "a pose for the one joint");
    }

    #[test]
    fn events_fire_once_as_the_cycle_passes_them() {
        let graph: Graph = ron::from_str(
            r#"(start: "walk", states: { "walk": (clip: "walk", events: [(0.25, "left"), (0.75, "right")]) })"#,
        )
        .unwrap();
        let mut animator = animator();
        let mut controller = Controller::new(graph);
        controller.update(&mut animator);
        assert!(controller.fired().is_empty(), "{:?}", controller.fired());
        let mut heard = Vec::new();
        for _ in 0..20 {
            animator.advance(0.1);
            controller.update(&mut animator);
            heard.extend(controller.fired().iter().cloned());
        }
        // Two seconds of a one-second walk: each foot twice, in order.
        assert_eq!(heard, ["left", "right", "left", "right"]);
    }

    #[test]
    fn a_graph_says_what_is_never_reached_never_left_or_never_taken() {
        let graph: Graph = ron::from_str(
            r#"(start: "idle", states: {
                "idle": (clip: "idle", transitions: [
                    (to: "walk"),
                    (to: "jump", when: [Trigger("jump")]),
                ]),
                "walk": (clip: "walk", speed_from: "speed"),
                "jump": (clip: "jump", looping: false),
                "swim": (clip: "idle"),
            })"#,
        )
        .unwrap();
        let found = graph.shape_problems();
        assert!(
            found.iter().any(|p| p.contains("`swim` is never reached")),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .any(|p| p.contains("`idle` → `jump` is never taken")),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .any(|p| p.contains("`jump` plays once and nothing leaves")),
            "{found:?}"
        );
        let wanted: Vec<String> = graph.parameters().into_iter().collect();
        assert_eq!(wanted, ["jump", "speed"]);
    }

    #[test]
    fn a_controller_remembers_which_way_it_went_and_why() {
        let graph: Graph = ron::from_str(GRAPH).unwrap();
        let mut animator = animator();
        let mut controller = Controller::new(graph);
        controller.update(&mut animator);
        controller.set("speed", 2.0);
        controller.update(&mut animator);
        let trail: Vec<String> = controller.trail().map(|p| p.to_string()).collect();
        assert_eq!(trail, ["#1 → idle (start)", "#2 idle → walk (speed > 0.1)"]);
    }

    #[test]
    fn an_edit_is_a_list_of_changes_each_taken_back_on_its_own() {
        let before: Graph = ron::from_str(GRAPH).unwrap();
        let mut after = before.clone();
        after.states.remove("jump");
        after
            .transitions
            .retain(|t| t.to != "jump" && t.from != "jump");
        after
            .states
            .insert("swim".into(), after.states["idle"].clone());
        after.states.get_mut("walk").unwrap().speed = 2.0;
        after.transitions.push(Transition {
            from: "idle".into(),
            to: "swim".into(),
            when: vec![Condition::Is("wet".into())],
            fade: 0.2,
        });
        let changes = diff(&before, &after);
        let said: Vec<String> = changes.iter().map(ToString::to_string).collect();
        assert!(said.contains(&"+ state swim".to_string()), "{said:?}");
        assert!(said.contains(&"~ state walk".to_string()), "{said:?}");
        assert!(said.contains(&"− state jump".to_string()), "{said:?}");
        assert!(
            said.contains(&"+ idle → swim when wet".to_string()),
            "{said:?}"
        );
        // Taking every change back is where it started.
        let mut back = after.clone();
        for change in &changes {
            change.revert(&mut back);
        }
        assert!(
            diff(&before, &back).is_empty(),
            "{:?}",
            diff(&before, &back)
        );
        // One alone: only walk's speed goes back.
        let mut one = after.clone();
        changes
            .iter()
            .find(|c| c.state() == Some("walk"))
            .unwrap()
            .revert(&mut one);
        assert_eq!(one.states["walk"].speed, before.states["walk"].speed);
        assert!(one.states.contains_key("swim"));
    }

    #[test]
    fn cases_play_the_graph_without_the_game() {
        let graph: Graph = ron::from_str(GRAPH).unwrap();
        let cases: Cases = ron::from_str(
            r#"(clips: {"jump": 0.5}, cases: [
                (name: "walks when fast, jumps, lands running", steps: [
                    (expect: "idle"),
                    (set: {"speed": 2.0}, expect: "walk"),
                    (trigger: ["jump"], expect: "jump"),
                    (wait: 0.6, expect: "walk"),
                    (set: {"speed": 0.0}, expect: "idle"),
                ]),
                (name: "a wrong one", steps: [(set: {"speed": 2.0}, expect: "idle")]),
            ])"#,
        )
        .unwrap();
        let failed = cases.run(&graph);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(
            failed[0].contains("`a wrong one`, step 1: in `walk`"),
            "{failed:?}"
        );
    }

    #[test]
    fn a_graph_that_cannot_work_says_why() {
        let graph: Graph = ron::from_str(&GRAPH.replace(
            "(to: \"idle\", when: [Finished])",
            "(to: \"idel\", when: [Finished])",
        ))
        .unwrap();
        let problems = graph.problems(&["idle", "walk"]);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`idel`") && p.contains("did you mean `idle`?")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("plays `jump`, which the model does not have")),
            "{problems:?}"
        );
    }
}


/// Every controller's trail, by the entity's id.
pub fn animator_trails(world: &hecs::World) -> Vec<(crate::id::EntityId, Vec<String>)> {
    let mut out: Vec<(crate::id::EntityId, Vec<String>)> = world
        .query::<(&crate::world::SceneId, &crate::animgraph::Controller)>()
        .iter()
        .map(|(id, c)| (id.0, c.trail().map(|p| p.to_string()).collect()))
        .collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

pub fn animator_state(world: &hecs::World, entity: hecs::Entity) -> String {
    world
        .get::<&crate::animgraph::Controller>(entity)
        .ok()
        .and_then(|c| c.state().map(str::to_string))
        .unwrap_or_default()
}

