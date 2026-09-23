//! Docking: the window's edges hold panels as tabs, and a tab dragged to
//! another edge takes its panel there.
//!
//! Unity's docking, at the grain an editor of this size needs: three docks
//! — left, right, under the view — each a card with a strip of tabs and the
//! active panel's content under it. A panel is only content; which dock it
//! is in, and which tab is on top, is this module's. Moving a panel moves
//! its nodes whole (`Ui::move_to`), so its scroll, its search text and its
//! focus go with it. Where everything is goes into the studio's layout
//! file and comes back next run.

use std::collections::HashMap;

use runity_ui::{Event, NodeId, Style, Ui};

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
}

impl Panel {
    pub const ALL: [Panel; 12] = [
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
    ];

    pub fn name(self) -> &'static str {
        match self {
            Panel::Hierarchy => "hierarchy",
            Panel::Inspector => "inspector",
            Panel::Project => "project",
            Panel::Console => "console",
            Panel::History => "history",
            Panel::Git => "git",
            Panel::Settings => "settings",
            Panel::Profiler => "profiler",
            Panel::Animation => "animation",
            Panel::Screens => "screens",
            Panel::Animator => "animator",
            Panel::Dialogues => "dialogues",
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
            Panel::Settings => "Settings",
            Panel::Profiler => "Profiler",
            Panel::Animation => "Animation",
            Panel::Screens => "UI Builder",
            Panel::Animator => "Animator",
            Panel::Dialogues => "Dialogues",
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
        }
    }

    pub fn from_name(name: &str) -> Option<Panel> {
        Panel::ALL.into_iter().find(|p| p.name() == name)
    }
}

struct Dock {
    card: NodeId,
    strip: NodeId,
    body: NodeId,
    /// The hint an empty dock shows.
    empty: NodeId,
    tabs: Vec<(Panel, NodeId)>,
    active: Option<Panel>,
}

/// What a dock event asked for, for the studio to act on.
pub enum Docked {
    /// A tab was clicked or dragged: panels may have come on top or moved,
    /// and want bringing up to date.
    Handled,
    /// A tab was right-clicked: its menu, at the pointer.
    Menu(Panel),
}

pub struct Docks {
    docks: Vec<Dock>,
    roots: HashMap<Panel, NodeId>,
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

impl Docks {
    /// Docks in `slots` (left, right, bottom), holding the panels as
    /// `layout` says, each panel's content being `roots[panel]`.
    pub fn new(
        ui: &mut Ui,
        slots: [NodeId; 3],
        roots: HashMap<Panel, NodeId>,
        layout: [Vec<Panel>; 3],
    ) -> Self {
        let mut docks = Vec::new();
        for (i, slot) in slots.into_iter().enumerate() {
            let card = ui.add(
                slot,
                Style::column()
                    .full()
                    .background(SURFACE)
                    .radius(RADIUS_MD)
                    .border(1.0, runity_ui::Color::TRANSPARENT)
                    .clip()
                    .clickable(),
            );
            ui.set_name(card, format!("dock {i}"));
            // More tabs than room: they wrap onto another row rather than
            // run off the dock's edge.
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
                    .wrap()
                    .center_items(),
            );
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
            docks.push(Dock {
                card,
                strip,
                body,
                empty,
                tabs: Vec::new(),
                active: None,
            });
        }
        let mut this = Self { docks, roots };
        let mut placed = Vec::new();
        for (i, panels) in layout.iter().enumerate() {
            for panel in panels {
                if !placed.contains(panel) {
                    this.put(ui, *panel, i);
                    placed.push(*panel);
                }
            }
        }
        // Anything the layout forgot goes under the view.
        for panel in Panel::ALL {
            if !placed.contains(&panel) {
                this.put(ui, panel, 2);
            }
        }
        for i in 0..this.docks.len() {
            let first = this.docks[i].tabs.first().map(|(p, _)| *p);
            if let Some(p) = first {
                this.activate(ui, p);
            }
            this.show_empty(ui, i);
        }
        this
    }

    /// Put a panel into dock `i`, as its last tab.
    fn put(&mut self, ui: &mut Ui, panel: Panel, i: usize) {
        let dock = &mut self.docks[i];
        let tab = ui.add(dock.strip, tab_style(false));
        ui.set_name(tab, format!("tab {}", panel.name()));
        icon(ui, tab, panel.icon(), LABEL);
        ui.add_text(tab, text().text_color(LABEL), panel.label());
        dock.tabs.push((panel, tab));
        let root = self.roots[&panel];
        ui.move_to(root, dock.body);
        ui.restyle(root, |s| s.hidden());
    }

    pub fn dock_of(&self, panel: Panel) -> Option<usize> {
        self.docks
            .iter()
            .position(|d| d.tabs.iter().any(|(p, _)| *p == panel))
    }

    /// Bring a panel's tab on top in its dock.
    pub fn activate(&mut self, ui: &mut Ui, panel: Panel) {
        let Some(i) = self.dock_of(panel) else { return };
        self.docks[i].active = Some(panel);
        for (p, tab) in self.docks[i].tabs.clone() {
            let on = p == panel;
            ui.set_style(tab, tab_style(on));
            let kids = ui.children(tab);
            ui.restyle(kids[0], |s| s.text_color(if on { ACCENT } else { LABEL }));
            ui.restyle(kids[1], |s| s.text_color(if on { TEXT } else { LABEL }));
            let root = self.roots[&p];
            ui.restyle(root, |s| if on { s.shown() } else { s.hidden() });
        }
    }

