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
        pub events: Vec<(f32, String)>,
        #[serde(default = "yes")]
        pub looping: bool,
        #[serde(default = "one")]
        pub speed: f32,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
        pub speed_from: Option<String>,
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
                        events: s.events,
                        looping: s.looping,
                        speed: s.speed,
                        speed_from: s.speed_from,
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
                                events: s.events.clone(),
                                looping: s.looping,
                                speed: s.speed,
                                speed_from: s.speed_from.clone(),
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
}

impl Controller {
    pub fn new(graph: Graph) -> Self {
        Self {
            graph,
            state: None,
            params: HashMap::new(),
            triggers: HashSet::new(),
            phase: None,
            fired: Vec::new(),
        }
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

    /// Take the first transition whose conditions hold, if any, and play
    /// what the state says on `animator`. Returns the state entered.
    pub fn update(&mut self, animator: &mut Animator) -> Option<String> {
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
                    .map(|t| (t.to.clone(), t.fade))
            }
        };
        self.triggers.clear();
        if let Some((name, fade)) = &entered {
            if let Some(state) = self.graph.states.get(name).filter(|s| !s.blend.is_empty()) {
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
            if entered.is_none() && !state.blend.is_empty() {
                if let Some((a, b, w)) = self.mix(state, animator) {
                    animator.blend(a, b, w, 0.0);
                }
            }
            let factor = state.speed_from.as_deref().map_or(1.0, |p| self.param(p));
            animator.set_speed(state.speed * factor);
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
        let id = crate::EntityId::fresh();
        world.spawn((
            crate::world::SceneId(id),
            crate::scene::Transform::default(),
            controller,
        ));
        let report = crate::save::capture(
            &world,
            &crate::components::Components::new(),
            &crate::Scene::default(),
        );
        assert_eq!(report.entities[0].animator, "idle");
        let text = ron::to_string(&report).unwrap();
        let back: crate::save::SaveGame = ron::from_str(&text).unwrap();
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
