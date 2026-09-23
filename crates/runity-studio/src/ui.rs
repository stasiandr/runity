//! Nocturne's components, as GPUI elements.
//!
//! Each is the class of the same name in the design system's `styles.css`,
//! drawn with the tokens in [`crate::theme`]: a panel is a `.card`, a
//! toolbar button a `.btn-ghost` or `.btn-icon`, the tool switch a `.seg`,
//! the Console's counts `.tag`s. Written here once so that the panels read
//! as one page and a change to the look is a change to one file.
//!
//! Icons are Lucide, from GPUI Kit's bundle. Nocturne names Phosphor; the
//! two are drawn to the same 24-unit grid at the same stroke, and swapping
//! the set is swapping [`icon`]'s path.

use gpui::{
    div, prelude::*, px, rgb, svg, AnyElement, Div, ElementId, Rgba, SharedString, Stateful, Svg,
};

use crate::theme::{self, *};

/// A Lucide icon by its file name — `play`, `move-3d`.
pub fn icon(name: &str, color: impl Into<Rgba>) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(14.0))
        .flex_none()
        .text_color(color.into())
}

/// A panel: Nocturne's `.card` — the surface on the ground, 8px round,
/// with its title as an `h6` (small caps, spaced, muted).
pub fn panel(title: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .size_full()
        .overflow_hidden()
        .rounded(RADIUS_MD)
        .bg(rgb(SURFACE))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .h(px(30.0))
                .flex_none()
                .px(SPACE_4)
                .text_size(px(10.5))
                .text_color(muted())
                .child(title.to_uppercase()),
        )
}

/// A panel's title row with something at its right end — a count, a
/// button. The same `h6` as [`panel`], with room.
pub fn panel_with(title: &'static str, right: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .size_full()
        .overflow_hidden()
        .rounded(RADIUS_MD)
        .bg(rgb(SURFACE))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .h(px(30.0))
                .flex_none()
                .pl(SPACE_4)
                .pr(SPACE_2)
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.5))
                        .text_color(muted())
                        .child(title.to_uppercase()),
                )
                .child(right),
        )
}

/// `.btn-icon` as a ghost: a square, transparent until hovered.
/// `on` draws it as the thing that is currently chosen: the accent as a
/// line and a tint, never a fill.
pub fn icon_button(id: impl Into<ElementId>, name: &str, on: bool) -> Stateful<Div> {
    let color = if on { rgb(ACCENT) } else { label() };
    let base = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(26.0))
        .rounded(RADIUS_MD)
        .cursor_pointer()
        .border_1()
        .child(icon(name, color));
    if on {
        base.border_color(rgb(ACCENT))
            .bg(accent_hover())
            .hover(|s| s.bg(theme::mix(ACCENT, 18)))
    } else {
        base.border_color(gpui::transparent_black())
            .hover(|s| s.bg(hover()))
            .active(|s| s.bg(pressed()))
    }
}

/// `.btn` with text: `.btn-primary` (an accent outline) when `primary`,
/// `.btn-secondary` (a divider outline) otherwise.
pub fn button(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    primary: bool,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(26.0))
        .px(SPACE_3)
        .rounded(RADIUS_MD)
        .border_1()
        .cursor_pointer()
        .text_size(px(12.5))
        .child(text.into());
    if primary {
        base.text_color(rgb(ACCENT))
            .border_color(rgb(ACCENT))
            .hover(|s| s.bg(accent_hover()))
            .active(|s| s.bg(theme::mix(ACCENT, 22)))
    } else {
        base.text_color(rgb(TEXT))
            .border_color(divider())
            .hover(|s| s.bg(hover()))
            .active(|s| s.bg(pressed()))
    }
}

/// `.seg`: options side by side in one outline, the chosen one drawn in
/// the accent as an inset line.
pub fn segmented(children: impl IntoIterator<Item = AnyElement>) -> Div {
    div()
        .flex()
        .flex_none()
        .h(px(26.0))
        .overflow_hidden()
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(divider())
        .children(children)
}

/// One `.seg-opt`: an icon and, when there is room, a word.
pub fn segment(
    id: impl Into<ElementId>,
    name: &str,
    text: Option<&'static str>,
    on: bool,
    first: bool,
) -> Stateful<Div> {
    let color = if on { rgb(ACCENT) } else { label() };
    let mut opt = div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.0))
        .h_full()
        .px(SPACE_3)
        .cursor_pointer()
        .text_size(px(12.0))
        .text_color(color)
        .child(icon(name, color));
    if let Some(text) = text {
        opt = opt.child(text);
    }
    if !first {
        opt = opt.border_l_1().border_color(divider());
    }
    if on {
        opt.bg(theme::mix(ACCENT, 10))
    } else {
        opt.hover(|s| s.bg(hover()))
    }
}

/// `.tag`: a small label tinted from a ramp.
pub fn tag(text: impl Into<SharedString>, fill: u32, ink: u32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.0))
        .h(px(18.0))
        .px(px(7.0))
        .rounded(px(6.0))
        .bg(rgb(fill))
        .text_color(rgb(ink))
        .text_size(px(10.5))
        .child(text.into())
}

/// A vertical hairline between groups of a toolbar.
pub fn separator() -> Div {
    div().w(px(1.0)).h(px(16.0)).mx(SPACE_1).bg(divider())
}
