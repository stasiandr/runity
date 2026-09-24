//! Docking: panels as tabs in stacks, and a stack divided in two — and
//! each half again — by dropping a tab on one of its edges.
//!
//! Unity's and Godot's model, at the grain an editor of this size needs.
//! The window has three dock areas — left, right, under the view — around
//! the Scene view, which stays in the middle and is not a tab. Each area is
//! a small binary tree ([`Tree`]): a leaf is a stack of tabs with the
//! active panel's content under them, a split is two areas one over the
//! other or side by side, with a bar between them to drag. A tab dropped
//! on the middle of a stack joins it; dropped on one of its four edges, it
//! splits that stack there. A stack left without tabs goes, and its
//! sibling takes the room.
//!
//! The tree is data: the layout file writes it as text a person reads
//! ([`Arrangement::write`]), and a layout preset is one. The nodes are
//! built from it again whenever its shape changes. A panel is only
//! content: it moves whole (`Ui::move_to`) into whatever stack holds it,
//! so its scroll, its search text and its focus go with it.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use scrap_ui::{Event, NodeId, Rect, Style, Ui};

use crate::theme::*;

/// The panels a dock can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Panel {
    Hierarchy,
    Inspector,
    Project,
    Console,
    History,
    Git,
    Settings,
    Profiler,
    Animation,
    Screens,
    Animator,
    Dialogues,
    /// The person's own settings, in every project: a window of its own
    /// when opened (⌘,), never in a dock unless someone puts it there.
    Preferences,
}

impl Panel {
    pub const ALL: [Panel; 13] = [
        Panel::Hierarchy,
        Panel::Inspector,
        Panel::Project,
        Panel::Console,
        Panel::History,
        Panel::Git,
        Panel::Settings,
        Panel::Profiler,
        Panel::Animation,
        Panel::Screens,
        Panel::Animator,
        Panel::Dialogues,
        Panel::Preferences,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Panel::Hierarchy => "hierarchy",
            Panel::Inspector => "inspector",
            Panel::Project => "project",
            Panel::Console => "console",
            Panel::History => "history",
            Panel::Git => "git",
            // The project's settings; the name is the one layouts saved
            // before Preferences split off, so they still load.
            Panel::Settings => "settings",
            Panel::Profiler => "profiler",
            Panel::Animation => "animation",
            Panel::Screens => "screens",
            Panel::Animator => "animator",
            Panel::Dialogues => "dialogues",
            Panel::Preferences => "preferences",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Panel::Hierarchy => "Hierarchy",
            Panel::Inspector => "Inspector",
            Panel::Project => "Project",
            Panel::Console => "Console",
            Panel::History => "History",
            Panel::Git => "Git",
            Panel::Settings => "Project Settings",
            Panel::Profiler => "Profiler",
            Panel::Animation => "Animation",
            Panel::Screens => "UI Builder",
            Panel::Animator => "Animator",
            Panel::Dialogues => "Dialogues",
            Panel::Preferences => "Preferences",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Panel::Hierarchy => "list-tree",
            Panel::Inspector => "sliders-horizontal",
            Panel::Project => "folder",
            Panel::Console => "terminal",
            Panel::History => "undo-2",
            Panel::Git => "layers-2",
            Panel::Settings => "settings",
            Panel::Profiler => "sliders-horizontal",
            Panel::Animation => "play",
            Panel::Screens => "layout-dashboard",
            Panel::Animator => "route",
            Panel::Dialogues => "type",
            Panel::Preferences => "sliders-horizontal",
        }
    }

    /// Whether the padlock in the tab strip means something for this
    /// panel. Where it does not, the strip shows none rather than one that
    /// does nothing.
    pub fn has_lock(self) -> bool {
        self == Panel::Inspector
    }

    pub fn from_name(name: &str) -> Option<Panel> {
        Panel::ALL.into_iter().find(|p| p.name() == name)
    }

    /// Whether the panel lives in a window of its own rather than a dock:
    /// out of the docks until opened, and closed again when its window is.
    pub fn floats(self) -> bool {
        self == Panel::Preferences
    }
}

/// A dock area: a stack of tabs, or two areas.
#[derive(Debug, Clone, PartialEq)]
pub enum Tree {
    /// Tabs, `active` the one on top.
    Stack {
        tabs: Vec<Panel>,
        active: Option<Panel>,
    },
    /// Two areas side by side (`across`) or one over the other, the first
    /// taking `ratio` of the room.
    Split {
        across: bool,
        ratio: f32,
        a: Box<Tree>,
        b: Box<Tree>,
    },
}

/// Where on a stack a dragged tab is let go: into its tabs, or beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

impl Tree {
    /// Tabs, the first on top.
    pub fn stack(tabs: &[Panel]) -> Tree {
        Tree::Stack {
            tabs: tabs.to_vec(),
            active: tabs.first().copied(),
        }
    }

    /// Two areas: side by side when `across`, else `a` over `b`.
    pub fn split(across: bool, ratio: f32, a: Tree, b: Tree) -> Tree {
        Tree::Split {
            across,
            ratio,
            a: Box::new(a),
            b: Box::new(b),
        }
    }

    fn is_empty_stack(&self) -> bool {
        matches!(self, Tree::Stack { tabs, .. } if tabs.is_empty())
    }

    #[cfg(test)]
    fn panels(&self, out: &mut Vec<Panel>) {
        match self {
            Tree::Stack { tabs, .. } => out.extend(tabs),
            Tree::Split { a, b, .. } => {
                a.panels(out);
                b.panels(out);
            }
        }
    }

