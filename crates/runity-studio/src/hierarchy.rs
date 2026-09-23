//! The Hierarchy: the document's lines, as a tree to click.
//!
//! What a line *is* — its depth, whether it is open, selected, hidden, an
//! instance of a prefab — is `Session::hierarchy`, tested without a window.
//! This draws it and turns a click into the same call an agent makes: a
//! click selects, Shift or Cmd adds, a double click frames it in the Scene
//! view, the arrow opens a line, the eye hides it, the lock keeps clicks in
//! the view off it, and dragging a line onto another makes it a child.

use std::ops::Range;

use gpui::{
    div, prelude::*, px, rgb, uniform_list, App, ClickEvent, Context, Entity, FocusHandle,
    Focusable, KeyDownEvent, Pixels, Point, Render, SharedString, UniformListScrollHandle, Window,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use runity::EntityId;
use runity_editor::panels::Row;
use runity_editor::Session;

use crate::theme::*;
use crate::ui::{icon, panel_with, tag};
use crate::viewport::SceneView;

/// How far each level of the tree steps in.
const INDENT: f32 = 14.0;
const ROW_HEIGHT: f32 = 24.0;

pub struct Hierarchy {
    session: Entity<Session>,
    scene_view: Entity<SceneView>,
    search: Entity<InputState>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

/// A line being dragged, and what it looks like under the cursor.
#[derive(Clone)]
struct Dragged {
    id: EntityId,
    name: SharedString,
}

impl Render for Dragged {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(SPACE_3)
            .py(SPACE_1)
            .rounded(RADIUS_SM)
            .bg(rgb(ACCENT_900))
            .border_1()
            .border_color(rgb(ACCENT))
            .text_size(px(12.5))
            .text_color(rgb(ACCENT_200))
            .child(self.name.clone())
    }
}

impl Hierarchy {
    pub fn new(
        session: Entity<Session>,
        scene_view: Entity<SceneView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        Self {
            session,
            scene_view,
            search,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        }
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let query = self.search.read(cx).value();
        let session = self.session.read(cx);
        if query.trim().is_empty() {
            session.hierarchy()
        } else {
            session.hierarchy_matching(query.trim()).unwrap_or_default()
        }
    }

    fn click(
        &mut self,
        id: EntityId,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.modifiers();
        let add = modifiers.shift || modifiers.platform || modifiers.control;
        let twice = event.click_count() >= 2;
        self.session.update(cx, |session, cx| {
            let _ = if add {
                session.add_to_selection(id)
            } else {
                session.select(Some(id))
            };
            if twice {
                session.focus_selected();
            }
            cx.notify();
        });
        // The keyboard follows the click: F, Delete and Ctrl D after
        // choosing a line act on it, as in Unity.
        window.focus(&self.focus, cx);
    }

    fn render_row(&self, row: Row, cx: &mut Context<Self>) -> impl IntoElement {
        let id = row.id;
        let name: SharedString = if row.name.is_empty() {
            "(unnamed)".into()
        } else {
            row.name.clone().into()
        };
        let ink = if row.selected {
            rgb(ACCENT_200)
        } else if row.hidden {
            muted()
        } else if row.part {
            label()
        } else {
            rgb(TEXT)
        };

        let chevron = if row.has_children {
            div()
                .id(("open", id.raw()))
                .flex()
                .items_center()
                .justify_center()
                .size(px(16.0))
                .rounded(RADIUS_SM)
                .hover(|s| s.bg(hover()))
                .child(icon(
                    if row.open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    muted(),
                ))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.session.update(cx, |session, cx| {
                        session.set_open(id, !row.open);
                        cx.notify();
                    });
                }))
                .into_any_element()
        } else {
            div().size(px(16.0)).into_any_element()
        };

        let kind = if row.prefab.is_some() {
            icon("package", rgb(ACCENT))
        } else if row.has_children {
            icon("boxes", muted())
        } else {
            icon("box", muted())
        };

        let eye = div()
            .id(("eye", id.raw()))
            .flex()
            .items_center()
            .justify_center()
            .size(px(20.0))
            .rounded(RADIUS_SM)
            .hover(|s| s.bg(hover()))
            .child(icon(
                if row.hidden { "eye-off" } else { "eye" },
                if row.hidden { label() } else { mix(TEXT, 30) },
            ))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.session.update(cx, |session, cx| {
                    let _ = session.set_hidden(&[id], !row.hidden);
                    cx.notify();
                });
            }));
        let lock = div()
            .id(("lock", id.raw()))
            .flex()
            .items_center()
            .justify_center()
            .size(px(20.0))
            .rounded(RADIUS_SM)
            .hover(|s| s.bg(hover()))
            .child(icon(
                if row.locked { "lock" } else { "lock-open" },
                if row.locked { label() } else { mix(TEXT, 30) },
            ))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.session.update(cx, |session, cx| {
                    let _ = session.set_pickable(&[id], row.locked);
                    cx.notify();
                });
            }));

        let dragged = Dragged {
            id,
            name: name.clone(),
        };
        let mut line = div()
            .id(("row", id.raw()))
            .group("row")
            .flex()
            .items_center()
            .gap(SPACE_1)
            .h(px(ROW_HEIGHT))
            .pl(px(6.0 + row.depth as f32 * INDENT))
            .pr(SPACE_2)
            .w_full()
            .rounded(RADIUS_SM)
            .border_1()
            .border_color(gpui::transparent_black())
            .text_size(px(12.5))
            .text_color(ink)
            .cursor_pointer()
            .child(chevron)
            .child(kind)
            .child(
                div()
                    .flex_1()
                    .ml(SPACE_1)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(name),
            )
            .children(row.prefab.clone().map(|p| tag(p, ACCENT_900, ACCENT_300)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .when(!row.hidden && !row.locked, |d| {
                        d.invisible().group_hover("row", |s| s.visible())
                    })
                    .child(eye)
                    .child(lock),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click(id, event, window, cx);
            }))
            .on_drag(dragged, |dragged, _: Point<Pixels>, _, cx| {
                cx.new(|_| dragged.clone())
            })
            .drag_over::<Dragged>(|s, _, _, _| s.border_color(rgb(ACCENT)).bg(mix(ACCENT, 12)))
            .on_drop(cx.listener(move |this, dragged: &Dragged, _, cx| {
                if dragged.id == id {
                    return;
                }
                this.session.update(cx, |session, cx| {
                    if let Err(e) = session.reparent(dragged.id, Some(id)) {
                        session.say(runity_editor::console::Level::Warning, e.to_string());
                    }
                    session.set_open(id, true);
                    cx.notify();
                });
            }));
        if row.selected {
            line = line.bg(rgb(ACCENT_900)).border_color(mix(ACCENT, 40));
        } else {
            line = line.hover(|s| s.bg(mix(TEXT, 5)));
        }
        // The list lays its lines out at their own width; this one takes
        // the panel's, less the margin the card keeps.
        div().w_full().px(SPACE_2).child(line)
    }
}

