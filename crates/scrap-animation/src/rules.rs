//! The animator as rules, not a graph (docs/animator.md): what the body
//! does now is the first **mode** whose condition holds, top to bottom;
//! how it moves in that mode is a **motion** (a clip, a 1D or 2D blend);
//! one-off moves — a strike, a flinch, a fidget — are **actions** the game
//! plays into a **slot** (a layer with a mask) over the mode.
//!
//! ```text
//! (
//!     params: { "speed": Float, "grounded": Bool, "dead": Bool },
//!     modes: [
//!         (name: "dead", when: "dead", play: "death", hold: true),
//!         (name: "fall", when: "!grounded", play: "airborne", enter_after: 0.1),
//!         (name: "move", when: "true", play: "locomotion"),
//!     ],
//!     blends: (default: 0.2, pairs: [(from: "fall", to: "move", via: "land", time: 0.1)]),
//!     motions: {
//!         "locomotion": (blend_by: "speed", blend: [(0.0, "idle"), (4.0, "run")]),
//!     },
//!     actions: {
//!         "attack1": (clip: "slash_a", events: [(0.35, "hit")],
//!                     chain: [(on: "attack", window: (0.2, 0.9), to: "attack2")],
//!                     interrupt: [(when: "speed > 0.1", after: 0.6)], end: 0.9),
//!     },
//!     slots: { "full": (mask: []) },
//! )
//! ```
//!
//! A mode is a function of the parameters now, not of the way here: N
//! rules instead of N² arrows, priority read top to bottom. The game says
//! what to do for one-offs (`play("attack1")`) and gets an [`ActionId`] to
//! ask about; the body's modes it leaves to the rules.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::animator::{Animator, LayerBlend};
use crate::animgraph::{mix_1d, mix_2d, State};

/// A parameter's kind, for `check` to name a wrong one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Float,
    Bool,
    Int,
    /// Pulled for one step: `attack`, true the step it is pulled.
    Trigger,
}

/// A rule: this mode, when this holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mode {
    pub name: String,
    /// An expression over the parameters: `!grounded && speed > 0.1`.
    pub when: String,
    /// A motion's name, or a clip's.
    pub play: String,
    /// Seconds the condition has to hold before the mode is taken: a bump
    /// does not start a fall.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub enter_after: f32,
    /// Once in, rules do not take the body out: only the game does
    /// ([`Rules::release`]). Death.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hold: bool,
}

/// How one mode gives way to another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    /// A mode's name or `*`.
    pub from: String,
    pub to: String,
    /// Seconds of crossfade; the default's when unsaid.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub time: Option<f32>,
    /// A clip played once between: the landing between a fall and a run.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub via: Option<String>,
    /// The `via` only when this holds (a hard landing's clip for a fast fall).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub when: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blends {
    #[serde(default = "fifth")]
    pub default: f32,
    #[serde(default)]
    pub pairs: Vec<Pair>,
}

impl Default for Blends {
    fn default() -> Self {
        Self { default: fifth(), pairs: Vec::new() }
    }
}

/// A window of an action in which a pull goes on to the next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chain {
    /// A trigger the game pulls.
    pub on: String,
    /// Where in the action (0..1 of its clip) the pull is taken.
    pub window: (f32, f32),
    pub to: String,
}

/// Leaving an action early: after this far into it, when this holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interrupt {
    pub when: String,
    #[serde(default)]
    pub after: f32,
}

/// A one-off move the game plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    /// The clip; several are one picked at random each time (a fidget).
    #[serde(default)]
    pub clip: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<String>,
    /// A 2D pick of the clip by two parameters: a flinch by the side hit
    /// from — `[(x, y, clip)]`, the nearest point taken.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pick: Vec<(f32, f32, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "plain")]
    pub pick_by: Option<(String, String)>,
    #[serde(default = "full")]
    pub slot: String,
    #[serde(default = "tenth")]
    pub blend_in: f32,
    #[serde(default = "fifth")]
    pub blend_out: f32,
    #[serde(default = "one")]
    pub speed: f32,
    /// Where it ends and gives the body back, 0..1 of the clip.
    #[serde(default = "one")]
    pub end: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<(f32, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chain: Vec<Chain>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interrupt: Vec<Interrupt>,
}

