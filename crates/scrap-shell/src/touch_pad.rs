//! A pad drawn on a touch screen, for a phone or a tablet: the native
//! twin of the browser's pads (`pad.js` in examples/kitchen-web, Not
//! Divinity's `touch.js`). Two ways to hold it ([`TouchMode`]):
//!
//! * [`TouchMode::Look`] — a first-person game's: the left thumb walks on a
//!   stick that comes to where it lands, the right one looks by dragging
//!   and presses buttons; in as a pad's and a mouse's would — `LeftX`/
//!   `LeftY`, `Pad(South)`, pointer motion — and only while the pointer is
//!   captured, which is how a game says it is being played rather than
//!   clicked through: in a menu the pad is not drawn and a finger is the
//!   mouse, as [`Input`](crate::input::Input) already makes the first one.
//! * [`TouchMode::Pad`] — a whole gamepad on the glass
//!   ([`TouchLayout::gamepad`]): two sticks that come to the thumbs, ✕ ○ □ △,
//!   L1 L2 R1 R2, Select and Start. It is a pad of its own, on a player
//!   slot like a pad plugged in (the shell's `PadConnected` and `…On`
//!   events), so a game that seats players by their pads seats a thumb
//!   like one and shows pad glyphs; no game system changes. Shown while
//!   touch is the device in use: a key or a real pad hides it (its slot is
//!   let go, as a pad unplugged), a touch brings it back.
//!
//! One piece for every platform that gives the shell touches (iOS,
//! Android); the layout is the game's ([`TouchLayout`]), on
//! [`WindowConfig::touch_pad`](crate::shell::WindowConfig::touch_pad).
//! Everything is placed in points from a corner of the screen's safe
//! area ([`TouchPad::set_safe_area`]): never under a notch or the home bar.

use glam::{Vec2, Vec4};

use crate::input::{InputEvent, PadAxis, PadButton, TouchPhase};
use crate::ui::{Quad, TextRun, Ui};

/// A button on the screen: which pad button it is, what it says, where.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchButton {
    pub button: PadButton,
    pub label: String,
    /// Its centre, in points from the safe area's corner `from`: x in, y
    /// in (both positive).
    pub at: Vec2,
    pub from: Corner,
    /// Its radius, in points (a pill's half height).
    pub radius: f32,
    /// A round button, or a pill this many points wide.
    pub shape: Shape,
    /// What is drawn on it: its label, or one of a pad's face symbols.
    pub glyph: Glyph,
    /// The glyph's colour.
    pub color: Vec4,
}

impl TouchButton {
    /// A round button with a label.
    pub fn round(button: PadButton, label: &str, at: Vec2, from: Corner, radius: f32) -> Self {
        Self { button, label: label.to_string(), at, from, radius, shape: Shape::Disc, glyph: Glyph::Label, color: WORDS }
    }

    /// A pill `width` × `height` points with a label.
    pub fn pill(button: PadButton, label: &str, at: Vec2, from: Corner, width: f32, height: f32) -> Self {
        Self { shape: Shape::Pill { width }, ..Self::round(button, label, at, from, height * 0.5) }
    }

    /// A face button: round, its symbol drawn in its colour.
    pub fn face(button: PadButton, glyph: Glyph, color: Vec4, at: Vec2, from: Corner, radius: f32) -> Self {
        Self { glyph, color, ..Self::round(button, "", at, from, radius) }
    }

    /// Its half size, in points.
    fn half(&self) -> Vec2 {
        match self.shape {
            Shape::Disc => Vec2::splat(self.radius),
            Shape::Pill { width } => Vec2::new(width * 0.5, self.radius),
        }
    }
}

/// A button's outline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Disc,
    Pill { width: f32 },
}

/// What a button shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Label,
    Cross,
    Circle,
    Square,
    Triangle,
}

