//! Buttons, toggles and sliders for a game's own screens.
//!
//! Unity's UGUI Button, Toggle and Slider, immediate-mode: a function call
//! a frame draws the widget into the [`Ui`] list and says what the player
//! did to it. No retained tree, no layout engine, no event system to wire
//! — a menu is the code that draws it, and a hot-patched `frame` changes it
//! while the game runs.
//!
//! The [`Ui`] list stays a list; the one thing a widget has to remember
//! between frames — which of them the mouse went down on — lives here, so
//! that a press that starts on "Quit" and ends on "Play" is neither.
//!
//! A gamepad works them too, as Unity's automatic navigation: call
//! [`Widgets::begin_frame`] before drawing them, and the d-pad or the left
//! stick moves a highlight to the nearest widget that way (by last
//! frame's places), South presses what is highlighted, East closes an open
//! list or lets go of a text field. Left and right move a highlighted
//! slider; up and down, an open list's choice. The mouse moving hides the
//! highlight again.

use glam::Vec4;

use crate::input::{Input, MouseButton};
use crate::ui::{Quad, TextRun, Ui};

/// What widgets look like.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub idle: Vec4,
    pub hover: Vec4,
    pub pressed: Vec4,
    pub accent: Vec4,
    pub text: Vec4,
    pub text_size: f32,
    /// Pixels a button's and a panel's corners are rounded by.
    pub radius: f32,
    /// A button's darker lip under its face, pixels: pressed, it sinks.
    pub bevel: f32,
    /// A panel's shadow, pixels down and right; none at 0.
    pub shadow: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            idle: Vec4::new(0.16, 0.17, 0.19, 0.9),
            hover: Vec4::new(0.24, 0.26, 0.29, 0.95),
            pressed: Vec4::new(0.10, 0.11, 0.12, 0.95),
            accent: Vec4::new(0.78, 0.62, 0.32, 1.0),
            text: Vec4::new(0.93, 0.93, 0.90, 1.0),
            text_size: 18.0,
            radius: 0.0,
            bevel: 0.0,
            shadow: 0.0,
        }
    }
}

/// Where a widget goes, in pixels from the top left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn contains(&self, at: glam::Vec2) -> bool {
        at.x >= self.x
            && at.y >= self.y
            && at.x < self.x + self.width
            && at.y < self.y + self.height
    }

    /// The next rect below this one, `gap` pixels down: a column of
    /// buttons without a layout engine.
    pub fn below(&self, gap: f32) -> Self {
        Self {
            y: self.y + self.height + gap,
            ..*self
        }
    }
}

/// What typing into a text field did this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Typed {
    /// The text is different.
    pub changed: bool,
    /// Enter was pressed in it.
    pub submitted: bool,
}

/// The widgets' memory between frames.
#[derive(Debug, Default)]
pub struct Widgets {
    /// The widget the mouse went down on, while it is held.
    active: Option<u64>,
    /// The text field typing goes into.
    focused: Option<u64>,
    /// The dropdown whose options are showing.
    open: Option<u64>,
    /// What the pad has highlighted, and whether the pad is in use.
    selected: Option<u64>,
    pad: bool,
    /// Widgets drawn this frame and last: where the pad can go.
    seen: Vec<(u64, Rect, Nav)>,
    last: Vec<(u64, Rect, Nav)>,
    /// South this frame, on what is highlighted.
    press: bool,
    /// A highlighted slider's or open list's step this frame: −1, 0 or 1.
    nudge: f32,
    stick_was: glam::Vec2,
    pub style: Style,
}

/// How the pad treats a widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nav {
    Press,
    Slider,
    List,
}

fn key(rect: Rect, label: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for v in [rect.x, rect.y, rect.width, rect.height] {
        v.to_bits().hash(&mut hasher);
    }
    label.hash(&mut hasher);
    hasher.finish()
}