/// A layer actions play in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    /// Joints, each with all under it; none is the whole body.
    #[serde(default)]
    pub mask: Vec<String>,
    #[serde(default)]
    pub blend: LayerBlend,
}

/// A whole animator of rules, as its file is written.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RulesFile {
    #[serde(default)]
    pub params: BTreeMap<String, Kind>,
    pub modes: Vec<Mode>,
    #[serde(default)]
    pub blends: Blends,
    #[serde(default)]
    pub motions: BTreeMap<String, State>,
    #[serde(default)]
    pub actions: BTreeMap<String, Action>,
    #[serde(default)]
    pub slots: BTreeMap<String, Slot>,
}

/// An optional field written as its value alone: `time: 0.1`, not
/// `time: Some(0.1)`.
mod plain {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<T: Serialize, S: Serializer>(v: &Option<T>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => v.serialize(s),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(d: D) -> Result<Option<T>, D::Error> {
        T::deserialize(d).map(Some)
    }
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}
fn is_false(v: &bool) -> bool {
    !*v
}
fn fifth() -> f32 {
    0.2
}
fn tenth() -> f32 {
    0.1
}
fn one() -> f32 {
    1.0
}
fn full() -> String {
    "full".into()
}

impl RulesFile {
    /// Whether a file's text is rules rather than a graph.
    pub fn is_rules(text: &str) -> bool {
        text.contains("modes:")
    }

    /// Every clip it names.
    pub fn clips(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        for m in &self.modes {
            if !self.motions.contains_key(&m.play) {
                out.insert(m.play.clone());
            }
        }
        for s in self.motions.values() {
            if !s.clip.is_empty() {
                out.insert(s.clip.clone());
            }
            out.extend(s.blend.iter().map(|b| b.1.clone()));
            out.extend(s.directional.iter().map(|b| b.2.clone()));
        }
        for a in self.actions.values() {
            if !a.clip.is_empty() {
                out.insert(a.clip.clone());
            }
            out.extend(a.clips.iter().cloned());
            out.extend(a.pick.iter().map(|p| p.2.clone()));
        }
        for p in &self.blends.pairs {
            out.extend(p.via.iter().cloned());
        }
        out
    }

