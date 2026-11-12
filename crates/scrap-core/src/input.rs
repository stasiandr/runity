//! Keyboard and mouse state, rebuilt from a stream of events.
//!
//! The keys are the engine's own enum rather than winit's, for the same
//! reason the renderer hides wgpu: a shell is one of several — winit today,
//! the editor's window, a `UIViewController` on iOS later — and a game
//! written against one platform's key codes is a game that has to be
//! rewritten for the next.
//!
//! State is edge-aware. "Is W held" and "was W pressed this frame" are
//! different questions and both get asked constantly; a game that only has
//! the first one ends up writing the second badly, usually by remembering
//! last frame's booleans in the wrong place.

use std::collections::HashSet;

use glam::Vec2;

/// A physical key, named by what is printed on a US layout.
///
/// Physical rather than logical on purpose: `W` is the key above `S`
/// whatever the layout says, which is what movement wants. Text entry wants
/// the opposite and gets [`InputEvent::Text`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Escape,
    Space,
    Enter,
    Tab,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Left,
    Right,
    Up,
    Down,
    LeftShift,
    RightShift,
    LeftControl,
    RightControl,
    LeftAlt,
    RightAlt,
    LeftSuper,
    RightSuper,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    /// The punctuation keys, by what a US layout prints on them: what an
    /// editor's shortcuts use (⌘, opens the preferences).
    Comma,
    Period,
    Slash,
    Semicolon,
    Quote,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Backquote,
    /// Anything the shell knows about and this enum does not. Carrying the
    /// platform's code keeps a rebindable game from losing keys it has no
    /// name for.
    Other(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

/// A gamepad button, named by where it is rather than what is printed on
/// it: `South` is A on an Xbox pad, cross on a PlayStation one, B on a
/// Switch — the button under the right thumb, which is what "jump" wants on
/// all of them. The Steam Deck's too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PadButton {
    South,
    East,
    West,
    North,
    LeftBumper,
    RightBumper,
    /// The triggers as buttons: pulled past half way.
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    /// The pad's own button (PS, Xbox, Home) — where the platform lets a
    /// game have it.
    Home,
    /// A DualSense's or DualShock's touchpad pressed down.
    Touchpad,
    /// A DualSense's microphone button — where the platform hands it to
    /// games ([`crate::pad::PadCaps::mute_button`]).
    Mute,
    /// A Share button apart from Select (Xbox Series' middle one). A
    /// DualSense's Create is [`PadButton::Select`].
    Share,
    /// Back paddles (Xbox Elite, DualSense Edge), in the platform's order.
    Paddle1,
    Paddle2,
    Paddle3,
    Paddle4,
    /// A finger on the touchpad (first, second): where, in
    /// `PadAxis::Finger1X` and its kin while it is down.
    Finger1,
    Finger2,
}

/// A gamepad's analog inputs. Sticks run −1..1 with up and right positive;
/// triggers 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PadAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
    /// A touchpad finger's place, −1..1, right and up positive, while
    /// `PadButton::Finger1` (or `Finger2`) is held. No dead zone.
    Finger1X,
    Finger1Y,
    Finger2X,
    Finger2Y,
}

impl PadAxis {
    /// The axes that run 0..1 and have no dead zone: triggers.
    pub fn is_trigger(self) -> bool {
        matches!(self, PadAxis::LeftTrigger | PadAxis::RightTrigger)
    }
    /// A touchpad finger's coordinate: no dead zone.
    pub fn is_finger(self) -> bool {
        matches!(self, PadAxis::Finger1X | PadAxis::Finger1Y | PadAxis::Finger2X | PadAxis::Finger2Y)
    }
}

pub use crate::pad::{
    Battery, BatteryState, Haptic, HapticPlace, PadCaps, PadFeedback, PadGlyphs, PadInfo, PadKind, PadMotion, PadPulse, Rumble,
    TriggerEffect, TriggerSide, TriggerThreshold,
};

/// How far a stick has to move before it counts. Every stick rests a little
/// off centre, and a character that creeps while nobody touches the pad is
/// the bug this prevents.
pub const DEAD_ZONE: f32 = 0.15;

