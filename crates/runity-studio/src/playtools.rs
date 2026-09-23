//! Tools for a game that is running: the network inspector, the world
//! diff, the saves and the systems — the windows Dacha Simulator's
//! development leans on (docs/NEXT_STEPS.md, «Инструменты»).
//!
//! They read what a game started from the editor reports (its state file,
//! with `diagnostics`), what is on disk (saves, `src/main.rs`), and the
//! document. Each is a list, rebuilt twice a second while it shows.

use std::path::PathBuf;
use std::time::Instant;

use runity::save::{Difference, SaveGame};
use runity_editor::Session;
use runity_ui::{Event, NodeId, Style, Ui};

use crate::theme::*;

/// Which of the four a panel is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Network,
    Diff,
    Saves,
    Systems,
}

pub struct PlayTool {
    pub root: NodeId,
    tool: Tool,
    body: NodeId,
    shown: Instant,
    /// The saves, as last listed, by their rows: what a click opens and a
    /// delete removes.
    saves: Vec<(NodeId, NodeId, PathBuf)>,
    open_save: Option<PathBuf>,
}

impl PlayTool {
    pub fn new(ui: &mut Ui, parent: NodeId, tool: Tool) -> Self {
        let root = ui.add(
            parent,
            Style::column()
                .fill()
                .full_width()
                .padding(SPACE_2)
                .gap(2.0),
        );
        ui.set_name(
            root,
            match tool {
                Tool::Network => "network",
                Tool::Diff => "world diff",
                Tool::Saves => "saves",
                Tool::Systems => "systems",
            },
        );
        let body = ui.add(root, Style::column().fill().full_width().gap(1.0).clip());
        Self {
            root,
            tool,
            body,
            shown: Instant::now() - std::time::Duration::from_secs(10),
            saves: Vec::new(),
            open_save: None,
        }
    }

    pub fn owns(&self, ui: &Ui, node: NodeId) -> bool {
        let mut at = Some(node);
        while let Some(n) = at {
            if n == self.root {
                return true;
            }
            at = ui.parent(n);
        }
        false
    }

    /// Rebuild the list, at most twice a second.
    pub fn update(&mut self, ui: &mut Ui, session: &mut Session) {
        if self.shown.elapsed().as_secs_f32() < 0.5 {
            return;
        }
        self.shown = Instant::now();
        self.rebuild(ui, session);
    }

    fn rebuild(&mut self, ui: &mut Ui, session: &mut Session) {
        ui.clear(self.body);
        self.saves.clear();
        match self.tool {
            Tool::Network => self.network(ui, session),
            Tool::Diff => self.diff(ui, session),
            Tool::Saves => self.saves(ui, session),
            Tool::Systems => self.systems(ui, session),
        }
    }

    fn line(&self, ui: &mut Ui, text: &str, color: runity_ui::Color) -> NodeId {
        ui.add_text(
            self.body,
            Style::default()
                .text_size(11.5)
                .text_color(color)
                .mono()
                .nowrap(),
            text,
        )
    }

    fn heading(&self, ui: &mut Ui, text: &str) {
        ui.add_text(self.body, caption(), text);
    }

    fn name(session: &Session, id: runity::EntityId) -> String {
        session.entity_name(id).unwrap_or_else(|| id.to_string())
    }

    fn no_game(&self, ui: &mut Ui) {
        self.line(
            ui,
            "No game running: Play › Play in the Game, and it reports here.",
            MUTED,
        );
    }