    /// What is wrong in it, in words: an expression that does not read, a
    /// parameter not declared, a mode, action, slot or motion named and
    /// not there, a rule below one that always holds.
    pub fn problems(&self, clips: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        let known = |name: &str| self.params.contains_key(name);
        let check_expr = |text: &str, place: &str, out: &mut Vec<String>| match Expr::parse(text) {
            Ok(e) => {
                for p in e.names() {
                    if !known(&p) {
                        let near = crate::spelling::closest(&p, self.params.keys().map(String::as_str));
                        out.push(match near {
                            Some(n) => format!("{place}: no parameter `{p}` — did you mean `{n}`?"),
                            None => format!("{place}: no parameter `{p}` in params"),
                        });
                    }
                }
            }
            Err(e) => out.push(format!("{place}: `{text}`: {e}")),
        };
        let mut always_before = None;
        for m in &self.modes {
            check_expr(&m.when, &format!("mode `{}`", m.name), &mut out);
            if let Some(first) = &always_before {
                out.push(format!("mode `{}` is never taken: `{first}` above it always holds", m.name));
            }
            if m.when.trim() == "true" {
                always_before = Some(m.name.clone());
            }
            if !self.motions.contains_key(&m.play) && !clips.is_empty() && !clips.contains(&m.play.as_str()) {
                out.push(format!("mode `{}` plays `{}`: no such motion or clip", m.name, m.play));
            }
        }
        let modes: HashSet<&str> = self.modes.iter().map(|m| m.name.as_str()).collect();
        for p in &self.blends.pairs {
            for end in [&p.from, &p.to] {
                if end != "*" && !modes.contains(end.as_str()) {
                    out.push(format!("blends: no mode `{end}`"));
                }
            }
            if let Some(w) = &p.when {
                check_expr(w, &format!("blend {} → {}", p.from, p.to), &mut out);
            }
        }
        for (name, a) in &self.actions {
            if a.slot != "full" && !self.slots.contains_key(&a.slot) {
                out.push(format!("action `{name}`: no slot `{}`", a.slot));
            }
            for c in &a.chain {
                if !self.actions.contains_key(&c.to) {
                    out.push(format!("action `{name}` chains to `{}`: no such action", c.to));
                }
                if !known(&c.on) {
                    out.push(format!("action `{name}` chains on `{}`: not in params", c.on));
                }
            }
            for i in &a.interrupt {
                check_expr(&i.when, &format!("action `{name}`"), &mut out);
            }
        }
        if !clips.is_empty() {
            for c in self.clips() {
                if !clips.contains(&c.as_str()) && !self.motions.contains_key(&c) {
                    out.push(format!("clip `{c}` is not on the model"));
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------- expressions

/// A condition over the parameters: numbers, names, `! && || ( )` and the
/// comparisons. A name is its parameter's value; true is above one half.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f32),
    Name(String),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Compare(Box<Expr>, Cmp, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl Expr {
    pub fn parse(text: &str) -> Result<Expr, String> {
        let tokens = tokenize(text)?;
        let mut p = Parser { tokens, at: 0 };
        let e = p.or()?;
        if p.at != p.tokens.len() {
            return Err(format!("unexpected `{}`", p.tokens[p.at]));
        }
        Ok(e)
    }

    pub fn eval(&self, value: &dyn Fn(&str) -> f32) -> f32 {
        let truth = |b: bool| if b { 1.0 } else { 0.0 };
        match self {
            Expr::Number(n) => *n,
            Expr::Name(n) => value(n),
            Expr::Not(e) => truth(e.eval(value) <= 0.5),
            Expr::Neg(e) => -e.eval(value),
            Expr::And(a, b) => truth(a.eval(value) > 0.5 && b.eval(value) > 0.5),
            Expr::Or(a, b) => truth(a.eval(value) > 0.5 || b.eval(value) > 0.5),
            Expr::Compare(a, op, b) => {
                let (x, y) = (a.eval(value), b.eval(value));
                truth(match op {
                    Cmp::Lt => x < y,
                    Cmp::Le => x <= y,
                    Cmp::Gt => x > y,
                    Cmp::Ge => x >= y,
                    Cmp::Eq => (x - y).abs() < 1e-4,
                    Cmp::Ne => (x - y).abs() >= 1e-4,
                })
            }
        }
    }

    pub fn holds(&self, value: &dyn Fn(&str) -> f32) -> bool {
        self.eval(value) > 0.5
    }

    /// The parameters it reads.
    pub fn names(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.walk(&mut |e| {
            if let Expr::Name(n) = e {
                if n != "true" && n != "false" {
                    out.push(n.clone());
                }
            }
        });
        out
    }

    fn walk(&self, f: &mut dyn FnMut(&Expr)) {
        f(self);
        match self {
            Expr::Not(e) | Expr::Neg(e) => e.walk(f),
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Compare(a, _, b) => {
                a.walk(f);
                b.walk(f);
            }
            _ => {}
        }
    }
}

fn tokenize(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else {
            let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            if ["&&", "||", "<=", ">=", "==", "!="].contains(&two.as_str()) {
                out.push(two);
                i += 2;
            } else if "!<>()-".contains(c) {
                out.push(c.to_string());
                i += 1;
            } else {
                return Err(format!("`{c}` is not part of a condition"));
            }
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<String>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.at).map(String::as_str)
    }
    fn eat(&mut self, t: &str) -> bool {
        if self.peek() == Some(t) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn or(&mut self) -> Result<Expr, String> {
        let mut e = self.and()?;
        while self.eat("||") {
            e = Expr::Or(Box::new(e), Box::new(self.and()?));
        }
        Ok(e)
    }
    fn and(&mut self) -> Result<Expr, String> {
        let mut e = self.compare()?;
        while self.eat("&&") {
            e = Expr::And(Box::new(e), Box::new(self.compare()?));
        }
        Ok(e)
    }
    fn compare(&mut self) -> Result<Expr, String> {
        let left = self.unary()?;
        let op = match self.peek() {
            Some("<") => Cmp::Lt,
            Some("<=") => Cmp::Le,
            Some(">") => Cmp::Gt,
            Some(">=") => Cmp::Ge,
            Some("==") => Cmp::Eq,
            Some("!=") => Cmp::Ne,
            _ => return Ok(left),
        };
        self.at += 1;
        Ok(Expr::Compare(Box::new(left), op, Box::new(self.unary()?)))
    }
    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.eat("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat("(") {
            let e = self.or()?;
            if !self.eat(")") {
                return Err("a `(` is not closed".into());
            }
            return Ok(e);
        }
        let Some(t) = self.peek().map(str::to_string) else {
            return Err("it ends too soon".into());
        };
        self.at += 1;
        match t.as_str() {
            "true" => Ok(Expr::Number(1.0)),
            "false" => Ok(Expr::Number(0.0)),
            _ => match t.parse::<f32>() {
                Ok(n) => Ok(Expr::Number(n)),
                Err(_) if t.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') => Ok(Expr::Name(t)),
                Err(_) => Err(format!("`{t}` is not a number or a name")),
            },
        }
    }
}

// ---------------------------------------------------------------- the controller

/// An action the game started, to ask about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActionId(pub u64);

/// Where an action is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Playing,
    /// It played to its end (or chained on).
    Done,
    /// Something ended it early: an interrupt, another action in its slot.
    Interrupted,
}

#[derive(Debug, Clone)]
struct Running {
    id: ActionId,
    name: String,
    clip: usize,
    /// Where it was at the last update, 0..1.
    phase: f32,
    ending: bool,
}

/// A rules animator on one entity: its parameters, mode and actions.
#[derive(Debug, Clone)]
pub struct Rules {
    pub file: RulesFile,
    whens: Vec<Result<Expr, String>>,
    interrupts: HashMap<String, Vec<Option<Expr>>>,
    params: HashMap<String, f32>,
    pulled: HashSet<String>,
    mode: Option<usize>,
    held: Vec<f32>,
    released: bool,
    /// A pair's `via` clip playing before the mode's motion.
    via: bool,
    /// Slot name → the animator's layer.
    slots: Vec<(String, usize)>,
    running: HashMap<String, Running>,
    /// Actions asked for since the last update.
    pending: Vec<(String, ActionId)>,
    status: HashMap<ActionId, Status>,
    next_id: u64,
    fired: Vec<String>,
    why: Vec<String>,
    seed: u64,
}

impl Rules {
    pub fn new(file: RulesFile) -> Self {
        let whens = file.modes.iter().map(|m| Expr::parse(&m.when)).collect();
        let interrupts = file
            .actions
            .iter()
            .map(|(n, a)| (n.clone(), a.interrupt.iter().map(|i| Expr::parse(&i.when).ok()).collect()))
            .collect();
        let held = vec![0.0; file.modes.len()];
        Self {
            file,
            whens,
            interrupts,
            params: HashMap::new(),
            pulled: HashSet::new(),
            mode: None,
            held,
            released: false,
            via: false,
            slots: Vec::new(),
            running: HashMap::new(),
            pending: Vec::new(),
            status: HashMap::new(),
            next_id: 1,
            fired: Vec::new(),
            why: Vec::new(),
            seed: 0x9e37_79b9_7f4a_7c15,
        }
    }

    pub fn set(&mut self, name: &str, value: f32) {
        match self.params.get_mut(name) {
            Some(v) => *v = value,
            None => {
                self.params.insert(name.to_string(), value);
            }
        }
    }

    pub fn set_bool(&mut self, name: &str, value: bool) {
        self.set(name, if value { 1.0 } else { 0.0 });
    }

    pub fn get(&self, name: &str) -> f32 {
        if self.pulled.contains(name) {
            return 1.0;
        }
        self.params.get(name).copied().unwrap_or(0.0)
    }

    /// Pull a trigger for the next update: what a `chain` waits on, and
    /// true in conditions for that one update.
    pub fn trigger(&mut self, name: &str) {
        self.pulled.insert(name.to_string());
    }

    /// Let a `hold` mode go: the rules pick again at the next update.
    pub fn release(&mut self) {
        self.released = true;
    }

    /// The mode it is in.
    pub fn mode(&self) -> Option<&str> {
        self.mode.and_then(|m| self.file.modes.get(m)).map(|m| m.name.as_str())
    }

    /// The action playing in a slot.
    pub fn action_in(&self, slot: &str) -> Option<&str> {
        self.running.get(slot).filter(|r| !r.ending).map(|r| r.name.as_str())
    }

    pub fn status(&self, id: ActionId) -> Status {
        self.status.get(&id).copied().unwrap_or(Status::Done)
    }

    /// Events passed in the last update, of the mode's motion and of actions.
    pub fn fired(&self) -> &[String] {
        &self.fired
    }

    /// Why the mode is what it is, a line per rule above and at it:
    /// `#1 dead: false`, `#2 fall: true (for 0.05 s of 0.1)`, `#3 move: true ←`.
    pub fn why(&self) -> &[String] {
        &self.why
    }

    /// Start an action. What plays in its slot ends (interrupted).
    pub fn play(&mut self, name: &str) -> Option<ActionId> {
        self.file.actions.get(name)?;
        let id = ActionId(self.next_id);
        self.next_id += 1;
        self.pending.push((name.to_string(), id));
        self.status.insert(id, Status::Playing);
        Some(id)
    }

    fn random(&mut self) -> u64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        self.seed
    }

    fn slot_layer(&mut self, animator: &mut Animator, slot: &str) -> Option<usize> {
        if let Some((_, layer)) = self.slots.iter().find(|(s, _)| s == slot) {
            return Some(*layer);
        }
        let (mask, blend) = match self.file.slots.get(slot) {
            Some(s) => (s.mask.clone(), s.blend),
            None if slot == "full" => (Vec::new(), LayerBlend::Override),
            None => return None,
        };
        let (layer, _) = animator.add_layer(&mask, blend);
        self.slots.push((slot.to_string(), layer));
        Some(layer)
    }

    fn start(&mut self, animator: &mut Animator, name: &str, id: ActionId) {
        let Some(action) = self.file.actions.get(name).cloned() else {
            return;
        };
        let clip_name = if !action.pick.is_empty() {
            let (px, py) = action.pick_by.clone().unwrap_or_default();
            let (x, y) = (self.get(&px), self.get(&py));
            action
                .pick
                .iter()
                .min_by(|a, b| {
                    let d = |p: &(f32, f32, String)| (p.0 - x).powi(2) + (p.1 - y).powi(2);
                    d(a).total_cmp(&d(b))
                })
                .map(|p| p.2.clone())
                .unwrap_or_default()
        } else if !action.clips.is_empty() {
            let i = (self.random() % action.clips.len() as u64) as usize;
            action.clips[i].clone()
        } else {
            action.clip.clone()
        };
        let Some(layer) = self.slot_layer(animator, &action.slot) else {
            self.status.insert(id, Status::Interrupted);
            return;
        };
        let Some(clip) = animator.clip_named(&clip_name) else {
            self.status.insert(id, Status::Interrupted);
            return;
        };
        if let Some(was) = self.running.remove(&action.slot) {
            if !was.ending {
                self.status.insert(was.id, Status::Interrupted);
            }
        }
        if let Some(l) = animator.layer_mut(layer) {
            l.weight = 1.0;
            // A new start of the same clip: from its beginning.
            l.animator.stop(0.0);
            l.animator.play_once(clip, action.blend_in);
            l.animator.set_speed(action.speed);
        }
        self.running.insert(
            action.slot.clone(),
            Running { id, name: name.to_string(), clip, phase: 0.0, ending: false },
        );
    }

    /// One step: pick the mode, play its motion, run the actions. `dt`
    /// for the rules' timers (`enter_after`).
    pub fn update(&mut self, animator: &mut Animator, dt: f32) {
        self.fired.clear();
        self.why.clear();
        let (params, pulled) = (self.params.clone(), self.pulled.clone());
        let value = move |name: &str| -> f32 {
            if pulled.contains(name) {
                return 1.0;
            }
            params.get(name).copied().unwrap_or(0.0)
        };
        // Modes: timers, then the first that holds long enough.
        let mut chosen = None;
        for (i, (mode, when)) in self.file.modes.iter().zip(&self.whens).enumerate() {
            let holds = when.as_ref().is_ok_and(|e| e.holds(&value));
            self.held[i] = if holds { self.held[i] + dt } else { 0.0 };
            let ready = holds && self.held[i] + 1e-6 >= mode.enter_after;
            if chosen.is_none() {
                self.why.push(match (holds, ready) {
                    (true, true) => format!("#{} {}: true ←", i + 1, mode.name),
                    (true, false) => format!(
                        "#{} {}: true for {:.2} s of {:.2}",
                        i + 1,
                        mode.name,
                        self.held[i],
                        mode.enter_after
                    ),
                    _ => format!("#{} {}: false ({})", i + 1, mode.name, mode.when),
                });
                if ready {
                    chosen = Some(i);
                }
            }
        }
        let hold = self.mode.is_some_and(|m| self.file.modes[m].hold) && !self.released;
        if hold {
            chosen = self.mode;
        }
        self.released = false;
        if let Some(to) = chosen.filter(|to| Some(*to) != self.mode) {
            let from_name = self.mode.map(|m| self.file.modes[m].name.clone()).unwrap_or_default();
            let to_name = self.file.modes[to].name.clone();
            let pair = self
                .file
                .blends
                .pairs
                .iter()
                .find(|p| {
                    (p.from == from_name || p.from == "*")
                        && (p.to == to_name || p.to == "*")
                        && p.when.as_deref().is_none_or(|w| Expr::parse(w).is_ok_and(|e| e.holds(&value)))
                })
                .cloned();
            let fade = if self.mode.is_none() {
                0.0
            } else {
                pair.as_ref().and_then(|p| p.time).unwrap_or(self.file.blends.default)
            };
            self.via = false;
            if let Some(via) = pair.as_ref().and_then(|p| p.via.as_ref()).filter(|_| self.mode.is_some()) {
                if let Some(clip) = animator.clip_named(via) {
                    animator.play_once(clip, fade);
                    animator.set_speed(1.0);
                    self.via = true;
                }
            }
            self.mode = Some(to);
            if !self.via {
                self.play_motion(animator, fade, true, &value);
            }
        } else if self.via && animator.finished() {
            self.via = false;
            let fade = self.file.blends.default;
            self.play_motion(animator, fade, true, &value);
        } else if !self.via {
            self.play_motion(animator, 0.0, false, &value);
        }
        self.run_actions(animator);
        self.pulled.clear();
    }

    fn play_motion(&mut self, animator: &mut Animator, fade: f32, entered: bool, value: &dyn Fn(&str) -> f32) {
        let Some(mode) = self.mode.map(|m| self.file.modes[m].clone()) else {
            return;
        };
        match self.file.motions.get(&mode.play) {
            Some(state) if !state.directional.is_empty() => {
                if let Some((a, b, w, third)) = mix_2d(state, animator, value(&state.blend_by), value(&state.blend_by_y)) {
                    animator.blend_three(a, b, w, third, fade);
                }
                let factor = state.speed_from.as_deref().map_or(1.0, value);
                animator.set_speed(state.speed * factor);
            }
            Some(state) if !state.blend.is_empty() => {
                if let Some((a, b, w)) = mix_1d(state, animator, value(&state.blend_by)) {
                    animator.blend(a, b, w, fade);
                }
                let factor = state.speed_from.as_deref().map_or(1.0, value);
                animator.set_speed(state.speed * factor);
            }
            Some(state) => {
                if entered {
                    if let Some(clip) = animator.clip_named(&state.clip) {
                        if state.looping {
                            animator.play(clip, fade);
                        } else {
                            animator.play_once(clip, fade);
                        }
                    }
                }
                let factor = state.speed_from.as_deref().map_or(1.0, value);
                animator.set_speed(state.speed * factor);
            }
            None if entered => {
                if let Some(clip) = animator.clip_named(&mode.play) {
                    animator.play(clip, fade);
                    animator.set_speed(1.0);
                }
            }
            None => {}
        }
    }

    fn run_actions(&mut self, animator: &mut Animator) {
        for (name, id) in std::mem::take(&mut self.pending) {
            self.start(animator, &name, id);
        }
        let slots: Vec<String> = self.running.keys().cloned().collect();
        for slot in slots {
            let Some(layer) = self.slots.iter().find(|(s, _)| *s == slot).map(|(_, l)| *l) else {
                continue;
            };
            let Some(mut run) = self.running.get(&slot).cloned() else {
                continue;
            };
            let Some(action) = self.file.actions.get(&run.name).cloned() else {
                continue;
            };
            let Some(l) = animator.layer_mut(layer) else {
                continue;
            };
            let duration = l.animator.clips.get(run.clip).map_or(0.0, |c| c.duration);
            let phase = match (l.animator.playing(), duration > 0.0) {
                (Some(p), true) if p.clip == run.clip => (p.time / duration).min(1.0),
                _ => 1.0,
            };
            if run.ending {
                if l.animator.presence() <= 0.0 {
                    self.running.remove(&slot);
                }
                continue;
            }
            // Events between where it was and where it is.
            for (at, event) in &action.events {
                if *at > run.phase && *at <= phase || (run.phase == 0.0 && *at == 0.0) {
                    self.fired.push(event.clone());
                }
            }
            run.phase = phase;
            // A pull in a window goes on to the next.
            let chained = action
                .chain
                .iter()
                .find(|c| self.pulled.contains(&c.on) && phase >= c.window.0 && phase <= c.window.1)
                .map(|c| c.to.clone());
            if let Some(next) = chained {
                self.status.insert(run.id, Status::Done);
                let id = ActionId(self.next_id);
                self.next_id += 1;
                self.status.insert(id, Status::Playing);
                self.running.remove(&slot);
                self.start(animator, &next, id);
                continue;
            }
            let value = |name: &str| -> f32 {
                if self.pulled.contains(name) {
                    return 1.0;
                }
                self.params.get(name).copied().unwrap_or(0.0)
            };
            let interrupted = action.interrupt.iter().zip(self.interrupts.get(&run.name).into_iter().flatten()).any(
                |(i, e)| phase >= i.after && e.as_ref().is_some_and(|e| e.holds(&value)),
            );
            if interrupted || phase >= action.end.min(1.0) {
                self.status.insert(run.id, if interrupted { Status::Interrupted } else { Status::Done });
                run.ending = true;
                if let Some(l) = animator.layer_mut(layer) {
                    l.animator.stop(action.blend_out);
                }
            }
            self.running.insert(slot, run);
        }
    }
}

/// On an entity: its animator hands its root's travel to the game
/// ([`Animator::take_root_motion`]) rather than playing it in place.
/// Unity's Apply Root Motion.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RootMotion;

/// Every rules animator in the world, a step.
pub fn run_rules(world: &mut hecs::World, dt: f32) {
    for (animator, rules) in world.query_mut::<(&mut Animator, &mut Rules)>() {
        rules.update(animator, dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Channel, Clip, Joint, Path, PoseTransform, Skeleton};
    use std::sync::Arc;

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
                clip("run", 1.0),
                clip("fall", 1.0),
                clip("land", 0.2),
                clip("death", 1.0),
                clip("slash_a", 1.0),
                clip("slash_b", 1.0),
            ]),
        )
    }

    const RULES: &str = r#"(
        params: { "speed": Float, "grounded": Bool, "dead": Bool, "attack": Trigger },
        modes: [
            (name: "dead", when: "dead", play: "death", hold: true),
            (name: "fall", when: "!grounded", play: "fall", enter_after: 0.1),
            (name: "move", when: "true", play: "locomotion"),
        ],
        blends: (default: 0.2, pairs: [(from: "fall", to: "move", via: "land", time: 0.05)]),
        motions: { "locomotion": (blend_by: "speed", blend: [(0.0, "idle"), (4.0, "run")]) },
        actions: {
            "attack1": (clip: "slash_a", events: [(0.3, "hit")],
                        chain: [(on: "attack", window: (0.2, 0.9), to: "attack2")],
                        interrupt: [(when: "speed > 0.1", after: 0.6)], end: 0.9),
            "attack2": (clip: "slash_b"),
        },
    )"#;