/// What a shell reports.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum InputEvent {
    KeyDown(Key),
    KeyUp(Key),
    /// Characters produced by the keyboard, after the layout and any dead
    /// keys have had their say. Not derivable from [`Key`].
    Text(String),
    MouseMoved {
        x: f32,
        y: f32,
    },
    /// Movement with no position — what a captured pointer reports while
    /// looking around, where the cursor is pinned and only the delta exists.
    MouseMotion {
        dx: f32,
        dy: f32,
    },
    MouseDown(MouseButton),
    MouseUp(MouseButton),
    Scroll {
        x: f32,
        y: f32,
    },
    /// A gamepad button, from any pad: the game sees one pad, which is what
    /// a player with one in their hands means.
    PadDown(PadButton),
    PadUp(PadButton),
    /// Where an analog input is now.
    PadMoved {
        axis: PadAxis,
        value: f32,
    },
    /// A button of one pad, by its player slot ([`PadSlots`]): the pad's
    /// own state ([`Input::pad`]) and the merged one ([`Input::pad_held`])
    /// both see it. What the window sends for a real pad, and what a
    /// script writes to be the second player: `PadDownOn(pad: 1, button:
    /// South)`.
    PadDownOn {
        pad: usize,
        button: PadButton,
    },
    PadUpOn {
        pad: usize,
        button: PadButton,
    },
    PadMovedOn {
        pad: usize,
        axis: PadAxis,
        value: f32,
    },
    /// A pad came into a slot, or went out of it: everything it held is let
    /// go, and the slot waits for it ([`PadSlots`]).
    PadConnected(usize),
    PadDisconnected(usize),
    /// What the pad in a slot is: kind, name, backend, abilities, battery
    /// ([`PadState::info`]). Sent when it comes and when its battery
    /// changes; a script can send it to be a DualSense.
    PadInfo {
        pad: usize,
        info: PadInfo,
    },
    /// A slot's pad's motion sensors, while switched on
    /// ([`PadFeedback::set_motion`]).
    PadMotion {
        pad: usize,
        motion: PadMotion,
    },
    /// A finger on a touch screen, by the id the platform gives it for as
    /// long as it is down.
    Touch {
        id: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
    },
    /// The window lost focus. Everything held is released, because the
    /// release event will be delivered to whatever has focus now, and a key
    /// that never comes up is held forever.
    FocusLost,
}

/// Where a touch is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

/// A finger down, as the game sees it this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Touch {
    pub id: u64,
    pub position: Vec2,
    /// Where it came down.
    pub start: Vec2,
    /// Came down this frame.
    pub began: bool,
    /// Lifted this frame (still listed for this one frame).
    pub ended: bool,
}

/// One pad's buttons and sticks: held, pressed and released this frame,
/// where each stick is. [`Input`] keeps one per player slot
/// ([`Input::pad`]) and one for every pad at once (`Input::pad_held` and
/// the rest).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PadState {
    connected: bool,
    held: HashSet<PadButton>,
    pressed: HashSet<PadButton>,
    released: HashSet<PadButton>,
    axes: std::collections::HashMap<PadAxis, f32>,
    /// The axes as the frame before ended: a threshold crossed this frame.
    axes_before: std::collections::HashMap<PadAxis, f32>,
    info: Option<PadInfo>,
    motion: Option<PadMotion>,
}

impl PadState {
    /// What the pad in this slot is (kind, name, backend, abilities,
    /// battery) — `None` until the backend said (a script's pad).
    pub fn info(&self) -> Option<&PadInfo> {
        self.info.as_ref()
    }
    /// Its kind: the glyphs to draw ([`PadKind::glyphs`]).
    pub fn kind(&self) -> PadKind {
        self.info.as_ref().map_or(PadKind::Generic, |i| i.kind)
    }
    /// Its motion sensors, while switched on.
    pub fn motion(&self) -> Option<&PadMotion> {
        self.motion.as_ref()
    }
    /// An analog input as it was when the frame before ended.
    pub fn axis_before(&self, axis: PadAxis) -> f32 {
        shape(axis, self.axes_before.get(&axis).copied().unwrap_or(0.0))
    }
    /// Whether a pad is in this slot now. A slot whose pad went keeps its
    /// number, empty, until it (or the next pad) comes back.
    pub fn connected(&self) -> bool {
        self.connected
    }
    pub fn held(&self, button: PadButton) -> bool {
        self.held.contains(&button)
    }
    pub fn pressed(&self, button: PadButton) -> bool {
        self.pressed.contains(&button)
    }
    pub fn released(&self, button: PadButton) -> bool {
        self.released.contains(&button)
    }
    /// Buttons held now.
    pub fn held_buttons(&self) -> impl Iterator<Item = PadButton> + '_ {
        self.held.iter().copied()
    }
    /// Every axis it reported, as it reported it (no dead zone).
    pub fn raw_axes(&self) -> impl Iterator<Item = (PadAxis, f32)> + '_ {
        self.axes.iter().map(|(a, v)| (*a, *v))
    }
    /// Buttons that went down this frame.
    pub fn pressed_buttons(&self) -> impl Iterator<Item = PadButton> + '_ {
        self.pressed.iter().copied()
    }
    /// An analog input, the [`DEAD_ZONE`] taken out of the sticks (see
    /// [`Input::pad_axis`]).
    pub fn axis(&self, axis: PadAxis) -> f32 {
        shape(axis, self.raw(axis))
    }
    fn raw(&self, axis: PadAxis) -> f32 {
        self.axes.get(&axis).copied().unwrap_or(0.0)
    }
    fn begin_frame(&mut self) {
        self.pressed.clear();
        self.released.clear();
        self.axes_before.clone_from(&self.axes);
    }
    fn down(&mut self, button: PadButton) {
        if self.held.insert(button) {
            self.pressed.insert(button);
        }
    }
    fn up(&mut self, button: PadButton) {
        if self.held.remove(&button) {
            self.released.insert(button);
        }
    }
    /// Everything held let go, the sticks back to rest.
    fn let_go(&mut self) {
        for button in std::mem::take(&mut self.held) {
            self.released.insert(button);
        }
        self.axes.clear();
        self.motion = None;
    }
}

