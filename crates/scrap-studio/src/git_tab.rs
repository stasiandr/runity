//! The Git tab: the project's repository as the editor sees it (DNA,
//! postulate 2 — history in the editor, merges of things, not lines).
//!
//! **Changes**, on the left: the branch, how far it is from the one it
//! follows, and each file of the project not as committed — the open
//! scene's unsaved edits too — each with a box to leave it out; a message,
//! and Commit (or Cmd/Ctrl Enter), which saves the scene first when it goes.
//!
//! **Conflicts**, while git is merging the open scene: each one in words,
//! with ours and theirs and which the document holds now; a click takes a
//! side, as one undo step. Mark resolved saves and tells git.
//!
//! **History**: the open scene's commits, or the project's, found by a
//! word; a click shows what that commit changed — files, and the scene's
//! things and fields, before → after — and Restore puts the scene back as
//! it was then, as one undo step (so does a double click).
//!
//! Everything here is a function of the session (`git_status`, `commit`,
//! `commit_changes`, `take_ours`, …) that the agent has as a tool too; the
//! status comes from `git_marks`' asking off the frame, so the tab is live
//! without asking git itself.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use scrap::merge::{Change, Conflict, Side};
use scrap::EntityId;
use scrap_editor::console::Level;
use scrap_editor::{Revision, Session, Status};
use scrap_ui::{Color, Event, NodeId, Style, Ui};

use crate::git_marks::GitMarks;
use crate::studio::Requests;
use crate::theme::*;

/// How many of the project's commits the history lists.
const PROJECT_COMMITS: usize = 300;
/// How many lines a list shows before it says how many more there are.
const MOST_LINES: usize = 200;

pub struct GitTab {
    // The bar
    branch: NodeId,
    refresh: NodeId,
    // Changes
    changes_title: NodeId,
    all: NodeId,
    files: NodeId,
    message: NodeId,
    commit: NodeId,
    /// Each file line, and its file.
    file_rows: HashMap<NodeId, PathBuf>,
    /// Files left out of the next commit, by the box on their line.
    left_out: HashSet<PathBuf>,
    /// The files listed, in order: what Commit takes.
    listed: Vec<PathBuf>,
    // Conflicts
    conflicts: NodeId,
    sides: HashMap<NodeId, (usize, Side)>,
    resolved: Option<NodeId>,
    /// The document revision the conflicts' sides were drawn for.
    conflicts_drawn: Option<u64>,
    has_conflicts: bool,
    // History
    scope_chips: [NodeId; 2],
    project_scope: bool,
    search: NodeId,
    count: NodeId,
    list: NodeId,
    details: NodeId,
    revisions: Vec<Revision>,
    rows: HashMap<NodeId, String>,
    selected: Option<String>,
    restore: Option<NodeId>,
    /// Each change line of the details, and the thing it is about.
    change_rows: HashMap<NodeId, EntityId>,
    // When to draw again
    /// Everything: the tab came on top, Refresh, a commit.
    stale: bool,
    /// The history, for a new `HEAD` or scope.
    history_stale: bool,
    /// The list only, for a word typed or a line chosen.
    list_stale: bool,
    seen_generation: Option<u64>,
    seen_head: String,
    /// Whether the document had unsaved edits when the files were drawn.
    seen_modified: Option<bool>,
    status: Option<Status>,
    /// Ask git again now rather than in a couple of seconds.
    ask_git: bool,
    /// The HEAD the history was read at.
    history_head: Option<String>,
}