    /// The way to the stack holding `panel`: at each split, 0 for the
    /// first area and 1 for the second.
    fn path_of(&self, panel: Panel) -> Option<Vec<u8>> {
        match self {
            Tree::Stack { tabs, .. } => tabs.contains(&panel).then(Vec::new),
            Tree::Split { a, b, .. } => [a, b].into_iter().enumerate().find_map(|(i, t)| {
                let mut path = t.path_of(panel)?;
                path.insert(0, i as u8);
                Some(path)
            }),
        }
    }

    fn at(&self, path: &[u8]) -> Option<&Tree> {
        match (self, path.split_first()) {
            (_, None) => Some(self),
            (Tree::Split { a, b, .. }, Some((i, rest))) => if *i == 0 { a } else { b }.at(rest),
            (Tree::Stack { .. }, Some(_)) => None,
        }
    }

    fn at_mut(&mut self, path: &[u8]) -> Option<&mut Tree> {
        match (self, path.split_first()) {
            (t, None) => Some(t),
            (Tree::Split { a, b, .. }, Some((i, rest))) => if *i == 0 { a } else { b }.at_mut(rest),
            (Tree::Stack { .. }, Some(_)) => None,
        }
    }

    /// The first stack, going down the first area of every split.
    fn first_stack(&self) -> Vec<u8> {
        match self {
            Tree::Stack { .. } => Vec::new(),
            Tree::Split { a, .. } => {
                let mut path = a.first_stack();
                path.insert(0, 0);
                path
            }
        }
    }

    /// Take a panel out of its stack. The stack stays, empty or not, until
    /// [`Tree::pruned`]: the ways to the others are the same meanwhile.
    fn remove(&mut self, panel: Panel) -> bool {
        match self {
            Tree::Stack { tabs, active } => {
                let Some(at) = tabs.iter().position(|p| *p == panel) else {
                    return false;
                };
                tabs.remove(at);
                if *active == Some(panel) {
                    *active = tabs.get(at.min(tabs.len().saturating_sub(1))).copied();
                }
                true
            }
            Tree::Split { a, b, .. } => a.remove(panel) || b.remove(panel),
        }
    }

    /// Put `panel` into the stack at `path`: among its tabs, on top, or
    /// beside it on an edge, which splits it.
    fn insert(&mut self, path: &[u8], panel: Panel, zone: Zone) {
        let Some(target) = self.at_mut(path) else {
            return;
        };
        if zone == Zone::Center {
            if let Tree::Stack { tabs, active } = target {
                tabs.push(panel);
                *active = Some(panel);
            }
            return;
        }
        let old = std::mem::replace(target, Tree::stack(&[]));
        let new = Tree::stack(&[panel]);
        let across = matches!(zone, Zone::Left | Zone::Right);
        *target = if matches!(zone, Zone::Left | Zone::Top) {
            Tree::split(across, 0.5, new, old)
        } else {
            Tree::split(across, 0.5, old, new)
        };
    }

    /// The tree without its empty stacks: a split with one gives its place
    /// to the other area. The area's own stack stays, empty, to fold away.
    fn pruned(self) -> Tree {
        match self {
            Tree::Split {
                across,
                ratio,
                a,
                b,
            } => {
                let (a, b) = (a.pruned(), b.pruned());
                if a.is_empty_stack() {
                    b
                } else if b.is_empty_stack() {
                    a
                } else {
                    Tree::split(across, ratio, a, b)
                }
            }
            stack => stack,
        }
    }

    /// Drop the panels already seen elsewhere, and keep `active` one of
    /// the tabs.
    fn dedupe(&mut self, seen: &mut Vec<Panel>) {
        match self {
            Tree::Stack { tabs, active } => {
                tabs.retain(|p| {
                    let new = !seen.contains(p);
                    seen.push(*p);
                    new
                });
                if !active.is_some_and(|a| tabs.contains(&a)) {
                    *active = tabs.first().copied();
                }
            }
            Tree::Split { ratio, a, b, .. } => {
                *ratio = ratio.clamp(0.05, 0.95);
                a.dedupe(seen);
                b.dedupe(seen);
            }
        }
    }

    fn write(&self, out: &mut String) {
        match self {
            Tree::Stack { tabs, active } => {
                out.push('[');
                for (i, p) in tabs.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    if *active == Some(*p) {
                        out.push('*');
                    }
                    out.push_str(p.name());
                }
                out.push(']');
            }
            Tree::Split {
                across,
                ratio,
                a,
                b,
            } => {
                let _ = write!(
                    out,
                    "{}({ratio:.3}, ",
                    if *across { "across" } else { "down" }
                );
                a.write(out);
                out.push_str(", ");
                b.write(out);
                out.push(')');
            }
        }
    }
}

/// Where every panel is: the three areas' trees, and the panels closed.
///
/// As text — what the layout file and a saved layout hold:
///
/// ```text
/// (left: down(0.500, [*hierarchy], [*project]), right: [*inspector],
///  lower: [*console, history, git], closed: [])
/// ```
///
/// `[…]` is a stack of tabs, the one on top marked `*`; `down(r, a, b)`
/// is `a` over `b`, `across(r, a, b)` side by side, `a` taking `r` of the
/// room.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrangement {
    /// Left, right, under the view.
    pub regions: [Tree; 3],
    pub closed: Vec<Panel>,
}

