//! Screens written in HTML and CSS: a game's menus and HUDs as files a
//! designer — or an agent — writes the way the web is written, built into
//! `scrap-ui`'s tree and drawn by the engine's own renderer. Not a
//! browser: the part of HTML and CSS this UI can draw, read strictly, and
//! no script. `docs/html-ui.md` is the card.
//!
//! ```html
//! <link rel="stylesheet" href="hud.css">
//! <div class="top">
//!   <span id="clock">{{round.clock}}</span>
//!   <div class="bar"><div class="fill" data-bind-style-width="{{player.hp / player.max * 100}}%"></div></div>
//! </div>
//! <div class="orders">
//!   <div class="order" data-bind-for="order:{{orders}}">
//!     <span>{{order.dish}}</span>
//!     <button id="cancel">@hud.cancel</button>
//!   </div>
//! </div>
//! <button id="leave" data-bind-if="{{round.over}}">Leave</button>
//! ```
//!
//! As in Coherent Gameface, what the screen shows comes from a model the
//! game gives it — [`Screen::set`] takes anything `Serialize` — and the
//! `data-bind-*` attributes say where each value goes: `data-bind-value`
//! (or `{{ }}` in the text), `data-bind-if`, `data-bind-for`,
//! `data-bind-class-toggle`, `data-bind-style-<property>`. What the player
//! did comes back as data, by element id: [`Screen::read`] gives a
//! [`Done`], and `done.clicked("leave")` is a system's `if`. The logic stays
//! the game's Rust (DNA, postulate 5).
//!
//! The screen's files are read again when saved ([`Screen::poll`]): the
//! tree is built anew from the model the game already gave, and a file
//! that does not read is a list of problems, each at its line and column,
//! with the last good screen kept (postulate 1).

pub mod css;
pub mod expr;
pub mod html;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use serde_json::Value;

use crate::style::{Style, TextStyle};
use crate::{Event, NodeId, Ui};
use css::{Context, Late, Sheet, State, Subject};
use expr::{Scope, Template};
use html::{Content, Element};

/// Where in a file: a line and a column, both from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
    pub line: u32,
    pub column: u32,
}

/// Something in a screen's files that cannot mean anything, where it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub file: String,
    pub at: Pos,
    pub message: String,
}

impl Problem {
    pub fn new(file: &str, at: Pos, message: impl Into<String>) -> Self {
        Self {
            file: file.to_string(),
            at,
            message: message.into(),
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.file, self.at.line, self.at.column, self.message
        )
    }
}

/// Every problem a screen's files have, one a line.
#[derive(Debug, Clone, PartialEq)]
pub struct Problems(pub Vec<Problem>);

impl fmt::Display for Problems {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (i, p) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{p}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Problems {}

/// The look every screen starts from — a browser's own stylesheet, for
/// the tags read here.
const UA: &str = include_str!("ua.css");

/// The tags that are boxes and words.
const BOXES: [&str; 22] = [
    "body", "div", "section", "header", "footer", "main", "nav", "aside", "article", "ul", "ol",
    "li", "span", "p", "label", "strong", "b", "small", "h1", "h2", "h3", "h4",
];

/// What an element is to the UI.
#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Box,
    Button,
    /// `<input>`: a line the player types.
    Field {
        placeholder: String,
        value: String,
    },
}

/// A loop: `data-bind-for="order:{{orders}}"`, or `"i, order:{{orders}}"`
/// for the index as well.
#[derive(Debug, Clone, PartialEq)]
struct Repeat {
    item: String,
    index: Option<String>,
    list: Template,
}

/// An element made ready to build: what it is, how it is styled, what it
/// is bound to.
#[derive(Debug, Clone, PartialEq)]
struct Tmpl {
    tag: String,
    id: Option<String>,
    classes: Vec<String>,
    inline: Vec<css::Decl>,
    kind: Kind,
    /// Its own words, when it holds words only.
    text: Option<Template>,
    kids: Vec<TKid>,
    show_if: Option<Template>,
    toggles: Vec<(String, Template)>,
    styles: Vec<(String, Template, Pos)>,
    repeat: Option<Repeat>,
    at: Pos,
}

#[derive(Debug, Clone, PartialEq)]
enum TKid {
    Elem(Box<Tmpl>),
    /// Words between elements: `Score: <b>{{score}}</b>`.
    Text(Template),
}

/// A screen's files, read.
#[derive(Debug, Clone, PartialEq)]
struct Document {
    body: Arc<Tmpl>,
    ua: Sheet,
    sheet: Sheet,
}

/// Who a node answers for when it is clicked: the nearest element with an
/// id, and which row of a list it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub id: String,
    /// The row of the innermost `data-bind-for` it is in.
    pub index: Option<usize>,
}