impl GitTab {
    pub fn new(ui: &mut Ui, root: NodeId) -> Self {
        let small = |c: Color| Style::default().text_size(11.5).text_color(c).nowrap();

        // The bar: branch, and Refresh.
        let bar = ui.add(
            root,
            Style::row()
                .full_width()
                .height(30.0)
                .fixed()
                .padding_x(SPACE_3)
                .gap(SPACE_2)
                .center_items(),
        );
        icon(ui, bar, "git-branch", LABEL);
        let branch = ui.add(bar, Style::row().gap(SPACE_2).center_items().fill());
        ui.set_name(branch, "git branch");
        let refresh = crate::theme::button(ui, bar, "git refresh", "Refresh", false);

        let body = ui.add(root, Style::row().fill().full_width());

        // Changes
        let left = ui.add(
            body,
            Style::column()
                .width(290.0)
                .fixed()
                .full_height()
                .padding_x(SPACE_3)
                .padding_bottom(SPACE_2)
                .gap(SPACE_1),
        );
        let head = ui.add(
            left,
            Style::row()
                .full_width()
                .height(22.0)
                .fixed()
                .gap(SPACE_2)
                .center_items(),
        );
        let all = check_box(ui, head, true);
        ui.set_name(all, "git all");
        let changes_title = ui.add_text(head, caption().fill(), "CHANGES");
        let files = ui.add(
            left,
            Style::column().fill().full_width().clip(),
        );
        ui.set_name(files, "git files");
        // The message and Commit on one line: the dock is often short.
        let commit_row = ui.add(
            left,
            Style::row()
                .full_width()
                .height(24.0)
                .fixed()
                .gap(SPACE_2)
                .center_items(),
        );
        let message = ui.add_textarea(commit_row, field_style().height(24.0).fill(), "");
        ui.set_name(message, "git message");
        ui.set_placeholder(message, "What changed, and why");
        let commit = crate::theme::button(ui, commit_row, "git commit", "Commit", true);

        ui.add(
            body,
            Style::default().width(1.0).fixed().full_height().background(DIVIDER),
        );

        // Conflicts, then the history.
        let right = ui.add(body, Style::column().fill().full_height());
        let conflicts = ui.add(right, Style::column().full_width().hidden());
        ui.set_name(conflicts, "git conflicts");
        let tools = ui.add(
            right,
            Style::row()
                .full_width()
                .height(28.0)
                .fixed()
                .padding_x(SPACE_3)
                .gap(SPACE_2)
                .center_items(),
        );
        let mut scope_chips = [tools; 2];
        for (i, label) in ["Scene", "Project"].into_iter().enumerate() {
            let chip = ui.add(
                tools,
                Style::row()
                    .height(22.0)
                    .padding_x(SPACE_2)
                    .center()
                    .radius(6.0)
                    .hover(HOVER),
            );
            ui.set_name(chip, format!("git scope {label}"));
            ui.add_text(chip, small(LABEL), label);
            scope_chips[i] = chip;
        }
        let search = ui.add_field(tools, field_style().width(200.0).height(22.0), "");
        ui.set_name(search, "git search");
        ui.set_placeholder(search, "Find a commit");
        spacer(ui, tools);
        let count = ui.add_text(tools, small(MUTED), "");
        let lower = ui.add(right, Style::row().fill().full_width());
        let list = ui.add(
            lower,
            Style::column().fill().full_height().padding_y(SPACE_1).clip(),
        );
        ui.set_name(list, "git lines");
        let details = ui.add(
            lower,
            Style::column()
                .width(340.0)
                .fixed()
                .full_height()
                .padding_x(SPACE_3)
                .padding_y(SPACE_1)
                .gap(SPACE_1)
                .clip()
                .hidden(),
        );
        ui.set_name(details, "git details");

        Self {
            branch,
            refresh,
            changes_title,
            all,
            files,
            message,
            commit,
            file_rows: HashMap::new(),
            left_out: HashSet::new(),
            listed: Vec::new(),
            conflicts,
            sides: HashMap::new(),
            resolved: None,
            conflicts_drawn: None,
            has_conflicts: false,
            scope_chips,
            project_scope: false,
            search,
            count,
            list,
            details,
            revisions: Vec::new(),
            rows: HashMap::new(),
            selected: None,
            restore: None,
            change_rows: HashMap::new(),
            stale: true,
            history_stale: true,
            list_stale: true,
            seen_generation: None,
            seen_head: String::new(),
            seen_modified: None,
            status: None,
            ask_git: false,
            history_head: None,
        }
    }