impl Arrangement {
    /// Unity's Default: the Hierarchy left, the Inspector right, the rest
    /// under the view.
    pub fn default_layout() -> Self {
        Self {
            regions: [
                Tree::stack(&[Panel::Hierarchy]),
                Tree::stack(&[Panel::Inspector]),
                Tree::stack(&[
                    Panel::Project,
                    Panel::Console,
                    Panel::History,
                    Panel::Git,
                    Panel::Animation,
                    Panel::Animator,
                    Panel::Dialogues,
                    Panel::Screens,
                    Panel::Settings,
                    Panel::Profiler,
                ]),
            ],
            closed: vec![Panel::Preferences],
        }
    }

    /// Unity's Tall: the Hierarchy over the Project in a column of their
    /// own, the Console under the view, the Inspector the window's height.
    pub fn tall() -> Self {
        Self {
            regions: [
                Tree::split(
                    false,
                    0.45,
                    Tree::stack(&[Panel::Hierarchy]),
                    Tree::stack(&[Panel::Project]),
                ),
                Tree::stack(&[Panel::Inspector]),
                Tree::stack(&[
                    Panel::Console,
                    Panel::History,
                    Panel::Git,
                    Panel::Animation,
                    Panel::Animator,
                    Panel::Dialogues,
                    Panel::Screens,
                    Panel::Settings,
                    Panel::Profiler,
                ]),
            ],
            closed: vec![Panel::Preferences],
        }
    }

    /// The format before stacks could split: three areas of tabs,
    /// `hierarchy|inspector|project,console` and the tabs on top
    /// `hierarchy|inspector|console`.
    pub fn from_old(docks: &str, active: &str) -> Option<Self> {
        let parts: Vec<Vec<Panel>> = docks
            .split('|')
            .map(|d| d.split(',').filter_map(Panel::from_name).collect())
            .collect();
        if parts.len() != 3 {
            return None;
        }
        let on: Vec<Option<Panel>> = active.split('|').map(Panel::from_name).collect();
        let stack = |i: usize| {
            let mut t = Tree::stack(&parts[i]);
            if let (Tree::Stack { tabs, active }, Some(Some(p))) = (&mut t, on.get(i)) {
                if tabs.contains(p) {
                    *active = Some(*p);
                }
            }
            t
        };
        Some(Self {
            regions: [stack(0), stack(1), stack(2)],
            closed: Vec::new(),
        })
    }

    /// Every panel once: a repeat goes, one the text forgot (a panel newer
    /// than the file) joins the tabs under the view, and empty stacks go.
    pub fn normalize(&mut self) {
        let mut seen = Vec::new();
        for region in &mut self.regions {
            region.dedupe(&mut seen);
        }
        self.closed.retain(|p| !seen.contains(p));
        self.closed.dedup();
        for panel in Panel::ALL {
            // One that lives in a window waits closed, out of the docks.
            if panel.floats() && !seen.contains(&panel) && !self.closed.contains(&panel) {
                self.closed.push(panel);
                continue;
            }
            if !seen.contains(&panel) && !self.closed.contains(&panel) {
                let path = self.regions[2].first_stack();
                self.regions[2].insert(&path, panel, Zone::Center);
                // Joining is not coming on top.
                if let Some(Tree::Stack { tabs, active }) = self.regions[2].at_mut(&path) {
                    if tabs.len() > 1 {
                        *active = tabs.first().copied();
                    }
                }
            }
        }
        for region in &mut self.regions {
            *region = std::mem::replace(region, Tree::stack(&[])).pruned();
        }
    }

    /// As the layout file writes it (see [`Arrangement`]).
    pub fn write(&self) -> String {
        let mut out = String::from("(");
        for (i, key) in ["left", "right", "lower"].into_iter().enumerate() {
            let _ = write!(out, "{key}: ");
            self.regions[i].write(&mut out);
            out.push_str(", ");
        }
        out.push_str("closed: [");
        let names: Vec<&str> = self.closed.iter().map(|p| p.name()).collect();
        out.push_str(&names.join(", "));
        out.push_str("])");
        out
    }

    /// Read what [`Arrangement::write`] wrote. Panels it does not know
    /// (from a newer editor) are skipped rather than failing the file.
    pub fn read(text: &str) -> Option<Self> {
        let mut r = Reader {
            s: text.as_bytes(),
            at: 0,
        };
        r.expect(b'(')?;
        let mut this = Self {
            regions: [Tree::stack(&[]), Tree::stack(&[]), Tree::stack(&[])],
            closed: Vec::new(),
        };
        loop {
            r.ws();
            if r.eat(b')') {
                break;
            }
            let key = r.word()?;
            r.expect(b':')?;
            match key.as_str() {
                "left" => this.regions[0] = r.tree()?,
                "right" => this.regions[1] = r.tree()?,
                "lower" => this.regions[2] = r.tree()?,
                "closed" => {
                    if let Tree::Stack { tabs, .. } = r.tree()? {
                        this.closed = tabs;
                    }
                }
                _ => return None,
            }
            r.ws();
            r.eat(b',');
        }
        Some(this)
    }
}

/// A cursor over the layout text.
struct Reader<'a> {
    s: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn ws(&mut self) {
        while self.s.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        let yes = self.s.get(self.at) == Some(&c);
        if yes {
            self.at += 1;
        }
        yes
    }

    fn expect(&mut self, c: u8) -> Option<()> {
        self.eat(c).then_some(())
    }

    fn word(&mut self) -> Option<String> {
        self.ws();
        let start = self.at;
        while self
            .s
            .get(self.at)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'.')
        {
            self.at += 1;
        }
        (self.at > start).then(|| String::from_utf8_lossy(&self.s[start..self.at]).into_owned())
    }

    fn tree(&mut self) -> Option<Tree> {
        if self.eat(b'[') {
            let (mut tabs, mut active) = (Vec::new(), None);
            loop {
                if self.eat(b']') {
                    break;
                }
                let on = self.eat(b'*');
                let name = self.word()?;
                if let Some(p) = Panel::from_name(&name) {
                    tabs.push(p);
                    if on {
                        active = Some(p);
                    }
                }
                self.eat(b',');
            }
            let active = active.or(tabs.first().copied());
            return Some(Tree::Stack { tabs, active });
        }
        let across = match self.word()?.as_str() {
            "across" => true,
            "down" => false,
            _ => return None,
        };
        self.expect(b'(')?;
        let ratio: f32 = self.word()?.parse().ok()?;
        self.expect(b',')?;
        let a = self.tree()?;
        self.expect(b',')?;
        let b = self.tree()?;
        self.eat(b',');
        self.expect(b')')?;
        Some(Tree::split(across, ratio, a, b))
    }
}

