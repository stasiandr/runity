//! Dialogue: who says what, and what the player can answer — a graph of
//! lines in `dialogues/<name>.ron`, as a graph of states is an animator
//! (DNA, «Графовые редакторы»: the same widget, the same kind of checks,
//! editable as text by a person or an agent). Card: docs/dialogue.md.
//!
//! ```ron
//! (
//!     start: "hello",
//!     lines: {
//!         "hello": (speaker: "@captain", text: "@captain.hello", next: "again"),
//!         "again": (speaker: "@captain", text: "@captain.again", when: [Is("met")], else: "first", next: "ask"),
//!         "first": (speaker: "@captain", text: "@captain.first", set: ["met"], next: "ask"),
//!         "ask": (speaker: "@captain", text: "@captain.ask", choices: [
//!             (text: "@yes", to: "thanks", set: ["agreed"], add: {"trust": 1}, event: "give_seeds"),
//!             (text: "@no", to: "bye", when: [Not("broke")]),
//!             (text: "@rumour", to: "rumour", once: true),
//!             (text: "@pay", to: "thanks", when: [Var("coins", Ge, 3)], add: {"coins": -3}),
//!         ]),
//!         ...
//!     },
//! )
//! ```
//!
//! Texts starting `@` are keys in `strings/` (as screens' are); a speaker
//! may be one too. The game keeps a [`State`] — flags for what happened,
//! whole numbers for what is counted, the once-only answers taken — and
//! hears the events a choice or a line fires. A line whose `when` does not
//! hold is passed over, on to its `else` (or its `next`). A line with
//! neither `next` nor choices ends the conversation.
//!
//! A line's stable name is `<dialogue>/<line>` (`captain/ask`): what a
//! voice recording and a translator's sheet are named by
//! ([`Dialogue::said`]).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The folder dialogues are in.
pub const DIR: &str = "dialogues";

/// How a number is compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Op {
    fn holds(self, a: i64, b: i64) -> bool {
        match self {
            Op::Eq => a == b,
            Op::Ne => a != b,
            Op::Lt => a < b,
            Op::Le => a <= b,
            Op::Gt => a > b,
            Op::Ge => a >= b,
        }
    }

    pub fn sign(self) -> &'static str {
        match self {
            Op::Eq => "=",
            Op::Ne => "≠",
            Op::Lt => "<",
            Op::Le => "≤",
            Op::Gt => ">",
            Op::Ge => "≥",
        }
    }
}

/// A condition on what the game remembers: a flag, or a number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Condition {
    Is(String),
    Not(String),
    /// A number compared with a constant; a number never set is 0.
    Var(String, Op, i64),
}

impl Condition {
    pub fn holds(&self, state: &State) -> bool {
        match self {
            Condition::Is(f) => state.flags.contains(f),
            Condition::Not(f) => !state.flags.contains(f),
            Condition::Var(v, op, n) => op.holds(state.var(v), *n),
        }
    }

    /// The flag or number it reads.
    pub fn name(&self) -> &str {
        match self {
            Condition::Is(n) | Condition::Not(n) | Condition::Var(n, _, _) => n,
        }
    }
}

/// `met, not broke, coins ≥ 3`: conditions in words.
pub fn describe(when: &[Condition]) -> String {
    let words: Vec<String> = when
        .iter()
        .map(|c| match c {
            Condition::Is(n) => n.clone(),
            Condition::Not(n) => format!("not {n}"),
            Condition::Var(n, op, v) => format!("{n} {} {v}", op.sign()),
        })
        .collect();
    words.join(", ")
}

/// What the game remembers for its dialogues: flags, numbers, and the
/// once-only answers already taken. Serialisable, for a save.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub flags: BTreeSet<String>,
    #[serde(default)]
    pub vars: BTreeMap<String, i64>,
    /// Once-only answers taken: `<dialogue>/<line>/<answer's text>`.
    #[serde(default)]
    pub chosen: BTreeSet<String>,
}