/// Which corner of the screen a button is placed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// How the pad is held (see the module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchMode {
    /// A stick and buttons while the pointer is captured; the right thumb
    /// looks by dragging (pointer motion).
    Look,
    /// A gamepad of its own: two sticks, the buttons, a player slot;
    /// shown while touch is the device in use.
    Pad,
}

/// How a game's pad on the screen is laid out and how it feels.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchLayout {
    pub mode: TouchMode,
    pub buttons: Vec<TouchButton>,
    /// How far the stick's knob goes from where the thumb landed, in
    /// points (`pad.js`: 56).
    pub stick_radius: f32,
    /// Of the stick's travel, how much in the middle counts as nothing
    /// (`pad.js`: 0.12); past it the stick starts from zero.
    pub dead_zone: f32,
    /// The share of the screen's width, from the left, where a thumb
    /// landing brings the (left) stick.
    pub stick_zone: f32,
    /// [`TouchMode::Pad`]: the share of the screen's height, from the top,
    /// where a thumb brings no stick (the game's top bar is there).
    pub top_zone: f32,
    /// [`TouchMode::Pad`]: where each stick rests when no thumb holds it,
    /// drawn faintly — points from the bottom-left corner for the left,
    /// the bottom-right for the right.
    pub rest: [Vec2; 2],
    /// Pointer motion per point a finger drags to look ([`TouchMode::Look`]).
    pub look_speed: f32,
}

const WORDS: Vec4 = Vec4::new(1.0, 1.0, 1.0, 0.85);

/// A colour from `0xRRGGBB`.
fn rgb(hex: u32) -> Vec4 {
    Vec4::new(((hex >> 16) & 255) as f32 / 255.0, ((hex >> 8) & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0, 0.95)
}

impl TouchLayout {
    /// The engine's standard pad, named for a first-person game: jump
    /// (South), use (West), the two hands (the triggers), run (L3) and
    /// the menu (Start). A game renames or moves them.
    pub fn standard() -> Self {
        let b = |button, label: &str, x, y, radius| TouchButton::round(button, label, Vec2::new(x, y), Corner::BottomRight, radius);
        Self {
            mode: TouchMode::Look,
            buttons: vec![
                b(PadButton::South, "Jump", 70.0, 70.0, 38.0),
                b(PadButton::West, "Use", 160.0, 50.0, 30.0),
                b(PadButton::RightTrigger, "R", 60.0, 170.0, 32.0),
                b(PadButton::LeftTrigger, "L", 150.0, 140.0, 32.0),
                b(PadButton::LeftStick, "Run", 240.0, 45.0, 26.0),
                TouchButton::round(PadButton::Start, "II", Vec2::new(40.0, 36.0), Corner::TopRight, 22.0),
            ],
            stick_radius: 56.0,
            dead_zone: 0.12,
            stick_zone: 0.4,
            top_zone: 0.0,
            rest: [Vec2::ZERO; 2],
            look_speed: 1.5,
        }
    }

    /// A whole gamepad on the glass, laid out as a phone is held in
    /// landscape (Not Divinity's `touch.js`, measured in its unit of 50
    /// points): the face diamond bottom right, the shoulders by the
    /// screen's sides, Select top left and Start top right, the sticks
    /// coming to the thumbs (left in the left 42 % of the screen, right
    /// anywhere else below the top bar). A game renames Select and Start.
    pub fn gamepad() -> Self {
        let u = 50.0;
        let face = |button, glyph, color, x, y| TouchButton::face(button, glyph, rgb(color), Vec2::new(x, y), Corner::BottomRight, u * 0.51);
        let c = 0.3 * u + 1.45 * u;
        let d = 0.94 * u;
        let (sw, sh) = (1.15 * u, 0.72 * u);
        Self {
            mode: TouchMode::Pad,
            buttons: vec![
                face(PadButton::North, Glyph::Triangle, 0x7fd6b8, c, c + d),
                face(PadButton::South, Glyph::Cross, 0x8fb4ff, c, c - d),
                face(PadButton::West, Glyph::Square, 0xf0a6d6, c + d, c),
                face(PadButton::East, Glyph::Circle, 0xff8a80, c - d, c),
                TouchButton::pill(PadButton::LeftTrigger, "L2", Vec2::new(8.0 + sw * 0.5, 132.0), Corner::TopLeft, sw, sh),
                TouchButton::pill(PadButton::LeftBumper, "L1", Vec2::new(8.0 + sw * 0.5, 132.0 + sh + 9.0), Corner::TopLeft, sw, sh),
                TouchButton::pill(PadButton::RightTrigger, "R2", Vec2::new(8.0 + sw * 0.5, 116.0), Corner::TopRight, sw, sh),
                TouchButton::pill(PadButton::RightBumper, "R1", Vec2::new(8.0 + sw * 0.5, 116.0 + sh + 9.0), Corner::TopRight, sw, sh),
                TouchButton::pill(PadButton::Select, "Select", Vec2::new(8.0 + sw * 0.5, 82.0), Corner::TopLeft, sw, 0.56 * u),
                TouchButton::pill(PadButton::Start, "Start", Vec2::new(8.0 + sw * 0.5, 66.0), Corner::TopRight, sw, 0.56 * u),
            ],
            stick_radius: u,
            dead_zone: 0.14,
            stick_zone: 0.42,
            top_zone: 0.18,
            rest: [Vec2::new(1.6 * u, 1.6 * u), Vec2::new(5.2 * u, 1.3 * u)],
            look_speed: 1.5,
        }
    }

    /// The button for `button`, to rename or move.
    pub fn button_mut(&mut self, button: PadButton) -> Option<&mut TouchButton> {
        self.buttons.iter_mut().find(|b| b.button == button)
    }
}

/// What a finger is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Finger {
    /// Holding a stick (the right one in [`TouchMode::Pad`]), which came
    /// to `home`.
    Stick { right: bool, home: Vec2, at: Vec2 },
    /// Dragging to look; last seen at `at`.
    Look { at: Vec2 },
    /// Holding a button down (until it lifts, wherever it goes).
    Button(PadButton),
    /// On the pad but on nothing: kept so its lift says nothing.
    Idle,
}