/// What the player did to a screen, by element id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Done {
    pub clicked: Vec<Target>,
    /// Fields typed into, with what they say now.
    pub changed: Vec<(Target, String)>,
    /// Fields Enter was pressed in.
    pub submitted: Vec<(Target, String)>,
}

impl Done {
    pub fn clicked(&self, id: &str) -> bool {
        self.clicked.iter().any(|t| t.id == id)
    }

    /// Which row's `id` was clicked: the cancel button of the second order
    /// is `Some(1)`.
    pub fn clicked_row(&self, id: &str) -> Option<usize> {
        self.clicked
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.index)
    }

    pub fn changed(&self, id: &str) -> Option<&str> {
        self.changed
            .iter()
            .find(|(t, _)| t.id == id)
            .map(|(_, s)| s.as_str())
    }

    pub fn submitted(&self, id: &str) -> Option<&str> {
        self.submitted
            .iter()
            .find(|(t, _)| t.id == id)
            .map(|(_, s)| s.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.clicked.is_empty() && self.changed.is_empty() && self.submitted.is_empty()
    }
}

/// One element as built: its node, and what was built for what is inside.
#[derive(Debug)]
struct Inst {
    node: NodeId,
    kids: Vec<IKid>,
}

#[derive(Debug)]
enum IKid {
    One(Inst),
    Text(NodeId),
    /// The rows of a `data-bind-for`, one per item.
    Many(Vec<Inst>),
}

impl IKid {
    fn nodes(&self) -> usize {
        match self {
            IKid::One(_) | IKid::Text(_) => 1,
            IKid::Many(rows) => rows.len(),
        }
    }
}

/// What an element is to its selectors, kept while the walk is under it.
#[derive(Debug, Clone)]
struct Owned {
    tag: String,
    id: Option<String>,
    classes: Vec<String>,
}

/// What the walk down the tree carries.
#[derive(Clone)]
struct Walk {
    locals: Vec<(String, Value)>,
    chain: Vec<Owned>,
    text: TextStyle,
    owner: Option<String>,
    row: Option<usize>,
    /// The window, for `vw` and `vh`.
    viewport: (f32, f32),
}

struct Mounted {
    parent: NodeId,
    inst: Inst,
    viewport: (f32, f32),
}

/// A screen: its files, kept up with, the model the game gives it, and
/// the nodes built for it in a [`Ui`].
pub struct Screen {
    name: String,
    html: Option<PathBuf>,
    doc: Document,
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
    since_poll: f32,
    model: Value,
    dirty: bool,
    strings: Option<scrap_core::strings::Strings>,
    mounted: Option<Mounted>,
    targets: HashMap<NodeId, Target>,
    /// The nodes of elements with an id of their own, not ones inside them.
    own_id: HashSet<NodeId>,
    missing: BTreeSet<String>,
    /// Bound styles that would not read, said once each.
    bad_styles: BTreeSet<String>,
}

fn stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// How a file is named in a problem: as the caller gave its path.
fn label(path: &Path) -> String {
    path.display().to_string()
}

