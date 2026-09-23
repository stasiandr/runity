//! Which animation plays when: states and transitions, as a file.
//!
//! Unity's Animator Controller, as RON a diff shows and a designer edits
//! while the game runs:
//!
//! ```text
//! (
//!     start: "idle",
//!     states: {
//!         "idle": (clip: "idle"),
//!         "walk": (clip: "walk", speed_from: "speed"),
//!         "move": (blend_by: "speed", blend: [(0.0, "idle"), (2.0, "walk")]),
//!         "jump": (clip: "jump", looping: false),
//!     },
//!     transitions: [
//!         (from: "idle", to: "walk", when: [Above("speed", 0.1)]),
//!         (from: "walk", to: "idle", when: [Below("speed", 0.1)]),
//!         (from: "*",    to: "jump", when: [Trigger("jump")], fade: 0.1),
//!         (from: "jump", to: "idle", when: [Finished]),
//!     ],
//! )
//! ```
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

/// A whole graph: its file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    pub start: String,
    pub states: BTreeMap<String, State>,
    #[serde(default)]
    pub transitions: Vec<Transition>,
}

impl Graph {
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
                if name != "*" && !self.states.contains_key(name) {
                    out.push(format!(
                        "a transition names `{name}`, which is not a state{}",
                        near(name, &states)
                    ));
                }
            }
        }
        for (name, state) in &self.states {
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
}

impl Controller {
    pub fn new(graph: Graph) -> Self {
        Self {
            graph,
            state: None,
            params: HashMap::new(),
            triggers: HashSet::new(),
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
                    .filter(|t| (t.from == *now || t.from == "*") && t.to != *now)
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
        entered.map(|(name, _)| name)
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
            "idle": (clip: "idle"),
            "walk": (clip: "walk", speed_from: "speed"),
            "jump": (clip: "jump", looping: false),
        },
        transitions: [
            (from: "idle", to: "walk", when: [Above("speed", 0.1)]),
            (from: "walk", to: "idle", when: [Below("speed", 0.1)]),
            (from: "*", to: "jump", when: [Trigger("jump")], fade: 0.1),
            (from: "jump", to: "idle", when: [Finished]),
        ],
    )"#;

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
                "move": (blend_by: "speed", blend: [(0.0, "idle"), (2.0, "walk")]),
                "jump": (clip: "jump", looping: false),
            },
            transitions: [
                (from: "move", to: "jump", when: [Trigger("jump")], fade: 0.1),
                (from: "jump", to: "move", when: [Finished]),
            ],
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
    fn a_graph_that_cannot_work_says_why() {
        let graph: Graph = ron::from_str(&GRAPH.replace(
            "to: \"idle\", when: [Finished]",
            "to: \"idel\", when: [Finished]",
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
