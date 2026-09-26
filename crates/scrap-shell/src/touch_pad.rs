//! A pad drawn on a touch screen, for a phone or a tablet: the native
//! twin of the browser's `pad.js` (examples/kitchen-web/web). The left
//! thumb walks on a stick that comes to where it lands; the right one
//! looks by dragging and presses buttons. Both come into the game as a
//! pad's and a mouse's would — `LeftX`/`LeftY`, `Pad(South)`, pointer
//! motion — so a game's `input.ron` reads fingers with nothing written
//! for them.
//!
//! Only while the pointer is captured, which is how a game says it is
//! being played rather than clicked through: in a menu the pad is not
//! drawn and a finger is the mouse, as [`Input`](crate::input::Input)
//! already makes the first one.
//!
//! One piece for every platform that gives the shell touches (iOS,
//! Android); the layout is the game's ([`TouchLayout`]), on
//! [`WindowConfig::touch_pad`](crate::shell::WindowConfig::touch_pad).

use glam::{Vec2, Vec4};

use crate::input::{InputEvent, PadAxis, PadButton, TouchPhase};
use crate::ui::{Quad, TextRun, Ui};

/// A button on the screen: which pad button it is, what it says, where.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchButton {
    pub button: PadButton,
    pub label: String,
    /// Its centre, in points from the screen's corner `from`: x in, y in
    /// (both positive).
    pub at: Vec2,
    pub from: Corner,
    /// Its radius, in points.
    pub radius: f32,
}

/// Which corner of the screen a button is placed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// How a game's pad on the screen is laid out and how it feels.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchLayout {
    pub buttons: Vec<TouchButton>,
    /// How far the stick's knob goes from where the thumb landed, in
    /// points (`pad.js`: 56).
    pub stick_radius: f32,
    /// Of the stick's travel, how much in the middle counts as nothing
    /// (`pad.js`: 0.12); past it the stick starts from zero.
    pub dead_zone: f32,
    /// The share of the screen's width, from the left, where a thumb
    /// landing brings the stick.
    pub stick_zone: f32,
    /// Pointer motion per point a finger drags to look.
    pub look_speed: f32,
}

impl TouchLayout {
    /// The engine's standard pad, named for a first-person game: jump
    /// (South), use (West), the two hands (the triggers), run (L3) and
    /// the menu (Start). A game renames or moves them.
    pub fn standard() -> Self {
        let b = |button, label: &str, x, y, radius| TouchButton {
            button,
            label: label.to_string(),
            at: Vec2::new(x, y),
            from: Corner::BottomRight,
            radius,
        };
        Self {
            buttons: vec![
                b(PadButton::South, "Jump", 70.0, 70.0, 38.0),
                b(PadButton::West, "Use", 160.0, 50.0, 30.0),
                b(PadButton::RightTrigger, "R", 60.0, 170.0, 32.0),
                b(PadButton::LeftTrigger, "L", 150.0, 140.0, 32.0),
                b(PadButton::LeftStick, "Run", 240.0, 45.0, 26.0),
                TouchButton {
                    button: PadButton::Start,
                    label: "II".to_string(),
                    at: Vec2::new(40.0, 36.0),
                    from: Corner::TopRight,
                    radius: 22.0,
                },
            ],
            stick_radius: 56.0,
            dead_zone: 0.12,
            stick_zone: 0.4,
            look_speed: 1.5,
        }
    }
}

/// What a finger is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Finger {
    /// Holding the stick, which came to `home`.
    Stick { home: Vec2, at: Vec2 },
    /// Dragging to look; last seen at `at`.
    Look { at: Vec2 },
    /// Holding a button down (until it lifts, wherever it goes).
    Button(PadButton),
}

/// The pad's state: which fingers do what.
#[derive(Debug, Clone)]
pub struct TouchPad {
    layout: TouchLayout,
    fingers: Vec<(u64, Finger)>,
    /// The last frame's screen, in pixels, and pixels per point.
    size: Vec2,
    scale: f32,
}

impl TouchPad {
    pub fn new(layout: TouchLayout) -> Self {
        Self {
            layout,
            fingers: Vec::new(),
            size: Vec2::ZERO,
            scale: 1.0,
        }
    }

    /// The screen is `size` pixels, `scale` pixels to a point.
    pub fn resize(&mut self, size: Vec2, scale: f32) {
        self.size = size;
        self.scale = scale.max(0.1);
    }

