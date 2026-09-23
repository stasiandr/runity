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
    pub style: Style,
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
        // An estimate of the width, which is enough to centre a label.
        let size = self.style.text_size;
        let width = text.chars().count() as f32 * size * 0.52;
        ui.text(TextRun::new(
            rect.x + (rect.width - width).max(0.0) * 0.5,
            rect.y + (rect.height - size) * 0.5,
            size,
            self.style.text,
            text,
        ));
    }

    /// A button. `true` on the frame it is clicked — released over it,
    /// having been pressed on it.
    pub fn button(&mut self, ui: &mut Ui, input: &Input, rect: Rect, label: &str) -> bool {
        let (over, held, clicked) = self.track(input, rect, key(rect, label));
        let color = match (held, over) {
            (true, _) => self.style.pressed,
            (false, true) => self.style.hover,
            _ => self.style.idle,
        };
        ui.quad(Quad::new(rect.x, rect.y, rect.width, rect.height, color));
        self.label(ui, rect, label);
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
        let (over, _, clicked) = self.track(input, rect, key(rect, label));
        if clicked {
            *value = !*value;
        }
        let back = if over {
            self.style.hover
        } else {
            self.style.idle
        };
        ui.quad(Quad::new(rect.x, rect.y, rect.width, rect.height, back));
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
        let (over, _, _) = self.track(input, rect, id);
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
        ui.quad(Quad::new(rect.x, rect.y, rect.width, rect.height, back));
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
        typed
    }

    /// Whether a text field has the keyboard: while it does, the game
    /// should not read letters as its own keys.
    pub fn typing(&self) -> bool {
        self.focused.is_some()
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
        let (_, held, _) = self.track(input, rect, key(rect, label));
        let before = *value;
        if held && rect.width > 0.0 {
            let t = ((input.mouse_position().x - rect.x) / rect.width).clamp(0.0, 1.0);
            *value = min + (max - min) * t;
        }
        let t = if max > min {
            ((*value - min) / (max - min)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ui.quad(Quad::new(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            self.style.idle,
        ));
        ui.quad(Quad::new(
            rect.x,
            rect.y,
            rect.width * t,
            rect.height,
            self.style.accent,
        ));
        self.label(ui, rect, &format!("{label}: {:.2}", *value));
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