/// An axis as a game reads it: triggers 0..1, finger places as they are,
/// sticks with the [`DEAD_ZONE`] taken out and stretched back to −1..1.
fn shape(axis: PadAxis, value: f32) -> f32 {
    if axis.is_trigger() {
        value.clamp(0.0, 1.0)
    } else if axis.is_finger() {
        value.clamp(-1.0, 1.0)
    } else if value.abs() < DEAD_ZONE {
        0.0
    } else {
        value.signum() * (value.abs() - DEAD_ZONE) / (1.0 - DEAD_ZONE)
    }
}

/// Which player slot each physical pad is in: the first pad that comes is
/// slot 0, the next slot 1, by the order they come — a couch's player 1 is
/// whoever plugged in first. A pad that goes keeps its slot empty for its
/// return; a pad seen for the first time takes the lowest empty slot before
/// a new one, so a pad that lost its battery and came back as another
/// device is still player 1. Devices are whatever number the platform
/// gives a pad for as long as it is connected (gilrs' id in the window).
#[derive(Debug, Clone, Default)]
pub struct PadSlots {
    /// Each slot's device, and whether it is in.
    slots: Vec<(u64, bool)>,
}

impl PadSlots {
    pub fn new() -> Self {
        Self::default()
    }

    /// A device came in: its slot.
    pub fn connect(&mut self, device: u64) -> usize {
        if let Some(i) = self.slots.iter().position(|(d, _)| *d == device) {
            self.slots[i].1 = true;
            return i;
        }
        if let Some(i) = self.slots.iter().position(|(_, on)| !on) {
            self.slots[i] = (device, true);
            return i;
        }
        self.slots.push((device, true));
        self.slots.len() - 1
    }

    /// A device went: the slot it leaves empty.
    pub fn disconnect(&mut self, device: u64) -> Option<usize> {
        let i = self.slot(device)?;
        self.slots[i].1 = false;
        Some(i)
    }

    /// The slot a connected device is in.
    pub fn slot(&self, device: u64) -> Option<usize> {
        self.slots.iter().position(|(d, on)| *d == device && *on)
    }

    /// The device connected in a slot.
    pub fn device(&self, slot: usize) -> Option<u64> {
        self.slots.get(slot).filter(|(_, on)| *on).map(|(d, _)| *d)
    }
}