/// The pad's state: which fingers do what.
#[derive(Debug, Clone)]
pub struct TouchPad {
    layout: TouchLayout,
    fingers: Vec<(u64, Finger)>,
    /// The last frame's screen, in pixels, and pixels per point.
    size: Vec2,
    scale: f32,
    /// The safe area's insets in pixels: left, top, right, bottom.
    safe: Vec4,
    /// [`TouchMode::Pad`]: drawn and listening (touch is the device in use).
    shown: bool,
}

impl TouchPad {
    pub fn new(layout: TouchLayout) -> Self {
        Self {
            layout,
            fingers: Vec::new(),
            size: Vec2::ZERO,
            scale: 1.0,
            safe: Vec4::ZERO,
            shown: true,
        }
    }

    pub fn layout(&self) -> &TouchLayout {
        &self.layout
    }

    pub fn mode(&self) -> TouchMode {
        self.layout.mode
    }

    /// The screen is `size` pixels, `scale` pixels to a point.
    pub fn resize(&mut self, size: Vec2, scale: f32) {
        self.size = size;
        self.scale = scale.max(0.1);
    }

    /// The screen's safe area, as insets in pixels from its edges (left,
    /// top, right, bottom): what a notch, rounded corners and the home bar
    /// leave. The pad is placed inside it.
    pub fn set_safe_area(&mut self, insets: Vec4) {
        self.safe = insets.max(Vec4::ZERO);
    }

    /// Whether the pad is shown ([`TouchMode::Pad`]; always in `Look`).
    pub fn shown(&self) -> bool {
        self.shown
    }

    /// Show the pad or hide it; hidden, whatever it held is let go
    /// (returned, to be heard before the slot goes).
    pub fn show(&mut self, on: bool) -> Vec<InputEvent> {
        self.shown = on;
        if on {
            Vec::new()
        } else {
            self.release_all()
        }
    }