impl State {
    pub fn var(&self, name: &str) -> i64 {
        self.vars.get(name).copied().unwrap_or(0)
    }

    fn apply(&mut self, set: &[String], add: &BTreeMap<String, i64>, put: &BTreeMap<String, i64>) {
        self.flags.extend(set.iter().cloned());
        for (name, n) in add {
            *self.vars.entry(name.clone()).or_insert(0) += n;
        }
        for (name, n) in put {
            self.vars.insert(name.clone(), *n);
        }
    }
}

/// One line someone says.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Line {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub speaker: String,
    pub text: String,
    /// Said only when these hold; otherwise passed over, on to `else`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<Condition>,
    /// Where a line whose `when` does not hold goes instead; its `next`
    /// when empty.
    #[serde(default, rename = "else", skip_serializing_if = "String::is_empty")]
    pub otherwise: String,
    /// The line after it, when there is no choosing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub next: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    /// Flags set when it is said.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set: Vec<String>,
    /// Numbers added to when it is said (a negative takes away).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub add: BTreeMap<String, i64>,
    /// Numbers set when it is said.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub put: BTreeMap<String, i64>,
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
    pub when: Vec<Condition>,
    /// Offered until taken once (Ink's once-only choice), remembered in
    /// the [`State`] by its text.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub once: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub add: BTreeMap<String, i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub put: BTreeMap<String, i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub event: String,
}

/// A whole conversation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Dialogue {
    /// Its name: the file's path in `dialogues/` without `.ron`. Not in
    /// the file; [`Dialogue::load`] gives it.
    #[serde(skip)]
    pub name: String,
    pub start: String,
    pub lines: BTreeMap<String, Line>,
}

/// One thing said, as a recording or a translation names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    /// `<dialogue>/<line>` for a line, `<dialogue>/<line>/<n>` for its
    /// n-th answer, from 1.
    pub id: String,
    pub speaker: String,
    pub text: String,
}