impl Screen {
    /// Read a screen from its HTML file and the stylesheets it links.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Problems> {
        let path = path.as_ref();
        let (doc, files) = read_files(path)?;
        let stamps = files.iter().map(|f| (f.clone(), stamp(f))).collect();
        Ok(Self::new(
            label(path),
            Some(path.to_path_buf()),
            doc,
            stamps,
        ))
    }

    /// A screen from text in memory: `css` are the stylesheets a `<link>`
    /// names, by the name it gives.
    pub fn from_source(name: &str, html: &str, css: &[(&str, &str)]) -> Result<Self, Problems> {
        let doc = document(name, html, &mut |href, at| {
            css.iter()
                .find(|(n, _)| *n == href)
                .map(|(_, s)| (href.to_string(), s.to_string()))
                .ok_or_else(|| Problem::new(name, at, format!("no stylesheet `{href}`")))
        })?;
        Ok(Self::new(name.to_string(), None, doc, Vec::new()))
    }

    /// Every `@key` the screen's words ask for: what `scrap check` holds
    /// against the languages in `content/localization/`.
    pub fn keys(&self) -> Vec<String> {
        fn literal(t: &Template) -> Option<&str> {
            match t.pieces.as_slice() {
                [expr::Piece::Text(s)] => s.strip_prefix('@'),
                _ => None,
            }
        }
        fn walk(t: &Tmpl, out: &mut Vec<String>) {
            out.extend(t.text.as_ref().and_then(literal).map(str::to_string));
            if let Kind::Field { placeholder, .. } = &t.kind {
                out.extend(placeholder.strip_prefix('@').map(str::to_string));
            }
            for k in &t.kids {
                match k {
                    TKid::Elem(e) => walk(e, out),
                    TKid::Text(tt) => out.extend(literal(tt).map(str::to_string)),
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.doc.body, &mut out);
        out
    }

    /// What is wrong with the screen at `path`, without building it: what
    /// `scrap check` says.
    pub fn check(path: impl AsRef<Path>) -> Vec<Problem> {
        match read_files(path.as_ref()) {
            Ok(_) => Vec::new(),
            Err(p) => p.0,
        }
    }

    fn new(
        name: String,
        html: Option<PathBuf>,
        doc: Document,
        stamps: Vec<(PathBuf, Option<SystemTime>)>,
    ) -> Self {
        Self {
            name,
            html,
            doc,
            stamps,
            since_poll: 0.0,
            model: Value::Object(Default::default()),
            dirty: true,
            strings: None,
            mounted: None,
            targets: HashMap::new(),
            own_id: HashSet::new(),
            missing: BTreeSet::new(),
            bad_styles: BTreeSet::new(),
        }
    }

    /// Give the screen a value: `screen.set("player", &player)`, or one
    /// field, `screen.set("round.clock", "1:30")`. Anything `Serialize`.
    /// The same value again changes nothing.
    pub fn set(&mut self, path: &str, value: impl serde::Serialize) {
        let value = serde_json::to_value(value).unwrap_or(Value::Null);
        let mut at = &mut self.model;
        let names: Vec<&str> = path.split('.').collect();
        for (i, name) in names.iter().enumerate() {
            if !at.is_object() {
                *at = Value::Object(Default::default());
            }
            let map = at.as_object_mut().expect("made an object above");
            if i + 1 == names.len() {
                if map.get(*name) != Some(&value) {
                    map.insert(name.to_string(), value);
                    self.dirty = true;
                }
                return;
            }
            at = map
                .entry(name.to_string())
                .or_insert_with(|| Value::Object(Default::default()));
        }
    }

    /// The model as the game has given it.
    pub fn model(&self) -> &Value {
        &self.model
    }

    /// `@key` texts in this table's language from now on.
    pub fn set_strings(&mut self, strings: scrap_core::strings::Strings) {
        self.strings = Some(strings);
        self.dirty = true;
    }

    /// The paths the screen asks for that the model does not have — the
    /// game forgot to `set` them, or the screen misspelled them.
    pub fn missing(&self) -> Vec<String> {
        self.missing.iter().cloned().collect()
    }

    /// Build the screen under `parent`; a screen built elsewhere moves.
    pub fn mount(&mut self, ui: &mut Ui, parent: NodeId) {
        self.unmount(ui);
        self.missing.clear();
        let (w, h, _) = ui.viewport();
        let inst = self.build(ui, parent, 0, None);
        self.mounted = Some(Mounted {
            parent,
            inst,
            viewport: (w, h),
        });
        self.dirty = false;
    }

    /// Take the screen's nodes out of the tree.
    pub fn unmount(&mut self, ui: &mut Ui) {
        if let Some(m) = self.mounted.take() {
            ui.remove(m.inst.node);
        }
        self.targets.clear();
        self.own_id.clear();
    }

    /// The node built for the element with this id (the first, in a list):
    /// what [`Ui::click`] takes in a test.
    pub fn node(&self, id: &str) -> Option<NodeId> {
        fn walk(inst: &Inst, want: &dyn Fn(NodeId) -> bool) -> Option<NodeId> {
            if want(inst.node) {
                return Some(inst.node);
            }
            inst.kids.iter().find_map(|k| match k {
                IKid::One(i) => walk(i, want),
                IKid::Many(rows) => rows.iter().find_map(|i| walk(i, want)),
                IKid::Text(_) => None,
            })
        }
        let m = self.mounted.as_ref()?;
        walk(&m.inst, &|n| {
            self.own_id.contains(&n) && self.targets.get(&n).is_some_and(|t| t.id == id)
        })
    }

    /// Bring the built tree up to the model, the window and the language:
    /// call it every frame — when nothing changed it costs nothing.
    pub fn update(&mut self, ui: &mut Ui) {
        let Some(m) = self.mounted.as_ref() else {
            return;
        };
        let (w, h, _) = ui.viewport();
        if !self.dirty && m.viewport == (w, h) {
            return;
        }
        let mut m = self.mounted.take().expect("looked at above");
        self.missing.clear();
        let inst = self.build(ui, m.parent, 0, Some(m.inst));
        m.inst = inst;
        m.viewport = (w, h);
        self.mounted = Some(m);
        self.dirty = false;
    }

    /// What the player did to this screen, from what the [`Ui`] says
    /// happened this frame. Several screens read the same list.
    pub fn read(&self, events: &[(NodeId, Event)]) -> Done {
        let mut done = Done::default();
        for (node, event) in events {
            let Some(target) = self.targets.get(node) else {
                continue;
            };
            match event {
                Event::Click { .. } => done.clicked.push(target.clone()),
                Event::Changed(text) => done.changed.push((target.clone(), text.clone())),
                Event::Submit(text) => done.submitted.push((target.clone(), text.clone())),
                _ => {}
            }
        }
        done
    }

    /// Read the files again when one has changed, a few times a second at
    /// most: call it every frame. A new screen is built from the model the
    /// game gave; one that does not read is its problems, and the last
    /// good screen stays.
    pub fn poll(&mut self, ui: &mut Ui, delta: f32) -> Option<Result<(), Problems>> {
        self.since_poll += delta;
        if self.since_poll < 0.25 {
            return None;
        }
        self.since_poll = 0.0;
        let path = self.html.clone()?;
        let changed = self.stamps.iter().any(|(f, s)| stamp(f) != *s);
        if !changed {
            return None;
        }
        match read_files(&path) {
            Ok((doc, files)) => {
                self.stamps = files.iter().map(|f| (f.clone(), stamp(f))).collect();
                self.doc = doc;
                self.bad_styles.clear();
                if let Some(parent) = self.mounted.as_ref().map(|m| m.parent) {
                    self.mount(ui, parent);
                }
                Some(Ok(()))
            }
            Err(problems) => {
                // Not again until it changes again.
                for (f, s) in &mut self.stamps {
                    *s = stamp(f);
                }
                Some(Err(problems))
            }
        }
    }

    /// Bound style values that would not read, as problems: a model value
    /// that made `width: abc%`.
    pub fn style_problems(&self) -> Vec<String> {
        self.bad_styles.iter().cloned().collect()
    }

    // --- building ------------------------------------------------------

    fn build(&mut self, ui: &mut Ui, parent: NodeId, index: usize, old: Option<Inst>) -> Inst {
        let (w, h, _) = ui.viewport();
        let walk = Walk {
            locals: Vec::new(),
            chain: Vec::new(),
            text: TextStyle::default(),
            owner: None,
            row: None,
            viewport: (w, h),
        };
        let body = self.doc.body.clone();
        self.realize(ui, &body, old, parent, index, &walk)
    }

    fn eval_scope<'a>(&'a self, walk: &'a Walk) -> Scope<'a> {
        Scope {
            model: &self.model,
            locals: &walk.locals,
        }
    }

    fn realize(
        &mut self,
        ui: &mut Ui,
        t: &Tmpl,
        old: Option<Inst>,
        parent: NodeId,
        index: usize,
        walk: &Walk,
    ) -> Inst {
        let mut missing = Vec::new();
        let scope = self.eval_scope(walk);
        let shown = t
            .show_if
            .as_ref()
            .is_none_or(|c| expr::truthy(&c.value(&scope, &mut missing)));
        let mut classes = t.classes.clone();
        for (class, cond) in &t.toggles {
            let on = expr::truthy(&cond.value(&scope, &mut missing));
            if on && !classes.contains(class) {
                classes.push(class.clone());
            }
            if !on {
                classes.retain(|c| c != class);
            }
        }
        let bound: Vec<(String, String, Pos)> = t
            .styles
            .iter()
            .map(|(p, v, at)| (p.clone(), v.text(&scope, &mut missing), *at))
            .collect();
        let text = t.text.as_ref().map(|tt| tt.text(&scope, &mut missing));
        self.missing.extend(missing);

        let me = Owned {
            tag: t.tag.clone(),
            id: t.id.clone(),
            classes,
        };
        let mut chain = walk.chain.clone();
        chain.push(me);
        let mut style = self.compute(t, &chain, &walk.text, &bound, walk.viewport);
        if !shown {
            style.layout.display = taffy::Display::None;
        }

        // The node: the old one restyled, or a new one.
        let node = match &old {
            Some(inst) => {
                ui.set_style(inst.node, style.clone());
                inst.node
            }
            None => {
                let node = match &t.kind {
                    Kind::Field { placeholder, value } => {
                        let n = ui.add_field(parent, style.clone(), value);
                        if !placeholder.is_empty() {
                            let hint = self.say(placeholder);
                            ui.set_placeholder(n, &hint);
                        }
                        ui.place_at(n, index);
                        n
                    }
                    _ => ui.add_at(parent, index, style.clone()),
                };
                if let Some(id) = &t.id {
                    ui.set_name(node, id.clone());
                }
                node
            }
        };
        let owner = t.id.clone().or_else(|| walk.owner.clone());
        if let Some(id) = &owner {
            self.targets.insert(
                node,
                Target {
                    id: id.clone(),
                    index: walk.row,
                },
            );
        }
        if t.id.is_some() {
            self.own_id.insert(node);
        }
        if let (Some(text), false) = (&text, matches!(t.kind, Kind::Field { .. })) {
            let words = self.say(text);
            ui.set_text(node, &words);
        }

        // What is inside.
        let mut under = walk.clone();
        under.chain = chain;
        under.text = style.text.clone();
        under.owner = owner;
        let mut olds = old.map(|o| o.kids).unwrap_or_default().into_iter();
        let mut kids = Vec::with_capacity(t.kids.len());
        let mut at = 0;
        for kid in &t.kids {
            let prev = olds.next();
            let built = match kid {
                TKid::Text(tt) => {
                    let mut missing = Vec::new();
                    let words = tt.text(&self.eval_scope(&under), &mut missing);
                    self.missing.extend(missing);
                    let words = self.say(&words);
                    let s = Style {
                        text: under.text.clone(),
                        ..Default::default()
                    };
                    let n = match prev {
                        Some(IKid::Text(n)) => {
                            ui.set_style(n, s);
                            n
                        }
                        _ => ui.add_at(node, at, s),
                    };
                    ui.set_text(n, &words);
                    IKid::Text(n)
                }
                TKid::Elem(child) if child.repeat.is_some() => {
                    let rows = match prev {
                        Some(IKid::Many(rows)) => rows,
                        _ => Vec::new(),
                    };
                    IKid::Many(self.rows(ui, child, rows, node, at, &under))
                }
                TKid::Elem(child) => {
                    let prev = match prev {
                        Some(IKid::One(i)) => Some(i),
                        _ => None,
                    };
                    IKid::One(self.realize(ui, child, prev, node, at, &under))
                }
            };
            at += built.nodes();
            kids.push(built);
        }
        Inst { node, kids }
    }

    /// The rows of a `data-bind-for`: as many as the list has, the ones
    /// already built kept, the extra ones removed.
    fn rows(
        &mut self,
        ui: &mut Ui,
        t: &Tmpl,
        mut rows: Vec<Inst>,
        parent: NodeId,
        start: usize,
        walk: &Walk,
    ) -> Vec<Inst> {
        let repeat = t.repeat.as_ref().expect("asked for rows of a loop");
        let mut missing = Vec::new();
        let list = repeat.list.value(&self.eval_scope(walk), &mut missing);
        self.missing.extend(missing);
        let items = match list {
            Value::Array(items) => items,
            Value::Null => Vec::new(),
            other => vec![other],
        };
        while rows.len() > items.len() {
            let gone = rows.pop().expect("longer than the items");
            self.forget(&gone);
            ui.remove(gone.node);
        }
        let mut out = Vec::with_capacity(items.len());
        let mut rows = rows.into_iter();
        for (i, item) in items.into_iter().enumerate() {
            let mut row = walk.clone();
            row.locals.push((repeat.item.clone(), item));
            if let Some(index) = &repeat.index {
                row.locals.push((index.clone(), Value::from(i)));
            }
            row.row = Some(i);
            out.push(self.realize(ui, t, rows.next(), parent, start + i, &row));
        }
        out
    }

    fn forget(&mut self, inst: &Inst) {
        self.targets.remove(&inst.node);
        self.own_id.remove(&inst.node);
        for kid in &inst.kids {
            match kid {
                IKid::One(i) => self.forget(i),
                IKid::Many(rows) => rows.iter().for_each(|i| self.forget(i)),
                IKid::Text(_) => {}
            }
        }
    }

    /// `@key` in the current language; anything else as it is.
    fn say(&self, text: &str) -> String {
        match &self.strings {
            Some(s) => s.resolve(text).to_string(),
            None => text.to_string(),
        }
    }

    /// An element's style: the UA sheet, then the screen's, by
    /// specificity and order, then `style=""`, then what is bound.
    fn compute(
        &mut self,
        t: &Tmpl,
        chain: &[Owned],
        parent_text: &TextStyle,
        bound: &[(String, String, Pos)],
        viewport: (f32, f32),
    ) -> Style {
        let subjects: Vec<Subject> = chain
            .iter()
            .map(|o| Subject {
                tag: &o.tag,
                id: o.id.as_deref(),
                classes: o.classes.iter().map(String::as_str).collect(),
            })
            .collect();
        let mut hits: Vec<Hit> = Vec::new();
        for (origin, sheet) in [(0u8, &self.doc.ua), (1, &self.doc.sheet)] {
            for (order, rule) in sheet.rules.iter().enumerate() {
                for sel in &rule.selectors {
                    if sel.matches(&subjects) {
                        for d in &rule.decls {
                            hits.push(((origin, sel.specificity(), order), sel.state, d));
                        }
                    }
                }
            }
        }
        hits.sort_by_key(|(key, state, _)| (*state as u8 != 0, *key));
        let mut style = Style::column();
        style.text = parent_text.clone();
        let mut late = Late::default();
        let vars = &self.doc.sheet.vars;
        let cx = Context { viewport, vars };
        for (_, state, d) in &hits {
            // Every value was tried when the sheet was read.
            let _ = css::apply(&mut style, &mut late, *state, &d.property, &d.value, &cx);
        }
        for d in &t.inline {
            let _ = css::apply(
                &mut style,
                &mut late,
                State::Always,
                &d.property,
                &d.value,
                &cx,
            );
        }
        let mut bad = Vec::new();
        for (p, v, at) in bound {
            if let Err(e) = css::apply(&mut style, &mut late, State::Always, p, v, &cx) {
                bad.push(format!("{}:{}:{}: {e}", self.name, at.line, at.column));
            }
        }
        self.bad_styles.extend(bad);
        late.settle(&mut style);
        match t.kind {
            Kind::Field { .. } => style.focusable().nowrap(),
            Kind::Button => {
                style.sense.click = true;
                style
            }
            Kind::Box => style,
        }
    }
}

/// Read the HTML at `path` and the stylesheets it links: the document, and
/// every file it was read from.
fn read_files(path: &Path) -> Result<(Document, Vec<PathBuf>), Problems> {
    let name = label(path);
    let src = std::fs::read_to_string(path).map_err(|e| {
        Problems(vec![Problem::new(
            &name,
            Pos::default(),
            format!("cannot be read: {e}"),
        )])
    })?;
    let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
    let mut files = vec![path.to_path_buf()];
    let doc = document(&name, &src, &mut |href, at| {
        let file = dir.join(href);
        files.push(file.clone());
        std::fs::read_to_string(&file)
            .map(|css| (label(&file), css))
            .map_err(|e| {
                Problem::new(
                    &name,
                    at,
                    format!("the stylesheet `{href}` cannot be read: {e}"),
                )
            })
    })?;
    Ok((doc, files))
}

/// What brings a stylesheet a `<link>` names: its label and its text.
type LoadSheet<'a> = dyn FnMut(&str, Pos) -> Result<(String, String), Problem> + 'a;

/// A declaration that applies to an element, with what orders it in the
/// cascade: the sheet (the UA's first), the selector's specificity, the
/// rule's place in the sheet.
type Hit<'a> = ((u8, (u32, u32, u32), usize), State, &'a css::Decl);