impl Widgets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Widgets drawn in the game's own colours and shapes.
    pub fn with_style(style: Style) -> Self {
        Self {
            style,
            ..Self::default()
        }
    }

    /// Take the pad's moves for this frame: call once before drawing the
    /// widgets. Without it they answer the mouse only.
    pub fn begin_frame(&mut self, input: &Input) {
        use crate::input::{PadAxis, PadButton};
        self.last = std::mem::take(&mut self.seen);
        self.press = false;
        self.nudge = 0.0;
        if input.mouse_motion() != glam::Vec2::ZERO || input.mouse_pressed(MouseButton::Left) {
            self.pad = false;
        }
        // Screen directions, y down.
        let stick = glam::Vec2::new(
            input.pad_axis(PadAxis::LeftX),
            -input.pad_axis(PadAxis::LeftY),
        );
        let pushed = stick.length() > 0.5 && self.stick_was.length() <= 0.5;
        self.stick_was = stick;
        let direction = if input.pad_pressed(PadButton::DPadUp) {
            Some(glam::Vec2::NEG_Y)
        } else if input.pad_pressed(PadButton::DPadDown) {
            Some(glam::Vec2::Y)
        } else if input.pad_pressed(PadButton::DPadLeft) {
            Some(glam::Vec2::NEG_X)
        } else if input.pad_pressed(PadButton::DPadRight) {
            Some(glam::Vec2::X)
        } else if pushed {
            Some(if stick.x.abs() > stick.y.abs() {
                glam::Vec2::new(stick.x.signum(), 0.0)
            } else {
                glam::Vec2::new(0.0, stick.y.signum())
            })
        } else {
            None
        };
        let south = input.pad_pressed(PadButton::South);
        if input.pad_pressed(PadButton::East) {
            self.open = None;
            self.focused = None;
        }
        if self.last.is_empty() || (direction.is_none() && !south) {
            return;
        }
        let current = self
            .selected
            .and_then(|id| self.last.iter().find(|(i, _, _)| *i == id).copied());
        let woke = !self.pad;
        self.pad = true;
        let Some((id, rect, nav)) = current else {
            // Nothing highlighted yet: the top-left one.
            self.selected = self
                .last
                .iter()
                .min_by(|a, b| (a.1.y, a.1.x).partial_cmp(&(b.1.y, b.1.x)).unwrap())
                .map(|(id, _, _)| *id);
            return;
        };
        if south {
            self.press = !woke;
        }
        let Some(direction) = direction.filter(|_| !woke) else {
            return;
        };
        if nav == Nav::Slider && direction.x != 0.0 {
            self.nudge = direction.x;
            return;
        }
        if nav == Nav::List && self.open == Some(id) && direction.y != 0.0 {
            self.nudge = direction.y;
            return;
        }
        // The nearest one that way, what is off to the side counting
        // double.
        let centre = |r: &Rect| glam::Vec2::new(r.x + r.width * 0.5, r.y + r.height * 0.5);
        let from = centre(&rect);
        let next = self
            .last
            .iter()
            .filter(|(i, _, _)| *i != id)
            .filter_map(|(i, r, _)| {
                let d = centre(r) - from;
                let along = d.dot(direction);
                (along > 1.0).then(|| (*i, along + 2.0 * (d - direction * along).length()))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((next, _)) = next {
            self.selected = Some(next);
            self.focused = None;
        }
    }

    /// Whether the pad has this widget highlighted.
    fn highlighted(&self, id: u64) -> bool {
        self.pad && self.selected == Some(id)
    }

    /// [`Self::track`] for a widget the pad can reach: highlighted counts
    /// as over, South on it as a click.
    fn reach(&mut self, input: &Input, rect: Rect, id: u64, nav: Nav) -> (bool, bool, bool) {
        self.seen.push((id, rect, nav));
        let (over, held, clicked) = self.track(input, rect, id);
        let lit = self.highlighted(id);
        (over || lit, held, clicked || (lit && self.press))
    }

    /// The pad's highlight round a widget, drawn over it.
    fn ring(&self, ui: &mut Ui, rect: Rect, id: u64) {
        if !self.highlighted(id) {
            return;
        }
        let (c, t) = (self.style.accent, 2.0);
        ui.quad(Quad::new(
            rect.x - t,
            rect.y - t,
            rect.width + 2.0 * t,
            t,
            c,
        ));
        ui.quad(Quad::new(
            rect.x - t,
            rect.y + rect.height,
            rect.width + 2.0 * t,
            t,
            c,
        ));
        ui.quad(Quad::new(rect.x - t, rect.y, t, rect.height, c));
        ui.quad(Quad::new(rect.x + rect.width, rect.y, t, rect.height, c));
    }

    /// Where the mouse is relative to a widget this frame: whether it is
    /// over it, whether it is held on it, and whether it was just released
    /// on it having gone down on it.
    fn track(&mut self, input: &Input, rect: Rect, id: u64) -> (bool, bool, bool) {
        let over = rect.contains(input.mouse_position());
        if over && input.mouse_pressed(MouseButton::Left) {
            self.active = Some(id);
        }
        let held = self.active == Some(id) && input.mouse_held(MouseButton::Left);
        let clicked = over && self.active == Some(id) && input.mouse_released(MouseButton::Left);
        if self.active == Some(id) && !input.mouse_held(MouseButton::Left) {
            self.active = None;
        }
        (over, held, clicked)
    }

    fn label(&self, ui: &mut Ui, rect: Rect, text: &str) {
        let size = self.style.text_size;
        ui.text(
            TextRun::new(
                rect.x,
                rect.y + (rect.height - size) * 0.5,
                size,
                self.style.text,
                text,
            )
            .within(rect.width, 0.5),
        );
    }

    /// A button. `true` on the frame it is clicked — released over it,
    /// having been pressed on it.
    pub fn button(&mut self, ui: &mut Ui, input: &Input, rect: Rect, label: &str) -> bool {
        let id = key(rect, label);
        let (over, held, clicked) = self.reach(input, rect, id, Nav::Press);
        let color = match (held, over) {
            (true, _) => self.style.pressed,
            (false, true) => self.style.hover,
            _ => self.style.idle,
        };
        let r = self.style.radius;
        let bevel = self.style.bevel;
        if bevel > 0.0 {
            // The lip: the face's colour, darker, showing under it.
            let lip = Vec4::new(color.x * 0.6, color.y * 0.6, color.z * 0.6, color.w);
            ui.quad(Quad::new(rect.x, rect.y, rect.width, rect.height, lip).rounded(r));
            let sunk = if held { bevel * 0.6 } else { 0.0 };
            let face = Rect::new(rect.x, rect.y + sunk, rect.width, rect.height - bevel);
            ui.quad(Quad::new(face.x, face.y, face.width, face.height, color).rounded(r));
            self.label(ui, face, label);
        } else {
            ui.quad(Quad::new(rect.x, rect.y, rect.width, rect.height, color).rounded(r));
            self.label(ui, rect, label);
        }
        self.ring(ui, rect, id);
        clicked
    }

    /// A box that is on or off. `true` on the frame it changes.
    pub fn toggle(
        &mut self,
        ui: &mut Ui,
        input: &Input,
        rect: Rect,
        label: &str,
        value: &mut bool,
    ) -> bool {
        let id = key(rect, label);
        let (over, _, clicked) = self.reach(input, rect, id, Nav::Press);
        if clicked {
            *value = !*value;
        }
        let back = if over {
            self.style.hover
        } else {
            self.style.idle
        };
        ui.quad(
            Quad::new(rect.x, rect.y, rect.width, rect.height, back).rounded(self.style.radius),
        );
        let mark = rect.height * 0.5;
        let fill = if *value {
            self.style.accent
        } else {
            self.style.pressed
        };
        ui.quad(Quad::new(
            rect.x + mark * 0.5,
            rect.y + mark * 0.5,
            mark,
            mark,
            fill,
        ));
        ui.text(TextRun::new(
            rect.x + mark * 2.0,
            rect.y + (rect.height - self.style.text_size) * 0.5,
            self.style.text_size,
            self.style.text,
            label,
        ));
        self.ring(ui, rect, id);
        clicked
    }

    /// A line of text the player types — a name, a seed, a chat line:
    /// Unity's InputField. A click on it takes the keyboard, a click
    /// anywhere else or Escape lets it go; Backspace takes a character back,
    /// Enter submits and lets go. Shows `placeholder` greyed while empty.
    pub fn text_field(
        &mut self,
        ui: &mut Ui,
        input: &Input,
        rect: Rect,
        placeholder: &str,
        value: &mut String,
    ) -> Typed {
        let id = key(rect, placeholder);
        let (over, _, pressed) = self.reach(input, rect, id, Nav::Press);
        if pressed && self.highlighted(id) {
            self.focused = Some(id);
        }
        if input.mouse_pressed(MouseButton::Left) {
            self.focused = if over {
                Some(id)
            } else {
                self.focused.filter(|f| *f != id)
            };
        }
        let mut typed = Typed::default();
        if self.focused == Some(id) {
            let before = value.len();
            value.extend(input.text().chars().filter(|c| !c.is_control()));
            if input.pressed(crate::input::Key::Backspace) {
                value.pop();
            }
            typed.changed = value.len() != before || input.pressed(crate::input::Key::Backspace);
            if input.pressed(crate::input::Key::Enter) {
                typed.submitted = true;
                self.focused = None;
            } else if input.pressed(crate::input::Key::Escape) {
                self.focused = None;
            }
        }
        let focused = self.focused == Some(id);
        let back = if focused || over {
            self.style.hover
        } else {
            self.style.idle
        };
        ui.quad(
            Quad::new(rect.x, rect.y, rect.width, rect.height, back).rounded(self.style.radius),
        );
        let size = self.style.text_size;
        let (shown, color) = if value.is_empty() && !focused {
            (
                placeholder.to_string(),
                self.style.text * glam::Vec4::new(1.0, 1.0, 1.0, 0.45),
            )
        } else if focused {
            (format!("{value}|"), self.style.text)
        } else {
            (value.clone(), self.style.text)
        };
        ui.text(TextRun::new(
            rect.x + size * 0.5,
            rect.y + (rect.height - size) * 0.5,
            size,
            color,
            shown,
        ));
        self.ring(ui, rect, id);
        typed
    }

    /// One of a few options — Unity's Dropdown: shows the chosen one; a
    /// click lists them all below it, a click on one chooses it, a click
    /// anywhere else closes the list. `true` on the frame the choice
    /// changes.
    pub fn dropdown(
        &mut self,
        ui: &mut Ui,
        input: &Input,
        rect: Rect,
        label: &str,
        options: &[String],
        chosen: &mut usize,
    ) -> bool {
        let id = key(rect, label);
        let (over, _, clicked) = self.reach(input, rect, id, Nav::List);
        let mut changed = false;
        let was_open = self.open == Some(id);
        if was_open && self.highlighted(id) && self.nudge != 0.0 && !options.is_empty() {
            let at = (*chosen as i64 + self.nudge as i64).clamp(0, options.len() as i64 - 1);
            if at as usize != *chosen {
                *chosen = at as usize;
                changed = true;
            }
        }
        if was_open {
            // The options, below, one row each.
            for (i, option) in options.iter().enumerate() {
                let row = Rect::new(
                    rect.x,
                    rect.y + rect.height * (i + 1) as f32,
                    rect.width,
                    rect.height,
                );
                let hovered = row.contains(input.mouse_position());
                let color = if hovered || i == *chosen {
                    self.style.hover
                } else {
                    self.style.idle
                };
                ui.quad(Quad::new(row.x, row.y, row.width, row.height, color));
                self.label(ui, row, option);
                if hovered && input.mouse_released(MouseButton::Left) {
                    if *chosen != i {
                        *chosen = i;
                        changed = true;
                    }
                    self.open = None;
                }
            }
            if input.mouse_pressed(MouseButton::Left) && !over {
                let below = Rect::new(
                    rect.x,
                    rect.y + rect.height,
                    rect.width,
                    rect.height * options.len() as f32,
                );
                if !below.contains(input.mouse_position()) {
                    self.open = None;
                }
            }
        }
        if clicked {
            self.open = if was_open { None } else { Some(id) };
        }
        let back = if over {
            self.style.hover
        } else {
            self.style.idle
        };
        ui.quad(
            Quad::new(rect.x, rect.y, rect.width, rect.height, back).rounded(self.style.radius),
        );
        let shown = options.get(*chosen).map_or("", String::as_str);
        let text = if label.is_empty() {
            format!("{shown} ▾")
        } else {
            format!("{label}: {shown} ▾")
        };
        self.label(ui, rect, &text);
        self.ring(ui, rect, id);
        changed
    }

    /// A scrolled view — Unity's ScrollRect: a box showing part of
    /// something `content_height` tall, moved by the wheel over it or a
    /// finger dragged in it. Returns where the content's top is now; draw
    /// the content from there, then call `ui.pop_clip()` — everything in
    /// between is cut to the box. A bar on the right shows where it is.
    pub fn scroll(
        &mut self,
        ui: &mut Ui,
        input: &Input,
        rect: Rect,
        content_height: f32,
        offset: &mut f32,
    ) -> f32 {
        let id = key(rect, "scroll");
        let over = rect.contains(input.mouse_position());
        if over {
            *offset -= input.scroll().y * 40.0;
        }
        let (_, held, _) = self.track(input, rect, id);
        if held {
            *offset -= input.mouse_motion().y;
        }
        let most = (content_height - rect.height).max(0.0);
        *offset = offset.clamp(0.0, most);
        ui.quad(Quad::new(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            self.style.idle,
        ));
        if most > 0.0 {
            let shown = rect.height / content_height;
            let bar = rect.height * shown;
            let at = rect.y + (rect.height - bar) * (*offset / most);
            ui.quad(Quad::new(
                rect.x + rect.width - 4.0,
                at,
                4.0,
                bar,
                self.style.accent,
            ));
        }
        ui.push_clip(rect.x, rect.y, rect.width - 6.0, rect.height);
        rect.y - *offset
    }

    /// A stick drawn on the screen for a thumb — the phone's gamepad: a
    /// finger (or the mouse) that goes down inside `rect` drags the knob,
    /// and what comes back is −1..1 on each axis, y up, zero when let go.
    /// Feed it to movement like [`crate::Actions::axis`].
    pub fn stick(&mut self, ui: &mut Ui, input: &Input, rect: Rect) -> glam::Vec2 {
        let id = key(rect, "stick");
        let centre = glam::Vec2::new(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
        let reach = rect.width.min(rect.height) * 0.5;
        // A finger that came down in it, or the mouse held from it.
        let finger = input
            .touches()
            .iter()
            .find(|t| rect.contains(t.start) && !t.ended)
            .map(|t| t.position);
        let (_, held, _) = self.track(input, rect, id);
        let at = finger.or(held.then(|| input.mouse_position()));
        let offset = at.map_or(glam::Vec2::ZERO, |p| (p - centre).clamp_length_max(reach));
        ui.quad(Quad::new(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            self.style.idle,
        ));
        let knob = reach * 0.6;
        ui.quad(Quad::new(
            centre.x + offset.x - knob * 0.5,
            centre.y + offset.y - knob * 0.5,
            knob,
            knob,
            self.style.accent,
        ));
        glam::Vec2::new(offset.x, -offset.y) / reach.max(1e-3)
    }

    /// Whether a text field has the keyboard: while it does, the game
    /// should not read letters as its own keys.
    pub fn typing(&self) -> bool {
        self.focused.is_some()
    }

    /// A row of tabs across `rect` — a settings screen's Video, Sound,
    /// Controls: the chosen one lit. `true` on the frame another is
    /// chosen. Each tab is a widget of its own to the mouse and the pad.
    pub fn tabs(
        &mut self,
        ui: &mut Ui,
        input: &Input,
        rect: Rect,
        names: &[&str],
        chosen: &mut usize,
    ) -> bool {
        let mut changed = false;
        let width = rect.width / names.len().max(1) as f32;
        for (i, name) in names.iter().enumerate() {
            let tab = Rect::new(rect.x + width * i as f32, rect.y, width, rect.height);
            let id = key(tab, name);
            let (over, _, clicked) = self.reach(input, tab, id, Nav::Press);
            if clicked && *chosen != i {
                *chosen = i;
                changed = true;
            }
            let color = if i == *chosen {
                self.style.hover
            } else if over {
                self.style.pressed
            } else {
                self.style.idle
            };
            ui.quad(Quad::new(tab.x, tab.y, tab.width, tab.height, color));
            if i == *chosen {
                ui.quad(Quad::new(
                    tab.x,
                    tab.y + tab.height - 3.0,
                    tab.width,
                    3.0,
                    self.style.accent,
                ));
            }
            self.label(ui, tab, name);
            self.ring(ui, tab, id);
        }
        changed
    }

    /// Rows to pick one of — saves, servers, recipes: a row `row_height`
    /// tall for each item from `rect`'s top, the picked one lit, what does
    /// not fit cut off (put it in a [`Self::scroll`] for more). `true` on
    /// the frame the pick changes; `picked` is `None` for no pick yet.
    #[allow(clippy::too_many_arguments)]
    pub fn list(
        &mut self,
        ui: &mut Ui,
        input: &Input,
        rect: Rect,
        row_height: f32,
        items: &[String],
        picked: &mut Option<usize>,
    ) -> bool {
        let mut changed = false;
        ui.quad(Quad::new(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            self.style.pressed,
        ));
        for (i, item) in items.iter().enumerate() {
            let row = Rect::new(
                rect.x,
                rect.y + row_height * i as f32,
                rect.width,
                row_height,
            );
            if row.y + row.height > rect.y + rect.height + 0.5 {
                break;
            }
            let id = key(row, item);
            let (over, _, clicked) = self.reach(input, row, id, Nav::Press);
            if clicked && *picked != Some(i) {
                *picked = Some(i);
                changed = true;
            }
            let color = if *picked == Some(i) {
                self.style.hover
            } else if over {
                self.style.idle
            } else {
                self.style.pressed
            };
            ui.quad(Quad::new(row.x, row.y, row.width, row.height, color));
            let size = self.style.text_size;
            ui.text(TextRun::new(
                row.x + size * 0.5,
                row.y + (row.height - size) * 0.5,
                size,
                self.style.text,
                item,
            ));
            self.ring(ui, row, id);
        }
        changed
    }

    /// A value between `min` and `max`, set by dragging along the bar.
    /// `true` on frames it changes. The range is `min..=max`.
    pub fn slider(
        &mut self,
        ui: &mut Ui,
        input: &Input,
        rect: Rect,
        label: &str,
        value: &mut f32,
        range: std::ops::RangeInclusive<f32>,
    ) -> bool {
        let (min, max) = (*range.start(), *range.end());
        let id = key(rect, label);
        let (_, held, _) = self.reach(input, rect, id, Nav::Slider);
        let before = *value;
        if self.highlighted(id) && self.nudge != 0.0 {
            *value = (*value + self.nudge * (max - min) * 0.05).clamp(min.min(max), max.max(min));
        }
        if held && rect.width > 0.0 {
            let t = ((input.mouse_position().x - rect.x) / rect.width).clamp(0.0, 1.0);
            *value = min + (max - min) * t;
        }
        let t = if max > min {
            ((*value - min) / (max - min)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let r = self.style.radius;
        ui.quad(Quad::new(rect.x, rect.y, rect.width, rect.height, self.style.idle).rounded(r));
        ui.quad(
            Quad::new(
                rect.x,
                rect.y,
                rect.width * t,
                rect.height,
                self.style.accent,
            )
            .rounded(r),
        );
        self.label(ui, rect, &format!("{label}: {:.2}", *value));
        self.ring(ui, rect, id);
        *value != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::InputEvent;

    /// One frame of input: the mouse moved to `at`, then whatever events.
    fn frame(input: &mut Input, at: (f32, f32), events: &[InputEvent]) {
        input.begin_frame();
        input.handle(&InputEvent::MouseMoved { x: at.0, y: at.1 });
        for event in events {
            input.handle(event);
        }
    }

    const PLAY: Rect = Rect {
        x: 10.0,
        y: 10.0,
        width: 100.0,
        height: 30.0,
    };
    const QUIT: Rect = Rect {
        x: 10.0,
        y: 50.0,
        width: 100.0,
        height: 30.0,
    };

    #[test]
    fn a_pad_moves_the_highlight_between_widgets_and_presses_one() {
        use crate::input::PadButton;
        let (mut widgets, mut input) = (Widgets::new(), Input::new());
        let mut volume = 0.5;
        let slider = Rect::new(150.0, 10.0, 100.0, 30.0);
        let draw = |widgets: &mut Widgets, input: &Input, volume: &mut f32| {
            widgets.begin_frame(input);
            let mut ui = Ui::new();
            let play = widgets.button(&mut ui, input, PLAY, "Play");
            let quit = widgets.button(&mut ui, input, QUIT, "Quit");
            widgets.slider(&mut ui, input, slider, "Volume", volume, 0.0..=1.0);
            (play, quit)
        };
        let pad = |input: &mut Input, button| {
            input.begin_frame();
            input.handle(&InputEvent::PadUp(button));
            input.begin_frame();
            input.handle(&InputEvent::PadDown(button));
        };
        let idle = |input: &mut Input| input.begin_frame();
        idle(&mut input);
        draw(&mut widgets, &input, &mut volume);
        // The first press only wakes the highlight, on the top-left one.
        pad(&mut input, PadButton::DPadDown);
        draw(&mut widgets, &input, &mut volume);
        pad(&mut input, PadButton::South);
        assert_eq!(
            draw(&mut widgets, &input, &mut volume),
            (true, false),
            "Play"
        );
        // Down to Quit, and press it.
        pad(&mut input, PadButton::DPadDown);
        draw(&mut widgets, &input, &mut volume);
        pad(&mut input, PadButton::South);
        assert_eq!(
            draw(&mut widgets, &input, &mut volume),
            (false, true),
            "Quit"
        );
        // Right, to the slider (up and across), and right again moves it.
        pad(&mut input, PadButton::DPadRight);
        draw(&mut widgets, &input, &mut volume);
        pad(&mut input, PadButton::DPadRight);
        draw(&mut widgets, &input, &mut volume);
        assert!((volume - 0.55).abs() < 1e-5, "{volume}");
        // The mouse moving hides the highlight: South does nothing.
        frame(&mut input, (400.0, 400.0), &[]);
        draw(&mut widgets, &input, &mut volume);
        pad(&mut input, PadButton::South);
        let pressed = draw(&mut widgets, &input, &mut volume);
        assert_eq!(pressed, (false, false));
    }

    #[test]
    fn a_tab_and_a_row_are_chosen_by_a_click() {
        use crate::input::MouseButton as M;
        let (mut widgets, mut input) = (Widgets::new(), Input::new());
        let (mut tab, mut row) = (0, None);
        let items: Vec<String> = ["Autumn save", "Spring save"].map(String::from).to_vec();
        let mut draw = |widgets: &mut Widgets, input: &Input| {
            let mut ui = Ui::new();
            let tabs = widgets.tabs(
                &mut ui,
                input,
                Rect::new(0.0, 0.0, 300.0, 30.0),
                &["Video", "Sound", "Controls"],
                &mut tab,
            );
            let list = widgets.list(
                &mut ui,
                input,
                Rect::new(0.0, 40.0, 300.0, 100.0),
                25.0,
                &items,
                &mut row,
            );
            (tabs, list, tab, row)
        };
        let click = |input: &mut Input, at: (f32, f32)| {
            frame(input, at, &[InputEvent::MouseDown(M::Left)]);
        };
        click(&mut input, (150.0, 15.0));
        draw(&mut widgets, &input);
        frame(&mut input, (150.0, 15.0), &[InputEvent::MouseUp(M::Left)]);
        assert_eq!(draw(&mut widgets, &input), (true, false, 1, None), "Sound");
        click(&mut input, (50.0, 75.0));
        draw(&mut widgets, &input);
        frame(&mut input, (50.0, 75.0), &[InputEvent::MouseUp(M::Left)]);
        assert_eq!(
            draw(&mut widgets, &input),
            (false, true, 1, Some(1)),
            "the second row"
        );
    }

    #[test]
    fn a_dropdown_opens_on_a_click_and_a_click_on_an_option_chooses_it() {
        use crate::input::MouseButton as M;
        let (mut widgets, mut input) = (Widgets::new(), Input::new());
        let options: Vec<String> = ["English", "Русский", "Deutsch"].map(String::from).to_vec();
        let mut chosen = 0;
        let pick = |widgets: &mut Widgets, input: &Input, chosen: &mut usize| {
            widgets.dropdown(&mut Ui::new(), input, PLAY, "Language", &options, chosen)
        };
        let click = |input: &mut Input, at: (f32, f32)| {
            frame(input, at, &[InputEvent::MouseDown(M::Left)]);
        };
        // Open it: a press and a release on it.
        click(&mut input, (20.0, 20.0));
        pick(&mut widgets, &input, &mut chosen);
        frame(&mut input, (20.0, 20.0), &[InputEvent::MouseUp(M::Left)]);
        pick(&mut widgets, &input, &mut chosen);
        // The second option is the second row below it.
        let row = (20.0, 10.0 + 30.0 * 2.0 + 5.0);
        click(&mut input, row);
        pick(&mut widgets, &input, &mut chosen);
        frame(&mut input, row, &[InputEvent::MouseUp(M::Left)]);
        assert!(pick(&mut widgets, &input, &mut chosen));
        assert_eq!(chosen, 1);
        // Closed now: the same spot does nothing.
        click(&mut input, row);
        pick(&mut widgets, &input, &mut chosen);
        frame(&mut input, row, &[InputEvent::MouseUp(M::Left)]);
        assert!(!pick(&mut widgets, &input, &mut chosen));
        assert_eq!(chosen, 1);
    }

    #[test]
    fn a_scrolled_list_moves_with_the_wheel_and_draws_nothing_outside_its_box() {
        let (mut widgets, mut input, mut ui) = (Widgets::new(), Input::new(), Ui::new());
        let view = Rect::new(0.0, 0.0, 200.0, 100.0);
        let mut offset = 0.0;
        frame(
            &mut input,
            (50.0, 50.0),
            &[InputEvent::Scroll { x: 0.0, y: -1.0 }],
        );
        let top = widgets.scroll(&mut ui, &input, view, 400.0, &mut offset);
        assert_eq!(offset, 40.0, "a notch down");
        assert_eq!(top, -40.0);
        // Ten rows 40 high: only those in the box are drawn, cut to it.
        let before = ui.quads.len();
        for i in 0..10 {
            ui.quad(Quad::new(
                0.0,
                top + i as f32 * 40.0,
                150.0,
                30.0,
                glam::Vec4::ONE,
            ));
            ui.text(TextRun::new(
                4.0,
                top + i as f32 * 40.0,
                16.0,
                glam::Vec4::ONE,
                "row",
            ));
        }
        ui.pop_clip();
        let rows = &ui.quads[before..];
        assert_eq!(rows.len(), 3, "rows 1 to 3 show, the third cut: {rows:?}");
        assert_eq!(rows[2].height, 20.0);
        assert!(rows.iter().all(|q| q.y >= 0.0 && q.y + q.height <= 100.0));
        assert!(ui
            .texts
            .iter()
            .all(|t| t.clip == Some([0.0, 0.0, 194.0, 100.0])));
        // Past the end it stops.
        frame(
            &mut input,
            (50.0, 50.0),
            &[InputEvent::Scroll { x: 0.0, y: -100.0 }],
        );
        widgets.scroll(&mut Ui::new(), &input, view, 400.0, &mut offset);
        assert_eq!(offset, 300.0);
    }

    #[test]
    fn a_finger_is_the_mouse_for_buttons_and_a_thumb_drives_the_stick() {
        use crate::input::TouchPhase::*;
        let (mut widgets, mut input) = (Widgets::new(), Input::new());
        let touch = |input: &mut Input, id: u64, phase, x: f32, y: f32| {
            input.handle(&InputEvent::Touch { id, phase, x, y });
        };
        // A tap on Play clicks it, as a mouse would.
        input.begin_frame();
        touch(&mut input, 1, Started, 20.0, 20.0);
        assert!(!widgets.button(&mut Ui::new(), &input, PLAY, "Play"));
        input.begin_frame();
        touch(&mut input, 1, Ended, 20.0, 20.0);
        assert!(widgets.button(&mut Ui::new(), &input, PLAY, "Play"));
        input.begin_frame();
        assert!(input.touches().is_empty(), "gone the frame after");

        // A thumb pushed right and up from the middle of the stick.
        let pad = Rect::new(0.0, 200.0, 100.0, 100.0);
        input.begin_frame();
        touch(&mut input, 7, Started, 50.0, 250.0);
        touch(&mut input, 7, Moved, 150.0, 200.0);
        let v = widgets.stick(&mut Ui::new(), &input, pad);
        assert!(
            v.x > 0.85 && v.y > 0.4 && (v.length() - 1.0).abs() < 1e-4,
            "{v}"
        );
        input.begin_frame();
        touch(&mut input, 7, Ended, 150.0, 200.0);
        input.begin_frame();
        assert_eq!(widgets.stick(&mut Ui::new(), &input, pad), glam::Vec2::ZERO);
    }

    #[test]
    fn a_field_takes_the_keyboard_on_a_click_and_gives_it_back_on_enter() {
        use crate::input::{Key, MouseButton as M};
        let (mut widgets, mut input) = (Widgets::new(), Input::new());
        let mut name = String::new();
        let field = |widgets: &mut Widgets, input: &Input, name: &mut String| {
            widgets.text_field(&mut Ui::new(), input, PLAY, "your name", name)
        };
        // Typing before a click goes nowhere.
        frame(&mut input, (500.0, 500.0), &[InputEvent::Text("x".into())]);
        field(&mut widgets, &input, &mut name);
        assert!(name.is_empty() && !widgets.typing());

        frame(&mut input, (20.0, 20.0), &[InputEvent::MouseDown(M::Left)]);
        field(&mut widgets, &input, &mut name);
        assert!(widgets.typing());
        frame(
            &mut input,
            (20.0, 20.0),
            &[InputEvent::MouseUp(M::Left), InputEvent::Text("Adx".into())],
        );
        assert!(field(&mut widgets, &input, &mut name).changed);
        frame(
            &mut input,
            (20.0, 20.0),
            &[InputEvent::KeyDown(Key::Backspace)],
        );
        field(&mut widgets, &input, &mut name);
        assert_eq!(name, "Ad");
        frame(
            &mut input,
            (20.0, 20.0),
            &[
                InputEvent::KeyUp(Key::Backspace),
                InputEvent::KeyDown(Key::Enter),
            ],
        );
        let typed = field(&mut widgets, &input, &mut name);
        assert!(typed.submitted && !widgets.typing());
    }

    #[test]
    fn a_button_clicks_on_release_over_it_and_not_on_a_drag_from_elsewhere() {
        let (mut widgets, mut input, mut ui) = (Widgets::new(), Input::new(), Ui::new());
        let clicks = |widgets: &mut Widgets, input: &Input, ui: &mut Ui| {
            (
                widgets.button(ui, input, PLAY, "Play"),
                widgets.button(ui, input, QUIT, "Quit"),
            )
        };

        frame(
            &mut input,
            (20.0, 20.0),
            &[InputEvent::MouseDown(MouseButton::Left)],
        );
        assert_eq!(
            clicks(&mut widgets, &input, &mut ui),
            (false, false),
            "down is not a click"
        );
        frame(
            &mut input,
            (20.0, 20.0),
            &[InputEvent::MouseUp(MouseButton::Left)],
        );
        assert_eq!(
            clicks(&mut widgets, &input, &mut ui),
            (true, false),
            "up over it is"
        );

        // Down on Quit, dragged to Play, released there: neither.
        frame(
            &mut input,
            (20.0, 60.0),
            &[InputEvent::MouseDown(MouseButton::Left)],
        );
        clicks(&mut widgets, &input, &mut ui);
        frame(
            &mut input,
            (20.0, 20.0),
            &[InputEvent::MouseUp(MouseButton::Left)],
        );
        assert_eq!(clicks(&mut widgets, &input, &mut ui), (false, false));
        assert_eq!(ui.quads.len(), 8, "drawn every frame: two quads a frame");
    }

    #[test]
    fn a_toggle_flips_and_a_slider_follows_the_drag() {
        let (mut widgets, mut input, mut ui) = (Widgets::new(), Input::new(), Ui::new());
        let mut on = false;
        let mut volume = 0.5;
        let bar = Rect::new(0.0, 100.0, 200.0, 20.0);

        frame(
            &mut input,
            (20.0, 20.0),
            &[InputEvent::MouseDown(MouseButton::Left)],
        );
        widgets.toggle(&mut ui, &input, PLAY, "Shadows", &mut on);
        frame(
            &mut input,
            (20.0, 20.0),
            &[InputEvent::MouseUp(MouseButton::Left)],
        );
        assert!(widgets.toggle(&mut ui, &input, PLAY, "Shadows", &mut on));
        assert!(on);

        frame(
            &mut input,
            (50.0, 110.0),
            &[InputEvent::MouseDown(MouseButton::Left)],
        );
        assert!(widgets.slider(&mut ui, &input, bar, "Volume", &mut volume, 0.0..=1.0));
        assert!((volume - 0.25).abs() < 1e-3);
        // Dragged off the end of the bar, still held: clamped, still followed.
        frame(&mut input, (400.0, 300.0), &[]);
        widgets.slider(&mut ui, &input, bar, "Volume", &mut volume, 0.0..=1.0);
        assert_eq!(volume, 1.0);
    }
}