/// A stack as built: its card, its strip of tabs with the lock and the ⋮
/// at the end, and where it is in the tree.
struct StackView {
    region: usize,
    path: Vec<u8>,
    card: NodeId,
    strip: NodeId,
    tabs: Vec<(Panel, NodeId)>,
    lock: NodeId,
    more: NodeId,
    empty: NodeId,
}

/// A split as built: its node, the bar between its areas, and the areas.
struct SplitView {
    region: usize,
    path: Vec<u8>,
    node: NodeId,
    bar: NodeId,
    across: bool,
    halves: [NodeId; 2],
}

/// What a dock event asked for, for the studio to act on.
pub enum Docked {
    /// A tab was clicked, dragged or dropped, or a bar dragged: panels may
    /// have come on top or moved, and want bringing up to date.
    Handled,
    /// A tab was right-clicked or its stack's ⋮ clicked: the panel's menu,
    /// at this point.
    Menu(Panel, f32, f32),
    /// A tab was double-clicked: its stack over the whole window, or back.
    Maximize(Panel),
    /// The padlock of the panel on top was clicked.
    Lock(Panel),
}

pub struct Docks {
    arrangement: Arrangement,
    /// The three areas' nodes, which the studio sizes and folds.
    slots: [NodeId; 3],
    /// A hidden node where closed panels wait.
    shelf: NodeId,
    /// What was built for each area, to take down when the shape changes.
    built: Vec<NodeId>,
    stacks: Vec<StackView>,
    splits: Vec<SplitView>,
    roots: HashMap<Panel, NodeId>,
    /// A tab is being dragged: an empty area shows, to be dropped on.
    dragging: bool,
    /// Where a dragged tab would land, lit up.
    drop_zone: NodeId,
    /// The panel whose stack has its area to itself.
    zoom: Option<Panel>,
    /// Panels whose padlock is shut.
    locked: HashSet<Panel>,
    /// Where a panel taken out goes back to: its area, and a panel that
    /// was a tab beside it.
    homes: HashMap<Panel, (usize, Option<Panel>)>,
}

fn tab_style(on: bool) -> Style {
    let s = Style::row()
        .height(24.0)
        .padding_x(SPACE_3)
        .gap(6.0)
        .center_items()
        .radius(6.0)
        .clickable()
        .draggable();
    if on {
        s.background(ACCENT_900).hover(ACCENT_900)
    } else {
        s.hover(HOVER)
    }
}

/// The lock and the ⋮ at the end of a strip: smaller than the toolbar's,
/// as quiet as the tabs.
fn header_style(on: bool) -> Style {
    let s = Style::row()
        .size(22.0, 22.0)
        .fixed()
        .center()
        .radius(6.0)
        .clickable();
    if on {
        s.background(ACCENT_HOVER).hover(ACCENT.alpha(18))
    } else {
        s.hover(HOVER).pressed(PRESSED)
    }
}

/// With a flex share of `grow`: a split's areas divide its room so.
fn grown(mut s: Style, grow: f32) -> Style {
    s.layout.flex_grow = grow;
    s
}

/// A stack's or bar's place, for its node's name: the area, then `a` or
/// `b` at each split — `stack 0` is the left area whole, `stack 0b` the
/// lower half of it split.
fn place(region: usize, path: &[u8]) -> String {
    let mut s = region.to_string();
    s.extend(path.iter().map(|i| if *i == 0 { 'a' } else { 'b' }));
    s
}

impl Docks {
    /// Docks in `slots` (left, right, under the view), holding the panels
    /// as `arrangement` says, each panel's content being `roots[panel]`.
    pub fn new(
        ui: &mut Ui,
        slots: [NodeId; 3],
        roots: HashMap<Panel, NodeId>,
        arrangement: Arrangement,
    ) -> Self {
        let top = ui.root();
        let shelf = ui.add(top, Style::column().hidden());
        for (i, slot) in slots.iter().enumerate() {
            ui.set_name(*slot, format!("dock {i}"));
        }
        for root in roots.values() {
            ui.move_to(*root, shelf);
        }
        let drop_zone = ui.add(
            top,
            Style::column()
                .absolute(0.0, 0.0)
                .size(0.0, 0.0)
                .radius(RADIUS_MD)
                .background(ACCENT.alpha(22))
                .border(2.0, ACCENT)
                .hidden(),
        );
        ui.set_layer(drop_zone, true);
        ui.set_name(drop_zone, "drop zone");
        let mut this = Self {
            arrangement: Arrangement::default_layout(),
            slots,
            shelf,
            built: Vec::new(),
            stacks: Vec::new(),
            splits: Vec::new(),
            roots,
            dragging: false,
            drop_zone,
            zoom: None,
            locked: HashSet::new(),
            homes: HashMap::new(),
        };
        this.set_arrangement(ui, arrangement);
        this
    }