    fn step(r: &mut Rules, a: &mut Animator, n: usize) {
        for _ in 0..n {
            r.update(a, 0.05);
            a.advance(0.05);
        }
    }

    #[test]
    fn the_first_rule_that_holds_is_the_mode() {
        let file: RulesFile = ron::from_str(RULES).unwrap();
        assert!(file.problems(&[]).is_empty(), "{:?}", file.problems(&[]));
        let (mut r, mut a) = (Rules::new(file), animator());
        r.set_bool("grounded", true);
        step(&mut r, &mut a, 1);
        assert_eq!(r.mode(), Some("move"));
        // A bump: not grounded for less than enter_after is no fall.
        r.set_bool("grounded", false);
        step(&mut r, &mut a, 1);
        assert_eq!(r.mode(), Some("move"), "{:?}", r.why());
        step(&mut r, &mut a, 2);
        assert_eq!(r.mode(), Some("fall"), "{:?}", r.why());
        // Landing plays `land` once, then the locomotion.
        r.set_bool("grounded", true);
        step(&mut r, &mut a, 1);
        assert_eq!(r.mode(), Some("move"));
        assert_eq!(a.playing().map(|p| p.clip), a.clip_named("land"));
        step(&mut r, &mut a, 6);
        assert!(a.blending().is_some(), "on to the blend after the landing");
        // Death holds until the game lets it go.
        r.set_bool("dead", true);
        step(&mut r, &mut a, 1);
        r.set_bool("dead", false);
        step(&mut r, &mut a, 3);
        assert_eq!(r.mode(), Some("dead"));
        r.release();
        step(&mut r, &mut a, 1);
        assert_eq!(r.mode(), Some("move"));
        assert!(r.why().last().unwrap().contains("move: true ←"), "{:?}", r.why());
    }

