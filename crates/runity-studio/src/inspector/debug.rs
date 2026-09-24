//! Debug mode: what is shown as the RON its file holds, whole, in one box —
//! the entity's block, or with nothing selected the scene's own settings.
//!
//! Unity's Debug Inspector shows the serialized fields as they are; here
//! the serialized text is the file, so that is what is shown. It reads and
//! writes through `Session::entity_ron` / `set_entity_ron` (and the scene's
//! settings through `scene_settings_ron`), one undo step an apply, the
//! entity's id, children and place kept. What does not read is said under
//! the box, and nothing changes.

use runity::EntityId;
use runity_editor::Session;
use runity_ui::{NodeId, Style, Ui};

use super::{Inspector, Part};
use crate::theme::*;

/// The box, and what it was last given.
pub(super) struct DebugBox {
    pub ron: NodeId,
    pub error: NodeId,
    /// Whose text it holds: an entity, or the scene (`None`).
    pub whose: Option<EntityId>,
    pub given: String,
}

impl Inspector {
    /// The text Debug mode shows for `whose`.
    fn debug_text(session: &Session, whose: Option<EntityId>) -> String {
        let text = match whose {
            Some(id) => session.entity_ron(id),
            None => session.scene_settings_ron(),
        };
        text.unwrap_or_else(|e| format!("// {e}"))
    }

    /// Lay out Debug mode for `ids`: one box, or a note when several are
    /// shown.
    pub(super) fn build_debug(&mut self, ui: &mut Ui, session: &Session, ids: &[EntityId]) {
        self.debug_box = None;
        let column = ui.add(
            self.body,
            Style::column()
                .full_width()
                .padding_x(SPACE_4)
                .padding_y(SPACE_2)
                .gap(SPACE_2),
        );
        let caption = |ui: &mut Ui, text: &str| {
            ui.add_text(
                column,
                Style::default().text_size(11.5).text_color(MUTED),
                text,
            );
        };
        if ids.len() > 1 {
            caption(
                ui,
                &format!(
                    "{} selected. Debug mode edits one thing at a time: select one, or switch to Normal to edit them together.",
                    ids.len()
                ),
            );
            return;
        }
        let whose = ids.first().copied();
        let head = ui.add(
            column,
            Style::row().full_width().gap(SPACE_2).center_items(),
        );
        let what = match whose {
            Some(id) => session.entity_name(id).unwrap_or_default(),
            None => "Scene settings".to_string(),
        };
        ui.add_text(head, text().fill().text_size(12.5), &what);
        tag(ui, head, "Debug", ACCENT_900, ACCENT_300);
        let ron_text = Self::debug_text(session, whose);
        let ron = ui.add_textarea(
            column,
            field_style()
                .full_width()
                .auto_height()
                .min_height(120.0)
                .padding_y(SPACE_2)
                .mono()
                .text_size(11.5),
            &ron_text,
        );
        ui.set_name(ron, "inspector ron");
        self.parts.insert(ron, Part::DebugRon);
        let error = ui.add_text(
            column,
            Style::default()
                .text_size(11.5)
                .text_color(ERROR)
                .mono()
                .hidden(),
            "",
        );
        ui.set_name(error, "inspector ron error");
        let foot = ui.add(
            column,
            Style::row().full_width().gap(SPACE_2).center_items(),
        );
        ui.add_text(
            foot,
            Style::default().fill().text_size(11.5).text_color(MUTED),
            match whose {
                Some(_) => "As the scene file writes it. Children are kept: they are lines of their own. Cmd/Ctrl Enter applies.",
                None => "Everything but the entities, as the scene file writes it. Cmd/Ctrl Enter applies.",
            },
        );
        let apply = ui.add(
            foot,
            Style::row()
                .height(22.0)
                .fixed()
                .padding_x(SPACE_3)
                .center()
                .radius(6.0)
                .border(1.0, DIVIDER)
                .hover(HOVER)
                .hover_border(ACCENT)
                .clickable(),
        );
        ui.add_text(apply, text().text_size(12.0), "Apply");
        ui.set_name(apply, "inspector ron apply");
        self.parts.insert(apply, Part::DebugApply);
        self.debug_box = Some(DebugBox {
            ron,
            error,
            whose,
            given: ron_text,
        });
    }

    /// Keep the box on what the document says — after an undo, an edit
    /// elsewhere — unless the person is in the middle of changing it.
    pub(super) fn update_debug(&mut self, ui: &mut Ui, session: &Session) {
        let Some(b) = &mut self.debug_box else {
            return;
        };
        let now = Self::debug_text(session, b.whose);
        if now == b.given {
            return;
        }
        let typed = ui.text(b.ron).unwrap_or_default();
        let editing = ui.focused() == Some(b.ron) && typed != b.given;
        if !editing {
            ui.set_text(b.ron, &now);
            say(ui, b.error, "");
            b.given = now;
        }
    }

    /// Apply what the box holds: one undo step, or the reason not under it.
    pub(super) fn apply_debug(&mut self, ui: &mut Ui, session: &mut Session) {
        let Some(b) = &mut self.debug_box else {
            return;
        };
        let typed = ui.text(b.ron).unwrap_or_default().to_string();
        let done = match b.whose {
            Some(id) => session.set_entity_ron(id, &typed),
            None => session.set_scene_settings_ron(&typed),
        };
        match done {
            Ok(()) => {
                // What the document holds now, as the file writes it.
                let now = Self::debug_text(session, b.whose);
                ui.set_text(b.ron, &now);
                say(ui, b.error, "");
                b.given = now;
            }
            Err(e) => say(ui, b.error, &e.to_string()),
        }
    }
}

/// The line under the box: why the text did not apply, or nothing (and no
/// room taken).
fn say(ui: &mut Ui, line: NodeId, text: &str) {
    ui.set_text(line, text);
    ui.restyle(line, |s| {
        if text.is_empty() {
            s.hidden()
        } else {
            s.shown()
        }
    });
}