    /// Where a button's centre is on the screen, and its radius, in pixels.
    fn place(&self, b: &TouchButton) -> (Vec2, f32) {
        let at = b.at * self.scale;
        let centre = match b.from {
            Corner::TopLeft => at,
            Corner::TopRight => Vec2::new(self.size.x - at.x, at.y),
            Corner::BottomLeft => Vec2::new(at.x, self.size.y - at.y),
            Corner::BottomRight => self.size - at,
        };
        (centre, b.radius * self.scale)
    }

    fn button_at(&self, p: Vec2) -> Option<PadButton> {
        self.layout.buttons.iter().find_map(|b| {
            let (c, r) = self.place(b);
            // A little more than drawn: a thumb is not a cursor.
            (c.distance(p) <= r * 1.25).then_some(b.button)
        })
    }

    /// One touch, as the game should hear it. `playing` is whether the
    /// pointer is captured: if not, the finger is passed through as a
    /// touch (the first one is the mouse) and whatever the pad held is let
    /// go.
    pub fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32, playing: bool) -> Vec<InputEvent> {
        let p = Vec2::new(x, y);
        let raw = InputEvent::Touch { id, phase, x, y };
        let mine = self.fingers.iter().position(|(f, _)| *f == id);
        if !playing && mine.is_none() {
            let mut out = self.release_all();
            out.push(raw);
            return out;
        }
        let mut out = Vec::new();
        match phase {
            TouchPhase::Started => {
                if let Some(i) = mine {
                    // A finger id reused without its end: forget the old one.
                    out.extend(self.lift(i));
                }
                if let Some(button) = self.button_at(p) {
                    self.fingers.push((id, Finger::Button(button)));
                    out.push(InputEvent::PadDown(button));
                } else if p.x < self.size.x * self.layout.stick_zone
                    && !self.fingers.iter().any(|(_, f)| matches!(f, Finger::Stick { .. }))
                {
                    self.fingers.push((id, Finger::Stick { home: p, at: p }));
                    out.extend(self.stick(p, p));
                } else if !self.fingers.iter().any(|(_, f)| matches!(f, Finger::Look { .. })) {
                    self.fingers.push((id, Finger::Look { at: p }));
                }
            }
            TouchPhase::Moved => {
                if let Some(i) = mine {
                    match self.fingers[i].1 {
                        Finger::Stick { home, .. } => {
                            self.fingers[i].1 = Finger::Stick { home, at: p };
                            out.extend(self.stick(home, p));
                        }
                        Finger::Look { at } => {
                            let d = (p - at) / self.scale * self.layout.look_speed;
                            self.fingers[i].1 = Finger::Look { at: p };
                            out.push(InputEvent::MouseMotion { dx: d.x, dy: d.y });
                        }
                        Finger::Button(_) => {}
                    }
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if let Some(i) = mine {
                    out.extend(self.lift(i));
                }
            }
        }
        out
    }

    /// The stick's axes with the thumb at `at` from `home` (`pad.js`'s
    /// `place`): clamped to the radius, up positive, the dead zone taken
    /// out and the rest rescaled from zero.
    fn stick(&self, home: Vec2, at: Vec2) -> [InputEvent; 2] {
        let radius = self.layout.stick_radius * self.scale;
        let d = (at - home).clamp_length_max(radius);
        let mut v = Vec2::new(d.x, -d.y) / radius;
        let m = v.length();
        let dead = self.layout.dead_zone;
        v = if m < dead { Vec2::ZERO } else { v * ((m - dead) / (1.0 - dead) / m) };
        [
            InputEvent::PadMoved { axis: PadAxis::LeftX, value: v.x },
            InputEvent::PadMoved { axis: PadAxis::LeftY, value: v.y },
        ]
    }

    fn lift(&mut self, i: usize) -> Vec<InputEvent> {
        match self.fingers.remove(i).1 {
            Finger::Stick { .. } => vec![
                InputEvent::PadMoved { axis: PadAxis::LeftX, value: 0.0 },
                InputEvent::PadMoved { axis: PadAxis::LeftY, value: 0.0 },
            ],
            Finger::Look { .. } => Vec::new(),
            Finger::Button(b) => vec![InputEvent::PadUp(b)],
        }
    }

    /// Let go of everything: nothing held over a change of screen (a stick
    /// let go of in the menu does not keep the player walking).
    pub fn release_all(&mut self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        while !self.fingers.is_empty() {
            out.extend(self.lift(0));
        }
        out
    }

    /// Draw the pad over the game's overlay: the stick where a thumb holds
    /// it, the buttons, a held one brighter.
    pub fn draw(&self, ui: &mut Ui) {
        let dim = Vec4::new(1.0, 1.0, 1.0, 0.16);
        let lit = Vec4::new(1.0, 1.0, 1.0, 0.38);
        let words = Vec4::new(1.0, 1.0, 1.0, 0.85);
        let disc = |ui: &mut Ui, c: Vec2, r: f32, color: Vec4| {
            ui.quad(Quad::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r, color).rounded(r));
        };
        for b in &self.layout.buttons {
            let (c, r) = self.place(b);
            let held = self.fingers.iter().any(|(_, f)| *f == Finger::Button(b.button));
            disc(ui, c, r, if held { lit } else { dim });
            let size = (r * 0.55).min(16.0 * self.scale);
            ui.text(TextRun::new(c.x - r, c.y - size * 0.6, size, words, b.label.clone()).within(2.0 * r, 0.5));
        }
        for (_, f) in &self.fingers {
            if let Finger::Stick { home, at } = *f {
                let radius = self.layout.stick_radius * self.scale;
                disc(ui, home, radius, dim);
                let knob = home + (at - home).clamp_length_max(radius);
                disc(ui, knob, radius * 0.45, lit);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad() -> TouchPad {
        let mut pad = TouchPad::new(TouchLayout::standard());
        pad.resize(Vec2::new(2400.0, 1100.0), 3.0);
        pad
    }

    fn axis(events: &[InputEvent], which: PadAxis) -> Option<f32> {
        events.iter().rev().find_map(|e| match e {
            InputEvent::PadMoved { axis, value } if *axis == which => Some(*value),
            _ => None,
        })
    }

    #[test]
    fn the_left_thumb_walks_on_a_stick_that_comes_to_it() {
        let mut pad = pad();
        let down = pad.touch(1, TouchPhase::Started, 300.0, 700.0, true);
        assert_eq!(axis(&down, PadAxis::LeftY), Some(0.0));
        // Up by the whole radius (56 points at 3 pixels each) and more.
        let up = pad.touch(1, TouchPhase::Moved, 300.0, 700.0 - 400.0, true);
        assert!((axis(&up, PadAxis::LeftY).unwrap() - 1.0).abs() < 1e-5);
        assert_eq!(axis(&up, PadAxis::LeftX), Some(0.0));
        // Inside the dead zone it is nothing.
        let still = pad.touch(1, TouchPhase::Moved, 310.0, 700.0, true);
        assert_eq!(axis(&still, PadAxis::LeftX), Some(0.0));
        let off = pad.touch(1, TouchPhase::Ended, 310.0, 700.0, true);
        assert_eq!(axis(&off, PadAxis::LeftX), Some(0.0));
    }

    #[test]
    fn a_button_is_held_while_its_finger_is_down_and_the_right_thumb_looks() {
        let mut pad = pad();
        let (c, _) = pad.place(&pad.layout.buttons[0].clone());
        assert_eq!(pad.touch(2, TouchPhase::Started, c.x, c.y, true), vec![InputEvent::PadDown(PadButton::South)]);
        assert!(pad.touch(2, TouchPhase::Moved, c.x - 300.0, c.y, true).is_empty());
        assert_eq!(pad.touch(2, TouchPhase::Ended, c.x - 300.0, c.y, true), vec![InputEvent::PadUp(PadButton::South)]);
        assert!(pad.touch(3, TouchPhase::Started, 1500.0, 400.0, true).is_empty());
        assert_eq!(
            pad.touch(3, TouchPhase::Moved, 1530.0, 394.0, true),
            vec![InputEvent::MouseMotion { dx: 15.0, dy: -3.0 }]
        );
    }

    #[test]
    fn in_a_menu_a_finger_is_a_touch_and_what_was_held_is_let_go() {
        let mut pad = pad();
        pad.touch(1, TouchPhase::Started, 300.0, 700.0, true);
        pad.touch(1, TouchPhase::Moved, 300.0, 500.0, true);
        let events = pad.touch(5, TouchPhase::Started, 1200.0, 500.0, false);
        assert_eq!(axis(&events, PadAxis::LeftY), Some(0.0));
        assert_eq!(events.last(), Some(&InputEvent::Touch { id: 5, phase: TouchPhase::Started, x: 1200.0, y: 500.0 }));
    }
}