/// Everything held, pressed and released, plus where the mouse is.
#[derive(Debug, Clone, Default)]
pub struct Input {
    held: HashSet<Key>,
    pressed: HashSet<Key>,
    released: HashSet<Key>,
    buttons_held: HashSet<MouseButton>,
    buttons_pressed: HashSet<MouseButton>,
    buttons_released: HashSet<MouseButton>,
    position: Vec2,
    motion: Vec2,
    scroll: Vec2,
    text: String,
    has_position: bool,
    /// Every pad at once: what a game with one player reads.
    pad: PadState,
    /// Each player slot's pad ([`InputEvent::PadDownOn`] and its kin).
    pads: Vec<PadState>,
    touches: Vec<Touch>,
    /// The touch the mouse follows: the first finger down, as Unity's
    /// simulated mouse, so buttons and sliders work under a finger.
    mouse_finger: Option<u64>,
}

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear what only lasted a frame. Called by the shell before it feeds
    /// in the new frame's events.
    pub fn begin_frame(&mut self) {
        self.pressed.clear();
        self.released.clear();
        self.buttons_pressed.clear();
        self.buttons_released.clear();
        self.pad.begin_frame();
        for pad in &mut self.pads {
            pad.begin_frame();
        }
        self.motion = Vec2::ZERO;
        self.scroll = Vec2::ZERO;
        self.text.clear();
        self.touches.retain(|t| !t.ended);
        for touch in &mut self.touches {
            touch.began = false;
        }
    }

    /// Every finger down, and those lifted this frame.
    pub fn touches(&self) -> &[Touch] {
        &self.touches
    }

    pub fn handle(&mut self, event: &InputEvent) {
        match event {
            InputEvent::KeyDown(key) => {
                // A held key repeats, and a repeat is not a press. Without
                // this an inventory bound to `Tab` opens and closes itself
                // while the key is down.
                if self.held.insert(*key) {
                    self.pressed.insert(*key);
                }
            }
            InputEvent::KeyUp(key) => {
                if self.held.remove(key) {
                    self.released.insert(*key);
                }
            }
            InputEvent::Text(text) => self.text.push_str(text),
            InputEvent::MouseMoved { x, y } => {
                let position = Vec2::new(*x, *y);
                if self.has_position {
                    self.motion += position - self.position;
                }
                self.position = position;
                self.has_position = true;
            }
            InputEvent::MouseMotion { dx, dy } => self.motion += Vec2::new(*dx, *dy),
            InputEvent::MouseDown(button) => {
                if self.buttons_held.insert(*button) {
                    self.buttons_pressed.insert(*button);
                }
            }
            InputEvent::MouseUp(button) => {
                if self.buttons_held.remove(button) {
                    self.buttons_released.insert(*button);
                }
            }
            InputEvent::Scroll { x, y } => self.scroll += Vec2::new(*x, *y),
            InputEvent::PadDown(button) => self.pad.down(*button),
            InputEvent::PadUp(button) => self.pad.up(*button),
            InputEvent::PadMoved { axis, value } => {
                self.pad.axes.insert(*axis, value.clamp(-1.0, 1.0));
            }
            InputEvent::PadDownOn { pad, button } => {
                self.heard(*pad).down(*button);
                self.pad.down(*button);
            }
            InputEvent::PadUpOn { pad, button } => {
                self.heard(*pad).up(*button);
                self.merge_up(*button);
            }
            InputEvent::PadMovedOn { pad, axis, value } => {
                self.heard(*pad).axes.insert(*axis, value.clamp(-1.0, 1.0));
                self.merge_axis(*axis);
            }
            InputEvent::PadConnected(pad) => {
                self.heard(*pad);
            }
            InputEvent::PadDisconnected(pad) => {
                let gone = self.slot(*pad);
                gone.connected = false;
                let buttons: Vec<PadButton> = gone.held.iter().copied().collect();
                let axes: Vec<PadAxis> = gone.axes.keys().copied().collect();
                gone.let_go();
                for button in buttons {
                    self.merge_up(button);
                }
                for axis in axes {
                    self.merge_axis(axis);
                }
            }
            InputEvent::Touch { id, phase, x, y } => {
                let at = Vec2::new(*x, *y);
                match phase {
                    TouchPhase::Started => {
                        self.touches.retain(|t| t.id != *id);
                        self.touches.push(Touch {
                            id: *id,
                            position: at,
                            start: at,
                            began: true,
                            ended: false,
                        });
                    }
                    TouchPhase::Moved => {
                        if let Some(t) = self.touches.iter_mut().find(|t| t.id == *id) {
                            t.position = at;
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        if let Some(t) = self.touches.iter_mut().find(|t| t.id == *id) {
                            t.position = at;
                            t.ended = true;
                        }
                    }
                }
                // The first finger is also the mouse.
                if *phase == TouchPhase::Started && self.mouse_finger.is_none() {
                    self.mouse_finger = Some(*id);
                    self.handle(&InputEvent::MouseMoved { x: *x, y: *y });
                    self.handle(&InputEvent::MouseDown(MouseButton::Left));
                } else if self.mouse_finger == Some(*id) {
                    self.handle(&InputEvent::MouseMoved { x: *x, y: *y });
                    if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                        self.handle(&InputEvent::MouseUp(MouseButton::Left));
                        self.mouse_finger = None;
                    }
                }
            }
            InputEvent::PadInfo { pad, info } => self.heard(*pad).info = Some(info.clone()),
            InputEvent::PadMotion { pad, motion } => self.heard(*pad).motion = Some(*motion),
            InputEvent::FocusLost => {
                self.pad.let_go();
                for pad in &mut self.pads {
                    pad.let_go();
                }
                for key in std::mem::take(&mut self.held) {
                    self.released.insert(key);
                }
                for button in std::mem::take(&mut self.buttons_held) {
                    self.buttons_released.insert(button);
                }
            }
        }
    }

    /// A slot's pad, made (connected) when a slot is first heard of: a
    /// script's `PadDownOn(pad: 1, …)` needs no `PadConnected` first.
    fn slot(&mut self, pad: usize) -> &mut PadState {
        if self.pads.len() <= pad {
            self.pads.resize_with(pad + 1, PadState::default);
        }
        &mut self.pads[pad]
    }

    /// A slot's pad that just said something: it is there.
    fn heard(&mut self, pad: usize) -> &mut PadState {
        let state = self.slot(pad);
        state.connected = true;
        state
    }

    /// Merged, a button is up when no slot's pad holds it.
    fn merge_up(&mut self, button: PadButton) {
        if !self.pads.iter().any(|p| p.held(button)) {
            self.pad.up(button);
        }
    }

    /// The merged pad's axis: the slot's pad pushed furthest, so one pad
    /// resting off centre does not drown the other's push.
    fn merge_axis(&mut self, axis: PadAxis) {
        let value = self
            .pads
            .iter()
            .map(|p| p.raw(axis))
            .fold(0.0f32, |best, v| if v.abs() > best.abs() { v } else { best });
        self.pad.axes.insert(axis, value);
    }

    /// One player slot's pad ([`PadSlots`]): `None` for a slot no pad has
    /// been in. What a game of two players on two pads reads, each its own.
    pub fn pad(&self, slot: usize) -> Option<&PadState> {
        self.pads.get(slot)
    }

    /// Every slot a pad has been in, by slot, connected or not now.
    pub fn pads(&self) -> &[PadState] {
        &self.pads
    }

    /// Every pad at once, as [`Input::pad_held`] and the rest read it.
    pub fn any_pad(&self) -> &PadState {
        &self.pad
    }

    pub fn pad_held(&self, button: PadButton) -> bool {
        self.pad.held(button)
    }
    pub fn pad_pressed(&self, button: PadButton) -> bool {
        self.pad.pressed(button)
    }
    pub fn pad_released(&self, button: PadButton) -> bool {
        self.pad.released(button)
    }

    /// An analog input, with the [`DEAD_ZONE`] taken out of the sticks and
    /// what is left stretched back to the full range, so a stick pushed just
    /// past the zone moves slowly rather than jumping to 15%.
    pub fn pad_axis(&self, axis: PadAxis) -> f32 {
        self.pad.axis(axis)
    }

    /// Keys that went down this frame, mouse buttons and pad buttons —
    /// what "press the key for Jump" waits for.
    pub fn pressed_keys(&self) -> impl Iterator<Item = Key> + '_ {
        self.pressed.iter().copied()
    }
    pub fn pressed_buttons(&self) -> impl Iterator<Item = MouseButton> + '_ {
        self.buttons_pressed.iter().copied()
    }
    pub fn pressed_pad_buttons(&self) -> impl Iterator<Item = PadButton> + '_ {
        self.pad.pressed_buttons()
    }

    pub fn held(&self, key: Key) -> bool {
        self.held.contains(&key)
    }
    pub fn pressed(&self, key: Key) -> bool {
        self.pressed.contains(&key)
    }
    pub fn released(&self, key: Key) -> bool {
        self.released.contains(&key)
    }
    pub fn mouse_held(&self, button: MouseButton) -> bool {
        self.buttons_held.contains(&button)
    }
    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.buttons_pressed.contains(&button)
    }
    pub fn mouse_released(&self, button: MouseButton) -> bool {
        self.buttons_released.contains(&button)
    }
    /// Where the cursor is, in pixels from the window's top left.
    pub fn mouse_position(&self) -> Vec2 {
        self.position
    }
    /// How far the mouse moved this frame.
    pub fn mouse_motion(&self) -> Vec2 {
        self.motion
    }
    pub fn scroll(&self) -> Vec2 {
        self.scroll
    }
    /// Characters typed this frame, layout applied.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// `1.0` when `positive` is held, `-1.0` when `negative` is, `0.0` when
    /// both or neither are.
    ///
    /// Both-at-once has to cancel rather than pick one, or a player holding
    /// left and right drifts in whichever direction the code happened to
    /// test first.
    pub fn axis(&self, negative: Key, positive: Key) -> f32 {
        f32::from(self.held(positive)) - f32::from(self.held(negative))
    }

    /// The `WASD` plane as `x` right and `y` forward, normalized so that
    /// walking diagonally is not faster than walking straight.
    pub fn move_axis(&self) -> Vec2 {
        let raw = Vec2::new(self.axis(Key::A, Key::D), self.axis(Key::S, Key::W));
        if raw.length_squared() > 1.0 {
            raw.normalize()
        } else {
            raw
        }
    }

    pub fn any_held(&self) -> bool {
        !self.held.is_empty() || !self.buttons_held.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_with(events: &[InputEvent]) -> Input {
        let mut input = Input::new();
        input.begin_frame();
        for event in events {
            input.handle(event);
        }
        input
    }

    #[test]
    fn a_press_lasts_one_frame_and_the_hold_lasts_until_release() {
        let mut input = input_with(&[InputEvent::KeyDown(Key::W)]);
        assert!(input.pressed(Key::W) && input.held(Key::W));

        input.begin_frame();
        assert!(!input.pressed(Key::W), "the press was last frame's");
        assert!(input.held(Key::W), "but it is still down");

        input.handle(&InputEvent::KeyUp(Key::W));
        assert!(input.released(Key::W) && !input.held(Key::W));
    }

    #[test]
    fn a_key_repeat_is_not_a_second_press() {
        // Held keys repeat. A menu bound to a press would open and close
        // itself while the key is down.
        let input = input_with(&[
            InputEvent::KeyDown(Key::Tab),
            InputEvent::KeyDown(Key::Tab),
            InputEvent::KeyDown(Key::Tab),
        ]);
        assert!(input.pressed(Key::Tab));
        assert_eq!(input.pressed.len(), 1);
    }

    #[test]
    fn losing_focus_releases_everything_instead_of_sticking() {
        // The key-up goes to whatever has focus now, so a key held across a
        // window switch is held forever.
        let mut input = input_with(&[
            InputEvent::KeyDown(Key::W),
            InputEvent::MouseDown(MouseButton::Left),
        ]);
        input.begin_frame();
        input.handle(&InputEvent::FocusLost);

        assert!(!input.held(Key::W));
        assert!(input.released(Key::W));
        assert!(!input.mouse_held(MouseButton::Left));
        assert!(!input.any_held());
    }

    #[test]
    fn opposite_keys_cancel_rather_than_one_winning() {
        let input = input_with(&[InputEvent::KeyDown(Key::A), InputEvent::KeyDown(Key::D)]);
        assert_eq!(input.axis(Key::A, Key::D), 0.0);
    }

    #[test]
    fn walking_diagonally_is_not_faster_than_walking_straight() {
        let diagonal = input_with(&[InputEvent::KeyDown(Key::W), InputEvent::KeyDown(Key::D)]);
        let straight = input_with(&[InputEvent::KeyDown(Key::W)]);
        assert!((diagonal.move_axis().length() - 1.0).abs() < 1e-5);
        assert!((straight.move_axis().length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_first_mouse_position_is_not_a_jump_from_the_origin() {
        // Treating the first report as motion from (0, 0) whips the camera
        // round on the first frame the cursor enters the window.
        let input = input_with(&[InputEvent::MouseMoved { x: 400.0, y: 300.0 }]);
        assert_eq!(input.mouse_position(), Vec2::new(400.0, 300.0));
        assert_eq!(input.mouse_motion(), Vec2::ZERO);
    }

    #[test]
    fn motion_accumulates_within_a_frame_and_resets_between_them() {
        // Several move events can arrive in one frame, and a camera that
        // reads only the last one loses the rest.
        let mut input = input_with(&[
            InputEvent::MouseMotion { dx: 3.0, dy: 0.0 },
            InputEvent::MouseMotion { dx: 4.0, dy: -2.0 },
        ]);
        assert_eq!(input.mouse_motion(), Vec2::new(7.0, -2.0));
        input.begin_frame();
        assert_eq!(input.mouse_motion(), Vec2::ZERO);
    }

    #[test]
    fn text_is_separate_from_keys_because_a_layout_sits_between_them() {
        let input = input_with(&[InputEvent::KeyDown(Key::A), InputEvent::Text("ф".into())]);
        assert!(input.held(Key::A), "the physical key is still A");
        assert_eq!(input.text(), "ф", "what was typed is not");
    }
}

/// Input played from a file instead of the hands: `SCRAP_INPUT_SCRIPT`
/// names a RON file of `(seconds, event)` pairs, from when the game
/// started — `[(2.0, KeyDown(W)), (4.0, KeyUp(W)), (5.0, MouseDown(Left))]`
/// — and, with `SCRAP_INPUT_QUIT`, the game ends that many seconds after
/// the last. The shell hands them to the same [`Input`] the keyboard
/// does, so what a script plays is played through the real window, the
/// real frame and the real loop: what a test harness beside the game
/// cannot vouch for. A pad of a player slot is addressed by its number —
/// `(1.0, PadDownOn(pad: 1, button: South))`, `(1.0, PadMovedOn(pad: 1,
/// axis: LeftX, value: 1.0))` — which is how two pads are played without
/// two pads.
#[derive(Debug, Clone, Default)]
pub struct InputScript {
    events: Vec<(f32, InputEvent)>,
    next: usize,
    quit_after: Option<f32>,
}

/// The variable naming the script.
pub const SCRIPT_VAR: &str = "SCRAP_INPUT_SCRIPT";

impl InputScript {
    /// The script `SCRAP_INPUT_SCRIPT` names, if any; what does not read is
    /// said and ignored.
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os(SCRIPT_VAR)?;
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{}: {e}", std::path::Path::new(&path).display());
                return None;
            }
        };
        match Self::parse(&text) {
            Ok(mut s) => {
                s.quit_after = std::env::var("SCRAP_INPUT_QUIT").ok().and_then(|q| q.parse().ok());
                Some(s)
            }
            Err(e) => {
                eprintln!("{}: {e}", std::path::Path::new(&path).display());
                None
            }
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut events: Vec<(f32, InputEvent)> = ron::from_str(text).map_err(|e| e.to_string())?;
        events.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(Self { events, next: 0, quit_after: None })
    }

    /// The events due by `seconds` since the start, into `input`.
    pub fn play(&mut self, seconds: f32, input: &mut Input) {
        for event in self.due(seconds) {
            input.handle(&event);
        }
    }

    /// The events due by `seconds` since the start, not given before: for
    /// a shell that hands them to more than the [`Input`] — the game's UI
    /// tree, as it hands the window's.
    pub fn due(&mut self, seconds: f32) -> Vec<InputEvent> {
        let mut out = Vec::new();
        while let Some((at, event)) = self.events.get(self.next) {
            if *at > seconds {
                break;
            }
            out.push(event.clone());
            self.next += 1;
        }
        out
    }

    /// Whether the script says the game is over by `seconds`.
    pub fn over(&self, seconds: f32) -> bool {
        let last = self.events.last().map_or(0.0, |e| e.0);
        self.quit_after.is_some_and(|q| seconds >= last + q)
    }
}

/// Input read from a file as it is written (`SCRAP_INPUT_LIVE=path`): a
/// line an event, as a script names it (`KeyDown(Space)`), taken one a
/// frame in the order written — so a `KeyDown` and its `KeyUp` written
/// together are a tap. What drives a running game from outside it, a
/// line at a time: a tool, an agent looking at its frames between keys.
#[derive(Debug)]
pub struct LiveInput {
    path: std::path::PathBuf,
    read: usize,
    queued: std::collections::VecDeque<InputEvent>,
}

/// The variable naming the file live input is read from.
pub const LIVE_VAR: &str = "SCRAP_INPUT_LIVE";

impl LiveInput {
    /// The file `SCRAP_INPUT_LIVE` names, if any (made empty).
    pub fn from_env() -> Option<Self> {
        let path = std::path::PathBuf::from(std::env::var_os(LIVE_VAR)?);
        if let Err(e) = std::fs::write(&path, "") {
            eprintln!("{}: {e}", path.display());
            return None;
        }
        Some(Self { path, read: 0, queued: Default::default() })
    }

    /// The next event written, if any, into `input`.
    pub fn play(&mut self, input: &mut Input) {
        if let Some(event) = self.next() {
            input.handle(&event);
        }
    }

    /// The next event written, if any: for a shell that hands it to more
    /// than the [`Input`] — the game's UI tree, as it hands the window's.
    #[allow(clippy::should_implement_trait)] // not an iterator: it reads the file as it goes
    pub fn next(&mut self) -> Option<InputEvent> {
        if self.queued.is_empty() {
            if let Ok(text) = std::fs::read_to_string(&self.path) {
                // Whole lines only: one being written waits for its end.
                let fresh = text.get(self.read..).unwrap_or_default();
                if let Some(end) = fresh.rfind('\n') {
                    for line in fresh[..end].lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")) {
                        match ron::from_str::<InputEvent>(line) {
                            Ok(event) => self.queued.push_back(event),
                            Err(e) => eprintln!("{}: {line}: {e}", self.path.display()),
                        }
                    }
                    self.read += end + 1;
                }
            }
        }
        self.queued.pop_front()
    }
}

