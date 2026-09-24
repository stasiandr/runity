//! Dialogue: who says what, and what the player can answer — a graph of
//! lines in `dialogues/<name>.ron`, as a graph of states is an animator
//! (DNA, «Графовые редакторы»: the same widget, the same kind of checks,
//! editable as text by a person or an agent).
//!
//! ```ron
//! (
//!     start: "hello",
//!     lines: {
//!         "hello": (speaker: "Captain", text: "@captain.hello", next: "ask"),
//!         "ask": (speaker: "Captain", text: "@captain.ask", choices: [
//!             (text: "@yes", to: "thanks", set: ["agreed"], event: "give_seeds"),
//!             (text: "@no", to: "bye", when: [Not("broke")]),
//!         ]),
//!         "thanks": (speaker: "Captain", text: "@captain.thanks"),
//!         "bye": (speaker: "Captain", text: "@captain.bye"),
//!     },
//! )
//! ```
//!
//! Texts starting `@` are keys in `strings/` (as screens' are). The game
//! keeps the flags — what happened, what was agreed — and hears the
//! events a choice or a line fires. A line with neither `next` nor
//! choices ends the conversation.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The folder dialogues are in.
pub const DIR: &str = "dialogues";

/// A condition on the game's flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flag {
    Is(String),
    Not(String),
}

impl Flag {
    fn holds(&self, flags: &BTreeSet<String>) -> bool {
        match self {
            Flag::Is(f) => flags.contains(f),
            Flag::Not(f) => !flags.contains(f),
        }
    }
}

/// One line someone says.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Line {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub speaker: String,
    pub text: String,
    /// The line after it, when there is no choosing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub next: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    /// Flags set when it is said.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set: Vec<String>,
    /// Told to the game when it is said.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub event: String,
}

/// An answer the player can give.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub text: String,
    pub to: String,
    /// Offered only when these hold.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<Flag>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub event: String,
}

/// A whole conversation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Dialogue {
    pub start: String,
    pub lines: BTreeMap<String, Line>,
}

impl Dialogue {
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = runity_core::files::read_to_string(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Every way on from a line: its `next` and its choices' `to`.
    pub fn edges(&self) -> Vec<(&str, &str)> {
        let mut out = Vec::new();
        for (name, line) in &self.lines {
            if !line.next.is_empty() {
                out.push((name.as_str(), line.next.as_str()));
            }
            for choice in &line.choices {
                out.push((name.as_str(), choice.to.as_str()));
            }
        }
        out
    }

    /// What will not work: a line that names one that is not there, a line
    /// nothing leads to, and a choice with both a next and choices.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let names: Vec<&str> = self.lines.keys().map(String::as_str).collect();
        let near = |name: &str| {
            crate::spelling::closest(name, names.iter().copied())
                .map(|n| format!(" — did you mean `{n}`?"))
                .unwrap_or_default()
        };
        if !self.lines.contains_key(&self.start) {
            out.push(format!(
                "start `{}` is not a line{}",
                self.start,
                near(&self.start)
            ));
        }
        for (from, to) in self.edges() {
            if !self.lines.contains_key(to) {
                out.push(format!(
                    "line `{from}` leads to `{to}`, which is not a line{}",
                    near(to)
                ));
            }
        }
        let mut reached: BTreeSet<&str> = BTreeSet::new();
        let mut next = vec![self.start.as_str()];
        let edges = self.edges();
        while let Some(line) = next.pop() {
            if reached.insert(line) {
                next.extend(edges.iter().filter(|(f, _)| *f == line).map(|(_, t)| *t));
            }
        }
        for name in self.lines.keys() {
            if !reached.contains(name.as_str()) {
                out.push(format!(
                    "line `{name}` is never reached from `{}`",
                    self.start
                ));
            }
        }
        for (name, line) in &self.lines {
            if !line.next.is_empty() && !line.choices.is_empty() {
                out.push(format!(
                    "line `{name}` has both a next and choices: the choices win"
                ));
            }
        }
        out
    }

    /// Every `@key` its texts ask `strings/` for.
    pub fn keys(&self) -> Vec<String> {
        self.lines
            .values()
            .flat_map(|l| std::iter::once(&l.text).chain(l.choices.iter().map(|c| &c.text)))
            .filter_map(|t| t.strip_prefix('@').map(str::to_string))
            .collect()
    }

    /// Every flag it reads or sets.
    pub fn flags(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for line in self.lines.values() {
            out.extend(line.set.iter().cloned());
            for choice in &line.choices {
                out.extend(choice.set.iter().cloned());
                for f in &choice.when {
                    match f {
                        Flag::Is(n) | Flag::Not(n) => out.insert(n.clone()),
                    };
                }
            }
        }
        out
    }
}

/// A conversation being had: which line it is at, and what the game hears.
#[derive(Debug, Clone)]
pub struct Conversation {
    pub dialogue: Dialogue,
    at: Option<String>,
    events: Vec<String>,
}