    /// Whether a panel is on top in its dock (its dock may still be hidden
    /// by the Window menu).
    pub fn is_active(&self, panel: Panel) -> bool {
        self.dock_of(panel)
            .is_some_and(|i| self.docks[i].active == Some(panel))
    }

    fn show_empty(&mut self, ui: &mut Ui, i: usize) {
        let empty = self.docks[i].tabs.is_empty();
        let hint = self.docks[i].empty;
        ui.restyle(hint, |s| if empty { s.shown() } else { s.hidden() });
    }

    /// Take a panel out of its dock, tab and all: it is going to a window
    /// of its own. Its content stays where it is until the caller moves it.
    pub fn take(&mut self, ui: &mut Ui, panel: Panel) -> Option<NodeId> {
        let from = self.dock_of(panel)?;
        let at = self.docks[from]
            .tabs
            .iter()
            .position(|(p, _)| *p == panel)?;
        let (_, tab) = self.docks[from].tabs.remove(at);
        ui.remove(tab);
        if self.docks[from].active == Some(panel) {
            self.docks[from].active = None;
            if let Some((next, _)) = self.docks[from].tabs.first().copied() {
                self.activate(ui, next);
            }
        }
        self.show_empty(ui, from);
        Some(self.roots[&panel])
    }

    /// Put a panel that was taken out back into dock `to`, on top there.
    pub fn give_back(&mut self, ui: &mut Ui, panel: Panel, to: usize) {
        if self.dock_of(panel).is_some() {
            return;
        }
        self.put(ui, panel, to);
        self.activate(ui, panel);
        self.show_empty(ui, to);
    }

    /// Whether a panel shows: on top of its dock, or in a window of its
    /// own (out of every dock).
    pub fn is_showing(&self, panel: Panel) -> bool {
        self.is_active(panel) || self.dock_of(panel).is_none()
    }

    /// Move a panel to dock `to`, on top there.
    pub fn move_panel(&mut self, ui: &mut Ui, panel: Panel, to: usize) {
        let Some(from) = self.dock_of(panel) else {
            return;
        };
        if from == to {
            return;
        }
        let at = self.docks[from]
            .tabs
            .iter()
            .position(|(p, _)| *p == panel)
            .expect("the panel is in its dock");
        let (_, tab) = self.docks[from].tabs.remove(at);
        ui.remove(tab);
        if self.docks[from].active == Some(panel) {
            self.docks[from].active = None;
            if let Some((next, _)) = self.docks[from].tabs.first().copied() {
                self.activate(ui, next);
            }
        }
        self.put(ui, panel, to);
        self.activate(ui, panel);
        self.show_empty(ui, from);
        self.show_empty(ui, to);
    }

    /// A tab clicked or dragged. `Some` when the event was the docks'.
    pub fn event(&mut self, ui: &mut Ui, node: NodeId, event: &Event) -> Option<Docked> {
        let panel = self
            .docks
            .iter()
            .flat_map(|d| d.tabs.iter())
            .find(|(_, t)| *t == node)
            .map(|(p, _)| *p)?;
        match event {
            Event::Click {
                button: runity::input::MouseButton::Right,
                ..
            } => Some(Docked::Menu(panel)),
            Event::Click { .. } => {
                self.activate(ui, panel);
                Some(Docked::Handled)
            }
            Event::Drag { .. } => {
                // The dock under the pointer lights up.
                let (x, y) = ui.pointer();
                for d in &self.docks {
                    let over = ui.rect(d.card).contains(x, y);
                    ui.restyle(d.card, |s| {
                        s.border(
                            1.0,
                            if over {
                                ACCENT
                            } else {
                                runity_ui::Color::TRANSPARENT
                            },
                        )
                    });
                }
                Some(Docked::Handled)
            }
            Event::DragEnd { .. } => {
                for d in &self.docks {
                    ui.restyle(d.card, |s| s.border(1.0, runity_ui::Color::TRANSPARENT));
                }
                let (x, y) = ui.pointer();
                if let Some(to) = self
                    .docks
                    .iter()
                    .position(|d| ui.rect(d.card).contains(x, y))
                {
                    self.move_panel(ui, panel, to);
                }
                Some(Docked::Handled)
            }
            _ => Some(Docked::Handled),
        }
    }

    /// Where every panel is, as the layout file writes it:
    /// `hierarchy|inspector|project,console,history,git` and the tabs on top.
    pub fn layout(&self) -> (String, String) {
        let docks = self
            .docks
            .iter()
            .map(|d| {
                d.tabs
                    .iter()
                    .map(|(p, _)| p.name())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect::<Vec<_>>()
            .join("|");
        let active = self
            .docks
            .iter()
            .map(|d| d.active.map_or("", Panel::name))
            .collect::<Vec<_>>()
            .join("|");
        (docks, active)
    }

    /// Read what [`Docks::layout`] wrote.
    pub fn parse(docks: &str) -> Option<[Vec<Panel>; 3]> {
        let parts: Vec<Vec<Panel>> = docks
            .split('|')
            .map(|d| d.split(',').filter_map(Panel::from_name).collect())
            .collect();
        (parts.len() == 3).then(|| [parts[0].clone(), parts[1].clone(), parts[2].clone()])
    }

    /// The default: Unity's — Hierarchy left, Inspector right, the rest
    /// under the view.
    pub fn default_layout() -> [Vec<Panel>; 3] {
        [
            vec![Panel::Hierarchy],
            vec![Panel::Inspector],
            vec![
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
            ],
        ]
    }
}