#[cfg(test)]
mod script_tests {
    use super::*;

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn live_input_is_taken_a_line_a_frame_as_it_is_written() {
        let path = std::env::temp_dir().join(format!("scrap_live_input_{}", std::process::id()));
        std::fs::write(&path, "").unwrap();
        let mut live = LiveInput { path: path.clone(), read: 0, queued: Default::default() };
        let mut input = Input::new();
        live.play(&mut input);
        assert!(!input.any_held());
        std::fs::write(&path, "KeyDown(Space)\nKeyUp(Space)\nKeyDown(W").unwrap();
        live.play(&mut input);
        assert!(input.pressed(Key::Space));
        input.begin_frame();
        live.play(&mut input);
        assert!(!input.held(Key::Space) && input.released(Key::Space));
        input.begin_frame();
        live.play(&mut input);
        assert!(!input.held(Key::W), "half a line waits");
        std::fs::write(&path, "KeyDown(Space)\nKeyUp(Space)\nKeyDown(W)\n").unwrap();
        live.play(&mut input);
        assert!(input.held(Key::W));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_script_plays_its_events_when_they_are_due() {
        let mut s = InputScript::parse("[(1.0, KeyDown(W)), (0.5, MouseDown(Left)), (2.0, KeyUp(W))]").unwrap();
        let mut input = Input::new();
        s.play(0.6, &mut input);
        assert!(input.mouse_held(MouseButton::Left) && !input.held(Key::W));
        input.begin_frame();
        s.play(1.5, &mut input);
        assert!(input.pressed(Key::W));
        input.begin_frame();
        s.play(2.5, &mut input);
        assert!(!input.held(Key::W));
        s.quit_after = Some(1.0);
        assert!(!s.over(2.5) && s.over(3.0));
    }

    #[test]
    fn a_script_drives_the_second_pad_and_not_the_first() {
        let mut s = InputScript::parse(
            "[(0.0, PadDownOn(pad: 1, button: South)), (0.0, PadMovedOn(pad: 1, axis: LeftX, value: 1.0)),
              (0.5, PadDownOn(pad: 0, button: East)), (1.0, PadUpOn(pad: 1, button: South))]",
        )
        .unwrap();
        let mut input = Input::new();
        s.play(0.1, &mut input);
        let (first, second) = (input.pad(0), input.pad(1).unwrap());
        assert!(first.is_none_or(|p| !p.held(PadButton::South)), "the first pad is not pressed");
        assert!(second.pressed(PadButton::South) && second.axis(PadAxis::LeftX) == 1.0);
        assert!(input.pad_pressed(PadButton::South), "the merged pad sees both");
        input.begin_frame();
        s.play(0.6, &mut input);
        assert!(input.pad(0).unwrap().pressed(PadButton::East));
        assert!(!input.pad(1).unwrap().held(PadButton::East));
        input.begin_frame();
        s.play(1.1, &mut input);
        assert!(input.pad(1).unwrap().released(PadButton::South) && !input.pad_held(PadButton::South));
    }
}

