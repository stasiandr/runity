use runity_math::Vec2;
use runity_platform::{Event, Key, MouseButton};
use std::collections::HashSet;

/// Keyboard and mouse state, rebuilt from the platform event stream.
#[derive(Debug, Default, Clone)]
pub struct Input {
    down: HashSet<Key>,
    pressed: HashSet<Key>,
    released: HashSet<Key>,
    buttons_down: HashSet<MouseButton>,
    buttons_pressed: HashSet<MouseButton>,
    buttons_released: HashSet<MouseButton>,
    mouse_position: Vec2,
    mouse_delta: Vec2,
    scroll: f32,
    has_mouse_position: bool,
}

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the per-frame edges. Call once before feeding a frame's events.
    pub fn begin_frame(&mut self) {
        self.pressed.clear();
        self.released.clear();
        self.buttons_pressed.clear();
        self.buttons_released.clear();
        self.mouse_delta = Vec2::ZERO;
        self.scroll = 0.0;
    }

    pub fn handle(&mut self, event: &Event) {
        match *event {
            Event::KeyDown(key) => {
                // The OS repeats keys while they are held; only the first
                // transition counts as a press.
                if self.down.insert(key) {
                    self.pressed.insert(key);
                }
            }
            Event::KeyUp(key) => {
                self.down.remove(&key);
                self.released.insert(key);
            }
            Event::MouseDown(button) => {
                if self.buttons_down.insert(button) {
                    self.buttons_pressed.insert(button);
                }
            }
            Event::MouseUp(button) => {
                self.buttons_down.remove(&button);
                self.buttons_released.insert(button);
            }
            Event::MouseMove { x, y } => {
                let position = Vec2::new(x as f32, y as f32);
                if self.has_mouse_position {
                    self.mouse_delta = self.mouse_delta + (position - self.mouse_position);
                }
                self.mouse_position = position;
                self.has_mouse_position = true;
            }
            Event::Scroll(amount) => self.scroll += amount,
            Event::FocusLost => {
                // Keys released while we were not focused never send KeyUp.
                for key in std::mem::take(&mut self.down) {
                    self.released.insert(key);
                }
                self.buttons_down.clear();
            }
            _ => {}
        }
    }

    /// Held right now.
    pub fn key_down(&self, key: Key) -> bool {
        self.down.contains(&key)
    }
    /// Went down this frame.
    pub fn key_pressed(&self, key: Key) -> bool {
        self.pressed.contains(&key)
    }
    /// Came up this frame.
    pub fn key_released(&self, key: Key) -> bool {
        self.released.contains(&key)
    }
    pub fn mouse_down(&self, button: MouseButton) -> bool {
        self.buttons_down.contains(&button)
    }
    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.buttons_pressed.contains(&button)
    }
    pub fn mouse_released(&self, button: MouseButton) -> bool {
        self.buttons_released.contains(&button)
    }
    pub fn mouse_position(&self) -> Vec2 {
        self.mouse_position
    }
    pub fn mouse_delta(&self) -> Vec2 {
        self.mouse_delta
    }
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    /// `+1` when `positive` is held, `-1` when `negative` is — the classic
    /// movement axis.
    pub fn axis(&self, negative: Key, positive: Key) -> f32 {
        (self.key_down(positive) as i32 - self.key_down(negative) as i32) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_is_an_edge_and_down_is_a_level() {
        let mut input = Input::new();
        input.begin_frame();
        input.handle(&Event::KeyDown(Key::W));
        assert!(input.key_pressed(Key::W) && input.key_down(Key::W));

        // Next frame, still held: down but no longer a fresh press.
        input.begin_frame();
        input.handle(&Event::KeyDown(Key::W)); // key repeat
        assert!(input.key_down(Key::W));
        assert!(!input.key_pressed(Key::W));

        input.begin_frame();
        input.handle(&Event::KeyUp(Key::W));
        assert!(input.key_released(Key::W));
        assert!(!input.key_down(Key::W));
    }

    #[test]
    fn axis_combines_two_keys() {
        let mut input = Input::new();
        input.handle(&Event::KeyDown(Key::D));
        assert_eq!(input.axis(Key::A, Key::D), 1.0);
        input.handle(&Event::KeyDown(Key::A));
        assert_eq!(input.axis(Key::A, Key::D), 0.0, "both held cancels out");
        input.handle(&Event::KeyUp(Key::D));
        assert_eq!(input.axis(Key::A, Key::D), -1.0);
    }

    #[test]
    fn mouse_delta_is_relative_and_resets_each_frame() {
        let mut input = Input::new();
        input.begin_frame();
        input.handle(&Event::MouseMove { x: 10, y: 10 });
        assert_eq!(
            input.mouse_delta(),
            Vec2::ZERO,
            "the first sample has no previous position"
        );
        input.handle(&Event::MouseMove { x: 15, y: 8 });
        assert_eq!(input.mouse_delta(), Vec2::new(5.0, -2.0));
        input.begin_frame();
        assert_eq!(input.mouse_delta(), Vec2::ZERO);
        assert_eq!(
            input.mouse_position(),
            Vec2::new(15.0, 8.0),
            "position persists"
        );
    }

    #[test]
    fn losing_focus_releases_everything() {
        let mut input = Input::new();
        input.handle(&Event::KeyDown(Key::Space));
        input.handle(&Event::MouseDown(MouseButton::Left));
        input.begin_frame();
        input.handle(&Event::FocusLost);
        assert!(!input.key_down(Key::Space));
        assert!(input.key_released(Key::Space));
        assert!(!input.mouse_down(MouseButton::Left));
    }

    #[test]
    fn scroll_accumulates_within_a_frame() {
        let mut input = Input::new();
        input.begin_frame();
        input.handle(&Event::Scroll(1.0));
        input.handle(&Event::Scroll(0.5));
        assert_eq!(input.scroll(), 1.5);
        input.begin_frame();
        assert_eq!(input.scroll(), 0.0);
    }
}
