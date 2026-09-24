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
}

/// How far a stick has to move before it counts. Every stick rests a little
/// off centre, and a character that creeps while nobody touches the pad is
/// the bug this prevents.
pub const DEAD_ZONE: f32 = 0.15;

/// What a shell reports.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    pad_held: HashSet<PadButton>,
    pad_pressed: HashSet<PadButton>,
    pad_released: HashSet<PadButton>,
    pad_axes: std::collections::HashMap<PadAxis, f32>,
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
        self.pad_pressed.clear();
        self.pad_released.clear();
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
            InputEvent::PadDown(button) => {
                if self.pad_held.insert(*button) {
                    self.pad_pressed.insert(*button);
                }
            }
            InputEvent::PadUp(button) => {
                if self.pad_held.remove(button) {
                    self.pad_released.insert(*button);
                }
            }
            InputEvent::PadMoved { axis, value } => {
                self.pad_axes.insert(*axis, value.clamp(-1.0, 1.0));
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
            InputEvent::FocusLost => {
                for button in std::mem::take(&mut self.pad_held) {
                    self.pad_released.insert(button);
                }
                self.pad_axes.clear();
                for key in std::mem::take(&mut self.held) {
                    self.released.insert(key);
                }
                for button in std::mem::take(&mut self.buttons_held) {
                    self.buttons_released.insert(button);
                }
            }
        }
    }

    pub fn pad_held(&self, button: PadButton) -> bool {
        self.pad_held.contains(&button)
    }
    pub fn pad_pressed(&self, button: PadButton) -> bool {
        self.pad_pressed.contains(&button)
    }
    pub fn pad_released(&self, button: PadButton) -> bool {
        self.pad_released.contains(&button)
    }

    /// An analog input, with the [`DEAD_ZONE`] taken out of the sticks and
    /// what is left stretched back to the full range, so a stick pushed just
    /// past the zone moves slowly rather than jumping to 15%.
    pub fn pad_axis(&self, axis: PadAxis) -> f32 {
        let value = self.pad_axes.get(&axis).copied().unwrap_or(0.0);
        match axis {
            PadAxis::LeftTrigger | PadAxis::RightTrigger => value.clamp(0.0, 1.0),
            _ if value.abs() < DEAD_ZONE => 0.0,
            _ => value.signum() * (value.abs() - DEAD_ZONE) / (1.0 - DEAD_ZONE),
        }
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
        self.pad_pressed.iter().copied()
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