#[cfg(test)]
mod pad_tests {
    use super::*;

    #[test]
    fn pads_take_slots_by_the_order_they_came_and_keep_them_while_away() {
        let mut slots = PadSlots::new();
        assert_eq!(slots.connect(70), 0);
        assert_eq!(slots.connect(12), 1);
        assert_eq!(slots.disconnect(70), Some(0));
        assert_eq!(slots.slot(12), Some(1), "the other player stays player 2");
        // Back as another device: the empty slot, not a third.
        assert_eq!(slots.connect(99), 0);
        assert_eq!(slots.connect(5), 2);
        assert_eq!(slots.disconnect(1234), None);
    }

    #[test]
    fn a_button_held_on_two_pads_is_held_merged_until_both_let_go() {
        let mut input = Input::new();
        let on = |pad, button| InputEvent::PadDownOn { pad, button };
        input.handle(&on(0, PadButton::South));
        input.handle(&on(1, PadButton::South));
        input.handle(&InputEvent::PadUpOn { pad: 0, button: PadButton::South });
        assert!(input.pad_held(PadButton::South), "pad 1 still holds it");
        assert!(!input.pad(0).unwrap().held(PadButton::South));
        // A pad unplugged lets go of what it held, and its stick rests.
        input.handle(&InputEvent::PadMovedOn { pad: 1, axis: PadAxis::LeftY, value: -1.0 });
        input.handle(&InputEvent::PadMovedOn { pad: 0, axis: PadAxis::LeftY, value: 0.05 });
        assert_eq!(input.pad_axis(PadAxis::LeftY), -1.0, "the push wins over the rest");
        input.handle(&InputEvent::PadDisconnected(1));
        assert!(!input.pad_held(PadButton::South) && input.pad_released(PadButton::South));
        assert_eq!(input.pad_axis(PadAxis::LeftY), 0.0);
        assert!(!input.pad(1).unwrap().connected() && input.pad(0).unwrap().connected());
        // Unchanged for a game of one: the old events still drive the merged pad.
        input.handle(&InputEvent::PadDown(PadButton::North));
        assert!(input.pad_held(PadButton::North));
    }
}