    /// A corner of the safe area, in pixels.
    fn corner(&self, from: Corner) -> Vec2 {
        let s = self.safe;
        match from {
            Corner::TopLeft => Vec2::new(s.x, s.y),
            Corner::TopRight => Vec2::new(self.size.x - s.z, s.y),
            Corner::BottomLeft => Vec2::new(s.x, self.size.y - s.w),
            Corner::BottomRight => Vec2::new(self.size.x - s.z, self.size.y - s.w),
        }
    }

    /// A point `at` points in from the corner `from`, in pixels.
    fn point(&self, at: Vec2, from: Corner) -> Vec2 {
        let at = at * self.scale;
        let c = self.corner(from);
        match from {
            Corner::TopLeft => c + at,
            Corner::TopRight => Vec2::new(c.x - at.x, c.y + at.y),
            Corner::BottomLeft => Vec2::new(c.x + at.x, c.y - at.y),
            Corner::BottomRight => c - at,
        }
    }

    /// Where a button's centre is on the screen, and its half size, in
    /// pixels.
    fn place(&self, b: &TouchButton) -> (Vec2, Vec2) {
        (self.point(b.at, b.from), b.half() * self.scale)
    }

    fn button_at(&self, p: Vec2) -> Option<PadButton> {
        self.layout.buttons.iter().find_map(|b| {
            let (c, half) = self.place(b);
            // A little more than drawn: a thumb is not a cursor.
            let hit = match b.shape {
                Shape::Disc => c.distance(p) <= half.x * 1.25,
                Shape::Pill { .. } => {
                    let d = (p - c).abs();
                    d.x <= half.x + half.y * 0.25 && d.y <= half.y * 1.25
                }
            };
            hit.then_some(b.button)
        })
    }

