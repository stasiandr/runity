//! The Console: what the editor has to say.
//!
//! The lines are `Session::console` — what opening a scene skipped, an
//! import warned about, an edit the Scene view refused, what a running game
//! printed — collapsed as Unity's Collapse does, a repeat counting up. This
//! draws them, newest at the bottom and followed while you are there, with
//! the counts by level and a way to clear them.

use std::ops::Range;

use gpui::{
    div, prelude::*, px, rgb, uniform_list, ClickEvent, Context, Entity, Render, ScrollStrategy,
    UniformListScrollHandle, Window,
};
use runity_editor::console::{Level, Line};
use runity_editor::Session;

use crate::theme::*;
use crate::ui::{icon, icon_button, panel_with, tag};

pub struct Console {
    session: Entity<Session>,
    scroll: UniformListScrollHandle,
    /// How many lines there were at the last draw: more means scroll down.
    seen: usize,
    /// Show only this level and worse.
    at_least: Level,
}

impl Console {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            scroll: UniformListScrollHandle::new(),
            seen: 0,
            at_least: Level::Info,
        }
    }

    fn lines(&self, cx: &gpui::App) -> Vec<Line> {
        self.session
            .read(cx)
            .console()
            .iter()
            .filter(|l| l.level >= self.at_least)
            .cloned()
            .collect()
    }
}

fn render_line(line: &Line, ix: usize) -> impl IntoElement {
    let (glyph, ink) = match line.level {
        Level::Info => ("info", muted()),
        Level::Warning => ("triangle-alert", rgb(WARNING)),
        Level::Error => ("circle-alert", rgb(ERROR)),
    };
    let text_ink = match line.level {
        Level::Info => label(),
        Level::Warning => rgb(WARNING),
        Level::Error => rgb(ERROR),
    };
    let first = line.text.lines().next().unwrap_or_default().to_string();
    div()
        .id(("console-line", ix))
        .flex()
        .items_center()
        .gap(SPACE_2)
        .h(px(22.0))
        .px(SPACE_4)
        .hover(|s| s.bg(mix(TEXT, 4)))
        .child(icon(glyph, ink))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_family(MONO)
                .text_size(px(11.5))
                .text_color(text_ink)
                .child(first),
        )
        .when(line.count > 1, |d| {
            d.child(tag(format!("{}", line.count), NEUTRAL_800, NEUTRAL_300))
        })
}

impl Render for Console {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (info, warnings, errors) = self.session.read(cx).console_counts();
        let len = self.lines(cx).len();
        if len > self.seen {
            self.scroll
                .scroll_to_item(len.saturating_sub(1), ScrollStrategy::Bottom);
        }
        self.seen = len;

        let filter =
            |id: &'static str, level: Level, glyph: &'static str, count: usize, ink: u32| {
                let on = self.at_least == level && level != Level::Info;
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .h(px(22.0))
                    .px(SPACE_2)
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(if on {
                        mix(ACCENT, 60)
                    } else {
                        gpui::transparent_black().into()
                    })
                    .cursor_pointer()
                    .hover(|s| s.bg(hover()))
                    .text_size(px(11.0))
                    .text_color(label())
                    .child(icon(glyph, rgb(ink)))
                    .child(format!("{count}"))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.at_least = if this.at_least == level {
                            Level::Info
                        } else {
                            level
                        };
                        this.seen = 0;
                        cx.notify();
                    }))
            };

        let right = div()
            .flex()
            .items_center()
            .gap(SPACE_1)
            .child(filter("f-info", Level::Info, "info", info, NEUTRAL_500))
            .child(filter(
                "f-warn",
                Level::Warning,
                "triangle-alert",
                warnings,
                WARNING,
            ))
            .child(filter("f-err", Level::Error, "circle-alert", errors, ERROR))
            .child(
                icon_button("console-clear", "x", false).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.clear_console();
                            cx.notify();
                        });
                        this.seen = 0;
                    },
                )),
            );

        let body = if len == 0 {
            div()
                .flex_1()
                .px(SPACE_4)
                .text_size(px(12.0))
                .text_color(mix(TEXT, 40))
                .child("Nothing to say.")
                .into_any_element()
        } else {
            uniform_list(
                "console-lines",
                len,
                cx.processor(|this, range: Range<usize>, _, cx| {
                    let lines = this.lines(cx);
                    range
                        .filter_map(|i| lines.get(i).map(|l| render_line(l, i)))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .flex_1()
            .into_any_element()
        };

        panel_with("Console", right).child(body)
    }
}