    /// Where every panel is.
    pub fn arrangement(&self) -> &Arrangement {
        &self.arrangement
    }

    /// Put every panel where `arrangement` says. A panel in a window of
    /// its own should be docked first: this does not know about windows.
    pub fn set_arrangement(&mut self, ui: &mut Ui, mut arrangement: Arrangement) {
        arrangement.normalize();
        self.arrangement = arrangement;
        self.zoom = None;
        self.rebuild(ui);
    }

    /// Build the areas' nodes from the tree again.
    fn rebuild(&mut self, ui: &mut Ui) {
        // The panels out of the old nodes first: removing those removes
        // everything under them.
        for view in &self.stacks {
            for (p, _) in &view.tabs {
                ui.move_to(self.roots[p], self.shelf);
            }
        }
        for node in self.built.drain(..) {
            ui.remove(node);
        }
        self.stacks.clear();
        self.splits.clear();
        for region in 0..3 {
            let tree = self.arrangement.regions[region].clone();
            let slot = self.slots[region];
            let outer = self.build(ui, slot, &tree, region, &mut Vec::new(), 1.0);
            self.built.push(outer);
        }
        self.apply_zoom(ui);
    }

    fn build(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        tree: &Tree,
        region: usize,
        path: &mut Vec<u8>,
        grow: f32,
    ) -> NodeId {
        match tree {
            Tree::Stack { tabs, .. } => {
                let card = ui.add(
                    parent,
                    grown(
                        Style::column()
                            .full()
                            .fill()
                            .background(SURFACE)
                            .radius(RADIUS_MD)
                            .clip()
                            .clickable(),
                        grow,
                    ),
                );
                ui.set_name(card, format!("stack {}", place(region, path)));
                let strip = ui.add(
                    card,
                    Style::row()
                        .auto_height()
                        .min_height(32.0)
                        .fixed()
                        .full_width()
                        .padding_x(SPACE_2)
                        .padding_y(4.0)
                        .gap(SPACE_1)
                        .center_items(),
                );
                // More tabs than room: they wrap onto another row rather
                // than run under the lock and the ⋮.
                let row = ui.add(
                    strip,
                    Style::row().fill().gap(SPACE_1).wrap().center_items(),
                );
                let header = ui.add(strip, Style::row().fixed().gap(2.0).center_items());
                let lock = ui.add(header, header_style(false));
                icon(ui, lock, "lock-open", LABEL);
                let more = ui.add(header, header_style(false));
                icon(ui, more, "ellipsis-vertical", LABEL);
                let body = ui.add(card, Style::column().fill().full_width());
                let empty = ui.add(body, Style::column().fill().full_width().center().hidden());
                ui.add_text(
                    empty,
                    Style::default()
                        .text_size(12.0)
                        .text_color(TEXT.alpha(40))
                        .nowrap(),
                    "Drag a tab here",
                );
                let mut views = Vec::new();
                for panel in tabs {
                    let tab = ui.add(row, tab_style(false));
                    ui.set_name(tab, format!("tab {}", panel.name()));
                    icon(ui, tab, panel.icon(), LABEL);
                    ui.add_text(tab, text().text_color(LABEL), panel.label());
                    views.push((*panel, tab));
                    ui.move_to(self.roots[panel], body);
                }
                self.stacks.push(StackView {
                    region,
                    path: path.clone(),
                    card,
                    strip,
                    tabs: views,
                    lock,
                    more,
                    empty,
                });
                self.show_active(ui, self.stacks.len() - 1);
                card
            }
            Tree::Split {
                across,
                ratio,
                a,
                b,
            } => {
                let base = if *across {
                    Style::row()
                } else {
                    Style::column()
                };
                let node = ui.add(parent, grown(base.full().fill().gap(3.0), grow));
                path.push(0);
                let first = self.build(ui, node, a, region, path, *ratio);
                path.pop();
                let bar = if *across {
                    Style::row().width(4.0).full_height().fixed()
                } else {
                    Style::row().height(4.0).full_width().fixed()
                };
                let bar = ui.add(node, bar.radius(2.0).hover(ACCENT.alpha(30)).draggable());
                ui.set_name(bar, format!("divider {}", place(region, path)));
                path.push(1);
                let second = self.build(ui, node, b, region, path, 1.0 - ratio);
                path.pop();
                self.splits.push(SplitView {
                    region,
                    path: path.clone(),
                    node,
                    bar,
                    across: *across,
                    halves: [first, second],
                });
                node
            }
        }
    }

    /// The panel on top of built stack `i`.
    fn active_of(&self, i: usize) -> Option<Panel> {
        let view = &self.stacks[i];
        match self.arrangement.regions[view.region].at(&view.path) {
            Some(Tree::Stack { active, .. }) => *active,
            _ => None,
        }
    }

    /// Show stack `i`'s panel on top and hide the rest; its tab lit, its
    /// strip's lock and ⋮ that panel's.
    fn show_active(&mut self, ui: &mut Ui, i: usize) {
        let active = self.active_of(i);
        let view = &self.stacks[i];
        for (p, tab) in &view.tabs {
            let on = Some(*p) == active;
            ui.set_style(*tab, tab_style(on));
            let kids = ui.children(*tab);
            ui.restyle(kids[0], |s| s.text_color(if on { ACCENT } else { LABEL }));
            ui.restyle(kids[1], |s| s.text_color(if on { TEXT } else { LABEL }));
            ui.restyle(self.roots[p], |s| if on { s.shown() } else { s.hidden() });
        }
        let empty = view.tabs.is_empty();
        ui.restyle(view.empty, |s| if empty { s.shown() } else { s.hidden() });
        self.header(ui, i);
    }