impl Conversation {
    /// Begin at the start; its line's flags and event happen at once.
    pub fn begin(dialogue: Dialogue, flags: &mut BTreeSet<String>) -> Self {
        let start = dialogue.start.clone();
        let mut c = Self {
            dialogue,
            at: None,
            events: Vec::new(),
        };
        c.enter(&start, flags);
        c
    }

    fn enter(&mut self, name: &str, flags: &mut BTreeSet<String>) {
        let Some(line) = self.dialogue.lines.get(name) else {
            self.at = None;
            return;
        };
        flags.extend(line.set.iter().cloned());
        if !line.event.is_empty() {
            self.events.push(line.event.clone());
        }
        self.at = Some(name.to_string());
    }

    /// The line being said; `None` once it is over.
    pub fn line(&self) -> Option<&Line> {
        self.at.as_ref().and_then(|n| self.dialogue.lines.get(n))
    }

    /// Its name.
    pub fn at(&self) -> Option<&str> {
        self.at.as_deref()
    }

    /// The answers on offer now, by their index in the line's list.
    pub fn choices<'a>(
        &'a self,
        flags: &'a BTreeSet<String>,
    ) -> impl Iterator<Item = (usize, &'a Choice)> {
        self.line()
            .into_iter()
            .flat_map(|l| l.choices.iter().enumerate())
            .filter(move |(_, c)| c.when.iter().all(|f| f.holds(flags)))
    }

    /// On, to the next line — when the line has no choices.
    pub fn next(&mut self, flags: &mut BTreeSet<String>) {
        let Some(line) = self.line().cloned() else {
            return;
        };
        if !line.choices.is_empty() {
            return;
        }
        if line.next.is_empty() {
            self.at = None;
        } else {
            self.enter(&line.next, flags);
        }
    }

    /// Answer with choice `index` (its place in the line's list), if it is
    /// on offer.
    pub fn choose(&mut self, index: usize, flags: &mut BTreeSet<String>) -> bool {
        let Some(choice) = self.line().and_then(|l| l.choices.get(index)).cloned() else {
            return false;
        };
        if !choice.when.iter().all(|f| f.holds(flags)) {
            return false;
        }
        flags.extend(choice.set.iter().cloned());
        if !choice.event.is_empty() {
            self.events.push(choice.event.clone());
        }
        self.enter(&choice.to, flags);
        true
    }

    /// Whether it is over.
    pub fn over(&self) -> bool {
        self.at.is_none()
    }

    /// The events fired since last asked, in order.
    pub fn events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPTAIN: &str = r#"(
        start: "hello",
        lines: {
            "hello": (speaker: "Captain", text: "@captain.hello", next: "ask"),
            "ask": (speaker: "Captain", text: "@captain.ask", choices: [
                (text: "@yes", to: "thanks", set: ["agreed"], event: "give_seeds"),
                (text: "@no", to: "bye", when: [Not("broke")]),
            ]),
            "thanks": (speaker: "Captain", text: "@captain.thanks"),
            "bye": (speaker: "Captain", text: "@captain.bye"),
        },
    )"#;

    #[test]
    fn a_conversation_goes_where_the_answers_take_it() {
        let dialogue: Dialogue = ron::from_str(CAPTAIN).unwrap();
        assert!(dialogue.problems().is_empty(), "{:?}", dialogue.problems());
        let mut flags = BTreeSet::new();
        let mut c = Conversation::begin(dialogue.clone(), &mut flags);
        assert_eq!(c.line().unwrap().text, "@captain.hello");
        c.next(&mut flags);
        assert_eq!(c.at(), Some("ask"));
        assert_eq!(c.choices(&flags).count(), 2);
        assert!(c.choose(0, &mut flags));
        assert_eq!(c.at(), Some("thanks"));
        assert!(flags.contains("agreed"));
        assert_eq!(c.events(), ["give_seeds"]);
        c.next(&mut flags);
        assert!(c.over());
        // Broke: "no" is not on offer.
        let mut broke: BTreeSet<String> = ["broke".to_string()].into();
        let mut c = Conversation::begin(dialogue, &mut broke);
        c.next(&mut broke);
        assert_eq!(c.choices(&broke).count(), 1);
        assert!(!c.choose(1, &mut broke), "not on offer");
    }

    #[test]
    fn a_dialogue_says_what_does_not_join_up() {
        let dialogue: Dialogue = ron::from_str(
            r#"(start: "hello", lines: {
                "hello": (text: "hi", next: "ask"),
                "lost": (text: "?"),
            })"#,
        )
        .unwrap();
        let found = dialogue.problems();
        assert!(
            found.iter().any(|p| p.contains("leads to `ask`")),
            "{found:?}"
        );
        assert!(
            found.iter().any(|p| p.contains("`lost` is never reached")),
            "{found:?}"
        );
        let keys: Dialogue = ron::from_str(CAPTAIN).unwrap();
        assert!(keys.keys().contains(&"captain.hello".to_string()));
        assert_eq!(
            keys.flags().into_iter().collect::<Vec<_>>(),
            ["agreed", "broke"]
        );
    }
}
