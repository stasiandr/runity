//! Quests: a named row of stages over the flags and numbers dialogues and
//! the game set — `quests/<name>.ron`. A quest keeps nothing of its own:
//! where it stands is read off the dialogues' [`State`], so a save, a
//! merge and a dialogue's cases all see the same thing.
//!
//! ```ron
//! (
//!     title: "@seeds.title",
//!     stages: [
//!         (name: "ask", text: "@seeds.ask", done_when: [Is("agreed")]),
//!         (name: "bring", text: "@seeds.bring", done_when: [Var("seeds", Ge, 3)]),
//!     ],
//! )
//! ```

use serde::{Deserialize, Serialize};

use crate::dialogue::{Condition, State};

/// The folder quests are in.
pub const DIR: &str = "quests";

/// One step of a quest: done when all its conditions hold.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub name: String,
    /// What the journal says while this is the stage; `@key` from
    /// `strings/`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    pub done_when: Vec<Condition>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Quest {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub stages: Vec<Stage>,
}

impl Quest {
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = scrap_core::files::read_to_string(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The stage it is at: the first not yet done. `None` when all are.
    pub fn stage(&self, state: &State) -> Option<&Stage> {
        self.stages
            .iter()
            .find(|s| !s.done_when.iter().all(|c| c.holds(state)))
    }

    /// Whether every stage is done.
    pub fn done(&self, state: &State) -> bool {
        self.stage(state).is_none()
    }

    /// The flags and numbers it reads.
    pub fn reads(&self) -> Vec<String> {
        self.stages
            .iter()
            .flat_map(|s| s.done_when.iter().map(|c| c.name().to_string()))
            .collect()
    }

    /// Every `@key` its title and stages ask `strings/` for.
    pub fn keys(&self) -> Vec<String> {
        std::iter::once(&self.title)
            .chain(self.stages.iter().map(|s| &s.text))
            .filter_map(|t| t.strip_prefix('@').map(str::to_string))
            .collect()
    }

    /// What will not work: no stages, a stage named twice, a stage done
    /// from the start.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.stages.is_empty() {
            out.push("a quest with no stages is done before it starts".to_string());
        }
        for (i, stage) in self.stages.iter().enumerate() {
            if self.stages[..i].iter().any(|s| s.name == stage.name) {
                out.push(format!("stage `{}` is named twice", stage.name));
            }
            if stage.done_when.is_empty() {
                out.push(format!(
                    "stage `{}` has no done_when: it is done from the start",
                    stage.name
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quest_stands_at_its_first_stage_not_done() {
        let q: Quest = ron::from_str(
            r#"(title: "@seeds.title", stages: [
                (name: "ask", text: "@seeds.ask", done_when: [Is("agreed")]),
                (name: "bring", done_when: [Var("seeds", Ge, 3)]),
            ])"#,
        )
        .unwrap();
        assert!(q.problems().is_empty());
        let mut state = State::default();
        assert_eq!(q.stage(&state).unwrap().name, "ask");
        state.flags.insert("agreed".into());
        assert_eq!(q.stage(&state).unwrap().name, "bring");
        state.vars.insert("seeds".into(), 3);
        assert!(q.done(&state));
        assert_eq!(q.reads(), ["agreed", "seeds"]);
        assert_eq!(q.keys(), ["seeds.title", "seeds.ask"]);
        let bad: Quest = ron::from_str(
            r#"(stages: [(name: "a", done_when: []), (name: "a", done_when: [Is("x")])])"#,
        )
        .unwrap();
        assert_eq!(bad.problems().len(), 2, "{:?}", bad.problems());
    }
}