    /// One touch, as the game should hear it. `playing` is whether the
    /// pointer is captured ([`TouchMode::Look`]): if not, the finger is
    /// passed through as a touch (the first one is the mouse) and whatever
    /// the pad held is let go. In [`TouchMode::Pad`] every finger is the
    /// pad's (`playing` is not asked), and the events are a pad's, for the
    /// shell to put on the pad's slot.
    pub fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32, playing: bool) -> Vec<InputEvent> {
        let p = Vec2::new(x, y);
        let pad = self.layout.mode == TouchMode::Pad;
        let raw = InputEvent::Touch { id, phase, x, y };
        let mine = self.fingers.iter().position(|(f, _)| *f == id);
        if !pad && !playing && mine.is_none() {
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
                let holds_stick = |me: &Self, right: bool| me.fingers.iter().any(|(_, f)| matches!(f, Finger::Stick { right: r, .. } if *r == right));
                if let Some(button) = self.button_at(p) {
                    self.fingers.push((id, Finger::Button(button)));
                    out.push(InputEvent::PadDown(button));
                    out.extend(trigger(button, 1.0));
                } else if pad && p.y < self.size.y * self.layout.top_zone {
                    self.fingers.push((id, Finger::Idle));
                } else if p.x < self.size.x * self.layout.stick_zone && !holds_stick(self, false) {
                    self.fingers.push((id, Finger::Stick { right: false, home: p, at: p }));
                    out.extend(self.stick(false, p, p));
                } else if pad && !holds_stick(self, true) && p.x >= self.size.x * self.layout.stick_zone {
                    self.fingers.push((id, Finger::Stick { right: true, home: p, at: p }));
                    out.extend(self.stick(true, p, p));
                } else if !pad && !self.fingers.iter().any(|(_, f)| matches!(f, Finger::Look { .. })) {
                    self.fingers.push((id, Finger::Look { at: p }));
                } else if pad {
                    self.fingers.push((id, Finger::Idle));
                }
            }
            TouchPhase::Moved => {
                if let Some(i) = mine {
                    match self.fingers[i].1 {
                        Finger::Stick { right, home, .. } => {
                            self.fingers[i].1 = Finger::Stick { right, home, at: p };
                            out.extend(self.stick(right, home, p));
                        }
                        Finger::Look { at } => {
                            let d = (p - at) / self.scale * self.layout.look_speed;
                            self.fingers[i].1 = Finger::Look { at: p };
                            out.push(InputEvent::MouseMotion { dx: d.x, dy: d.y });
                        }
                        Finger::Button(_) | Finger::Idle => {}
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

    /// A stick's axes with the thumb at `at` from `home` (`pad.js`'s
    /// `place`): clamped to the radius, up positive, the dead zone taken
    /// out and the rest rescaled from zero.
    fn stick(&self, right: bool, home: Vec2, at: Vec2) -> [InputEvent; 2] {
        let radius = self.layout.stick_radius * self.scale;
        let d = (at - home).clamp_length_max(radius);
        let mut v = Vec2::new(d.x, -d.y) / radius;
        let m = v.length();
        let dead = self.layout.dead_zone;
        v = if m < dead { Vec2::ZERO } else { v * ((m - dead) / (1.0 - dead) / m) };
        let (x, y) = axes(right);
        [InputEvent::PadMoved { axis: x, value: v.x }, InputEvent::PadMoved { axis: y, value: v.y }]
    }

    fn lift(&mut self, i: usize) -> Vec<InputEvent> {
        match self.fingers.remove(i).1 {
            Finger::Stick { right, .. } => {
                let (x, y) = axes(right);
                vec![InputEvent::PadMoved { axis: x, value: 0.0 }, InputEvent::PadMoved { axis: y, value: 0.0 }]
            }
            Finger::Look { .. } | Finger::Idle => Vec::new(),
            Finger::Button(b) => {
                let mut out = vec![InputEvent::PadUp(b)];
                out.extend(trigger(b, 0.0));
                out
            }
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

    /// Draw the pad over the game's overlay: the sticks where thumbs hold
    /// them (in [`TouchMode::Pad`] faintly where they rest), the buttons, a
    /// held one brighter.
    pub fn draw(&self, ui: &mut Ui) {
        if !self.shown {
            return;
        }
        let pad = self.layout.mode == TouchMode::Pad;
        let (dim, lit, rim) = if pad {
            // The page's look (touch.js): a dark glass, a pale rim drawn
            // under it (what shows round its edge), a pressed one pale.
            (Vec4::new(0.05, 0.04, 0.06, 0.5), Vec4::new(0.94, 0.9, 0.82, 0.38), Vec4::new(0.94, 0.9, 0.82, 0.14))
        } else {
            (Vec4::new(1.0, 1.0, 1.0, 0.16), Vec4::new(1.0, 1.0, 1.0, 0.38), Vec4::ZERO)
        };
        let s = self.scale;
        let shape = |ui: &mut Ui, c: Vec2, half: Vec2, color: Vec4| {
            ui.quad(Quad::new(c.x - half.x, c.y - half.y, 2.0 * half.x, 2.0 * half.y, color).rounded(half.y.min(half.x)));
        };
        let ringed = |ui: &mut Ui, c: Vec2, half: Vec2, fill: Vec4| {
            if rim.w > 0.0 {
                shape(ui, c, half, rim);
                shape(ui, c, half - Vec2::splat(2.0 * s), fill);
            } else {
                shape(ui, c, half, fill);
            }
        };
        for b in &self.layout.buttons {
            let (c, half) = self.place(b);
            let held = self.fingers.iter().any(|(_, f)| *f == Finger::Button(b.button));
            ringed(ui, c, half, if held { lit } else { dim });
            match b.glyph {
                Glyph::Label => {
                    let size = (half.y * 0.75).min(16.0 * s);
                    ui.text(TextRun::new(c.x - half.x, c.y - size * 0.62, size, b.color, b.label.clone()).within(2.0 * half.x, 0.5));
                }
                glyph => symbol(ui, glyph, c, half.y * 0.48, (1.6 * s).max(1.5), if held { Vec4::new(0.07, 0.06, 0.08, 0.95) } else { b.color }),
            }
        }
        let radius = self.layout.stick_radius * s;
        for right in [false, true] {
            if !pad && right {
                continue;
            }
            let held = self.fingers.iter().find_map(|(_, f)| match *f {
                Finger::Stick { right: r, home, at } if r == right => Some((home, at)),
                _ => None,
            });
            let (home, at, alpha) = match held {
                Some((home, at)) => (home, home + (at - home).clamp_length_max(radius), 1.0),
                None if pad => {
                    let rest = self.point(self.layout.rest[right as usize], if right { Corner::BottomRight } else { Corner::BottomLeft });
                    (rest, rest, if right { 0.3 } else { 0.6 })
                }
                None => continue,
            };
            let fade = |c: Vec4| Vec4::new(c.x, c.y, c.z, c.w * alpha);
            if pad {
                // A ring of dots, the knob: the game shows through.
                let n = 48;
                for i in 0..n {
                    let a = i as f32 / n as f32 * std::f32::consts::TAU;
                    let p = home + Vec2::new(a.cos(), a.sin()) * radius;
                    shape(ui, p, Vec2::splat(1.2 * s), fade(rim));
                }
                shape(ui, at, Vec2::splat(radius * 0.45), fade(Vec4::new(0.94, 0.9, 0.82, 0.3)));
            } else {
                ringed(ui, home, Vec2::splat(radius), dim);
                shape(ui, at, Vec2::splat(radius * 0.45), lit);
            }
        }
    }
}

/// The axes of the left or the right stick.
fn axes(right: bool) -> (PadAxis, PadAxis) {
    if right {
        (PadAxis::RightX, PadAxis::RightY)
    } else {
        (PadAxis::LeftX, PadAxis::LeftY)
    }
}

/// A trigger is a button and an axis on a pad: both, pressed all the way.
fn trigger(button: PadButton, value: f32) -> Option<InputEvent> {
    let axis = match button {
        PadButton::LeftTrigger => PadAxis::LeftTrigger,
        PadButton::RightTrigger => PadAxis::RightTrigger,
        _ => return None,
    };
    Some(InputEvent::PadMoved { axis, value })
}

/// A face button's symbol, `r` pixels from its middle, drawn in strokes
/// of `w` pixels (dots along its lines: the overlay draws boxes, and a
/// font may lack ✕ ○ □ △).
fn symbol(ui: &mut Ui, glyph: Glyph, c: Vec2, r: f32, w: f32, color: Vec4) {
    let dot = |ui: &mut Ui, p: Vec2| {
        ui.quad(Quad::new(p.x - w * 0.5, p.y - w * 0.5, w, w, color).rounded(w * 0.5));
    };
    let line = |ui: &mut Ui, a: Vec2, b: Vec2| {
        let n = ((b - a).length() / (w * 0.5)).ceil().max(1.0) as usize;
        for i in 0..=n {
            dot(ui, a.lerp(b, i as f32 / n as f32));
        }
    };
    match glyph {
        Glyph::Label => {}
        Glyph::Cross => {
            let k = r * 0.8;
            line(ui, c + Vec2::new(-k, -k), c + Vec2::new(k, k));
            line(ui, c + Vec2::new(-k, k), c + Vec2::new(k, -k));
        }
        Glyph::Circle => {
            let n = ((std::f32::consts::TAU * r) / (w * 0.5)).ceil() as usize;
            for i in 0..n {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                dot(ui, c + Vec2::new(a.cos(), a.sin()) * r * 0.85);
            }
        }
        Glyph::Square => {
            let k = r * 0.75;
            let corners = [Vec2::new(-k, -k), Vec2::new(k, -k), Vec2::new(k, k), Vec2::new(-k, k)];
            for i in 0..4 {
                line(ui, c + corners[i], c + corners[(i + 1) % 4]);
            }
        }
        Glyph::Triangle => {
            let top = c + Vec2::new(0.0, -r);
            let left = c + Vec2::new(-r * 0.92, r * 0.6);
            let right = c + Vec2::new(r * 0.92, r * 0.6);
            line(ui, top, left);
            line(ui, left, right);
            line(ui, right, top);
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

    /// An iPhone 15 in landscape: 2556 × 1179 pixels at 3 to a point, the
    /// notch's 59 points on the left and the home bar's 21 below.
    fn gamepad() -> TouchPad {
        let mut pad = TouchPad::new(TouchLayout::gamepad());
        pad.resize(Vec2::new(2556.0, 1179.0), 3.0);
        pad.set_safe_area(Vec4::new(177.0, 0.0, 177.0, 63.0));
        pad
    }

    #[test]
    fn the_gamepad_has_two_sticks_that_come_to_the_thumbs_and_every_button() {
        let mut pad = gamepad();
        // Both thumbs at once, each on its own stick, menus or not.
        let left = pad.touch(1, TouchPhase::Started, 600.0, 900.0, false);
        let right = pad.touch(2, TouchPhase::Started, 1500.0, 800.0, false);
        assert_eq!(axis(&left, PadAxis::LeftX), Some(0.0));
        assert_eq!(axis(&right, PadAxis::RightX), Some(0.0));
        let walk = pad.touch(1, TouchPhase::Moved, 600.0 + 400.0, 900.0, false);
        assert!((axis(&walk, PadAxis::LeftX).unwrap() - 1.0).abs() < 1e-5);
        let look = pad.touch(2, TouchPhase::Moved, 1500.0, 800.0 + 400.0, false);
        assert!((axis(&look, PadAxis::RightY).unwrap() + 1.0).abs() < 1e-5);
        assert_eq!(axis(&pad.touch(2, TouchPhase::Ended, 0.0, 0.0, false), PadAxis::RightY), Some(0.0));
        // Every button where it is drawn, the triggers as axes too.
        for b in pad.layout.buttons.clone() {
            let (c, _) = pad.place(&b);
            let down = pad.touch(9, TouchPhase::Started, c.x, c.y, false);
            assert_eq!(down.first(), Some(&InputEvent::PadDown(b.button)), "{:?} at {c}", b.button);
            let up = pad.touch(9, TouchPhase::Ended, c.x, c.y, false);
            assert_eq!(up.first(), Some(&InputEvent::PadUp(b.button)));
            if b.button == PadButton::RightTrigger {
                assert_eq!(axis(&down, PadAxis::RightTrigger), Some(1.0));
                assert_eq!(axis(&up, PadAxis::RightTrigger), Some(0.0));
            }
        }
        // The top bar brings no stick.
        assert!(pad.touch(4, TouchPhase::Started, 1300.0, 100.0, false).is_empty());
    }

    #[test]
    fn the_gamepad_stays_in_the_safe_area_and_hiding_it_lets_go() {
        let mut pad = gamepad();
        for b in pad.layout.buttons.clone() {
            let (c, half) = pad.place(&b);
            assert!(c.x - half.x >= 177.0 && c.x + half.x <= 2556.0 - 177.0, "{:?} off the safe area: {c}", b.button);
            assert!(c.y + half.y <= 1179.0 - 63.0 && c.y - half.y >= 0.0, "{:?} off the safe area: {c}", b.button);
        }
        pad.touch(1, TouchPhase::Started, 600.0, 900.0, false);
        pad.touch(1, TouchPhase::Moved, 600.0, 600.0, false);
        let gone = pad.show(false);
        assert_eq!(axis(&gone, PadAxis::LeftY), Some(0.0));
        assert!(!pad.shown());
        let mut ui = Ui::new();
        pad.draw(&mut ui);
        assert!(ui.quads.is_empty());
        pad.show(true);
        pad.draw(&mut ui);
        assert!(ui.quads.len() > 10);
    }
}