    /// Who owns what, as each player sees it.
    fn network(&self, ui: &mut Ui, session: &mut Session) {
        let states = session.player_states();
        if states.is_empty() {
            self.no_game(ui);
            return;
        }
        for (player, state) in &states {
            let net = state
                .diagnostics
                .as_ref()
                .map(|d| d.net.as_slice())
                .unwrap_or(&[]);
            self.heading(ui, &format!("PLAYER {player} · {} networked", net.len()));
            if net.is_empty() {
                self.line(
                    ui,
                    "  nothing networked (a game alone, or not yet joined)",
                    MUTED,
                );
            }
            for line in net.iter().take(200) {
                let (tick, color) = match line.tick {
                    Some((tick, age)) if age > 500.0 => {
                        (format!("tick {tick}, {age:.0} ms ago"), WARNING)
                    }
                    Some((tick, age)) => (format!("tick {tick}, {age:.0} ms ago"), TEXT),
                    None => (String::new(), TEXT),
                };
                let how = if line.replica { "copy" } else { "owns" };
                self.line(
                    ui,
                    &format!(
                        "  {:<24} player {} {how:<4} {tick}",
                        Self::name(session, line.id),
                        line.owner + 1
                    ),
                    color,
                );
            }
        }
    }

    /// Where the players' worlds disagree with player 1's, and where
    /// player 1's world has left the document.
    fn diff(&self, ui: &mut Ui, session: &mut Session) {
        let states = session.player_states();
        if states.is_empty() {
            self.no_game(ui);
            return;
        }
        let (first, host) = &states[0];
        for (player, other) in states.iter().skip(1) {
            let found = runity::save::diff(host, other, 0.05);
            self.heading(
                ui,
                &format!(
                    "PLAYER {player} AGAINST PLAYER {first} · {} differences",
                    found.len()
                ),
            );
            self.differences(ui, session, &found);
        }
        // The game against the scene it started from.
        let document = document_as_save(session);
        let found: Vec<Difference> = runity::save::diff(&document, host, 0.05)
            .into_iter()
            // The game reports only the components it saves, and what it
            // spawned the scene does not have: placement is what compares.
            .filter(|d| !matches!(d, Difference::OnlyIn(_, 1) | Difference::Component(..)))
            .collect();
        self.heading(
            ui,
            &format!(
                "PLAYER {first} AGAINST THE SCENE · {} differences",
                found.len()
            ),
        );
        self.differences(ui, session, &found);
    }

    fn differences(&self, ui: &mut Ui, session: &Session, found: &[Difference]) {
        if found.is_empty() {
            self.line(ui, "  the same", MUTED);
        }
        for d in found.iter().take(200) {
            let text = match d {
                Difference::OnlyIn(id, side) => format!(
                    "  {:<24} only in the {}",
                    Self::name(session, *id),
                    if *side == 0 { "first" } else { "second" }
                ),
                Difference::Moved(id, m) => {
                    format!("  {:<24} {m:.2} m apart", Self::name(session, *id))
                }
                Difference::Turned(id, deg) => {
                    format!("  {:<24} turned {deg:.0}° apart", Self::name(session, *id))
                }
                Difference::Scaled(id) => {
                    format!("  {:<24} scaled differently", Self::name(session, *id))
                }
                Difference::Component(id, c) => {
                    format!("  {:<24} `{c}` differs", Self::name(session, *id))
                }
            };
            self.line(ui, &text, TEXT);
        }
    }