    /// The lock and the ⋮ at the end of stack `i`'s strip, named after the
    /// panel on top — `inspector lock`, `console more` — so a test or an
    /// agent finds a panel's header by the panel.
    fn header(&self, ui: &mut Ui, i: usize) {
        let view = &self.stacks[i];
        let active = self.active_of(i);
        let owner = active.map_or_else(
            || format!("stack {}", place(view.region, &view.path)),
            |p| p.name().to_string(),
        );
        ui.set_name(view.lock, format!("{owner} lock"));
        ui.set_name(view.more, format!("{owner} more"));
        let has_lock = active.is_some_and(Panel::has_lock);
        let on = active.is_some_and(|p| self.locked.contains(&p));
        ui.set_style(
            view.lock,
            if has_lock {
                header_style(on)
            } else {
                header_style(false).hidden()
            },
        );
        if let Some(glyph) = ui.children(view.lock).first().copied() {
            ui.set_icon(glyph, if on { "lock" } else { "lock-open" });
            ui.restyle(glyph, |s| s.text_color(if on { ACCENT } else { LABEL }));
        }
        ui.restyle(view.more, |s| {
            if active.is_some() {
                s.shown()
            } else {
                s.hidden()
            }
        });
    }

    fn stack_of(&self, panel: Panel) -> Option<usize> {
        self.stacks
            .iter()
            .position(|v| v.tabs.iter().any(|(p, _)| *p == panel))
    }

    /// Which area a panel is in, and the way to its stack there.
    fn place_of(&self, panel: Panel) -> Option<(usize, Vec<u8>)> {
        (0..3).find_map(|r| {
            self.arrangement.regions[r]
                .path_of(panel)
                .map(|path| (r, path))
        })
    }

    /// Which area a panel is in: 0 left, 1 right, 2 under the view.
    pub fn region_of(&self, panel: Panel) -> Option<usize> {
        self.place_of(panel).map(|(r, _)| r)
    }

    /// Bring a panel's tab on top in its stack.
    pub fn activate(&mut self, ui: &mut Ui, panel: Panel) {
        let Some((region, path)) = self.place_of(panel) else {
            return;
        };
        if let Some(Tree::Stack { active, .. }) = self.arrangement.regions[region].at_mut(&path) {
            *active = Some(panel);
        }
        if let Some(i) = self.stack_of(panel) {
            self.show_active(ui, i);
        }
    }

    /// Whether a panel is on top in its stack (its area may still be
    /// hidden by the Window menu).
    pub fn is_active(&self, panel: Panel) -> bool {
        self.place_of(panel).is_some_and(|(r, path)| {
            matches!(self.arrangement.regions[r].at(&path),
                Some(Tree::Stack { active, .. }) if *active == Some(panel))
        })
    }

    /// Whether area `i` holds no panel: the studio folds it away, and the
    /// view takes its room, until a tab is dragged.
    pub fn is_empty(&self, i: usize) -> bool {
        self.arrangement.regions[i].is_empty_stack()
    }

    pub fn is_closed(&self, panel: Panel) -> bool {
        self.arrangement.closed.contains(&panel)
    }

    /// Whether a tab is being dragged.
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    /// The panel on top of the stack at a point of the window.
    pub fn stack_at(&self, ui: &Ui, x: f32, y: f32) -> Option<Panel> {
        let i = self
            .stacks
            .iter()
            .position(|v| ui.is_shown(v.card) && ui.rect(v.card).contains(x, y))?;
        self.active_of(i)
    }

    /// Whether a panel shows: on top of its stack, or in a window of its
    /// own (neither docked nor closed).
    pub fn is_showing(&self, panel: Panel) -> bool {
        self.is_active(panel) || (self.place_of(panel).is_none() && !self.is_closed(panel))
    }

    /// Take a panel out of its stack, tab and all: it is going to a window
    /// of its own. Its content waits hidden until the caller moves it.
    pub fn take(&mut self, ui: &mut Ui, panel: Panel) -> Option<NodeId> {
        // A closed panel waits on the shelf: it goes as it is.
        if self.is_closed(panel) {
            self.arrangement.closed.retain(|p| *p != panel);
            return Some(self.roots[&panel]);
        }
        self.lift(panel)?;
        self.rebuild(ui);
        Some(self.roots[&panel])
    }

    /// A panel back on the shelf, closed, from wherever it was taken to
    /// (its window closed).
    pub fn put_away(&mut self, ui: &mut Ui, panel: Panel) {
        if self.place_of(panel).is_some() {
            self.close(ui, panel);
            return;
        }
        ui.move_to(self.roots[&panel], self.shelf);
        if !self.arrangement.closed.contains(&panel) {
            self.arrangement.closed.push(panel);
        }
    }

    /// Close a panel's tab: its content waits hidden until the Window menu
    /// opens it again.
    pub fn close(&mut self, ui: &mut Ui, panel: Panel) {
        if self.lift(panel).is_some() {
            self.arrangement.closed.push(panel);
            self.rebuild(ui);
        }
    }

    /// Out of the tree, remembering where it was.
    fn lift(&mut self, panel: Panel) -> Option<()> {
        let (region, path) = self.place_of(panel)?;
        let beside = match self.arrangement.regions[region].at(&path) {
            Some(Tree::Stack { tabs, .. }) => tabs.iter().copied().find(|p| *p != panel),
            _ => None,
        };
        self.homes.insert(panel, (region, beside));
        let tree = &mut self.arrangement.regions[region];
        tree.remove(panel);
        *tree = std::mem::replace(tree, Tree::stack(&[])).pruned();
        if self.zoom == Some(panel) {
            self.zoom = None;
        }
        Some(())
    }