    /// Draw again from scratch, next time the tab is on top.
    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    /// Bring the tab up to date: what git said last (`marks`) and the
    /// document. Only what changed is drawn again.
    pub fn update(&mut self, ui: &mut Ui, session: &mut Session, marks: &mut GitMarks) {
        if std::mem::take(&mut self.ask_git) {
            marks.ask_soon();
        }
        let mut files_stale = false;
        let mut conflicts_stale = false;
        if std::mem::take(&mut self.stale) {
            if marks.status.is_some() {
                // What git said last, shown at once, and git asked again
                // beside the frames: the tab comes on top without waiting
                // for it. The history is read again only for a new HEAD.
                self.status = marks.status.clone();
                marks.ask_soon();
                if self.history_head.as_deref() != Some(marks.head.as_str()) {
                    self.history_stale = true;
                }
            } else {
                // Asked here, once: git_marks' answer may be seconds away,
                // or never come outside a project.
                self.status = session.git_status().ok();
                self.history_stale = true;
            }
            self.seen_generation = Some(marks.generation);
            self.seen_head = marks.head.clone();
            files_stale = true;
            conflicts_stale = true;
        } else if self.seen_generation != Some(marks.generation) {
            self.seen_generation = Some(marks.generation);
            if marks.status.is_some() {
                self.status = marks.status.clone();
            }
            if marks.head != self.seen_head {
                self.seen_head = marks.head.clone();
                self.history_stale = true;
            }
            files_stale = true;
            conflicts_stale = true;
        }
        let modified = session.is_modified();
        if self.seen_modified != Some(modified) {
            files_stale = true;
        }
        if self.has_conflicts && self.conflicts_drawn != Some(session.revision()) {
            conflicts_stale = true;
        }
        if files_stale {
            self.seen_modified = Some(modified);
            self.draw_branch(ui);
            self.draw_files(ui, session);
        }
        if conflicts_stale {
            self.draw_conflicts(ui, session);
        }
        if std::mem::take(&mut self.history_stale) {
            self.history_head = Some(marks.head.clone());
            self.load_history(session);
            self.list_stale = true;
        }
        if std::mem::take(&mut self.list_stale) {
            self.draw_list(ui);
            self.draw_details(ui, session);
        }
    }

    fn draw_branch(&mut self, ui: &mut Ui) {
        ui.clear(self.branch);
        let small = |c| Style::default().text_size(11.5).text_color(c).nowrap();
        let Some(status) = &self.status else {
            ui.add_text(self.branch, small(MUTED), "Not in a git repository");
            return;
        };
        ui.add_text(
            self.branch,
            small(TEXT),
            status.branch.as_deref().unwrap_or("detached HEAD"),
        );
        if let Some(upstream) = &status.upstream {
            ui.add_text(self.branch, small(MUTED), &format!("→ {upstream}"));
            if status.ahead > 0 {
                tag(ui, self.branch, &format!("↑{} to push", status.ahead), ACCENT_900, ACCENT_300);
            }
            if status.behind > 0 {
                tag(ui, self.branch, &format!("↓{} to pull", status.behind), NEUTRAL_900, WARNING);
            }
            if status.ahead == 0 && status.behind == 0 {
                ui.add_text(self.branch, small(MUTED), "· up to date");
            }
        }
        if status.merging {
            tag(ui, self.branch, "merging", NEUTRAL_900, WARNING);
        }
    }