/// Read a document: its HTML, and the stylesheets `load` brings by the
/// name a `<link>` gives (as the file's label and its text).
fn document(name: &str, src: &str, load: &mut LoadSheet) -> Result<Document, Problems> {
    let top = html::parse(name, src).map_err(|p| Problems(vec![p]))?;
    let mut problems = Vec::new();
    let mut sheet = Sheet::default();
    let mut body: Option<Element> = None;
    let mut loose: Vec<Content> = Vec::new();

    // `<html>`, `<head>` and `<body>` are optional: a fragment is a body.
    fn gather(
        name: &str,
        items: Vec<Content>,
        body: &mut Option<Element>,
        loose: &mut Vec<Content>,
        heads: &mut Vec<Element>,
        problems: &mut Vec<Problem>,
    ) {
        for c in items {
            match c {
                Content::Element(e) if e.tag == "html" => {
                    gather(name, e.children, body, loose, heads, problems)
                }
                Content::Element(e) if e.tag == "head" => {
                    heads.extend(e.children.into_iter().filter_map(|c| match c {
                        Content::Element(e) => Some(e),
                        Content::Text(..) => None,
                    }))
                }
                Content::Element(e)
                    if matches!(e.tag.as_str(), "link" | "style" | "meta" | "title") =>
                {
                    heads.push(e)
                }
                Content::Element(e) if e.tag == "body" => {
                    if body.is_some() {
                        problems.push(Problem::new(name, e.at, "a second `<body>`"));
                    }
                    *body = Some(e);
                }
                other => loose.push(other),
            }
        }
    }
    let mut heads = Vec::new();
    gather(name, top, &mut body, &mut loose, &mut heads, &mut problems);

    for e in heads {
        match e.tag.as_str() {
            "link" => {
                if e.attr("rel") != Some("stylesheet") {
                    problems.push(Problem::new(
                        name,
                        e.at,
                        "a `<link>` here is a stylesheet: `rel=\"stylesheet\"`",
                    ));
                    continue;
                }
                let Some(href) = e.attr("href") else {
                    problems.push(Problem::new(name, e.at, "a `<link>` without `href`"));
                    continue;
                };
                match load(href, e.at) {
                    Ok((file, css)) => merge(
                        &mut sheet,
                        css::parse(&file, &css, Pos { line: 1, column: 1 }),
                        &mut problems,
                    ),
                    Err(p) => problems.push(p),
                }
            }
            "style" => {
                if let Some(Content::Text(css, at)) = e.children.first() {
                    merge(&mut sheet, css::parse(name, css, *at), &mut problems);
                }
            }
            "script" => problems.push(script(name, e.at)),
            _ => {}
        }
    }
    let body = match body {
        Some(b) => {
            if let Some(c) = loose
                .iter()
                .map(|c| match c {
                    Content::Element(e) => e.at,
                    Content::Text(_, at) => *at,
                })
                .next()
            {
                problems.push(Problem::new(name, c, "outside `<body>`: put it inside"));
            }
            b
        }
        None => Element {
            tag: "body".into(),
            attrs: Vec::new(),
            children: loose,
            at: Pos { line: 1, column: 1 },
        },
    };
    problems.extend(css::check(&sheet));
    let mut ids = HashMap::new();
    let body = compile(name, &body, false, &mut ids, &mut problems);
    let ua = css::parse("ua.css", UA, Pos { line: 1, column: 1 }).expect("the UA sheet reads");
    if problems.is_empty() {
        Ok(Document {
            body: Arc::new(body),
            ua,
            sheet,
        })
    } else {
        Err(Problems(problems))
    }
}