    /// The saves on disk; one opened shows what it holds.
    fn saves(&mut self, ui: &mut Ui, session: &mut Session) {
        let files = session.save_files();
        if files.is_empty() {
            self.line(
                ui,
                "No saves yet: a game saves with runity::save (SaveGame::write).",
                MUTED,
            );
            return;
        }
        for (path, save) in files {
            let row = ui.add(
                self.body,
                Style::row()
                    .full_width()
                    .height(22.0)
                    .fixed()
                    .padding_x(SPACE_2)
                    .gap(SPACE_2)
                    .center_items()
                    .radius(RADIUS_SM)
                    .hover(HOVER)
                    .clickable(),
            );
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.set_name(row, format!("save {name}"));
            icon(ui, row, "save", LABEL);
            ui.add_text(
                row,
                text().fill().nowrap(),
                &format!(
                    "{name} · {} things, {} gone · {}",
                    save.entities.len(),
                    save.gone.len(),
                    path.parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                ),
            );
            let delete = icon_button(ui, row, &format!("delete save {name}"), "trash", false);
            if self.open_save.as_ref() == Some(&path) {
                for s in save.entities.iter().take(60) {
                    let p = s.transform.position;
                    let comps: Vec<&str> = s.components.iter().map(|(n, _)| n.as_str()).collect();
                    self.line(
                        ui,
                        &format!(
                            "    {:<24} ({:.1}, {:.1}, {:.1}) {}{}",
                            Self::name(session, s.id),
                            p.x,
                            p.y,
                            p.z,
                            comps.join(", "),
                            if s.prefab.is_empty() {
                                String::new()
                            } else {
                                format!(" · spawned {}", s.prefab)
                            }
                        ),
                        TEXT,
                    );
                }
            }
            self.saves.push((row, delete, path));
        }
    }

    /// The systems in the order they run, with what each costs.
    fn systems(&self, ui: &mut Ui, session: &mut Session) {
        let order = session.systems();
        let timings: Vec<(String, f32, f32)> = session
            .player_states()
            .into_iter()
            .next()
            .and_then(|(_, s)| s.diagnostics)
            .map(|d| d.systems)
            .unwrap_or_default();
        if order.is_empty() && timings.is_empty() {
            self.line(ui, "No systems: runity add system NAME makes one.", MUTED);
            return;
        }
        self.heading(ui, "IN ORDER");
        let cost = |name: &str| {
            timings
                .iter()
                .find(|(n, _, _)| n == name)
                .map(|(_, median, worst)| format!("{median:.3} ms, worst {worst:.3}"))
        };
        for (i, name) in order.iter().enumerate() {
            let text = format!(
                "{:>3}. {name:<24} {}",
                i + 1,
                cost(name).unwrap_or_else(|| "—".into())
            );
            self.line(ui, &text, TEXT);
        }
        let rest: Vec<&(String, f32, f32)> = timings
            .iter()
            .filter(|(n, _, _)| !order.contains(n))
            .collect();
        if !rest.is_empty() {
            self.heading(ui, "AND THE ENGINE'S");
            for (name, median, worst) in rest {
                self.line(
                    ui,
                    &format!("     {name:<24} {median:.3} ms, worst {worst:.3}"),
                    LABEL,
                );
            }
        }
    }

    pub fn event(&mut self, ui: &mut Ui, session: &mut Session, node: NodeId, event: &Event) {
        let Event::Click { .. } = event else { return };
        if let Some((_, _, path)) = self.saves.iter().find(|(_, d, _)| *d == node).cloned() {
            match std::fs::remove_file(&path) {
                Ok(()) => session.say(
                    runity_editor::console::Level::Info,
                    format!("deleted the save {}", path.display()),
                ),
                Err(e) => session.say(runity_editor::console::Level::Error, e.to_string()),
            }
            self.rebuild(ui, session);
            return;
        }
        if let Some((_, _, path)) = self.saves.iter().find(|(r, _, _)| *r == node).cloned() {
            self.open_save = if self.open_save.as_ref() == Some(&path) {
                None
            } else {
                Some(path)
            };
            self.rebuild(ui, session);
        }
    }
}

/// The document as a save: every entity where the scene puts it, for the
/// world diff to hold a running game against.
fn document_as_save(session: &Session) -> SaveGame {
    SaveGame {
        entities: session
            .expanded()
            .flatten()
            .into_iter()
            .map(|(desc, _)| runity::save::Saved {
                id: desc.id,
                transform: desc.transform,
                components: desc
                    .components
                    .iter()
                    .map(|(n, v)| (n.clone(), v.get_ron().to_string()))
                    .collect(),
                prefab: String::new(),
                animator: String::new(),
            })
            .collect(),
        ..Default::default()
    }
}