    fn draw_files(&mut self, ui: &mut Ui, session: &Session) {
        ui.clear(self.files);
        self.file_rows.clear();
        self.listed.clear();
        let small = |c| Style::default().text_size(11.5).text_color(c).nowrap();
        let root = session.project().map(|p| p.root().to_path_buf());
        let scene = session
            .scene_path()
            .map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
        let unsaved = session.is_modified();
        // (file, letter, what it is, unsaved)
        let mut lines: Vec<(PathBuf, char, &str, bool)> = self
            .status
            .iter()
            .flat_map(|s| &s.files)
            .map(|f| {
                let here = scene.as_ref() == Some(&f.path);
                (f.path.clone(), f.letter(), f.word(), here && unsaved)
            })
            .collect();
        if unsaved && self.status.is_some() {
            if let Some(scene) = &scene {
                if !lines.iter().any(|(p, ..)| p == scene) {
                    lines.insert(0, (scene.clone(), 'M', "changed", true));
                }
            }
        }
        // Files gone from the list are not left out of anything any more.
        let now: HashSet<&PathBuf> = lines.iter().map(|(p, ..)| p).collect();
        self.left_out.retain(|p| now.contains(p));

        let chosen = lines.iter().filter(|(p, ..)| !self.left_out.contains(p)).count();
        ui.set_text(
            self.changes_title,
            &if lines.is_empty() {
                "CHANGES".to_string()
            } else {
                format!("CHANGES · {chosen} of {}", lines.len())
            },
        );
        set_check(ui, self.all, !lines.is_empty() && chosen == lines.len());

        if self.status.is_none() {
            let h = ui.add(self.files, Style::row().padding_y(SPACE_2));
            ui.add_text(h, small(MUTED), "No repository here: git init to start one.");
            return;
        }
        if lines.is_empty() {
            let h = ui.add(self.files, Style::row().padding_y(SPACE_2));
            ui.add_text(h, small(MUTED), "Nothing to commit: all as committed.");
            return;
        }
        for (path, letter, word, unsaved) in lines.iter().take(MOST_LINES) {
            let row = ui.add(
                self.files,
                Style::row()
                    .height(22.0)
                    .fixed()
                    .full_width()
                    .gap(SPACE_2)
                    .center_items()
                    .radius(RADIUS_SM)
                    .hover(TEXT.alpha(5))
                    .clickable(),
            );
            let shown = shown_name(path, root.as_deref());
            ui.set_name(row, format!("git file {shown}"));
            check_box(ui, row, !self.left_out.contains(path));
            let ink = letter_ink(*letter);
            ui.add_text(row, small(ink).mono().width(12.0).fixed(), &letter.to_string());
            let (folder, file) = match shown.rsplit_once('/') {
                Some((folder, file)) => (Some(folder), file),
                None => (None, shown.as_str()),
            };
            ui.add_text(row, small(TEXT), file);
            if let Some(folder) = folder {
                ui.add_text(row, small(MUTED).fill(), folder);
            } else {
                spacer(ui, row);
            }
            if *unsaved {
                tag(ui, row, "unsaved", NEUTRAL_900, WARNING);
            } else if *letter == 'U' {
                tag(ui, row, word, NEUTRAL_900, ERROR);
            }
            self.file_rows.insert(row, path.clone());
            self.listed.push(path.clone());
        }
        if lines.len() > MOST_LINES {
            let h = ui.add(self.files, Style::row().padding_y(SPACE_1));
            ui.add_text(
                h,
                small(MUTED),
                &format!("and {} more", lines.len() - MOST_LINES),
            );
            self.listed
                .extend(lines[MOST_LINES..].iter().map(|(p, ..)| p.clone()));
        }
    }

    fn draw_conflicts(&mut self, ui: &mut Ui, session: &mut Session) {
        ui.clear(self.conflicts);
        self.sides.clear();
        self.resolved = None;
        self.conflicts_drawn = Some(session.revision());
        let conflicts = session.merge_conflicts().unwrap_or_default();
        self.has_conflicts = !conflicts.is_empty();
        if conflicts.is_empty() {
            ui.restyle(self.conflicts, Style::hidden);
            return;
        }
        ui.restyle(self.conflicts, Style::shown);
        let sides = session.conflict_sides();
        let small = |c| Style::default().text_size(11.5).text_color(c).nowrap();
        let head = ui.add(
            self.conflicts,
            Style::row()
                .full_width()
                .height(28.0)
                .fixed()
                .padding_x(SPACE_3)
                .gap(SPACE_2)
                .center_items()
                .background(WARNING.alpha(18)),
        );
        icon(ui, head, "git-merge", WARNING);
        ui.add_text(
            head,
            small(TEXT).fill(),
            &format!(
                "{} in this scene's merge — the merge kept ours; pick a side where it should be theirs",
                match conflicts.len() {
                    1 => "1 conflict".to_string(),
                    n => format!("{n} conflicts"),
                }
            ),
        );
        let resolved = crate::theme::button(ui, head, "git resolved", "Mark resolved", true);
        self.resolved = Some(resolved);
        for (i, c) in conflicts.iter().enumerate() {
            let row = ui.add(
                self.conflicts,
                Style::row()
                    .height(26.0)
                    .fixed()
                    .full_width()
                    .padding_x(SPACE_3)
                    .gap(SPACE_2)
                    .center_items(),
            );
            ui.set_name(row, format!("conflict {i}"));
            icon(ui, row, "triangle-alert", WARNING);
            let side = sides.get(i).copied().flatten();
            match conflict_values(c) {
                Some((ours, theirs)) => {
                    ui.add_text(row, small(TEXT), &conflict_subject(c));
                    spacer(ui, row);
                    for (s, word, value) in [(Side::Ours, "Ours", ours), (Side::Theirs, "Theirs", theirs)] {
                        let chip = side_chip(ui, row, word, &value, side == Some(s));
                        let lower = word.to_lowercase();
                        ui.set_name(chip, format!("take {lower} {i}"));
                        self.sides.insert(chip, (i, s));
                    }
                }
                None => {
                    ui.add_text(row, small(TEXT).fill(), &c.to_string());
                }
            }
        }
        ui.add(
            self.conflicts,
            Style::default().height(1.0).fixed().full_width().background(DIVIDER),
        );
    }