impl Focusable for Hierarchy {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Hierarchy {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.session.read(cx).entity_count();
        let rows = self.rows(cx);
        let len = rows.len();
        let list = uniform_list(
            "hierarchy-rows",
            len,
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                // Read again, not captured: the list asks for its lines
                // after this render, and the scene may have moved on.
                let rows = this.rows(cx);
                range
                    .filter_map(|i| rows.get(i).cloned())
                    .map(|row| this.render_row(row, cx).into_any_element())
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .pb(SPACE_2);

        panel_with(
            "Hierarchy",
            div()
                .text_size(px(11.0))
                .text_color(muted())
                .child(format!("{count}")),
        )
        .id("hierarchy")
        .track_focus(&self.focus)
        .key_context("Hierarchy")
        // The Scene view's shortcuts, from here: Delete, F, Ctrl D, Ctrl Z
        // on the lines just chosen.
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            // Not while typing into the search: those letters are a name.
            if this.search.read(cx).focus_handle(cx).is_focused(window) {
                return;
            }
            this.scene_view
                .update(cx, |view, cx| view.key_down(event, cx));
        }))
        .on_key_up(cx.listener(|this, event: &gpui::KeyUpEvent, _, cx| {
            this.scene_view
                .update(cx, |view, cx| view.key_up(event, cx));
        }))
        .child(
            div().px(SPACE_2).pb(SPACE_2).child(
                Input::new(&self.search)
                    .prefix(icon("search", muted()))
                    .cleanable(true),
            ),
        )
        .child(list)
        // Dropped on the panel rather than a line: to the top of the tree.
        .on_drop(cx.listener(|this, dragged: &Dragged, _, cx| {
            this.session.update(cx, |session, cx| {
                let _ = session.reparent(dragged.id, None);
                cx.notify();
            });
        }))
    }
}