    /// Put a panel taken out or closed back where it was — beside the tab
    /// it was beside, or in its area — on top there.
    pub fn give_back(&mut self, ui: &mut Ui, panel: Panel) {
        if self.place_of(panel).is_some() {
            return;
        }
        self.arrangement.closed.retain(|p| *p != panel);
        let (region, beside) = self.homes.get(&panel).copied().unwrap_or((2, None));
        let (region, path) = beside
            .and_then(|p| self.place_of(p))
            .unwrap_or_else(|| (region, self.arrangement.regions[region].first_stack()));
        self.arrangement.regions[region].insert(&path, panel, Zone::Center);
        self.rebuild(ui);
    }

    /// Give the stack holding `panel` its whole area, or, with `None`,
    /// every stack its own room back. The studio gives the area the window.
    pub fn set_zoom(&mut self, ui: &mut Ui, panel: Option<Panel>) {
        self.zoom = panel;
        self.apply_zoom(ui);
    }

    fn apply_zoom(&mut self, ui: &mut Ui) {
        let place = self.zoom.and_then(|p| self.place_of(p));
        for view in &self.splits {
            let keep = place.as_ref().and_then(|(region, path)| {
                (view.region == *region && path.starts_with(&view.path))
                    .then(|| path[view.path.len()] as usize)
            });
            let shown = [
                keep.is_none_or(|k| k == 0),
                keep.is_none(),
                keep.is_none_or(|k| k == 1),
            ];
            // Shares under one leave the rest of the room empty: the half
            // kept alone takes all of it.
            let ratio = match self.arrangement.regions[view.region].at(&view.path) {
                Some(Tree::Split { ratio, .. }) if keep.is_none() => *ratio,
                _ => 1.0,
            };
            let grow = [ratio, if keep.is_none() { 1.0 - ratio } else { 1.0 }];
            for (i, (node, on)) in [view.halves[0], view.bar, view.halves[1]]
                .into_iter()
                .zip(shown)
                .enumerate()
            {
                ui.restyle(node, |s| {
                    let s = if on { s.shown() } else { s.hidden() };
                    match i {
                        0 => grown(s, grow[0]),
                        2 => grown(s, grow[1]),
                        _ => s,
                    }
                });
            }
        }
    }

    /// Show a panel's padlock shut or open.
    pub fn set_locked(&mut self, ui: &mut Ui, panel: Panel, on: bool) {
        let changed = if on {
            self.locked.insert(panel)
        } else {
            self.locked.remove(&panel)
        };
        if changed {
            if let Some(i) = self.stack_of(panel) {
                self.header(ui, i);
            }
        }
    }

    /// A tab, a stack's lock or ⋮, or a bar between areas, clicked or
    /// dragged. `Some` when the event was the docks'.
    pub fn event(&mut self, ui: &mut Ui, node: NodeId, event: &Event) -> Option<Docked> {
        if let Some(i) = self.splits.iter().position(|s| s.bar == node) {
            if let Event::Drag { dx, dy, .. } = event {
                self.drag_bar(ui, i, *dx, *dy);
            }
            return Some(Docked::Handled);
        }
        if let Some(i) = self
            .stacks
            .iter()
            .position(|v| v.lock == node || v.more == node)
        {
            let Event::Click { .. } = event else {
                return Some(Docked::Handled);
            };
            let panel = self.active_of(i)?;
            if node == self.stacks[i].lock {
                return Some(Docked::Lock(panel));
            }
            let r = ui.rect(node);
            return Some(Docked::Menu(
                panel,
                r.x + r.width - 236.0,
                r.y + r.height + 4.0,
            ));
        }
        let panel = self
            .stacks
            .iter()
            .flat_map(|v| v.tabs.iter())
            .find(|(_, t)| *t == node)
            .map(|(p, _)| *p)?;
        match event {
            Event::Click {
                button: scrap::input::MouseButton::Right,
                ..
            } => {
                let (x, y) = ui.pointer();
                Some(Docked::Menu(panel, x, y))
            }
            Event::Click { count: 2, .. } => {
                self.activate(ui, panel);
                Some(Docked::Maximize(panel))
            }
            Event::Click { .. } => {
                self.activate(ui, panel);
                Some(Docked::Handled)
            }
            Event::Drag { .. } => {
                self.dragging = true;
                let target = self.drop_target(ui);
                self.show_drop(ui, target);
                Some(Docked::Handled)
            }
            Event::DragEnd { .. } => {
                self.dragging = false;
                let target = self.drop_target(ui);
                self.show_drop(ui, None);
                if let Some((i, zone)) = target {
                    self.drop(ui, panel, i, zone);
                }
                Some(Docked::Handled)
            }
            _ => Some(Docked::Handled),
        }
    }

    /// The stack under the pointer and where on it: over its strip or its
    /// middle, into its tabs; near an edge, beside it.
    fn drop_target(&self, ui: &Ui) -> Option<(usize, Zone)> {
        let (x, y) = ui.pointer();
        let i = self
            .stacks
            .iter()
            .position(|v| ui.is_shown(v.card) && ui.rect(v.card).contains(x, y))?;
        let view = &self.stacks[i];
        let r = ui.rect(view.card);
        let strip = ui.rect(view.strip);
        if view.tabs.is_empty() || y <= strip.y + strip.height {
            return Some((i, Zone::Center));
        }
        let fx = (x - r.x) / r.width.max(1.0);
        let fy = (y - r.y) / r.height.max(1.0);
        let (near, zone) = [
            (fx, Zone::Left),
            (1.0 - fx, Zone::Right),
            (fy, Zone::Top),
            (1.0 - fy, Zone::Bottom),
        ]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
        Some((i, if near < 0.3 { zone } else { Zone::Center }))
    }