    fn load_history(&mut self, session: &Session) {
        if session.scene_path().is_none() {
            self.project_scope = true;
        }
        let revisions = if self.project_scope {
            session.project_history(PROJECT_COMMITS)
        } else {
            session.scene_history()
        };
        self.revisions = revisions.unwrap_or_default();
        if self
            .selected
            .as_ref()
            .is_some_and(|c| !self.revisions.iter().any(|r| &r.commit == c))
        {
            self.selected = None;
        }
    }

    fn draw_list(&mut self, ui: &mut Ui) {
        for (i, chip) in self.scope_chips.iter().enumerate() {
            let on = (i == 1) == self.project_scope;
            ui.restyle(*chip, |s| {
                s.background(if on { ACCENT_HOVER } else { Color::TRANSPARENT })
                    .border(1.0, if on { ACCENT } else { Color::TRANSPARENT })
            });
        }
        ui.clear(self.list);
        self.rows.clear();
        let small = |c| Style::default().text_size(11.5).text_color(c).nowrap();
        let word = ui.text(self.search).unwrap_or_default().trim().to_lowercase();
        let found: Vec<&Revision> = self
            .revisions
            .iter()
            .filter(|r| {
                word.is_empty()
                    || r.summary.to_lowercase().contains(&word)
                    || r.author.to_lowercase().contains(&word)
                    || r.commit.starts_with(&word)
            })
            .collect();
        ui.set_text(
            self.count,
            &match (word.is_empty(), self.revisions.len()) {
                (_, 0) => String::new(),
                (true, n) => format!("{n} commits"),
                (false, n) => format!("{} of {n}", found.len()),
            },
        );
        if found.is_empty() {
            let h = ui.add(self.list, Style::row().padding_x(SPACE_4).padding_y(SPACE_1));
            let said = if !word.is_empty() {
                "No commit says that."
            } else if self.project_scope {
                "No commits in this project yet."
            } else {
                "No commits of this scene yet."
            };
            ui.add_text(h, small(MUTED), said);
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        for r in found.into_iter().take(MOST_LINES) {
            let row = ui.add(
                self.list,
                Style::row()
                    .height(22.0)
                    .fixed()
                    .full_width()
                    .padding_x(SPACE_4)
                    .gap(SPACE_3)
                    .center_items(),
            );
            ui.set_name(row, format!("revision {}", short(&r.commit, 8)));
            ui.add_text(row, small(MUTED).mono().width(64.0).fixed(), short(&r.commit, 7));
            ui.add_text(row, small(MUTED).width(84.0).fixed(), &ago(r.when, now, &r.date));
            ui.add_text(row, small(LABEL).width(120.0).fixed(), &r.author);
            ui.add_text(row, small(TEXT).fill(), &r.summary);
            self.rows.insert(row, r.commit.clone());
        }
        self.light_rows(ui);
    }

    /// Light the chosen line, and only it.
    fn light_rows(&self, ui: &mut Ui) {
        for (row, commit) in &self.rows {
            let chosen = self.selected.as_ref() == Some(commit);
            ui.restyle(*row, |s| {
                s.background(if chosen { ACCENT_HOVER } else { Color::TRANSPARENT })
                    .hover(if chosen { ACCENT_HOVER } else { TEXT.alpha(5) })
            });
            if let Some(hash) = ui.children(*row).first().copied() {
                ui.restyle(hash, |s| s.text_color(if chosen { ACCENT } else { MUTED }));
            }
        }
    }

    fn draw_details(&mut self, ui: &mut Ui, session: &Session) {
        ui.clear(self.details);
        self.change_rows.clear();
        self.restore = None;
        let Some(commit) = self.selected.clone() else {
            ui.restyle(self.details, Style::hidden);
            return;
        };
        let Some(r) = self.revisions.iter().find(|r| r.commit == commit).cloned() else {
            ui.restyle(self.details, Style::hidden);
            return;
        };
        ui.restyle(self.details, Style::shown);
        let small = |c| Style::default().text_size(11.5).text_color(c).nowrap();
        let head = ui.add(
            self.details,
            Style::row().full_width().height(24.0).fixed().gap(SPACE_2).center_items(),
        );
        icon(ui, head, "git-commit-horizontal", ACCENT);
        ui.add_text(head, small(ACCENT).mono(), short(&r.commit, 10));
        ui.add_text(head, small(MUTED).fill(), &format!("{} · {}", r.author, r.date));
        ui.add_text(
            self.details,
            Style::default().text_size(12.0).text_color(TEXT).wrap().full_width(),
            &r.summary,
        );
        let changes = match session.commit_changes(&commit) {
            Ok(c) => c,
            Err(e) => {
                ui.add_text(self.details, small(ERROR), &e.to_string());
                return;
            }
        };
        let scene_touched = !changes.scene.is_empty();
        if session.scene_path().is_some() && (scene_touched || !self.project_scope) {
            let row = ui.add(
                self.details,
                Style::row().full_width().padding_y(SPACE_1).gap(SPACE_2).center_items(),
            );
            let b = crate::theme::button(ui, row, "git restore", "Restore scene", false);
            ui.add_text(row, small(MUTED), "as of this commit, one undo step");
            self.restore = Some(b);
        }
        if !changes.scene.is_empty() {
            ui.add_text(
                self.details,
                caption().padding_top(SPACE_2),
                &format!("IN THIS SCENE · {}", changes.scene.len()),
            );
            for c in changes.scene.iter().take(MOST_LINES) {
                let row = ui.add(
                    self.details,
                    Style::row()
                        .height(20.0)
                        .fixed()
                        .full_width()
                        .gap(SPACE_2)
                        .center_items()
                        .radius(RADIUS_SM)
                        .hover(TEXT.alpha(5)),
                );
                let (glyph, ink) = match c {
                    Change::Added { .. } => ("plus", SUCCESS),
                    Change::Removed { .. } => ("minus", ERROR),
                    Change::Field { .. } => ("pencil", INFO),
                };
                icon(ui, row, glyph, ink);
                ui.add_text(row, small(TEXT).fill(), &change_line(c));
                if let Some(id) = c.entity() {
                    self.change_rows.insert(row, id);
                }
            }
            if changes.scene.len() > MOST_LINES {
                ui.add_text(
                    self.details,
                    small(MUTED),
                    &format!("and {} more", changes.scene.len() - MOST_LINES),
                );
            }
        }
        ui.add_text(
            self.details,
            caption().padding_top(SPACE_2),
            &format!("FILES · {}", changes.files.len()),
        );
        for (letter, name) in changes.files.iter().take(MOST_LINES) {
            let row = ui.add(
                self.details,
                Style::row().height(18.0).fixed().full_width().gap(SPACE_2).center_items(),
            );
            ui.add_text(row, small(letter_ink(*letter)).mono().width(12.0).fixed(), &letter.to_string());
            ui.add_text(row, small(LABEL).fill(), name);
        }
    }

    /// Whether a click or a key on `node` was the tab's, and what it did.
    pub fn event(
        &mut self,
        ui: &mut Ui,
        session: &mut Session,
        node: NodeId,
        event: &Event,
        requests: &mut Requests,
    ) -> bool {
        match event {
            Event::Click { .. } if node == self.refresh => {
                self.stale = true;
                self.ask_git = true;
            }
            Event::Click { .. } if node == self.commit => self.commit(ui, session, requests),
            Event::Submit(_) if node == self.message => self.commit(ui, session, requests),
            Event::Click { .. } if node == self.all => {
                let all_in = self.listed.iter().all(|p| !self.left_out.contains(p));
                if all_in {
                    self.left_out.extend(self.listed.iter().cloned());
                } else {
                    self.left_out.clear();
                }
                self.seen_modified = None;
            }
            Event::Click { .. } if self.file_rows.contains_key(&node) => {
                let path = self.file_rows[&node].clone();
                if !self.left_out.remove(&path) {
                    self.left_out.insert(path);
                }
                self.seen_modified = None;
            }
            Event::Click { .. } if self.sides.contains_key(&node) => {
                let (i, side) = self.sides[&node];
                let taken = match side {
                    Side::Ours => session.take_ours(i),
                    Side::Theirs => session.take_theirs(i),
                };
                if let Err(e) = taken {
                    session.say(Level::Error, e.to_string());
                }
                self.conflicts_drawn = None;
                requests.refresh = true;
            }
            Event::Click { .. } if Some(node) == self.resolved => {
                let done = (|| {
                    if session.is_modified() {
                        session.save_scene(None)?;
                    }
                    session.mark_resolved()
                })();
                match done {
                    Ok(()) => session.say(
                        Level::Info,
                        "the scene's conflicts are settled for git: commit to finish the merge",
                    ),
                    Err(e) => session.say(Level::Error, e.to_string()),
                }
                self.stale = true;
                self.ask_git = true;
                requests.refresh = true;
            }
            Event::Click { .. } if self.scope_chips.contains(&node) => {
                let project = node == self.scope_chips[1];
                if project != self.project_scope {
                    self.project_scope = project;
                    self.selected = None;
                    self.history_stale = true;
                }
            }
            Event::Changed(_) | Event::Cancel if node == self.search => self.list_stale = true,
            Event::Click { count, .. } if self.rows.contains_key(&node) => {
                let commit = self.rows[&node].clone();
                if *count >= 2 {
                    if session.scene_path().is_some() {
                        self.restore(session, &commit, requests);
                    }
                    self.selected = Some(commit);
                } else if self.selected.as_ref() == Some(&commit) {
                    self.selected = None;
                } else {
                    self.selected = Some(commit);
                }
                // The lines stay the same nodes, so a second click counts
                // as a double one.
                self.light_rows(ui);
                self.draw_details(ui, session);
            }
            Event::Click { .. } if Some(node) == self.restore => {
                if let Some(commit) = self.selected.clone() {
                    self.restore(session, &commit, requests);
                }
            }
            Event::Click { .. } if self.change_rows.contains_key(&node) => {
                let id = self.change_rows[&node];
                if session.scene().get(id).is_some() && session.select(Some(id)).is_ok() {
                    requests.refresh = true;
                }
            }
            _ => return false,
        }
        true
    }

    fn restore(&mut self, session: &mut Session, commit: &str, requests: &mut Requests) {
        match session.restore_revision(commit) {
            Ok(()) => session.say(
                Level::Info,
                format!("brought back the scene as of {}", short(commit, 7)),
            ),
            Err(e) => session.say(Level::Error, e.to_string()),
        }
        requests.refresh = true;
    }

    /// Commit what is ticked, the scene saved first if it goes with unsaved
    /// edits.
    fn commit(&mut self, ui: &mut Ui, session: &mut Session, requests: &mut Requests) {
        let message = ui.text(self.message).unwrap_or_default().trim().to_string();
        let paths: Vec<PathBuf> = self
            .listed
            .iter()
            .filter(|p| !self.left_out.contains(*p))
            .cloned()
            .collect();
        let scene = session
            .scene_path()
            .map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
        let done = (|| {
            if message.is_empty() {
                return Err("write what changed first: a commit needs a message".to_string());
            }
            if paths.is_empty() {
                return Err("nothing ticked to commit".to_string());
            }
            if session.is_modified() && scene.as_ref().is_some_and(|s| paths.contains(s)) {
                session.save_scene(None).map_err(|e| e.to_string())?;
            }
            session.commit(&paths, &message).map_err(|e| e.to_string())
        })();
        match done {
            Ok(short) => {
                let first = message.lines().next().unwrap_or_default();
                session.say(Level::Info, format!("committed {short}: {first}"));
                ui.set_text(self.message, "");
                self.left_out.clear();
                self.stale = true;
                self.ask_git = true;
                requests.refresh = true;
            }
            Err(e) => session.say(Level::Error, e),
        }
    }
}

/// A box that is ticked or not: Unity's checkbox, as the Inspector draws it.
fn check_box(ui: &mut Ui, parent: NodeId, on: bool) -> NodeId {
    let b = ui.add(parent, Style::row().size(14.0, 14.0).fixed().center().radius(4.0).clickable());
    set_check(ui, b, on);
    b
}

fn set_check(ui: &mut Ui, b: NodeId, on: bool) {
    ui.restyle(b, |s| {
        s.border(1.0, if on { ACCENT } else { NEUTRAL_500 })
            .background(if on { ACCENT } else { Color::TRANSPARENT })
            .hover_border(ACCENT)
    });
    ui.clear(b);
    if on {
        ui.add_icon(b, Style::default().size(11.0, 11.0).fixed().text_color(NEUTRAL_900), "check");
    }
}

/// A chip that takes a side of a conflict: the side's word and value, lit
/// when the document holds it.
fn side_chip(ui: &mut Ui, parent: NodeId, word: &str, value: &str, on: bool) -> NodeId {
    let chip = ui.add(
        parent,
        Style::row()
            .height(22.0)
            .max_width(260.0)
            .padding_x(SPACE_2)
            .gap(SPACE_1)
            .center_items()
            .radius(6.0)
            .border(1.0, if on { ACCENT } else { DIVIDER })
            .background(if on { ACCENT_HOVER } else { Color::TRANSPARENT })
            .hover(if on { ACCENT_HOVER } else { HOVER })
            .clip()
            .clickable(),
    );
    if on {
        ui.add_icon(chip, Style::default().size(11.0, 11.0).fixed().text_color(ACCENT), "check");
    }
    ui.add_text(
        chip,
        Style::default().text_size(11.0).text_color(if on { ACCENT } else { LABEL }).nowrap(),
        word,
    );
    ui.add_text(
        chip,
        Style::default().text_size(11.0).text_color(MUTED).nowrap().mono(),
        value,
    );
    chip
}

/// What a conflict is about: "`pine` · its position".
fn conflict_subject(c: &Conflict) -> String {
    match c.entity {
        Some(_) => format!("`{}` · {}", c.entity_name, c.field),
        None => format!("the scene · {}", c.field),
    }
}

/// Ours and theirs, when there is a choice between them.
fn conflict_values(c: &Conflict) -> Option<(String, String)> {
    (!c.ours.is_empty() || !c.theirs.is_empty()).then(|| (c.ours.clone(), c.theirs.clone()))
}

/// A change of the details, without the entity's id.
fn change_line(c: &Change) -> String {
    match c {
        Change::Added { name, .. } => format!("added `{name}`"),
        Change::Removed { name, .. } => format!("removed `{name}`"),
        Change::Field {
            entity: Some(_),
            entity_name,
            field,
            before,
            after,
            ..
        } => format!("`{entity_name}` {field}: {before} → {after}"),
        Change::Field {
            entity: None,
            field,
            before,
            after,
            ..
        } => format!("the scene's {field}: {before} → {after}"),
    }
}

fn letter_ink(letter: char) -> Color {
    match letter {
        'A' => SUCCESS,
        'D' | 'U' => ERROR,
        'R' => INFO,
        _ => WARNING,
    }
}

/// A file as the project names it, `/` between folders; outside the
/// project, as it is.
fn shown_name(path: &Path, root: Option<&Path>) -> String {
    let root = root.map(|r| r.canonicalize().unwrap_or_else(|_| r.to_path_buf()));
    root.as_deref()
        .and_then(|r| path.strip_prefix(r).ok())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn short(commit: &str, n: usize) -> &str {
    &commit[..commit.len().min(n)]
}

/// "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago" — and
/// the date past a month.
fn ago(when: i64, now: i64, date: &str) -> String {
    let s = (now - when).max(0);
    match s {
        _ if when <= 0 => date.to_string(),
        0..60 => "just now".into(),
        60..3_600 => format!("{} min ago", s / 60),
        3_600..86_400 => format!("{} h ago", s / 3_600),
        86_400..172_800 => "yesterday".into(),
        172_800..2_592_000 => format!("{} days ago", s / 86_400),
        _ => date.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_is_said_as_people_say_it() {
        let now = 1_000_000_000;
        assert_eq!(ago(now - 5, now, "d"), "just now");
        assert_eq!(ago(now - 300, now, "d"), "5 min ago");
        assert_eq!(ago(now - 3 * 3_600, now, "d"), "3 h ago");
        assert_eq!(ago(now - 100_000, now, "d"), "yesterday");
        assert_eq!(ago(now - 4 * 86_400, now, "d"), "4 days ago");
        assert_eq!(ago(now - 90 * 86_400, now, "2001-06-01"), "2001-06-01");
    }
}