impl Dialogue {
    /// Read a dialogue; its name is the file's.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = scrap_core::files::read_to_string(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut dialogue: Self =
            ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        dialogue.name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(dialogue)
    }

    /// Every way on from a line: its `next`, its `else` and its choices'
    /// `to`.
    pub fn edges(&self) -> Vec<(&str, &str)> {
        let mut out = Vec::new();
        for (name, line) in &self.lines {
            if !line.next.is_empty() {
                out.push((name.as_str(), line.next.as_str()));
            }
            if !line.otherwise.is_empty() {
                out.push((name.as_str(), line.otherwise.as_str()));
            }
            for choice in &line.choices {
                out.push((name.as_str(), choice.to.as_str()));
            }
        }
        out
    }

    /// What will not work: a line that names one that is not there, a line
    /// nothing leads to, a line with both a next and choices, an `else` on
    /// a line always said.
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
            if !line.otherwise.is_empty() && line.when.is_empty() {
                out.push(format!(
                    "line `{name}` has an else and no when: it is always said, the else never taken"
                ));
            }
            let mut texts = BTreeSet::new();
            for choice in line.choices.iter().filter(|c| c.once) {
                if !texts.insert(&choice.text) {
                    out.push(format!(
                        "line `{name}` has two once-only answers “{}”: taking one takes both",
                        choice.text
                    ));
                }
            }
        }
        out
    }

    /// Every `@key` its texts and speakers ask `strings/` for.
    pub fn keys(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .lines
            .values()
            .flat_map(|l| {
                [&l.speaker, &l.text]
                    .into_iter()
                    .chain(l.choices.iter().map(|c| &c.text))
            })
            .filter_map(|t| t.strip_prefix('@').map(str::to_string))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Every flag and number it reads or sets.
    pub fn flags(&self) -> BTreeSet<String> {
        self.reads().into_iter().chain(self.writes()).collect()
    }

    /// The flags and numbers its conditions read.
    pub fn reads(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for line in self.lines.values() {
            let whens = std::iter::once(&line.when).chain(line.choices.iter().map(|c| &c.when));
            for when in whens {
                out.extend(when.iter().map(|c| c.name().to_string()));
            }
        }
        out
    }

    /// The flags and numbers its lines and answers set or add to.
    pub fn writes(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for line in self.lines.values() {
            out.extend(
                line.set
                    .iter()
                    .chain(line.add.keys())
                    .chain(line.put.keys())
                    .cloned(),
            );
            for c in &line.choices {
                out.extend(
                    c.set
                        .iter()
                        .chain(c.add.keys())
                        .chain(c.put.keys())
                        .cloned(),
                );
            }
        }
        out
    }

    /// The events it tells the game.
    pub fn events(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for line in self.lines.values() {
            out.insert(line.event.clone());
            out.extend(line.choices.iter().map(|c| c.event.clone()));
        }
        out.remove("");
        out
    }

    /// Everything said in it, with the stable name each is recorded and
    /// translated by: every line from the start on (then the rest by
    /// name), each followed by its answers.
    pub fn said(&self) -> Vec<Said> {
        let edges = self.edges();
        let mut placed: Vec<&str> = Vec::new();
        let mut queue = std::collections::VecDeque::from([self.start.as_str()]);
        while let Some(name) = queue.pop_front() {
            if self.lines.contains_key(name) && !placed.contains(&name) {
                placed.push(name);
                queue.extend(edges.iter().filter(|(f, _)| *f == name).map(|(_, t)| *t));
            }
        }
        for name in self.lines.keys() {
            if !placed.contains(&name.as_str()) {
                placed.push(name);
            }
        }
        let mut out = Vec::new();
        for name in placed {
            let line = &self.lines[name];
            out.push(Said {
                id: format!("{}/{name}", self.name),
                speaker: line.speaker.clone(),
                text: line.text.clone(),
            });
            for (i, choice) in line.choices.iter().enumerate() {
                out.push(Said {
                    id: format!("{}/{name}/{}", self.name, i + 1),
                    speaker: String::new(),
                    text: choice.text.clone(),
                });
            }
        }
        out
    }
}

/// Lines named twice in a dialogue's text: RON reads a map's key twice
/// without a word, and the first line is lost.
pub fn named_twice(text: &str) -> Vec<String> {
    use scrap_core::ron_edit as patch;
    let Some(open) = patch::value_start(text, "lines") else {
        return Vec::new();
    };
    let Some(found) = patch::items(text, open) else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for item in &found.items {
        let key = text[item.clone()].split(':').next().unwrap_or("").trim();
        if let Ok(key) = ron::from_str::<String>(key) {
            if !seen.insert(key.clone()) {
                out.push(format!(
                    "line `{key}` is written twice: only the last is read"
                ));
            }
        }
    }
    out
}

/// A conversation being had: which line it is at, and what the game hears.
#[derive(Debug, Clone)]
pub struct Conversation {
    pub dialogue: Dialogue,
    at: Option<String>,
    events: Vec<String>,
    /// Every line said, in order: the trail an editor shows.
    trail: Vec<String>,
}

impl Conversation {
    /// Begin at the start; its line's flags and event happen at once.
    pub fn begin(dialogue: Dialogue, state: &mut State) -> Self {
        let start = dialogue.start.clone();
        let mut c = Self {
            dialogue,
            at: None,
            events: Vec::new(),
            trail: Vec::new(),
        };
        c.enter(&start, state);
        c
    }

    /// Begin at `line` instead of the start: to play from the middle.
    pub fn begin_at(dialogue: Dialogue, line: &str, state: &mut State) -> Self {
        let mut c = Self {
            dialogue,
            at: None,
            events: Vec::new(),
            trail: Vec::new(),
        };
        c.enter(line, state);
        c
    }