fn merge(into: &mut Sheet, sheet: Result<Sheet, Vec<Problem>>, problems: &mut Vec<Problem>) {
    match sheet {
        Ok(s) => {
            into.rules.extend(s.rules);
            into.vars.extend(s.vars);
        }
        Err(p) => problems.extend(p),
    }
}

fn script(name: &str, at: Pos) -> Problem {
    Problem::new(
        name,
        at,
        "no scripts: a screen shows the model and the game's Rust does the rest — bind with `data-bind-*` and read clicks by id (DNA, postulate 5)",
    )
}

/// An element made ready to build, every problem with it listed.
fn compile(
    name: &str,
    e: &Element,
    in_loop: bool,
    ids: &mut HashMap<String, Pos>,
    problems: &mut Vec<Problem>,
) -> Tmpl {
    let mut say = |at: Pos, m: String| problems.push(Problem::new(name, at, m));
    let kind = match e.tag.as_str() {
        t if BOXES.contains(&t) => Kind::Box,
        "button" => Kind::Button,
        "input" => {
            match e.attr("type").unwrap_or("text") {
                "text" => {}
                other => say(
                    e.at,
                    format!("`<input type=\"{other}\">` is not read yet: only text fields"),
                ),
            }
            Kind::Field {
                placeholder: e.attr("placeholder").unwrap_or("").to_string(),
                value: e.attr("value").unwrap_or("").to_string(),
            }
        }
        "script" => {
            problems.push(script(name, e.at));
            Kind::Box
        }
        "img" => {
            say(e.at, "`<img>` is not read yet: pictures on screens wait for asset references (docs/refs.md)".into());
            Kind::Box
        }
        "a" => {
            say(
                e.at,
                "no links: a `<button id=\"…\">` the game reads by id".into(),
            );
            Kind::Box
        }
        other => {
            say(e.at, format!("`<{other}>` is not an element this UI reads"));
            Kind::Box
        }
    };
    let mut t = Tmpl {
        tag: e.tag.clone(),
        id: None,
        classes: e.classes().map(str::to_string).collect(),
        inline: Vec::new(),
        kind,
        text: None,
        kids: Vec::new(),
        show_if: None,
        toggles: Vec::new(),
        styles: Vec::new(),
        repeat: None,
        at: e.at,
    };
    let mut value: Option<Template> = None;
    for a in &e.attrs {
        let template = |v: &str| Template::parse(v);
        match a.name.as_str() {
            "id" => {
                t.id = Some(a.value.clone());
            }
            "class" => {}
            "style" => match css::parse(name, &format!("*{{{}}}", a.value), a.at) {
                Ok(s) => {
                    problems.extend(css::check(&s).into_iter().map(|mut p| {
                        p.at = a.at;
                        p
                    }));
                    t.inline = s.rules.into_iter().flat_map(|r| r.decls).collect();
                }
                Err(p) => problems.extend(p.into_iter().map(|mut p| {
                    p.at = a.at;
                    p
                })),
            },
            "placeholder" | "value" | "type" if e.tag == "input" => {}
            "data-bind-value" => match template(&a.value) {
                Ok(v) => value = Some(v),
                Err(m) => problems.push(Problem::new(name, a.at, m)),
            },
            "data-bind-if" => match template(&a.value) {
                Ok(v) => t.show_if = Some(v),
                Err(m) => problems.push(Problem::new(name, a.at, m)),
            },
            "data-bind-for" => {
                let parsed = (|| {
                    let (vars, list) = a
                        .value
                        .split_once(':')
                        .ok_or("`data-bind-for` wants `item:{{list}}`")?;
                    let names: Vec<&str> = vars.split(',').map(str::trim).collect();
                    let (index, item) = match names.as_slice() {
                        [item] => (None, *item),
                        [index, item] => (Some(index.to_string()), *item),
                        _ => return Err("`data-bind-for` wants `item:{{list}}` or `i, item:{{list}}`".to_string()),
                    };
                    if item.is_empty() || !item.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        return Err(format!("`{item}` is not a name for the item"));
                    }
                    Ok(Repeat {
                        item: item.to_string(),
                        index,
                        list: template(list.trim())?,
                    })
                })();
                match parsed {
                    Ok(r) => t.repeat = Some(r),
                    Err(m) => problems.push(Problem::new(name, a.at, m)),
                }
            }
            "data-bind-class-toggle" => {
                for part in a.value.split(';').map(str::trim).filter(|p| !p.is_empty()) {
                    match part.split_once(':') {
                        Some((class, cond)) => match template(cond.trim()) {
                            Ok(c) => t.toggles.push((class.trim().to_string(), c)),
                            Err(m) => problems.push(Problem::new(name, a.at, m)),
                        },
                        None => problems.push(Problem::new(
                            name,
                            a.at,
                            format!("`{part}`: `class:{{{{condition}}}}`"),
                        )),
                    }
                }
            }
            n if n.starts_with("data-bind-style-") => {
                let property = &n["data-bind-style-".len()..];
                match template(&a.value) {
                    Ok(v) => {
                        // Tried with every `{{ }}` as 0: the property and
                        // the shape of its value are known now.
                        let probe: String = v
                            .pieces
                            .iter()
                            .map(|p| match p {
                                expr::Piece::Text(s) => s.clone(),
                                expr::Piece::Expr(_) => "0".into(),
                            })
                            .collect();
                        let mut s = Style::column();
                        let mut late = Late::default();
                        let vars = HashMap::new();
                        let cx = Context { viewport: (1280.0, 720.0), vars: &vars };
                        match css::apply(&mut s, &mut late, State::Always, property, &probe, &cx) {
                            Err(m) if m.contains("not a property") || m.contains("is not read") => {
                                problems.push(Problem::new(name, a.at, m))
                            }
                            _ => t.styles.push((property.to_string(), v, a.at)),
                        }
                    }
                    Err(m) => problems.push(Problem::new(name, a.at, m)),
                }
            }
            n if n.starts_with("on") => problems.push(script(name, a.at)),
            n if n.starts_with("data-bind-") => problems.push(Problem::new(
                name,
                a.at,
                format!("`{n}` is not a binding: data-bind-value, -if, -for, -class-toggle, -style-<property>"),
            )),
            n => problems.push(Problem::new(
                name,
                a.at,
                format!("`{n}` is not an attribute this UI reads"),
            )),
        }
    }
    let looped = in_loop || t.repeat.is_some();
    if let Some(id) = &t.id {
        if !looped {
            if let Some(first) = ids.insert(id.clone(), e.at) {
                problems.push(Problem::new(
                    name,
                    e.at,
                    format!(
                        "`{id}` is the id of another element too (line {})",
                        first.line
                    ),
                ));
            }
        }
    }

    // What is inside: words only, or elements (and words between them).
    let all_text = e.children.iter().all(|c| matches!(c, Content::Text(..)));
    if all_text {
        let words: String = e
            .children
            .iter()
            .filter_map(|c| match c {
                Content::Text(s, _) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        let words = words.trim();
        if value.is_some() && !words.is_empty() {
            problems.push(Problem::new(
                name,
                e.at,
                "both `data-bind-value` and words inside: one or the other",
            ));
        }
        t.text = match value {
            Some(v) => Some(v),
            None if !words.is_empty() => match Template::parse(words) {
                Ok(v) => Some(v),
                Err(m) => {
                    problems.push(Problem::new(name, e.at, m));
                    None
                }
            },
            None => None,
        };
        if matches!(t.kind, Kind::Field { .. }) && t.text.is_some() {
            problems.push(Problem::new(
                name,
                e.at,
                "an `<input>` holds nothing: `value=\"…\"`",
            ));
        }
    } else {
        if value.is_some() {
            problems.push(Problem::new(
                name,
                e.at,
                "`data-bind-value` on an element with elements inside: put it on a `<span>` of its own",
            ));
        }
        let last = e.children.len().saturating_sub(1);
        for (i, c) in e.children.iter().enumerate() {
            match c {
                Content::Element(child) => t.kids.push(TKid::Elem(Box::new(compile(
                    name, child, looped, ids, problems,
                )))),
                Content::Text(s, at) => {
                    let mut s = s.as_str();
                    if i == 0 {
                        s = s.trim_start();
                    }
                    if i == last {
                        s = s.trim_end();
                    }
                    match Template::parse(s) {
                        Ok(v) => t.kids.push(TKid::Text(v)),
                        Err(m) => problems.push(Problem::new(name, *at, m)),
                    }
                }
            }
        }
    }
    t
}

#[cfg(test)]
mod tests;