    /// Light up where a dragged tab would land: the whole stack, or the
    /// half of it the new stack would take.
    fn show_drop(&self, ui: &mut Ui, target: Option<(usize, Zone)>) {
        let Some((i, zone)) = target else {
            ui.restyle(self.drop_zone, |s| s.hidden());
            return;
        };
        let r = ui.rect(self.stacks[i].card);
        let (w, h) = (r.width / 2.0, r.height / 2.0);
        let z = match zone {
            Zone::Center => r,
            Zone::Left => Rect { width: w, ..r },
            Zone::Right => Rect {
                x: r.x + w,
                width: w,
                ..r
            },
            Zone::Top => Rect { height: h, ..r },
            Zone::Bottom => Rect {
                y: r.y + h,
                height: h,
                ..r
            },
        };
        ui.restyle(self.drop_zone, |s| {
            s.absolute(z.x, z.y).size(z.width, z.height).shown()
        });
    }

    /// A tab let go on stack `i`.
    fn drop(&mut self, ui: &mut Ui, panel: Panel, i: usize, zone: Zone) {
        let (region, path) = (self.stacks[i].region, self.stacks[i].path.clone());
        let Some((from, from_path)) = self.place_of(panel) else {
            return;
        };
        let own = from == region && from_path == path;
        // Into its own stack, or beside itself when it is the only tab:
        // nothing moves.
        if own && (zone == Zone::Center || self.stacks[i].tabs.len() == 1) {
            return;
        }
        // Out first, in second, and only then are empty stacks taken away:
        // the way to the target is still the same when it is used.
        self.arrangement.regions[from].remove(panel);
        self.arrangement.regions[region].insert(&path, panel, zone);
        for r in [from, region] {
            let tree = &mut self.arrangement.regions[r];
            *tree = std::mem::replace(tree, Tree::stack(&[])).pruned();
        }
        self.zoom = None;
        self.rebuild(ui);
    }

    /// Move the bar between split `i`'s areas: each keeps room to show.
    fn drag_bar(&mut self, ui: &mut Ui, i: usize, dx: f32, dy: f32) {
        let view = &self.splits[i];
        let r = ui.rect(view.node);
        let (length, delta) = if view.across {
            (r.width, dx)
        } else {
            (r.height, dy)
        };
        let room = (length - 10.0).max(1.0);
        let least = (80.0 / room).min(0.45);
        let halves = view.halves;
        let Some(Tree::Split { ratio, .. }) =
            self.arrangement.regions[view.region].at_mut(&view.path)
        else {
            return;
        };
        *ratio = (*ratio + delta / room).clamp(least, 1.0 - least);
        let r = *ratio;
        ui.restyle(halves[0], |s| grown(s, r));
        ui.restyle(halves[1], |s| grown(s, 1.0 - r));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_arrangement_reads_back_what_it_writes() {
        for a in [Arrangement::default_layout(), Arrangement::tall()] {
            let text = a.write();
            assert_eq!(Arrangement::read(&text), Some(a), "{text}");
        }
        let text = "(left: down(0.450, [*hierarchy], [*project]), right: [*inspector], lower: [*console, history], closed: [git])";
        let a = Arrangement::read(text).unwrap();
        assert_eq!(a.write(), text);
    }

    #[test]
    fn an_edge_splits_a_stack_and_an_emptied_one_goes() {
        let mut a = Arrangement::default_layout();
        a.regions[2].remove(Panel::Project);
        a.regions[0].insert(&[], Panel::Project, Zone::Bottom);
        assert_eq!(
            a.regions[0],
            Tree::split(
                false,
                0.5,
                Tree::stack(&[Panel::Hierarchy]),
                Tree::stack(&[Panel::Project])
            )
        );
        a.regions[0].remove(Panel::Hierarchy);
        let left = std::mem::replace(&mut a.regions[0], Tree::stack(&[])).pruned();
        assert_eq!(left, Tree::stack(&[Panel::Project]));
    }

    #[test]
    fn the_old_format_maps_onto_stacks_and_a_forgotten_panel_joins_the_lower_tabs() {
        let mut a = Arrangement::from_old(
            "hierarchy,console|inspector|project",
            "hierarchy|inspector|",
        )
        .unwrap();
        a.normalize();
        assert_eq!(
            a.regions[0],
            Tree::Stack {
                tabs: vec![Panel::Hierarchy, Panel::Console],
                active: Some(Panel::Hierarchy)
            }
        );
        let mut all = Vec::new();
        for r in &a.regions {
            r.panels(&mut all);
        }
        // All but Preferences, which waits closed for its window.
        assert_eq!(all.len(), Panel::ALL.len() - 1, "{all:?}");
        assert!(!all.contains(&Panel::Preferences));
        assert_eq!(a.closed, vec![Panel::Preferences]);
    }

    /// Layouts written before Preferences split off name the project's
    /// settings `settings`: that is still the Project Settings tab.
    #[test]
    fn an_old_layouts_settings_is_project_settings() {
        let mut a =
            Arrangement::read("(left: [*hierarchy], right: [*inspector], lower: [*settings], closed: [])")
                .unwrap();
        a.normalize();
        assert!(matches!(&a.regions[2], Tree::Stack { active: Some(Panel::Settings), .. }));
        assert_eq!(Panel::Settings.label(), "Project Settings");
        assert!(a.closed.contains(&Panel::Preferences));
    }
}