    /// Go to `name`, passing over lines whose `when` does not hold; a
    /// circle of such lines ends it.
    fn enter(&mut self, name: &str, state: &mut State) {
        let mut name = name.to_string();
        for _ in 0..=self.dialogue.lines.len() {
            let Some(line) = self.dialogue.lines.get(&name) else {
                break;
            };
            if line.when.iter().all(|c| c.holds(state)) {
                state.apply(&line.set, &line.add, &line.put);
                if !line.event.is_empty() {
                    self.events.push(line.event.clone());
                }
                self.trail.push(name.clone());
                self.at = Some(name);
                return;
            }
            let on = if line.otherwise.is_empty() {
                &line.next
            } else {
                &line.otherwise
            };
            name = on.clone();
        }
        self.at = None;
    }

    /// The line being said; `None` once it is over.
    pub fn line(&self) -> Option<&Line> {
        self.at.as_ref().and_then(|n| self.dialogue.lines.get(n))
    }

    /// Its name.
    pub fn at(&self) -> Option<&str> {
        self.at.as_deref()
    }

    /// The lines said so far, in order.
    pub fn trail(&self) -> &[String] {
        &self.trail
    }

    fn once_key(&self, choice: &Choice) -> String {
        format!(
            "{}/{}/{}",
            self.dialogue.name,
            self.at.as_deref().unwrap_or(""),
            choice.text
        )
    }

    fn offered(&self, choice: &Choice, state: &State) -> bool {
        choice.when.iter().all(|f| f.holds(state))
            && !(choice.once && state.chosen.contains(&self.once_key(choice)))
    }

    /// The answers on offer now, by their index in the line's list.
    pub fn choices<'a>(&'a self, state: &'a State) -> impl Iterator<Item = (usize, &'a Choice)> {
        self.line()
            .into_iter()
            .flat_map(|l| l.choices.iter().enumerate())
            .filter(move |(_, c)| self.offered(c, state))
    }

    /// On, to the next line — when the line has no choices.
    pub fn next(&mut self, state: &mut State) {
        let Some(line) = self.line().cloned() else {
            return;
        };
        if !line.choices.is_empty() {
            return;
        }
        if line.next.is_empty() {
            self.at = None;
        } else {
            self.enter(&line.next, state);
        }
    }

    /// Answer with choice `index` (its place in the line's list), if it is
    /// on offer.
    pub fn choose(&mut self, index: usize, state: &mut State) -> bool {
        let Some(choice) = self.line().and_then(|l| l.choices.get(index)).cloned() else {
            return false;
        };
        if !self.offered(&choice, state) {
            return false;
        }
        if choice.once {
            state.chosen.insert(self.once_key(&choice));
        }
        state.apply(&choice.set, &choice.add, &choice.put);
        if !choice.event.is_empty() {
            self.events.push(choice.event.clone());
        }
        self.enter(&choice.to, state);
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

/// A dialogue's cases: what it should do, played without the game — as
/// `dialogues/<name>.cases.ron` beside it, which `scrap check` plays (as
/// an animator's are).
///
/// ```ron
/// (cases: [
///     (name: "agrees", steps: [At("hello"), Next("ask"), Choose("@yes", "thanks"), Told("give_seeds"), Holds(Is("agreed")), Next("")]),
///     (name: "broke", flags: ["broke"], steps: [Next("ask"), Offers(["@yes"])]),
/// ])
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Cases {
    pub cases: Vec<Case>,
}

/// One playthrough from the start, with what the game remembers before it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub name: String,
    #[serde(default)]
    pub flags: BTreeSet<String>,
    #[serde(default)]
    pub vars: BTreeMap<String, i64>,
    pub steps: Vec<Step>,
}