    #[test]
    fn an_action_chains_in_its_window_and_fires_its_events() {
        let (mut r, mut a) = (Rules::new(ron::from_str(RULES).unwrap()), animator());
        r.set_bool("grounded", true);
        step(&mut r, &mut a, 1);
        let first = r.play("attack1").unwrap();
        let mut fired = Vec::new();
        for _ in 0..3 {
            step(&mut r, &mut a, 1);
            fired.extend(r.fired().iter().cloned());
        }
        assert_eq!(r.action_in("full"), Some("attack1"));
        // Too early for the window (0.2): nothing.
        r.trigger("attack");
        step(&mut r, &mut a, 1);
        fired.extend(r.fired().iter().cloned());
        assert_eq!(r.action_in("full"), Some("attack1"));
        step(&mut r, &mut a, 3);
        fired.extend(r.fired().iter().cloned());
        assert_eq!(fired, ["hit"]);
        r.trigger("attack");
        step(&mut r, &mut a, 1);
        assert_eq!(r.action_in("full"), Some("attack2"));
        assert_eq!(r.status(first), Status::Done);
    }

    #[test]
    fn running_ends_an_action_early() {
        let (mut r, mut a) = (Rules::new(ron::from_str(RULES).unwrap()), animator());
        r.set_bool("grounded", true);
        let id = r.play("attack1").unwrap();
        r.set("speed", 3.0);
        step(&mut r, &mut a, 10);
        assert_eq!(r.status(id), Status::Playing, "before 0.6 it holds");
        step(&mut r, &mut a, 4);
        assert_eq!(r.status(id), Status::Interrupted);
    }

    #[test]
    fn a_misspelt_parameter_and_an_unreachable_rule_are_named() {
        let file: RulesFile = ron::from_str(
            r#"(params: { "grounded": Bool },
               modes: [(name: "move", when: "true", play: "idle"),
                       (name: "fall", when: "!grounde", play: "fall")])"#,
        )
        .unwrap();
        let p = file.problems(&["idle", "fall"]).join("\n");
        assert!(p.contains("did you mean `grounded`"), "{p}");
        assert!(p.contains("`fall` is never taken"), "{p}");
    }

    #[test]
    fn conditions_read_as_written() {
        let e = Expr::parse("!grounded && (speed > 0.1 || attack) && hp != 0").unwrap();
        let v = |s: &str| match s {
            "grounded" => 0.0,
            "speed" => 0.05,
            "attack" => 1.0,
            "hp" => 3.0,
            _ => 0.0,
        };
        assert!(e.holds(&v));
        assert!(Expr::parse("speed >").is_err());
        assert!(Expr::parse("a $ b").is_err());
    }
}