/// A move and what should follow, or a look at where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    /// It is at this line now.
    At(String),
    /// On from a line without choices; then at this line (`""`: over).
    Next(String),
    /// The answer with this text; then at this line (`""`: over).
    Choose(String, String),
    /// The answers on offer now, by text, in order.
    Offers(Vec<String>),
    /// A condition holds now.
    Holds(Condition),
    /// This event was told since the step before.
    Told(String),
}

impl Cases {
    /// Play every case on `dialogue`: what went otherwise, in words.
    pub fn run(&self, dialogue: &Dialogue) -> Vec<String> {
        let mut out = Vec::new();
        for case in &self.cases {
            let mut state = State {
                flags: case.flags.clone(),
                vars: case.vars.clone(),
                chosen: BTreeSet::new(),
            };
            let mut talk = Conversation::begin(dialogue.clone(), &mut state);
            let mut told = talk.events();
            for (i, step) in case.steps.iter().enumerate() {
                let wrong = step_wrong(&mut talk, &mut state, &mut told, step);
                if let Some(wrong) = wrong {
                    out.push(format!("case `{}`, step {}: {wrong}", case.name, i + 1));
                    break;
                }
            }
        }
        out
    }
}

fn step_wrong(
    talk: &mut Conversation,
    state: &mut State,
    told: &mut Vec<String>,
    step: &Step,
) -> Option<String> {
    let here = |talk: &Conversation| match talk.at() {
        Some(at) => format!("at `{at}`"),
        None => "over".to_string(),
    };
    let arrived = |talk: &Conversation, expect: &str| {
        let now = talk.at().unwrap_or("");
        (now != expect).then(|| {
            let wanted = if expect.is_empty() {
                "over".to_string()
            } else {
                format!("at `{expect}`")
            };
            format!("{}, expected {wanted}", here(talk))
        })
    };
    match step {
        Step::At(line) => arrived(talk, line),
        Step::Next(line) => {
            if talk.line().is_some_and(|l| !l.choices.is_empty()) {
                return Some(format!("{} waits for an answer", here(talk)));
            }
            told.clear();
            talk.next(state);
            told.extend(talk.events());
            arrived(talk, line)
        }
        Step::Choose(text, line) => {
            let Some(current) = talk.line() else {
                return Some("over, nothing to answer".into());
            };
            let Some(index) = current.choices.iter().position(|c| c.text == *text) else {
                let near =
                    crate::spelling::closest(text, current.choices.iter().map(|c| c.text.as_str()))
                        .map(|n| format!(" — did you mean “{n}”?"))
                        .unwrap_or_default();
                return Some(format!("{} there is no answer “{text}”{near}", here(talk)));
            };
            told.clear();
            if !talk.choose(index, state) {
                return Some(format!("{} “{text}” is not on offer", here(talk)));
            }
            told.extend(talk.events());
            arrived(talk, line)
        }
        Step::Offers(texts) => {
            let offered: Vec<&str> = talk.choices(state).map(|(_, c)| c.text.as_str()).collect();
            (offered != texts.iter().map(String::as_str).collect::<Vec<_>>()).then(|| {
                format!(
                    "{} offers [{}], expected [{}]",
                    here(talk),
                    offered.join(", "),
                    texts.join(", ")
                )
            })
        }
        Step::Holds(condition) => (!condition.holds(state)).then(|| {
            format!(
                "{} `{}` does not hold (flags: {}; numbers: {})",
                here(talk),
                describe(std::slice::from_ref(condition)),
                state.flags.iter().cloned().collect::<Vec<_>>().join(", "),
                state
                    .vars
                    .iter()
                    .map(|(k, v)| format!("{k} = {v}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }),
        Step::Told(event) => (!told.contains(event)).then(|| {
            format!(
                "`{event}` was not told (told: {})",
                if told.is_empty() {
                    "nothing".to_string()
                } else {
                    told.join(", ")
                }
            )
        }),
    }
}

/// One difference between two versions of a dialogue: what an agent's
/// edit did, for a person to look over and take back piece by piece.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    LineAdded(String),
    LineRemoved(String, Line),
    /// The line is there in both, and says or leads otherwise.
    LineChanged(String, Line),
    /// The start was the first.
    StartMoved(String),
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Change::LineAdded(n) => write!(f, "+ line {n}"),
            Change::LineRemoved(n, _) => write!(f, "− line {n}"),
            Change::LineChanged(n, _) => write!(f, "~ line {n}"),
            Change::StartMoved(was) => write!(f, "~ start (was {was})"),
        }
    }
}

/// What changed from `before` to `after`, lines first.
pub fn diff(before: &Dialogue, after: &Dialogue) -> Vec<Change> {
    let mut out = Vec::new();
    for (name, line) in &after.lines {
        match before.lines.get(name) {
            None => out.push(Change::LineAdded(name.clone())),
            Some(was) if was != line => out.push(Change::LineChanged(name.clone(), was.clone())),
            _ => {}
        }
    }
    for (name, line) in &before.lines {
        if !after.lines.contains_key(name) {
            out.push(Change::LineRemoved(name.clone(), line.clone()));
        }
    }
    if before.start != after.start {
        out.push(Change::StartMoved(before.start.clone()));
    }
    out
}

impl Change {
    /// Take this change back in `dialogue`, as it was before.
    pub fn revert(&self, dialogue: &mut Dialogue) {
        match self {
            Change::LineAdded(n) => {
                dialogue.lines.remove(n);
            }
            Change::LineRemoved(n, line) | Change::LineChanged(n, line) => {
                dialogue.lines.insert(n.clone(), line.clone());
            }
            Change::StartMoved(was) => dialogue.start = was.clone(),
        }
    }

    /// The line this change is about, if it is one: what the canvas marks.
    pub fn line(&self) -> Option<&str> {
        match self {
            Change::LineAdded(n) | Change::LineRemoved(n, _) | Change::LineChanged(n, _) => Some(n),
            Change::StartMoved(_) => None,
        }
    }
}

/// Rename line `from` to `to`, and everything in the dialogue that names
/// it: the start, `next`, `else`, answers' `to`.
pub fn rename(dialogue: &mut Dialogue, from: &str, to: &str) -> Result<(), String> {
    if !dialogue.lines.contains_key(from) {
        let near = crate::spelling::closest(from, dialogue.lines.keys().map(String::as_str))
            .map(|n| format!(" — did you mean `{n}`?"))
            .unwrap_or_default();
        return Err(format!("`{from}` is not a line{near}"));
    }
    if to.is_empty() {
        return Err("a line needs a name".into());
    }
    if from != to && dialogue.lines.contains_key(to) {
        return Err(format!("there is a line `{to}` already"));
    }
    let line = dialogue.lines.remove(from).expect("checked");
    dialogue.lines.insert(to.to_string(), line);
    if dialogue.start == from {
        dialogue.start = to.to_string();
    }
    for line in dialogue.lines.values_mut() {
        for end in [&mut line.next, &mut line.otherwise]
            .into_iter()
            .chain(line.choices.iter_mut().map(|c| &mut c.to))
        {
            if end == from {
                *end = to.to_string();
            }
        }
    }
    Ok(())
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

    /// Conditions on lines, numbers, and an answer offered once.
    const MARKET: &str = r#"(
        start: "hello",
        lines: {
            "hello": (speaker: "@trader", text: "@again", when: [Is("met")], else: "first", next: "ask"),
            "first": (speaker: "@trader", text: "@first", set: ["met"], next: "ask"),
            "ask": (speaker: "@trader", text: "@ask", choices: [
                (text: "@rumour", to: "rumour", once: true),
                (text: "@buy", to: "sold", when: [Var("coins", Ge, 3)], add: {"coins": -3}, event: "sold"),
                (text: "@bye", to: "bye"),
            ]),
            "rumour": (speaker: "@trader", text: "@rumour", add: {"trust": 1}, next: "ask"),
            "sold": (speaker: "@trader", text: "@sold", put: {"stock": 0}),
            "bye": (speaker: "@trader", text: "@bye"),
        },
    )"#;

    #[test]
    fn a_conversation_goes_where_the_answers_take_it() {
        let dialogue: Dialogue = ron::from_str(CAPTAIN).unwrap();
        assert!(dialogue.problems().is_empty(), "{:?}", dialogue.problems());
        let mut state = State::default();
        let mut c = Conversation::begin(dialogue.clone(), &mut state);
        assert_eq!(c.line().unwrap().text, "@captain.hello");
        c.next(&mut state);
        assert_eq!(c.at(), Some("ask"));
        assert_eq!(c.choices(&state).count(), 2);
        assert!(c.choose(0, &mut state));
        assert_eq!(c.at(), Some("thanks"));
        assert!(state.flags.contains("agreed"));
        assert_eq!(c.events(), ["give_seeds"]);
        c.next(&mut state);
        assert!(c.over());
        // Broke: "no" is not on offer.
        let mut broke = State {
            flags: ["broke".to_string()].into(),
            ..State::default()
        };
        let mut c = Conversation::begin(dialogue, &mut broke);
        c.next(&mut broke);
        assert_eq!(c.choices(&broke).count(), 1);
        assert!(!c.choose(1, &mut broke), "not on offer");
    }

    #[test]
    fn a_line_is_passed_over_numbers_count_and_an_answer_goes_once_taken() {
        let mut market: Dialogue = ron::from_str(MARKET).unwrap();
        market.name = "market".into();
        assert!(market.problems().is_empty(), "{:?}", market.problems());
        let mut state = State::default();
        state.vars.insert("coins".into(), 4);
        let mut c = Conversation::begin(market.clone(), &mut state);
        assert_eq!(c.at(), Some("first"), "not met yet: the else");
        c.next(&mut state);
        let offered: Vec<&str> = c.choices(&state).map(|(_, c)| c.text.as_str()).collect();
        assert_eq!(offered, ["@rumour", "@buy", "@bye"]);
        assert!(c.choose(0, &mut state));
        assert_eq!(state.var("trust"), 1);
        c.next(&mut state);
        let offered: Vec<&str> = c.choices(&state).map(|(_, c)| c.text.as_str()).collect();
        assert_eq!(offered, ["@buy", "@bye"], "the rumour is told once");
        assert!(state.chosen.contains("market/ask/@rumour"));
        assert!(c.choose(1, &mut state));
        assert_eq!(state.var("coins"), 1);
        assert_eq!(state.var("stock"), 0);
        assert!(state.vars.contains_key("stock"));
        // Met: the first line is passed over.
        let c = Conversation::begin(market, &mut state);
        assert_eq!(c.at(), Some("hello"));
        assert_eq!(c.trail(), ["hello"]);
    }

    #[test]
    fn a_circle_of_lines_passed_over_ends_the_conversation() {
        let d: Dialogue = ron::from_str(
            r#"(start: "a", lines: {
                "a": (text: "a", when: [Is("x")], else: "b"),
                "b": (text: "b", when: [Is("x")], else: "a"),
            })"#,
        )
        .unwrap();
        let c = Conversation::begin(d, &mut State::default());
        assert!(c.over());
    }

    #[test]
    fn a_dialogue_says_what_does_not_join_up() {
        let dialogue: Dialogue = ron::from_str(
            r#"(start: "hello", lines: {
                "hello": (text: "hi", next: "ask", else: "bye"),
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
        assert!(
            found.iter().any(|p| p.contains("else never taken")),
            "{found:?}"
        );
        let keys: Dialogue = ron::from_str(CAPTAIN).unwrap();
        assert!(keys.keys().contains(&"captain.hello".to_string()));
        assert_eq!(
            keys.flags().into_iter().collect::<Vec<_>>(),
            ["agreed", "broke"]
        );
        let market: Dialogue = ron::from_str(MARKET).unwrap();
        assert_eq!(
            market.reads().into_iter().collect::<Vec<_>>(),
            ["coins", "met"]
        );
        assert_eq!(
            market.writes().into_iter().collect::<Vec<_>>(),
            ["coins", "met", "stock", "trust"]
        );
        assert!(
            market.keys().contains(&"trader".to_string()),
            "the speaker's key"
        );
        assert_eq!(
            named_twice("(start: \"a\", lines: {\"a\": (text: \"1\"), \"a\": (text: \"2\")})"),
            ["line `a` is written twice: only the last is read"]
        );
    }

    #[test]
    fn cases_play_the_dialogue_and_say_where_it_went_otherwise() {
        let mut market: Dialogue = ron::from_str(MARKET).unwrap();
        market.name = "market".into();
        let cases: Cases = ron::from_str(
            r#"(cases: [
                (name: "buys", vars: {"coins": 3}, steps: [
                    At("first"), Next("ask"), Choose("@rumour", "rumour"), Next("ask"),
                    Offers(["@buy", "@bye"]), Choose("@buy", "sold"), Told("sold"),
                    Holds(Var("coins", Eq, 0)), Holds(Is("met")), Next(""),
                ]),
                (name: "poor", flags: ["met"], steps: [At("hello"), Next("ask"), Offers(["@rumour", "@bye"])]),
            ])"#,
        )
        .unwrap();
        assert_eq!(cases.run(&market), Vec::<String>::new());
        let wrong: Cases = ron::from_str(
            r#"(cases: [
                (name: "a", steps: [Next("ask"), Choose("@buy", "sold")]),
                (name: "b", steps: [Next("bye")]),
                (name: "c", steps: [Next("ask"), Choose("@rumor", "rumour")]),
                (name: "d", steps: [Next("ask"), Next("bye")]),
            ])"#,
        )
        .unwrap();
        let found = wrong.run(&market);
        assert_eq!(
            found,
            [
                "case `a`, step 2: at `ask` “@buy” is not on offer",
                "case `b`, step 1: at `ask`, expected at `bye`",
                "case `c`, step 2: at `ask` there is no answer “@rumor” — did you mean “@rumour”?",
                "case `d`, step 2: at `ask` waits for an answer",
            ]
        );
    }

    #[test]
    fn what_is_said_is_named_by_dialogue_and_line_in_the_order_it_is_said() {
        let mut market: Dialogue = ron::from_str(MARKET).unwrap();
        market.name = "market".into();
        let ids: Vec<String> = market.said().into_iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            [
                "market/hello",
                "market/ask",
                "market/ask/1",
                "market/ask/2",
                "market/ask/3",
                "market/first",
                "market/rumour",
                "market/sold",
                "market/bye",
            ]
        );
    }

    #[test]
    fn a_diff_is_taken_back_a_change_at_a_time_and_a_rename_follows_the_line() {
        let before: Dialogue = ron::from_str(CAPTAIN).unwrap();
        let mut after = before.clone();
        after.lines.get_mut("thanks").unwrap().text = "@captain.thanks2".into();
        after.lines.insert(
            "later".into(),
            Line {
                text: "@later".into(),
                ..Line::default()
            },
        );
        after.lines.remove("bye");
        let changes = diff(&before, &after);
        let words: Vec<String> = changes.iter().map(ToString::to_string).collect();
        assert_eq!(words, ["+ line later", "~ line thanks", "− line bye"]);
        for change in &changes {
            change.revert(&mut after);
        }
        assert_eq!(after, before);

        rename(&mut after, "ask", "question").unwrap();
        assert_eq!(after.lines["hello"].next, "question");
        assert!(rename(&mut after, "questoin", "x")
            .unwrap_err()
            .contains("did you mean"));
    }
}
